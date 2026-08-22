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
(*     taken verbatim into Fairness below: the binding does not restrict   *)
(*     the runtime's own actions, so assuming the runtime keeps its        *)
(*     promises is citation, not proof.  The six host ones may NOT be      *)
(*     assumed - deriving them from the binding's mechanisms is the point  *)
(*     of the level.                                                       *)
(*  3. The managed-side invariants: roots, the single reader with its     *)
(*     waiting and parsing states, dispose ordering, and the retry         *)
(*     protocol.                                                           *)
(*                                                                         *)
(* Scope.  The model is the generic bidirectional-streaming call: the      *)
(* five CallInvoker methods are refinements of it that fix the number of   *)
(* messages in each direction, not separate machines, and are therefore    *)
(* not modelled.  The start/send command queue is level-1 stutter.  The    *)
(* .NET async runtime is not modelled: completions signal, continuations   *)
(* run elsewhere - RunContinuationsAsynchronously is an implementation     *)
(* rule verified by review and tests, not a theorem.  The memory model of  *)
(* the ring stays a coding rule for review, outside every level.           *)
(*                                                                         *)
(* The application owes two progression facts - per call, eventually       *)
(* begin the next read or dispose (one weak fairness), and dispose every   *)
(* call it created, which the finite token universe turns into the         *)
(* theorem PublishedCallEventuallyDisposed - plus one conformity           *)
(* hypothesis of safety: MoveNext calls are serialized, as                 *)
(* IAsyncStreamReader requires, which the single reader_state value        *)
(* encodes by representation.  Everything else - the four callback         *)
(* returns, the buffer return, the drain, the roots - is carried by the    *)
(* binding, under one stated hypothesis: user serialization and parsing    *)
(* terminate.  The disposable wrapper returns the buffer on success and    *)
(* on exception alike; nothing can cover code that never comes back.       *)
(***************************************************************************)

EXTENDS DotNetBindingState, Naturals, Sequences

F == INSTANCE FfiGrpcTheorems

l1_vars == F!vars
vars == <<l1_vars, managed_vars>>

ManagedStutter == UNCHANGED managed_vars

(***************************************************************************)
(* DERIVED PREDICATES                                                      *)
(***************************************************************************)

\* The ring read through level-1 state: the trampoline publishes inside the
\* delivery callback (head = events delivered), ak_event_consumed releases
\* (tail = payloads consumed).  Advancing a counter can only release the
\* oldest, which is PayloadsReleasedInOrder held by representation.
RingHead(cId) == Len(events_delivered[cId])
RingTail(cId) == payloads_consumed_by_host[cId]
RingOccupancy(cId) == RingHead(cId) - RingTail(cId)
RingDrained(cId) == RingTail(cId) = RingHead(cId)

ConsumerPhases == {"prologue", "application", "drain", "done"}
ReaderStates == {"idle", "waiting", "parsing"}
RetryStates == {"idle", "awaiting_budget"}
CallDisposeStates == {"active", "draining", "disposed"}
RuntimeDisposeStates == {"active", "disposing_calls", "destroying",
                         "destroyed"}

\* The remembered length's sentinel: one past every request length, so the
\* whole domain stays integer - a string sentinel would make the type
\* heterogeneous, which TLC cannot compare.
NoRetryLen == Ceiling + 2

\* The binding downcalls on a call only while neither the call nor the
\* runtime is being torn down.  The channel and runtime downcalls carry
\* their own runtime-level guards; NoDowncallAfterDestroy is the sum of
\* both plus the dispose ordering, not this predicate alone.
BindingMayDowncall(cId) ==
    /\ call_dispose_state[cId] = "active"
    /\ runtime_dispose_state = "active"

ManagedTypeOK ==
    /\ call_token_published \in [CallIds -> BOOLEAN]
    /\ call_root_live \in [CallIds -> BOOLEAN]
    /\ runtime_root_live \in BOOLEAN
    /\ consumer_phase \in [CallIds -> ConsumerPhases]
    /\ reader_state \in [CallIds -> ReaderStates]
    /\ retry_state \in [CallIds -> RetryStates]
    /\ retry_len \in [CallIds -> F!RequestLengths \union {NoRetryLen}]
    /\ call_dispose_state \in [CallIds -> CallDisposeStates]
    /\ runtime_dispose_state \in RuntimeDisposeStates

(***************************************************************************)
(* INIT                                                                    *)
(***************************************************************************)

ManagedInit ==
    /\ call_token_published = [c \in CallIds |-> FALSE]
    /\ call_root_live = [c \in CallIds |-> FALSE]
    /\ runtime_root_live = FALSE
    /\ consumer_phase = [c \in CallIds |-> "prologue"]
    /\ reader_state = [c \in CallIds |-> "idle"]
    /\ retry_state = [c \in CallIds |-> "idle"]
    /\ retry_len = [c \in CallIds |-> NoRetryLen]
    /\ call_dispose_state = [c \in CallIds |-> "active"]
    /\ runtime_dispose_state = "active"

Init == F!Init /\ ManagedInit

(***************************************************************************)
(* CONSTRUCTION - atomic with exposure.  The runtime belongs to the        *)
(* process: the factory materializes it at first use, every invoker        *)
(* borrows it (a refcount above this model, verified by tests), and the    *)
(* last release tears it down.  The factory's materialization allocates    *)
(* the RuntimeState root, performs the native create and only then         *)
(* publishes the holder - no thread sees a half-built runtime, so neither  *)
(* does the model.  A call's constructor is the same shape at call scope.  *)
(***************************************************************************)

\* The process factory materializes the runtime at first use: the shared
\* RuntimeState's root and ak_runtime_create in one step.  Dispose cannot
\* precede this - there is nothing to release.
CreateRuntime(rtId) ==
    /\ ~runtime_root_live
    /\ runtime_dispose_state = "active"
    /\ F!RuntimeCreate(rtId)
    /\ runtime_root_live' = TRUE
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   reader_state, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* The call constructor: GCHandle.Alloc, ak_call_start, and only then the
\* object returns to the user - the token, the root and the start are one
\* step, and the application's fairness can only see an exposed call.
StartCall(cId, chId) ==
    /\ ~call_token_published[cId]
    /\ BindingMayDowncall(cId)
    /\ F!CallStart(cId, chId)
    /\ call_token_published' = [call_token_published EXCEPT ![cId] = TRUE]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = TRUE]
    /\ UNCHANGED <<runtime_root_live, consumer_phase, reader_state,
                   retry_state, retry_len, call_dispose_state,
                   runtime_dispose_state>>

(***************************************************************************)
(* THE READER - one per call, in three states.  BeginMoveNext may suspend  *)
(* on an empty ring; the parse begins when a payload exists; the release   *)
(* happens when the parse completes.  A waiter caught by a dispose is      *)
(* resolved by the binding, never abandoned.                               *)
(***************************************************************************)

\* MoveNext called: the reader is committed, payload or not.  This is the
\* application's step - the one thing its fairness promises.
BeginMoveNext(cId) ==
    /\ consumer_phase[cId] = "application"
    /\ call_dispose_state[cId] = "active"
    /\ reader_state[cId] = "idle"
    /\ reader_state' = [reader_state EXCEPT ![cId] = "waiting"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* A payload exists and the call is still active: the suspended MoveNext
\* wakes and takes the slot.  The wake-up is the binding's (the TCS
\* completion), so it carries WF.  A dispose that linearized first wins:
\* the waiter then resolves through CancelWaiter, never by parsing a ring
\* the drain already owns.
BeginParse(cId) ==
    /\ reader_state[cId] = "waiting"
    /\ call_dispose_state[cId] = "active"
    /\ RingOccupancy(cId) > 0
    /\ reader_state' = [reader_state EXCEPT ![cId] = "parsing"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* The parse completes: the slot is released.  ak_event_consumed under the
\* reader's exclusive right, whatever the dispose machine did meanwhile.
FinishConsumePayload(cId) ==
    /\ reader_state[cId] = "parsing"
    /\ F!HostConsumesEvent(cId)
    /\ reader_state' = [reader_state EXCEPT ![cId] = "idle"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* A waiter caught by the dispose: the binding resolves it - the suspended
\* MoveNext completes exceptionally, it does not linger on a ring it no
\* longer owns.
CancelWaiter(cId) ==
    /\ reader_state[cId] = "waiting"
    /\ call_dispose_state[cId] # "active"
    /\ reader_state' = [reader_state EXCEPT ![cId] = "idle"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* The drain takes the ring once the reader is idle: a parse in flight
\* completes first, a waiter is resolved first - the drain starts behind
\* them, never beside them.
HandoffToDrain(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] \in {"prologue", "application"}
    /\ reader_state[cId] = "idle"
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "drain"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   reader_state, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

(***************************************************************************)
(* MANAGED-ONLY ACTIONS - the dispose machines and the retry exits.        *)
(***************************************************************************)

\* The last reference released: the process factory begins the teardown.
\* Remembered, not deferred - the binding settles the calls still open,
\* and only then does the native shutdown begin.  Guarded on the runtime
\* existing: there is nothing to release before CreateRuntime.
RequestRuntimeDispose ==
    /\ runtime_root_live
    /\ runtime_dispose_state = "active"
    /\ runtime_dispose_state' = "disposing_calls"
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state, retry_state, retry_len,
                   call_dispose_state>>

\* BudgetCancellationStopsRetry, the transition half: cancellation or
\* dispose ends the wait.
LeaveBudgetWait(cId) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ \/ cancel_requested[cId]
       \/ call_dispose_state[cId] # "active"
       \/ runtime_dispose_state # "active"
    /\ retry_state' = [retry_state EXCEPT ![cId] = "idle"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = NoRetryLen]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state,
                   call_dispose_state, runtime_dispose_state>>

\* The drain finished: the ring is empty and the terminal was released -
\* a started call always has one to drain.  Nothing remains to downcall -
\* the runtime reclaims the call on its own fairness (ReleaseCallHandle
\* is its step).
FinishDisposeCall(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] = "drain"
    /\ RingDrained(cId)
    /\ F!L0!HasStatus(cId)
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "disposed"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "done"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   reader_state, retry_state, retry_len,
                   runtime_dispose_state>>

\* The runtime's root is freed after ak_runtime_destroy returned, later
\* than every event of every kind.
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
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   reader_state, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

(***************************************************************************)
(* COUPLED ACTIONS - a level-1 action conjoined with the managed step      *)
(* that rides on it.                                                       *)
(***************************************************************************)

\* A non-terminal delivery callback returns after publishing the slot and
\* completing the TCS the event answers.
OnEventReturns(cId) ==
    /\ ~F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ ManagedStutter

\* The terminal callback's return is the call root's last access: freeing
\* the root is its final instruction, one linearization point, not two.
TerminalCallbackReturns(cId) ==
    /\ F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<call_token_published, runtime_root_live,
                   consumer_phase, reader_state, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* WRITE_DONE completes the pending write's TCS the same way.
WriteDoneReturns(cId) ==
    /\ F!WriteDoneReturns(cId)
    /\ ManagedStutter

\* The runtime-level callbacks return without touching managed call state.
ShutdownReturns(rtId) ==
    /\ F!ShutdownCallbackReturns(rtId)
    /\ ManagedStutter

ResourcesReleasedReturns(rtId) ==
    /\ F!ResourcesReleasedCallbackReturns(rtId)
    /\ ManagedStutter

\* The prologue owns slot 0: it releases the header and hands the ring to
\* the application.  ak_event_consumed on the INITIAL_METADATA event.  The
\* prologue is the binding's own bounded read, so it needs no reader
\* state: nothing of the application runs inside it.
ConsumeHeader(cId) ==
    /\ consumer_phase[cId] = "prologue"
    /\ call_dispose_state[cId] = "active"
    /\ RingTail(cId) = 0
    /\ F!HostConsumesEvent(cId)
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "application"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   reader_state, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* DisposeAsync on a call - also fired by the invoker's dispose for every
\* call it still holds.  Requests cancellation if the call can still take
\* one; the ring goes to the drain only once the reader is idle, through
\* HandoffToDrain.
BeginDisposeCall(cId) ==
    /\ call_token_published[cId]
    /\ call_dispose_state[cId] = "active"
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "draining"]
    /\ \/ F!RequestCallCancellation(cId)
       \/ /\ \/ ~F!L0!IsActiveCall(cId)
             \/ cancel_requested[cId]
          /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state, retry_state, retry_len,
                   runtime_dispose_state>>

\* The teardown settles every call still open, one by one.
DisposeCallForRuntime(cId) ==
    /\ runtime_dispose_state = "disposing_calls"
    /\ BeginDisposeCall(cId)

\* The drain releases what is left, from tail up, terminal last.
DrainRelease(cId) ==
    /\ consumer_phase[cId] = "drain"
    /\ F!HostConsumesEvent(cId)
    /\ ManagedStutter

\* The disposable wrapper closing over serialization, refusal and
\* cancellation alike: the lent buffer goes back on success and on
\* exception.  The one hypothesis is that user serialization terminates.
ReturnLentBuffer(cId, b) ==
    /\ F!HostReturnsBuffer(cId, b)
    /\ ManagedStutter

\* The budget refusal itself enters the wait: the cause and the state are
\* one step, so no trace observes BUDGET_BUSY without the wait it
\* promises.  The refused length is remembered - the wait is about this
\* request.  Eligibility (ContemplatesLend) already requires the call to
\* hold no buffer, which is RetryingCallHoldsNoBuffer at the door.  The
\* wait is cancellable and nothing more: repeated attempts are admitted -
\* a lend or refusal of the same length remains enabled at level 1 - but
\* never promised, and no cadence exists.
ObserveBudgetRefusal(cId, len, charge) ==
    /\ retry_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ ~cancel_requested[cId]
    /\ F!RefuseLendForBudget(cId, len, charge)
    /\ retry_state' = [retry_state EXCEPT ![cId] = "awaiting_budget"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = len]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state,
                   call_dispose_state, runtime_dispose_state>>

\* The wait ends with the lend that succeeds: ak_get_call_buffer returns
\* OK for the remembered request, inside the same downcall.  The lend
\* carries no fairness - the poll is never owed a grant - so this action
\* carries none either.
RetryLendSucceeds(cId, b, charge) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ BindingMayDowncall(cId)
    /\ F!LendSendBuffer(cId, b, retry_len[cId], charge)
    /\ retry_state' = [retry_state EXCEPT ![cId] = "idle"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = NoRetryLen]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state,
                   call_dispose_state, runtime_dispose_state>>

\* Every call settled: the native shutdown begins.  A published call must
\* have been disposed; an untouched slot needs nothing.
BeginRuntimeShutdown(rtId) ==
    /\ runtime_dispose_state = "disposing_calls"
    /\ \A c \in CallIds :
           \/ call_dispose_state[c] = "disposed"
           \/ F!L0!IsUnusedCall(c)
    /\ F!RuntimeBeginShutdown(rtId)
    /\ runtime_dispose_state' = "destroying"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state, retry_state, retry_len,
                   call_dispose_state>>

\* ak_runtime_destroy returns AK_STATUS_OK: dispose may complete.  A
\* runtime that failed instead never reaches this - the liveness carries
\* the ~NotFailed escape, and no termination is guaranteed past a failure.
FinishDisposeRuntime(rtId) ==
    /\ runtime_dispose_state = "destroying"
    /\ F!RuntimeDestroy(rtId)
    /\ runtime_dispose_state' = "destroyed"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, reader_state, retry_state, retry_len,
                   call_dispose_state>>

(***************************************************************************)
(* PASSTHROUGHS - level-1 actions the binding does not decorate.  The      *)
(* runtime's own steps pass unguarded; the binding's downcalls carry the   *)
(* teardown guard, and the lend family additionally the retry discipline:  *)
(* while a call waits on the budget, only the remembered request runs.     *)
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
    \/ \E chId \in ChannelIds, rtId \in RuntimeIds :
           /\ runtime_dispose_state = "active"
           /\ F!ChannelCreate(chId, rtId)
    \/ \E chId \in ChannelIds :
           /\ runtime_dispose_state = "active"
           /\ F!ChannelStartClosing(chId)
    \/ \E cId \in CallIds :
           /\ BindingMayDowncall(cId)
           /\ F!RequestCallCancellation(cId)
    \/ \E cId \in CallIds, b \in BufferIds,
         len \in F!Sizes, charge \in F!Sizes :
           /\ BindingMayDowncall(cId)
           /\ retry_state[cId] = "idle"
           /\ F!LendSendBuffer(cId, b, len, charge)
    \/ \E cId \in CallIds, len \in F!RequestLengths :
           /\ BindingMayDowncall(cId)
           /\ retry_state[cId] = "idle"
           /\ F!RefuseLendTooLarge(cId, len)
    \/ \E cId \in CallIds, len \in F!Sizes :
           /\ BindingMayDowncall(cId)
           /\ retry_state[cId] = "idle"
           /\ F!RefuseLendForSlot(cId, len)
    \/ \E cId \in CallIds, charge \in F!CandidateCharges :
           /\ BindingMayDowncall(cId)
           /\ retry_state[cId] = "awaiting_budget"
           /\ F!RefuseLendForBudget(cId, retry_len[cId], charge)
    \/ \E cId \in CallIds, msg \in Messages, b \in BufferIds :
           /\ BindingMayDowncall(cId)
           /\ F!SendMessage(cId, msg, b)
    \/ \E cId \in CallIds :
           /\ BindingMayDowncall(cId)
           /\ F!EndSend(cId)

Passthrough == (RuntimeSteps \/ BindingDowncalls) /\ ManagedStutter

(***************************************************************************)
(* NEXT                                                                    *)
(***************************************************************************)

Next ==
    \/ Passthrough
    \/ RequestRuntimeDispose
    \/ FreeRuntimeRoot
    \/ \E rtId \in RuntimeIds : CreateRuntime(rtId)
    \/ \E cId \in CallIds :
           \/ BeginMoveNext(cId)
           \/ BeginParse(cId)
           \/ FinishConsumePayload(cId)
           \/ CancelWaiter(cId)
           \/ HandoffToDrain(cId)
           \/ LeaveBudgetWait(cId)
           \/ FinishDisposeCall(cId)
           \/ OnEventReturns(cId)
           \/ TerminalCallbackReturns(cId)
           \/ WriteDoneReturns(cId)
           \/ ConsumeHeader(cId)
           \/ BeginDisposeCall(cId)
           \/ DisposeCallForRuntime(cId)
           \/ DrainRelease(cId)
    \/ \E cId \in CallIds, chId \in ChannelIds : StartCall(cId, chId)
    \/ \E cId \in CallIds, b \in BufferIds : ReturnLentBuffer(cId, b)
    \/ \E cId \in CallIds, len \in F!Sizes, charge \in F!CandidateCharges :
           ObserveBudgetRefusal(cId, len, charge)
    \/ \E cId \in CallIds, b \in BufferIds, charge \in F!Sizes :
           RetryLendSucceeds(cId, b, charge)
    \/ \E rtId \in RuntimeIds :
           \/ ShutdownReturns(rtId)
           \/ ResourcesReleasedReturns(rtId)
           \/ BeginRuntimeShutdown(rtId)
           \/ FinishDisposeRuntime(rtId)

(***************************************************************************)
(* FAIRNESS                                                                *)
(*                                                                         *)
(* Three tiers.  The runtime's and the FFI dispatch's thirteen families    *)
(* are taken verbatim from level 1 - same actions, same tuple - because    *)
(* the binding restricts none of them: their extraction from Spec is       *)
(* citation.  The binding's own tier is what derives the six host          *)
(* conjuncts of F!Fairness.  The application's tier is a single conjunct   *)
(* per call - begin the next read or dispose - and disposing every call    *)
(* it created is normative in the API, encoded by the same conjunct.       *)
(***************************************************************************)

RuntimeOwedFairness ==
    /\ \A cId \in CallIds : WF_l1_vars(F!NetworkSend(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!ReceiveStatus(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverInitialMetadata(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverMessage(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverStatus(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!DeliverCancelled(cId))
    /\ \A cId \in CallIds : WF_l1_vars(F!EmitWriteDone(cId))
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!RuntimeRelease(rtId))
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!EmitShutdownComplete(rtId))
    /\ \A rtId \in RuntimeIds : WF_l1_vars(F!EmitResourcesReleased(rtId))
    /\ \A chId \in ChannelIds : WF_l1_vars(F!ChannelFinishClosing(chId))
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_l1_vars(F!FreeReturnedBuffer(cId, b))
    /\ \A cId \in CallIds : WF_l1_vars(F!ReleaseCallHandle(cId))

\* The binding's own promises.  FinishConsumePayload and ReturnLentBuffer
\* carry the one stated hypothesis: user parsing and serialization
\* terminate - the wrapper covers success and exception, nothing covers
\* code that never comes back.
BindingOwedFairness ==
    /\ \A cId \in CallIds : WF_vars(OnEventReturns(cId))
    /\ \A cId \in CallIds : WF_vars(TerminalCallbackReturns(cId))
    /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(ShutdownReturns(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(ResourcesReleasedReturns(rtId))
    /\ \A cId \in CallIds : WF_vars(ConsumeHeader(cId))
    /\ \A cId \in CallIds : WF_vars(BeginParse(cId))
    /\ \A cId \in CallIds : WF_vars(FinishConsumePayload(cId))
    /\ \A cId \in CallIds : WF_vars(CancelWaiter(cId))
    /\ \A cId \in CallIds : WF_vars(HandoffToDrain(cId))
    /\ \A cId \in CallIds : WF_vars(DrainRelease(cId))
    /\ \A cId \in CallIds : WF_vars(FinishDisposeCall(cId))
    /\ \A cId \in CallIds : WF_vars(DisposeCallForRuntime(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(BeginRuntimeShutdown(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(FinishDisposeRuntime(rtId))
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_vars(ReturnLentBuffer(cId, b))
    /\ \A cId \in CallIds : WF_vars(LeaveBudgetWait(cId))
    /\ WF_vars(FreeRuntimeRoot)

\* The application's single obligation, per call: begin the next read or
\* dispose.  Disposing every call it created is the normative half.
ApplicationOwedFairness ==
    \A cId \in CallIds :
        WF_vars(BeginMoveNext(cId) \/ BeginDisposeCall(cId))

Fairness ==
    /\ RuntimeOwedFairness
    /\ BindingOwedFairness
    /\ ApplicationOwedFairness

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* INVARIANTS                                                              *)
(***************************************************************************)

\* The GCHandle exists from the same step as ak_call_start: no used call
\* without its token.
TokenPublishedBeforeStart ==
    \A cId \in CallIds :
        ~F!L0!IsUnusedCall(cId) => call_token_published[cId]

\* Every call callback in flight resolves its call_ctx to a live root.
RootSurvivesCallbacks ==
    \A cId \in CallIds :
        \/ delivery_callback_running[cId]
        \/ write_done_callback_running[cId]
        => call_root_live[cId]

\* Every callback of every kind carries runtime_ctx, so all of them keep
\* the invoker's root alive - the call callbacks too, not only the two
\* runtime events.
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
               call_dispose_state[cId] = "disposed"
        /\ consumer_phase[cId] = "drain" =>
               call_dispose_state[cId] = "draining"
        /\ call_dispose_state[cId] = "active" =>
               consumer_phase[cId] \in {"prologue", "application"}

\* The reader - waiting or parsing - exists only where the application
\* reads; one value per call is what makes it unique, matching
\* IAsyncStreamReader's single-read contract.
AtMostOneReaderOutstanding ==
    \A cId \in CallIds :
        reader_state[cId] # "idle" =>
            consumer_phase[cId] = "application"

\* The drain never runs beside an application read or a suspended one.
DrainNeverOverlapsApplicationConsumer ==
    \A cId \in CallIds :
        consumer_phase[cId] \in {"drain", "done"} =>
            reader_state[cId] = "idle"

\* A call waiting on the budget holds no lent buffer - the deadlock
\* level 1 cannot see, forbidden at the door.
RetryingCallHoldsNoBuffer ==
    \A cId \in CallIds :
        retry_state[cId] = "awaiting_budget" =>
            buffers_held_by_host[cId] = 0

\* The wait is caused by the refusal it treats: a waiting call's last
\* lend result is BUDGET_BUSY, never MESSAGE_TOO_LARGE - which is how the
\* permanent refusal stays out of the loop.
RetryOnlyAfterBudgetRefusal ==
    \A cId \in CallIds :
        retry_state[cId] = "awaiting_budget" =>
            last_lend_status[cId] = "BUDGET_BUSY"

\* The remembered length exists exactly while waiting.
RetryLenMatchesWait ==
    \A cId \in CallIds :
        retry_state[cId] = "awaiting_budget" <=> retry_len[cId] # NoRetryLen

\* Dispose completed means ak_runtime_destroy returned.
DisposeAwaitsDestroy ==
    runtime_dispose_state = "destroyed" =>
        \E rtId \in RuntimeIds : runtime_destroyed[rtId]

\* Inherited corollary, restated in ring vocabulary: level 1's payload
\* accounting read through the mapping.  Re-proved by citation, not by
\* induction.
RingNeverOverflows ==
    \A cId \in CallIds : RingOccupancy(cId) <= DeliveryCredits + 1

(***************************************************************************)
(* LIVENESS TARGETS - properties the theorems module states.  Every        *)
(* promise that crosses the native runtime carries the ~NotFailed escape,  *)
(* like every level-0 and level-1 promise: a failed runtime is the         *)
(* contract's one admitted way out, and no termination is guaranteed past  *)
(* a failure.                                                              *)
(***************************************************************************)

\* A refused-and-waiting call stops waiting once cancellation or dispose
\* arrives.  Conditional by design: no deadline is mandatory, cancellation
\* may never come, and acquisition is never promised.  The wait is
\* cancellable and nothing more - no repetition of attempts is promised
\* either, which is why no action models the attempts: a retried refusal
\* changes nothing observable, and the successful retry is
\* RetryLendSucceeds.
BudgetCancellationStopsRetry ==
    \A cId \in CallIds :
        (/\ retry_state[cId] = "awaiting_budget"
         /\ \/ cancel_requested[cId]
            \/ call_dispose_state[cId] # "active"
            \/ runtime_dispose_state # "active")
            ~> retry_state[cId] = "idle"

\* A disposed call settles: the drain reaches the terminal and releases
\* everything, unless the runtime failed.
CallDisposeCompletes ==
    \A cId \in CallIds :
        call_dispose_state[cId] = "draining" ~>
            (call_dispose_state[cId] = "disposed" \/ ~F!L0!NotFailed)

\* The invoker's dispose completes, from the public request on, unless the
\* runtime failed.
RuntimeDisposeCompletes ==
    (runtime_dispose_state \in {"disposing_calls", "destroying"}) ~>
        (runtime_dispose_state = "destroyed" \/ ~F!L0!NotFailed)

\* Every allocated root dies: the call's at its terminal callback, the
\* invoker's after destroy.
CallRootEventuallyFreed ==
    \A cId \in CallIds :
        call_root_live[cId] ~>
            (~call_root_live[cId] \/ ~F!L0!NotFailed)

RuntimeRootEventuallyFreed ==
    (runtime_root_live /\ runtime_dispose_state # "active") ~>
        (~runtime_root_live \/ ~F!L0!NotFailed)

\* A parse completes and its slot is released - under the stated
\* hypothesis that user parsing terminates, which the binding's WF on
\* FinishConsumePayload encodes.
InFlightPayloadEventuallyReleased ==
    \A cId \in CallIds :
        reader_state[cId] = "parsing" ~> reader_state[cId] = "idle"

\* A waiter is resolved by payload or dispose, never abandoned: the
\* level-1 delivery liveness brings the payload and BeginParse is fair,
\* or the dispose arrives and CancelWaiter is fair.
WaitingReaderEventuallyResolved ==
    \A cId \in CallIds :
        reader_state[cId] = "waiting" ~>
            (reader_state[cId] # "waiting" \/ ~F!L0!NotFailed)

\* Every published call is disposed in the end.  This is a theorem of the
\* model, not only a norm of the API: occurrence tokens are globally
\* unique and finite, so no reader reads forever - the reads dry up, the
\* application's fairness has only the dispose left to take.
PublishedCallEventuallyDisposed ==
    \A cId \in CallIds :
        call_token_published[cId] ~>
            (call_dispose_state[cId] = "disposed" \/ ~F!L0!NotFailed)

===============================================================================
