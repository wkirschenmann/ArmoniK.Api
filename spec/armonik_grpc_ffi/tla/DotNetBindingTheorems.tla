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

\* The last releaser's public task never completes early.  Which channel
\* emptied the lease set is latched at its release, so the claim is about
\* that channel and not about whatever the set holds later: a channel
\* that was the last resolves its DisposeAsync only once the generation
\* IT released was destroyed.  An action theorem, the fact being about a
\* step rather than about a state.
\* MoveNext's token cancels the CALL, never the read alone: the only
\* action that resolves a read on its token also requests the call's
\* cancellation, in the same step.
THEOREM ReadCancellationCancelsCall ==
    Spec => \A cId \in CallIds :
                [][CancelReadInFlight(cId) => cancel_requested'[cId]]_vars

\* A completed read's token is inert: no step cancels on behalf of a read
\* that is no longer in flight.  The citable shadow of an absence, like
\* the permanent refusal that enters no wait.
THEOREM CompletedReadIgnoresItsToken ==
    Spec => \A cId \in CallIds :
                [][CancelReadInFlight(cId) => ReadInFlight(cId)]_vars

THEOREM ReadInFlightEventuallyResolvedHolds ==
    Spec => ReadInFlightEventuallyResolved

\* The latch is posed by the release that empties the set: this is the
\* half the ordering theorem below cannot carry, and without it a
\* regression that stopped marking released_last would leave that theorem
\* vacuously true.
THEOREM LastReleaseIsLatched ==
    Spec => \A chId \in ChannelIds :
                [][(/\ FinishDisposeChannel(chId)
                    /\ IsLastRelease(chId))
                       => /\ channel_dispose_state'[chId] = "released_last"
                          /\ runtime_dispose_state' = "shutdown_pending"]_vars

THEOREM LastChannelDisposeAwaitsDestroy ==
    Spec => \A chId \in ChannelIds :
                [][(/\ ResolveChannelDispose(chId)
                    /\ channel_dispose_state[chId] = "released_last")
                       => runtime_destroyed[channel_runtime[chId]]]_vars

\* A channel's dispose settles its own calls and no one else's, stated
\* per call because that is where the content is: the step belongs to a
\* channel that is disposing, and it changes nothing of any other call -
\* not its dispose state, not its reader, its writer, its completions,
\* nor the level-1 cancellation it might have latched.  An action
\* theorem, not an invariant: ownership is what the step reads, and no
\* state records which dispose caused which change.
THEOREM ChannelDisposeIsolatesItsCalls ==
    Spec => \A cId \in CallIds :
                [][DisposeCallForChannel(cId) =>
                       /\ channel_dispose_state[call_channel[cId]]
                              = "disposing"
                       /\ \A other \in CallIds \ {cId} :
                              /\ call_dispose_state'[other]
                                     = call_dispose_state[other]
                              /\ headers_completion'[other]
                                     = headers_completion[other]
                              /\ status_completion'[other]
                                     = status_completion[other]
                              /\ reader_state'[other] = reader_state[other]
                              /\ writer_state'[other] = writer_state[other]
                              /\ cancel_requested'[other]
                                     = cancel_requested[other]]_vars

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

THEOREM PublishedCallEventuallySettledHolds ==
    Spec => PublishedCallEventuallySettled

THEOREM HeadersEventuallyResolvedHolds == Spec => HeadersEventuallyResolved

THEOREM StatusEventuallyResolvedHolds == Spec => StatusEventuallyResolved

THEOREM ManagedLivenessTheorem == Spec => ManagedLiveness

===============================================================================
