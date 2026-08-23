---------------------- MODULE DotNetBindingTheorems_proofs ----------------------
(***************************************************************************)
(* Proofs of the level-2 interface.  Extends the defs module rather than    *)
(* the declarations one: extending the declarations would inherit their     *)
(* unproved statements as facts to re-derive, and tlapm would try each of   *)
(* them again.                                                             *)
(***************************************************************************)

EXTENDS DotNetBinding_defs, TLAPS

(***************************************************************************)
(* REFINEMENT - the initial predicate and the fairness.                    *)
(*                                                                         *)
(* Init conjoins level 1's own Init, so the first is a definition unfold.   *)
(* The fairness is the interesting half: level 1 asks for nineteen weak     *)
(* fairness families over its own tuple, and this level restates every one  *)
(* of them verbatim - thirteen in RuntimeOwedFairness, six in              *)
(* BindingOwedFairness - so the implication is a projection of a            *)
(* conjunction rather than an argument about enabledness.  Isabelle again,  *)
(* the atoms being WF_.                                                    *)
(***************************************************************************)

THEOREM RefinesInit == Init => L1!Init
    BY DEF Init

(***************************************************************************)
(* THE FAIRNESS TRANSFER                                                   *)
(*                                                                         *)
(* Level 1 asks for nineteen weak-fairness families over its own tuple and *)
(* this level states none of them: every conjunct of its fairness is an    *)
(* action of this module.  So each family is earned rather than restated,  *)
(* and the shape is three statements per family - the level-2 step         *)
(* projects onto the level-1 one, the level-1 enabledness brings the       *)
(* level-2 one, and PTL turns the two into the weak fairness.              *)
(*                                                                         *)
(* Each is stated boxed, with the state-level fact as its own step: a      *)
(* lemma proved at a fixed state and boxed by PTL in the citing step needs *)
(* the citation instantiated before necessitation, and that instantiation  *)
(* is what fails.                                                         *)
(*                                                                         *)
(* THE RUNTIME'S THIRTEEN.  Each is carried by the passthrough that is     *)
(* that family and nothing more, so the transfer is one for one and the    *)
(* frame is the whole proof.                                               *)
(***************************************************************************)

LEMMA NetworkSendProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassNetworkSend(cId)>>_vars => <<L1!NetworkSend(cId)>>_l1_vars)
<1>1. <<PassNetworkSend(cId)>>_vars => <<L1!NetworkSend(cId)>>_l1_vars
    BY SMT DEF PassNetworkSend, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA NetworkSendBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!NetworkSend(cId)>>_l1_vars
                  => ENABLED <<PassNetworkSend(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!NetworkSend(cId)>>_l1_vars
          => ENABLED <<PassNetworkSend(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassNetworkSend, ManagedStutter, ManagedTypeOK, L1!NetworkSend,
       L1!L0!NetworkSend, L1!L0!ChannelVars, L1!L0!IsActiveCall,
       L1!L0!RuntimeVars, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM NetworkSendLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassNetworkSend(cId))
           => WF_l1_vars(L1!NetworkSend(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassNetworkSend(cId))
      PROVE  WF_l1_vars(L1!NetworkSend(cId))
    BY <1>1, NetworkSendProjects, NetworkSendBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA ReceiveStatusProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassReceiveStatus(cId)>>_vars => <<L1!ReceiveStatus(cId)>>_l1_vars)
<1>1. <<PassReceiveStatus(cId)>>_vars => <<L1!ReceiveStatus(cId)>>_l1_vars
    BY SMT DEF PassReceiveStatus, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA ReceiveStatusBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!ReceiveStatus(cId)>>_l1_vars
                  => ENABLED <<PassReceiveStatus(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!ReceiveStatus(cId)>>_l1_vars
          => ENABLED <<PassReceiveStatus(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassReceiveStatus, ManagedStutter, ManagedTypeOK,
       L1!ReceiveStatus, L1!L0!ReceiveStatus, L1!L0!ChannelVars,
       L1!L0!IsActiveCall, L1!L0!RuntimeVars, vars, l1_vars, managed_vars,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM ReceiveStatusLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassReceiveStatus(cId))
           => WF_l1_vars(L1!ReceiveStatus(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassReceiveStatus(cId))
      PROVE  WF_l1_vars(L1!ReceiveStatus(cId))
    BY <1>1, ReceiveStatusProjects, ReceiveStatusBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA DeliverInitialMetadataProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassDeliverInitialMetadata(cId)>>_vars => <<L1!DeliverInitialMetadata(cId)>>_l1_vars)
<1>1. <<PassDeliverInitialMetadata(cId)>>_vars => <<L1!DeliverInitialMetadata(cId)>>_l1_vars
    BY SMT DEF PassDeliverInitialMetadata, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA DeliverInitialMetadataBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!DeliverInitialMetadata(cId)>>_l1_vars
                  => ENABLED <<PassDeliverInitialMetadata(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!DeliverInitialMetadata(cId)>>_l1_vars
          => ENABLED <<PassDeliverInitialMetadata(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassDeliverInitialMetadata, ManagedStutter, ManagedTypeOK,
       L1!DeliverInitialMetadata, L1!L0!DeliverInitialMetadata,
       L1!L0!ChannelVars, L1!L0!IsActiveCall, L1!L0!RuntimeVars,
       L1!HandPayloadToHost, L1!HasFreeDeliverySlot, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM DeliverInitialMetadataLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassDeliverInitialMetadata(cId))
           => WF_l1_vars(L1!DeliverInitialMetadata(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassDeliverInitialMetadata(cId))
      PROVE  WF_l1_vars(L1!DeliverInitialMetadata(cId))
    BY <1>1, DeliverInitialMetadataProjects, DeliverInitialMetadataBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA DeliverMessageProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassDeliverMessage(cId)>>_vars => <<L1!DeliverMessage(cId)>>_l1_vars)
<1>1. <<PassDeliverMessage(cId)>>_vars => <<L1!DeliverMessage(cId)>>_l1_vars
    BY SMT DEF PassDeliverMessage, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA DeliverMessageBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!DeliverMessage(cId)>>_l1_vars
                  => ENABLED <<PassDeliverMessage(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!DeliverMessage(cId)>>_l1_vars
          => ENABLED <<PassDeliverMessage(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassDeliverMessage, ManagedStutter, ManagedTypeOK,
       L1!DeliverMessage, L1!L0!DeliverMessage, L1!L0!ChannelVars,
       L1!L0!HasStatus, L1!L0!IsActiveCall, L1!L0!RuntimeVars,
       L1!HandPayloadToHost, L1!HasFreeDeliverySlot,
       L1!HasFreeDeliverySlotForTerminal, L1!IsCancelRequested, vars,
       l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars, L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM DeliverMessageLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassDeliverMessage(cId))
           => WF_l1_vars(L1!DeliverMessage(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassDeliverMessage(cId))
      PROVE  WF_l1_vars(L1!DeliverMessage(cId))
    BY <1>1, DeliverMessageProjects, DeliverMessageBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA DeliverStatusProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassDeliverStatus(cId)>>_vars => <<L1!DeliverStatus(cId)>>_l1_vars)
<1>1. <<PassDeliverStatus(cId)>>_vars => <<L1!DeliverStatus(cId)>>_l1_vars
    BY SMT DEF PassDeliverStatus, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA DeliverStatusBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!DeliverStatus(cId)>>_l1_vars
                  => ENABLED <<PassDeliverStatus(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!DeliverStatus(cId)>>_l1_vars
          => ENABLED <<PassDeliverStatus(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassDeliverStatus, ManagedStutter, ManagedTypeOK,
       L1!DeliverStatus, L1!L0!CallCancel, L1!L0!ChannelVars,
       L1!L0!HasStatus, L1!L0!IsActiveCall, L1!L0!RuntimeVars,
       L1!L0!DeliverStatus, L1!HandPayloadToHost,
       L1!HasFreeDeliverySlotForTerminal, L1!HasNoSendInFlight,
       L1!IsCancelRequested, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM DeliverStatusLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassDeliverStatus(cId))
           => WF_l1_vars(L1!DeliverStatus(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassDeliverStatus(cId))
      PROVE  WF_l1_vars(L1!DeliverStatus(cId))
    BY <1>1, DeliverStatusProjects, DeliverStatusBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA DeliverCancelledProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassDeliverCancelled(cId)>>_vars => <<L1!DeliverCancelled(cId)>>_l1_vars)
<1>1. <<PassDeliverCancelled(cId)>>_vars => <<L1!DeliverCancelled(cId)>>_l1_vars
    BY SMT DEF PassDeliverCancelled, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA DeliverCancelledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!DeliverCancelled(cId)>>_l1_vars
                  => ENABLED <<PassDeliverCancelled(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!DeliverCancelled(cId)>>_l1_vars
          => ENABLED <<PassDeliverCancelled(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassDeliverCancelled, ManagedStutter, ManagedTypeOK,
       L1!DeliverCancelled, L1!L0!CallCancel, L1!L0!ChannelVars,
       L1!L0!HasStatus, L1!L0!IsActiveCall, L1!L0!RuntimeVars,
       L1!HandPayloadToHost, L1!HasFreeDeliverySlotForTerminal,
       L1!HasNoDeliveredEvents, L1!HasNoSendInFlight,
       L1!IsCancelRequested, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM DeliverCancelledLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassDeliverCancelled(cId))
           => WF_l1_vars(L1!DeliverCancelled(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassDeliverCancelled(cId))
      PROVE  WF_l1_vars(L1!DeliverCancelled(cId))
    BY <1>1, DeliverCancelledProjects, DeliverCancelledBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA EmitWriteDoneProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassEmitWriteDone(cId)>>_vars => <<L1!EmitWriteDone(cId)>>_l1_vars)
<1>1. <<PassEmitWriteDone(cId)>>_vars => <<L1!EmitWriteDone(cId)>>_l1_vars
    BY SMT DEF PassEmitWriteDone, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA EmitWriteDoneBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!EmitWriteDone(cId)>>_l1_vars
                  => ENABLED <<PassEmitWriteDone(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!EmitWriteDone(cId)>>_l1_vars
          => ENABLED <<PassEmitWriteDone(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassEmitWriteDone, ManagedStutter, ManagedTypeOK,
       L1!EmitWriteDone, L1!IsAwaitingWriteDone,
       L1!IsWriteDoneCallbackRunning, vars, l1_vars, managed_vars,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM EmitWriteDoneLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassEmitWriteDone(cId))
           => WF_l1_vars(L1!EmitWriteDone(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassEmitWriteDone(cId))
      PROVE  WF_l1_vars(L1!EmitWriteDone(cId))
    BY <1>1, EmitWriteDoneProjects, EmitWriteDoneBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA ReleaseCallHandleProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](<<PassReleaseCallHandle(cId)>>_vars => <<L1!ReleaseCallHandle(cId)>>_l1_vars)
<1>1. <<PassReleaseCallHandle(cId)>>_vars => <<L1!ReleaseCallHandle(cId)>>_l1_vars
    BY SMT DEF PassReleaseCallHandle, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA ReleaseCallHandleBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!ReleaseCallHandle(cId)>>_l1_vars
                  => ENABLED <<PassReleaseCallHandle(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!ReleaseCallHandle(cId)>>_l1_vars
          => ENABLED <<PassReleaseCallHandle(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassReleaseCallHandle, ManagedStutter, ManagedTypeOK,
       L1!ReleaseCallHandle, L1!L0!IsActiveCall, L1!L0!ActiveCallStates,
       L1!L0!IsUnusedCall, L1!HostHoldsNoBuffer, L1!HostOwnsNoPayload,
       L1!IsDeliveryCallbackRunning, L1!IsHandleReleased,
       L1!IsReturnedBuffer, L1!IsRuntimeOfCallDestroyed, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM ReleaseCallHandleLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassReleaseCallHandle(cId))
           => WF_l1_vars(L1!ReleaseCallHandle(cId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassReleaseCallHandle(cId))
      PROVE  WF_l1_vars(L1!ReleaseCallHandle(cId))
    BY <1>1, ReleaseCallHandleProjects, ReleaseCallHandleBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA FreeReturnedBufferProjects ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  [](<<PassFreeReturnedBuffer(cId, b)>>_vars => <<L1!FreeReturnedBuffer(cId, b)>>_l1_vars)
<1>1. <<PassFreeReturnedBuffer(cId, b)>>_vars => <<L1!FreeReturnedBuffer(cId, b)>>_l1_vars
    BY SMT DEF PassFreeReturnedBuffer, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA FreeReturnedBufferBridge ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!FreeReturnedBuffer(cId, b)>>_l1_vars
                  => ENABLED <<PassFreeReturnedBuffer(cId, b)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!FreeReturnedBuffer(cId, b)>>_l1_vars
          => ENABLED <<PassFreeReturnedBuffer(cId, b)>>_vars
    BY ExpandENABLED, SMT
    DEF PassFreeReturnedBuffer, ManagedStutter, ManagedTypeOK,
       L1!FreeReturnedBuffer, L1!CarriesNoUnacquittedSend,
       L1!IsReturnedBuffer, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM FreeReturnedBufferLifted ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassFreeReturnedBuffer(cId, b))
           => WF_l1_vars(L1!FreeReturnedBuffer(cId, b))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassFreeReturnedBuffer(cId, b))
      PROVE  WF_l1_vars(L1!FreeReturnedBuffer(cId, b))
    BY <1>1, FreeReturnedBufferProjects, FreeReturnedBufferBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA RuntimeReleaseProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](<<PassRuntimeRelease(rtId)>>_vars => <<L1!RuntimeRelease(rtId)>>_l1_vars)
<1>1. <<PassRuntimeRelease(rtId)>>_vars => <<L1!RuntimeRelease(rtId)>>_l1_vars
    BY SMT DEF PassRuntimeRelease, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA RuntimeReleaseBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!RuntimeRelease(rtId)>>_l1_vars
                  => ENABLED <<PassRuntimeRelease(rtId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!RuntimeRelease(rtId)>>_l1_vars
          => ENABLED <<PassRuntimeRelease(rtId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassRuntimeRelease, ManagedStutter, ManagedTypeOK,
       L1!RuntimeRelease, L1!L0!RuntimeRelease, L1!L0!CallVars,
       L1!L0!CallsOf, L1!L0!ChannelVars, L1!L0!ChannelsOf,
       L1!FreeReturnedBuffer, L1!IsShutdownCallbackRunning,
       L1!IsShutdownEventEmitted, L1!ReleaseCallHandle, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM RuntimeReleaseLifted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassRuntimeRelease(rtId))
           => WF_l1_vars(L1!RuntimeRelease(rtId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassRuntimeRelease(rtId))
      PROVE  WF_l1_vars(L1!RuntimeRelease(rtId))
    BY <1>1, RuntimeReleaseProjects, RuntimeReleaseBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA EmitShutdownCompleteProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](<<PassEmitShutdownComplete(rtId)>>_vars => <<L1!EmitShutdownComplete(rtId)>>_l1_vars)
<1>1. <<PassEmitShutdownComplete(rtId)>>_vars => <<L1!EmitShutdownComplete(rtId)>>_l1_vars
    BY SMT DEF PassEmitShutdownComplete, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA EmitShutdownCompleteBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!EmitShutdownComplete(rtId)>>_l1_vars
                  => ENABLED <<PassEmitShutdownComplete(rtId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!EmitShutdownComplete(rtId)>>_l1_vars
          => ENABLED <<PassEmitShutdownComplete(rtId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassEmitShutdownComplete, ManagedStutter, ManagedTypeOK,
       L1!EmitShutdownComplete, L1!IsRuntimeDrained,
       L1!IsShutdownEventEmitted, L1!IsStoppingRuntime, L1!NoHostDebt,
       vars, l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars, L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM EmitShutdownCompleteLifted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassEmitShutdownComplete(rtId))
           => WF_l1_vars(L1!EmitShutdownComplete(rtId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassEmitShutdownComplete(rtId))
      PROVE  WF_l1_vars(L1!EmitShutdownComplete(rtId))
    BY <1>1, EmitShutdownCompleteProjects, EmitShutdownCompleteBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA EmitResourcesReleasedProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](<<PassEmitResourcesReleased(rtId)>>_vars => <<L1!EmitResourcesReleased(rtId)>>_l1_vars)
<1>1. <<PassEmitResourcesReleased(rtId)>>_vars => <<L1!EmitResourcesReleased(rtId)>>_l1_vars
    BY SMT DEF PassEmitResourcesReleased, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA EmitResourcesReleasedBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!EmitResourcesReleased(rtId)>>_l1_vars
                  => ENABLED <<PassEmitResourcesReleased(rtId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!EmitResourcesReleased(rtId)>>_l1_vars
          => ENABLED <<PassEmitResourcesReleased(rtId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassEmitResourcesReleased, ManagedStutter, ManagedTypeOK,
       L1!EmitResourcesReleased, L1!IsResourcesReleasedEmitted,
       L1!IsShutdownCallbackRunning, L1!IsShutdownEventEmitted,
       L1!NoHostDebt, L1!RuntimeHoldsNoReturnedBytes, L1!SecondEventOwed,
       vars, l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars, L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM EmitResourcesReleasedLifted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassEmitResourcesReleased(rtId))
           => WF_l1_vars(L1!EmitResourcesReleased(rtId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassEmitResourcesReleased(rtId))
      PROVE  WF_l1_vars(L1!EmitResourcesReleased(rtId))
    BY <1>1, EmitResourcesReleasedProjects, EmitResourcesReleasedBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA ChannelFinishClosingProjects ==
    ASSUME NEW chId \in ChannelIds
    PROVE  [](<<PassChannelFinishClosing(chId)>>_vars => <<L1!ChannelFinishClosing(chId)>>_l1_vars)
<1>1. <<PassChannelFinishClosing(chId)>>_vars => <<L1!ChannelFinishClosing(chId)>>_l1_vars
    BY SMT DEF PassChannelFinishClosing, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA ChannelFinishClosingBridge ==
    ASSUME NEW chId \in ChannelIds
    PROVE  [](ManagedTypeOK /\ ENABLED <<L1!ChannelFinishClosing(chId)>>_l1_vars
                  => ENABLED <<PassChannelFinishClosing(chId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!ChannelFinishClosing(chId)>>_l1_vars
          => ENABLED <<PassChannelFinishClosing(chId)>>_vars
    BY ExpandENABLED, SMT
    DEF PassChannelFinishClosing, ManagedStutter, ManagedTypeOK,
       L1!ChannelFinishClosing, L1!L0!CallsOf, L1!L0!ChannelFinishClosing,
       L1!L0!HasStatus, L1!L0!IsActiveCall, L1!L0!RuntimeVars,
       L1!L0!ActiveCallStates, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

THEOREM ChannelFinishClosingLifted ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []ManagedTypeOK
           /\ [][Next]_vars
           /\ WF_vars(PassChannelFinishClosing(chId))
           => WF_l1_vars(L1!ChannelFinishClosing(chId))
<1>1. ASSUME []ManagedTypeOK, [][Next]_vars,
             WF_vars(PassChannelFinishClosing(chId))
      PROVE  WF_l1_vars(L1!ChannelFinishClosing(chId))
    BY <1>1, ChannelFinishClosingProjects, ChannelFinishClosingBridge, PTL
<1>2. QED BY <1>1, PTL


(***************************************************************************)
(* THE FOUR TRAMPOLINE RETURNS                                             *)
(*                                                                         *)
(* Same shape as the passthroughs, with one difference that changes the    *)
(* proof: these level-2 actions do managed work in the same step, so       *)
(* <<A2>>_vars no longer forces the level-1 tuple to move by itself.       *)
(* Showing that it moved means showing the level-1 EXCEPT changed          *)
(* something, which needs level 1's typing to know the variable is a       *)
(* function on the index set - hence the lifts run off the inductive core  *)
(* rather than the managed typing alone.                                   *)
(***************************************************************************)

LEMMA DeliveryProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](L1!TypeOK /\ <<DeliveryReturns(cId)>>_vars
                  => <<L1!DeliveryCallbackReturns(cId)>>_l1_vars)
<1>1. L1!TypeOK /\ <<DeliveryReturns(cId)>>_vars
          => <<L1!DeliveryCallbackReturns(cId)>>_l1_vars
    BY SMT DEF L1!TypeOK, DeliveryReturns, OnEventReturns,
       TerminalCallbackReturns, L1!DeliveryCallbackReturns,
       L1!IsDeliveryCallbackRunning, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA DeliveryBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](L1!TypeOK /\ ManagedTypeOK
                  /\ ENABLED <<L1!DeliveryCallbackReturns(cId)>>_l1_vars
                  => ENABLED <<DeliveryReturns(cId)>>_vars)
\* One witness per branch, and the case is what picks it: the two split on
\* the status, so a single goal would ask for both witnesses at once and
\* get neither.
<1>1. ASSUME L1!TypeOK, ManagedTypeOK,
             ENABLED <<L1!DeliveryCallbackReturns(cId)>>_l1_vars
      PROVE  ENABLED <<DeliveryReturns(cId)>>_vars
  <2>1. CASE L1!L0!HasStatus(cId)
    BY <1>1, <2>1, ExpandENABLED, SMT
    DEF L1!TypeOK, DeliveryReturns, OnEventReturns,
       TerminalCallbackReturns, ManagedStutter, ManagedTypeOK,
       L1!DeliveryCallbackReturns, L1!IsDeliveryCallbackRunning,
       L1!L0!HasStatus, L1!L0!RuntimeVars, L1!L0!ChannelVars,
       L1!L0!CallVars, vars, l1_vars, managed_vars, L1!vars, L1!l0_vars,
       L1!ffi_vars, L1!L0!vars
  \* The non-terminal branch stutters the managed half, so the frame is the
  \* whole argument and neither typing is needed - and unfolding them here
  \* only buries the witness.  It is also the branch that needs the time:
  \* the work is in the frame, not in the guard.
  <2>2. CASE ~L1!L0!HasStatus(cId)
    BY <1>1, <2>2, ExpandENABLED, SMTT(300)
    DEF DeliveryReturns, OnEventReturns, TerminalCallbackReturns,
       ManagedStutter, L1!DeliveryCallbackReturns,
       L1!IsDeliveryCallbackRunning, L1!L0!HasStatus, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
  <2>3. QED BY <2>1, <2>2
<1>2. QED BY <1>1, PTL

THEOREM DeliveryCallbackReturnsLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedIndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliveryReturns(cId))
           => WF_l1_vars(L1!DeliveryCallbackReturns(cId))
<1>1. ASSUME []ManagedIndInv, [][Next]_vars,
             WF_vars(DeliveryReturns(cId))
      PROVE  WF_l1_vars(L1!DeliveryCallbackReturns(cId))
  <2>1. []ManagedTypeOK /\ []L1!TypeOK
    BY <1>1, PTL DEF ManagedIndInv, L1!IndInv
  <2>2. QED
    BY <1>1, <2>1, DeliveryProjects, DeliveryBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA WriteDoneProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  [](L1!TypeOK /\ <<WriteDoneCompletes(cId)>>_vars
                  => <<L1!WriteDoneReturns(cId)>>_l1_vars)
<1>1. L1!TypeOK /\ <<WriteDoneCompletes(cId)>>_vars
          => <<L1!WriteDoneReturns(cId)>>_l1_vars
    BY SMT DEF L1!TypeOK, WriteDoneCompletes, L1!WriteDoneReturns,
       L1!IsWriteDoneCallbackRunning, vars, l1_vars, managed_vars,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA WriteDoneBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  [](ManagedTypeOK
                  /\ ENABLED <<L1!WriteDoneReturns(cId)>>_l1_vars
                  => ENABLED <<WriteDoneCompletes(cId)>>_vars)
<1>1. ManagedTypeOK /\ ENABLED <<L1!WriteDoneReturns(cId)>>_l1_vars
          => ENABLED <<WriteDoneCompletes(cId)>>_vars
    BY ExpandENABLED, SMT
    DEF WriteDoneCompletes, ManagedTypeOK, L1!WriteDoneReturns,
       L1!IsWriteDoneCallbackRunning, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, vars, l1_vars, managed_vars,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

THEOREM WriteDoneReturnsLifted ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []ManagedIndInv
           /\ [][Next]_vars
           /\ WF_vars(WriteDoneCompletes(cId))
           => WF_l1_vars(L1!WriteDoneReturns(cId))
<1>1. ASSUME []ManagedIndInv, [][Next]_vars,
             WF_vars(WriteDoneCompletes(cId))
      PROVE  WF_l1_vars(L1!WriteDoneReturns(cId))
  <2>1. []ManagedTypeOK /\ []L1!TypeOK
    BY <1>1, PTL DEF ManagedIndInv, L1!IndInv
  <2>2. QED
    BY <1>1, <2>1, WriteDoneProjects, WriteDoneBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA ShutdownProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](<<ShutdownReturns(rtId)>>_vars
                  => <<L1!ShutdownCallbackReturns(rtId)>>_l1_vars)
<1>1. <<ShutdownReturns(rtId)>>_vars
          => <<L1!ShutdownCallbackReturns(rtId)>>_l1_vars
    BY SMT DEF ShutdownReturns, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA ShutdownBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](ManagedTypeOK
                  /\ ENABLED <<L1!ShutdownCallbackReturns(rtId)>>_l1_vars
                  => ENABLED <<ShutdownReturns(rtId)>>_vars)
<1>1. ManagedTypeOK
          /\ ENABLED <<L1!ShutdownCallbackReturns(rtId)>>_l1_vars
          => ENABLED <<ShutdownReturns(rtId)>>_vars
    BY ExpandENABLED, SMT
    DEF ShutdownReturns, ManagedStutter, ManagedTypeOK,
       L1!ShutdownCallbackReturns, L1!IsShutdownCallbackRunning,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       vars, l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars
<1>2. QED BY <1>1, PTL

THEOREM ShutdownCallbackReturnsLifted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []ManagedIndInv
           /\ [][Next]_vars
           /\ WF_vars(ShutdownReturns(rtId))
           => WF_l1_vars(L1!ShutdownCallbackReturns(rtId))
<1>1. ASSUME []ManagedIndInv, [][Next]_vars,
             WF_vars(ShutdownReturns(rtId))
      PROVE  WF_l1_vars(L1!ShutdownCallbackReturns(rtId))
  <2>1. []ManagedTypeOK /\ []L1!TypeOK
    BY <1>1, PTL DEF ManagedIndInv, L1!IndInv
  <2>2. QED
    BY <1>1, <2>1, ShutdownProjects, ShutdownBridge, PTL
<1>2. QED BY <1>1, PTL

LEMMA ResourcesReleasedProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](<<ResourcesReleasedReturns(rtId)>>_vars
                  => <<L1!ResourcesReleasedCallbackReturns(rtId)>>_l1_vars)
<1>1. <<ResourcesReleasedReturns(rtId)>>_vars
          => <<L1!ResourcesReleasedCallbackReturns(rtId)>>_l1_vars
    BY SMT DEF ResourcesReleasedReturns, ManagedStutter, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

LEMMA ResourcesReleasedBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](ManagedTypeOK
                  /\ ENABLED
                         <<L1!ResourcesReleasedCallbackReturns(rtId)>>_l1_vars
                  => ENABLED <<ResourcesReleasedReturns(rtId)>>_vars)
<1>1. ManagedTypeOK
          /\ ENABLED
                 <<L1!ResourcesReleasedCallbackReturns(rtId)>>_l1_vars
          => ENABLED <<ResourcesReleasedReturns(rtId)>>_vars
    BY ExpandENABLED, SMT
    DEF ResourcesReleasedReturns, ManagedStutter, ManagedTypeOK,
       L1!ResourcesReleasedCallbackReturns,
       L1!IsResourcesReleasedCallbackRunning,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       vars, l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars
<1>2. QED BY <1>1, PTL

THEOREM ResourcesReleasedCallbackReturnsLifted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []ManagedIndInv
           /\ [][Next]_vars
           /\ WF_vars(ResourcesReleasedReturns(rtId))
           => WF_l1_vars(L1!ResourcesReleasedCallbackReturns(rtId))
<1>1. ASSUME []ManagedIndInv, [][Next]_vars,
             WF_vars(ResourcesReleasedReturns(rtId))
      PROVE  WF_l1_vars(L1!ResourcesReleasedCallbackReturns(rtId))
  <2>1. []ManagedTypeOK /\ []L1!TypeOK
    BY <1>1, PTL DEF ManagedIndInv, L1!IndInv
  <2>2. QED
    BY <1>1, <2>1, ResourcesReleasedProjects, ResourcesReleasedBridge, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* THE BUFFER RETURN                                                       *)
(*                                                                         *)
(* Not a one-for-one transfer, and it does not need to be.  Level 1 asks   *)
(* that a lent buffer eventually come back or stop being owed; this level  *)
(* promises that a serializing writer settles, and every way it settles    *)
(* takes the buffer out of the host's hands - the abort returns it, the    *)
(* commit hands it to the runtime.  So the level-1 fairness holds because  *)
(* its enabledness cannot persist, which is the second disjunct of a weak  *)
(* fairness and just as good as the first.                                 *)
(***************************************************************************)

\* What the level-1 action asks for, as a state predicate: this is the
\* whole of its guard, so it is exactly its enabledness.
BufferOwed(cId, b) == L1!IsLentBuffer(cId, b) /\ L1!HostHoldsSomeBuffer(cId)

LEMMA OwedIsEnabled ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  [](ENABLED <<L1!HostReturnsBuffer(cId, b)>>_l1_vars
                  => BufferOwed(cId, b))
<1>1. ENABLED <<L1!HostReturnsBuffer(cId, b)>>_l1_vars
          => BufferOwed(cId, b)
    BY ExpandENABLED, SMT
    DEF BufferOwed, L1!HostReturnsBuffer, L1!IsLentBuffer,
       L1!HostHoldsSomeBuffer, l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars
<1>2. QED BY <1>1, PTL

\* An owed buffer means a serializing writer, and a serializing writer can
\* always abort: the wrapper's return needs nothing but the buffer it
\* holds.  So the settling is enabled wherever the level-1 action is.
LEMMA OwedMeansSettlingEnabled ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  [](L1!TypeOK /\ ManagedTypeOK
                  /\ SerializingWriterHoldsTheBuffer
                  /\ BufferOwed(cId, b)
                  => ENABLED <<SerializationSettles(cId)>>_vars)
\* Three steps, and the order is forced: ExpandENABLED will not build a
\* witness for a quantifier written inside the action, but it does carry
\* one across an implication.  So the abort at a named buffer first - its
\* witness is syntactic - then the buffer quantified, then the branch
\* picked out of the settling.
<1>1. ASSUME L1!TypeOK, ManagedTypeOK, SerializingWriterHoldsTheBuffer,
             BufferOwed(cId, b)
      PROVE  ENABLED <<SerializationSettles(cId)>>_vars
  <2>1. ENABLED <<WriteAborted(cId, b)>>_vars
    BY <1>1, ExpandENABLED, SMT
    DEF BufferOwed, WriteAborted, SerializingWriterHoldsTheBuffer,
       L1!TypeOK, ManagedTypeOK, L1!HostReturnsBuffer, L1!IsLentBuffer,
       L1!HostHoldsSomeBuffer, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars
  <2>2. ENABLED <<WriteAbortsSomewhere(cId)>>_vars
    BY <2>1, ExpandENABLED, SMTT(120)
    DEF WriteAbortsSomewhere, WriteAborted, L1!HostReturnsBuffer,
       L1!IsLentBuffer, L1!HostHoldsSomeBuffer, vars, l1_vars,
       managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
  <2>3. QED
    BY <2>2, ExpandENABLED, SMTT(120)
    DEF WriteAbortsSomewhere, SerializationSettles, WriteAborted,
       CommitWrite, L1!HostReturnsBuffer, L1!SendMessage,
       L1!L0!SendMessage, L1!IsLentBuffer, L1!HostHoldsSomeBuffer, vars,
       l1_vars, managed_vars, L1!vars, L1!l0_vars, L1!ffi_vars,
       L1!L0!vars, L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars
<1>2. QED BY <1>1, PTL

\* However the serialization settles, the host no longer holds a buffer:
\* the abort gives it back and the commit sends it, and the writer held
\* exactly one, so the count reaches zero either way.  The debt therefore
\* cannot outlive one settling step.
LEMMA SettlingClearsTheDebt ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  [](L1!TypeOK /\ SerializingWriterHoldsTheBuffer
                  /\ BufferOwed(cId, b)
                  /\ <<SerializationSettles(cId)>>_vars
                  => ~(BufferOwed(cId, b))')
<1>1. L1!TypeOK /\ SerializingWriterHoldsTheBuffer
          /\ BufferOwed(cId, b)
          /\ <<SerializationSettles(cId)>>_vars
          => ~(BufferOwed(cId, b))'
    BY SMT
    DEF BufferOwed, SerializationSettles, WriteAbortsSomewhere,
       WriteAborted, CommitWrite, SerializingWriterHoldsTheBuffer,
       L1!TypeOK, L1!HostReturnsBuffer, L1!SendMessage, L1!IsLentBuffer,
       L1!HostHoldsSomeBuffer, vars, l1_vars, managed_vars, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars
<1>2. QED BY <1>1, PTL

THEOREM HostReturnsBufferLifted ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ []ManagedIndInv
           /\ [][Next]_vars
           /\ WF_vars(SerializationSettles(cId))
           => WF_l1_vars(L1!HostReturnsBuffer(cId, b))
<1>1. ASSUME []ManagedIndInv, [][Next]_vars,
             WF_vars(SerializationSettles(cId))
      PROVE  WF_l1_vars(L1!HostReturnsBuffer(cId, b))
  <2>1. []L1!TypeOK /\ []ManagedTypeOK /\ []SerializingWriterHoldsTheBuffer
    BY <1>1, PTL
    DEF ManagedIndInv, L1!IndInv, ManagedMachineInv, WriterInv
  <2>2. QED
    BY <1>1, <2>1, OwedIsEnabled, OwedMeansSettlingEnabled,
       SettlingClearsTheDebt, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* REFINEMENT OF THE NEXT-STATE RELATION                                   *)
(*                                                                         *)
(* Every action of this level does one of two things to level 1's tuple:    *)
(* it leaves it alone, being managed-only, or it conjoins the level-1       *)
(* action it rides on.  So each disjunct gets its own lemma - a managed one *)
(* needs no more than its own definition, a coupled one needs level 1's    *)
(* Next unfolded to see the disjunct it lands in - and the theorem is the   *)
(* case analysis over Next.                                                *)
(***************************************************************************)

LEMMA ProjectsFreeRuntimeRoot ==
    ASSUME FreeRuntimeRoot
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FreeRuntimeRoot

LEMMA ProjectsCreateRuntime ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds, CreateRuntime(rtId, chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CreateRuntime, L1!Next

LEMMA ProjectsAcquireLease ==
    ASSUME NEW chId \in ChannelIds, AcquireLease(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF AcquireLease

LEMMA ProjectsCreateChannel ==
    \* The witness for level 1's runtime is current_runtime, which is in
    \* RuntimeIds only because the invariant says so and the guard excludes
    \* the sentinel.
    ASSUME ManagedTypeOK, NEW chId \in ChannelIds, CreateChannel(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF ManagedTypeOK, CreateChannel, L1!Next

LEMMA ProjectsRejectChannelCreation ==
    ASSUME NEW chId \in ChannelIds, RejectChannelCreation(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF RejectChannelCreation

LEMMA ProjectsBeginDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, BeginDisposeChannel(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF BeginDisposeChannel

LEMMA ProjectsFinishDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, FinishDisposeChannel(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeChannel, L1!Next

LEMMA ProjectsResolveChannelDispose ==
    ASSUME NEW chId \in ChannelIds, ResolveChannelDispose(chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF ResolveChannelDispose

LEMMA ProjectsBeginRuntimeShutdown ==
    ASSUME NEW rtId \in RuntimeIds, BeginRuntimeShutdown(rtId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF BeginRuntimeShutdown, L1!Next

LEMMA ProjectsFinishDisposeRuntime ==
    ASSUME NEW rtId \in RuntimeIds, FinishDisposeRuntime(rtId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeRuntime, L1!Next

LEMMA ProjectsShutdownReturns ==
    ASSUME NEW rtId \in RuntimeIds, ShutdownReturns(rtId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF ShutdownReturns, L1!Next

LEMMA ProjectsResourcesReleasedReturns ==
    ASSUME NEW rtId \in RuntimeIds, ResourcesReleasedReturns(rtId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF ResourcesReleasedReturns, L1!Next

LEMMA ProjectsBeginMoveNext ==
    ASSUME NEW cId \in CallIds, BeginMoveNext(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF BeginMoveNext

LEMMA ProjectsBeginParseEvent ==
    ASSUME NEW cId \in CallIds, BeginParseEvent(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF BeginParseEvent

LEMMA ProjectsFinishConsumePayload ==
    ASSUME NEW cId \in CallIds, FinishConsumePayload(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FinishConsumePayload, L1!Next

LEMMA ProjectsCancelWaiter ==
    ASSUME NEW cId \in CallIds, CancelWaiter(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CancelWaiter

LEMMA ProjectsRequestReadCancellation ==
    ASSUME NEW cId \in CallIds, RequestReadCancellation(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF RequestReadCancellation

LEMMA ProjectsCancelWaitingRead ==
    ASSUME NEW cId \in CallIds, CancelWaitingRead(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CancelWaitingRead, L1!Next

LEMMA ProjectsCancelParsingRead ==
    ASSUME NEW cId \in CallIds, CancelParsingRead(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CancelParsingRead, L1!Next

LEMMA ProjectsFinishCancelledParse ==
    ASSUME NEW cId \in CallIds, FinishCancelledParse(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FinishCancelledParse, L1!Next

LEMMA ProjectsHandoffToDrain ==
    ASSUME NEW cId \in CallIds, HandoffToDrain(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF HandoffToDrain

LEMMA ProjectsConsumeHeader ==
    ASSUME NEW cId \in CallIds, ConsumeHeader(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF ConsumeHeader, L1!Next

LEMMA ProjectsBeginDisposeCall ==
    ASSUME NEW cId \in CallIds, BeginDisposeCall(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF BeginDisposeCall, L1!Next

LEMMA ProjectsDisposeCallForChannel ==
    \* Delegates to BeginDisposeCall, so it projects through it.
    ASSUME NEW cId \in CallIds, DisposeCallForChannel(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF DisposeCallForChannel, BeginDisposeCall, L1!Next

LEMMA ProjectsDrainRelease ==
    ASSUME NEW cId \in CallIds, DrainRelease(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF DrainRelease, L1!Next

LEMMA ProjectsFinishDisposeCall ==
    ASSUME NEW cId \in CallIds, FinishDisposeCall(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeCall

LEMMA ProjectsSettleCall ==
    ASSUME NEW cId \in CallIds, SettleCall(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF SettleCall

LEMMA ProjectsCancelWriterWait ==
    ASSUME NEW cId \in CallIds, CancelWriterWait(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CancelWriterWait

LEMMA ProjectsWriteDoneCompletes ==
    ASSUME NEW cId \in CallIds, WriteDoneCompletes(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF WriteDoneCompletes, L1!Next

LEMMA ProjectsCloseWriter ==
    ASSUME NEW cId \in CallIds, CloseWriter(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CloseWriter, L1!Next

LEMMA ProjectsOnEventReturns ==
    ASSUME NEW cId \in CallIds, OnEventReturns(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF OnEventReturns, L1!Next

LEMMA ProjectsTerminalCallbackReturns ==
    ASSUME NEW cId \in CallIds, TerminalCallbackReturns(cId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF TerminalCallbackReturns, L1!Next

LEMMA ProjectsStartCall ==
    ASSUME NEW cId \in CallIds, NEW chId \in ChannelIds, StartCall(cId, chId)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF StartCall, L1!Next

LEMMA ProjectsWriteLendSucceeds ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW len \in L1!Sizes, NEW charge \in L1!Sizes, WriteLendSucceeds(cId, b, len, charge)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF WriteLendSucceeds, L1!Next

LEMMA ProjectsWriteRefusedBudget ==
    ASSUME NEW cId \in CallIds, NEW len \in L1!Sizes, NEW charge \in L1!CandidateCharges, WriteRefusedBudget(cId, len, charge)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF WriteRefusedBudget, L1!Next

LEMMA ProjectsWriteRefusedTooLarge ==
    ASSUME NEW cId \in CallIds, NEW len \in L1!RequestLengths, WriteRefusedTooLarge(cId, len)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF WriteRefusedTooLarge, L1!Next

LEMMA ProjectsRetryLendSucceeds ==
    \* The witness for level 1's length is retry_len[cId], which is a real
    \* length rather than the sentinel exactly while the write is waiting -
    \* which is RetryLenMatchesWait, not a syntactic fact.
    ASSUME ManagedTypeOK, RetryLenMatchesWait,
           NEW cId \in CallIds, NEW b \in BufferIds, NEW charge \in L1!Sizes,
           RetryLendSucceeds(cId, b, charge)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    \* The sentinel lies outside every length level 1 quantifies over -
    \* NoRetryLen = Ceiling + 2 > Ceiling + 1 - which is arithmetic on a
    \* positive Ceiling, not a definition unfold, so the assumption and the
    \* solver both appear here.
    <1>1. NoRetryLen \notin L1!Sizes
        BY L1!CeilingIsPositive, SMT DEF NoRetryLen, L1!Sizes
    <1>2. retry_len[cId] \in L1!Sizes
        BY <1>1 DEF ManagedTypeOK, RetryLenMatchesWait, RetryLendSucceeds
    <1>3. QED
        BY <1>1, <1>2
        DEF ManagedTypeOK, RetryLenMatchesWait, RetryLendSucceeds, L1!Next

LEMMA ProjectsCommitWrite ==
    ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW b \in BufferIds, CommitWrite(cId, msg, b)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF CommitWrite, L1!Next

LEMMA ProjectsWriteAborted ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, WriteAborted(cId, b)
    PROVE  L1!Next \/ UNCHANGED l1_vars
    BY DEF WriteAborted, L1!Next

THEOREM RefinesNext == ManagedSafety /\ [Next]_vars => [L1!Next]_l1_vars
    \* One lemma per disjunct, so this is the case analysis and nothing
    \* else.  The stuttering side is inherited: vars holds l1_vars, so
    \* nothing changing means level 1's tuple did not change either.
    <1>1. ASSUME ManagedSafety, Next PROVE L1!Next \/ UNCHANGED l1_vars
        BY <1>1, ProjectsFreeRuntimeRoot, ProjectsCreateRuntime,
           ProjectsAcquireLease, ProjectsCreateChannel,
           ProjectsRejectChannelCreation, ProjectsBeginDisposeChannel,
           ProjectsFinishDisposeChannel, ProjectsResolveChannelDispose,
           ProjectsBeginRuntimeShutdown, ProjectsFinishDisposeRuntime,
           ProjectsShutdownReturns, ProjectsResourcesReleasedReturns,
           ProjectsBeginMoveNext, ProjectsBeginParseEvent,
           ProjectsFinishConsumePayload, ProjectsCancelWaiter,
           ProjectsRequestReadCancellation, ProjectsCancelWaitingRead,
           ProjectsCancelParsingRead, ProjectsFinishCancelledParse,
           ProjectsHandoffToDrain, ProjectsConsumeHeader,
           ProjectsBeginDisposeCall, ProjectsDisposeCallForChannel,
           ProjectsDrainRelease, ProjectsFinishDisposeCall,
           ProjectsSettleCall, ProjectsCancelWriterWait,
           ProjectsWriteDoneCompletes, ProjectsCloseWriter,
           ProjectsOnEventReturns, ProjectsTerminalCallbackReturns,
           ProjectsStartCall, ProjectsWriteLendSucceeds,
           ProjectsWriteRefusedBudget, ProjectsWriteRefusedTooLarge,
           ProjectsRetryLendSucceeds, ProjectsCommitWrite,
           ProjectsWriteAborted
           DEF Next, Passthrough, RuntimeSteps, BindingDowncalls,
               ManagedStutter, L1!Next, ManagedSafety
    <1>2. ASSUME UNCHANGED vars PROVE UNCHANGED l1_vars
        BY <1>2 DEF vars
    <1>3. QED
        BY <1>1, <1>2

(***************************************************************************)
(* THE INDUCTIVE CORE IMPLIES THE SAFETY CONTRACT                          *)
(*                                                                         *)
(* The derivation is the point: every conjunct proved here leaves the      *)
(* induction for good.  The bridge predicates are named because the        *)
(* content is arithmetic - a solver sees an equality of differences where  *)
(* an unfold sees two unrelated operators.                                 *)
(***************************************************************************)

\* The ring's occupancy IS level 1's owed payload count: same difference,
\* two names, one on each side of the instance.
LEMMA RingOccupancyIsOwedPayloads ==
    \A cId \in CallIds : RingOccupancy(cId) = L1!OwedPayloads(cId)
    BY DEF RingOccupancy, RingHead, RingTail, L1!OwedPayloads

\* L1!IndInv bounds the owed count by the credits plus the terminal.
LEMMA CoreBoundsTheRing ==
    ManagedIndInv => RingNeverOverflows
    <1>1. ASSUME ManagedIndInv
          PROVE  \A cId \in CallIds :
                     L1!OwedPayloads(cId) <= DeliveryCredits + 1
        BY <1>1 DEF ManagedIndInv, L1!IndInv, L1!FfiCallInv,
                    L1!PayloadsOwnedWithinCreditsPlusOne,
                    L1!HostOwnsAtMostCreditsPlusOne
    <1>2. QED
        \* SMT: the goal is a bound carried across an equality of integer
        \* differences, which Zenon does not do.
        BY <1>1, RingOccupancyIsOwedPayloads, SMT DEF RingNeverOverflows

THEOREM ManagedIndInvImpliesSafety == ManagedIndInv => ManagedSafety
    \* Projection for the carried conjuncts, derivation for the rest.
    BY CoreBoundsTheRing
    DEF ManagedIndInv, ManagedMachineInv, ReaderInv, WriterInv,
        LifecycleInv, ManagedSafety



(***************************************************************************)
(* PRESERVATION OF THE MANAGED LAYER, ACTION BY ACTION                     *)
(*                                                                         *)
(* One lemma per disjunct of Next, validated slice by slice.  The recipe,   *)
(* measured to closure: the USE unfolds three tuple levels - the level-0    *)
(* one hides behind L1!L0!vars - plus level 0's GROUP tuples, whose        *)
(* UNCHANGED would otherwise hide the frame; every machine conjunct and     *)
(* helper is unfolded once in the USE so cross-conjunct facts are visible;  *)
(* plain Zenon carries the frame, SMT the steps whose conjunct the action   *)
(* writes - those also unfold L1!IndInv, L1!TypeOK and L1!L0!TypeOK for    *)
(* the level-1 state's typing - and no step uses Isabelle.  Where a goal    *)
(* mixes an IF, an EXCEPT and a 14-conjunct typing, it is decomposed:      *)
(* first the IF's value, then the updated function, then the rest.         *)
(***************************************************************************)

LEMMA KeepsFreeRuntimeRoot ==
    ASSUME FreeRuntimeRoot, ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FreeRuntimeRoot
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY SMT DEF LiveChannelUsesCurrentRuntime, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY SMT DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, TeardownLeavesCallsSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCreateRuntime ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds, CreateRuntime(rtId, chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CreateRuntime, L1!RuntimeCreate, L1!L0!RuntimeCreate
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY SMT DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY SMT DEF LiveChannelUsesCurrentRuntime, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY SMT DEF RejectedChannelHasNoNativeHalf, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>17. ChannelStateMatchesNative'
        BY SMT DEF ChannelStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>18. RuntimeStateMatchesNative'
        BY SMT DEF RuntimeStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, NotInitRuntimeIsUndestroyed
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsAcquireLease ==
    ASSUME NEW chId \in ChannelIds, AcquireLease(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           AcquireLease
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCreateChannel ==
    ASSUME NEW chId \in ChannelIds, CreateChannel(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CreateChannel, L1!ChannelCreate, L1!L0!ChannelCreate
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY SMT DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY SMT DEF LiveChannelUsesCurrentRuntime, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY SMT DEF RejectedChannelHasNoNativeHalf, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>17. ChannelStateMatchesNative'
        BY SMT DEF ChannelStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>18. RuntimeStateMatchesNative'
        BY SMT DEF RuntimeStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsRejectChannelCreation ==
    ASSUME NEW chId \in ChannelIds, RejectChannelCreation(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           RejectChannelCreation
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY SMT DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY SMT DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY SMT DEF RejectedChannelHasNoNativeHalf, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY SMT DEF RuntimeStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsBeginDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, BeginDisposeChannel(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           BeginDisposeChannel
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsFinishDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, FinishDisposeChannel(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FinishDisposeChannel, L1!ChannelStartClosing,
           L1!L0!ChannelStartClosing
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY SMT DEF LiveChannelKeepsRuntimeAlive, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, IsLastRelease
    <1>15. NoRuntimeShutdownWhileLeased'
        BY SMT DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, IsLastRelease
    <1>16. RejectedChannelHasNoNativeHalf'
        BY SMT DEF RejectedChannelHasNoNativeHalf, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>17. ChannelStateMatchesNative'
        BY SMT DEF ChannelStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsResolveChannelDispose ==
    ASSUME NEW chId \in ChannelIds, ResolveChannelDispose(chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           ResolveChannelDispose
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY SMT DEF ChannelStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, ChannelDisposeMayResolve, IsLastRelease
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY SMT DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, ChannelDisposeMayResolve, IsLastRelease
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsBeginRuntimeShutdown ==
    ASSUME NEW rtId \in RuntimeIds, BeginRuntimeShutdown(rtId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           BeginRuntimeShutdown, L1!RuntimeBeginShutdown, L1!L0!ChannelsOf,
           L1!L0!RuntimeBeginShutdown
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsFinishDisposeRuntime ==
    ASSUME NEW rtId \in RuntimeIds, FinishDisposeRuntime(rtId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FinishDisposeRuntime, L1!RuntimeDestroy
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY NoneNotInRuntimeIds, SMT DEF DisposeAwaitsDestroy, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!IsRuntimeQuiescent, L1!IsReleasedRuntime, L1!IsShutdownCallbackRunning, L1!IsResourcesReleasedCallbackRunning, L1!SecondEventOwed, L1!IsResourcesReleasedEmitted, L1!IsRuntimeDestroyed
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY SMT DEF RuntimeStateMatchesNative, L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsShutdownReturns ==
    ASSUME NEW rtId \in RuntimeIds, ShutdownReturns(rtId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           ShutdownReturns, L1!ShutdownCallbackReturns
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY SMT DEF RuntimeRootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!IsShutdownCallbackRunning
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsResourcesReleasedReturns ==
    ASSUME NEW rtId \in RuntimeIds, ResourcesReleasedReturns(rtId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           ResourcesReleasedReturns, L1!ResourcesReleasedCallbackReturns
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY SMT DEF RuntimeRootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!IsResourcesReleasedCallbackRunning
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsBeginMoveNext ==
    ASSUME NEW cId \in CallIds, BeginMoveNext(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           BeginMoveNext
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsBeginParseEvent ==
    ASSUME NEW cId \in CallIds, BeginParseEvent(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           BeginParseEvent
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsFinishConsumePayload ==
    ASSUME NEW cId \in CallIds, FinishConsumePayload(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FinishConsumePayload, L1!HostConsumesEvent
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ReadCancellationSettled
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ReadCancellationSettled
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsCancelWaiter ==
    ASSUME NEW cId \in CallIds, CancelWaiter(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CancelWaiter
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsRequestReadCancellation ==
    ASSUME NEW cId \in CallIds, RequestReadCancellation(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           RequestReadCancellation
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCancelWaitingRead ==
    ASSUME NEW cId \in CallIds, CancelWaitingRead(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CancelWaitingRead, L1!RequestCallCancellation,
           L1!L0!IsUnusedCall
    <1>t. ManagedTypeOK'
      <2>v. (IF headers_completion[cId] = "pending" THEN "failed"
             ELSE headers_completion[cId]) \in HeadersCompletions
          \* the IF decomposed first: each branch lands in the set, one by
          \* name, the other by the pre-state typing - the EXCEPT is then
          \* an ordinary update
          BY SMT DEF HeadersCompletions
      <2>h. headers_completion' \in [CallIds -> HeadersCompletions]
          BY <2>v, SMT
      <2>q. QED
          BY <2>h DEF ReaderStates, WriterStates, ConsumerPhases,
             CallDisposeStates, ChannelDisposeStates, RuntimeDisposeStates,
             HeadersCompletions, StatusCompletions, NoRetryLen, L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCancelParsingRead ==
    ASSUME NEW cId \in CallIds, CancelParsingRead(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CancelParsingRead, L1!RequestCallCancellation,
           L1!L0!IsUnusedCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsFinishCancelledParse ==
    ASSUME NEW cId \in CallIds, FinishCancelledParse(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FinishCancelledParse, L1!HostConsumesEvent
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY SMT DEF ReadCancelPendingOnlyInFlight, ReadInFlight, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal, CancelledParseHasNoPendingRequest
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsHandoffToDrain ==
    ASSUME NEW cId \in CallIds, HandoffToDrain(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           HandoffToDrain
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsConsumeHeader ==
    ASSUME NEW cId \in CallIds, ConsumeHeader(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           ConsumeHeader, L1!HostConsumesEvent
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal, PrologueReaderOnlyWaits
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsBeginDisposeCall ==
    ASSUME NEW cId \in CallIds, BeginDisposeCall(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           BeginDisposeCall, L1!RequestCallCancellation,
           L1!L0!IsUnusedCall, L1!L0!IsActiveCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsDisposeCallForChannel ==
    ASSUME NEW cId \in CallIds, DisposeCallForChannel(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           DisposeCallForChannel, BeginDisposeCall,
           L1!RequestCallCancellation, L1!L0!IsUnusedCall,
           L1!L0!IsActiveCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsDrainRelease ==
    ASSUME NEW cId \in CallIds, DrainRelease(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           DrainRelease, L1!HostConsumesEvent
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY SMT DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!HostConsumesEvent, ConsumingTerminal
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsFinishDisposeCall ==
    ASSUME NEW cId \in CallIds, FinishDisposeCall(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           FinishDisposeCall, L1!L0!HasStatus
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY SMT DEF DisposeLeavesNoManagedWaiter, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, PastPrologueHeadersAnswered, PrologueReaderOnlyWaits
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, RingDrained
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsSettleCall ==
    ASSUME NEW cId \in CallIds, SettleCall(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           SettleCall, L1!HostHoldsNoBuffer, L1!HostOwnsNoPayload,
           L1!L0!IsTerminalCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY SMT DEF DisposeLeavesNoManagedWaiter, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, PastPrologueHeadersAnswered, PrologueReaderOnlyWaits
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCancelWriterWait ==
    ASSUME NEW cId \in CallIds, CancelWriterWait(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CancelWriterWait, L1!L0!IsActiveCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsWriteDoneCompletes ==
    ASSUME NEW cId \in CallIds, WriteDoneCompletes(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           WriteDoneCompletes, L1!WriteDoneReturns
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY SMT DEF RootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!WriteDoneReturns, L1!IsWriteDoneCallbackRunning
    <1>9. RuntimeRootSurvivesCallbacks'
        BY SMT DEF RuntimeRootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!WriteDoneReturns, L1!IsWriteDoneCallbackRunning
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCloseWriter ==
    ASSUME NEW cId \in CallIds, CloseWriter(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CloseWriter, L1!EndSend, L1!L0!EndSend
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY SMT DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!EndSend
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsOnEventReturns ==
    ASSUME NEW cId \in CallIds, OnEventReturns(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           OnEventReturns, L1!DeliveryCallbackReturns, L1!L0!HasStatus
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY SMT DEF RootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!DeliveryCallbackReturns, L1!IsDeliveryCallbackRunning
    <1>9. RuntimeRootSurvivesCallbacks'
        BY SMT DEF RuntimeRootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!DeliveryCallbackReturns, L1!IsDeliveryCallbackRunning
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsTerminalCallbackReturns ==
    ASSUME NEW cId \in CallIds, TerminalCallbackReturns(cId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           TerminalCallbackReturns, L1!DeliveryCallbackReturns,
           L1!L0!HasStatus
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
      <2>w. ~write_done_callback_running[cId]
          \* the one fact the root release rests on, from whichever of
          \* level 1's clean-call conjuncts covers the call's state
          BY SMT DEF StatusMeansTerminal,
             L1!L0!SafetyCore, L1!L0!IndInv, L1!IndInv, L1!FfiCallInv,
             L1!TerminalCallHasNoSendInFlight, L1!HasNoSendInFlight,
             L1!ReleasedCallIsClean, L1!UnusedCallsAreFfiClean,
             L1!IsWriteDoneCallbackRunning, L1!IsHandleReleased,
             L1!L0!HasStatus, L1!L0!IsTerminalCall, L1!L0!IsActiveCall,
             L1!L0!ActiveCallStates, L1!L0!IsUnusedCall, L1!TypeOK,
             L1!L0!TypeOK
      <2>q. QED
          BY <2>w, SMT DEF RootSurvivesCallbacks,
             L1!DeliveryCallbackReturns, L1!IsDeliveryCallbackRunning,
             L1!IndInv, L1!TypeOK, L1!L0!TypeOK
    <1>9. RuntimeRootSurvivesCallbacks'
        BY SMT DEF RuntimeRootSurvivesCallbacks, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!DeliveryCallbackReturns, L1!IsDeliveryCallbackRunning
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsStartCall ==
    ASSUME NEW cId \in CallIds, NEW chId \in ChannelIds, StartCall(cId, chId), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           StartCall, L1!CallStart, L1!L0!CallStart
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen
    <1>7. TokenPublishedBeforeStart'
        BY SMT DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!CallStart, L1!L0!CallStart
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsWriteLendSucceeds ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW len \in L1!Sizes, NEW charge \in L1!Sizes, WriteLendSucceeds(cId, b, len, charge), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           WriteLendSucceeds, L1!LendSendBuffer, L1!L0!IsActiveCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!LendSendBuffer, L1!IsLendable, L1!LendStatuses
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY SMT DEF DisposeLeavesNoManagedWaiter, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!LendSendBuffer, BindingMayDowncall
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!LendSendBuffer, BindingMayDowncall
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsWriteRefusedBudget ==
    ASSUME NEW cId \in CallIds, NEW len \in L1!Sizes, NEW charge \in L1!CandidateCharges, WriteRefusedBudget(cId, len, charge), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           WriteRefusedBudget, L1!RefuseLendForBudget
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY CeilingIsPositive, SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!RefuseLendForBudget, BindingMayDowncall, L1!LendStatuses, L1!IsLendable, NoRetryLen, L1!Sizes
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY SMT DEF DisposeLeavesNoManagedWaiter, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, L1!RefuseLendForBudget, BindingMayDowncall
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

LEMMA KeepsWriteRefusedTooLarge ==
    ASSUME NEW cId \in CallIds, NEW len \in L1!RequestLengths, WriteRefusedTooLarge(cId, len), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           WriteRefusedTooLarge, L1!RefuseLendTooLarge
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!RefuseLendTooLarge, L1!LendStatuses
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsRetryLendSucceeds ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW charge \in L1!Sizes, RetryLendSucceeds(cId, b, charge), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           RetryLendSucceeds, L1!LendSendBuffer, L1!L0!IsActiveCall
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!LendSendBuffer, L1!IsLendable, L1!LendStatuses
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!LendSendBuffer
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsCommitWrite ==
    ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW b \in BufferIds, CommitWrite(cId, msg, b), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           CommitWrite, L1!SendMessage, L1!L0!SendMessage
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!SendMessage, L1!IsReturnedBuffer, L1!IsLentBuffer, L1!LendStatuses
    <1>7. TokenPublishedBeforeStart'
        BY SMT DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!SendMessage, L1!L0!SendMessage
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!SendMessage
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21
LEMMA KeepsWriteAborted ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, WriteAborted(cId, b), ManagedIndInv
    PROVE  (ManagedTypeOK /\ ManagedMachineInv)'
    <1> USE DEF ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, LifecycleInv, ManagedGlue,
           ConsumerPhaseMatchesDispose, AtMostOneReaderOutstanding,
           DrainNeverOverlapsApplicationConsumer, ReadCancelPendingOnlyInFlight,
           ParsingReadOwnsItsSlot, WriterInv, WaitingWriterHoldsNoBuffer,
           SerializingWriterHoldsTheBuffer, WaitMatchesRefusal,
           ManagedWriterNeverObservesSlotBusy, RetryLenMatchesWait,
           TokenPublishedBeforeStart, RootSurvivesCallbacks,
           RuntimeRootSurvivesCallbacks, DisposeAwaitsDestroy,
           RuntimeManagerCoherent, LiveChannelUsesCurrentRuntime,
           ManagedShutdownHasNoHostDebt, LiveChannelKeepsRuntimeAlive,
           NoRuntimeShutdownWhileLeased, RejectedChannelHasNoNativeHalf,
           ChannelStateMatchesNative, RuntimeStateMatchesNative,
           DisposeLeavesNoManagedWaiter, SettledCallOwesNothing,
           AbsentRuntimeOwesNothing, ReadInFlight, RingOccupancy, RingHead,
           RingTail, RingDrained, ChannelSettled, AllLeasesReleased,
           ConsumingTerminal, NoRetryLen, L1!HostOwnsNoPayload,
           L1!HostHoldsNoBuffer, L1!OwedPayloads, L1!IsLentBuffer,
           L1!IsReturnedBuffer, L1!L0!IsUnusedCall,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           l1_vars, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           ManagedStutter, managed_vars,
           WriteAborted, L1!HostReturnsBuffer
    <1>t. ManagedTypeOK'
        BY DEF ReaderStates, WriterStates, ConsumerPhases, CallDisposeStates,
           ChannelDisposeStates, RuntimeDisposeStates,
           HeadersCompletions, StatusCompletions, NoRetryLen,
           L1!Sizes
    <1>1. ConsumerPhaseMatchesDispose'
        BY DEF ConsumerPhaseMatchesDispose
    <1>2. AtMostOneReaderOutstanding'
        BY DEF AtMostOneReaderOutstanding, ReadInFlight
    <1>3. DrainNeverOverlapsApplicationConsumer'
        BY DEF DrainNeverOverlapsApplicationConsumer
    <1>4. ReadCancelPendingOnlyInFlight'
        BY DEF ReadCancelPendingOnlyInFlight, ReadInFlight
    <1>5. ParsingReadOwnsItsSlot'
        BY DEF ParsingReadOwnsItsSlot, RingOccupancy, RingHead, RingTail
    <1>6. WriterInv'
        BY SMT DEF WriterInv, WaitingWriterHoldsNoBuffer, SerializingWriterHoldsTheBuffer,
             WaitMatchesRefusal, ManagedWriterNeverObservesSlotBusy,
             RetryLenMatchesWait, L1!HostHoldsNoBuffer, L1!IsLentBuffer,
             L1!IsReturnedBuffer, NoRetryLen, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!HostReturnsBuffer, L1!IsLentBuffer, L1!IsReturnedBuffer, L1!LendStatuses
    <1>7. TokenPublishedBeforeStart'
        BY DEF TokenPublishedBeforeStart, L1!L0!IsUnusedCall
    <1>8. RootSurvivesCallbacks'
        BY DEF RootSurvivesCallbacks
    <1>9. RuntimeRootSurvivesCallbacks'
        BY DEF RuntimeRootSurvivesCallbacks
    <1>10. DisposeAwaitsDestroy'
        BY DEF DisposeAwaitsDestroy
    <1>11. RuntimeManagerCoherent'
        BY DEF RuntimeManagerCoherent
    <1>12. LiveChannelUsesCurrentRuntime'
        BY DEF LiveChannelUsesCurrentRuntime
    <1>13. ManagedShutdownHasNoHostDebt'
        BY DEF ManagedShutdownHasNoHostDebt, L1!HostOwnsNoPayload,
             L1!HostHoldsNoBuffer, L1!OwedPayloads
    <1>14. LiveChannelKeepsRuntimeAlive'
        BY DEF LiveChannelKeepsRuntimeAlive
    <1>15. NoRuntimeShutdownWhileLeased'
        BY DEF NoRuntimeShutdownWhileLeased, AllLeasesReleased, ChannelSettled
    <1>16. RejectedChannelHasNoNativeHalf'
        BY DEF RejectedChannelHasNoNativeHalf
    <1>17. ChannelStateMatchesNative'
        BY DEF ChannelStateMatchesNative
    <1>18. RuntimeStateMatchesNative'
        BY DEF RuntimeStateMatchesNative
    <1>19. DisposeLeavesNoManagedWaiter'
        BY DEF DisposeLeavesNoManagedWaiter
    <1>20. SettledCallOwesNothing'
        BY SMT DEF SettledCallOwesNothing, L1!HostOwnsNoPayload, L1!HostHoldsNoBuffer,
             L1!OwedPayloads, RingDrained, RingHead, RingTail, L1!IndInv, L1!TypeOK, L1!L0!TypeOK, BindingMayDowncall, L1!HostReturnsBuffer
    <1>21. AbsentRuntimeOwesNothing'
        BY DEF AbsentRuntimeOwesNothing, AllLeasesReleased, ChannelSettled
    <1>q. QED
        BY <1>t, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19, <1>20, <1>21

(***************************************************************************)
(* LEVEL 1'S OWN INDUCTIVE STEP, CITED                                     *)
(*                                                                         *)
(* Level 1 proved this; level 2 uses it.  The prefix counts INSTANCE hops   *)
(* and nothing else: level 1's theorems state their hypotheses with the     *)
(* IsFiniteSet that reached FfiGrpcState by EXTENDS, and EXTENDS prefixes   *)
(* nothing, so one INSTANCE hop later they read L1!IsFiniteSet.             *)
(* L1!L0!IsFiniteSet is a different symbol - the copy FfiGrpc made for its  *)
(* own use, two hops away - well formed, same meaning, and it does not      *)
(* match.  Getting that wrong costs a Zenon timeout rather than a clean     *)
(* refusal, because both symbols are opaque one-argument operators.         *)
(***************************************************************************)

LEMMA L1Assumptions ==
    /\ "none" \notin RuntimeIds /\ "none" \notin ChannelIds
    /\ "none" \notin CallIds
    /\ L1!IsFiniteSet(CallIds) /\ L1!IsFiniteSet(ChannelIds)
    /\ L1!IsFiniteSet(RuntimeIds)
    /\ MaxSendsInFlight \in Nat \ {0} /\ DeliveryCredits \in Nat \ {0}
    /\ Ceiling \in Nat \ {0} /\ MessageLength \in [Messages -> Nat]
    /\ L1!IsFiniteSet(BufferIds) /\ BufferIds # {}
    BY NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds,
       FiniteCallIds, FiniteChannelIds, FiniteRuntimeIds,
       MaxSendsInFlightIsPositive, DeliveryCreditsArePositive,
       CeilingIsPositive, MessageLengthIsNat,
       BufferIdsAreAFiniteNonemptySet, Zenon
    DEF L1!IsFiniteSet, IsFiniteSet

LEMMA L1StepPreservesL1Inv ==
    L1!IndInv /\ [L1!Next]_(L1!vars) => L1!IndInv'
    BY L1Assumptions, L1!IndInvPreserved, Zenon

LEMMA L1InitEstablishesL1Inv == L1!Init => L1!IndInv
    BY L1Assumptions, L1!InitEstablishesIndInv, Zenon

===============================================================================
