--------------------------- MODULE FfiGrpc_MCwitnessCancelDrop ---------------------------
(***************************************************************************)
(* Reachability witness for the bytes a cancellation gives back before the *)
(* terminal.                                                               *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's; this adds one   *)
(* target.                                                                 *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

(***************************************************************************)
(* A call still active, whose task has taken a cancellation into account,  *)
(* holds a message it received and never delivered, and the count is back *)
(* at zero.  Stated negatively: the violation trace is the witness that    *)
(* the latch drops the undelivered message's bytes before the terminal, as *)
(* the engine does on a cancellation or a deadline.                        *)
(***************************************************************************)

CancelDropUnreachable ==
    ~\E cId \in CallIds :
        /\ L0!IsActiveCall(cId)
        /\ IsCancelRequested(cId)
        /\ ~HasDeliveredAllReceived(cId)
        /\ memory_used = 0

===============================================================================
