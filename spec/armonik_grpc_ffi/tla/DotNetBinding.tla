---------------------------- MODULE DotNetBinding ----------------------------
(***************************************************************************)
(* Level 2: the .NET binding, refining FfiGrpc.                            *)
(*                                                                         *)
(* SPEC UNDER CONVERGENCE - no proofs exist for this module yet.           *)
(*                                                                         *)
(* What this level proves, once frozen:                                    *)
(*  1. Spec => F!Spec - the refinement.  Everything proved at levels 0     *)
(*     and 1 is inherited through it; L0!Spec follows by transitivity      *)
(*     from level 1's RefinesSpec, never re-proved here.                   *)
(*  2. The six host fairness conjuncts of F!Fairness become theorems.      *)
(*     The other thirteen - the runtime's and the FFI dispatch's - are     *)
(*     taken verbatim into Fairness below.                                 *)
(*  3. The managed-side contract: the channel-held lease on the shared     *)
(*     runtime and its generations, the single reader, the single writer   *)
(*     completing at WRITE_DONE, the managed completions, the roots, and   *)
(*     the dispose ordering.                                               *)
(*                                                                         *)
(* Scope.  The model is the generic bidirectional-streaming call: the      *)
(* five CallInvoker methods are refinements of it that fix the number of   *)
(* messages in each direction, not separate machines, and are therefore    *)
(* not modelled.  The start/send command queue is level-1 stutter.  The    *)
(* .NET async runtime is not modelled: completions signal, continuations   *)
(* run elsewhere - RunContinuationsAsynchronously is an implementation     *)
(* rule verified by review and tests, not a theorem.  The memory model of  *)
(* the ring stays a coding rule for review, outside every level.  The     *)
(* lease refcount is derived, never counted: "last" is the set of          *)
(* channels not yet settled being empty.                                   *)
(*                                                                         *)
(* The application owes one progression fact per call, and only while the  *)
(* response stream is readable: begin the next read, or dispose the call   *)
(* early.  Nothing is asked once the terminal has been consumed - a        *)
(* finished call settles by itself, Dispose being optional for a completed *)
(* call in the .NET API - so PublishedCallEventuallySettled is a guarantee *)
(* of the binding rather than an obligation on the caller.  Beside that    *)
(* sit two conformity hypotheses of safety, encoded by representation:     *)
(* MoveNext calls are serialized (IAsyncStreamReader) and write operations *)
(* are serialized (IClientStreamWriter).  Everything else is carried by    *)
(* the binding, under one stated hypothesis: user serialization and        *)
(* parsing terminate.                                                     *)
(* The disposable wrapper returns the buffer on success and on exception   *)
(* alike; nothing can cover code that never comes back.                    *)
(***************************************************************************)

EXTENDS DotNetBindingState, Naturals, Sequences

\* The instance reaches level 1's theorems as well as its definitions.
\* What it must not reach is a theorem whose statement WRITES ENABLED:
\* TLAPS normalizes an instantiated module's statements eagerly, and the
\* ENABLED-elimination wrapper belongs to the module where the ENABLED is
\* written, so a substituted one aborts the prover.  A literal WF is fine
\* - it is a definition's body once expanded - which is why the three
\* conditional-enabledness theorems live apart in
\* FfiGrpcEnabledTheorems, a sibling this instance does not drag in.
F == INSTANCE FfiGrpcTheorems

l1_vars == F!vars
vars == <<l1_vars, managed_vars>>

ManagedStutter == UNCHANGED managed_vars

(***************************************************************************)
(* DERIVED PREDICATES                                                      *)
(***************************************************************************)

\* The ring read through level-1 state: head is what the trampoline
\* published, tail is what ak_event_consumed released.
RingHead(cId) == Len(events_delivered[cId])
RingTail(cId) == payloads_consumed_by_host[cId]
RingOccupancy(cId) == RingHead(cId) - RingTail(cId)
RingDrained(cId) == RingTail(cId) = RingHead(cId)

ConsumerPhases == {"prologue", "application", "drain", "done"}
ReaderStates == {"idle", "waiting", "parsing", "parsing_cancelled",
                 "finished"}

\* A read that has not completed: suspended, parsing, or parsing after its
\* own cancellation.  MoveNext(ct) cancels the CALL while its read is
\* unfinished and must do nothing once that read is done, which is the
\* contract IAsyncStreamReader states.  This is the window in which a
\* request may be RAISED, not the window in which one is outstanding: a
\* cancelled parse is still in flight while its request has already been
\* discharged, and the guard on the trigger keeps a second one from
\* arming.  A token firing outside the window finds no read to arm.
ReadInFlight(cId) ==
    reader_state[cId] \in {"waiting", "parsing", "parsing_cancelled"}
WriterStates == {"idle", "serializing", "waiting_budget",
                 "awaiting_write_done", "closed"}
CallDisposeStates == {"active", "draining", "settled"}
ChannelDisposeStates == {"unopened", "constructing", "rejected", "active",
                         "disposing", "released", "released_last",
                         "disposed"}
RuntimeDisposeStates == {"absent", "active", "shutdown_pending",
                         "destroying", "destroyed"}
HeadersCompletions == {"pending", "succeeded", "failed"}
StatusCompletions == {"pending", "resolved"}

\* The remembered length's sentinel: one past every request length, so the
\* whole domain stays integer - a string sentinel would make the type
\* heterogeneous, which TLC cannot compare.
NoRetryLen == Ceiling + 2

\* A settled channel holds no lease: it never opened, or it released.
\* Releasing the lease and completing the public DisposeAsync are two
\* steps, and which channel drove the count to zero is remembered rather
\* than recomputed: the release itself decides it, under the same lock
\* the implementation holds, and latches the manager to shutdown_pending
\* so no later lease can resurrect the generation between the zero and
\* the destroy.
ChannelSettled(chId) ==
    channel_dispose_state[chId] \in
        {"unopened", "rejected", "released", "released_last", "disposed"}

AllLeasesReleased == \A chId \in ChannelIds : ChannelSettled(chId)

\* This release is the one that empties the set: every other channel is
\* already settled.  Read inside FinishDisposeChannel, so the decision
\* and the latch are one step.
IsLastRelease(chId) ==
    \A other \in ChannelIds : other # chId => ChannelSettled(other)

\* A released channel's task may complete.  One that was not the last
\* owes nothing more.  The last one waits for the destroy of the
\* generation IT released - channel_runtime, not whichever runtime
\* happens to be current later.
ChannelDisposeMayResolve(chId) ==
    \/ channel_dispose_state[chId] = "released"
    \/ /\ channel_dispose_state[chId] = "released_last"
       /\ runtime_destroyed[channel_runtime[chId]]

\* The binding downcalls on a call only while neither the call nor the
\* runtime is being torn down.  The channel downcalls carry their own
\* channel-machine guards; NoDowncallAfterDestroy is the sum of all of
\* them plus the dispose ordering, not this predicate alone.
BindingMayDowncall(cId) ==
    /\ call_dispose_state[cId] = "active"
    /\ runtime_dispose_state = "active"

ManagedTypeOK ==
    /\ call_token_published \in [CallIds -> BOOLEAN]
    /\ call_root_live \in [CallIds -> BOOLEAN]
    /\ runtime_root_live \in BOOLEAN
    /\ current_runtime \in RuntimeIds \union {"none"}
    /\ runtime_dispose_state \in RuntimeDisposeStates
    /\ channel_dispose_state \in [ChannelIds -> ChannelDisposeStates]
    /\ consumer_phase \in [CallIds -> ConsumerPhases]
    /\ reader_state \in [CallIds -> ReaderStates]
    /\ read_cancel_pending \in [CallIds -> BOOLEAN]
    /\ writer_state \in [CallIds -> WriterStates]
    /\ retry_len \in [CallIds -> F!RequestLengths \union {NoRetryLen}]
    /\ headers_completion \in [CallIds -> HeadersCompletions]
    /\ status_completion \in [CallIds -> StatusCompletions]
    /\ call_dispose_state \in [CallIds -> CallDisposeStates]

(***************************************************************************)
(* INIT                                                                    *)
(***************************************************************************)

ManagedInit ==
    /\ call_token_published = [c \in CallIds |-> FALSE]
    /\ call_root_live = [c \in CallIds |-> FALSE]
    /\ runtime_root_live = FALSE
    /\ current_runtime = "none"
    /\ runtime_dispose_state = "absent"
    /\ channel_dispose_state = [ch \in ChannelIds |-> "unopened"]
    /\ consumer_phase = [c \in CallIds |-> "prologue"]
    /\ reader_state = [c \in CallIds |-> "idle"]
    /\ read_cancel_pending = [c \in CallIds |-> FALSE]
    /\ writer_state = [c \in CallIds |-> "idle"]
    /\ retry_len = [c \in CallIds |-> NoRetryLen]
    /\ headers_completion = [c \in CallIds |-> "pending"]
    /\ status_completion = [c \in CallIds |-> "pending"]
    /\ call_dispose_state = [c \in CallIds |-> "active"]

Init == F!Init /\ ManagedInit

(***************************************************************************)
(* THE FACTORY AND THE CHANNELS.  The first channel materializes the       *)
(* shared runtime; every later one takes a lease; the last release starts  *)
(* the teardown; a fresh generation may follow.  Construction is atomic    *)
(* with exposure at each scope: the constructor returns only after its     *)
(* native create, and the constructing window is invisible outside it.     *)
(***************************************************************************)

\* The first channel's constructor reaches the factory with no runtime
\* materialized: the shared RuntimeState's root and ak_runtime_create in
\* one step, the channel's construction now pending.
CreateRuntime(rtId, chId) ==
    /\ runtime_dispose_state = "absent"
    /\ channel_dispose_state[chId] = "unopened"
    /\ F!RuntimeCreate(rtId)
    /\ runtime_root_live' = TRUE
    /\ current_runtime' = rtId
    /\ runtime_dispose_state' = "active"
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "constructing"]
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* A later channel's constructor borrows the materialized runtime: the
\* lease is taken under the factory's lock, no native step.
AcquireLease(chId) ==
    /\ runtime_dispose_state = "active"
    /\ channel_dispose_state[chId] = "unopened"
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "constructing"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* ak_channel_create on the current runtime: the constructor returns and
\* the channel object is exposed.  Binding-owed - a constructor that
\* began completes.
CreateChannel(chId) ==
    /\ channel_dispose_state[chId] = "constructing"
    /\ current_runtime # "none"
    /\ F!ChannelCreate(chId, current_runtime)
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "active"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* ak_channel_create refused the configuration.  It performs no I/O, so it
\* fails only on a bad config or a stale runtime handle - local errors,
\* not runtime failures: a typo in an endpoint may not kill the
\* process-wide runtime every other channel leases.  The lease goes back,
\* and a first channel that fails takes its just-materialized generation
\* with it, retired rather than left acquirable - the constructor's own
\* local resources go, while the shared RuntimeState's root lives until
\* that destroy.  Only an allocation
\* failure is a runtime failure, and that is F!RuntimeFail's business.
RejectChannelCreation(chId) ==
    /\ channel_dispose_state[chId] = "constructing"
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "rejected"]
    /\ IF \A other \in ChannelIds :
              other # chId => ChannelSettled(other)
       THEN runtime_dispose_state' = "shutdown_pending"
       ELSE UNCHANGED runtime_dispose_state
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* GrpcChannel.DisposeAsync: remembered at once; the binding then settles
\* this channel's own calls and no one else's.
BeginDisposeChannel(chId) ==
    /\ channel_dispose_state[chId] = "active"
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "disposing"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* ak_channel_release once every owned call is settled: the channel's
\* native half closes and the lease is gone.  A channel the runtime
\* already latched to closing needs no downcall of its own.
FinishDisposeChannel(chId) ==
    /\ channel_dispose_state[chId] = "disposing"
    /\ \A c \in CallIds :
           /\ call_channel[c] = chId
           => call_dispose_state[c] = "settled"
    /\ \/ F!ChannelStartClosing(chId)
       \/ /\ channel_state[chId] \in {"closing", "closed"}
          /\ UNCHANGED l1_vars
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT
                ![chId] = IF IsLastRelease(chId) THEN "released_last"
                          ELSE "released"]
    /\ runtime_dispose_state' =
           IF IsLastRelease(chId) THEN "shutdown_pending"
           ELSE runtime_dispose_state
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The public DisposeAsync completes.  For a channel that was not the
\* last it completes with its own release; for the last one it waits for
\* the destroy it triggered, which is what its task promised.
ResolveChannelDispose(chId) ==
    /\ ChannelDisposeMayResolve(chId)
    /\ channel_dispose_state' =
           [channel_dispose_state EXCEPT ![chId] = "disposed"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The latch is set: the factory begins the native shutdown.  No lease
\* can have been taken since, AcquireLease requiring an active manager.
BeginRuntimeShutdown(rtId) ==
    /\ runtime_dispose_state = "shutdown_pending"
    /\ rtId = current_runtime
    /\ F!RuntimeBeginShutdown(rtId)
    /\ runtime_dispose_state' = "destroying"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, channel_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* ak_runtime_destroy returns AK_STATUS_OK for the current generation.
FinishDisposeRuntime(rtId) ==
    /\ runtime_dispose_state = "destroying"
    /\ rtId = current_runtime
    /\ F!RuntimeDestroy(rtId)
    /\ runtime_dispose_state' = "destroyed"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, channel_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The generation's root dies after destroy, later than every callback of
\* every kind - and the factory re-arms: a fresh generation may follow.
FreeRuntimeRoot ==
    /\ runtime_root_live
    /\ runtime_dispose_state = "destroyed"
    /\ \A rtId \in RuntimeIds :
           /\ ~shutdown_callback_running[rtId]
           /\ ~resources_released_callback_running[rtId]
    /\ \A cId \in CallIds :
           /\ ~delivery_callback_running[cId]
           /\ ~write_done_callback_running[cId]
    /\ runtime_root_live' = FALSE
    /\ current_runtime' = "none"
    /\ runtime_dispose_state' = "absent"
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live,
                   channel_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

(***************************************************************************)
(* THE CALLS - construction, reader, dispose.                              *)
(***************************************************************************)

\* The call constructor: GCHandle.Alloc, ak_call_start on an active
\* channel, and only then the object returns to the user.
StartCall(cId, chId) ==
    /\ ~call_token_published[cId]
    /\ BindingMayDowncall(cId)
    /\ channel_dispose_state[chId] = "active"
    /\ F!CallStart(cId, chId)
    /\ call_token_published' = [call_token_published EXCEPT ![cId] = TRUE]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = TRUE]
    /\ UNCHANGED <<runtime_root_live, current_runtime,
                   runtime_dispose_state, channel_dispose_state,
                   consumer_phase, reader_state, writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* MoveNext called: the reader is committed, payload or not.
BeginMoveNext(cId) ==
    /\ call_token_published[cId]
    /\ consumer_phase[cId] \in {"prologue", "application"}
    /\ call_dispose_state[cId] = "active"
    /\ reader_state[cId] = "idle"
    /\ reader_state' = [reader_state EXCEPT ![cId] = "waiting"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The suspended MoveNext wakes and takes the event at the ring's tail: a
\* message, or the terminal one carrying the status and the trailers -
\* hence "event" and not "message".  A dispose that linearized first
\* wins.  The phase is what keeps the application off slot 0: during the
\* prologue the ring's only occupant is the metadata, which is
\* ConsumeHeader's and never something a public read may return.
BeginParseEvent(cId) ==
    /\ reader_state[cId] = "waiting"
    /\ consumer_phase[cId] = "application"
    /\ call_dispose_state[cId] = "active"
    /\ RingOccupancy(cId) > 0
    /\ reader_state' = [reader_state EXCEPT ![cId] = "parsing"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* Everything this level holds about one call, as a single term.  The
\* isolation theorem compares this rather than listing components, because
\* a list narrows what the theorem claims every time a variable is added
\* to the level, and nothing warns of it.  Level 1's cancellation latch
\* belongs here too, being per call and settable by a dispose.
ManagedCallState(cId) ==
    <<call_token_published[cId], call_root_live[cId],
      consumer_phase[cId], reader_state[cId], read_cancel_pending[cId],
      writer_state[cId], retry_len[cId],
      headers_completion[cId], status_completion[cId],
      call_dispose_state[cId], cancel_requested[cId]>>

\* The consumed slot is the terminal one exactly when it is the last
\* published event of a call that has its status.
ConsumingTerminal(cId) ==
    /\ F!L0!HasStatus(cId)
    /\ RingTail(cId) = RingHead(cId) - 1

\* A posted request is discharged by the binding's reaction and by
\* nothing else while the call is live.  The escape is not a weakening:
\* once the call has left "active" something already cancelled it or it
\* had already ended, so the request has nothing left to obtain and the
\* completion may carry it away.  Without this, a read finishing normally
\* would swallow a token that linearized before the end of the read -
\* losing the cancellation IAsyncStreamReader promises to honour.
ReadCancellationSettled(cId) ==
    \/ ~read_cancel_pending[cId]
    \/ call_dispose_state[cId] # "active"

\* The parse completes: the consumer decodes the slot - status and
\* trailers included when it is the terminal - resolves what it answers,
\* then releases it.  ak_event_consumed under the reader's right.  A
\* consumed terminal leaves the reader finished, the stable fact that the
\* stream ended: every later MoveNext answers from it at once, false when
\* the status is OK and an RpcException when it is not.  Either answer is
\* fixed and touches no state, managed or native, which is why no such
\* read appears in this model.
FinishConsumePayload(cId) ==
    /\ reader_state[cId] = "parsing"
    /\ ReadCancellationSettled(cId)
    /\ F!HostConsumesEvent(cId)
    /\ reader_state' =
           [reader_state EXCEPT
                ![cId] = IF ConsumingTerminal(cId) THEN "finished"
                         ELSE "idle"]
    /\ read_cancel_pending' = [read_cancel_pending EXCEPT ![cId] = FALSE]
    /\ status_completion' =
           [status_completion EXCEPT
                ![cId] = IF ConsumingTerminal(cId) THEN "resolved" ELSE @]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len,
                   headers_completion, call_dispose_state>>

\* MoveNext's token fires.  The environment's step, not the binding's: it
\* carries no fairness, because a token that never fires is the normal
\* case and no promise may turn a possibility into an obligation.  The
\* request belongs to the read in flight and dies with it, so a token
\* firing after its own read completed finds no read to arm and does
\* nothing at all - the other half of IAsyncStreamReader's contract, held
\* by the flag's lifetime rather than by an epoch.
RequestReadCancellation(cId) ==
    /\ ReadInFlight(cId)
    /\ ~read_cancel_pending[cId]
    /\ call_dispose_state[cId] = "active"
    /\ read_cancel_pending' = [read_cancel_pending EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   writer_state, retry_len,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The binding reacts to a request on a suspended read: the read resolves
\* exceptionally and the CALL is cancelled - the contract cancels the
\* call, not the read alone - and the call goes to its drain in the same
\* step, so no further user action is needed to settle it.  A read
\* suspended in the prologue is waiting on the metadata through
\* EnsureHeadersAsync, so its cancellation faults the headers too: that
\* task can have no other outcome once the call is cancelled, and leaving
\* it pending would strand the settlement.
CancelWaitingRead(cId) ==
    /\ read_cancel_pending[cId]
    /\ reader_state[cId] = "waiting"
    /\ call_dispose_state[cId] = "active"
    /\ F!RequestCallCancellation(cId)
    /\ reader_state' = [reader_state EXCEPT ![cId] = "idle"]
    /\ read_cancel_pending' = [read_cancel_pending EXCEPT ![cId] = FALSE]
    /\ call_dispose_state' =
           [call_dispose_state EXCEPT ![cId] = "draining"]
    /\ headers_completion' =
           [headers_completion EXCEPT
                ![cId] = IF @ = "pending" THEN "failed" ELSE @]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len, status_completion>>

\* A request that lands on a parse cannot preempt it: a synchronous
\* marshaller already writing is not interruptible, so the slot stays
\* this reader's until it returns.  The call is cancelled at once; the
\* payload is not abandoned.
CancelParsingRead(cId) ==
    /\ read_cancel_pending[cId]
    /\ reader_state[cId] = "parsing"
    /\ call_dispose_state[cId] = "active"
    /\ F!RequestCallCancellation(cId)
    /\ reader_state' = [reader_state EXCEPT ![cId] = "parsing_cancelled"]
    /\ read_cancel_pending' = [read_cancel_pending EXCEPT ![cId] = FALSE]
    /\ call_dispose_state' =
           [call_dispose_state EXCEPT ![cId] = "draining"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len,
                   headers_completion, status_completion>>

\* The cancelled parse returns: its slot is released exactly once, here
\* and nowhere else, and the reader is done.  When the slot it held was
\* the terminal one, the status is decoded and kept all the same: the
\* read's own result is exceptional because its token won, but GetStatus,
\* the drain and the settlement all need that status, and once this slot
\* is gone no other consumer can decode it.
FinishCancelledParse(cId) ==
    /\ reader_state[cId] = "parsing_cancelled"
    /\ F!HostConsumesEvent(cId)
    /\ reader_state' =
           [reader_state EXCEPT
                ![cId] = IF ConsumingTerminal(cId) THEN "finished"
                         ELSE "idle"]
    /\ status_completion' =
           [status_completion EXCEPT
                ![cId] = IF ConsumingTerminal(cId) THEN "resolved" ELSE @]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   read_cancel_pending, writer_state, retry_len,
                   headers_completion, call_dispose_state>>

\* A waiter caught by the dispose resolves exceptionally.
CancelWaiter(cId) ==
    /\ reader_state[cId] = "waiting"
    /\ call_dispose_state[cId] # "active"
    /\ reader_state' = [reader_state EXCEPT ![cId] = "idle"]
    /\ read_cancel_pending' = [read_cancel_pending EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase,
                   writer_state, retry_len,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The drain takes the ring once no read is outstanding - idle, or
\* finished on the consumed terminal.  Both hold nothing; only a parse in
\* flight or a suspended MoveNext makes the drain wait.
HandoffToDrain(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] \in {"prologue", "application"}
    /\ reader_state[cId] \in {"idle", "finished"}
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "drain"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion,
                   call_dispose_state>>

\* The prologue owns slot 0: it releases the header, resolves the headers
\* completion, and hands the ring to the application.
ConsumeHeader(cId) ==
    /\ consumer_phase[cId] = "prologue"
    /\ call_dispose_state[cId] = "active"
    /\ RingTail(cId) = 0
    /\ F!HostConsumesEvent(cId)
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "application"]
    /\ headers_completion' =
           [headers_completion EXCEPT ![cId] = "succeeded"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending,
                   status_completion, call_dispose_state>>

\* DisposeAsync on a call - also fired by its channel's dispose.  Latches
\* cancellation if the call can still take one, and faults a headers task
\* still pending: no managed waiter survives a dispose.
BeginDisposeCall(cId) ==
    /\ call_token_published[cId]
    /\ call_dispose_state[cId] = "active"
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "draining"]
    /\ headers_completion' =
           [headers_completion EXCEPT
                ![cId] = IF @ = "pending" THEN "failed" ELSE @]
    /\ \/ F!RequestCallCancellation(cId)
       \/ /\ \/ ~F!L0!IsActiveCall(cId)
             \/ cancel_requested[cId]
          /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending,
                   status_completion>>

\* A disposing channel settles its own calls - and no one else's:
\* ownership is call_channel, the level-0 relation.
DisposeCallForChannel(cId) ==
    /\ call_channel[cId] \in ChannelIds
    /\ channel_dispose_state[call_channel[cId]] = "disposing"
    /\ BeginDisposeCall(cId)

\* The drain releases what is left, from tail up, terminal last - and
\* the terminal is parsed and resolved before its owner is given back:
\* the dispose never depends on bytes it already returned.
DrainRelease(cId) ==
    /\ consumer_phase[cId] = "drain"
    /\ F!HostConsumesEvent(cId)
    /\ status_completion' =
           [status_completion EXCEPT
                ![cId] = IF ConsumingTerminal(cId) THEN "resolved" ELSE @]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, call_dispose_state>>

\* The drain finished, the writer settled, the status resolved by the
\* consumer that parsed the terminal: the call is disposed, and a dispose
\* leaves no managed waiter of any kind.
FinishDisposeCall(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] = "drain"
    /\ RingDrained(cId)
    /\ F!L0!HasStatus(cId)
    /\ writer_state[cId] \in {"idle", "closed"}
    /\ status_completion[cId] = "resolved"
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "settled"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "done"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending, headers_completion,
                   status_completion>>

\* The call is over and owes nothing, so it settles - no user step, and no
\* Dispose: for a normally finished call the .NET API says disposing does
\* nothing, so demanding it would be a discipline stricter than the
\* surface this binding implements.  The last two conjuncts are exactly
\* what F!ReleaseCallHandle waits on, so this settlement is the condition
\* that unblocks the native reclamation rather than a parallel state
\* ignoring it.
SettleCall(cId) ==
    /\ call_dispose_state[cId] = "active"
    /\ call_token_published[cId]
    /\ F!L0!IsTerminalCall(cId)
    /\ RingDrained(cId)
    /\ reader_state[cId] = "finished"
    /\ writer_state[cId] \in {"idle", "closed"}
    /\ status_completion[cId] = "resolved"
    /\ F!HostOwnsNoPayload(cId)
    /\ F!HostHoldsNoBuffer(cId)
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "settled"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "done"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending,
                   headers_completion, status_completion>>

(***************************************************************************)
(* THE WRITER.  WriteAsync begins with the lend; a write completes at its  *)
(* WRITE_DONE; serializing is the only state holding a buffer;             *)
(* MESSAGE_TOO_LARGE faults synchronously and enters no wait.              *)
(***************************************************************************)

\* The lend succeeds at once: the marshaller serializes into the buffer.
WriteLendSucceeds(cId, b, len, charge) ==
    /\ writer_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ F!LendSendBuffer(cId, b, len, charge)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "serializing"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   retry_len,
                   read_cancel_pending, headers_completion, status_completion,
                   call_dispose_state>>

\* BUDGET_BUSY: the cancellable wait, remembering the refused length.
WriteRefusedBudget(cId, len, charge) ==
    /\ writer_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ ~cancel_requested[cId]
    /\ F!RefuseLendForBudget(cId, len, charge)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "waiting_budget"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = len]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   headers_completion, status_completion,
                   call_dispose_state, read_cancel_pending>>

