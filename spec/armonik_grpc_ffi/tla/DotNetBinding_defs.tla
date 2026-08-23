-------------------------- MODULE DotNetBinding_defs --------------------------
(***************************************************************************)
(* The level-2 property manifests: the citable conjunctions the proofs     *)
(* will open by name and ci/check_property_manifest.py binds to design.md. *)
(* Both TLAPS and the TLC configurations extend this module, following     *)
(* FfiGrpc_defs.                                                           *)
(*                                                                         *)
(* The inductive invariant is NOT here yet: it is a proof artifact, and    *)
(* no proof exists for this level.  It will layer over F!IndInv the way    *)
(* level 1 layered over level 0.                                           *)
(***************************************************************************)

EXTENDS DotNetBinding

\* The managed safety contract.  ManagedTypeOK is a conjunct, like TypeOK
\* in level 0's SafetyCore; the manifest checker treats both as structural
\* and the document lists the guarantees.
ManagedSafety ==
    /\ ManagedTypeOK
    /\ TokenPublishedBeforeStart
    /\ RootSurvivesCallbacks
    /\ RuntimeRootSurvivesCallbacks
    /\ ConsumerPhaseMatchesDispose
    /\ AtMostOneReaderOutstanding
    /\ DrainNeverOverlapsApplicationConsumer
    /\ WaitingWriterHoldsNoBuffer
    /\ SerializingWriterHoldsTheBuffer
    /\ WaitMatchesRefusal
    /\ ManagedWriterNeverObservesSlotBusy
    /\ RetryLenMatchesWait
    /\ DisposeAwaitsDestroy
    /\ RuntimeManagerCoherent
    /\ LiveChannelUsesCurrentRuntime
    /\ ManagedShutdownHasNoHostDebt
    /\ LiveChannelKeepsRuntimeAlive
    /\ NoRuntimeShutdownWhileLeased
    /\ RejectedChannelHasNoNativeHalf
    /\ ReadCancelPendingOnlyInFlight
    /\ ParsingReadOwnsItsSlot
    /\ ChannelStateMatchesNative
    /\ RuntimeStateMatchesNative
    /\ DisposeLeavesNoManagedWaiter
    /\ SettledCallOwesNothing
    /\ AbsentRuntimeOwesNothing
    /\ RingNeverOverflows

\* The inductive core, on level 1's own pattern: the typing, the machine
\* coherence carried as such, and level 1's core - everything else in
\* ManagedSafety is DERIVED from this by ManagedIndInvImpliesSafety, so a
\* conjunct proved derivable leaves the induction and never returns.
\* RingNeverOverflows is the first: the ring's occupancy IS level 1's owed
\* payload count, and F!IndInv already bounds it.
ManagedMachineInv ==
    /\ TokenPublishedBeforeStart
    /\ RootSurvivesCallbacks
    /\ RuntimeRootSurvivesCallbacks
    /\ ConsumerPhaseMatchesDispose
    /\ AtMostOneReaderOutstanding
    /\ DrainNeverOverlapsApplicationConsumer
    /\ WaitingWriterHoldsNoBuffer
    /\ SerializingWriterHoldsTheBuffer
    /\ WaitMatchesRefusal
    /\ ManagedWriterNeverObservesSlotBusy
    /\ RetryLenMatchesWait
    /\ DisposeAwaitsDestroy
    /\ RuntimeManagerCoherent
    /\ LiveChannelUsesCurrentRuntime
    /\ ManagedShutdownHasNoHostDebt
    /\ LiveChannelKeepsRuntimeAlive
    /\ NoRuntimeShutdownWhileLeased
    /\ RejectedChannelHasNoNativeHalf
    /\ ReadCancelPendingOnlyInFlight
    /\ ParsingReadOwnsItsSlot
    /\ ChannelStateMatchesNative
    /\ RuntimeStateMatchesNative
    /\ DisposeLeavesNoManagedWaiter
    /\ SettledCallOwesNothing
    /\ AbsentRuntimeOwesNothing

ManagedIndInv ==
    /\ F!IndInv
    /\ ManagedTypeOK
    /\ ManagedMachineInv

\* The managed liveness contract.  Every promise crossing the native
\* runtime carries the ~NotFailed escape; none rests on a deadline.
ManagedLiveness ==
    /\ BudgetWaitEndsWhenHopeless
    /\ PendingWriteEventuallySettled
    /\ CallDisposeCompletes
    /\ ChannelConstructionCompletes
    /\ ChannelLeaseEventuallyReleased
    /\ ChannelDisposeCompletes
    /\ RuntimeDisposeCompletes
    /\ CallRootEventuallyFreed
    /\ RuntimeRootEventuallyFreed
    /\ InFlightPayloadEventuallyReleased
    /\ ReadInFlightEventuallyResolved
    /\ PendingReadCancellationEventuallyObserved
    /\ CancelledReadEventuallyDrainsCall
    /\ WaitingReaderEventuallyResolved
    /\ PublishedCallEventuallySettled
    /\ HeadersEventuallyResolved
    /\ StatusEventuallyResolved

===============================================================================
