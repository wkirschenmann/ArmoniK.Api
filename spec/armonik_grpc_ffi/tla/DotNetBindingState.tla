-------------------------- MODULE DotNetBindingState --------------------------
(***************************************************************************)
(* The level-2 state: the shared level-0/level-1 state plus the managed    *)
(* variables of the .NET binding.                                          *)
(*                                                                         *)
(* Declarations only, following FfiGrpcState: DotNetBinding defines the    *)
(* level-2 machinery over it and reaches the level-1 machinery through an  *)
(* INSTANCE of FfiGrpcTheorems.                                            *)
(*                                                                         *)
(* No new constants.  The managed side adds discipline, not capacity: the  *)
(* pipelining depths, the byte ceiling and the identity spaces are the     *)
(* ABI's and arrived at level 1.  Generations of the reusable runtime are  *)
(* bounded by the finite RuntimeIds and reopened channels by the finite    *)
(* ChannelIds - modelling bounds, like BufferIds.                          *)
(*                                                                         *)
(* The ring is deliberately NOT here.  Both of its indexes are level-1     *)
(* state read through level-2 names: the trampoline publishes inside the   *)
(* delivery callback, so the published prefix IS events_delivered and the  *)
(* head is its length; a release is ak_event_consumed, so the tail IS      *)
(* payloads_consumed_by_host.  Advancing a counter can only release the    *)
(* oldest, which is release order held by representation.                  *)
(*                                                                         *)
(* Call ownership is not here either: call_channel, the level-0 relation,  *)
(* already says which channel a call belongs to, and the channel is the    *)
(* unit of borrowing - a CallInvoker is a stateless view over it.          *)
(***************************************************************************)

EXTENDS FfiGrpcState

VARIABLES
    (***********************************************************************)
    (* Roots and identity plumbing.  What must hold of callback, call_ctx  *)
    (* and runtime_ctx - the arguments level 1 does not model - is that    *)
    (* they resolve to live objects whenever a callback runs.              *)
    (***********************************************************************)
    call_token_published,   \* [CallIds -> BOOLEAN]: GCHandle.Alloc done,
                            \* in the same step as ak_call_start
    call_root_live,         \* [CallIds -> BOOLEAN]: the call's GC root
    runtime_root_live,      \* BOOLEAN: the shared RuntimeState's GC root

    (***********************************************************************)
    (* The shared runtime, by generation.  The factory materializes it     *)
    (* with the first channel, tears it down when the last lease goes,     *)
    (* and may materialize a fresh generation afterwards: the identity of  *)
    (* the current one is state, so teardown promises attach to it and    *)
    (* not to some earlier destroyed generation.                           *)
    (***********************************************************************)
    current_runtime,        \* RuntimeIds \union {"none"}
    runtime_dispose_state,  \* {"absent", "active", "shutdown_pending",
                            \*  "destroying",
                            \*  "destroyed"} - absent means no runtime is
                            \* materialized; FreeRuntimeRoot re-arms to it

    (***********************************************************************)
    (* The channels, each holding a lease on the runtime.  A channel is    *)
    (* the object the application creates and disposes; constructing       *)
    (* covers the window between the factory's materialization and the     *)
    (* channel's own ak_channel_create, invisible outside the constructor. *)
    (* "Last lease released" is derived: every channel is unopened or      *)
    (* disposed.                                                           *)
    (***********************************************************************)
    channel_dispose_state,  \* [ChannelIds -> {"unopened", "constructing",
                            \*                 "active", "disposing",
                            \*                 "released",
                            \*                 "released_last",
                            \*                 "disposed"}]: released gave
                            \* the lease back, released_last was the one
                            \* that emptied the set, disposed completed
                            \* the public task

    (***********************************************************************)
    (* The ring's consumer.  The phase says which class of consumer has    *)
    (* the ring; the reader says what the application's read is doing -    *)
    (* idle, waiting on an empty ring (a suspended MoveNext), or parsing   *)
    (* a taken slot.  One value per call is IAsyncStreamReader's           *)
    (* single-read contract made structural.                               *)
    (***********************************************************************)
    consumer_phase,         \* [CallIds -> {"prologue", "application",
                            \*              "drain", "done"}]
    reader_state,           \* [CallIds -> {"idle", "waiting", "parsing",
                            \*              "finished"}] - finished is the
                            \* consumed terminal: MoveNext answers false

    (***********************************************************************)
    (* The writer.  One value per call is IClientStreamWriter's            *)
    (* single-writer contract made structural; a write completes at its    *)
    (* WRITE_DONE, so the .NET surface exercises a native depth of one.    *)
    (* serializing is the only state that holds a lent buffer.             *)
    (***********************************************************************)
    writer_state,           \* [CallIds -> {"idle", "serializing",
                            \*              "waiting_budget",
                            \*              "awaiting_write_done",
                            \*              "closed"}]
    retry_len,              \* [CallIds -> RequestLengths + a sentinel]:
                            \* the refused request's length while a wait
                            \* is in progress, NoRetryLen otherwise

    (***********************************************************************)
    (* Managed completions.  The public objects that must never be left    *)
    (* pending: the headers resolve at the prologue or fault at a dispose  *)
    (* before the metadata; the status is resolved by whoever consumes the *)
    (* terminal slot - the reader or the drain - which is where its        *)
    (* payload is decoded.  The writer's completion is writer_state.       *)
    (***********************************************************************)
    headers_completion,     \* [CallIds -> {"pending", "succeeded",
                            \*              "failed"}]
    status_completion,      \* [CallIds -> {"pending", "resolved"}]

    (***********************************************************************)
    (* The call's dispose machine, driving the drain.                      *)
    (***********************************************************************)
    call_dispose_state      \* [CallIds -> {"active", "draining",
                            \*              "disposed"}]

managed_vars == <<call_token_published, call_root_live, runtime_root_live,
                  current_runtime, runtime_dispose_state,
                  channel_dispose_state,
                  consumer_phase, reader_state,
                  writer_state, retry_len,
                  headers_completion, status_completion,
                  call_dispose_state>>

===============================================================================
