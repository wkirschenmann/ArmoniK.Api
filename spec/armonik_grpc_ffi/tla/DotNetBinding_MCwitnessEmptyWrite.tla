--------------------------- MODULE DotNetBinding_MCwitnessEmptyWrite ---------------------------
(***************************************************************************)
(* Reachability witness for the write of a message of no bytes.            *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as    *)
(* its sibling witnesses do; this adds a message length of zero and one    *)
(* target.                                                                 *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

\* Every message empty, so a write commits the empty buffer.
MC_EmptyMessageLength == [msg \in Messages |-> 0]

(***************************************************************************)
(* A write of an empty message has completed at its WRITE_DONE on a call  *)
(* that was never lent a buffer.  Stated negatively: the violation trace   *)
(* is the witness that CommitEmptyWrite is a live action of this level.    *)
(***************************************************************************)

EmptyWriteCompletedUnreachable ==
    ~\E cId \in CallIds :
        /\ L1!HasAcceptedSendAt(cId, 1)
        /\ L1!IsSendAcquittedAt(cId, 1)
        /\ writer_state[cId] = "idle"
        /\ \A b \in BufferIds : L1!IsFreshBuffer(cId, b)

===============================================================================