\* MESSAGE_TOO_LARGE: the write faults synchronously - permanent refusal,
\* no wait, no retry.
WriteRefusedTooLarge(cId, len) ==
    /\ writer_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ F!RefuseLendTooLarge(cId, len)
    /\ ManagedStutter

\* The budget wait's retry succeeds: the window is always open, so a
\* successful lend is the only way out besides cancel and dispose.
RetryLendSucceeds(cId, b, charge) ==
    /\ writer_state[cId] = "waiting_budget"
    /\ BindingMayDowncall(cId)
    /\ F!LendSendBuffer(cId, b, retry_len[cId], charge)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "serializing"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = NoRetryLen]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   headers_completion, status_completion,
                   call_dispose_state, read_cancel_pending>>

\* Serialization completed: the commit is accepted and the write is in
\* flight, its task to be completed by WRITE_DONE.
CommitWrite(cId, msg, b) ==
    /\ writer_state[cId] = "serializing"
    /\ BindingMayDowncall(cId)
    /\ F!SendMessage(cId, msg, b)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "awaiting_write_done"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   retry_len,
                   read_cancel_pending, headers_completion, status_completion,
                   call_dispose_state>>

\* Serialization threw, or the commit is refused (cancellation latched):
\* the disposable wrapper returns the buffer and the write faults.
WriteAborted(cId, b) ==
    /\ writer_state[cId] = "serializing"
    /\ F!HostReturnsBuffer(cId, b)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "idle"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   retry_len,
                   read_cancel_pending, headers_completion, status_completion,
                   call_dispose_state>>

