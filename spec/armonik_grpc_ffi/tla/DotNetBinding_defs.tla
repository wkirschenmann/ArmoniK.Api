-------------------------- MODULE DotNetBinding_defs --------------------------
(***************************************************************************)
(* The level-2 invariant manifests, following FfiGrpc_defs: the citable    *)
(* conjunctions the proofs open by name and the property-manifest checker  *)
(* binds to design.md.                                                     *)
(*                                                                         *)
(* PLACEHOLDER.  Filled once the DotNetBinding action inventory is agreed: *)
(*  - ManagedCallInv: the per-call conjunction (TokenPublishedBeforeStart, *)
(*    RootSurvivesCallbacks, ReleasesWithinPublications,                   *)
(*    DrainOnlyWhileDisposing, RetryingCallHoldsNoBuffer, ...).            *)
(*  - ManagedRuntimeInv: RuntimeRootSurvivesCallbacks,                     *)
(*    DisposeAwaitsDestroy, the NoDowncallAfterDestroy ordering.           *)
(*  - The inductive invariant IndInv, layered over F!IndInv the way        *)
(*    level 1 layered over level 0.                                        *)
(***************************************************************************)

EXTENDS DotNetBinding

===============================================================================
