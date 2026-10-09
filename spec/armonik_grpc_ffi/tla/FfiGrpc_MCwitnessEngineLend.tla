--------------------------- MODULE FfiGrpc_MCwitnessEngineLend ---------------------------
(***************************************************************************)
(* Reachability witness for a lend the engine's own bytes refuse.          *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's, so this extends *)
(* that module rather than restating them.  What it adds is one target, in *)
(* a module of its own because ci/tlc.sh resolves a configuration to the   *)
(* module of the same name.                                                *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

(***************************************************************************)
(* Stated negatively: the violation trace IS the witness.  The engine      *)
(* holds bytes and a lend of some length would be refused now, though it   *)
(* would fit if the engine held nothing - the lend the model admitted      *)
(* before it had a term for these bytes.                                   *)
(***************************************************************************)

EngineBytesNeverRefuseALend ==
    ~(/\ HasEngineHeldBytes
      /\ \E cId \in CallIds, len \in 1..Ceiling :
            /\ ContemplatesLend(cId)
            /\ HasFreeSendSlot(cId)
            /\ ~IsMemoryAvailable(len)
            /\ memory_used - BytesHeldByEngine + len <= Ceiling)

===============================================================================
