------------------------- MODULE AbstractGrpcTheorems -------------------------
(***************************************************************************)
(* Declarations of the proved results about AbstractGrpc; the proofs are   *)
(* in AbstractGrpcTheorems_proofs, which restates each declaration         *)
(* verbatim and discharges it with tlapm (safety and all five liveness     *)
(* guarantees, no omitted step, no assumption beyond the six constant      *)
(* assumptions of the specification).                                      *)
(*                                                                         *)
(* This module is the citable interface: read it alone to know what holds  *)
(* and under which fairness assumptions, and INSTANCE it from higher       *)
(* levels.                                                                 *)
(*                                                                         *)
(* The XxxFairnessRequirement theorems state each guarantee in the         *)
(* failure-free world against the WF conjuncts it actually consumes,       *)
(* listed one by one.  Level 0 has no demand variable, so every Deliver*   *)
(* action is guarded by availability alone: a WF on one of them is a       *)
(* continuing obligation of the caller, not a promise of the library.      *)
(* The trigger of each guarantee (CallStart, SendMessage,                  *)
(* RuntimeBeginShutdown) is deliberately unfair and sits in the            *)
(* antecedent of the leads-to.  Shutdown is the only guarantee with no     *)
(* Deliver* conjunct: once the caller has asked for shutdown, completion   *)
(* does not depend on the caller reading anything.  The level-1            *)
(* refinement must discharge ShutdownFairness with binding-owned threads   *)
(* alone.                                                                  *)
(***************************************************************************)

EXTENDS AbstractGrpc_defs

THEOREM InitEstablishesIndInv == Init => IndInv

THEOREM IndInvPreserved == IndInv /\ [Next]_vars => IndInv'

THEOREM IndInvImpliesSafetyInvariant == IndInv => SafetyInvariant

THEOREM SafetyBehaviorEstablishesIndInv ==
    Init /\ [][Next]_vars => []IndInv

THEOREM NominalBehaviorEstablishesStrongInv ==
    Init /\ [][NextSafe]_vars => []StrongInv

THEOREM MetadataProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => (MetadataWaiting(cId) ~> MetadataDelivered(cId))

THEOREM PendingProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => ((IsActiveCall(cId) /\ status_pending[cId]) ~> HasStatus(cId))

THEOREM TerminalProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(ReceiveStatus(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (TerminalWaiting(cId) ~> TerminalReached(cId))

THEOREM ShutdownProgressSafeFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           => (ShutdownWaiting(rtId) ~> ShutdownReleased(rtId))

THEOREM SubmitProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(NetworkSend(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i))

THEOREM DeliveryProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))

THEOREM SafetyTheorem == Spec => []SafetyInvariant

THEOREM MetadataFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => (MetadataWaiting(cId) ~> MetadataDelivered(cId))

THEOREM TerminalFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(ReceiveStatus(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (TerminalWaiting(cId) ~> TerminalReached(cId))

THEOREM ShutdownFairnessRequirement ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           => (ShutdownWaiting(rtId) ~> ShutdownReleased(rtId))

THEOREM SubmitFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(NetworkSend(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i))

THEOREM DeliveryFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))

THEOREM EventualMetadataHolds == Spec => EventualMetadata

\* The per-call form, declared so a refinement can take one call's
\* promise without stripping the quantifier through a backend.
THEOREM EventualTerminalAt ==
    ASSUME NEW cId \in CallIds
    PROVE  Spec => (TerminalPending(cId) ~> TerminalAnswered(cId))

THEOREM EventualMetadataAt ==
    ASSUME NEW cId \in CallIds
    PROVE  Spec => (MetadataPending(cId) ~> MetadataAnswered(cId))

THEOREM EventualTerminalHolds == Spec => EventualTerminal

THEOREM SubmitProgressHolds == Spec => SubmitProgress

THEOREM DeliveryProgressHolds == Spec => DeliveryProgress

THEOREM EventualShutdownHolds == Spec => EventualShutdown

THEOREM LivenessTheorem == Spec => LivenessProperties

=============================================================================
