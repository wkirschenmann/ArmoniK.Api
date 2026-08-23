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

LEMMA FairnessRefines == Fairness => L1!Fairness
    \* One step per family rather than one goal for all nineteen.  The
    \* monolithic form is what Isabelle cannot close: expanding an
    \* instantiated Fairness through the substitution builds a single
    \* enormous goal, and under several threads it starves its
    \* neighbours besides, so a run reports failures that a single-
    \* threaded one does not.  Each step here is a projection of a
    \* conjunction; only the QED assembles them, with every conjunct
    \* already a fact.
    <1>1. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!NetworkSend(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>2. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!ReceiveStatus(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>3. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!DeliverInitialMetadata(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>4. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!DeliverMessage(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>5. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!DeliverStatus(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>6. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!DeliverCancelled(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>7. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!EmitWriteDone(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>8. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(L1!RuntimeRelease(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>9. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(L1!EmitShutdownComplete(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>10. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(L1!EmitResourcesReleased(rtId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>11. Fairness => \A chId \in ChannelIds :
              WF_l1_vars(L1!ChannelFinishClosing(chId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>12. Fairness => \A cId \in CallIds, b \in BufferIds :
              WF_l1_vars(L1!FreeReturnedBuffer(cId, b))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>13. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!ReleaseCallHandle(cId))
        BY Isa DEF Fairness, RuntimeOwedFairness
    <1>14. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!DeliveryCallbackReturns(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>15. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!WriteDoneReturns(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>16. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(L1!ShutdownCallbackReturns(rtId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>17. Fairness => \A rtId \in RuntimeIds :
              WF_l1_vars(L1!ResourcesReleasedCallbackReturns(rtId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>18. Fairness => \A cId \in CallIds :
              WF_l1_vars(L1!HostConsumesEvent(cId))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>19. Fairness => \A cId \in CallIds, b \in BufferIds :
              WF_l1_vars(L1!HostReturnsBuffer(cId, b))
        BY Isa DEF Fairness, BindingOwedFairness
    <1>20. QED
        BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16, <1>17, <1>18, <1>19 DEF L1!Fairness, l1_vars

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
                WF_l1_vars(L1!DeliveryCallbackReturns(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM WriteDoneReturnsDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(L1!WriteDoneReturns(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM ShutdownCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(L1!ShutdownCallbackReturns(rtId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM ResourcesReleasedCallbackReturnsDischarged ==
    Spec => \A rtId \in RuntimeIds :
                WF_l1_vars(L1!ResourcesReleasedCallbackReturns(rtId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM HostConsumesEventDischarged ==
    Spec => \A cId \in CallIds : WF_l1_vars(L1!HostConsumesEvent(cId))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

THEOREM HostReturnsBufferDischarged ==
    Spec => \A cId \in CallIds, b \in BufferIds :
                WF_l1_vars(L1!HostReturnsBuffer(cId, b))
    BY Isa DEF Spec, Fairness, BindingOwedFairness

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
===============================================================================
