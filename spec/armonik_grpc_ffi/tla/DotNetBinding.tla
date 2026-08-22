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
(*  3. The managed-side invariants: roots, single consumer with its        *)
(*     in-flight reservation, dispose ordering, and the retry protocol.    *)
(*                                                                         *)
(* Scope.  The model is the generic bidirectional-streaming call: the      *)
(* five CallInvoker methods are refinements of it that fix the number of   *)
(* messages in each direction, not separate machines, and are therefore    *)
(* not modelled.  The start/send command queue is level-1 stutter.  The    *)
(* memory model of the ring (release/acquire on the indexes) is a coding   *)
(* rule for review, deliberately outside every level of the specification. *)
(*                                                                         *)
(* The application's obligations are exactly two facts: per call, it       *)
(* eventually begins the next read or disposes the call (one weak          *)
(* fairness), and disposing every call it created is normative - the API   *)
(* requires it, and the same fairness conjunct encodes it.  Everything     *)
(* else - the four callback returns, the buffer return, the drain, the     *)
(* roots - is carried by the binding, under one stated hypothesis: user    *)
(* serialization and parsing terminate.  The disposable wrapper returns    *)
(* the buffer on success and on exception alike; no wrapper can fire       *)
(* inside a call that never returns.                                       *)
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

\* The remembered length's sentinel: one past every request length, so the
\* whole domain stays integer - a string sentinel would make the type
\* heterogeneous, which TLC cannot compare.
NoRetryLen == Ceiling + 2
RuntimeDisposeStates == {"active", "disposing_calls", "destroying",
                         "destroyed"}

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
    /\ consumer_in_flight \in [CallIds -> BOOLEAN]
    /\ pending_continuations \in [CallIds -> Nat]
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
    /\ consumer_in_flight = [c \in CallIds |-> FALSE]
    /\ pending_continuations = [c \in CallIds |-> 0]
    /\ retry_state = [c \in CallIds |-> "idle"]
    /\ retry_len = [c \in CallIds |-> NoRetryLen]
    /\ call_dispose_state = [c \in CallIds |-> "active"]
    /\ runtime_dispose_state = "active"

Init == F!Init /\ ManagedInit

(***************************************************************************)
(* MANAGED-ONLY ACTIONS                                                    *)
(* The level-1 state stutters, which the refinement admits for free.       *)
(***************************************************************************)

\* GCHandle.Alloc(callState): the token exists before any downcall can
\* carry it, and is never allocated into a call or a runtime being torn
\* down - a root allocated after dispose would have no freeing path.
PublishCallToken(cId) ==
    /\ ~call_token_published[cId]
    /\ BindingMayDowncall(cId)
    /\ call_token_published' = [call_token_published EXCEPT ![cId] = TRUE]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<runtime_root_live, consumer_phase, consumer_in_flight,
                   pending_continuations, retry_state, retry_len,
                   call_dispose_state, runtime_dispose_state>>

\* The invoker's root: allocated before the runtime is created, freed only
\* after ak_runtime_destroy returned.  This is the allocation half.
PublishRuntimeRoot ==
    /\ ~runtime_root_live
    /\ runtime_dispose_state = "active"
    /\ runtime_root_live' = TRUE
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, consumer_phase,
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* A completion's continuation runs as its own step, on its own thread:
\* the queue between signal and run is what ContinuationsAsync names, and
\* its weak fairness below is the thread pool's.
RunContinuation(cId) ==
    /\ pending_continuations[cId] > 0
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ - 1]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* MoveNext: the application reserves the sole right to read the current
\* slot.  The reservation is what a parse in flight is; nothing else may
\* release or hand the ring over until it completes.
BeginConsumePayload(cId) ==
    /\ consumer_phase[cId] = "application"
    /\ call_dispose_state[cId] = "active"
    /\ ~consumer_in_flight[cId]
    /\ RingOccupancy(cId) > 0
    /\ consumer_in_flight' = [consumer_in_flight EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* The public DisposeAsync on the invoker: remembered, not deferred - the
\* binding then disposes the calls it still holds, and only when they are
\* settled does the native shutdown begin.
RequestRuntimeDispose ==
    /\ runtime_dispose_state = "active"
    /\ runtime_dispose_state' = "disposing_calls"
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, pending_continuations,
                   retry_state, retry_len, call_dispose_state>>

