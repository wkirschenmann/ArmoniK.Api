----------------------------- MODULE FfiGrpc_MC ----------------------------
(***************************************************************************)
(* TLC model checking configuration for FfiGrpc.                           *)
(* The load-bearing check is AbstractSpec: TLC verifies that Spec refines  *)
(* the full level-0 specification, fairness included, so every fairness    *)
(* lift is model-checked before it is proved.                              *)
(***************************************************************************)

EXTENDS FfiGrpc, TLC

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
