------------------------- MODULE DotNetBindingTheorems -------------------------
(***************************************************************************)
(* The public level-2 interface: what the .NET binding proves, stated      *)
(* without proofs.  The proofs live in DotNetBindingTheorems_proofs, which *)
(* carries the refinement's initial predicate, the fairness projection and *)
(* the six host discharges; the rest of this module is still owed.  The    *)
(* pair is registered in ci/check.sh only once every declaration here has  *)
(* a proof there, since that checker's contract is completeness.           *)
(***************************************************************************)

EXTENDS DotNetBinding_defs

(***************************************************************************)
(* REFINEMENT - established the way level 1 established Spec => L0!Spec.   *)
(* Everything proved at levels 0 and 1 is inherited through it, and        *)
(* L0!Spec follows from level 1's RefinesSpec by transitivity.             *)
(***************************************************************************)

THEOREM RefinesInit == Init => L1!Init

\* Relative to the invariant, unlike level 1's own RefinesNext, and for a
\* reason worth naming: two coupled actions witness a level-1 existential
\* with a piece of MANAGED state - CreateChannel passes current_runtime as
\* the runtime of the new channel, RetryLendSucceeds passes retry_len[cId]
\* as the length being lent.  That those values lie in the sets level 1
\* quantifies over is an invariant of this level, not a syntactic fact:
\* current_runtime may be "none", and retry_len may be the sentinel.  So
\* the projection holds on reachable states, which is what ManagedSafety
\* characterizes.  RefinesSpec composes it with ManagedSafetyHolds and is
\* unconditional again.
THEOREM RefinesNext == ManagedSafety /\ [Next]_vars => [L1!Next]_l1_vars

THEOREM RefinesSpec == Spec => L1!Spec

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
                WF_l1_vars(L1!DeliveryCallbackReturns(cId))

THEOREM WriteDoneReturnsDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(L1!WriteDoneReturns(cId))

THEOREM ShutdownCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(L1!ShutdownCallbackReturns(rtId))

THEOREM ResourcesReleasedCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(L1!ResourcesReleasedCallbackReturns(rtId))

THEOREM HostConsumesEventDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(L1!HostConsumesEvent(cId))

THEOREM HostReturnsBufferDischarged ==
    Spec => \A cId \in CallIds, b \in BufferIds :
                WF_l1_vars(L1!HostReturnsBuffer(cId, b))

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

\* MoveNext's token cancels the CALL, never the read alone: the only
\* action that resolves a read on its token also requests the call's
\* cancellation, in the same step.
THEOREM ReadCancellationCancelsCall ==
    Spec => \A cId \in CallIds :
                [][(CancelWaitingRead(cId) \/ CancelParsingRead(cId))
                       => /\ cancel_requested'[cId]
                          /\ call_dispose_state'[cId] = "draining"]_vars

\* A completed read's token is inert.  The content is not that the
\* reaction respects its own guard - it does, trivially - but that the
\* firing itself arms nothing once the read is over: a request can only
\* be raised on a read in flight, so no step of a finished or idle reader
\* ever cancels the call on a stale token's behalf.
THEOREM CompletedReadTokenArmsNothing ==
    Spec => \A cId \in CallIds :
                [][(/\ ~ReadInFlight(cId)
                    /\ ~read_cancel_pending[cId])
                       => ~read_cancel_pending'[cId]]_vars

\* A posted request outlives every step but its own reaction.  This is
\* what forbids the lost cancellation: while the call is still active, no
\* normal completion of the read may clear the flag, and the only two
\* steps that may are the binding's reactions - both of which cancel the
\* call as they discharge it.  Without this, a token that linearized
\* before the end of a parse could be swallowed by that parse finishing.
THEOREM LiveRequestOnlyDischargedByReaction ==
    Spec => \A cId \in CallIds :
                [][(/\ read_cancel_pending[cId]
                    /\ call_dispose_state[cId] = "active"
                    /\ ~read_cancel_pending'[cId])
                       => \/ CancelWaitingRead(cId)
                          \/ CancelParsingRead(cId)]_vars

\* The cancelled parse's slot is acquitted once, and by one action.
\* While that parse is outstanding no other step may advance the call's
\* tail - the header's consumer is in the prologue and the drain's is
\* past the application, neither of which can hold a cancelled parse -
\* and the step that does advance it moves it by exactly one.  Stated on
\* level 1's own consumption counter, which is where an over-release
\* would show.
THEOREM CancelledParseReleasesItsSlotOnce ==
    Spec => \A cId \in CallIds :
                [][(/\ reader_state[cId] = "parsing_cancelled"
                    /\ RingTail(cId)' # RingTail(cId))
                       => /\ FinishCancelledParse(cId)
                          /\ RingTail(cId)' = RingTail(cId) + 1]_vars

THEOREM PendingReadCancellationEventuallyObservedHolds ==
    Spec => PendingReadCancellationEventuallyObserved

THEOREM CancelledReadEventuallyDrainsCallHolds ==
    Spec => CancelledReadEventuallyDrainsCall

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

\* The last releaser's public task never completes early.  Which channel
\* emptied the lease set is latched at its release, so the claim is about
\* that channel and not about whatever the set holds later: a channel
\* that was the last resolves its DisposeAsync only once the generation
\* IT released was destroyed.  An action theorem, the fact being about a
\* step rather than about a state.
THEOREM LastChannelDisposeAwaitsDestroy ==
    Spec => \A chId \in ChannelIds :
                [][(/\ ResolveChannelDispose(chId)
                    /\ channel_dispose_state[chId] = "released_last")
                       => runtime_destroyed[channel_runtime[chId]]]_vars

\* A channel's dispose settles its own calls and no one else's, stated
\* per call because that is where the content is: the step belongs to a
\* channel that is disposing, and it leaves every other call exactly as
\* it found it.  Stated over ManagedCallState rather than over a list of
\* components, so a variable added to the level cannot quietly shrink
\* what "isolates" means.  An action theorem, not an invariant:
\* ownership is what the step reads, and no state records which dispose
\* caused which change.
THEOREM ChannelDisposeIsolatesItsCalls ==
    Spec => \A cId \in CallIds :
                [][DisposeCallForChannel(cId) =>
                       /\ channel_dispose_state[call_channel[cId]]
                              = "disposing"
                       /\ \A other \in CallIds \ {cId} :
                              ManagedCallState(other)'
                                  = ManagedCallState(other)]_vars

(***************************************************************************)
(* MANAGED LIVENESS - one theorem per public promise, aggregated last.     *)
(***************************************************************************)

THEOREM BudgetWaitEndsWhenHopelessHolds ==
    Spec => BudgetWaitEndsWhenHopeless

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