\* The drain takes the ring once no application read is in flight: the
\* parse completes first, the drain starts behind it, never beside it.
HandoffToDrain(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] \in {"prologue", "application"}
    /\ ~consumer_in_flight[cId]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "drain"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

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
                   consumer_phase, consumer_in_flight, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* The drain finished: the ring is empty and the terminal was released (or
\* the call never started).  Nothing remains to downcall - the runtime
\* reclaims the call on its own fairness (ReleaseCallHandle is its step).
FinishDisposeCall(cId) ==
    /\ call_dispose_state[cId] = "draining"
    /\ consumer_phase[cId] = "drain"
    /\ \/ /\ RingDrained(cId)
          /\ F!L0!HasStatus(cId)
       \/ F!L0!IsUnusedCall(cId)
    /\ call_dispose_state' = [call_dispose_state EXCEPT ![cId] = "disposed"]
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "done"]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, runtime_dispose_state>>

\* A published call that never started has no terminal callback to free
\* its root: its dispose is the freeing path.
FreeUnstartedCallRoot(cId) ==
    /\ call_root_live[cId]
    /\ F!L0!IsUnusedCall(cId)
    /\ call_dispose_state[cId] = "disposed"
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l1_vars
    /\ UNCHANGED <<call_token_published, runtime_root_live, consumer_phase,
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

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
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

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

\* A non-terminal delivery callback returns after publishing the slot and
\* completing the TCS the event answers; the continuation is queued, never
\* run here.
OnEventReturns(cId) ==
    /\ ~F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ + 1]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* The terminal callback's return is the call root's last access: freeing
\* the root is its final instruction, one linearization point, not two.
TerminalCallbackReturns(cId) ==
    /\ F!L0!HasStatus(cId)
    /\ F!DeliveryCallbackReturns(cId)
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ + 1]
    /\ call_root_live' = [call_root_live EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<call_token_published, runtime_root_live,
                   consumer_phase, consumer_in_flight, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* WRITE_DONE completes the pending write's TCS the same way.
WriteDoneReturnsQueues(cId) ==
    /\ F!WriteDoneReturns(cId)
    /\ pending_continuations' =
           [pending_continuations EXCEPT ![cId] = @ + 1]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* The runtime-level callbacks return without touching managed call state.
ShutdownReturns(rtId) ==
    /\ F!ShutdownCallbackReturns(rtId)
    /\ ManagedStutter

ResourcesReleasedReturns(rtId) ==
    /\ F!ResourcesReleasedCallbackReturns(rtId)
    /\ ManagedStutter

\* The prologue owns slot 0: it releases the header and hands the ring to
\* the application.  ak_event_consumed on the INITIAL_METADATA event.  The
\* prologue is the binding's own bounded read, so it needs no in-flight
\* reservation: it is atomic here because nothing of the application runs
\* inside it.
ConsumeHeader(cId) ==
    /\ consumer_phase[cId] = "prologue"
    /\ call_dispose_state[cId] = "active"
    /\ RingTail(cId) = 0
    /\ F!HostConsumesEvent(cId)
    /\ consumer_phase' = [consumer_phase EXCEPT ![cId] = "application"]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_in_flight, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* The reserved read completes: the parse is done, the slot is released,
\* the reservation is given back.  ak_event_consumed under the in-flight
\* right, whatever the dispose machine did meanwhile.
FinishConsumePayload(cId) ==
    /\ consumer_in_flight[cId]
    /\ F!HostConsumesEvent(cId)
    /\ consumer_in_flight' = [consumer_in_flight EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, pending_continuations, retry_state,
                   retry_len, call_dispose_state, runtime_dispose_state>>

\* DisposeAsync on a call - also fired by the invoker's dispose for every
\* call it still holds.  Requests cancellation if the call can still take
\* one; the ring goes to the drain only once no read is in flight, through
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
                   consumer_phase, consumer_in_flight, pending_continuations,
                   retry_state, retry_len, runtime_dispose_state>>

\* The invoker's dispose settles the calls it still holds, one by one.
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
\* exception.  The one hypothesis is that user serialization terminates -
\* no wrapper fires inside a call that never returns.
ReturnLentBuffer(cId, b) ==
    /\ F!HostReturnsBuffer(cId, b)
    /\ ManagedStutter

