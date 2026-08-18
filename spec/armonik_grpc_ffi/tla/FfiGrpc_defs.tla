--------------------------- MODULE FfiGrpc_defs -----------------------------
(***************************************************************************)
(* Proof-support definitions for FfiGrpc: the footprint decomposition of   *)
(* Next, and the inductive strengthening.  Pure definitions only, so both  *)
(* TLAPS modules and TLC model-check configurations can extend this        *)
(* module.  Local names mirror the level-0 module: StrongInv here is the   *)
(* level-1 strengthening, L0!StrongInv the abstract one it contains.       *)
(***************************************************************************)

EXTENDS FfiGrpc

(***************************************************************************)
(* NEXT-STATE DECOMPOSITION BY WRITE FOOTPRINT                             *)
(* Two families: refining actions conjoin a level-0 action and inherit     *)
(* its footprint; FFI-only actions stutter on the level-0 variables and    *)
(* write one FFI family.                                                   *)
(***************************************************************************)

NextSafeRuntimeOnly ==
    \/ \E rtId \in RuntimeIds : RuntimeCreate(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeRelease(rtId)

NextSafeRuntimeChannel ==
    \E rtId \in RuntimeIds : RuntimeBeginShutdown(rtId)

NextSafeChannelOnly ==
    \/ \E chId \in ChannelIds, rtId \in RuntimeIds : ChannelCreate(chId, rtId)
    \/ \E chId \in ChannelIds : ChannelStartClosing(chId)

NextSafeChannelCall ==
    \E chId \in ChannelIds : ChannelFinishClosing(chId)

NextSafeCallOnly ==
    \/ \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
    \/ \E cId \in CallIds, msg \in Messages, b \in BufferIds :
           SendMessage(cId, msg, b)
    \/ \E cId \in CallIds : EndSend(cId)
    \/ \E cId \in CallIds : NetworkSend(cId)
    \/ \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    \/ \E cId \in CallIds : ReceiveStatus(cId)
    \/ \E cId \in CallIds : DeliverInitialMetadata(cId)
    \/ \E cId \in CallIds : DeliverMessage(cId)
    \/ \E cId \in CallIds : DeliverStatus(cId)
    \/ \E cId \in CallIds : DeliverCancelled(cId)

\* The refining actions: each conjoins a level-0 action, so each level-1
\* safe step of this family projects to a level-0 safe step.
NextSafeRefining ==
    \/ NextSafeRuntimeOnly
    \/ NextSafeRuntimeChannel
    \/ NextSafeChannelOnly
    \/ NextSafeChannelCall
    \/ NextSafeCallOnly

\* The FFI-only actions: UNCHANGED L0!vars, so each projects to a level-0
\* stutter.
NextSafeShutdownFfi ==
    \/ \E rtId \in RuntimeIds : EmitShutdownComplete(rtId)
    \/ \E rtId \in RuntimeIds : ShutdownCallbackReturns(rtId)
    \/ \E rtId \in RuntimeIds : EmitResourcesReleased(rtId)
    \/ \E rtId \in RuntimeIds : ResourcesReleasedCallbackReturns(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeDestroy(rtId)

NextSafeCallFfi ==
    \/ \E cId \in CallIds : RequestCallCancellation(cId)
    \/ \E cId \in CallIds : ReleaseCallHandle(cId)
    \/ \E cId \in CallIds, b \in BufferIds :
           LendSendBuffer(cId, b)
    \/ \E cId \in CallIds, b \in BufferIds :
           HostReturnsBuffer(cId, b)
    \/ \E cId \in CallIds, b \in BufferIds :
           FreeReturnedBuffer(cId, b)
    \/ \E cId \in CallIds : EmitWriteDone(cId)
    \/ \E cId \in CallIds : WriteDoneReturns(cId)
    \/ \E cId \in CallIds : DeliveryCallbackReturns(cId)
    \/ \E cId \in CallIds : HostConsumesEvent(cId)

NextSafeFfiOnly ==
    \/ NextSafeShutdownFfi
    \/ NextSafeCallFfi

NextSafe ==
    \/ NextSafeRefining
    \/ NextSafeFfiOnly

NextFail ==
    \E rtId \in RuntimeIds : RuntimeFail(rtId)

NextExplicitStutter ==
    \/ \E rtId \in RuntimeIds : RemainFailed(rtId)
    \/ RemainReleased

NextByFootprint ==
    \/ NextSafe
    \/ NextFail
    \/ NextExplicitStutter

(***************************************************************************)
(* MINIMAL INDUCTIVE STRENGTHENING - FFI CONJUNCTS                         *)
(***************************************************************************)

\* No handle, no FFI state: everything is clean until CallStart, and the
\* downcalls are guarded on a started call.
UnusedCallsAreFfiClean ==
    \A cId \in CallIds :
        L0!IsUnusedCall(cId) =>
            /\ HasNoSendInFlight(cId)
            /\ HostHoldsNoBuffer(cId)
            /\ ~IsDeliveryCallbackRunning(cId)
            /\ HostOwnsNoPayload(cId)
            /\ ~IsHandleReleased(cId)
            /\ ~IsCancelRequested(cId)

\* The end-of-call guarantee: the runtime retires the handle only once the
\* call is over and everything lent to the host is back, and afterwards
\* nothing can lend or deliver again - both need an active call, and a
\* retired call is not unused so it can never be started again.  A
\* retired call therefore stays clean, which is what makes its arena
\* droppable in one piece.  The reclaiming step also waits for the call's
\* own delivery callback to return and for every buffer given back to be
\* released, both being conditions only the runtime can read.
ReleasedCallIsClean ==
    \A cId \in CallIds :
        IsHandleReleased(cId) =>
            /\ ~L0!IsActiveCall(cId)
            /\ HostOwnsNoPayload(cId)
            /\ HostHoldsNoBuffer(cId)

\* The bridge between the counted view and the named one.  Both are kept:
\* the counter is what the send window and every existing proof read, the
\* states are what carry an identity, and without this conjunct the two
\* could drift - which is exactly how the send window and the ABI comment
\* describing it came apart once already.
LentCountMatchesBufferStates ==
    \A cId \in CallIds :
        buffers_held_by_host[cId] =
            Cardinality({b \in BufferIds : IsLentBuffer(cId, b)})

\* A buffer never goes back: an identity is used once and its state only
\* advances.  This is what makes "eventually freed" a statement about that
\* buffer rather than about a count that happens to reach zero.
UnusedCallsHaveFreshBuffers ==
    \A cId \in CallIds :
        L0!IsUnusedCall(cId) =>
            \A b \in BufferIds : IsFreshBuffer(cId, b)

\* The pipelining contract: never more buffers out of the arena than the
\* depth.  LendSendBuffer is guarded on remaining capacity, sending moves
\* a buffer from the held count to the awaiting count without changing
\* the total, and emitting the WRITE_DONE is what gives the slot back.
SendsInFlightWithinLimit ==
    \A cId \in CallIds :
        SendWindowOccupancy(cId) <= MaxSendsInFlight

\* The send side, in counts: never more WRITE_DONEs than accepted sends,
\* and a running acquittal callback has its emission.  A WRITE_DONE settles
\* a send; it does not claim the message reached the network, and one is
\* owed even for a send abandoned when the call ended.
WriteDonesNeverExceedSends ==
    \A cId \in CallIds :
        write_dones_emitted[cId] <= Len(submitted[cId])

RunningWriteDoneWasEmitted ==
    \A cId \in CallIds :
        IsWriteDoneCallbackRunning(cId) => write_dones_emitted[cId] >= 1

\* Conservation of the release count, which is what counters can carry: the
\* host never releases more payloads than were delivered.  Identities are
\* not modelled, so this is no over-consumption under the conformance
\* assumption that the host releases the right owner, once, in order - not
\* a proof that a given payload is freed exactly once.
ReleasesNeverExceedDeliveries ==
    \A cId \in CallIds :
        payloads_consumed_by_host[cId] <= Len(events_delivered[cId])

\* Both terminals are guarded on a drained send side, and no send is
\* accepted on a terminal call: WRITE_DONE always precedes the terminal.
TerminalCallHasNoSendInFlight ==
    \A cId \in CallIds :
        L0!IsTerminalCall(cId) => HasNoSendInFlight(cId)

\* Both paths into "closing" latch cancellation on the channel's active
\* calls, and no call can start on a closing channel.  This is what lets
\* WF(DeliverCancelled) drain a closing channel.
ClosingChannelCallsCancelRequested ==
    \A cId \in CallIds :
        /\ L0!IsActiveCall(cId)
        /\ call_channel[cId] \in ChannelIds
        /\ IsClosingChannel(call_channel[cId])
        => IsCancelRequested(cId)

\* While a call is active it owes at most the credit count: non-terminal
\* deliveries require a credit left, and the terminal ends activity.
ActiveCallPayloadsWithinCredits ==
    \A cId \in CallIds :
        L0!IsActiveCall(cId) => HostOwnsAtMostCredits(cId)

\* One over the credits only when the last is the terminal.
PayloadsOwnedWithinCreditsPlusOne ==
    \A cId \in CallIds : HostOwnsAtMostCreditsPlusOne(cId)

\* A call that has never received a delivery owes nothing: the delivery
\* debt is created by HandPayloadToHost alone, which also appends the
\* first event.
\* This is what gives the initial metadata its delivery credit without
\* any help from the host.
NoDeliveryImpliesNoDebt ==
    \A cId \in CallIds :
        HasNoDeliveredEvents(cId) =>
            /\ HostOwnsNoPayload(cId)
            /\ ~IsDeliveryCallbackRunning(cId)

\* A status event terminates the call in the same step that appends it,
\* so an active call never carries one.  Guard-based, so it holds on the
\* whole behavior; the cancellation drain needs it after a failure too.
ActiveCallHasNoStatus ==
    \A cId \in CallIds :
        L0!IsActiveCall(cId) => ~L0!HasStatus(cId)

\* Deliveries require an active call, so an unused one has no events;
\* starting a call therefore starts it status-free.  Guard-based too.
UnusedCallHasNoEvents ==
    \A cId \in CallIds :
        L0!IsUnusedCall(cId) => HasNoDeliveredEvents(cId)

FfiCallInv ==
    /\ UnusedCallsAreFfiClean
    /\ ReleasedCallIsClean
    /\ SendsInFlightWithinLimit
    /\ WriteDonesNeverExceedSends
    /\ RunningWriteDoneWasEmitted
    /\ TerminalCallHasNoSendInFlight
    /\ ClosingChannelCallsCancelRequested
    /\ ActiveCallPayloadsWithinCredits
    /\ PayloadsOwnedWithinCreditsPlusOne
    /\ ReleasesNeverExceedDeliveries
    /\ NoDeliveryImpliesNoDebt
    /\ ActiveCallHasNoStatus
    /\ UnusedCallHasNoEvents

\* The buffer identities, kept apart from FfiCallInv for the same reason
\* BufferTypes is kept apart from FfiTypes: every per-action lemma about
\* FfiCallInv would otherwise have to be reopened, and none of them has
\* anything to say about a buffer's name.  One lemma over Next carries
\* both conjuncts instead.
\* A send index recorded against a buffer is a send that exists.
BufferSendIndicesExist ==
    \A cId \in CallIds :
        \A b \in BufferIds :
            buffer_send[cId][b] <= Len(submitted[cId])

\* A buffer that carries a send has been given back - committing it is one
\* of the two ways to give a buffer back - so it is returned or already
\* freed, never lent and never fresh.  This is what stops one allocation
\* being handed out twice.
CommittedBuffersAreGivenBack ==
    \A cId \in CallIds :
        \A b \in BufferIds :
            buffer_send[cId][b] # 0 =>
                buffer_state[cId][b] \in {"returned", "freed"}

\* The one that carries real content: the bytes of an unacquitted send are
\* still there.  Without the send-to-buffer link the model would have
\* permitted releasing the memory of a message the transport had not read
\* yet - a use-after-free no counter could have caught, because a counter
\* does not know which allocation carries which send.
UnacquittedSendKeepsItsBytes ==
    \A cId \in CallIds :
        \A b \in BufferIds :
            buffer_send[cId][b] > write_dones_emitted[cId] =>
                buffer_state[cId][b] = "returned"

\* Every accepted send lives in an allocation.  The step that accepts a
\* send is the step that records it against the buffer the caller passed,
\* so a send without a buffer is unreachable - and this is what says so,
\* rather than leaving it to be read off the actions.
EverySendHasItsBuffer ==
    \A cId \in CallIds :
        \A k \in 1..Len(submitted[cId]) :
            \E b \in BufferIds : buffer_send[cId][b] = k

\* And no two allocations claim the same send: a commit records the index
\* the submitted sequence is about to reach, which is above every index
\* already recorded.  With the conjunct above, each accepted send lives in
\* exactly one allocation - the formal content of that sentence, which the
\* keying alone does not give.
SendsLiveInOneBuffer ==
    \A cId \in CallIds :
        \A b1, b2 \in BufferIds :
            (/\ buffer_send[cId][b1] # 0
             /\ buffer_send[cId][b1] = buffer_send[cId][b2]) => b1 = b2

BufferStateInv ==
    /\ LentCountMatchesBufferStates
    /\ UnusedCallsHaveFreshBuffers
    /\ BufferSendIndicesExist
    /\ CommittedBuffersAreGivenBack
    /\ UnacquittedSendKeepsItsBytes
    /\ EverySendHasItsBuffer
    /\ SendsLiveInOneBuffer

\* The shutdown signal discipline.  The last conjunct is the formal content of
\* "SHUTDOWN_COMPLETE is emitted from a drained runtime": every channel closed,
\* no delivery callback on the host stack, every send acquitted.  That is the
\* functional drain and no more - it says nothing about what the host still
\* holds, which is what the release tag reports and what quiescence adds.
ShutdownSignalCore ==
    \A rtId \in RuntimeIds :
        /\ (IsShutdownCallbackRunning(rtId) => IsShutdownEventEmitted(rtId))
        /\ (IsShutdownEventEmitted(rtId) =>
                (IsStoppingRuntime(rtId) \/ IsReleasedRuntime(rtId)))
        /\ (IsReleasedRuntime(rtId) =>
                /\ IsShutdownEventEmitted(rtId)
                /\ ~IsShutdownCallbackRunning(rtId))
        /\ (IsShutdownEventEmitted(rtId) => IsRuntimeDrained(rtId))

\* The release signal, kept apart from the four conjuncts above because it is
\* preserved by a different argument: those follow from the shutdown steps
\* alone, this one also needs that a drained runtime's ledger cannot grow.
\* Splitting them keeps each preservation obligation the size it was.
ReleaseSignalInv ==
    \A rtId \in RuntimeIds :
        /\ (IsResourcesReleasedCallbackRunning(rtId) =>
                IsResourcesReleasedEmitted(rtId))
        /\ (IsResourcesReleasedEmitted(rtId) =>
                /\ IsShutdownEventEmitted(rtId)
                /\ SecondEventOwed(rtId))
        /\ (IsShutdownEventEmitted(rtId) /\ ~SecondEventOwed(rtId) =>
                NoHostDebt(rtId))
\* What the event means, and not only when it may go out: once RESOURCES_RELEASED
\* has been emitted, nothing of the runtime is in the host's hands and nothing
\* the host gave back is still waiting to be freed.  Without this the proof says
\* the event arrives and says nothing about what it announces.
        /\ (IsResourcesReleasedEmitted(rtId) =>
                /\ NoHostDebt(rtId)
                /\ RuntimeHoldsNoReturnedBytes(rtId))

ShutdownSignalInv ==
    /\ ShutdownSignalCore
    /\ ReleaseSignalInv

\* The unload condition, kept apart from the four conjuncts above because
\* it is the only one that reaches into the calls: a destroyed runtime was
\* finished and owed the host nothing, and stays that way.  Quiescence is
\* not enough for the second half - shutdown deliberately does not wait
\* for the host to consume - so the guard on RuntimeDestroy is what
\* establishes it, and no call of a released runtime being active is what
\* keeps it.
DestroyedRuntimeIsClean ==
    \A rtId \in RuntimeIds :
        IsRuntimeDestroyed(rtId) => IsRuntimeQuiescent(rtId)

StrongInv ==
    /\ L0!StrongInv
    /\ TypeOK
    /\ FfiCallInv
    /\ BufferStateInv
    /\ ShutdownSignalInv
    /\ DestroyedRuntimeIsClean

\* FfiCallInv sits outside the NotFailed umbrella: its preservation is
\* guard-based only, and the fairness lifts need the send and delivery
\* disciplines on the whole behavior, failure included.  ShutdownSignalInv
\* stays under NotFailed: failure breaks its runtime-state conjunct.
IndInv ==
    /\ TypeOK
    /\ L0!SingleRuntime
    /\ FfiCallInv
    /\ BufferStateInv
    /\ (L0!NotFailed => StrongInv)

\* The level-1 safety contract: the inherited level-0 invariant plus the
\* FFI disciplines, the failure-free ones under the same umbrella as at
\* level 0.
SafetyInvariant ==
    /\ L0!SafetyInvariant
    /\ FfiCallInv
    /\ BufferStateInv
    /\ (L0!NotFailed => ShutdownSignalInv)
    /\ (L0!NotFailed => DestroyedRuntimeIsClean)

=============================================================================
