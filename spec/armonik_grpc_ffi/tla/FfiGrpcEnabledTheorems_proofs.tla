------------------------ MODULE FfiGrpcEnabledTheorems_proofs ------------------------
(***************************************************************************)
(* The proofs of the three conditional-enabledness theorems.  Separate     *)
(* from FfiGrpcTheorems_proofs because the declarations are: a proofs       *)
(* module pairs with one declarations module, the checker binds them that   *)
(* way, and these three cost seconds rather than riding the level's full    *)
(* pass.                                                                   *)
(***************************************************************************)

\* Extends the defs module, never the declarations module: a proofs module
\* restates its statements verbatim - the checker enforces that - and
\* extending the declarations would inherit them unproved as well.
EXTENDS FfiGrpc_defs, TLAPS

\* The same folded-state vocabulary FfiGrpcTheorems_proofs opens: the
\* refusals' guards read these predicates, and their proofs were
\* written against them being transparent.
USE DEF IsAwaitingWriteDone, IsWriteDoneCallbackRunning, HasFreeSendSlot,
        WriteDonesReturned, SendWindowOccupancy, HasNoSendInFlight,
        HasAcceptedSendAt, IsSendAcquittedAt, PayloadIndices,
        IsDeliveryCallbackRunning, IsCancelRequested, IsHandleReleased,
        OwedPayloads, HostOwnsPayload, HostOwnsNoPayload, HostOwnsSomePayload,
        HostHasDeliveryCredit, HostOwnsAtMostCredits,
        HostOwnsAtMostCreditsPlusOne, IsShutdownEventEmitted,
        IsShutdownCallbackRunning, IsStoppingRuntime, IsReleasedRuntime,
        SecondEventOwed, IsResourcesReleasedEmitted,
        IsResourcesReleasedCallbackRunning,
        IsClosingChannel, IsClosedChannel, HasNoDeliveredEvents

THEOREM TooLargeRefusalEnabled ==
    ASSUME NEW cId \in CallIds,
           L0!IsActiveCall(cId),
           ~IsHandleReleased(cId),
           ~IsCancelRequested(cId),
           HostHoldsNoBuffer(cId)
    PROVE  /\ Ceiling + 1 \in RequestLengths
           /\ ~IsLendable(Ceiling + 1)
           /\ ENABLED RefuseLendTooLarge(cId, Ceiling + 1)
<1>1. Ceiling + 1 \in RequestLengths /\ ~IsLendable(Ceiling + 1)
    BY CeilingIsPositive, SMT DEF RequestLengths, IsLendable
<1>2. ENABLED RefuseLendTooLarge(cId, Ceiling + 1)
    BY CeilingIsPositive, ExpandENABLED, SMT
    DEF RefuseLendTooLarge, ContemplatesLend, IsLendable,
        l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>3. QED BY <1>1, <1>2

THEOREM SlotRefusalEnabled ==
    ASSUME NEW cId \in CallIds,
           L0!IsActiveCall(cId),
           ~IsHandleReleased(cId),
           ~IsCancelRequested(cId),
           HostHoldsNoBuffer(cId),
           ~HasFreeSendSlot(cId)
    PROVE  /\ 0 \in Sizes
           /\ IsLendable(0)
           /\ ENABLED RefuseLendForSlot(cId, 0)
<1>1. 0 \in Sizes /\ IsLendable(0)
    BY CeilingIsPositive, SMT DEF Sizes, IsLendable
<1>2. ENABLED RefuseLendForSlot(cId, 0)
    BY CeilingIsPositive, ExpandENABLED, SMT
    DEF RefuseLendForSlot, ContemplatesLend, IsLendable,
        l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>3. QED BY <1>1, <1>2

THEOREM BudgetRefusalEnabled ==
    ASSUME NEW cId \in CallIds,
           NEW len \in Sizes,
           NEW charge \in CandidateCharges,
           L0!IsActiveCall(cId),
           ~IsHandleReleased(cId),
           ~IsCancelRequested(cId),
           HostHoldsNoBuffer(cId),
           HasFreeSendSlot(cId),
           IsLendable(len),
           CoversRequest(charge, len),
           ~IsMemoryAvailable(charge)
    PROVE  ENABLED RefuseLendForBudget(cId, len, charge)
<1>1. QED
    BY ExpandENABLED, SMT
    DEF RefuseLendForBudget, ContemplatesLend,
        l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars

===============================================================================
