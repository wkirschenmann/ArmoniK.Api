--------------------------- MODULE FfiGrpc_MCwitnessResize ---------------------------
(***************************************************************************)
(* Reachability witness for the exchange of a lent buffer.                 *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's, so this extends *)
(* that module rather than restating them.  What it adds is one target, in *)
(* a module of its own because ci/tlc.sh resolves a configuration to the   *)
(* module of the same name.                                                *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

(***************************************************************************)
(* A buffer is lent and the exchange for a larger one is possible.  Stated *)
(* negatively: the violation trace IS the witness that ResizeSendBuffer is *)
(* a live action, which is what keeps a proof from being about a step that *)
(* never fires.  The growth is the case: the buffer is exchanged for one   *)
(* that holds more.                                                        *)
(***************************************************************************)

GrowingExchangeUnreachable ==
    ~ENABLED (\E cId \in CallIds, b \in BufferIds, nb \in BufferIds,
                 len \in Sizes, charge \in Sizes :
                 /\ buffer_length[<<cId, b>>] < len
                 /\ ResizeSendBuffer(cId, b, nb, len, charge))

===============================================================================
