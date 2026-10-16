--------------------------- MODULE FfiGrpc_MCwitnessEmptySend ---------------------------
(***************************************************************************)
(* Reachability witness for the send no buffer carries.                    *)
(*                                                                         *)
(* The bounds and the constant overrides are FfiGrpc_MC's, so this extends *)
(* that module rather than restating them; it adds a message length of     *)
(* zero, which FfiGrpc_MC's lengths never are, and one target.             *)
(***************************************************************************)

EXTENDS FfiGrpc_MC

\* Every message empty, so the empty send is the one a call can make
\* without a lend.
MC_EmptyMessageLength == [msg \in Messages |-> 0]

(***************************************************************************)
(* A call's first send is an empty message, acquitted by its WRITE_DONE,  *)
(* while the call has never been lent a buffer.  Stated negatively: the    *)
(* violation trace is the witness that SendEmptyMessage is a live action   *)
(* and that its send takes the path every send takes to its acquittal.     *)
(***************************************************************************)

EmptySendAcquittedUnreachable ==
    ~\E cId \in CallIds :
        /\ HasAcceptedSendAt(cId, 1)
        /\ IsSendAcquittedAt(cId, 1)
        /\ \A b \in BufferIds : IsFreshBuffer(cId, b)

===============================================================================
