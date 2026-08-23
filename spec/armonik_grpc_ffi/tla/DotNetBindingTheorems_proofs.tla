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

THEOREM RefinesInit == Init => F!Init
    BY DEF Init

LEMMA FairnessRefines == Fairness => F!Fairness
    \* One step per family rather than one goal for all nineteen.  The
    \* monolithic form is what Isabelle cannot close: expanding an
    \* instantiated Fairness through the substitution builds a single
    \* enormous goal, and under several threads it starves its
    \* neighbours besides, so a run reports failures that a single-
    \* threaded one does not.  Each step here is a projection of a
    \* conjunction; only the QED assembles them, with every conjunct
    \* already a fact.
    <1>1. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!NetworkSend(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>2. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!ReceiveStatus(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>3. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!DeliverInitialMetadata(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>4. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!DeliverMessage(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>5. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!DeliverStatus(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>6. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!DeliverCancelled(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>7. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!EmitWriteDone(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>8. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(F!RuntimeRelease(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>9. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(F!EmitShutdownComplete(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>10. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(F!EmitResourcesReleased(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>11. Fairness => \A chId \in ChannelIds :
              WF_l1_vars(F!ChannelFinishClosing(chId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>12. Fairness => \A cId \in CallIds, b \in BufferIds :
              WF_l1_vars(F!FreeReturnedBuffer(cId, b))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>13. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!ReleaseCallHandle(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>14. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!DeliveryCallbackReturns(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>15. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!WriteDoneReturns(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>16. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(F!ShutdownCallbackReturns(rtId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>17. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(F!ResourcesReleasedCallbackReturns(rtId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>18. Fairness => \A cId \in CallIds :
              WF_l1_vars(F!HostConsumesEvent(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>19. Fairness => \A cId \in CallIds, b \in BufferIds :
              WF_l1_vars(F!HostReturnsBuffer(cId, b))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>20. QED
        BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19 DEF F!Fairness, l1_vars

(***************************************************************************)
(* DISCHARGE OF THE SIX HOST HYPOTHESES                                    *)
(*                                                                         *)
(* Each is a conjunct of BindingOwedFairness, written in level 1's own      *)
(* tuple for exactly this reason: the discharge is a citation rather than   *)
(* an enabling argument.  Isabelle, because the conclusion is a WF_ atom    *)
(* and Zenon cannot read one at all.                                       *)
(***************************************************************************)

THEOREM DeliveryCallbackReturnsDischarged ==
    Spec => \A cId \in CallIds :
                WF_l1_vars(F!DeliveryCallbackReturns(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM WriteDoneReturnsDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(F!WriteDoneReturns(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM ShutdownCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(F!ShutdownCallbackReturns(rtId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM ResourcesReleasedCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(F!ResourcesReleasedCallbackReturns(rtId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM HostConsumesEventDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(F!HostConsumesEvent(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM HostReturnsBufferDischarged ==
    Spec => \A cId \in CallIds, b \in BufferIds :
                WF_l1_vars(F!HostReturnsBuffer(cId, b))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

===============================================================================
