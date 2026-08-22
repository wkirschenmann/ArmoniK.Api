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
(*  3. The managed-side invariants: roots, single consumer, dispose        *)
(*     ordering, and the retry protocol.                                   *)
(*                                                                         *)
(* Scope.  The model is the generic bidirectional-streaming call: the      *)
(* five CallInvoker methods are refinements of it that fix the number of   *)
(* messages in each direction, not separate machines, and are therefore    *)
(* not modelled.  The start/send command queue is level-1 stutter.  The    *)
(* memory model of the ring (release/acquire on the indexes) is a coding   *)
(* rule for review, deliberately outside every level of the specification. *)
(*                                                                         *)
(* The application's residual obligation is exactly one weak fairness per  *)
(* call: eventually consume the next response or dispose the call.  Every  *)
(* other host obligation - the four callback returns, the buffer return,   *)
(* the drain - is carried by the binding itself.                           *)
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
RetryStates == {"idle", "awaiting_budget"}
CallDisposeStates == {"active", "draining", "disposed"}
RuntimeDisposeStates == {"active", "destroying", "destroyed"}

\* The binding downcalls on a call only while neither the call nor the
\* runtime is being torn down - the obligation side of
\* DestroyedRuntimeRejectsHandles, ordered rather than raced.
BindingMayDowncall(cId) ==
    /\ call_dispose_state[cId] = "active"
    /\ runtime_dispose_state = "active"

ManagedTypeOK ==
    /\ call_token_published \in [CallIds -> BOOLEAN]
    /\ call_root_live \in [CallIds -> BOOLEAN]
    /\ runtime_root_live \in BOOLEAN
    /\ consumer_phase \in [CallIds -> ConsumerPhases]
    /\ pending_continuations \in [CallIds -> Nat]
    /\ retry_state \in [CallIds -> RetryStates]
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
    /\ pending_continuations = [c \in CallIds |-> 0]
    /\ retry_state = [c \in CallIds |-> "idle"]
    /\ call_dispose_state = [c \in CallIds |-> "active"]
    /\ runtime_dispose_state = "active"

Init == F!Init /\ ManagedInit

(***************************************************************************)
(* MANAGED-ONLY ACTIONS                                                    *)
(* The level-1 state stutters, which the refinement admits for free.       *)
(***************************************************************************)

\* GCHandle.Alloc(callState): the token exists before any downcall can
\* carry it.  TokenPublishedBeforeStart is this guard made an invariant.
PublishCallToken(cId) ==
    /\ ~call_token_published[cId]
    /\ call_token_published' = [call_token_published EXCEPT ![cId] = TRUE]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<runtime_root_live, consumer_phase,
                   pending_continuations, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* The invoker's root: allocated before the runtime is created, freed only
\* after ak_runtime_destroy returned.  This is the allocation half.
PublishRuntimeRoot ==
    /\ ~runtime_root_live
    /\ runtime_dispose_state = "active"
    /\ runtime_root_live' = TRUE
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   pending_continuations, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* A completion's continuation runs as its own step, on its own thread:
\* the queue between signal and run is what ContinuationsAsync names, and
\* its weak fairness below is the thread pool's.
RunContinuation(cId) ==
    /\ pending_continuations[cId] > 0
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ - 1]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state,
                   call_dispose_state, runtime_dispose_state>>

\* BUDGET_BUSY observed: the call enters the polling wait.  The guard that
\* it holds no lent buffer is RetryingCallHoldsNoBuffer made structural -
\* a call still holding one returns it before it may wait.  There is no
\* action entering this wait from MESSAGE_TOO_LARGE:
\* MessageTooLargeIsNotRetried is the absence.
EnterBudgetWait(cId) ==
    /\ retry_state[cId] = "idle"
    /\ last_lend_status[cId] = "BUDGET_BUSY"
    /\ buffers_held_by_host[cId] = 0
    /\ ~cancel_requested[cId]
    /\ call_dispose_state[cId] = "active"
    /\ retry_state' = [retry_state EXCEPT ![cId] = "awaiting_budget"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* BudgetCancellationStopsRetry, the transition half: cancellation or
