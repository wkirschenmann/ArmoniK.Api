--------------------------- MODULE FfiGrpcEnabledTheorems ---------------------------
(***************************************************************************)
(* The level-1 theorems whose statements write ENABLED, kept apart from    *)
(* FfiGrpcTheorems for one mechanical reason: TLAPS normalizes an          *)
(* instantiated module's theorem statements eagerly, and a literal ENABLED *)
(* under substitution aborts the prover - the ENABLED-elimination wrapper  *)
(* belongs to the module where the ENABLED is written.  A statement whose  *)
(* WF is literal instantiates fine, and so does one that merely names a    *)
(* formula, so this module holds exactly the three declarations that       *)
(* cannot: the refusals' conditional enabledness.  Level 2 instantiates    *)
(* FfiGrpcTheorems and never needs these - they speak of level-1 refusal   *)
(* actions the managed writer does not realize.                            *)
(*                                                                         *)
(* A sibling of FfiGrpcTheorems, not a layer above it: both extend         *)
(* FfiGrpc_defs, so instantiating either drags nothing of the other.       *)
(***************************************************************************)

EXTENDS FfiGrpc_defs

\* The refusal statuses are not dead letters.  TLAPS proves that a dead
\* action preserves everything, so each status carries its own enabledness
\* obligation: at any eligible state, the refusal whose guard holds is a
\* step the model can take.  MESSAGE_TOO_LARGE exhibits its own witness -
\* one length above the ceiling always exists and is never lendable; the
\* other two are enabled whenever the state their guard describes arises.
THEOREM TooLargeRefusalEnabled ==
    ASSUME NEW cId \in CallIds,
           L0!IsActiveCall(cId),
           ~IsHandleReleased(cId),
           ~IsCancelRequested(cId),
           HostHoldsNoBuffer(cId)
    PROVE  /\ Ceiling + 1 \in RequestLengths
           /\ ~IsLendable(Ceiling + 1)
           /\ ENABLED RefuseLendTooLarge(cId, Ceiling + 1)

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

===============================================================================
