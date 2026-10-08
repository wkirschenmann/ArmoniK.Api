--------------------------- MODULE DotNetBinding_MCwitnessResize ---------------------------
(***************************************************************************)
(* Reachability witness for the serializer that outgrows its buffer.       *)
(*                                                                         *)
(* Bounds and constant overrides come from DotNetBinding_MCdirected, as    *)
(* its sibling witnesses do; what this adds is one target, in a module of  *)
(* its own because ci/tlc.sh resolves a configuration to the module of the *)
(* same name.                                                              *)
(***************************************************************************)

EXTENDS DotNetBinding_MCdirected

(***************************************************************************)
(* A write is serializing and the exchange of its buffer for a larger one  *)
(* is possible.  Stated negatively: the violation trace is the witness     *)
(* that WriteResizesBuffer is a live action of this level.                 *)
(***************************************************************************)

WriteResizeUnreachable ==
    ~ENABLED (\E cId \in CallIds, b \in BufferIds, nb \in BufferIds,
                 len \in L1!Sizes, charge \in L1!Sizes :
                 /\ buffer_length[<<cId, b>>] < len
                 /\ WriteResizesBuffer(cId, b, nb, len, charge))

===============================================================================
