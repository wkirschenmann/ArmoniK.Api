----------------------------- MODULE FfiGrpc_MC ----------------------------
(***************************************************************************)
(* TLC model checking configuration for FfiGrpc.                           *)
(* Exploration and debugging, not evidence: every property named here is   *)
(* proved by tlapm over unbounded constants, where these configurations    *)
(* fix small ones.  A checker that prints a counterexample trace is the    *)
(* fastest way to understand a broken draft, which is why they stay.       *)
(***************************************************************************)

\* FfiGrpc_defs rather than FfiGrpc: its header says both TLAPS and the TLC
\* configurations are meant to extend it, and extending FfiGrpc instead is what
\* left the level-1 SafetyInvariant out of every configuration while level 0
\* checked its own.
EXTENDS FfiGrpc_defs, TLC

(***************************************************************************)
(* SYMMETRY SETS (optional, to reduce state space)                         *)
(***************************************************************************)

Symmetry == Permutations(Messages)
         \union Permutations(CallIds)
         \union Permutations(ChannelIds)
         \union Permutations(RuntimeIds)

(***************************************************************************)
(* FINITE EXPLORATION BOUND                                                *)
(***************************************************************************)

\* The FFI variables are bounded by the level-0 streams, so bounding the
\* streams is enough for a finite graph.
StateConstraint ==
    /\ \A cId \in CallIds : Len(submitted[cId]) <= 2
    /\ \A cId \in CallIds : Len(received[cId]) <= 2

\* The semantic progress properties quantify over all positive indices.  The
\* finite TLC model checks exactly the indices admitted by StateConstraint.
MCPositiveNaturals == 1..2

\* Overrides PayloadIndices in every configuration so TLC can expand the
\* per-payload guarantee.  Bounded by StateConstraint: metadata plus two
\* delivered messages plus one terminal.
MC_PayloadIndices == 1..4

\* Overrides MessageLength in every configuration: a .cfg constant assignment
\* cannot carry a function literal, and Messages is empty in one of them.  One
\* byte per message - what the checks exercise is the ceiling, not a spread of
\* sizes.
MC_MessageLength == [msg \in Messages |-> 1]

\* BudgetEventuallyAdmits quantifies over Nat.  Above the ceiling IsLendable
\* is false and the property is vacuous, so the bounded range loses nothing.
MCBudgetEventuallyAdmits ==
    \A len \in 0..Ceiling :
        (~IsRequestAdmissible(len) ~> IsRequestAdmissible(len))

MCSendsEventuallyAcquitted ==
    \A cId \in CallIds :
        \A k \in MCPositiveNaturals : SendAcquittedAt(cId, k)

(***************************************************************************)
(* TLC WORKAROUND                                                          *)
(***************************************************************************)

\* Overrides l0_vars in every configuration: TLC cannot resolve the
\* instantiated L0!vars inside ENABLED.
MC_l0_vars == <<runtime_state, channel_state, channel_runtime,
                call_state, call_channel, submitted, sent, received,
                delivered, events_delivered, send_closed, status_pending>>

(***************************************************************************)
(* LEVEL-0 FORMULAS UNDER CFG-CITABLE NAMES                                *)
(***************************************************************************)

\* The whole level-0 specification, checked as a property of Spec.
AbstractSpec == L0!Spec

AbstractSafety == L0!SafetyInvariant

EventualTerminal == L0!EventualTerminal
EventualShutdown == L0!EventualShutdown
EventualMetadata == L0!EventualMetadata

MCSubmitProgress ==
    \A cId \in CallIds :
        \A i \in MCPositiveNaturals : L0!SubmitProgressAt(cId, i)

MCDeliveryProgress ==
    \A cId \in CallIds :
        \A i \in MCPositiveNaturals : L0!DeliveryProgressAt(cId, i)

=============================================================================
