----------------------------- MODULE FfiGrpc -------------------------------
(***************************************************************************)
(* Level 1 - The FFI boundary refined over AbstractGrpc.                   *)
(*                                                                         *)
(* The state is shared, the machinery is instantiated: the constants and   *)
(* the twelve level-0 variables come from AbstractGrpcState (declared      *)
(* once, never redeclared), this module adds the FFI constants and the     *)
(* twenty-three FFI variables, and every level-0 definition is reached     *)
(* through the L0 prefix.  Every local name (vars, TypeOK, Init, Next,     *)
(* Spec, the action names) denotes the level-1 concept.                    *)
(*                                                                         *)
(* Every action either refines a level-0 action (conjoining FFI guards     *)
(* and updates onto the L0 engine) or stutters on the level-0 variables.   *)
(* Writing a level-0 variable outside an L0!action is the tell that the    *)
(* refinement rule is being broken: the level-0 machinery is the only      *)
(* legal writer of the level-0 state.                                      *)
(*                                                                         *)
(* The FFI adds two per-call disciplines, each with a configurable         *)
(* pipelining depth, and both are FIFO:                                    *)
(*  - the delivery side: one callback at a time, at most DeliveryCredits   *)
(*    unreleased payloads (one more when the last is a terminal, so a      *)
(*    terminal is never blocked by unread messages); a payload is the      *)
(*    index of its event, and release follows delivery order;              *)
(*  - the send side: at most MaxSendsInFlight buffers out of the call's    *)
(*    arena at once, counting both those the host is filling and those     *)
(*    already committed; a send is the index of its message, WRITE_DONE    *)
(*    acquits in send order, always arrives, exactly once, and always      *)
(*    before the terminal.                                                 *)
(*                                                                         *)
(* Each side is therefore one monotone counter against a level-0 sequence, *)
(* and the outstanding objects are the gap between them.  The per-object   *)
(* guarantees survive: payload k is owed while the released count is below *)
(* k, so consuming in order still discharges every payload individually.   *)
(*                                                                         *)
(* Both borrowings are call-scoped, and the call is reclaimed by the       *)
(* runtime rather than by a downcall: once the terminal is past and        *)
(* everything lent is back, the actor retires the handle itself, and       *)
(* nothing can be lent or delivered afterwards.  The ABI has no            *)
(* ak_call_release; ak_runtime_destroy stays a downcall because only the   *)
(* host can ask whether it may unload.                                     *)
(*                                                                         *)
(* Six fairness conjuncts are obligations on the host, not promises of     *)
(* the runtime: WF on HostConsumesEvent and WF on HostReturnsBuffer, which *)
(* say the host gives back what it holds - one per payload stream, one per *)
(* buffer, because payload release is FIFO and buffer returns are not -    *)
(* and WF on DeliveryCallbackReturns, WriteDoneReturns,                    *)
(* ShutdownCallbackReturns and ResourcesReleasedCallbackReturns, which say *)
(* the callback the host installed eventually returns.  Our own binding    *)
(* implements all of them; a generic host is required to.  Every other     *)
(* conjunct is the runtime's own thread.                                   *)
(* Shutdown needs nothing the host *consumes*, but it does wait for the    *)
(* host's shutdown callback to return, so "no host action" would overstate *)
(* it.                                                                     *)
(***************************************************************************)

\* Functions for SumFunctionOnSet: the budget's total is a sum of a function
\* over a set of indices.  First order in both arguments, which is what lets
\* the primed total be written at all - tlapm does not distribute a prime
\* through a higher-order application.  Its theory is FunctionTheorems.
EXTENDS FfiGrpcState, Naturals, Sequences, Functions

(***************************************************************************)
(* The whole state comes from FfiGrpcState: the constants and the twelve   *)
(* level-0 variables (shared through AbstractGrpcState, they feed the      *)
(* implicit substitution of the INSTANCE below) plus the FFI constants and *)
(* the twenty-three FFI variables.  Nothing is declared here.              *)
(***************************************************************************)

\* The level-0 engine: definitions and proved theorems, all under L0.
L0 == INSTANCE AbstractGrpcTheorems

\* The payload identity space.  TLC cannot expand a property quantified
\* over Nat, so the MC configurations override this definition with a
\* finite interval covering every reachable index.
PayloadIndices == Nat

ffi_vars == <<buffers_held_by_host,
              write_dones_emitted, write_done_callback_running,
              delivery_callback_running, payloads_consumed_by_host,
              handle_released, cancel_requested,
              shutdown_event_emitted, shutdown_callback_running,
              runtime_destroyed, buffer_state, buffer_send,
              second_event_owed, resources_released_emitted,
              resources_released_callback_running,
              last_lend_status,
              buffer_charge, buffer_length, memory_used, engine_held,
              read_admitted, lend_waiting, budget_wake_owed>>

\* The level-0 state, under a local name for the stuttering actions.  TLC
\* cannot resolve an instantiated tuple inside ENABLED, so the MC
\* configurations override this definition with a spelled-out copy.
l0_vars == L0!vars

\* The level-1 state is the level-0 state plus the FFI state.
vars == <<l0_vars, ffi_vars>>

\* --- DOMAIN-DRIVEN GROUPINGS ---
RuntimeVars == <<runtime_state, shutdown_event_emitted,
                 shutdown_callback_running, runtime_destroyed,
                 second_event_owed, resources_released_emitted,
                 resources_released_callback_running>>
ChannelVars == <<channel_state, channel_runtime>>
CallVars    == <<call_state, call_channel, handle_released,
                 cancel_requested>>
MessageVars == <<submitted, sent, received, delivered, events_delivered,
                 send_closed, status_pending, buffers_held_by_host,
                 write_dones_emitted, write_done_callback_running,
                 payloads_consumed_by_host, delivery_callback_running,
                 buffer_state, buffer_send>>

(***************************************************************************)
(* STATE PREDICATES                                                        *)
(* Guards, invariants and properties speak through these; reading a        *)
(* variable directly is reserved to update expressions, Init and TypeOK.   *)
(***************************************************************************)

\* WRITE_DONEs fully acquitted: emitted and their callback returned.  The
\* callback discipline is one at a time, always for the latest emission.
WriteDonesReturned(cId) ==
    write_dones_emitted[cId] -
        (IF write_done_callback_running[cId] THEN 1 ELSE 0)

\* What the send window holds: the buffers the host is still filling, plus
\* the accepted sends whose WRITE_DONE has not gone out.  The slot is
\* freed when WRITE_DONE is *emitted*, not when its callback returns -
\* by then the send is settled - written to the transport or abandoned
\* because the call ended - and the return of a callback is not
\* something the host can observe, so nothing it must wait for may be
\* gated on it.  The callback still on the stack is a separate notion,
\* carried by write_done_callback_running and used only for quiescence
\* and for the terminal.
SendWindowOccupancy(cId) ==
    buffers_held_by_host[cId] + Len(submitted[cId]) - write_dones_emitted[cId]

\* The host has a buffer to write into, or has given everything back.
HostHoldsSomeBuffer(cId) == buffers_held_by_host[cId] > 0
HostHoldsNoBuffer(cId) == buffers_held_by_host[cId] = 0

\* No unacquitted send lives in this buffer.  A send is acquitted once its
\* WRITE_DONE has gone out, which is also when its slot came back, so this
\* is the memory half of the same event.
\* Zero means the buffer carries no send, and zero is below every count, so
\* a buffer given back unused is always free to go.
CarriesNoUnacquittedSend(cId, b) ==
    buffer_send[cId][b] <= write_dones_emitted[cId]

\* The per-buffer lifecycle.  Monotone along none -> lent -> returned ->
\* freed, and an identity is never reused, which is what makes the states
\* a chain rather than a cycle.  "returned" is the host's step, "freed" is
\* the runtime's: the transport may still read the bytes past the return.
BufferStates == {"none", "lent", "returned", "freed"}
IsFreshBuffer(cId, b) == buffer_state[cId][b] = "none"
IsLentBuffer(cId, b) == buffer_state[cId][b] = "lent"
IsReturnedBuffer(cId, b) == buffer_state[cId][b] = "returned"
IsFreedBuffer(cId, b) == buffer_state[cId][b] = "freed"
\* What the host owes on a buffer: it was handed one and has not given it
\* back.  The liveness obligation is stated on this.
HostOwesBuffer(cId, b) == IsLentBuffer(cId, b)
\* What the runtime owes: the bytes are back but not yet released.
RuntimeOwesFree(cId, b) == IsReturnedBuffer(cId, b)

\* At least one accepted send has no WRITE_DONE yet.
IsAwaitingWriteDone(cId) ==
    write_dones_emitted[cId] < Len(submitted[cId])

\* The WRITE_DONE callback is on the host stack.
IsWriteDoneCallbackRunning(cId) == write_done_callback_running[cId]

\* Capacity left: ak_get_call_buffer may lend one more.  A host woken by
\* WRITE_DONE therefore always finds the slot it was promised.
HasFreeSendSlot(cId) == SendWindowOccupancy(cId) < MaxSendsInFlight

\* The send side is drained: every accepted send acquitted, the last
\* acquittal callback returned.  Guards the terminals and quiescence.
HasNoSendInFlight(cId) ==
    /\ write_dones_emitted[cId] = Len(submitted[cId])
    /\ ~write_done_callback_running[cId]

\* The send-side identities: send k exists once the level-0 machinery has
\* accepted the k-th message, and is acquitted once its in-order
\* WRITE_DONE has been emitted and has returned.
HasAcceptedSendAt(cId, k) == Len(submitted[cId]) >= k
IsSendAcquittedAt(cId, k) == WriteDonesReturned(cId) >= k

\* A data callback (metadata, message or terminal) is on the host stack.
IsDeliveryCallbackRunning(cId) == delivery_callback_running[cId]

\* ak_call_cancel has latched the request, or the runtime has latched it
\* on behalf of a closing channel.
IsCancelRequested(cId) == cancel_requested[cId]

\* The runtime has retired the handle: no further downcall on it, and
\* nothing of the call is lent out any more.
IsHandleReleased(cId) == handle_released[cId]

\* A payload is the index of its event in the level-0 events_delivered
\* sequence.  Release is in delivery order, so the counter names the
\* released prefix and the host still holds everything above it.
OwedPayloads(cId) ==
    Len(events_delivered[cId]) - payloads_consumed_by_host[cId]
HostOwnsPayload(cId, k) ==
    /\ k <= Len(events_delivered[cId])
    /\ k > payloads_consumed_by_host[cId]
HostOwnsNoPayload(cId) == OwedPayloads(cId) = 0
HostOwnsSomePayload(cId) == OwedPayloads(cId) > 0
HostHasDeliveryCredit(cId) == OwedPayloads(cId) < DeliveryCredits
HostOwnsAtMostCredits(cId) == OwedPayloads(cId) <= DeliveryCredits

HostOwnsAtMostCreditsPlusOne(cId) ==
    OwedPayloads(cId) <= DeliveryCredits + 1

\* A send buffer is outstanding: lent to the host, or given back and not yet
\* freed - the buffers the budget is holding on the send side.  Named by the
\* states the model has instead of by
\* the bytes it does not, and bounded by a constant, BufferIds being finite
\* with each buffer used once, which is what keeps the relief argument finite
\* rather than a well-founded induction.
BufferOutstanding(cId, b) ==
    IsLentBuffer(cId, b) \/ IsReturnedBuffer(cId, b)

SomeBufferOutstanding ==
    \E cId \in CallIds, b \in BufferIds : BufferOutstanding(cId, b)

\* --- THE EMISSION BUDGET, IN BYTES ---
\* Every comparison lives in a predicate.  A bare inequality in a property is
\* two unrelated atoms to the temporal backend, which cannot then see that one
\* is the negation of the other; named, it is one atom and its negation.

\* A request the ABI will consider at all: no larger than the ceiling.  Above
\* that the refusal is AK_STATUS_MESSAGE_TOO_LARGE, permanent, and the
\* complement of this predicate is exactly that condition.  Zero is no request:
\* ak_get_call_buffer refuses it as an invalid argument, and an empty message
\* goes through ak_call_send_message with no buffer, a send this model does not
\* represent - it takes no memory, so nothing here would tell it apart.
IsLendable(len) == len <= Ceiling

\* Room for a charge right now.  Its negation is what AK_STATUS_BUDGET_BUSY
\* reports, through RefuseLendForBudget: a refusal for the charge the
\* allocator picked, recorded per request, while a smaller charge may fit.
IsMemoryAvailable(charge) == memory_used + charge <= Ceiling

\* Room for a charge once the charge of the buffer it replaces is off: an
\* exchange is one step for the counter, which sees the difference alone.
\* The first threshold is asked only for a growth: received messages may
\* have taken the counter past it, and giving memory back is never refused.
IsMemoryAvailableForExchange(cId, b, charge) ==
    \/ charge <= buffer_charge[<<cId, b>>]
    \/ memory_used - buffer_charge[<<cId, b>>] + charge <= Ceiling

\* The allocator hands out at least what was asked.  The budget charges what
\* it handed out, not what was asked: the ceiling then bounds the bytes the
\* runtime really holds.  Nothing here models a rounding policy.
CoversRequest(charge, len) == len <= charge

\* The sizes a lend may be asked for.  Ceiling + 1 is the abstract
\* representative of every length or charge strictly above the plafond: no
\* state retains the exact value, so one witness is as good as them all.  A
\* successful charge is necessarily under the ceiling; a refused one need not
\* be, which is what makes MESSAGE_TOO_LARGE and the budget refusal of an
\* oversized class representable at all.
Sizes == 0..Ceiling
RequestLengths == 0..(Ceiling + 1)
CandidateCharges == 0..(Ceiling + 1)

LendStatuses ==
    {"NONE", "OK", "SLOT_BUSY", "BUDGET_BUSY", "MESSAGE_TOO_LARGE"}

HasAccountingRoomForSomeCharge(len) ==
    \E charge \in Sizes :
        CoversRequest(charge, len) /\ IsMemoryAvailable(charge)

\* The occurrence discipline, both directions.  A token names one occurrence:
\* once globally within its direction, on whichever call committed it, and
\* never in both directions - a received token cannot reappear in emission,
\* nor an emitted one in reception.  Position already orders each direction;
\* the tokens are what buffer_send and the k-th WRITE_DONE key on.
NeverSubmitted(msg) ==
    \A c \in CallIds :
        \A i \in DOMAIN submitted[c] : submitted[c][i] # msg

NeverReceived(msg) ==
    \A c \in CallIds :
        \A i \in DOMAIN received[c] : received[c][i] # msg

\* A message fits the buffer it was given: the commit declares how many bytes
\* the host wrote, the message's length, and that is at most what was lent.
\* Read at the commit, where the message appears - the lend saw only a
\* length.
FitsInBuffer(msg, cId, b) ==
    MessageLength[msg] <= buffer_length[<<cId, b>>]

\* The buffers the budget is holding, as a set of pairs, and the charge of
\* one.  Named so the sums below are folds over an atom.
OutstandingPairs ==
    {q \in CallIds \X BufferIds : BufferOutstanding(q[1], q[2])}
LentPairs ==
    {q \in CallIds \X BufferIds : IsLentBuffer(q[1], q[2])}
InFlightPairs ==
    {q \in CallIds \X BufferIds :
        IsReturnedBuffer(q[1], q[2]) /\ ~CarriesNoUnacquittedSend(q[1], q[2])}
HeldPairs ==
    {q \in CallIds \X BufferIds :
        IsReturnedBuffer(q[1], q[2]) /\ CarriesNoUnacquittedSend(q[1], q[2])}

\* The four numbers the ABI publishes, as sums over those sets.  Definitions
\* rather than counters: what the implementation maintains is memory_used, and
\* MemoryAccountingExact is what says it agrees with these.  The partition
\* identity between the three categories and the total is a lemma, not an
\* invariant - it holds of any state, so there is nothing to preserve.
BytesOutstanding  == SumFunctionOnSet(buffer_charge, OutstandingPairs)
BytesHostLent     == SumFunctionOnSet(buffer_charge, LentPairs)
BytesSendInFlight == SumFunctionOnSet(buffer_charge, InFlightPairs)
BytesRuntimeHeld  == SumFunctionOnSet(buffer_charge, HeldPairs)

\* The sixth category: what the engine keeps for itself.  One number, so no
\* sum to take.
BytesHeldByEngine == engine_held

HasEngineHeldBytes == 0 < engine_held
IsEngineHoldingAtLeast(n) == n <= engine_held

\* The engine's total stays within the first threshold.  Implied by the room
\* the ceiling has once the accounting holds, and kept as a guard so that the
\* type of the held bytes is proved without reaching for an invariant, as
\* HostReturnsBuffer keeps its count.
IsEngineRoomAvailable(n) == engine_held + n <= Ceiling

\* --- THE RECEIVE SIDE, IN BYTES ---

\* The trailers are in: level 0 receives nothing more on the call.
IsStatusPending(cId) == status_pending[cId]

\* A read is two steps.  Admitted, the call may read its next message; the
\* decision is taken there.  Reads stop at the first threshold, lowered by the
\* length every refused send waits on, so a send refused for room is served
\* before new reads.
IsReadAdmitted(cId) == read_admitted[cId]
\* The engine reads the next message once it has handed the last to the
\* delivery, so a call holds at most one decoded message it has not delivered.
HasDeliveredAllReceived(cId) == Len(delivered[cId]) = Len(received[cId])
IsLendWaiting(cId) == lend_waiting[cId] > 0
IsLendWaitingFor(cId, len) == lend_waiting[cId] = len
IsReadAdmissible ==
    \A c \in CallIds : memory_used + lend_waiting[c] < Ceiling
\* The second step: a decoded message the second threshold lets the engine
\* keep.
IsWithinHardCeiling(len) == memory_used + len <= HardCeiling

IsBudgetWakeOwed(cId) == budget_wake_owed[cId]

\* The received messages of a call whose bytes are back: those the host
\* consumed.  Release follows delivery order and the first payload is the
\* initial metadata, so the k-th payload consumed is the (k-1)-th message.
\* Past the last message the count names messages that do not exist, which
\* holds nothing: the accounting below reads it only as a lower bound.
MessagesReleased(cId) ==
    IF payloads_consumed_by_host[cId] = 0 THEN 0
    ELSE payloads_consumed_by_host[cId] - 1

\* The last received message whose bytes are held.  A call that ended keeps
\* what it delivered and nothing else: what it received and never delivered
\* went with it.
HeldReceivedTop(cId) ==
    IF L0!IsTerminalCall(cId) THEN Len(delivered[cId]) ELSE Len(received[cId])

\* The bytes of each received message, keyed like the buffer charges.
\* PayloadIndices rather than Nat, for the reason it exists.
ReceivedLength ==
    [q \in CallIds \X PayloadIndices |->
        IF q[2] \in 1..Len(received[q[1]])
        THEN MessageLength[received[q[1]][q[2]]]
        ELSE 0]

\* The received messages whose bytes are held, and the two categories they
\* fall in: delivered, the host holds them; not yet delivered, the engine
\* does.  Defined from the counts alone, so the accounting needs no level-0
\* invariant and holds past a failure, as the send side's does.
ReceivedPairs ==
    {q \in CallIds \X PayloadIndices :
        MessagesReleased(q[1]) < q[2] /\ q[2] <= HeldReceivedTop(q[1])}
HostReceivedPairs ==
    {q \in ReceivedPairs : q[2] <= Len(delivered[q[1]])}
RuntimeReceivedPairs ==
    {q \in ReceivedPairs : Len(delivered[q[1]]) < q[2]}

BytesHostReceived    == SumFunctionOnSet(ReceivedLength, HostReceivedPairs)
BytesRuntimeReceived == SumFunctionOnSet(ReceivedLength, RuntimeReceivedPairs)
BytesReceived        == SumFunctionOnSet(ReceivedLength, ReceivedPairs)

\* What the next consumption gives back: the bytes of the message it releases,
\* none when the payload is the metadata or the terminal - the pair it would
\* release is then held by nothing.
NextConsumedLength(cId) ==
    LET k == payloads_consumed_by_host[cId]
    IN  IF 1 <= k /\ k <= HeldReceivedTop(cId)
        THEN ReceivedLength[<<cId, k>>]
        ELSE 0

\* What a call that ends without delivering them gives back: the held pairs
\* past its deliveries.
UndeliveredPairs(cId) ==
    {q \in {cId} \X PayloadIndices :
        /\ MessagesReleased(cId) < q[2]
        /\ Len(delivered[cId]) < q[2]
        /\ q[2] <= Len(received[cId])}
UndeliveredBytes(cId) ==
    SumFunctionOnSet(ReceivedLength, UndeliveredPairs(cId))

\* A release that gives bytes back owes a wake-up to every call whose send
\* waits; a call that ends waits on nothing more.
OweBudgetWakeToWaitingCalls ==
    budget_wake_owed' =
        [c \in CallIds |-> IsBudgetWakeOwed(c) \/ IsLendWaiting(c)]
EndWaitOf(cId) ==
    /\ lend_waiting' = [lend_waiting EXCEPT ![cId] = 0]
    /\ budget_wake_owed' = [budget_wake_owed EXCEPT ![cId] = FALSE]

\* The SHUTDOWN_COMPLETE discipline, per runtime.
IsShutdownEventEmitted(rtId) == shutdown_event_emitted[rtId]
IsShutdownCallbackRunning(rtId) == shutdown_callback_running[rtId]

\* The tag SHUTDOWN_COMPLETE carried, and the second event it promises when the
\* tag was set.  Recorded rather than recomputed, because the promise is about
\* what was true at the emission: a host told nothing is owed is owed no
\* second callback, whatever happens later.
SecondEventOwed(rtId) == second_event_owed[rtId]
IsResourcesReleasedEmitted(rtId) == resources_released_emitted[rtId]
IsResourcesReleasedCallbackRunning(rtId) ==
    resources_released_callback_running[rtId]

\* ak_runtime_destroy has been accepted: every handle of the runtime is
\* void and its memory is gone.
IsRuntimeDestroyed(rtId) == runtime_destroyed[rtId]

\* The failure is a state of the runtime, and it is absorbing: no action
\* writes a slot out of it.  FailedRuntimeAbsorbing is the citable form.
IsFailedRuntime(rtId) == runtime_state[rtId] = "FAILED_UNQUIESCED"

\* Level-0 state read through named predicates: the level-1 spec never
\* compares a level-0 variable to a literal outside these.
IsStoppingRuntime(rtId) == runtime_state[rtId] = "STOPPING"
IsReleasedRuntime(rtId) == runtime_state[rtId] = "RELEASED"
IsClosingChannel(chId) == channel_state[chId] = "closing"
IsClosedChannel(chId) == channel_state[chId] = "closed"
HasNoDeliveredEvents(cId) == events_delivered[cId] = <<>>

\* The delivery slot is open: no callback on the host stack and at least
\* one credit left.  Guards metadata and message delivery (backpressure).
HasFreeDeliverySlot(cId) ==
    /\ ~IsDeliveryCallbackRunning(cId)
    /\ HostHasDeliveryCredit(cId)

\* The relaxed form terminals use: a terminal may go out with every
\* credit spent, so a terminal is never blocked by unread messages.
HasFreeDeliverySlotForTerminal(cId) ==
    /\ ~IsDeliveryCallbackRunning(cId)
    /\ HostOwnsAtMostCredits(cId)

\* Every delivery hands one payload to the host and runs its callback.
\* The payload is the event the level-0 action appends in the same step,
\* so nothing has to be recorded here: the debt is the gap between the
\* event count and the released count, and the event count just grew.
HandPayloadToHost(cId) ==
    /\ delivery_callback_running' =
           [delivery_callback_running EXCEPT ![cId] = TRUE]
    /\ UNCHANGED payloads_consumed_by_host

\* Cancellation is latched on every active call of a channel set.  Used by
\* both closing paths: closing a channel cancels its calls, and shutdown
\* closes every channel of the runtime.
RequestCancellationOfActiveCalls(chs) ==
    cancel_requested' = [cId \in CallIds |->
        IF call_channel[cId] \in chs /\ L0!IsActiveCall(cId)
        THEN TRUE
        ELSE IsCancelRequested(cId)]

\* Nothing of the runtime's memory is in the host's hands: every call of
\* every channel of the runtime has given its payloads and its buffers
\* back.  Handles are not part of this - destroying voids them - because a
\* handle names runtime state, while a payload or a lent buffer is memory
\* the host may still be reading or writing.  A host that forgets to give
\* something back must leave the runtime untidy rather than turn shutdown
\* into something that never completes.
\* A call whose runtime has been destroyed.  Its handle names freed
\* runtime state, so every downcall on it is refused - AK_HANDLE_STALE
\* at the ABI, not enabled here.  False on an unused call, which has no
\* owner yet.
IsRuntimeOfCallDestroyed(cId) ==
    \E rtId \in RuntimeIds :
        /\ call_channel[cId] \in L0!ChannelsOf(rtId)
        /\ IsRuntimeDestroyed(rtId)

\* What the host still holds of the runtime, and nothing else: a payload not
\* consumed or a buffer not given back.  This is what the tag reports, because
\* it is what the host can act on - a buffer already given back is the
\* runtime's, and the host could do nothing about it.
NoHostDebt(rtId) ==
    \A cId \in CallIds :
        call_channel[cId] \in L0!ChannelsOf(rtId) =>
            /\ HostOwnsNoPayload(cId)
            /\ HostHoldsNoBuffer(cId)

\* And what the runtime still owes itself: bytes given back but not released.
\* Needs nothing from the host, so folding it into the unload condition costs
\* no fairness hypothesis on the host.
RuntimeHoldsNoReturnedBytes(rtId) ==
    \A cId \in CallIds :
        call_channel[cId] \in L0!ChannelsOf(rtId) =>
            \A b \in BufferIds : ~IsReturnedBuffer(cId, b)

\* AK_RUNTIME_QUIESCENT: the level-0 RELEASED state plus an empty ledger.  The
\* level-0 model has no notion of a handle to free, so both observable statuses
\* refine that one state and the difference is carried here.  The last callback
\* having returned is part of it: a callback runs on the runtime's own thread,
\* so no callback can ever report that the thread is gone - only this can.
IsRuntimeQuiescent(rtId) ==
    /\ IsReleasedRuntime(rtId)
    /\ ~IsShutdownCallbackRunning(rtId)
    /\ ~IsResourcesReleasedCallbackRunning(rtId)
    /\ (SecondEventOwed(rtId) => IsResourcesReleasedEmitted(rtId))
    /\ NoHostDebt(rtId)
    /\ RuntimeHoldsNoReturnedBytes(rtId)

\* The runtime has nothing left to run: every channel closed (their calls
\* are then terminal by the level-0 invariant), no delivery callback on
\* the host stack, every send side drained.  SHUTDOWN_COMPLETE is emitted
\* only from here, which is what makes it the last callback of the functional
\* shutdown.  Unconsumed payloads do not block that drain: the host may consume
\* them later, and what they do block is the quiescence the status reports.
IsRuntimeDrained(rtId) ==
    /\ \A chId \in L0!ChannelsOf(rtId) : IsClosedChannel(chId)
    /\ \A cId \in CallIds :
           call_channel[cId] \in L0!ChannelsOf(rtId) =>
               /\ ~IsDeliveryCallbackRunning(cId)
               /\ HasNoSendInFlight(cId)

(***************************************************************************)
(* TYPE INVARIANT AND INITIAL STATE                                        *)
(***************************************************************************)

TypeOK ==
    /\ L0!TypeOK
    /\ buffers_held_by_host \in [CallIds -> Nat]
    /\ write_dones_emitted \in [CallIds -> Nat]
    /\ write_done_callback_running \in [CallIds -> BOOLEAN]
    /\ delivery_callback_running \in [CallIds -> BOOLEAN]
    /\ payloads_consumed_by_host \in [CallIds -> Nat]
    /\ handle_released \in [CallIds -> BOOLEAN]
    /\ cancel_requested \in [CallIds -> BOOLEAN]
    /\ shutdown_event_emitted \in [RuntimeIds -> BOOLEAN]
    /\ shutdown_callback_running \in [RuntimeIds -> BOOLEAN]
    /\ runtime_destroyed \in [RuntimeIds -> BOOLEAN]
    /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
    /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
    /\ second_event_owed \in [RuntimeIds -> BOOLEAN]
    /\ resources_released_emitted \in [RuntimeIds -> BOOLEAN]
    /\ resources_released_callback_running \in [RuntimeIds -> BOOLEAN]
    /\ last_lend_status \in [CallIds -> LendStatuses]
    /\ buffer_charge \in [CallIds \X BufferIds -> Nat]
    /\ buffer_length \in [CallIds \X BufferIds -> Nat]
    \* Int, not Nat: the free subtracts, so staying in Nat would need the
    \* accounting inside the type proof.  MemoryAccountingExact pins it to a
    \* sum of naturals, which is where non-negativity belongs.
    /\ memory_used \in Int
    /\ engine_held \in Sizes
    /\ read_admitted \in [CallIds -> BOOLEAN]
    /\ lend_waiting \in [CallIds -> Nat]
    /\ budget_wake_owed \in [CallIds -> BOOLEAN]

Init ==
    /\ L0!Init
    /\ buffers_held_by_host = [cId \in CallIds |-> 0]
    /\ write_dones_emitted = [cId \in CallIds |-> 0]
    /\ write_done_callback_running = [cId \in CallIds |-> FALSE]
    /\ delivery_callback_running = [cId \in CallIds |-> FALSE]
    /\ payloads_consumed_by_host = [cId \in CallIds |-> 0]
    /\ handle_released = [cId \in CallIds |-> FALSE]
    /\ cancel_requested = [cId \in CallIds |-> FALSE]
    /\ shutdown_event_emitted = [rtId \in RuntimeIds |-> FALSE]
    /\ shutdown_callback_running = [rtId \in RuntimeIds |-> FALSE]
    /\ runtime_destroyed = [rtId \in RuntimeIds |-> FALSE]
    /\ buffer_state =
           [cId \in CallIds |-> [b \in BufferIds |-> "none"]]
    /\ buffer_send =
           [cId \in CallIds |-> [b \in BufferIds |-> 0]]
    /\ second_event_owed = [rtId \in RuntimeIds |-> FALSE]
    /\ resources_released_emitted = [rtId \in RuntimeIds |-> FALSE]
    /\ resources_released_callback_running =
           [rtId \in RuntimeIds |-> FALSE]
    /\ last_lend_status = [cId \in CallIds |-> "NONE"]
    /\ buffer_charge = [q \in CallIds \X BufferIds |-> 0]
    /\ buffer_length = [q \in CallIds \X BufferIds |-> 0]
    /\ memory_used = 0
    /\ engine_held = 0
    /\ read_admitted = [cId \in CallIds |-> FALSE]
    /\ lend_waiting = [cId \in CallIds |-> 0]
    /\ budget_wake_owed = [cId \in CallIds |-> FALSE]

(***************************************************************************)
(* ACTIONS - Runtime lifecycle                                             *)
(***************************************************************************)

\* Level 0 lets a new runtime start as soon as the others are RELEASED, which
\* covers both observable statuses.  Level 1 refuses until they are destroyed:
\* a released handle is still a handle, and the budget its observers report is
\* one runtime-wide counter, so a successor may not exist while any observer
\* of the old accounting does.  Strengthening a guard is what a refinement may
\* do; weakening one is not.
\* Behind a name so that expanding RuntimeCreate yields one atom: inline, the
\* quantifier lands in every obligation that reads the action, and it made a
\* heavy preservation lemma intractable rather than merely slower.
NoOtherRuntimeOutstanding(rtId) ==
    \A other \in RuntimeIds :
        (other # rtId /\ IsReleasedRuntime(other)) =>
            IsRuntimeDestroyed(other)

RuntimeCreate(rtId) ==
    /\ L0!RuntimeCreate(rtId)
    /\ NoOtherRuntimeOutstanding(rtId)
    \* A restriction of the model, which keeps one counter where the code keeps
    \* a ledger per runtime: a successor starts once the engine holds nothing.
    /\ ~HasEngineHeldBytes
    /\ UNCHANGED ffi_vars

\* Shutdown closes the channels (level 0) and latches cancellation on
\* every active call of the runtime: the drain must not depend on the
\* host, per the level-0 directive.
RuntimeBeginShutdown(rtId) ==
    /\ L0!RuntimeBeginShutdown(rtId)
    /\ RequestCancellationOfActiveCalls(L0!ChannelsOf(rtId))
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

EmitShutdownComplete(rtId) ==
    /\ IsStoppingRuntime(rtId)
    /\ ~IsShutdownEventEmitted(rtId)
    /\ IsRuntimeDrained(rtId)
    /\ shutdown_event_emitted' =
           [shutdown_event_emitted EXCEPT ![rtId] = TRUE]
    /\ shutdown_callback_running' =
           [shutdown_callback_running EXCEPT ![rtId] = TRUE]
\* The tag: whether the host still holds memory of this runtime.  Recording it
\* here is what makes the second event owed or not owed, and so whether the host
\* has anything to give back at all.  It is not a permission to unload: only the
\* status reaching QUIESCENT is that, in both cases of the tag.
    /\ second_event_owed' =
           [second_event_owed EXCEPT ![rtId] =
                ~NoHostDebt(rtId)]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested>>
    /\ UNCHANGED <<runtime_destroyed, buffer_state, buffer_send,
                   resources_released_emitted,
                   resources_released_callback_running, last_lend_status,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

ShutdownCallbackReturns(rtId) ==
    /\ IsShutdownCallbackRunning(rtId)
    /\ shutdown_callback_running' =
           [shutdown_callback_running EXCEPT ![rtId] = FALSE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* Release happens after the SHUTDOWN_COMPLETE callback has returned, and it
\* publishes AK_RUNTIME_GRPC_STOPPED: the gRPC machinery - channels,
\* connections, transport - is done.  The teardown thread is not: it may still
\* have RESOURCES_RELEASED to carry, which is why the status and not this step
\* is the gate for unloading.
RuntimeRelease(rtId) ==
    /\ IsShutdownEventEmitted(rtId)
    /\ ~IsShutdownCallbackRunning(rtId)
    /\ L0!RuntimeRelease(rtId)
    /\ UNCHANGED ffi_vars

\* ak_runtime_destroy: the handles go void and the arena may be unloaded.
\* It is the runtime-level image of ReleaseCallHandle - refused until the
\* runtime is finished and nothing lent is outstanding, and carrying no
\* fairness, because it is a downcall and the model never promises the
\* host acts.  It does not move buffer_state: an identity the host has
\* already given back is the runtime's own, and when its bytes go is not
\* observable at the ABI, so the model leaves that to FreeReturnedBuffer
\* whether or not the runtime is destroyed.
RuntimeDestroy(rtId) ==
    /\ IsRuntimeQuiescent(rtId)
    /\ ~IsRuntimeDestroyed(rtId)
    /\ runtime_destroyed' = [runtime_destroyed EXCEPT ![rtId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* The second signal: the host has given everything back and the runtime has
\* released it.  Owed only when SHUTDOWN_COMPLETE said so, because a host told
\* nothing was outstanding is waiting for nothing.  Its order against
\* RuntimeRelease is free - the guard waits for the first event and its
\* callback, not for the state - so its return completes the resources branch
\* without making the status QUIESCENT on its own: the level-0 transition may
\* still be owed.
EmitResourcesReleased(rtId) ==
    /\ IsShutdownEventEmitted(rtId)
    /\ SecondEventOwed(rtId)
    /\ ~IsResourcesReleasedEmitted(rtId)
    /\ ~IsShutdownCallbackRunning(rtId)
    /\ NoHostDebt(rtId)
    /\ RuntimeHoldsNoReturnedBytes(rtId)
    /\ resources_released_emitted' =
           [resources_released_emitted EXCEPT ![rtId] = TRUE]
    /\ resources_released_callback_running' =
           [resources_released_callback_running EXCEPT ![rtId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed, last_lend_status,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

ResourcesReleasedCallbackReturns(rtId) ==
    /\ IsResourcesReleasedCallbackRunning(rtId)
    /\ resources_released_callback_running' =
           [resources_released_callback_running EXCEPT ![rtId] = FALSE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed, last_lend_status,
                   resources_released_emitted, buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

RuntimeFail(rtId) ==
    /\ L0!RuntimeFail(rtId)
    /\ UNCHANGED ffi_vars

RemainFailed(rtId) ==
    /\ L0!RemainFailed(rtId)
    /\ UNCHANGED ffi_vars

RemainReleased ==
    /\ L0!RemainReleased
    /\ UNCHANGED ffi_vars

(***************************************************************************)
(* ACTIONS - Channel lifecycle                                             *)
(***************************************************************************)

ChannelCreate(chId, rtId) ==
    /\ L0!ChannelCreate(chId, rtId)
    /\ UNCHANGED ffi_vars

\* Closing a channel cancels its calls: without this, a channel whose
\* host neither cancels nor consumes would never drain.
ChannelStartClosing(chId) ==
    /\ L0!ChannelStartClosing(chId)
    /\ RequestCancellationOfActiveCalls({chId})
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* At level 1 the close completes only once every call has terminated on
\* its own (through DeliverCancelled): the level-0 cancel-en-masse
\* comprehensions degenerate to the identity, so no callback is skipped.
ChannelFinishClosing(chId) ==
    /\ \A cId \in L0!CallsOf(chId) : ~L0!IsActiveCall(cId)
    /\ L0!ChannelFinishClosing(chId)
    /\ UNCHANGED ffi_vars

(***************************************************************************)
(* ACTIONS - Call lifecycle                                                *)
(***************************************************************************)

\* The per-call FFI variables are clean on an unused call (an invariant,
\* like the level-0 UnusedCallsAreEmpty), so starting needs no reset.
CallStart(cId, chId) ==
    /\ L0!CallStart(cId, chId)
    /\ UNCHANGED ffi_vars

\* ak_call_cancel only latches the request; the cancellation itself is
\* the DeliverCancelled callback.  A handle exists only for a started,
\* not-yet-released call.
RequestCallCancellation(cId) ==
    /\ ~L0!IsUnusedCall(cId)
    /\ ~IsHandleReleased(cId)
    /\ ~IsRuntimeOfCallDestroyed(cId)
    /\ cancel_requested' = [cancel_requested EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* Reclaiming a call is the runtime's own step, not a downcall: the ABI has
\* no ak_call_release.  Every resource a call lends out comes back through
\* an event the runtime already observes, so it knows when a terminal call
\* owes nothing and reclaims the handle and the arena itself.  What follows
\* - ReleasedCallIsClean - is therefore unconditional rather than contingent
\* on the host calling anything.
\* ak_runtime_destroy stays a downcall for the opposite reason: it answers a
\* question only the host can ask, may I unload, so the host needs a verdict
\* it can act on.  Reclaiming a call answers a question the runtime resolves
\* for itself, and the verdict is of no use to the host.
ReleaseCallHandle(cId) ==
    /\ ~L0!IsUnusedCall(cId)
    /\ ~L0!IsActiveCall(cId)
    /\ ~IsHandleReleased(cId)
    /\ ~IsRuntimeOfCallDestroyed(cId)
    /\ HostOwnsNoPayload(cId)
    /\ HostHoldsNoBuffer(cId)
    \* Both are conditions only the runtime can read: the host cannot see
    \* whether one of its own callbacks is still on the stack, nor whether
    \* the bytes of a buffer it gave back are gone.  The arena goes in this
    \* step, so no allocation of it may still be outstanding.
    /\ ~IsDeliveryCallbackRunning(cId)
    /\ \A b \in BufferIds : ~IsReturnedBuffer(cId, b)
    /\ handle_released' = [handle_released EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

(***************************************************************************)
(* ACTIONS - Send path                                                     *)
(***************************************************************************)

\* ak_get_call_buffer: the call arena lends the host a buffer to
\* serialize into.  The slot budget is charged here rather than at the
\* send, because the allocation is what costs memory.  Refused once
\* cancellation is latched and after reclamation, which is what keeps a
\* reclaimed call clean.  Allocating and handing over are one step: the
\* downcall has not returned in between, so nothing can observe a
\* difference, and splitting them would only be needed to model an
\* allocation failure.
LendSendBuffer(cId, b, len, charge) ==
    /\ L0!IsActiveCall(cId)
    /\ ~IsHandleReleased(cId)
    /\ ~IsCancelRequested(cId)
    /\ HasFreeSendSlot(cId)
    /\ IsFreshBuffer(cId, b)
    /\ HostHoldsNoBuffer(cId)
    \* The three budget conditions.  A request outside IsLendable is refused
    \* permanently - that is AK_STATUS_MESSAGE_TOO_LARGE, and the model derives
    \* the permanence rather than asserting it: the charge covers the request,
    \* so a length above the ceiling leaves IsMemoryAvailable false in every
    \* state.  A request that does not fit right now is AK_STATUS_BUDGET_BUSY,
    \* recorded by RefuseLendForBudget for the charge that did not fit.
    /\ 0 < len
    /\ IsLendable(len)
    /\ CoversRequest(charge, len)
    /\ IsMemoryAvailable(charge)
    /\ buffers_held_by_host' =
           [buffers_held_by_host EXCEPT ![cId] = @ + 1]
    /\ buffer_state' = [buffer_state EXCEPT ![cId][b] = "lent"]
    \* The charge is recorded once, on a fresh buffer, and rewritten only by
    \* an exchange, to zero, when it becomes the new buffer's.  The counter
    \* moves by it here and back by it at the free, which is what the
    \* accounting checks.
    /\ buffer_charge' = [buffer_charge EXCEPT ![<<cId, b>>] = charge]
    /\ buffer_length' = [buffer_length EXCEPT ![<<cId, b>>] = len]
    /\ memory_used' = memory_used + charge
    /\ last_lend_status' = [last_lend_status EXCEPT ![cId] = "OK"]
    \* Served: the call waits on nothing more, and reads are no longer held
    \* back for it.
    /\ EndWaitOf(cId)
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed,
                   buffer_send,
                   second_event_owed, resources_released_emitted,
                   resources_released_callback_running,
                   engine_held, read_admitted>>

\* The three refusals of ak_get_call_buffer.  They write nothing but the status
\* the downcall returned, so every property proved of the other actions crosses
\* them unchanged.  What they buy is the observable frontier: the states a
\* level-2 binding refines its retry decisions against, and the place where
\* the ABI's status codes get their meaning.  None of them carries fairness -
\* refusing is never owed.
ContemplatesLend(cId) ==
    /\ L0!IsActiveCall(cId)
    /\ ~IsHandleReleased(cId)
    /\ ~IsCancelRequested(cId)
    \* One lent buffer at a time: asking while holding one is refused at the
    \* ABI as a state error, so the model never contemplates it.  It is what
    \* makes "SLOT_BUSY wakes on the next WRITE_DONE" true - a host eligible
    \* to ask holds nothing, so the window is in-flight sends only.
    /\ HostHoldsNoBuffer(cId)

\* Permanent, and derived rather than asserted: IsLendable reads the request
\* and the ceiling, so no return by anyone changes the answer.
RefuseLendTooLarge(cId, len) ==
    /\ ContemplatesLend(cId)
    /\ ~IsLendable(len)
    /\ last_lend_status' =
           [last_lend_status EXCEPT ![cId] = "MESSAGE_TOO_LARGE"]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

RefuseLendForSlot(cId, len) ==
    /\ ContemplatesLend(cId)
    /\ 0 < len
    /\ IsLendable(len)
    /\ ~HasFreeSendSlot(cId)
    /\ last_lend_status' = [last_lend_status EXCEPT ![cId] = "SLOT_BUSY"]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* The charge is a parameter because the allocator picks it: this refusal is
\* one that did not fit, not the claim that none would.  Guarding it on
\* ~HasAccountingRoomForSomeCharge instead would make the refusal impossible
\* whenever any charge fits, denying the observable frontier exactly the
\* states it exists to record.
RefuseLendForBudget(cId, len, charge) ==
    /\ ContemplatesLend(cId)
    /\ 0 < len
    /\ IsLendable(len)
    /\ HasFreeSendSlot(cId)
    /\ CoversRequest(charge, len)
    /\ ~IsMemoryAvailable(charge)
    /\ last_lend_status' = [last_lend_status EXCEPT ![cId] = "BUDGET_BUSY"]
    \* The send now waits on its length, which holds reads back until it has
    \* room.  The length is never zero: no lend asks for none.
    /\ lend_waiting' = [lend_waiting EXCEPT ![cId] = len]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, budget_wake_owed>>


\* ak_return_call_buffer: the host gives a buffer back unused.  Legal on
\* a cancelled or terminal call - it is the only exit for a buffer whose
\* send is refused, and reclamation waits for it.
HostReturnsBuffer(cId, b) ==
    /\ IsLentBuffer(cId, b)
    \* Redundant with the line above once LentCountMatchesBufferStates is
    \* known, and kept anyway: it is what lets the type of the counter be
    \* proved without reaching for an invariant.
    /\ HostHoldsSomeBuffer(cId)
    /\ buffers_held_by_host' =
           [buffers_held_by_host EXCEPT ![cId] = @ - 1]
    /\ buffer_state' = [buffer_state EXCEPT ![cId][b] = "returned"]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed,
                   buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* The runtime releases the bytes of a buffer the host has given back.
\* Not an ABI event at all: when the allocation actually goes is Rust's
\* business.  This is the step that makes "returned" and
\* "freed" two states rather than one, and it is the runtime's own, so it
\* carries fairness where the return does not.
FreeReturnedBuffer(cId, b) ==
    /\ IsReturnedBuffer(cId, b)
    \* The bytes of a send are not released while the send is unacquitted:
    \* the transport may still be reading them.  A buffer given back unused
    \* carries no send, so it
    \* is free to go at once.
    /\ CarriesNoUnacquittedSend(cId, b)
    /\ buffer_state' = [buffer_state EXCEPT ![cId][b] = "freed"]
    \* Exactly the charge the lend added comes back off the counter.  The
    \* charge itself stays recorded and is never summed again, the sums running
    \* over the buffers still out.
    /\ memory_used' = memory_used - buffer_charge[<<cId, b>>]
    /\ IF buffer_charge[<<cId, b>>] > 0
       THEN OweBudgetWakeToWaitingCalls
       ELSE UNCHANGED budget_wake_owed
    /\ UNCHANGED <<buffer_charge, buffer_length>>
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host,
                   write_dones_emitted, write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed,
                   buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   engine_held, read_admitted, lend_waiting>>

\* ak_resize_call_buffer: the host exchanges the buffer it holds for one of
\* another length, keeping what it wrote.  A lend of the new buffer and the
\* return of the old one in one step: the old charge becomes the new one on
\* the counter, which moves by the difference alone, so the two are never
\* both on it and the room the old one held is never offered to another call
\* in between.  The old buffer is returned with nothing left to free: its
\* charge is the new buffer's, so its release takes nothing off the counter,
\* and it carries no send, so that release is owed at once.  The call's one
\* lent buffer, its slot and its count are the old buffer's and do not move,
\* which is why the count and the window are unchanged.  What the host wrote
\* is no state of the model, so the bytes kept are not an argument.
\* A refusal takes no step, and a refused resize records no wait: the host
\* holds a buffer, and the wait is the lend's.
ResizeSendBuffer(cId, b, nb, len, charge) ==
    /\ L0!IsActiveCall(cId)
    /\ ~IsHandleReleased(cId)
    /\ ~IsCancelRequested(cId)
    /\ IsLentBuffer(cId, b)
    /\ IsFreshBuffer(cId, nb)
    /\ 0 < len
    /\ IsLendable(len)
    /\ CoversRequest(charge, len)
    /\ IsMemoryAvailableForExchange(cId, b, charge)
    \* Written pointwise: two entries of one row move, and a lookup of the
    \* result then needs no reading of an update applied twice.
    /\ buffer_state' =
           [c \in CallIds |-> [x \in BufferIds |->
               IF c = cId /\ x = b THEN "returned"
               ELSE IF c = cId /\ x = nb THEN "lent"
               ELSE buffer_state[c][x]]]
    /\ buffer_charge' =
           [q \in CallIds \X BufferIds |->
               IF q = <<cId, b>> THEN 0
               ELSE IF q = <<cId, nb>> THEN charge
               ELSE buffer_charge[q]]
    /\ buffer_length' = [buffer_length EXCEPT ![<<cId, nb>>] = len]
    /\ memory_used' = memory_used - buffer_charge[<<cId, b>>] + charge
    \* A release of bytes owes the calls whose send waits their wake-up; a
    \* growth releases none.
    /\ IF buffer_charge[<<cId, b>>] > charge
       THEN OweBudgetWakeToWaitingCalls
       ELSE UNCHANGED budget_wake_owed
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host,
                   write_dones_emitted, write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed,
                   buffer_send,
                   second_event_owed, last_lend_status,
                   resources_released_emitted,
                   resources_released_callback_running,
                   engine_held, read_admitted, lend_waiting>>

\* ak_call_send_message commits a buffer the host already holds, so the
\* bound was checked when it was lent and the count of outstanding
\* buffers does not move: one leaves the host's hands and becomes the
\* send that keeps it.  Refused once cancellation is latched, and the
\* level-0 guard already refuses it once the trailers are in
\* (~status_pending), so no new send can ever delay a pending terminal.
\* The send takes its identity from the level-0 machinery (its index in
\* submitted).
\* The buffer is an argument, as it is at the ABI: ak_call_send_message
\* names the allocation it commits.  Choosing it inside the action instead
\* would let the model decide where the host decides, and would make the
\* send-to-buffer link a record of a nondeterministic choice rather than of
\* what the caller passed.
\* The message must fit the buffer it was handed.  The lend saw a length
\* only; here the message is known, so this is where the check belongs.
SendMessage(cId, msg, b) ==
    /\ HostHoldsSomeBuffer(cId)
    /\ ~IsCancelRequested(cId)
    /\ IsLentBuffer(cId, b)
    \* A message token is submitted at most once globally: the submitted
    \* sequences are jointly injective, so the token identifies one send.
    /\ NeverSubmitted(msg)
    /\ NeverReceived(msg)
    /\ FitsInBuffer(msg, cId, b)
    /\ L0!SendMessage(cId, msg)
    /\ buffers_held_by_host' =
           [buffers_held_by_host EXCEPT ![cId] = @ - 1]
    /\ buffer_state' = [buffer_state EXCEPT ![cId][b] = "returned"]
    \* This allocation now holds the send being accepted, whose index is the
    \* length submitted reaches.
    /\ buffer_send' =
           [buffer_send EXCEPT ![cId][b] = Len(submitted[cId]) + 1]
    /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

EndSend(cId) ==
    /\ ~IsHandleReleased(cId)
    /\ L0!EndSend(cId)
    /\ UNCHANGED ffi_vars

\* WRITE_DONE acquits the oldest unacquitted send, in send order, and the
\* host may unpin that buffer - the same allocation the lend handed out
\* and the send pinned.  One acquittal
\* callback at a time; WRITE_DONE always arrives, exactly once per
\* accepted send, and always before the terminal: the send side is
\* driven by binding threads alone.  On a call that declared one request,
\* the commit takes this step and its return with no callback.
EmitWriteDone(cId) ==
    /\ IsAwaitingWriteDone(cId)
    /\ ~IsWriteDoneCallbackRunning(cId)
    /\ write_dones_emitted' = [write_dones_emitted EXCEPT ![cId] = @ + 1]
    /\ write_done_callback_running' =
           [write_done_callback_running EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, delivery_callback_running,
                   payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

WriteDoneReturns(cId) ==
    /\ IsWriteDoneCallbackRunning(cId)
    /\ write_done_callback_running' =
           [write_done_callback_running EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

(***************************************************************************)
(* ACTIONS - Network (identical to level 0)                                *)
(***************************************************************************)

NetworkSend(cId) ==
    /\ L0!NetworkSend(cId)
    /\ UNCHANGED ffi_vars

\* The first step of a read, where the decision is taken: the call may read its
\* next message.  No fairness - what arrives is the peer's to drive, as the
\* level-0 receive is.
AdmitRead(cId) ==
    /\ L0!IsActiveCall(cId)
    /\ ~IsStatusPending(cId)
    /\ ~IsReadAdmitted(cId)
    /\ HasDeliveredAllReceived(cId)
    /\ IsReadAdmissible
    /\ read_admitted' = [read_admitted EXCEPT ![cId] = TRUE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   lend_waiting, budget_wake_owed>>

\* The second step: the message arrives decoded and is charged its length.
\* Calls admitted together may take the count past the first threshold by a
\* message each; a message past the second is EndCallPastHardCeiling's.
NetworkReceive(cId, msg) ==
    /\ IsReadAdmitted(cId)
    /\ IsWithinHardCeiling(MessageLength[msg])
    /\ L0!NetworkReceive(cId, msg)
    \* The occurrence discipline: a fresh token, never seen in either
    \* direction.  A strengthening of the level-0 action, as a refinement may.
    /\ NeverReceived(msg)
    /\ NeverSubmitted(msg)
    /\ read_admitted' = [read_admitted EXCEPT ![cId] = FALSE]
    /\ memory_used' = memory_used + MessageLength[msg]
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, engine_held,
                   lend_waiting, budget_wake_owed>>

\* A message that would take the count past the second threshold ends its
\* call with RESOURCE_EXHAUSTED and is freed at once, never charged.  It
\* refines the arrival of a status, whose code level 0 does not carry.
EndCallPastHardCeiling(cId, msg) ==
    /\ IsReadAdmitted(cId)
    /\ ~IsWithinHardCeiling(MessageLength[msg])
    /\ L0!ReceiveStatus(cId)
    /\ read_admitted' = [read_admitted EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   lend_waiting, budget_wake_owed>>

ReceiveStatus(cId) ==
    /\ L0!ReceiveStatus(cId)
    /\ UNCHANGED ffi_vars

(***************************************************************************)
(* ACTIONS - Delivery                                                      *)
(***************************************************************************)

DeliverInitialMetadata(cId) ==
    /\ HasFreeDeliverySlot(cId)
    /\ L0!DeliverInitialMetadata(cId)
    /\ HandPayloadToHost(cId)
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

DeliverMessage(cId) ==
    /\ HasFreeDeliverySlot(cId)
    /\ ~IsCancelRequested(cId)
    /\ L0!DeliverMessage(cId)
    /\ HandPayloadToHost(cId)
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* Terminals wait for the send side to drain: WRITE_DONE precedes the
\* terminal, so the terminal is the last callback of the call and the
\* call ctx lifetime ends there.  HasFreeDeliverySlotForTerminal lets
\* them out with every credit spent.
DeliverStatus(cId) ==
    /\ HasFreeDeliverySlotForTerminal(cId)
    /\ ~IsCancelRequested(cId)
    /\ HasNoSendInFlight(cId)
    /\ L0!DeliverStatus(cId)
    /\ HandPayloadToHost(cId)
    \* Every message was delivered, so nothing is left to give back here.
    /\ EndWaitOf(cId)
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted>>

\* Cancellation completes as a delivered CANCELLED terminal.  Refines
\* L0!CallCancel: the callback is the cancellation.
DeliverCancelled(cId) ==
    /\ HasFreeDeliverySlotForTerminal(cId)
    /\ IsCancelRequested(cId)
    /\ HasNoSendInFlight(cId)
    \* One event per step.  The branch of L0!CallCancel that would deliver
    \* metadata and the terminal in one step has no level-2 refinement: metadata
    \* goes out first, in a step of its own, and only then may the cancellation
    \* settle the call.  The two steps may share a callback.
    /\ ~HasNoDeliveredEvents(cId)
    /\ L0!CallCancel(cId)
    /\ HandPayloadToHost(cId)
    \* What the call received and never delivered goes with it, and a release
    \* that gives bytes back owes the other waiting sends their wake-up.
    /\ memory_used' = memory_used - UndeliveredBytes(cId)
    /\ lend_waiting' = [lend_waiting EXCEPT ![cId] = 0]
    /\ budget_wake_owed' =
           [c \in CallIds |->
               IF c = cId THEN FALSE
               ELSE IsBudgetWakeOwed(c)
                    \/ (UndeliveredBytes(cId) > 0 /\ IsLendWaiting(c))]
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length,
                   engine_held, read_admitted>>

DeliveryCallbackReturns(cId) ==
    /\ IsDeliveryCallbackRunning(cId)
    /\ delivery_callback_running' =
           [delivery_callback_running EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* ak_event_consumed: frees the oldest payload the host still holds and
\* arms the next delivery in one gesture.  Takes the payload, not the
\* call handle: legal after the terminal and after release - and destroy
\* requires every payload consumed, so nothing survives the runtime.
\* Release follows delivery order, which is what lets one counter stand
\* for the whole outstanding set.
HostConsumesEvent(cId) ==
    /\ HostOwnsSomePayload(cId)
    /\ payloads_consumed_by_host' =
           [payloads_consumed_by_host EXCEPT ![cId] = @ + 1]
    \* A message payload gives its bytes back, and a release that gives some
    \* back owes every waiting send its wake-up.
    /\ memory_used' = memory_used - NextConsumedLength(cId)
    /\ IF NextConsumedLength(cId) > 0
       THEN OweBudgetWakeToWaitingCalls
       ELSE UNCHANGED budget_wake_owed
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send,
                   second_event_owed, last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length,
                   engine_held, read_admitted, lend_waiting>>

\* A release woke a waiting send: its callback tells the host to try again,
\* which the host is obliged to do or to cancel the call.  One step, the
\* callback's return included: the engine raises it from the task that drives
\* the call, the one that also delivers the terminal, so the two never overlap
\* and the terminal stays the call's last callback.  Not on a call being
\* cancelled or whose trailers are in, where it would wake a send that call can
\* no longer make.
EmitBudgetWake(cId) ==
    /\ IsBudgetWakeOwed(cId)
    /\ L0!IsActiveCall(cId)
    /\ ~IsCancelRequested(cId)
    /\ ~IsStatusPending(cId)
    /\ budget_wake_owed' = [budget_wake_owed EXCEPT ![cId] = FALSE]
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running, delivery_callback_running,
                   payloads_consumed_by_host, handle_released,
                   cancel_requested, shutdown_event_emitted,
                   shutdown_callback_running, runtime_destroyed,
                   buffer_state, buffer_send, second_event_owed,
                   last_lend_status, resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length, memory_used, engine_held,
                   read_admitted, lend_waiting>>

(***************************************************************************)
(* ACTIONS - The engine's own bytes                                        *)
(***************************************************************************)

\* The engine makes the compressed copy of a message its call sent and charges
\* it as a lend is charged, against the first threshold: CopyBudget.hold_copy.
\* A copy is made for a message of an active call, which is why the call has
\* sent one.  The model names neither the copy nor its message, only the bytes.
\* A copy that does not fit is not made, and the engine sends the message as
\* the host wrote it, so a refusal takes no step: taking never blocks and is
\* never owed.  The host owes nothing for the bytes, and the count a shutdown
\* waits on does not move.
EngineTakesBytes(cId, n) ==
    /\ L0!IsActiveCall(cId)
    /\ HasAcceptedSendAt(cId, 1)
    /\ 0 < n
    /\ IsMemoryAvailable(n)
    /\ IsEngineRoomAvailable(n)
    /\ engine_held' = engine_held + n
    /\ memory_used' = memory_used + n
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status,
                   resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length,
                   read_admitted, lend_waiting, budget_wake_owed>>

\* The engine drops messages that held copies and gives their bytes back:
\* CopyBudget.release_copy.  Some of what it holds, not all: the messages go
\* one by one.  Like every release of bytes it owes the calls whose send waits
\* their wake-up.
EngineGivesBackBytes(n) ==
    /\ 0 < n
    /\ IsEngineHoldingAtLeast(n)
    /\ engine_held' = engine_held - n
    /\ memory_used' = memory_used - n
    /\ OweBudgetWakeToWaitingCalls
    /\ UNCHANGED l0_vars
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested,
                   shutdown_event_emitted, shutdown_callback_running,
                   runtime_destroyed, buffer_state, buffer_send,
                   second_event_owed, last_lend_status,
                   resources_released_emitted,
                   resources_released_callback_running,
                   buffer_charge, buffer_length,
                   read_admitted, lend_waiting>>

\* The instance the fairness names: everything the engine holds, given back.
EngineGivesBackAllBytes == EngineGivesBackBytes(BytesHeldByEngine)

(***************************************************************************)
(* NEXT STATE RELATION                                                     *)
(***************************************************************************)

Next ==
    \/ \E rtId \in RuntimeIds : RuntimeCreate(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeBeginShutdown(rtId)
    \/ \E rtId \in RuntimeIds : EmitShutdownComplete(rtId)
    \/ \E rtId \in RuntimeIds : ShutdownCallbackReturns(rtId)
    \/ \E rtId \in RuntimeIds : EmitResourcesReleased(rtId)
    \/ \E rtId \in RuntimeIds : ResourcesReleasedCallbackReturns(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeRelease(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeDestroy(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeFail(rtId)
    \/ \E rtId \in RuntimeIds : RemainFailed(rtId)
    \/ RemainReleased
    \/ \E chId \in ChannelIds, rtId \in RuntimeIds : ChannelCreate(chId, rtId)
    \/ \E chId \in ChannelIds : ChannelStartClosing(chId)
    \/ \E chId \in ChannelIds : ChannelFinishClosing(chId)
    \/ \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
    \/ \E cId \in CallIds : RequestCallCancellation(cId)
    \/ \E cId \in CallIds : ReleaseCallHandle(cId)
    \/ \E cId \in CallIds, b \in BufferIds, len \in Sizes, charge \in Sizes :
           LendSendBuffer(cId, b, len, charge)
    \/ \E cId \in CallIds, len \in RequestLengths :
           RefuseLendTooLarge(cId, len)
    \/ \E cId \in CallIds, len \in Sizes : RefuseLendForSlot(cId, len)
    \/ \E cId \in CallIds, len \in Sizes, charge \in CandidateCharges :
           RefuseLendForBudget(cId, len, charge)
    \/ \E cId \in CallIds, b \in BufferIds :
           HostReturnsBuffer(cId, b)
    \/ \E cId \in CallIds, b \in BufferIds :
           FreeReturnedBuffer(cId, b)
    \/ \E cId \in CallIds, b \in BufferIds, nb \in BufferIds,
          len \in Sizes, charge \in Sizes :
           ResizeSendBuffer(cId, b, nb, len, charge)
    \/ \E cId \in CallIds, msg \in Messages, b \in BufferIds :
           SendMessage(cId, msg, b)
    \/ \E cId \in CallIds : EndSend(cId)
    \/ \E cId \in CallIds : EmitWriteDone(cId)
    \/ \E cId \in CallIds : WriteDoneReturns(cId)
    \/ \E cId \in CallIds : NetworkSend(cId)
    \/ \E cId \in CallIds : AdmitRead(cId)
    \/ \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    \/ \E cId \in CallIds, msg \in Messages : EndCallPastHardCeiling(cId, msg)
    \/ \E cId \in CallIds : ReceiveStatus(cId)
    \/ \E cId \in CallIds : DeliverInitialMetadata(cId)
    \/ \E cId \in CallIds : DeliverMessage(cId)
    \/ \E cId \in CallIds : DeliverStatus(cId)
    \/ \E cId \in CallIds : DeliverCancelled(cId)
    \/ \E cId \in CallIds : DeliveryCallbackReturns(cId)
    \/ \E cId \in CallIds : HostConsumesEvent(cId)
    \/ \E cId \in CallIds : EmitBudgetWake(cId)
    \/ \E cId \in CallIds, n \in Sizes : EngineTakesBytes(cId, n)
    \/ \E n \in Sizes : EngineGivesBackBytes(n)

(***************************************************************************)
(* NEW LIVENESS PROPERTIES                                                 *)
(* Stated over named formulas, level-0 style.  The ones whose premise is a *)
(* level-0 invariant embed the ~NotFailed escape, because level-0 safety   *)
(* is asserted only outside the failed state; the ones that rest on the    *)
(* FFI invariants alone are unconditional, and say so.  The send and       *)
(* payload guarantees are per object, following the level-0 per-index      *)
(* style: the aggregate forms are corollaries, never the statement.        *)
(***************************************************************************)

\* Cancellation completes without the host: the strongest new guarantee.
CancellationCompletes ==
    \A cId \in CallIds :
        (L0!IsActiveCall(cId) /\ IsCancelRequested(cId) /\ L0!NotFailed) ~>
            (~L0!IsActiveCall(cId) \/ ~L0!NotFailed)

\* Every pinned buffer is individually given back: the k-th accepted send
\* is acquitted by its in-order WRITE_DONE, whose callback returns.
SendAcquittedAt(cId, k) ==
    (HasAcceptedSendAt(cId, k) /\ L0!NotFailed) ~>
        (IsSendAcquittedAt(cId, k) \/ ~L0!NotFailed)

SendsEventuallyAcquitted ==
    \A cId \in CallIds :
        \A k \in L0!PositiveNaturals : SendAcquittedAt(cId, k)

\* Every payload handed to the host is individually consumed.  This is
\* the guarantee that rests on the per-payload host hypothesis.
PayloadConsumedAt(cId, k) ==
    (HostOwnsPayload(cId, k) /\ L0!NotFailed) ~>
        (~HostOwnsPayload(cId, k) \/ ~L0!NotFailed)

PayloadsEventuallyConsumed ==
    \A cId \in CallIds :
        \A k \in PayloadIndices : PayloadConsumedAt(cId, k)

\* The SHUTDOWN_COMPLETE contract, a named corollary of the inherited
\* EventualShutdown and the strengthened release guard.
ShutdownEventEmitted ==
    \A rtId \in RuntimeIds :
        (IsStoppingRuntime(rtId)) ~>
            (IsShutdownEventEmitted(rtId) \/ ~L0!NotFailed)

\* Every callback the runtime hands out comes back.  These are the three
\* obligations the ABI imposes on the host, stated as guarantees so the
\* fairness conjuncts they rest on are not hypotheses with nothing to buy.
DeliveryCallbacksReturn ==
    \A cId \in CallIds :
        IsDeliveryCallbackRunning(cId) ~> ~IsDeliveryCallbackRunning(cId)

WriteDoneCallbacksReturn ==
    \A cId \in CallIds :
        IsWriteDoneCallbackRunning(cId) ~> ~IsWriteDoneCallbackRunning(cId)

ShutdownCallbacksReturn ==
    \A rtId \in RuntimeIds :
        IsShutdownCallbackRunning(rtId) ~> ~IsShutdownCallbackRunning(rtId)

ResourcesReleasedCallbacksReturn ==
    \A rtId \in RuntimeIds :
        IsResourcesReleasedCallbackRunning(rtId) ~>
            ~IsResourcesReleasedCallbackRunning(rtId)

\* Every buffer the arena lends out is given back and then released.  The
\* first half is the host's obligation, per buffer because returns are
\* unordered; the second is the runtime's, and the gap between them is the
\* transport's.
BufferEventuallyFreed ==
    \A cId \in CallIds, b \in BufferIds :
        IsLentBuffer(cId, b) ~> IsFreedBuffer(cId, b)

\* A terminal call is reclaimed without the host doing anything beyond
\* giving back what it holds.  This is what makes the arena's return a
\* property of the model rather than a decision of the implementation, and
\* it is unconditional where a downcall-driven release could simply never
\* be called.
\* The escape is destruction, and it is not a weakening: ak_runtime_destroy
\* takes the arena with the runtime, so there is nothing left to reclaim.
\* It is the only one - a runtime failure does not stop the drain, because
\* every action it rests on is untouched by failure.
CallEventuallyReclaimed ==
    \A cId \in CallIds :
        L0!IsTerminalCall(cId) ~>
            (IsHandleReleased(cId) \/ IsRuntimeOfCallDestroyed(cId))

\* And the runtime-level image: a runtime that has stopped running becomes
\* quiescent, which is to say destructible, unloadable and replaceable.  Note
\* the shape - the runtime promises the permission, never the destruction,
\* because ak_runtime_destroy is the host's call to make.  This is what a host
\* polling ak_runtime_status is entitled to expect, and it is stronger than the
\* host's ledger emptying: it also carries the last callback having returned,
\* which is the only thing that can say the trampoline thread is gone.
\* The failure escape is the same one ShutdownEventEmitted carries, and for
\* the same reason: what makes a released runtime's calls terminal is a
\* level-0 invariant, and level-0 safety is asserted only while no runtime
\* sits in the deliberately unconstrained failed state.
RuntimeEventuallyQuiescent ==
    \A rtId \in RuntimeIds :
        IsReleasedRuntime(rtId) ~>
            (IsRuntimeQuiescent(rtId) \/ ~L0!NotFailed)
\* The second event arrives when it was owed.  This is the promise the release
\* tag makes, and the reason a host told AK_HOST_MUST_RETURN may wait for the
\* callback rather than poll: the wait terminates.  Stated on the tag rather
\* than on the runtime state because the tag is what the host reads.
ResourcesReleasedEventually ==
    \A rtId \in RuntimeIds :
        (IsShutdownEventEmitted(rtId) /\ SecondEventOwed(rtId)) ~>
            (IsResourcesReleasedEmitted(rtId) \/ ~L0!NotFailed)


\* A send refused for room eventually has room while it waits.  Reads are held
\* back for it, so once the reads already admitted have landed nothing new is
\* received; every buffer out is eventually freed and every message received
\* eventually given back, so the counter falls, and the first admission past
\* that point finds room for the waiting length.  The engine's copies hold
\* part of the counter for as long as the engine takes them: room is seen at a
\* state where it holds nothing, which its fairness brings about, and it may
\* take bytes again at once.  Without the hold, received
\* bytes would keep the counter up under steady traffic and nothing would
\* fall.  Stated on a predicate and its negation rather than on two
\* inequalities: the temporal backend matches formulas, and two arithmetic
\* comparisons are two unrelated atoms to it.
\* It says nothing about who is served: lending carries no fairness, and a
\* retry loop's own guarantees are level 2's.
\* Stated on the request, because the request is what the host makes.  The
\* antecedent is that no charge admits it, the consequent that some charge
\* does: a refusal at one charge followed by a success at another, with nothing
\* freed, is a step this says nothing about - and must not, since the lend
\* carries no fairness and the allocator's choice is not the budget's promise.
\* The existential collapses at its best witness, charge = len, so this is
\* IsMemoryAvailable(len) said without reaching into the counter.
\* The guard sits outside the leads-to, not inside its antecedent.  It reads
\* only len and Ceiling, both rigid, so the two forms are equivalent - but the
\* temporal backend treats every atom as flexible, and inside the antecedent it
\* cannot know the guard still holds when the room arrives.
\* The wait ends without room when the call does, or when the host asks again
\* for another length; either way this send no longer holds reads back.  An
\* empty request is never lent, so it never waits.  The failure
\* escape is the one every promise resting on level-0 delivery carries: what
\* drains the received side is that delivery.
RefusedSendEventuallyHasRoom ==
    \A cId \in CallIds, len \in Nat :
        (0 < len /\ IsLendable(len)) =>
            ((/\ IsLendWaitingFor(cId, len)
              /\ ~HasAccountingRoomForSomeCharge(len)
              /\ L0!NotFailed)
                ~> (\/ HasAccountingRoomForSomeCharge(len)
                    \/ ~IsLendWaitingFor(cId, len)
                    \/ ~L0!NotFailed))

\* The engine holds nothing again: a copy goes with the message that holds it,
\* so what the engine keeps for itself is given back.  It says nothing about
\* what the engine takes meanwhile.  This is what the fairness on
\* EngineGivesBackAllBytes buys, and what RefusedSendEventuallyHasRoom reads.
EngineBytesEventuallyGivenBack ==
    HasEngineHeldBytes ~> ~HasEngineHeldBytes

\* What ak_channel_release promises: a channel told to close closes, its
\* calls cancelled and drained on the runtime's fairness plus the host
\* obligations - a callback that never returns holds the drain open.
EventualChannelClosed ==
    \A chId \in ChannelIds :
        IsClosingChannel(chId) ~> IsClosedChannel(chId)

LivenessProperties ==
    /\ CancellationCompletes
    /\ SendsEventuallyAcquitted
    /\ PayloadsEventuallyConsumed
    /\ ShutdownEventEmitted
    /\ DeliveryCallbacksReturn
    /\ WriteDoneCallbacksReturn
    /\ ShutdownCallbacksReturn
    /\ ResourcesReleasedCallbacksReturn
    /\ BufferEventuallyFreed
    /\ CallEventuallyReclaimed
    /\ RuntimeEventuallyQuiescent
    /\ ResourcesReleasedEventually
    /\ RefusedSendEventuallyHasRoom
    /\ EngineBytesEventuallyGivenBack
    /\ EventualChannelClosed

(***************************************************************************)
(* FAIRNESS AND SPEC                                                       *)
(*                                                                         *)
(* Twenty-one action families under WF, all individual, and which side     *)
(* owes each one, or what it rests on, is what the four groups below       *)
(* record.                                                                 *)
(*                                                                         *)
(* The Rust runtime owes ten: NetworkSend, ReceiveStatus, EmitWriteDone,   *)
(* EmitBudgetWake, RuntimeRelease, EmitShutdownComplete,                   *)
(* EmitResourcesReleased, ChannelFinishClosing, FreeReturnedBuffer and     *)
(* ReleaseCallHandle.  These are its own threads and its own allocator,    *)
(* and its own code inside a downcall - ReleaseCallHandle is taken where   *)
(* the last debt clears, on the host's thread too - so nothing outside the *)
(* library can stall them.                                                 *)
(*                                                                         *)
(* One more, EngineGivesBackAllBytes, is the runtime's step and an         *)
(* assumption on the application: that its compressed sending stops.  A    *)
(* copy goes with the message that holds it, written or not, so the code's *)
(* count reaches zero once the messages that make copies stop; under       *)
(* compressed sending that never stops the engine may hold some copy at    *)
(* every instant, and the conjunct does not hold.  The model has no copy   *)
(* to name, so the fairness is on the whole of what is held.  Giving back  *)
(* some would not do: the engine takes without bound here, and one that    *)
(* gave a byte back and took a byte would keep the count up for ever.      *)
(* Taking and giving back some carry none: neither is owed, and the        *)
(* counter falling is all that a waiting send needs.                       *)
(*                                                                         *)
(* The FFI layer owes four, the upcall dispatches: DeliverInitialMetadata, *)
(* DeliverMessage, DeliverStatus and DeliverCancelled.  An event that      *)
(* reaches the queue reaches the host.                                     *)
(*                                                                         *)
(* The host - binding and application, indistinguishable at this level -   *)
(* owes six: DeliveryCallbackReturns, WriteDoneReturns,                    *)
(* ShutdownCallbackReturns, ResourcesReleasedCallbackReturns,              *)
(* HostConsumesEvent and HostReturnsBuffer.  The first four say a callback *)
(* returns, which is what a callback contract means; the last two say the  *)
(* host gives back what it borrows.  These are the six hypotheses a        *)
(* level-2 binding has to discharge.  One                                  *)
(* HostConsumesEvent per call is enough because release is FIFO, whereas   *)
(* buffer returns are unordered and so need one per buffer.                *)
(*                                                                         *)
(* The remaining downcalls (CallStart, LendSendBuffer, ResizeSendBuffer,  *)
(* SendMessage, EndSend, RequestCallCancellation, RuntimeBeginShutdown)    *)
(* carry no fairness: the model never promises the host acts, only what    *)
(* follows when it does.                                                   *)
(***************************************************************************)

Fairness ==
    /\ \A cId \in CallIds : WF_vars(NetworkSend(cId))
    /\ \A cId \in CallIds : WF_vars(ReceiveStatus(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverInitialMetadata(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverMessage(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverStatus(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
    /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
    /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
    /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
    /\ \A cId \in CallIds :
           WF_vars(HostConsumesEvent(cId))
    /\ \A cId \in CallIds : WF_vars(EmitBudgetWake(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(RuntimeRelease(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(EmitShutdownComplete(rtId))
    /\ \A rtId \in RuntimeIds : WF_vars(ShutdownCallbackReturns(rtId))
    \* The runtime announces, the host lets the announcement return.
    /\ \A rtId \in RuntimeIds : WF_vars(EmitResourcesReleased(rtId))
    /\ \A rtId \in RuntimeIds :
           WF_vars(ResourcesReleasedCallbackReturns(rtId))
    /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
    \* Per buffer, not per call: returns are unordered, so a per-call
    \* conjunct would let a host cycle buffers while starving one.  The
    \* payload side gets away with one per call because release is FIFO.
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_vars(HostReturnsBuffer(cId, b))
    /\ \A cId \in CallIds, b \in BufferIds :
           WF_vars(FreeReturnedBuffer(cId, b))
    \* Reclaiming a call is the runtime's own step, not a downcall, so the
    \* runtime is the side that owes it.
    /\ \A cId \in CallIds : WF_vars(ReleaseCallHandle(cId))
    \* The runtime's step, assumed of the application: its compressed sending
    \* stops, so the copies, which go with their messages, all go.
    /\ WF_vars(EngineGivesBackAllBytes)

Spec == Init /\ [][Next]_vars /\ Fairness

=============================================================================
