--------------------------- MODULE FfiGrpc_MCwitnessEngineBytes ---------------------------
(***************************************************************************)
(* Reachability witness for the bytes the engine holds for itself.         *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's, so this extends *)
(* that module rather than restating them.  What it adds is one target, in *)
(* a module of its own because ci/tlc.sh resolves a configuration to the   *)
(* module of the same name.                                                *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

(***************************************************************************)
(* Stated negatively: the violation trace IS the witness, which is what    *)
(* keeps a proof from being about a step that never fires.  The engine     *)
(* takes bytes and gives some back, so a step of each action is part of    *)
(* the trace.                                                              *)
(***************************************************************************)

EngineNeverGivesBackBytes ==
    [][~\E n \in Sizes : EngineGivesBackBytes(n)]_vars

===============================================================================
