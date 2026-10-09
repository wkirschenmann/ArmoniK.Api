--------------------------- MODULE DotNetBinding_MCwitnessEngineBytes ---------------------------
(***************************************************************************)
(* Reachability witness for the bytes the engine holds for itself.         *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as    *)
(* its sibling witnesses do; what this adds is one target, in a module of  *)
(* its own because ci/tlc.sh resolves a configuration to the module of the *)
(* same name.                                                              *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* Stated negatively: the violation trace IS the witness that the engine   *)
(* takes bytes under the binding and gives them back, the steps of the     *)
(* second level that carry them being passthroughs.                        *)
(***************************************************************************)

EngineNeverGivesBackBytes ==
    [][~\E n \in L1!Sizes : L1!EngineGivesBackBytes(n)]_vars

===============================================================================
