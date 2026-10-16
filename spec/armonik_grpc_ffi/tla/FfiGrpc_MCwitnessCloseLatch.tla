--------------------------- MODULE FfiGrpc_MCwitnessCloseLatch ---------------------------
(***************************************************************************)
(* Reachability witness for a call that delivers a message after its       *)
(* channel was told to close.                                              *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's; this adds one   *)
(* target.                                                                 *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

(***************************************************************************)
(* A message can be delivered on a call of a closing channel whose task    *)
(* has not yet taken the close into account.  Stated negatively: the      *)
(* violation trace is the witness that ChannelStartClosing latches nothing *)
(* on the calls, and that CancelCallOfClosingChannel is a step of its own. *)
(***************************************************************************)

CloseThenMessageUnreachable ==
    ~ENABLED (\E cId \in CallIds :
                 /\ IsOnClosingChannel(cId)
                 /\ DeliverMessage(cId))

===============================================================================