\* The budget refusal itself enters the wait: the cause and the state are
\* one step, so no trace observes BUDGET_BUSY without the retry loop it
\* promises.  The refused length is remembered - the wait is about this
\* request.  Eligibility (ContemplatesLend) already requires the call to
\* hold no buffer, which is RetryingCallHoldsNoBuffer at the door.
ObserveBudgetRefusal(cId, len, charge) ==
    /\ retry_state[cId] = "idle"
    /\ BindingMayDowncall(cId)
    /\ ~cancel_requested[cId]
    /\ F!RefuseLendForBudget(cId, len, charge)
    /\ retry_state' = [retry_state EXCEPT ![cId] = "awaiting_budget"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = len]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* The poll refused again: same request, same wait.  No other length may
\* be asked while the call waits.
RetryBudgetRefusedAgain(cId, charge) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ BindingMayDowncall(cId)
    /\ F!RefuseLendForBudget(cId, retry_len[cId], charge)
    /\ ManagedStutter

\* The retry succeeding: ak_get_call_buffer returns OK for the remembered
\* request and the wait ends inside the same downcall.  The lend carries
\* no fairness - the poll is never owed a grant - so this action carries
\* none either.
RetryLendSucceeds(cId, b, charge) ==
    /\ retry_state[cId] = "awaiting_budget"
    /\ BindingMayDowncall(cId)
    /\ F!LendSendBuffer(cId, b, retry_len[cId], charge)
    /\ retry_state' = [retry_state EXCEPT ![cId] = "idle"]
    /\ retry_len' = [retry_len EXCEPT ![cId] = NoRetryLen]
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, pending_continuations,
                   call_dispose_state, runtime_dispose_state>>

\* Every call settled: the native shutdown begins.  A published call must
\* have been disposed; an untouched slot needs nothing.
BeginRuntimeShutdown(rtId) ==
    /\ runtime_dispose_state = "disposing_calls"
    /\ \A c \in CallIds :
           \/ call_dispose_state[c] = "disposed"
           \/ /\ F!L0!IsUnusedCall(c)
              /\ ~call_token_published[c]
    /\ F!RuntimeBeginShutdown(rtId)
    /\ runtime_dispose_state' = "destroying"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, pending_continuations,
                   retry_state, retry_len, call_dispose_state>>

\* ak_runtime_destroy returns AK_STATUS_OK: dispose may complete.  A
\* runtime that failed instead never reaches this - the liveness carries
\* the ~NotFailed escape, like every level-0 and level-1 promise.
FinishDisposeRuntime(rtId) ==
    /\ runtime_dispose_state = "destroying"
    /\ F!RuntimeDestroy(rtId)
    /\ runtime_dispose_state' = "destroyed"
    /\ UNCHANGED <<call_token_published, call_root_live, runtime_root_live,
                   consumer_phase, consumer_in_flight, pending_continuations,
                   retry_state, retry_len, call_dispose_state>>

