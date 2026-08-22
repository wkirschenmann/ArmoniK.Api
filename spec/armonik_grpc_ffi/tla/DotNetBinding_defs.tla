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

\* The managed safety contract.  ManagedTypeOK is deliberately not a
\* conjunct: it says the state is well typed, not what the binding
\* guarantees, exactly as TypeOK is kept out of the level-0 and level-1
\* manifests.
ManagedSafety ==
    /\ TokenPublishedBeforeStart
    /\ RootSurvivesCallbacks
    /\ RuntimeRootSurvivesCallbacks
    /\ ConsumerPhaseMatchesDispose
    /\ RetryingCallHoldsNoBuffer
    /\ DisposeAwaitsDestroy
    /\ RingNeverOverflows

\* The managed liveness contract.  Both are conditional on the fairness
\* tiers of DotNetBinding, never on a deadline.
ManagedLiveness ==
    /\ BudgetCancellationStopsRetry
    /\ RuntimeDisposeCompletes

===============================================================================