\* dispose ends the wait.  The retry poll itself needs no action of its
\* own - a retry is the level-1 lend or one of its refusals firing again.
LeaveBudgetWait(cId) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ \/ cancel_requested[cId]
       \/ call_dispose_state[cId] # "active"
    /\ retry_state' = [retry_state EXCEPT ![cId] = "idle"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* The drain finished: the ring is empty and the terminal was released (or
\* the call never started).  Nothing remains to downcall - the runtime
\* reclaims the call on its own fairness (ReleaseCallHandle is its step).
FinishDisposeCall(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ \/ /\ RingDrained(cId)
          /\ F!L0!HasStatus(cId)
       \/ F!L0!IsUnusedCall(cId)
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "disposed"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "done"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   pending_continuations, retry_state,
                   runtime_dispose_state>>

\* The terminal callback's last act, native side: the call's root is freed
\* once the terminal was delivered and no callback of the call is running.
FreeCallRoot(cId) ==
    /\ call_root_live[cId]
    /\ F!L0!HasStatus(cId)
    /\ ~delivery_callback_running[cId]
    /\ ~write_done_callback_running[cId]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, runtime_root_live, consumer_phase,
                   pending_continuations, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* The runtime's root is freed after ak_runtime_destroy returned, later
\* than every event of every kind.
FreeRuntimeRoot ==
    /\ runtime_root_live
    /\ runtime_dispose_state = "destroyed"
    /\ \A rtId \in RuntimeIds :
           /\ ~shutdown_callback_running[rtId]
           /\ ~resources_released_callback_running[rtId]
    /\ runtime_root_live' = FALSE
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   pending_continuations, retry_state, call_dispose_state,
                   runtime_dispose_state>>

(***************************************************************************)
(* COUPLED ACTIONS - a level-1 action conjoined with the managed step      *)
(* that rides on it.                                                       *)
(***************************************************************************)

\* ak_call_start carries the token, so the token exists first.
StartCall(cId, chId) ==
    /\ call_token_published[cId]
    /\ BindingMayDowncall(cId)
    /\ F!CallStart(cId, chId)
    /\ ManagedStutter

\* The delivery callback returns after publishing the slot and completing
\* the TCS the event answers; the continuation is queued, never run here.
OnEventReturns(cId) ==
    /\ F!DeliveryCallbackReturns(cId)
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ + 1]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* WRITE_DONE completes the pending write's TCS the same way.
WriteDoneReturnsQueues(cId) ==
    /\ F!WriteDoneReturns(cId)
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ + 1]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* The runtime-level callbacks return without touching managed call state.
ShutdownReturns(rtId) ==
    /\ F!ShutdownCallbackReturns(rtId)
    /\ ManagedStutter

ResourcesReleasedReturns(rtId) ==
    /\ F!ResourcesReleasedCallbackReturns(rtId)
    /\ ManagedStutter

\* The prologue owns slot 0: it releases the header and hands the ring to
\* the application.  ak_event_consumed on the INITIAL_METADATA event.
ConsumeHeader(cId) ==
    /\ consumer_phase[cId] = "prologue"
    /\ RingTail(cId) = 0
    /\ F!HostConsumesEvent(cId)
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "application"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   pending_continuations, retry_state, call_dispose_state,
                   runtime_dispose_state>>

\* TryTake: the application releases the oldest slot.  This action and
\* BeginDisposeCall together carry the application's one obligation.
ConsumePayload(cId) ==
    /\ consumer_phase[cId] = "application"
    /\ call_dispose_state[cId] = "active"
    /\ F!HostConsumesEvent(cId)
    /\ ManagedStutter

\* DisposeAsync: requests cancellation if the call can still take one, and
\* hands the ring to the drain.  A call already cancelled, terminal or
\* never started has nothing to request.
BeginDisposeCall(cId) ==
    /\ call_dispose_state[cId] = "active"
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "draining"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "drain"]
    /\ \/ F!RequestCallCancellation(cId)
       \/ /\ \/ ~F!L0!IsActiveCall(cId)
             \/ cancel_requested[cId]
          /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   pending_continuations, retry_state,
                   runtime_dispose_state>>

\* The drain releases what is left, from tail up, terminal last.
DrainRelease(cId) ==
    /\ consumer_phase[cId] = "drain"
    /\ F!HostConsumesEvent(cId)
    /\ ManagedStutter