(***************************************************************************)
(* PASSTHROUGHS - level-1 actions the binding does not decorate.  The      *)
(* runtime's own steps pass unguarded; the binding's downcalls carry the   *)
(* teardown guard, and the lend family additionally the retry discipline:  *)
(* while a call waits on the budget, only the remembered request runs,     *)
(* through the coupled actions above.                                      *)
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
           /\ retry_state[cId] = "idle"
           /\ F!RefuseLendTooLarge(cId, len)
    \/ \E cId \in CallIds, len \in F!Sizes :
           /\ BindingMayDowncall(cId)
           /\ retry_state[cId] = "idle"
           /\ F!RefuseLendForSlot(cId, len)
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
    \/ RequestRuntimeDispose
    \/ FreeRuntimeRoot
    \/ \E cId \in CallIds :
           \/ PublishCallToken(cId)
           \/ RunContinuation(cId)
           \/ BeginConsumePayload(cId)
           \/ FinishConsumePayload(cId)
           \/ HandoffToDrain(cId)
           \/ LeaveBudgetWait(cId)
           \/ FinishDisposeCall(cId)
           \/ FreeUnstartedCallRoot(cId)
           \/ OnEventReturns(cId)
           \/ TerminalCallbackReturns(cId)
           \/ WriteDoneReturnsQueues(cId)
           \/ ConsumeHeader(cId)
           \/ BeginDisposeCall(cId)
           \/ DisposeCallForRuntime(cId)
           \/ DrainRelease(cId)
    \/ \E cId \in CallIds, chId \in ChannelIds : StartCall(cId, chId)
    \/ \E cId \in CallIds, b \in BufferIds : ReturnLentBuffer(cId, b)
    \/ \E cId \in CallIds, len \in F!Sizes, charge \in F!CandidateCharges :
           ObserveBudgetRefusal(cId, len, charge)
    \/ \E cId \in CallIds, charge \in F!CandidateCharges :
           RetryBudgetRefusedAgain(cId, charge)
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
\* terminate - the wrapper covers success and exception, nothing covers a
\* call that never returns.
BindingOwedFairness ==
    /\ \A cId \in CallIds : WF_vars(OnEventReturns(cId))
    /\ \A cId \in CallIds : WF_vars(TerminalCallbackReturns(cId))
    /\ \A cId \in CallIds : WF_vars(WriteDoneReturnsQueues(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(ShutdownReturns(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(ResourcesReleasedReturns(rtId))
    /\ \A cId \in CallIds : WF_vars(ConsumeHeader(cId))
    /\ \A cId \in CallIds : WF_vars(FinishConsumePayload(cId))
    /\ \A cId \in CallIds : WF_vars(HandoffToDrain(cId))
    /\ \A cId \in CallIds : WF_vars(DrainRelease(cId))
    /\ \A cId \in CallIds : WF_vars(FinishDisposeCall(cId))
    /\ \A cId \in CallIds : WF_vars(DisposeCallForRuntime(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(BeginRuntimeShutdown(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(FinishDisposeRuntime(rtId))
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_vars(ReturnLentBuffer(cId, b))
    /\ \A cId \in CallIds : WF_vars(RunContinuation(cId))
    /\ \A cId \in CallIds : WF_vars(LeaveBudgetWait(cId))
    /\ \A cId \in CallIds : WF_vars(FreeUnstartedCallRoot(cId))
    /\ WF_vars(FreeRuntimeRoot)

\* The application's single obligation, per call: begin the next read or
\* dispose.  Disposing every call it created is the normative half.
ApplicationOwedFairness ==
    \A cId \in CallIds :
        WF_vars(BeginConsumePayload(cId) \/ BeginDisposeCall(cId))

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

\* The in-flight reservation exists only where the application reads; the
\* boolean is what makes the reader unique.
AtMostOneConsumerInFlight ==
    \A cId \in CallIds :
        consumer_in_flight[cId] => consumer_phase[cId] = "application"

\* The drain never runs beside an application read.
DrainNeverOverlapsApplicationConsumer ==
    \A cId \in CallIds :
        consumer_phase[cId] \in {"drain", "done"} =>
            ~consumer_in_flight[cId]

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
(* contract's one admitted way out, and DisposeAsync then surfaces the     *)
(* failure rather than a clean teardown.                                   *)
(***************************************************************************)

\* A refused-and-waiting call stops waiting once cancellation or dispose
\* arrives.  Conditional by design: no deadline is mandatory, cancellation
\* may never come, and acquisition is never promised.
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

\* Every allocated root dies: the call's at its terminal callback or its
\* unstarted dispose, the invoker's after destroy.
CallRootEventuallyFreed ==
    \A cId \in CallIds :
        call_root_live[cId] ~>
            (~call_root_live[cId] \/ ~F!L0!NotFailed)

RuntimeRootEventuallyFreed ==
    (runtime_root_live /\ runtime_dispose_state # "active") ~>
        (~runtime_root_live \/ ~F!L0!NotFailed)

\* A reserved read completes: the parse ends and the slot is released -
\* under the stated hypothesis that user parsing terminates, which the
\* binding's WF on FinishConsumePayload encodes.
InFlightPayloadEventuallyReleased ==
    \A cId \in CallIds :
        consumer_in_flight[cId] ~> ~consumer_in_flight[cId]

\* Once the terminal is in, no callback queues anything more, so the
\* thread pool drains the queue.
QueuedContinuationEventuallyRuns ==
    \A cId \in CallIds :
        (pending_continuations[cId] > 0 /\ F!L0!HasStatus(cId)) ~>
            (pending_continuations[cId] = 0 \/ ~F!L0!NotFailed)

===============================================================================
