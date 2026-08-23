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
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FreeRuntimeRoot

LEMMA ProjectsCreateRuntime ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds, CreateRuntime(rtId, chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CreateRuntime, F!Next

LEMMA ProjectsAcquireLease ==
    ASSUME NEW chId \in ChannelIds, AcquireLease(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF AcquireLease

LEMMA ProjectsCreateChannel ==
    \* The witness for level 1's runtime is current_runtime, which is in
    \* RuntimeIds only because the invariant says so and the guard excludes
    \* the sentinel.
    ASSUME ManagedTypeOK, NEW chId \in ChannelIds, CreateChannel(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF ManagedTypeOK, CreateChannel, F!Next

LEMMA ProjectsRejectChannelCreation ==
    ASSUME NEW chId \in ChannelIds, RejectChannelCreation(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF RejectChannelCreation

LEMMA ProjectsBeginDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, BeginDisposeChannel(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF BeginDisposeChannel

LEMMA ProjectsFinishDisposeChannel ==
    ASSUME NEW chId \in ChannelIds, FinishDisposeChannel(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeChannel, F!Next

LEMMA ProjectsResolveChannelDispose ==
    ASSUME NEW chId \in ChannelIds, ResolveChannelDispose(chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF ResolveChannelDispose

LEMMA ProjectsBeginRuntimeShutdown ==
    ASSUME NEW rtId \in RuntimeIds, BeginRuntimeShutdown(rtId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF BeginRuntimeShutdown, F!Next

LEMMA ProjectsFinishDisposeRuntime ==
    ASSUME NEW rtId \in RuntimeIds, FinishDisposeRuntime(rtId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeRuntime, F!Next

LEMMA ProjectsShutdownReturns ==
    ASSUME NEW rtId \in RuntimeIds, ShutdownReturns(rtId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF ShutdownReturns, F!Next

LEMMA ProjectsResourcesReleasedReturns ==
    ASSUME NEW rtId \in RuntimeIds, ResourcesReleasedReturns(rtId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF ResourcesReleasedReturns, F!Next

LEMMA ProjectsBeginMoveNext ==
    ASSUME NEW cId \in CallIds, BeginMoveNext(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF BeginMoveNext

LEMMA ProjectsBeginParseEvent ==
    ASSUME NEW cId \in CallIds, BeginParseEvent(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF BeginParseEvent

LEMMA ProjectsFinishConsumePayload ==
    ASSUME NEW cId \in CallIds, FinishConsumePayload(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FinishConsumePayload, F!Next

LEMMA ProjectsCancelWaiter ==
    ASSUME NEW cId \in CallIds, CancelWaiter(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CancelWaiter

LEMMA ProjectsRequestReadCancellation ==
    ASSUME NEW cId \in CallIds, RequestReadCancellation(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF RequestReadCancellation

LEMMA ProjectsCancelWaitingRead ==
    ASSUME NEW cId \in CallIds, CancelWaitingRead(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CancelWaitingRead, F!Next

LEMMA ProjectsCancelParsingRead ==
    ASSUME NEW cId \in CallIds, CancelParsingRead(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CancelParsingRead, F!Next

LEMMA ProjectsFinishCancelledParse ==
    ASSUME NEW cId \in CallIds, FinishCancelledParse(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FinishCancelledParse, F!Next

LEMMA ProjectsHandoffToDrain ==
    ASSUME NEW cId \in CallIds, HandoffToDrain(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF HandoffToDrain

LEMMA ProjectsConsumeHeader ==
    ASSUME NEW cId \in CallIds, ConsumeHeader(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF ConsumeHeader, F!Next

LEMMA ProjectsBeginDisposeCall ==
    ASSUME NEW cId \in CallIds, BeginDisposeCall(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF BeginDisposeCall, F!Next

LEMMA ProjectsDisposeCallForChannel ==
    \* Delegates to BeginDisposeCall, so it projects through it.
    ASSUME NEW cId \in CallIds, DisposeCallForChannel(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF DisposeCallForChannel, BeginDisposeCall, F!Next

LEMMA ProjectsDrainRelease ==
    ASSUME NEW cId \in CallIds, DrainRelease(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF DrainRelease, F!Next

LEMMA ProjectsFinishDisposeCall ==
    ASSUME NEW cId \in CallIds, FinishDisposeCall(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF FinishDisposeCall

LEMMA ProjectsSettleCall ==
    ASSUME NEW cId \in CallIds, SettleCall(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF SettleCall

LEMMA ProjectsCancelWriterWait ==
    ASSUME NEW cId \in CallIds, CancelWriterWait(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CancelWriterWait

LEMMA ProjectsWriteDoneCompletes ==
    ASSUME NEW cId \in CallIds, WriteDoneCompletes(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF WriteDoneCompletes, F!Next

LEMMA ProjectsCloseWriter ==
    ASSUME NEW cId \in CallIds, CloseWriter(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CloseWriter, F!Next

LEMMA ProjectsOnEventReturns ==
    ASSUME NEW cId \in CallIds, OnEventReturns(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF OnEventReturns, F!Next

LEMMA ProjectsTerminalCallbackReturns ==
    ASSUME NEW cId \in CallIds, TerminalCallbackReturns(cId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF TerminalCallbackReturns, F!Next

LEMMA ProjectsStartCall ==
    ASSUME NEW cId \in CallIds, NEW chId \in ChannelIds, StartCall(cId, chId)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF StartCall, F!Next

LEMMA ProjectsWriteLendSucceeds ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW len \in F!Sizes, NEW charge \in F!Sizes, WriteLendSucceeds(cId, b, len, charge)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF WriteLendSucceeds, F!Next

LEMMA ProjectsWriteRefusedBudget ==
    ASSUME NEW cId \in CallIds, NEW len \in F!Sizes, NEW charge \in F!CandidateCharges, WriteRefusedBudget(cId, len, charge)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF WriteRefusedBudget, F!Next

LEMMA ProjectsWriteRefusedTooLarge ==
    ASSUME NEW cId \in CallIds, NEW len \in F!RequestLengths, WriteRefusedTooLarge(cId, len)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF WriteRefusedTooLarge, F!Next

LEMMA ProjectsRetryLendSucceeds ==
    \* The witness for level 1's length is retry_len[cId], which is a real
    \* length rather than the sentinel exactly while the write is waiting -
    \* which is RetryLenMatchesWait, not a syntactic fact.
    ASSUME ManagedTypeOK, RetryLenMatchesWait,
           NEW cId \in CallIds, NEW b \in BufferIds, NEW charge \in F!Sizes,
           RetryLendSucceeds(cId, b, charge)
    PROVE  F!Next \/ UNCHANGED l1_vars
    \* The sentinel lies outside every length level 1 quantifies over -
    \* NoRetryLen = Ceiling + 2 > Ceiling + 1 - which is arithmetic on a
    \* positive Ceiling, not a definition unfold, so the assumption and the
    \* solver both appear here.
    <1>1. NoRetryLen \notin F!Sizes
        BY F!CeilingIsPositive, SMT DEF NoRetryLen, F!Sizes
    <1>2. retry_len[cId] \in F!Sizes
        BY <1>1 DEF ManagedTypeOK, RetryLenMatchesWait, RetryLendSucceeds
    <1>3. QED
        BY <1>1, <1>2
        DEF ManagedTypeOK, RetryLenMatchesWait, RetryLendSucceeds, F!Next

LEMMA ProjectsCommitWrite ==
    ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW b \in BufferIds, CommitWrite(cId, msg, b)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF CommitWrite, F!Next

LEMMA ProjectsWriteAborted ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, WriteAborted(cId, b)
    PROVE  F!Next \/ UNCHANGED l1_vars
    BY DEF WriteAborted, F!Next

THEOREM RefinesNext == ManagedSafety /\ [Next]_vars => [F!Next]_l1_vars
    \* One lemma per disjunct, so this is the case analysis and nothing
    \* else.  The stuttering side is inherited: vars holds l1_vars, so
    \* nothing changing means level 1's tuple did not change either.
    <1>1. ASSUME ManagedSafety, Next PROVE F!Next \/ UNCHANGED l1_vars
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
               ManagedStutter, F!Next, ManagedSafety
    <1>2. ASSUME UNCHANGED vars PROVE UNCHANGED l1_vars
        BY <1>2 DEF vars
    <1>3. QED
        BY <1>1, <1>2
===============================================================================
