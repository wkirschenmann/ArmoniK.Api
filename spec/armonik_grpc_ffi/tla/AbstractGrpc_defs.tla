------------------------- MODULE AbstractGrpc_defs --------------------------
(***************************************************************************)
(* Proof-support definitions for AbstractGrpc: the footprint decomposition *)
(* of Next, the inductive strengthening, and the status-pending phases.    *)
(* Pure definitions only, so both TLAPS modules and TLC model-check        *)
(* configurations can extend this module.                                  *)
(***************************************************************************)

EXTENDS AbstractGrpc

(***************************************************************************)
(* NEXT-STATE DECOMPOSITION BY WRITE FOOTPRINT                             *)
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
    \/ \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
    \/ \E cId \in CallIds : EndSend(cId)
    \/ \E cId \in CallIds : NetworkSend(cId)
    \/ \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    \/ \E cId \in CallIds : ReceiveStatus(cId)
    \/ \E cId \in CallIds : DeliverInitialMetadata(cId)
    \/ \E cId \in CallIds : DeliverMessage(cId)
    \/ \E cId \in CallIds : DeliverStatus(cId)
    \/ \E cId \in CallIds : CallCancel(cId)

NextSafe ==
    \/ NextSafeRuntimeOnly
    \/ NextSafeRuntimeChannel
    \/ NextSafeChannelOnly
    \/ NextSafeChannelCall
    \/ NextSafeCallOnly

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
(* MINIMAL INDUCTIVE STRENGTHENING                                         *)
(***************************************************************************)

ChannelSentinelEquivalence ==
    \A chId \in ChannelIds :
        channel_state[chId] = "none" <=> channel_runtime[chId] = "none"

CallSentinelEquivalence ==
    \A cId \in CallIds :
        IsUnusedCall(cId) <=> call_channel[cId] = "none"

UnusedCallsAreEmpty ==
    \A cId \in CallIds :
        IsUnusedCall(cId) =>
            /\ submitted[cId] = <<>>
            /\ sent[cId] = <<>>
            /\ received[cId] = <<>>
            /\ delivered[cId] = <<>>
            /\ events_delivered[cId] = <<>>
            /\ send_closed[cId] = FALSE

\* Before failure, a created channel's state compactly characterizes the
\* possible lifecycle state of its owning runtime.
ChannelLifecycleInv ==
    \A chId \in UsedChannels :
        /\ channel_runtime[chId] \in RuntimeIds
        /\ CASE channel_state[chId] = "open" ->
                    runtime_state[channel_runtime[chId]] = "RUNNING"
                [] channel_state[chId] = "closing" ->
                    runtime_state[channel_runtime[chId]] \in {"RUNNING", "STOPPING"}
                [] channel_state[chId] = "closed" ->
                    runtime_state[channel_runtime[chId]] \in {"RUNNING", "STOPPING", "RELEASED"}
                [] OTHER -> FALSE

\* Used calls retain a created owner channel. Active calls remain on active
\* channels and their send state identifies whether sending has ended.
\* Pending status is confined to active calls and is cleared at terminal.
CallLifecycleInv ==
    /\ TerminalStatusEquivalence
    /\ \A cId \in CallIds :
        /\ IsUnusedCall(cId) => ~status_pending[cId]
        /\ IsTerminalCall(cId) => ~status_pending[cId]
        /\ status_pending[cId] =>
              /\ IsActiveCall(cId)
              /\ ~HasStatus(cId)
    /\ \A cId \in UsedCalls :
        /\ call_channel[cId] \in UsedChannels
        /\ IsActiveCall(cId) => call_channel[cId] \in ActiveChannels
        /\ IsActiveCall(cId) =>
              (send_closed[cId] <=> call_state[cId] = "half_closed")

MessageFlowInv ==
    /\ SubmittedPrefixOfSent
    /\ ReceivedPrefixOfDelivered
    /\ CompleteDelivery

StructuralInv ==
    /\ TypeOK
    /\ SingleRuntime
    /\ ChannelSentinelEquivalence
    /\ CallSentinelEquivalence
    /\ UnusedCallsAreEmpty
    /\ ChannelLifecycleInv
    /\ CallLifecycleInv

EventTraceInv ==
    /\ EventStreamShape
    /\ MessageEventsMatchDelivered

StrongInv ==
    /\ StructuralInv
    /\ EventTraceInv
    /\ MessageFlowInv

IndInv ==
    /\ TypeOK
    /\ SingleRuntime
    /\ (NotFailed => StrongInv)

(***************************************************************************)
(* PROOF-ONLY STATUS-PENDING PHASES                                        *)
(***************************************************************************)

ReceiveDebt(cId) == Len(received[cId]) - Len(delivered[cId])

NeedsInitialMetadata(cId) ==
    /\ status_pending[cId]
    /\ events_delivered[cId] = <<>>

HasMessageBacklog(cId) ==
    /\ status_pending[cId]
    /\ events_delivered[cId] # <<>>
    /\ ReceiveDebt(cId) > 0

ReadyToDeliverStatus(cId) ==
    /\ status_pending[cId]
    /\ events_delivered[cId] # <<>>
    /\ ReceiveDebt(cId) = 0

PendingStatusPhase ==
    \A cId \in CallIds :
        status_pending[cId] =>
            \/ NeedsInitialMetadata(cId)
            \/ HasMessageBacklog(cId)
            \/ ReadyToDeliverStatus(cId)

=============================================================================
