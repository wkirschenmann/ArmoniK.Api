--------------------------- MODULE DotNetBinding_MCwitnessPrologue ---------------------------
(***************************************************************************)
(* Reachability witness for the read entered during the prologue.          *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as     *)
(* its sibling witness does; what this adds is one target, in a module of   *)
(* its own because ci/tlc.sh resolves a configuration to the module of the  *)
(* same name.                                                              *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* The prologue read, and its cancellation.  A MoveNext may be the first    *)
(* thing an application calls, so the wait for the metadata is part of      *)
(* that read; a token firing then must find a read to arm.  Stated         *)
(* negatively: the trace is the witness that the window exists and is       *)
(* reachable, which is what keeps the guard widened for it from being       *)
(* dead.                                                                   *)
(***************************************************************************)

PrologueReadCancellationUnreachable ==
    \A cId \in CallIds :
        ~(/\ consumer_phase[cId] = "prologue"
          /\ reader_state[cId] = "waiting"
          /\ read_cancel_pending[cId]
          /\ F!L0!NotFailed)

===============================================================================