\* A waiting write caught by cancellation or dispose resolves
\* exceptionally, like the waiting reader.
CancelWriterWait(cId) ==
    /\ writer_state[cId] = "waiting_budget"
    /\ \/ cancel_requested[cId]
       \/ ~F!L0!IsActiveCall(cId)
       \/ call_dispose_state[cId] # "active"
       \/ runtime_dispose_state # "active"
    /\ writer_state' = [writer_state EXCEPT ![cId] = "idle"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = NoRetryLen]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   headers_completion, status_completion,
                   call_dispose_state, read_cancel_pending>>

\* The WRITE_DONE callback returns: the write in flight - there is at
\* most one, the writer being single - completes its task.
WriteDoneCompletes(cId) ==
    /\ F!WriteDoneReturns(cId)
    /\ writer_state' =
           [writer_state EXCEPT
                ![cId] = IF @ = "awaiting_write_done" THEN "idle" ELSE @]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   retry_len,
                   read_cancel_pending, headers_completion, status_completion,
                   call_dispose_state>>

\* CompleteAsync: end_send, legal only beside no pending write.
CloseWriter(cId) ==
    /\ writer_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ F!EndSend(cId)
    /\ writer_state' = [writer_state EXCEPT ![cId] = "closed"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   retry_len,
                   read_cancel_pending, headers_completion, status_completion,
                   call_dispose_state>>