\* The retry succeeding: ak_get_call_buffer returns OK and the wait ends
\* inside the same downcall.  The lend itself carries no fairness - the
\* poll is never owed a grant - so this action carries none either.
RetryLendSucceeds(cId, b, len, charge) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ BindingMayDowncall(cId)
    /\ F!LendSendBuffer(cId, b, len, charge)
    /\ retry_state' = [retry_state EXCEPT ![cId] = "idle"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* The using scope closing over serialization, refusal and cancellation
\* alike: the lent buffer goes back whatever happened.
ReturnLentBuffer(cId, b) ==
    /\ F!HostReturnsBuffer(cId, b)
    /\ ManagedStutter

\* DisposeAsync on the invoker: every call it holds is disposed first -
\* the ordering that makes NoDowncallAfterDestroy structural - then the
\* shutdown chain starts.
BeginDisposeRuntime(rtId) ==
    /\ runtime_dispose_state = "active"
    /\ \A c \in CallIds :
           call_dispose_state[c] = "disposed" \/ F!L0!IsUnusedCall(c)
    /\ F!RuntimeBeginShutdown(rtId)
    /\ runtime_dispose_state' = "destroying"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations, retry_state,
                   call_dispose_state>>

\* ak_runtime_destroy returns AK_STATUS_OK: dispose may complete.
FinishDisposeRuntime(rtId) ==
    /\ runtime_dispose_state = "destroying"
    /\ F!RuntimeDestroy(rtId)
    /\ runtime_dispose_state' = "destroyed"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations, retry_state,
                   call_dispose_state>>

(***************************************************************************)
(* PASSTHROUGHS - level-1 actions the binding does not decorate.  The      *)
(* runtime's own steps pass unguarded; the binding's downcalls carry the   *)
(* teardown guard.                                                         *)
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
    \/ \E rtId \in RuntimeIds :
           /\ runtime_root_live
           /\ runtime_dispose_state = "active"
           /\ F!RuntimeCreate(rtId)
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
           /\ F!RefuseLendTooLarge(cId, len)
    \/ \E cId \in CallIds, len \in F!Sizes :
           /\ BindingMayDowncall(cId)
           /\ F!RefuseLendForSlot(cId, len)
    \/ \E cId \in CallIds, len \in F!Sizes, charge \in F!CandidateCharges :
           /\ BindingMayDowncall(cId)
           /\ F!RefuseLendForBudget(cId, len, charge)
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
    \/ PublishRuntimeRoot
    \/ FreeRuntimeRoot
    \/ \E cId \in CallIds :
           \/ PublishCallToken(cId)
           \/ RunContinuation(cId)
           \/ EnterBudgetWait(cId)
           \/ LeaveBudgetWait(cId)
           \/ FinishDisposeCall(cId)
           \/ FreeCallRoot(cId)
           \/ OnEventReturns(cId)
           \/ WriteDoneReturnsQueues(cId)
           \/ ConsumeHeader(cId)
           \/ ConsumePayload(cId)
           \/ BeginDisposeCall(cId)
           \/ DrainRelease(cId)
    \/ \E cId \in CallIds, chId \in ChannelIds : StartCall(cId, chId)
    \/ \E cId \in CallIds, b \in BufferIds : ReturnLentBuffer(cId, b)
    \/ \E cId \in CallIds, b \in BufferIds,
         len \in F!Sizes, charge \in F!Sizes :
           RetryLendSucceeds(cId, b, len, charge)
    \/ \E rtId \in RuntimeIds :
           \/ ShutdownReturns(rtId)
           \/ ResourcesReleasedReturns(rtId)
           \/ BeginDisposeRuntime(rtId)
           \/ FinishDisposeRuntime(rtId)

