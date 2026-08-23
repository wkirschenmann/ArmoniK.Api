--------------------------- MODULE DotNetBinding_MCwitnessBudget ---------------------------
(***************************************************************************)
(* Reachability witness for the budget wait a terminal has made hopeless.  *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as     *)
(* its sibling witnesses do; what this adds is one target, in a module of   *)
(* its own because ci/tlc.sh resolves a configuration to the module of the  *)
(* same name.                                                              *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* A write is waiting on the send budget, the server has ended the call,    *)
(* and the application has not disposed anything.  No budget signal can    *)
(* lead to a lend from here, so the wait is owed a result by the binding    *)
(* rather than by the caller - which is what CancelWriterWait's guard on a  *)
(* call that is no longer active says.  Stated negatively: the violation    *)
(* trace is the witness that the state exists, so that guard is a live      *)
(* disjunct rather than one no behaviour reaches.  Without it the liveness  *)
(* still holds, but only because ApplicationOwedFairness eventually forces  *)
(* a Dispose - a guarantee resting on the caller, which is not what the     *)
(* level claims.                                                           *)
(***************************************************************************)

HopelessBudgetWaitUnreachable ==
    \A cId \in CallIds :
        ~(/\ writer_state[cId] = "waiting_budget"
          /\ ~F!L0!IsActiveCall(cId)
          /\ ~F!L0!IsUnusedCall(cId)
          /\ call_dispose_state[cId] = "active"
          /\ ~cancel_requested[cId]
          /\ F!L0!NotFailed)

===============================================================================