(***************************************************************************)
(* CALLBACK RETURNS AND ROOTS                                              *)
(***************************************************************************)

\* A non-terminal delivery callback returns after publishing the slot and
\* completing the TCS the event answers.
OnEventReturns(cId) ==
    /\ ~F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ ManagedStutter

\* The terminal callback's return: the call root's last access frees it.
\* It resolves nothing - the callback publishes without decoding, and the
\* status payload (code, message, trailers) is the terminal consumer's to
\* parse.
TerminalCallbackReturns(cId) ==
    /\ F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<call_token_published, runtime_root_live,
                   current_runtime, runtime_dispose_state,
                   channel_dispose_state, consumer_phase, reader_state,
                   writer_state, retry_len,
                   read_cancel_pending, headers_completion,
                   status_completion, call_dispose_state>>

\* The runtime-level callbacks return without touching managed call state.
ShutdownReturns(rtId) ==
    /\ F!ShutdownCallbackReturns(rtId)
    /\ ManagedStutter

ResourcesReleasedReturns(rtId) ==
    /\ F!ResourcesReleasedCallbackReturns(rtId)
    /\ ManagedStutter

(***************************************************************************)
(* PASSTHROUGHS - the runtime's own steps, undecorated.  Every binding     *)
(* downcall is coupled above; nothing lends, sends, closes or releases     *)
(* outside the machines.                                                   *)
(***************************************************************************)

RuntimeSteps ==
    \/ \E rtId \in RuntimeIds :
           \/ F!EmitShutdownComplete(rtId)
           \/ F!EmitResourcesReleased(rtId)
           \/ F!RuntimeRelease(rtId)
           \/ F!RuntimeFail(rtId)
           \/ F!RemainFailed(rtId)
    \/ F!RemainReleased
    \/ \E chId \in ChannelIds : F!ChannelFinishClosing(chId)
    \/ \E cId \in CallIds :
           \/ F!EmitWriteDone(cId)
           \/ F!NetworkSend(cId)
           \/ F!ReceiveStatus(cId)
           \/ F!DeliverInitialMetadata(cId)
           \/ F!DeliverMessage(cId)
           \/ F!DeliverStatus(cId)
           \/ F!DeliverCancelled(cId)
           \/ F!ReleaseCallHandle(cId)
    \/ \E cId \in CallIds, msg \in Messages : F!NetworkReceive(cId, msg)
    \/ \E cId \in CallIds, b \in BufferIds : F!FreeReturnedBuffer(cId, b)

BindingDowncalls ==
    \E cId \in CallIds :
        /\ BindingMayDowncall(cId)
        /\ F!RequestCallCancellation(cId)

Passthrough == (RuntimeSteps \/ BindingDowncalls) /\ ManagedStutter

(***************************************************************************)
(* NEXT                                                                    *)
(***************************************************************************)

Next ==
    \/ Passthrough
    \/ FreeRuntimeRoot
    \/ \E rtId \in RuntimeIds, chId \in ChannelIds :
           CreateRuntime(rtId, chId)
    \/ \E chId \in ChannelIds :
           \/ AcquireLease(chId)
           \/ CreateChannel(chId)
           \/ RejectChannelCreation(chId)
           \/ BeginDisposeChannel(chId)
           \/ FinishDisposeChannel(chId)
           \/ ResolveChannelDispose(chId)
    \/ \E rtId \in RuntimeIds :
           \/ BeginRuntimeShutdown(rtId)
           \/ FinishDisposeRuntime(rtId)
           \/ ShutdownReturns(rtId)
           \/ ResourcesReleasedReturns(rtId)
    \/ \E cId \in CallIds :
           \/ BeginMoveNext(cId)
           \/ BeginParseEvent(cId)
           \/ FinishConsumePayload(cId)
           \/ CancelWaiter(cId)
           \/ RequestReadCancellation(cId)
           \/ CancelWaitingRead(cId)
           \/ CancelParsingRead(cId)
           \/ FinishCancelledParse(cId)
           \/ HandoffToDrain(cId)
           \/ ConsumeHeader(cId)
           \/ BeginDisposeCall(cId)
           \/ DisposeCallForChannel(cId)
           \/ DrainRelease(cId)
           \/ FinishDisposeCall(cId)
           \/ SettleCall(cId)
           \/ CancelWriterWait(cId)
           \/ WriteDoneCompletes(cId)
           \/ CloseWriter(cId)
           \/ OnEventReturns(cId)
           \/ TerminalCallbackReturns(cId)
    \/ \E cId \in CallIds, chId \in ChannelIds : StartCall(cId, chId)
    \/ \E cId \in CallIds, b \in BufferIds,
         len \in F!Sizes, charge \in F!Sizes :
           WriteLendSucceeds(cId, b, len, charge)
    \/ \E cId \in CallIds, len \in F!Sizes, charge \in F!CandidateCharges :
           WriteRefusedBudget(cId, len, charge)
    \/ \E cId \in CallIds, len \in F!RequestLengths :
           WriteRefusedTooLarge(cId, len)
    \/ \E cId \in CallIds, b \in BufferIds, charge \in F!Sizes :
           RetryLendSucceeds(cId, b, charge)
    \/ \E cId \in CallIds, msg \in Messages, b \in BufferIds :
           CommitWrite(cId, msg, b)
    \/ \E cId \in CallIds, b \in BufferIds : WriteAborted(cId, b)

(***************************************************************************)
(* FAIRNESS - three tiers.  The runtime's and the FFI dispatch's thirteen  *)
(* families are taken verbatim from level 1; the binding's tier derives    *)
(* the six host conjuncts and completes what it began; the application's   *)
(* tier is one conjunct per call.  Serialization settling is disjunction   *)
(* fairness: which branch fires is the allocator's or the marshaller's     *)
(* business, that one fires is the binding's promise - under the stated    *)
(* hypothesis that user code terminates.  The budget wait is not a         *)
(* disjunction: CancelWriterWait is one action whose guard names several   *)
(* causes, and nothing promises a successful retry, so no fairness here    *)
(* implies the budget ever becomes available.  No slot wait exists at all: *)
(* a write completes at its WRITE_DONE, so the next lend always finds the  *)
(* window open, which ManagedWriterNeverObservesSlotBusy states.           *)
(***************************************************************************)

SerializationSettles(cId) ==
    \/ \E msg \in Messages, b \in BufferIds : CommitWrite(cId, msg, b)
    \/ \E b \in BufferIds : WriteAborted(cId, b)

RuntimeOwedFairness ==
    \* accepted sends reach the wire, so a committed write can be acquitted
    /\ \A cId \in CallIds : WF_l1_vars(F!NetworkSend(cId))
    \* a terminal arrives at all, so every call has an end to deliver
    /\ \A cId \in CallIds : WF_l1_vars(F!ReceiveStatus(cId))
    \* the header reaches the ring, so the prologue can resolve the headers
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverInitialMetadata(cId))
    \* a received message reaches the ring, so a waiting reader wakes
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverMessage(cId))
    \* the terminal reaches the ring, so the reader or the drain can end
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverStatus(cId))
    \* a cancelled call still gets its terminal, so its dispose can finish
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverCancelled(cId))
    \* the acquittal comes, which is where a write completes
    /\ \A cId \in CallIds : WF_l1_vars(F!EmitWriteDone(cId))
    \* the runtime reaches released, without which destroy is refused
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!RuntimeRelease(rtId))
    \* the shutdown announces itself, the first link of the teardown chain
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!EmitShutdownComplete(rtId))
    \* the second event when owed - never at this level, kept for the lift
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!EmitResourcesReleased(rtId))
    \* a closing channel closes, so its calls end and its lease can go
    /\ \A chId \in ChannelIds : WF_l1_vars(F!ChannelFinishClosing(chId))
    \* returned bytes are freed, which is what recredits the byte budget
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_l1_vars(F!FreeReturnedBuffer(cId, b))
    \* a settled call is reclaimed, so its arena goes with it
    /\ \A cId \in CallIds : WF_l1_vars(F!ReleaseCallHandle(cId))

