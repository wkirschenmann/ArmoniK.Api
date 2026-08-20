--------------------------- MODULE FfiGrpcTheorems ---------------------------
(***************************************************************************)
(* Declarations of the proved results about FfiGrpc; the proofs are in     *)
(* FfiGrpcTheorems_proofs, which restates each declaration verbatim and    *)
(* discharges it with tlapm.                                               *)
(*                                                                         *)
(* This module is the citable interface: read it alone to know what holds  *)
(* and under which fairness assumptions, and INSTANCE it from level 2.     *)
(*                                                                         *)
(* RefinesSpec is the headline: every level-1 behavior is a level-0        *)
(* behavior, fairness included, so level-0 safety and the five level-0     *)
(* liveness guarantees hold here without being re-proved.  The seven       *)
(* XxxLift theorems carry the fairness half: each level-0 WF conjunct is   *)
(* discharged by the level-1 machinery, and the WFs each lift consumes     *)
(* are listed one by one.                                                  *)
(*                                                                         *)
(* Level 1 leaves six hypotheses open, all on the host: WF on              *)
(* HostConsumesEvent, WF on HostReturnsBuffer per buffer, and the four     *)
(* callback returns.  The first says the host consumes each payload it     *)
(* holds and is consumed by the DeliverMessage lift and                    *)
(* PayloadsEventuallyConsumed; the second drains the send arena and feeds  *)
(* BufferEventuallyFreed and the two reclamation results.  Every other     *)
(* fairness conjunct is a promise                                          *)
(* of binding-owned threads.  The shutdown chain depends on no ownership   *)
(* return - neither HostConsumesEvent nor HostReturnsBuffer appears in     *)
(* ShutdownEventEmitted or the RuntimeRelease lift - which is what         *)
(* discharges the level-0 directive on ShutdownFairness.  It does depend   *)
(* on the callbacks already dispatched returning: the lift consumes        *)
(* ShutdownCallbackReturns and DeliveryCallbackReturns, and                *)
(* ShutdownEmitFairnessRequirement consumes WriteDoneReturns too.          *)
(* RuntimeEventuallyQuiescent is the one guarantee past that chain that    *)
(* does depend on the host: nothing can report that the last callback      *)
(* returned except the host returning it.                                  *)
(* Level 2 decides who honors the hypothesis; level 1 only names it.       *)
(***************************************************************************)

EXTENDS FfiGrpc_defs

(***************************************************************************)
(* INDUCTIVE INVARIANT AND SAFETY                                          *)
(***************************************************************************)

THEOREM InitEstablishesIndInv == Init => IndInv

THEOREM IndInvPreserved == IndInv /\ [Next]_vars => IndInv'

THEOREM IndInvImpliesSafetyInvariant == IndInv => SafetyInvariant

THEOREM BehaviorEstablishesIndInv ==
    Init /\ [][Next]_vars => []IndInv

THEOREM NominalBehaviorEstablishesStrongInv ==
    Init /\ [][NextSafe]_vars => []StrongInv

THEOREM SafetyTheorem == Spec => []SafetyInvariant

\* The notification is honest: WRITE_DONE really hands a slot back, so no
\* allocation is refused for want of a slot.  It does not say a lend is
\* enabled - cancellation, a closed send side or a retired handle each refuse
\* one on their own grounds, and none of those is a slot shortage.  Nothing
\* forced this to be stated - the send side is host-driven, so no fairness
\* lift needed it - and its absence is exactly what let the slot
\* accounting drift from the ABI it documents.
THEOREM WriteDoneFreesASlot ==
    ASSUME NEW cId \in CallIds, StrongInv, EmitWriteDone(cId)
    PROVE  (HasFreeSendSlot(cId))'

\* Destroying a runtime really does void its call handles: nothing that
\* names a call of a destroyed runtime is ever enabled again.  Requesting
\* a cancellation is the only downcall a finished call could still accept,
\* and a guard says so; the rest follow from what destruction required -
\* a released runtime has no active call, and nothing of its memory is
\* out.  Reclamation is in the list too, and it is the runtime's own step:
\* it does not reclaim a call whose runtime is already gone.
THEOREM DestroyedRuntimeRejectsHandles ==
    ASSUME StrongInv, NEW rtId \in RuntimeIds, IsRuntimeDestroyed(rtId),
           NEW cId \in CallIds,
           call_channel[cId] \in L0!ChannelsOf(rtId)
    PROVE  /\ ~ReleaseCallHandle(cId)
           /\ ~RequestCallCancellation(cId)
           /\ ~EndSend(cId)
           /\ \A msg \in Messages, b \in BufferIds :
                 ~SendMessage(cId, msg, b)
           /\ \A b \in BufferIds, msg \in Messages, ch \in Sizes :
                 ~LendSendBuffer(cId, b, msg, ch)
           /\ \A b \in BufferIds : ~HostReturnsBuffer(cId, b)

(***************************************************************************)
(* REFINEMENT - the step half                                              *)
(***************************************************************************)

THEOREM RefinesInit == Init => L0!Init

THEOREM RefinesNext == [Next]_vars => [L0!Next]_l0_vars

THEOREM RefinesSafeNext == [NextSafe]_vars => [L0!NextSafe]_l0_vars

(***************************************************************************)
(* REFINEMENT - the fairness half: the seven lifts                         *)
(* Stated on whole behaviors ([]IndInv, [][Next]_vars): a level-0 WF       *)
(* constrains the behavior after a failure too, and the send and delivery  *)
(* disciplines (FfiCallInv) hold there by guards alone.                    *)
(***************************************************************************)

