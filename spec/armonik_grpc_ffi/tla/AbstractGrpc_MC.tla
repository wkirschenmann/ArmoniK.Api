--------------------------- MODULE AbstractGrpc_MC -------------------------
(***************************************************************************)
(* TLC model checking configuration for AbstractGrpc.                      *)
(* This module adds TLC-specific overrides and small finite constants.     *)
(* Kept separate from the main spec to avoid conflicts with TLAPS.         *)
(***************************************************************************)

EXTENDS AbstractGrpc, TLC

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

\* AbstractGrpc intentionally permits unbounded streams.  Bound only the
\* TLC model so exhaustive safety/liveness exploration has a finite graph.
StateConstraint ==
    /\ \A cId \in CallIds : Len(submitted[cId]) <= 2
    /\ \A cId \in CallIds : Len(received[cId]) <= 2

\* The semantic progress properties quantify over all positive indices.  The
\* finite TLC model checks exactly the indices admitted by StateConstraint.
MCPositiveNaturals == 1..2

MCSubmitProgress ==
    \A cId \in CallIds :
        \A i \in MCPositiveNaturals : SubmitProgressAt(cId, i)

MCDeliveryProgress ==
    \A cId \in CallIds :
        \A i \in MCPositiveNaturals : DeliveryProgressAt(cId, i)

=============================================================================
