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
(* ABI's and arrived at level 1.  The retry cadence is deliberately not a  *)
(* constant - no property of the model reads it, and the retry liveness    *)
(* is conditional on cancellation, not on time.                            *)
(*                                                                         *)
(* The ring is deliberately NOT here.  Both of its indexes are level-1     *)
(* state read through level-2 names: the trampoline publishes inside the   *)
(* delivery callback, so the published prefix IS events_delivered and the  *)
(* head is its length; a release is ak_event_consumed, so the tail IS      *)
(* payloads_consumed_by_host.  One counter standing for the outstanding    *)
(* set is also what makes release order structural: advancing a counter    *)
(* can only release the oldest.  What level 2 adds about the ring is who   *)
(* consumes it: the phase machine and the reader below.                    *)
(*                                                                         *)
(* The construction boundary is deliberately atomic: the concrete          *)
(* constructors allocate the root, perform the native create or start,     *)
(* and only then return the object, all before any other thread can see    *)
(* it - so root allocation and the native downcall are one step here       *)
(* (CreateRuntime, StartCall), and no state exists in which an allocated   *)
(* object is not yet exposed.                                              *)
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
    (* The ring's consumer.  The phase says which class of consumer has    *)
    (* the ring - the header prologue, the application, the drain.  The    *)
    (* reader says what the application's read is doing: idle, waiting on  *)
    (* an empty ring (a suspended MoveNext), or parsing a taken slot.      *)
    (* One value per call is the single-reader contract of                 *)
    (* IAsyncStreamReader made structural: a second concurrent MoveNext    *)
    (* is unrepresentable, exactly as the API forbids it.                  *)
    (***********************************************************************)
    consumer_phase,         \* [CallIds -> {"prologue", "application",
                            \*              "drain", "done"}]
    reader_state,           \* [CallIds -> {"idle", "waiting", "parsing"}]

    (***********************************************************************)
    (* The retry protocol.  The state says whether the call waits on the   *)
    (* budget, and the length says for which request: the wait is entered  *)
    (* by the budget refusal itself and exits on the successful lend,      *)
    (* cancellation or dispose.  The wait is cancellable, nothing more:    *)
    (* no repetition of attempts is promised, no cadence exists.           *)
    (***********************************************************************)
    retry_state,            \* [CallIds -> {"idle", "awaiting_budget"}]
    retry_len,              \* [CallIds -> RequestLengths + a sentinel]:
                            \* the refused request's length, NoRetryLen
                            \* when not waiting

    (***********************************************************************)
    (* Dispose machines.  The call's drives the drain; the runtime's       *)
    (* starts when the last reference is released, settles the calls       *)
    (* still open, and only then begins the native shutdown.               *)
    (***********************************************************************)
    call_dispose_state,     \* [CallIds -> {"active", "draining",
                            \*              "disposed"}]
    runtime_dispose_state   \* {"active", "disposing_calls", "destroying",
                            \*  "destroyed"}

managed_vars == <<call_token_published, call_root_live, runtime_root_live,
                  consumer_phase, reader_state,
                  retry_state, retry_len,
                  call_dispose_state, runtime_dispose_state>>

===============================================================================