BindingOwedFairness ==
    \* THE SIX HOST HYPOTHESES, in level 1's own tuple.  Stated this way
    \* on purpose: a weak fairness over l1_vars is what F!Fairness asks
    \* for, so the discharge is a citation and no enabling bridge is
    \* needed - and none could be built, TLAPS being unable to expand an
    \* ENABLED whose action reaches through an instance.  Nothing is
    \* weakened: each of these level-1 actions occurs in this model only
    \* inside the coupled action named beside it, which does the managed
    \* half in the same step, so demanding the level-1 action demands the
    \* whole step.
    \* OnEventReturns and TerminalCallbackReturns: the trampoline returns
    \* after bounded work, the terminal one freeing the root as it goes
    /\ \A cId \in CallIds :
           WF_l1_vars(F!DeliveryCallbackReturns(cId))
    \* WriteDoneCompletes: a counter and a signal, then the write's task
    /\ \A cId \in CallIds : WF_l1_vars(F!WriteDoneReturns(cId))
    \* ShutdownReturns: nothing but a signal
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!ShutdownCallbackReturns(rtId))
    \* ResourcesReleasedReturns: the same, for the event this level never
    \* owes
    /\ \A rtId \in RuntimeIds :
           WF_l1_vars(F!ResourcesReleasedCallbackReturns(rtId))
    \* ConsumeHeader, FinishConsumePayload, DrainRelease: whichever of the
    \* three the phase admits releases the slot - the last two under the
    \* stated hypothesis that user parsing terminates
    /\ \A cId \in CallIds : WF_l1_vars(F!HostConsumesEvent(cId))
    \* WriteAborted: the disposable wrapper gives the buffer back, on the
    \* serializing state's exception path as on its refusal path - and
    \* serializing is the only state that holds one, so this is the only
    \* return there is
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_l1_vars(F!HostReturnsBuffer(cId, b))

    \* THE BINDING'S OWN MACHINERY, in this level's tuple: steps that move
    \* managed state, so nothing below asks for them and only this level
    \* can promise them.
    \* wakes a suspended MoveNext once a payload exists; true: the TCS
    \* completion is the binding's, and the pool runs it
    /\ \A cId \in CallIds : WF_vars(BeginParseEvent(cId))
    \* resolves a waiter caught by a dispose; true: the binding cancels it
    /\ \A cId \in CallIds : WF_vars(CancelWaiter(cId))
    \* a request that landed is acted on - the trigger itself carries no
    \* fairness, so nothing here obliges a cancellation to happen
    /\ \A cId \in CallIds : WF_vars(CancelWaitingRead(cId))
    /\ \A cId \in CallIds : WF_vars(CancelParsingRead(cId))
    /\ \A cId \in CallIds : WF_vars(FinishCancelledParse(cId))
    \* gives the drain the ring, without which a dispose cannot end;
    \* true: the binding's own step once no read is outstanding
    /\ \A cId \in CallIds : WF_vars(HandoffToDrain(cId))
    \* the call reaches settled, so its channel may release its lease;
    \* true: the binding's own step once the drain and writer are settled
    /\ \A cId \in CallIds : WF_vars(FinishDisposeCall(cId))
    \* a finished call settles with no user step: the .NET API does not
    \* require Dispose of a completed call, so neither does this model
    /\ \A cId \in CallIds : WF_vars(SettleCall(cId))
    \* a disposing channel settles the calls it owns; true: its own loop
    /\ \A cId \in CallIds : WF_vars(DisposeCallForChannel(cId))
    \* resolves a writer caught by a cancel or a dispose; true: the
    \* binding faults the pending write, it waits for nothing
    /\ \A cId \in CallIds : WF_vars(CancelWriterWait(cId))
    \* the serializing state ends, by commit or by the wrapper's abort;
    \* true: under the stated hypothesis that the marshaller terminates
    /\ \A cId \in CallIds : WF_vars(SerializationSettles(cId))
    \* a constructor that began completes; true: one downcall, no wait
    \* the downcall owes a RESULT, not a success: a refused configuration
    \* is an answer, and the constructor's task ends either way
    /\ \A chId \in ChannelIds :
           WF_vars(CreateChannel(chId) \/ RejectChannelCreation(chId))
    \* the lease goes back, without which no teardown starts; true: the
    \* binding's own step once the channel's calls are settled
    /\ \A chId \in ChannelIds : WF_vars(FinishDisposeChannel(chId))
    \* the public DisposeAsync task completes; true: the binding's own
    \* step as soon as its guard holds
    /\ \A chId \in ChannelIds : WF_vars(ResolveChannelDispose(chId))
    \* the teardown starts at the latch; true: the binding's own downcall
    /\ \A rtId \in RuntimeIds : WF_vars(BeginRuntimeShutdown(rtId))
    \* destroy returns, which is what dispose promised; true: the
    \* binding's own downcall once the runtime reached quiescence
    /\ \A rtId \in RuntimeIds : WF_vars(FinishDisposeRuntime(rtId))
    \* the root dies and the factory re-arms; true: the binding's own step
    \* once no callback of any kind is in flight
    /\ WF_vars(FreeRuntimeRoot)

