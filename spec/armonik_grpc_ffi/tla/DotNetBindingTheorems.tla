------------------------- MODULE DotNetBindingTheorems -------------------------
(***************************************************************************)
(* The public level-2 interface: what the .NET binding proves, stated      *)
(* without proofs.  The proofs will live in DotNetBindingTheorems_proofs,  *)
(* which does not exist yet - every declaration below is an obligation the *)
(* freeze of this level requires discharged, and none is discharged today. *)
(***************************************************************************)

EXTENDS DotNetBinding_defs

(***************************************************************************)
(* REFINEMENT - established the way level 1 established Spec => L0!Spec.   *)
(* Everything proved at levels 0 and 1 is inherited through it, and        *)
(* L0!Spec follows from level 1's RefinesSpec by transitivity.             *)
(***************************************************************************)

THEOREM RefinesInit == Init => F!Init

THEOREM RefinesNext == [Next]_vars => [F!Next]_l1_vars

THEOREM RefinesSpec == Spec => F!Spec

(***************************************************************************)
(* DISCHARGE OF THE SIX HOST HYPOTHESES - the point of the level.  Level 1 *)
(* imposed these six weak-fairness conjuncts on its host and could not     *)
(* enforce them; the binding's own fairness implies each.  The two that    *)
(* involve user code - parsing and serialization - hold under the stated   *)
(* hypothesis that user code terminates: the wrapper covers success and    *)
(* exception, nothing covers a call that never returns.                    *)
(***************************************************************************)

THEOREM DeliveryCallbackReturnsDischarged ==
    Spec => \A cId \in CallIds :
                WF_l1_vars(F!DeliveryCallbackReturns(cId))

THEOREM WriteDoneReturnsDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(F!WriteDoneReturns(cId))

THEOREM ShutdownCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(F!ShutdownCallbackReturns(rtId))

THEOREM ResourcesReleasedCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(F!ResourcesReleasedCallbackReturns(rtId))

THEOREM HostConsumesEventDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(F!HostConsumesEvent(cId))

THEOREM HostReturnsBufferDischarged ==
    Spec => \A cId \in CallIds, b \in BufferIds :
                WF_l1_vars(F!HostReturnsBuffer(cId, b))

(***************************************************************************)
(* MANAGED SAFETY                                                          *)
(***************************************************************************)

THEOREM ManagedTypeOKHolds == Spec => []ManagedTypeOK

THEOREM ManagedSafetyHolds == Spec => []ManagedSafety

(***************************************************************************)
(* MANAGED LIVENESS - one theorem per public promise, aggregated last.     *)
(***************************************************************************)

THEOREM BudgetCancellationStopsRetryHolds ==
    Spec => BudgetCancellationStopsRetry

THEOREM CallDisposeCompletesHolds == Spec => CallDisposeCompletes

THEOREM RuntimeDisposeCompletesHolds == Spec => RuntimeDisposeCompletes

THEOREM CallRootEventuallyFreedHolds == Spec => CallRootEventuallyFreed

THEOREM RuntimeRootEventuallyFreedHolds ==
    Spec => RuntimeRootEventuallyFreed

THEOREM InFlightPayloadEventuallyReleasedHolds ==
    Spec => InFlightPayloadEventuallyReleased

THEOREM QueuedContinuationEventuallyRunsHolds ==
    Spec => QueuedContinuationEventuallyRuns

THEOREM ManagedLivenessTheorem == Spec => ManagedLiveness

===============================================================================
