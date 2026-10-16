--------------------------- MODULE DotNetBinding_MCwitnessDeadline ---------------------------
(***************************************************************************)
(* Reachability witness for an uncommitted one-request call ending        *)
(* cancelled, which is how the model has its deadline passing.             *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as    *)
(* its sibling witnesses do; what this adds is one target.  Its            *)
(* configuration declares only the request's shape.                        *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* A call that declared one request ends cancelled with nothing submitted, *)
(* its writer idle and the binding never having disposed it. The engine's  *)
(* cancellation at the deadline is that step: the binding's cancellation   *)
(* downcall, which the model does not tell apart from a deadline passing.  *)
(* Stated negatively: the violation trace is the witness that the model    *)
(* lets an uncommitted call end at its deadline.                           *)
(***************************************************************************)

UncommittedDeadlineUnreachable ==
    ~\E cId \in OneRequestCalls :
        /\ L1!L0!IsCancelled(cId)
        /\ ~L1!HasAcceptedSendAt(cId, 1)
        /\ writer_state[cId] = "idle"
        /\ call_dispose_state[cId] = "active"
        /\ reader_state[cId] = "idle"

===============================================================================