THEOREM NetworkSendLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(NetworkSend(cId))
           => WF_l0_vars(L0!NetworkSend(cId))

THEOREM ReceiveStatusLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(ReceiveStatus(cId))
           => WF_l0_vars(L0!ReceiveStatus(cId))

\* NoDeliveryImpliesNoDebt gives the metadata delivery its credit on
\* its own: no host help.
THEOREM MetadataDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => WF_l0_vars(L0!DeliverInitialMetadata(cId))

\* The one lift that consumes the host hypothesis: a message delivery
\* needs a delivery credit, and only HostConsumesEvent frees one.
THEOREM MessageDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => WF_l0_vars(L0!DeliverMessage(cId))

\* HasFreeDeliverySlotForTerminal tolerates spent credits, so the
\* terminal does not need HostConsumesEvent; it waits only for the
\* callback and the send drain.
THEOREM StatusDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverStatus(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ WF_vars(DeliverCancelled(cId))
           => WF_l0_vars(L0!DeliverStatus(cId))

THEOREM ReleaseLift ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ WF_vars(ShutdownCallbackReturns(rtId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           => WF_l0_vars(L0!RuntimeRelease(rtId))

THEOREM ChannelCloseLift ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => WF_l0_vars(L0!ChannelFinishClosing(chId))

(***************************************************************************)
(* REFINEMENT - assembled                                                  *)
(***************************************************************************)

THEOREM RefinesSpec == Spec => L0!Spec

(***************************************************************************)
(* INHERITED GUARANTEES                                                    *)
(***************************************************************************)

THEOREM InheritedSafety == Spec => []L0!SafetyInvariant

THEOREM InheritedLivenessTheorem == Spec => L0!LivenessProperties

(***************************************************************************)
(* NEW LIVENESS - failure-free world, then Init-anchored, then under Spec  *)
(* Each XxxFairnessRequirement lists exactly the WF conjuncts the          *)
(* guarantee consumes.                                                     *)
(***************************************************************************)

THEOREM CancellationProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                   ~L0!IsActiveCall(cId))

THEOREM SendAcquittalProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))

\* The host hypothesis, isolated: payload k is discharged by the host
\* releasing what it holds, and by nothing else.  Release being FIFO, one
\* fairness conjunct per call carries every payload of that call.
THEOREM PayloadsProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((HostOwnsPayload(cId, k)) ~>
                   (~HostOwnsPayload(cId, k)))

THEOREM ShutdownEmitProgressSafeFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => ((IsStoppingRuntime(rtId)) ~>
                   IsShutdownEventEmitted(rtId))

THEOREM CancellationFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                   ~L0!IsActiveCall(cId))

THEOREM SendAcquittalFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))

THEOREM PayloadsFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((HostOwnsPayload(cId, k)) ~>
                   (~HostOwnsPayload(cId, k)))

THEOREM ShutdownEmitFairnessRequirement ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => ((IsStoppingRuntime(rtId)) ~>
                   IsShutdownEventEmitted(rtId))

THEOREM CancellationCompletesHolds == Spec => CancellationCompletes

THEOREM SendsEventuallyAcquittedHolds == Spec => SendsEventuallyAcquitted

THEOREM PayloadsEventuallyConsumedHolds == Spec => PayloadsEventuallyConsumed

THEOREM ShutdownEventEmittedHolds == Spec => ShutdownEventEmitted

\* The four callback-return guarantees.  Each is one step of the fairness
\* it rests on, and each turns an obligation the ABI imposes on the host
\* into something named that the manifest can check.
THEOREM DeliveryCallbacksReturnHolds == Spec => DeliveryCallbacksReturn

THEOREM WriteDoneCallbacksReturnHolds == Spec => WriteDoneCallbacksReturn

THEOREM ShutdownCallbacksReturnHolds == Spec => ShutdownCallbacksReturn

THEOREM ResourcesReleasedCallbacksReturnHolds ==
    Spec => ResourcesReleasedCallbacksReturn

\* Every lent buffer is given back and then released.
THEOREM BufferEventuallyFreedHolds == Spec => BufferEventuallyFreed

\* And the two reclamation guarantees, which are what the removal of
\* ak_call_release from the ABI buys: the runtime does the work, so the
\* model can promise it.
THEOREM CallEventuallyReclaimedHolds == Spec => CallEventuallyReclaimed

THEOREM RuntimeEventuallyQuiescentHolds ==
    Spec => RuntimeEventuallyQuiescent

\* The emission budget's own promise, on the state alone: a request the ABI
\* would consider, refused for want of room, eventually has room.  It says
\* nothing about who is served.
THEOREM BudgetEventuallyAdmitsHolds ==
    Spec => BudgetEventuallyAdmits

\* What the host is actually promised: the request that was refused is granted.
\* Weaker than it looks in one respect and stronger in another - the fairness it
\* rests on is the host's own, since LendSendBuffer is a successful downcall and
\* forcing it forces the host to keep asking.  The escape is the call leaving
\* the state where lending means anything: a cancelled call is owed no buffer.
THEOREM BudgetRefusalEventuallyLendsHolds ==
    Spec => BudgetRefusalEventuallyLends

\* The release tag's own promise, named so a level-2 binding can refine it
\* rather than re-derive it: a host that was told to give memory back is told
\* when the runtime is done with what came back.
THEOREM ResourcesReleasedEventuallyHolds ==
    Spec => ResourcesReleasedEventually

THEOREM LivenessTheorem == Spec => LivenessProperties

=============================================================================