\* What the application owes, per call: one conjunct, and only while the
\* response stream is still readable - read the next response, or dispose
\* the call early.  Nothing is asked once the terminal has been consumed:
\* a finished call settles by itself, so Dispose is not required, which
\* is what the .NET API says of a completed call.  The one case this
\* leaves to hypothesis is an application that abandons a readable
\* stream, neither reading nor disposing - the misuse level 1 already
\* assumes away with its own consumption fairness.
ApplicationOwedFairness ==
    \* needed so a live call moves at all: the reader's liveness and the
    \* consumption discharge both start from a read the application began.
    \* Not true of every program - assumed of a conforming one
    /\ \A cId \in CallIds :
           WF_vars(BeginMoveNext(cId) \/ BeginDisposeCall(cId))

Fairness ==
    /\ RuntimeOwedFairness
    /\ BindingOwedFairness
    /\ ApplicationOwedFairness

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* INVARIANTS                                                              *)
(***************************************************************************)

\* The GCHandle exists from the same step as ak_call_start.
TokenPublishedBeforeStart ==
    \A cId \in CallIds :
        ~F!L0!IsUnusedCall(cId) => call_token_published[cId]

\* Every call callback in flight resolves its call_ctx to a live root.
RootSurvivesCallbacks ==
    \A cId \in CallIds :
        \/ delivery_callback_running[cId]
        \/ write_done_callback_running[cId]
        => call_root_live[cId]

\* Every callback of every kind keeps the shared RuntimeState's root
\* alive - the call callbacks too.
RuntimeRootSurvivesCallbacks ==
    /\ \A rtId \in RuntimeIds :
           \/ shutdown_callback_running[rtId]
           \/ resources_released_callback_running[rtId]
           => runtime_root_live
    /\ \A cId \in CallIds :
           \/ delivery_callback_running[cId]
           \/ write_done_callback_running[cId]
           => runtime_root_live