(***************************************************************************)
(* FAIRNESS                                                                *)
(*                                                                         *)
(* Three tiers.  The runtime's and the FFI dispatch's thirteen families    *)
(* are taken verbatim from level 1 - same actions, same tuple - because    *)
(* the binding restricts none of them: their extraction from Spec is       *)
(* citation.  The binding's own tier is what derives the six host          *)
(* conjuncts of F!Fairness.  The application's tier is a single conjunct   *)
(* per call: eventually consume or dispose - a very liberal contract.      *)
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

BindingOwedFairness ==
    /\ \A cId \in CallIds : WF_vars(OnEventReturns(cId))
    /\ \A cId \in CallIds : WF_vars(WriteDoneReturnsQueues(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(ShutdownReturns(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(ResourcesReleasedReturns(rtId))
    /\ \A cId \in CallIds : WF_vars(ConsumeHeader(cId))
    /\ \A cId \in CallIds : WF_vars(DrainRelease(cId))
    /\ \A cId \in CallIds : WF_vars(FinishDisposeCall(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(FinishDisposeRuntime(rtId))
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_vars(ReturnLentBuffer(cId, b))
    /\ \A cId \in CallIds : WF_vars(RunContinuation(cId))
    /\ \A cId \in CallIds : WF_vars(LeaveBudgetWait(cId))
    /\ \A cId \in CallIds : WF_vars(FreeCallRoot(cId))
    /\ WF_vars(FreeRuntimeRoot)

\* The application's single obligation, per call.
ApplicationOwedFairness ==
    \A cId \in CallIds :
        WF_vars(ConsumePayload(cId) \/ BeginDisposeCall(cId))

Fairness ==
    /\ RuntimeOwedFairness
    /\ BindingOwedFairness
    /\ ApplicationOwedFairness

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* INVARIANTS                                                              *)
(***************************************************************************)

\* The GCHandle exists before ak_call_start can carry it.
TokenPublishedBeforeStart ==
    \A cId \in CallIds :
        ~F!L0!IsUnusedCall(cId) => call_token_published[cId]

\* Every callback in flight resolves its context to a live root.
RootSurvivesCallbacks ==
    \A cId \in CallIds :
        \/ delivery_callback_running[cId]
        \/ write_done_callback_running[cId]
        => call_root_live[cId]

\* The runtime's root outlives every runtime event.
RuntimeRootSurvivesCallbacks ==
    \A rtId \in RuntimeIds :
        \/ shutdown_callback_running[rtId]
        \/ resources_released_callback_running[rtId]
        => runtime_root_live

\* The phase machine and the dispose machine agree: the drain exists only
\* while disposing, done only once disposed.  Single consumer in phases.
ConsumerPhaseMatchesDispose ==
    \A cId \in CallIds :
        /\ consumer_phase[cId] = "drain" <=>
               call_dispose_state[cId] = "draining"
        /\ consumer_phase[cId] = "done" <=>
               call_dispose_state[cId] = "disposed"

\* A call waiting on the budget holds no lent buffer - the deadlock
\* level 1 cannot see, forbidden by construction.
RetryingCallHoldsNoBuffer ==
    \A cId \in CallIds :
        retry_state[cId] = "awaiting_budget" =>
            buffers_held_by_host[cId] = 0

\* Dispose completed means ak_runtime_destroy returned.
DisposeAwaitsDestroy ==
    runtime_dispose_state = "destroyed" =>
        \E rtId \in RuntimeIds : runtime_destroyed[rtId]

\* Inherited corollaries, restated in ring vocabulary: level 1's payload
\* accounting read through the mapping.  Re-proved by citation, not by
\* induction.
RingNeverOverflows ==
    \A cId \in CallIds : RingOccupancy(cId) <= DeliveryCredits + 1

(***************************************************************************)
(* LIVENESS TARGETS - properties the theorems module will state.           *)
(***************************************************************************)

\* A refused-and-waiting call stops waiting once cancellation or dispose
\* arrives.  Conditional by design: no deadline is mandatory, cancellation
\* may never come, and acquisition is never promised.
BudgetCancellationStopsRetry ==
    \A cId \in CallIds :
        (/\ retry_state[cId] = "awaiting_budget"
         /\ (cancel_requested[cId] \/ call_dispose_state[cId] # "active"))
            ~> retry_state[cId] = "idle"

\* Dispose completes: the invoker that began destroying finishes.
RuntimeDisposeCompletes ==
    (runtime_dispose_state = "destroying") ~>
        (runtime_dispose_state = "destroyed")

===============================================================================
