--------------------------- MODULE DotNetBinding_MCwitness ---------------------------
(***************************************************************************)
(* Reachability witness for the cancelled parse of the terminal slot.      *)
(*                                                                         *)
(* The exploration bound, the constant overrides and the doubly-           *)
(* instantiated tuple workarounds are DotNetBinding_MCdirected's, so this  *)
(* extends that module rather than restating them.  What it adds is one    *)
(* target - and a module of its own, because ci/tlc.sh resolves a          *)
(* configuration to the module of the same name.                          *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* The case FinishCancelledParse decodes the status for.  Once it releases  *)
(* that slot no other consumer can produce the status, so a reader that     *)
(* skipped it would strand the drain and the settlement for good.  Stated   *)
(* negatively, like the second-event target it sits beside: the violation   *)
(* trace IS the witness that the branch is live rather than dead code,      *)
(* which is what keeps a proof from being about a step that never fires.    *)
(* Restricted to a healthy runtime - a failed one is the case every promise *)
(* already escapes, so a witness there would show nothing.                 *)
(***************************************************************************)

CancelledTerminalParseUnreachable ==
    \A cId \in CallIds :
        ~(/\ reader_state[cId] = "parsing_cancelled"
          /\ ConsumingTerminal(cId)
          /\ L1!L0!NotFailed)

===============================================================================