\* The phase machine and the dispose machine agree.
ConsumerPhaseMatchesDispose ==
    \A cId \in CallIds :
        /\ consumer_phase[cId] = "done" <=>
               call_dispose_state[cId] = "settled"
        /\ consumer_phase[cId] = "drain" =>
               call_dispose_state[cId] = "draining"
        /\ call_dispose_state[cId] = "active" =>
               consumer_phase[cId] \in {"prologue", "application"}

\* An outstanding read exists only while a consumer of the application's
\* own is on the ring; one value per call is what makes it unique,
\* matching IAsyncStreamReader's single-read contract.  The prologue
\* counts: MoveNext may be the first thing an application calls, and the
\* wait for the metadata is part of that read rather than something
\* preceding it - which is what makes a token firing then belong to a
\* read the model can see.  A cancelled parse is still outstanding too,
\* since it holds a slot, so this is stated over ReadInFlight rather than
\* over a list of states of its own.  Finished is not outstanding: it is
\* the stable fact that the stream ended, and it survives the hand-off
\* and the dispose.
AtMostOneReaderOutstanding ==
    \A cId \in CallIds :
        ReadInFlight(cId) =>
            consumer_phase[cId] \in {"prologue", "application"}

\* The drain never runs beside an application read.
DrainNeverOverlapsApplicationConsumer ==
    \A cId \in CallIds :
        consumer_phase[cId] \in {"drain", "done"} =>
            reader_state[cId] \in {"idle", "finished"}

\* A waiting writer holds no lent buffer; the serializing one holds
\* exactly the one it was lent - waiting while holding is the deadlock
\* forbidden at the door.
WaitingWriterHoldsNoBuffer ==
    \A cId \in CallIds :
        writer_state[cId] = "waiting_budget" =>
            buffers_held_by_host[cId] = 0

\* The conformant surface never meets SLOT_BUSY: a write completes at its
\* WRITE_DONE, whose emission already freed the slot, so the next lend
\* always finds the window open.  RefuseLendForSlot is dead by design at
\* this level - a SLOT_BUSY reaching the binding would be a defect, and
\* this invariant is the claim.
ManagedWriterNeverObservesSlotBusy ==
    \A cId \in CallIds : last_lend_status[cId] # "SLOT_BUSY"

SerializingWriterHoldsTheBuffer ==
    \A cId \in CallIds :
        /\ writer_state[cId] = "serializing" =>
               buffers_held_by_host[cId] = 1
        /\ writer_state[cId] # "serializing" =>
               buffers_held_by_host[cId] = 0

\* The wait shows the refusal that caused it.
WaitMatchesRefusal ==
    \A cId \in CallIds :
        writer_state[cId] = "waiting_budget" =>
            last_lend_status[cId] = "BUDGET_BUSY"

\* The remembered length exists exactly while a wait is in progress.
RetryLenMatchesWait ==
    \A cId \in CallIds :
        writer_state[cId] = "waiting_budget" <=>
            retry_len[cId] # NoRetryLen

\* Dispose completed means ak_runtime_destroy returned - for the CURRENT
\* generation, not some earlier one.
DisposeAwaitsDestroy ==
    runtime_dispose_state = "destroyed" =>
        /\ current_runtime # "none"
        /\ runtime_destroyed[current_runtime]

\* The manager's own state is coherent: a materialized generation has an
\* identity and a root, an absent one has neither.
RuntimeManagerCoherent ==
    /\ (runtime_dispose_state = "absent")
           <=> (current_runtime = "none" /\ ~runtime_root_live)
    /\ (runtime_dispose_state # "absent")
           => (current_runtime \in RuntimeIds /\ runtime_root_live)

\* A live channel hangs off the current generation, never an earlier
\* destroyed one.
LiveChannelUsesCurrentRuntime ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] \in {"active", "disposing"} =>
            /\ current_runtime \in RuntimeIds
            /\ channel_runtime[chId] = current_runtime

\* The level-2 dispose discipline settles every debt before the last
\* release, so the shutdown never owes the second event: the runtime
\* finds no host debt when it latches the tag.
ManagedShutdownHasNoHostDebt ==
    \A rtId \in RuntimeIds :
        shutdown_event_emitted[rtId] => ~second_event_owed[rtId]

\* A channel with a live lease keeps the runtime alive.
LiveChannelKeepsRuntimeAlive ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] \in
            {"constructing", "active", "disposing"} =>
                runtime_dispose_state = "active"

\* The native shutdown never starts while any lease is out.
NoRuntimeShutdownWhileLeased ==
    runtime_dispose_state \in
        {"shutdown_pending", "destroying", "destroyed"} =>
            AllLeasesReleased

\* The channel machine and the native channel agree, state by state: an
\* unbuilt channel has no native half, an exposed one is open, a
\* disposing one is open or already latched closing by the runtime, and
\* a released one is closing or closed - never open, the release having
\* closed it.
ChannelStateMatchesNative ==
    \A chId \in ChannelIds :
        /\ channel_dispose_state[chId] \in
               {"unopened", "constructing", "rejected"} =>
                   channel_state[chId] = "none"
        /\ channel_dispose_state[chId] = "active" =>
               channel_state[chId] = "open"
        /\ channel_dispose_state[chId] = "disposing" =>
               channel_state[chId] \in {"open", "closing", "closed"}
        /\ channel_dispose_state[chId] \in
               {"released", "released_last", "disposed"} =>
                   channel_state[chId] \in {"closing", "closed"}

\* A rejected channel never got its native half, and holds no lease -
\* ChannelSettled says the second, this says the first.
RejectedChannelHasNoNativeHalf ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] = "rejected" =>
            channel_state[chId] = "none"

\* The manager's state and the native runtime agree: a generation that
\* has not begun tearing down is running (or failed, the residual
\* guarantee), one being torn down is not yet destroyed, and the
\* destroyed state means the downcall returned for that generation.
RuntimeStateMatchesNative ==
    /\ runtime_dispose_state \in {"active", "shutdown_pending"} =>
           /\ ~runtime_destroyed[current_runtime]
           /\ runtime_state[current_runtime] \in
                  {"RUNNING", "FAILED_UNQUIESCED"}
    /\ runtime_dispose_state = "destroying" =>
           ~runtime_destroyed[current_runtime]
    /\ runtime_dispose_state = "destroyed" =>
           runtime_destroyed[current_runtime]

\* A settled call left no managed waiter: reader idle, writer settled,
\* headers and status resolved.
DisposeLeavesNoManagedWaiter ==
    \A cId \in CallIds :
        call_dispose_state[cId] = "settled" =>
            /\ reader_state[cId] \in {"idle", "finished"}
            /\ writer_state[cId] \in {"idle", "closed"}
            /\ headers_completion[cId] # "pending"
            /\ status_completion[cId] = "resolved"

\* A settled call owes level 1 nothing, so the runtime's own reclamation
\* is free to take it - and F!CallEventuallyReclaimed, inherited, says it
\* will.  This is the managed half of that handshake.
SettledCallOwesNothing ==
    \A cId \in CallIds :
        call_dispose_state[cId] = "settled" =>
            /\ F!HostOwnsNoPayload(cId)
            /\ F!HostHoldsNoBuffer(cId)

\* Inherited corollary, restated in ring vocabulary.
RingNeverOverflows ==
    \A cId \in CallIds : RingOccupancy(cId) <= DeliveryCredits + 1

(***************************************************************************)
(* LIVENESS TARGETS.  Every promise crossing the native runtime carries    *)
(* the ~NotFailed escape; no termination is guaranteed past a failure.     *)
(***************************************************************************)

