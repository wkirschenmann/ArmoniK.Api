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
(* exception, nothing covers code that never comes back.                   *)
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

\* The hand-off moves no level-1 state: in particular the release counter
\* is untouched, so the drain resumes exactly where the application
\* stopped.  Structural - HandoffToDrain stutters on l1_vars - and stated
\* so nobody reads CallRootEventuallyFreed as carrying it.
THEOREM ConsumerHandoffPreservesTail ==
    Spec => \A cId \in CallIds :
                [][HandoffToDrain(cId) =>
                       payloads_consumed_by_host' =
                           payloads_consumed_by_host]_vars

\* The last releaser's public task never completes early: a channel
\* resolves its DisposeAsync only when another lease is still out - so it
\* was not the last - or the generation it released has finished tearing
\* down.  An action theorem, not an invariant: a later channel taking a
\* lease and releasing it changes no earlier resolution, so no state
\* predicate can carry this.
THEOREM LastChannelDisposeAwaitsDestroy ==
    Spec => \A chId \in ChannelIds :
                [][ResolveChannelDispose(chId) =>
                       \/ ~AllLeasesReleased
                       \/ runtime_dispose_state \in
                              {"destroyed", "absent"}]_vars

\* A channel's dispose settles its own calls and no one else's: a step
\* that disposes a call of one channel leaves every call of every other
\* channel untouched.  An action theorem, not an invariant - ownership is
\* what the step reads, and no state records which dispose caused which
\* change.  Stated over the calls NOT owned, because the guard already
\* names the owned ones and an implication about them would be a
\* tautology rather than a check.
THEOREM ChannelDisposeAffectsOnlyOwnedCalls ==
    Spec => \A chId \in ChannelIds :
                [][(\E cId \in CallIds :
                        /\ DisposeCallForChannel(cId)
                        /\ call_channel[cId] = chId)
                       => \A other \in CallIds :
                              call_channel[other] # chId =>
                                  call_dispose_state'[other]
                                      = call_dispose_state[other]]_vars

(***************************************************************************)
(* MANAGED LIVENESS - one theorem per public promise, aggregated last.     *)
(***************************************************************************)

THEOREM BudgetCancellationStopsRetryHolds ==
    Spec => BudgetCancellationStopsRetry

THEOREM PendingWriteEventuallySettledHolds ==
    Spec => PendingWriteEventuallySettled

THEOREM CallDisposeCompletesHolds == Spec => CallDisposeCompletes

THEOREM ChannelConstructionCompletesHolds ==
    Spec => ChannelConstructionCompletes

THEOREM ChannelLeaseEventuallyReleasedHolds ==
    Spec => ChannelLeaseEventuallyReleased

THEOREM ChannelDisposeCompletesHolds == Spec => ChannelDisposeCompletes

THEOREM RuntimeDisposeCompletesHolds == Spec => RuntimeDisposeCompletes

THEOREM CallRootEventuallyFreedHolds == Spec => CallRootEventuallyFreed

THEOREM RuntimeRootEventuallyFreedHolds ==
    Spec => RuntimeRootEventuallyFreed

THEOREM InFlightPayloadEventuallyReleasedHolds ==
    Spec => InFlightPayloadEventuallyReleased

THEOREM WaitingReaderEventuallyResolvedHolds ==
    Spec => WaitingReaderEventuallyResolved

THEOREM PublishedCallEventuallyDisposedHolds ==
    Spec => PublishedCallEventuallyDisposed

THEOREM HeadersEventuallyResolvedHolds == Spec => HeadersEventuallyResolved

THEOREM StatusEventuallyResolvedHolds == Spec => StatusEventuallyResolved

THEOREM ManagedLivenessTheorem == Spec => ManagedLiveness

===============================================================================