\* A waiting write stops waiting once cancellation or dispose arrives -
\* the budget wait promises nothing else, and no other wait exists.
BudgetWaitEndsWhenHopeless ==
    \A cId \in CallIds :
        (/\ writer_state[cId] = "waiting_budget"
         /\ \/ cancel_requested[cId]
            \/ ~F!L0!IsActiveCall(cId)
            \/ call_dispose_state[cId] # "active"
            \/ runtime_dispose_state # "active")
            ~> writer_state[cId] # "waiting_budget"

\* A write in flight settles: its WRITE_DONE completes it, level 1
\* guaranteeing the acquittal before the terminal.
PendingWriteEventuallySettled ==
    \A cId \in CallIds :
        writer_state[cId] \in {"serializing", "awaiting_write_done"} ~>
            (writer_state[cId] \in {"idle", "closed"} \/ ~F!L0!NotFailed)

\* A constructor that began completes, one way or the other: the channel
\* is exposed, or the configuration was refused and it ends in rejected
\* with its lease returned - a terminal outcome, and a completion rather
\* than a stall.  Unless the runtime failed under it.
ChannelConstructionCompletes ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] = "constructing" ~>
            (\/ channel_dispose_state[chId] = "active"
             \/ channel_dispose_state[chId] = "rejected"
             \/ ~F!L0!NotFailed)

\* A draining call settles, a disposing channel settles, the teardown
\* completes - each unless the runtime failed.
CallDisposeCompletes ==
    \A cId \in CallIds :
        call_dispose_state[cId] = "draining" ~>
            (call_dispose_state[cId] = "settled" \/ ~F!L0!NotFailed)

ChannelLeaseEventuallyReleased ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] = "disposing" ~>
            (ChannelSettled(chId) \/ ~F!L0!NotFailed)

\* The public task completes: for the last releaser only after the
\* destroy it triggered returned, which is the contract DisposeAsync
\* states.
ChannelDisposeCompletes ==
    \A chId \in ChannelIds :
        channel_dispose_state[chId] = "disposing" ~>
            (channel_dispose_state[chId] = "disposed" \/ ~F!L0!NotFailed)

RuntimeDisposeCompletes ==
    (runtime_dispose_state \in {"shutdown_pending", "destroying"}) ~>
        (runtime_dispose_state = "absent" \/ ~F!L0!NotFailed)

\* Every allocated root dies: the call's at its terminal callback, the
\* generation's after destroy - the factory then re-arms.
CallRootEventuallyFreed ==
    \A cId \in CallIds :
        call_root_live[cId] ~>
            (~call_root_live[cId] \/ ~F!L0!NotFailed)

RuntimeRootEventuallyFreed ==
    (runtime_root_live /\ runtime_dispose_state # "active") ~>
        (~runtime_root_live \/ ~F!L0!NotFailed)

\* A parse completes and its slot is released - under the stated
\* hypothesis that user parsing terminates.  Both states that own a slot
\* are covered: a cancelled parse holds its payload exactly as a live one
\* does, and releasing it is what the property is about.
InFlightPayloadEventuallyReleased ==
    \A cId \in CallIds :
        reader_state[cId] \in {"parsing", "parsing_cancelled"} ~>
            reader_state[cId] \in {"idle", "finished"}

\* No manager, no lease, no debt.  Whenever the runtime is back to absent
\* and no channel holds a lease, nothing is owed: no live generation root,
\* and no published call left unsettled - DisposeLeavesNoManagedWaiter
\* then carries the rest, since a settled call has its reader, its writer
\* and its public objects resolved.  This holds at the initial state and
\* between generations, not only at the end of a run: the antecedent is
\* "absent and unleased", which is deliberately weaker than "the channel
\* set is spent".  It is what makes the legitimate terminal state of a
\* configuration whose finite channel set IS spent recognizable as
\* quiescence rather than a stall, and a stall with work outstanding
\* breaks it - as it breaks the liveness properties besides.
AbsentRuntimeOwesNothing ==
    (/\ AllLeasesReleased
     /\ runtime_dispose_state = "absent")
        => /\ ~runtime_root_live
           /\ \A cId \in CallIds :
                  call_token_published[cId] =>
                      call_dispose_state[cId] = "settled"

\* A cancellation request is armed only on a read that is in flight, which
\* is what makes a late token inert: there is nothing for it to arm.
ReadCancelPendingOnlyInFlight ==
    \A cId \in CallIds :
        read_cancel_pending[cId] => ReadInFlight(cId)

\* A parse owns its slot for as long as it lasts, cancelled or not: the
\* reader holds the borrow until the marshaller returns, and the release
\* happens there and only there.  This is the lifetime the implementation
\* has to respect - native bytes are readable exactly while the reader is
\* in one of these two states - so it is stated rather than left to the
\* actions' shape, where nothing would catch a release moved earlier.
ParsingReadOwnsItsSlot ==
    \A cId \in CallIds :
        reader_state[cId] \in {"parsing", "parsing_cancelled"} =>
            RingOccupancy(cId) > 0

\* A request that landed is acted on - the reaction is the binding's, and
\* it does not wait for the application.  The target is the effect rather
\* than the disappearance of the flag, because only the effect is the
\* promise: the call has left "active", and either it was cancelled or it
\* had already ended, in which case there was nothing left to cancel.  A
\* flag going away proves nothing on its own; what the application is
\* owed is the cancellation.
PendingReadCancellationEventuallyObserved ==
    \A cId \in CallIds :
        (read_cancel_pending[cId] /\ call_dispose_state[cId] = "active") ~>
            \/ /\ call_dispose_state[cId] # "active"
               /\ \/ F!IsCancelRequested(cId)
                  \/ ~F!L0!IsActiveCall(cId)
            \/ ~F!L0!NotFailed

\* A cancelled read leaves the call on its way out, with no further user
\* action needed: the contract cancelled the call, so the binding drains
\* it.
CancelledReadEventuallyDrainsCall ==
    \A cId \in CallIds :
        read_cancel_pending[cId] ~>
            (call_dispose_state[cId] # "active" \/ ~F!L0!NotFailed)

\* A read in flight resolves: by its payload, by its own cancellation, or
\* by the dispose - never left pending.
ReadInFlightEventuallyResolved ==
    \A cId \in CallIds :
        ReadInFlight(cId) ~>
            (~ReadInFlight(cId) \/ ~F!L0!NotFailed)

\* A waiter is resolved by payload or dispose, never abandoned.
WaitingReaderEventuallyResolved ==
    \A cId \in CallIds :
        reader_state[cId] = "waiting" ~>
            (reader_state[cId] # "waiting" \/ ~F!L0!NotFailed)

\* Every published call settles in the end: by SettleCall when it
\* finishes normally, by the drain when it is disposed early.  A
\* guarantee of the binding, not an obligation on the caller - the only
\* hypothesis it needs is that a readable stream is eventually read.
PublishedCallEventuallySettled ==
    \A cId \in CallIds :
        call_token_published[cId] ~>
            (call_dispose_state[cId] = "settled" \/ ~F!L0!NotFailed)

\* The public completions are never left pending.
HeadersEventuallyResolved ==
    \A cId \in CallIds :
        (headers_completion[cId] = "pending"
             /\ ~F!L0!IsUnusedCall(cId)) ~>
            (headers_completion[cId] # "pending" \/ ~F!L0!NotFailed)

StatusEventuallyResolved ==
    \A cId \in CallIds :
        (status_completion[cId] = "pending"
             /\ ~F!L0!IsUnusedCall(cId)) ~>
            (status_completion[cId] = "resolved" \/ ~F!L0!NotFailed)

===============================================================================
