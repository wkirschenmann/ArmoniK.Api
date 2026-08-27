---------------------- MODULE AbstractGrpcTheorems_proofs ---------------------
(***************************************************************************)
(* Proofs of every theorem declared in AbstractGrpcTheorems, plus the      *)
(* internal lemmas they rest on.  Statements of declared theorems must     *)
(* stay textually identical to the declarations; tlapm verifies this       *)
(* module and discharges every obligation.                                 *)
(***************************************************************************)

EXTENDS AbstractGrpc_defs, SequenceTheorems, NaturalsInduction,
        FiniteSetTheorems, TLAPS

\* The split event sets, re-enumerated: gives SMT flat memberships
\* instead of union reasoning inside large obligations.
\* Where a step names a time budget - SMTT, IsaT - it is because the default
\* one closes it with no margin, so it passes on a quiet machine and fails on
\* a busy one.  The figures are five times what the solver was measured to
\* need, and they are stated per step rather than obtained by raising
\* --stretch, which would hide the next such step instead of fixing this one.

THEOREM StatusKindsExpansion ==
    /\ StatusKinds = {"COMPLETED", "CANCELLED"}
    /\ EventKinds = {"INITIAL_METADATA", "MESSAGE", "COMPLETED", "CANCELLED"}
    /\ {"MESSAGE"} \union StatusKinds = {"MESSAGE", "COMPLETED", "CANCELLED"}
<1>1. QED
    BY SMTT(90) DEF StatusKinds, EventKinds

(***************************************************************************)
(* FOOTPRINT DECOMPOSITION AND FRAME FACTS                                 *)
(***************************************************************************)

THEOREM NextDecomposition == Next <=> NextByFootprint
<1>1. QED
    BY DEF Next, NextByFootprint, NextSafe, NextSafeRuntimeOnly,
           NextSafeRuntimeChannel, NextSafeChannelOnly,
           NextSafeChannelCall, NextSafeCallOnly, NextFail,
           NextExplicitStutter

THEOREM NextSafeRuntimeOnlyFrame ==
    NextSafeRuntimeOnly => UNCHANGED <<ChannelVars, CallVars>>
<1>1. QED
    BY SMTT(90) DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease

THEOREM NextSafeRuntimeChannelFrame ==
    NextSafeRuntimeChannel => UNCHANGED CallVars
<1>1. QED
    BY SMTT(90) DEF NextSafeRuntimeChannel, RuntimeBeginShutdown

THEOREM NextSafeChannelOnlyFrame ==
    NextSafeChannelOnly => UNCHANGED <<RuntimeVars, CallVars>>
<1>1. QED
    BY DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing

THEOREM NextSafeChannelCallFrame ==
    NextSafeChannelCall => UNCHANGED RuntimeVars
<1>1. QED
    BY DEF NextSafeChannelCall, ChannelFinishClosing

THEOREM NextSafeCallOnlyFrame ==
    NextSafeCallOnly => UNCHANGED <<RuntimeVars, ChannelVars>>
<1>1. QED
    BY DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
           NetworkSend, NetworkReceive, ReceiveStatus,
           DeliverInitialMetadata, DeliverMessage, DeliverStatus, CallCancel

THEOREM NextExplicitStutterFrame ==
    NextExplicitStutter => UNCHANGED vars
<1>1. QED
    BY DEF NextExplicitStutter, RemainFailed, RemainReleased

\* Appending on the long side preserves a prefix.  Specialized to the
\* message sort: SMT instantiates element quantifiers, not set ones.
THEOREM IsPrefixAppendRight ==
    ASSUME NEW s \in Seq(Messages), NEW t \in Seq(Messages),
           NEW e \in Messages,
           IsPrefix(s, t)
    PROVE  IsPrefix(s, Append(t, e))
<1>1. QED
    BY SMT DEF IsPrefix

(***************************************************************************)
(* FRAME TRANSPORT                                                         *)
(* A predicate that reads only CallVars survives any step that leaves      *)
(* CallVars unchanged.  These lemmas carry EventTraceInv and               *)
(* MessageFlowInv across the three call-silent footprint groups and across *)
(* stuttering steps, so only the call-writing groups ever argue about      *)
(* them directly.                                                          *)
(***************************************************************************)

THEOREM CallFramePreservesEventTrace ==
    EventTraceInv /\ UNCHANGED CallVars => EventTraceInv'
<1>1. QED
    BY SMT DEF MessageEventsMatchDelivered, HasStatus, CallVars, EventTraceInv, EventStreamShape,
               HasStatus, UsedCalls, IsUnusedCall

THEOREM CallFramePreservesMessageFlow ==
    MessageFlowInv /\ UNCHANGED CallVars => MessageFlowInv'
<1>1. QED
    BY SMT DEF CallVars, MessageFlowInv, SubmittedPrefixOfSent,
               ReceivedPrefixOfDelivered, CompleteDelivery, IsPrefix,
               HasStatus, IsCancelled, IsTerminalCall,
               UsedCalls, IsUnusedCall

THEOREM VarsFramePreservesTypeOK ==
    TypeOK /\ UNCHANGED vars => TypeOK'
<1>1. QED
    BY SMT DEF vars, TypeOK

THEOREM VarsFramePreservesStructural ==
    StructuralInv /\ UNCHANGED vars => StructuralInv'
<1>1. QED
    BY SMT DEF vars, StructuralInv, TypeOK, SingleRuntime,
               ChannelSentinelEquivalence, CallSentinelEquivalence,
               UnusedCallsAreEmpty, ChannelLifecycleInv, CallLifecycleInv,
               TerminalStatusEquivalence, HasStatus,
               UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
               ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM StutteringPreservesHealthyStrongInv ==
    StrongInv /\ NotFailed /\ UNCHANGED vars => StrongInv' /\ NotFailed'
<1>1. StructuralInv /\ UNCHANGED vars => StructuralInv'
    BY VarsFramePreservesStructural
<1>2. EventTraceInv /\ UNCHANGED vars => EventTraceInv'
    BY SMT DEF MessageEventsMatchDelivered, HasStatus, vars, EventTraceInv, EventStreamShape, HasStatus,
               UsedCalls, IsUnusedCall
<1>3. MessageFlowInv /\ UNCHANGED vars => MessageFlowInv'
    BY SMT DEF vars, MessageFlowInv, SubmittedPrefixOfSent,
               ReceivedPrefixOfDelivered, CompleteDelivery, IsPrefix,
               HasStatus, IsCancelled, IsTerminalCall, UsedCalls, IsUnusedCall
<1>4. NotFailed /\ UNCHANGED vars => NotFailed'
    BY SMT DEF vars, NotFailed
<1>5. QED BY <1>1, <1>2, <1>3, <1>4 DEF StrongInv

\* Appending a terminal event to a well-formed stream that carries no
\* status yet keeps the stream well-formed.  Shared by DeliverStatus,
\* CallCancel and ChannelFinishClosing arguments.
THEOREM AppendTerminalKeepsShape ==
    ASSUME NEW es \in Seq(EventKinds), NEW ev \in EventKinds,
           ev \in StatusKinds,
           Len(es) > 0,
           es[1] = "INITIAL_METADATA",
           \A i \in 2..Len(es) : es[i] \in {"MESSAGE"} \union StatusKinds,
           \A i \in 2..Len(es) : i < Len(es) => es[i] = "MESSAGE",
           es[Len(es)] \notin StatusKinds
    PROVE  /\ Append(es, ev)[1] = "INITIAL_METADATA"
           /\ \A i \in 2..Len(Append(es, ev)) :
                  Append(es, ev)[i] \in {"MESSAGE"} \union StatusKinds
           /\ \A i \in 2..Len(Append(es, ev)) :
                  i < Len(Append(es, ev)) => Append(es, ev)[i] = "MESSAGE"
<1>1. Len(Append(es, ev)) = Len(es) + 1 /\ Append(es, ev)[Len(es) + 1] = ev
    BY SMT DEF EventKinds, StatusKinds
<1>2. \A i \in 1..Len(es) : Append(es, ev)[i] = es[i]
    BY SMT DEF EventKinds, StatusKinds
<1>3. Len(es) >= 2 => es[Len(es)] = "MESSAGE"
    BY SMT DEF EventKinds, StatusKinds
<1>4. QED BY <1>1, <1>2, <1>3, SMTT(90) DEF EventKinds, StatusKinds

\* The cancel is the one action with two write shapes: from an empty stream
\* it writes metadata and terminal in one step, from a nonempty one it appends
\* the terminal alone.  Both keep the event count equal to one metadata, one
\* event per delivered message, and the terminal.
THEOREM CancelKeepsAlignment ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ MessageEventsMatchDelivered /\ CallCancel(cId) =>
               MessageEventsMatchDelivered'
<1>00. SUFFICES ASSUME TypeOK, MessageEventsMatchDelivered, CallCancel(cId)
                PROVE  MessageEventsMatchDelivered'
    OBVIOUS
<1>0. /\ events_delivered \in [CallIds -> Seq(EventKinds)]
      /\ delivered \in [CallIds -> Seq(Messages)]
      /\ UNCHANGED delivered
    BY <1>00, SMT DEF TypeOK, CallCancel
<1>1. CASE events_delivered[cId] = <<>>
  <2>1. events_delivered' = [events_delivered EXCEPT
            ![cId] = <<"INITIAL_METADATA", "CANCELLED">>]
    BY <1>00, <1>1, Zenon DEF CallCancel
  <2>2. /\ Len(events_delivered'[cId]) = 2
        /\ events_delivered'[cId][2] = "CANCELLED"
        /\ Len(delivered[cId]) = 0
    BY <1>00, <1>0, <1>1, <2>1, SMT
    DEF MessageEventsMatchDelivered, HasStatus, UsedCalls, IsUnusedCall,
        CallCancel, IsActiveCall, ActiveCallStates, CallStates, TypeOK
  <2>3. QED
    BY <1>00, <1>0, <1>1, <2>1, <2>2, SMTT(120)
    DEF MessageEventsMatchDelivered, HasStatus, StatusKinds,
        UsedCalls, IsUnusedCall,
        CallCancel, IsActiveCall, ActiveCallStates, CallStates, TypeOK
<1>2. CASE events_delivered[cId] # <<>>
  <2>1. events_delivered' = [events_delivered EXCEPT
            ![cId] = Append(events_delivered[cId], "CANCELLED")]
    BY <1>00, <1>2, Zenon DEF CallCancel
  <2>2. ~HasStatus(cId)
    BY <1>00, <1>2, Zenon DEF CallCancel, HasStatus
  <2>25. /\ Len(events_delivered'[cId]) = Len(events_delivered[cId]) + 1
         /\ events_delivered'[cId][Len(events_delivered[cId]) + 1]
                = "CANCELLED"
         /\ \A c \in CallIds : c # cId =>
                events_delivered'[c] = events_delivered[c]
    BY <1>00, <1>0, <1>2, <2>1, AppendProperties, SMT
    DEF TypeOK, EventKinds
  <2>26. HasStatus(cId)'
    BY <1>00, <1>0, <2>25, SMT DEF HasStatus, StatusKinds, TypeOK, EventKinds
  <2>3. QED
    BY <1>00, <1>0, <1>2, <2>2, <2>25, <2>26, SMTT(120)
    DEF MessageEventsMatchDelivered, HasStatus, StatusKinds, EventKinds,
        UsedCalls, IsUnusedCall,
        CallCancel, IsActiveCall, ActiveCallStates, CallStates, TypeOK
<1>3. QED BY <1>1, <1>2

THEOREM DeliverKeepsAlignment ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ MessageEventsMatchDelivered /\ DeliverMessage(cId) =>
               MessageEventsMatchDelivered'
<1>00. SUFFICES ASSUME TypeOK, MessageEventsMatchDelivered, DeliverMessage(cId)
                PROVE  MessageEventsMatchDelivered'
    OBVIOUS
<1>0. /\ events_delivered \in [CallIds -> Seq(EventKinds)]
      /\ delivered \in [CallIds -> Seq(Messages)]
      /\ received \in [CallIds -> Seq(Messages)]
      /\ UNCHANGED call_state
    BY <1>00, SMT DEF TypeOK, DeliverMessage, CallVars
<1>1. /\ events_delivered' = [events_delivered EXCEPT
              ![cId] = Append(events_delivered[cId], "MESSAGE")]
      /\ delivered' = [delivered EXCEPT
              ![cId] = Append(delivered[cId],
                              received[cId][Len(delivered[cId]) + 1])]
      /\ Len(delivered[cId]) < Len(received[cId])
    BY <1>00, Zenon DEF DeliverMessage
<1>2. ~HasStatus(cId) /\ Len(events_delivered[cId]) >= 1
    BY <1>00, Zenon DEF DeliverMessage
<1>25. /\ Len(events_delivered'[cId]) = Len(events_delivered[cId]) + 1
       /\ events_delivered'[cId][Len(events_delivered[cId]) + 1] = "MESSAGE"
       /\ \A c \in CallIds : c # cId =>
              events_delivered'[c] = events_delivered[c]
    BY <1>00, <1>0, <1>1, AppendProperties, SMT DEF TypeOK, EventKinds
<1>26. /\ Len(delivered'[cId]) = Len(delivered[cId]) + 1
       /\ \A c \in CallIds : c # cId => delivered'[c] = delivered[c]
    BY <1>00, <1>0, <1>1, AppendProperties, SMT DEF TypeOK
<1>27. ~HasStatus(cId)'
    BY <1>0, <1>25, SMT DEF HasStatus, StatusKinds, TypeOK, EventKinds
<1>3. QED
    BY <1>00, <1>0, <1>2, <1>25, <1>26, <1>27, SMTT(120)
    DEF MessageEventsMatchDelivered, HasStatus, StatusKinds, EventKinds,
        UsedCalls, IsUnusedCall, TypeOK

THEOREM StutteringPreservesIndInv == IndInv /\ UNCHANGED vars => IndInv'
<1>1. ASSUME IndInv, UNCHANGED vars
      PROVE IndInv'
  <2>1. TypeOK' /\ SingleRuntime'
    BY <1>1, VarsFramePreservesTypeOK DEF vars, IndInv, SingleRuntime
  <2>2. NotFailed' => StrongInv'
    <3>1. CASE NotFailed
      <4>1. StrongInv BY <1>1, <3>1 DEF IndInv
      <4>2. QED BY <1>1, <3>1, <4>1, StutteringPreservesHealthyStrongInv
    <3>2. CASE ~NotFailed
      <4>1. ~NotFailed' BY <1>1, <3>2 DEF vars, NotFailed
      <4>2. QED BY <4>1
    <3>3. QED BY <3>1, <3>2
  <2>3. QED BY <2>1, <2>2 DEF IndInv
<1>2. QED BY <1>1

(***************************************************************************)
(* FOUNDATIONAL IMPLICATIONS                                               *)
(***************************************************************************)

THEOREM EventStreamShapeImpliesMetadataFirst ==
    EventStreamShape => MetadataFirst
<1>1. QED
    BY DEF EventStreamShape, MetadataFirst

THEOREM EventStreamShapeImpliesNoEventAfterStatus ==
    TypeOK /\ EventStreamShape => NoEventAfterStatus
<1>1. QED
    BY SMTT(90) DEF TypeOK, EventStreamShape, NoEventAfterStatus,
               UsedCalls, EventKinds, StatusKinds

THEOREM ChannelLifecycleInvImpliesChannelSafety ==
    TypeOK /\ ChannelSentinelEquivalence /\ ChannelLifecycleInv =>
        /\ ChannelOwnership
        /\ ActiveChannelImpliesActiveRuntime
        /\ StoppingClosesChannels
        /\ ReleasedNoChannels
<1>1. USE NoneNotInRuntimeIds
<1>2. QED
    BY SMT DEF TypeOK, RuntimeStates, ChannelStates,
               ChannelSentinelEquivalence, ChannelLifecycleInv,
               ChannelOwnership, ActiveChannelImpliesActiveRuntime,
               StoppingClosesChannels, ReleasedNoChannels,
               UsedChannels, ActiveChannelStates, ActiveChannels, ChannelsOf

THEOREM CallLifecycleInvImpliesActiveNoStatus ==
    TypeOK /\ CallLifecycleInv =>
        \A cId \in ActiveCalls : ~HasStatus(cId)
<1>1. QED
    BY SMT DEF TypeOK, CallStates, CallLifecycleInv,
               TerminalStatusEquivalence, ActiveCallStates, ActiveCalls,
               UsedCalls, IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM CallLifecycleInvImpliesSendAfterEndSend ==
    TypeOK /\ CallLifecycleInv => SendAfterEndSend
<1>1. QED
    BY SMT DEF TypeOK, CallStates, CallLifecycleInv,
               SendAfterEndSend, ActiveCallStates, ActiveCalls, UsedCalls,
               IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM CallLifecycleInvImpliesClosedChannelNoCalls ==
    TypeOK /\ CallSentinelEquivalence /\ CallLifecycleInv =>
        ClosedChannelNoCalls
<1>1. USE NoneNotInChannelIds
<1>2. QED
    BY SMTT(90) DEF TypeOK, ChannelStates, CallStates,
               CallSentinelEquivalence, CallLifecycleInv,
               ClosedChannelNoCalls, UsedChannels, ActiveChannelStates,
               ActiveChannels, UsedCalls, ActiveCallStates, ActiveCalls,
               CallsOf, IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM CallLifecycleInvImpliesCallSafety ==
    TypeOK /\ CallSentinelEquivalence /\ ChannelLifecycleInv /\
    CallLifecycleInv =>
        /\ SendAfterEndSend
        /\ CallOwnership
        /\ ActiveCallImpliesActiveChannel
        /\ ClosedChannelNoCalls
        /\ ReleasedNoCalls
        /\ TerminalStatusEquivalence
<1>1. USE NoneNotInChannelIds, NoneNotInRuntimeIds,
          CallLifecycleInvImpliesActiveNoStatus,
          CallLifecycleInvImpliesSendAfterEndSend,
          CallLifecycleInvImpliesClosedChannelNoCalls
<1>2. QED
    BY SMTT(90) DEF TypeOK, RuntimeStates, ChannelStates, CallStates,
               CallSentinelEquivalence, ChannelLifecycleInv,
               CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
               SendAfterEndSend, CallOwnership,
               ActiveCallImpliesActiveChannel, ClosedChannelNoCalls,
               ReleasedNoCalls, UsedChannels, ActiveChannels, UsedCalls,
               ActiveCallStates, ActiveCalls, CallsOf, ChannelsOf,
               IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM StructuralInvImpliesLifecycleSafety ==
    StructuralInv =>
        /\ SendAfterEndSend
        /\ SingleRuntime
        /\ ChannelOwnership
        /\ CallOwnership
        /\ ActiveCallImpliesActiveChannel
        /\ ClosedChannelNoCalls
        /\ ActiveChannelImpliesActiveRuntime
        /\ StoppingClosesChannels
        /\ ReleasedNoChannels
        /\ ReleasedNoCalls
        /\ TerminalStatusEquivalence
<1>1. USE ChannelLifecycleInvImpliesChannelSafety,
          CallLifecycleInvImpliesCallSafety
<1>2. QED
    BY SMTT(90) DEF StructuralInv

THEOREM StrongInvImpliesSafetyCore == StrongInv => SafetyCore
<1>1. USE EventStreamShapeImpliesMetadataFirst,
          EventStreamShapeImpliesNoEventAfterStatus,
          StructuralInvImpliesLifecycleSafety
<1>2. QED
    BY DEF StrongInv, StructuralInv, EventTraceInv, MessageFlowInv,
           SafetyCore

THEOREM IndInvImpliesSafetyInvariant == IndInv => SafetyInvariant
<1>1. USE StrongInvImpliesSafetyCore
<1>2. QED
    BY DEF IndInv, SafetyInvariant

(***************************************************************************)
(* TYPE PRESERVATION, ONE LEMMA PER FOOTPRINT GROUP                        *)
(***************************************************************************)

THEOREM RuntimeOnlyPreservesTypeOK ==
    TypeOK /\ NextSafeRuntimeOnly => TypeOK'
<1>1. QED
    BY SMT DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
               RuntimeVars, ChannelVars, CallVars,
               TypeOK, RuntimeStates, ChannelStates, CallStates, EventKinds, StatusKinds

THEOREM RuntimeChannelPreservesTypeOK ==
    TypeOK /\ NextSafeRuntimeChannel => TypeOK'
<1>1. QED
    BY SMT DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
               RuntimeVars, ChannelVars, CallVars,
               TypeOK, RuntimeStates, ChannelStates, CallStates, EventKinds, StatusKinds

THEOREM ChannelOnlyPreservesTypeOK ==
    TypeOK /\ NextSafeChannelOnly => TypeOK'
<1>1. QED
    BY SMT DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
               RuntimeVars, ChannelVars, CallVars,
               TypeOK, RuntimeStates, ChannelStates, CallStates, EventKinds, StatusKinds

THEOREM ChannelCallPreservesTypeOK ==
    TypeOK /\ NextSafeChannelCall => TypeOK'
<1>1. QED
    BY StatusKindsExpansion, SMTT(90)
    DEF NextSafeChannelCall, ChannelFinishClosing,
               RuntimeVars, ChannelVars, CallVars,
               TypeOK, RuntimeStates, ChannelStates, CallStates, EventKinds, StatusKinds

THEOREM CallOnlyPreservesTypeOK ==
    TypeOK /\ NextSafeCallOnly => TypeOK'
<1>1. ASSUME TypeOK, NextSafeCallOnly
      PROVE TypeOK'
  <2>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
    BY <1>1, <2>1, StatusKindsExpansion, SMTT(90)
    DEF CallStart, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>2. CASE \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
    BY <1>1, <2>2, StatusKindsExpansion, SMTT(90)
    DEF SendMessage, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>3. CASE \E cId \in CallIds : EndSend(cId)
    BY <1>1, <2>3, StatusKindsExpansion, SMTT(90)
    DEF EndSend, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>4. CASE \E cId \in CallIds : NetworkSend(cId)
    BY <1>1, <2>4, StatusKindsExpansion, SMTT(90)
    DEF NetworkSend, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    BY <1>1, <2>5, StatusKindsExpansion, SMTT(90)
    DEF NetworkReceive, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
    BY <1>1, <2>6, StatusKindsExpansion, SMTT(90)
    DEF ReceiveStatus, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
    BY <1>1, <2>7, StatusKindsExpansion, SMTT(90)
    DEF DeliverInitialMetadata, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>8. CASE \E cId \in CallIds : DeliverMessage(cId)
    BY <1>1, <2>8, StatusKindsExpansion, SMTT(90)
    DEF DeliverMessage, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>9. CASE \E cId \in CallIds : DeliverStatus(cId)
    BY <1>1, <2>9, StatusKindsExpansion, SMTT(90)
    DEF DeliverStatus, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>10. CASE \E cId \in CallIds : CallCancel(cId)
    BY <1>1, <2>10, StatusKindsExpansion, SMTT(90)
    DEF CallCancel, RuntimeVars, ChannelVars, CallVars,
        TypeOK, RuntimeStates, ChannelStates, CallStates,
        EventKinds, StatusKinds
  <2>11. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10 DEF NextSafeCallOnly
<1>2. QED BY <1>1

THEOREM FailPreservesTypeOK ==
    TypeOK /\ NextFail => TypeOK'
<1>1. QED
    BY SMT DEF NextFail, RuntimeFail,
               RuntimeVars, ChannelVars, CallVars,
               TypeOK, RuntimeStates, ChannelStates, CallStates, EventKinds, StatusKinds

THEOREM NextPreservesTypeOK == TypeOK /\ Next => TypeOK'
<1>1. ASSUME TypeOK, Next
      PROVE TypeOK'
  <2>1. CASE NextSafeRuntimeOnly
    BY <1>1, <2>1, RuntimeOnlyPreservesTypeOK
  <2>2. CASE NextSafeRuntimeChannel
    BY <1>1, <2>2, RuntimeChannelPreservesTypeOK
  <2>3. CASE NextSafeChannelOnly
    BY <1>1, <2>3, ChannelOnlyPreservesTypeOK
  <2>4. CASE NextSafeChannelCall
    BY <1>1, <2>4, ChannelCallPreservesTypeOK
  <2>5. CASE NextSafeCallOnly
    BY <1>1, <2>5, CallOnlyPreservesTypeOK
  <2>6. CASE NextFail
    BY <1>1, <2>6, FailPreservesTypeOK
  <2>7. CASE NextExplicitStutter
    BY <1>1, <2>7, NextExplicitStutterFrame, VarsFramePreservesTypeOK
  <2>8. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7,
       NextDecomposition, SMTT(90) DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

(***************************************************************************)
(* CONDITIONAL INDUCTION ACROSS FAILURE                                    *)
(***************************************************************************)

THEOREM InitEstablishesIndInv == Init => IndInv
<1>1. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
<1>2. QED
    BY SMT DEF MessageEventsMatchDelivered, HasStatus, Init, IndInv, NotFailed, StrongInv, StructuralInv,
               EventTraceInv, MessageFlowInv, TypeOK, RuntimeStates,
               ChannelStates, CallStates, EventKinds, StatusKinds, SingleRuntime,
               ChannelSentinelEquivalence, CallSentinelEquivalence,
               UnusedCallsAreEmpty, ChannelLifecycleInv, CallLifecycleInv,
               EventStreamShape,
               SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
               CompleteDelivery, TerminalStatusEquivalence, HasStatus,
               UsedChannels, UsedCalls, ActiveChannels, ActiveCalls,
               ActiveChannelStates, ActiveCallStates, ChannelsOf, CallsOf,
               IsUnusedCall, IsActiveCall, IsTerminalCall, IsCancelled,
               IsPrefix

(***************************************************************************)
(* STRONG INVARIANT PRESERVATION, ONE LEMMA PER FOOTPRINT GROUP            *)
(* The three call-silent groups get EventTraceInv and MessageFlowInv for   *)
(* free from the transport lemmas; only StructuralInv is argued.           *)
(***************************************************************************)

THEOREM RuntimeOnlyPreservesStrongInv ==
    StrongInv /\ NotFailed /\ NextSafeRuntimeOnly => StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafeRuntimeOnly
      PROVE StrongInv' /\ NotFailed'
  <2>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
  <2>1. UNCHANGED <<ChannelVars, CallVars>>
    BY <1>1, NextSafeRuntimeOnlyFrame
  <2>2. EventTraceInv' /\ MessageFlowInv'
    BY <1>1, <2>1, CallFramePreservesEventTrace, CallFramePreservesMessageFlow
       DEF StrongInv, ChannelVars, CallVars
  <2>3. StructuralInv' /\ NotFailed'
    BY <1>1, <2>1, SMTT(90)
    DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
        RuntimeVars, ChannelVars, CallVars,
        StrongInv, StructuralInv, TypeOK, RuntimeStates, ChannelStates,
        CallStates, EventKinds, StatusKinds, SingleRuntime, ChannelSentinelEquivalence,
        CallSentinelEquivalence, UnusedCallsAreEmpty, ChannelLifecycleInv,
        CallLifecycleInv, TerminalStatusEquivalence, HasStatus, NotFailed,
        UsedChannels, UsedCalls, ActiveChannels, ActiveCalls,
        ActiveChannelStates, ActiveCallStates, ChannelsOf, CallsOf,
        IsUnusedCall, IsActiveCall, IsTerminalCall
  <2>4. QED BY <2>2, <2>3 DEF StrongInv
<1>2. QED BY <1>1

THEOREM RuntimeChannelPreservesStrongInv ==
    StrongInv /\ NotFailed /\ NextSafeRuntimeChannel =>
        StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafeRuntimeChannel
      PROVE StrongInv' /\ NotFailed'
  <2>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
  <2>1. UNCHANGED CallVars
    BY <1>1, NextSafeRuntimeChannelFrame
  <2>2. EventTraceInv' /\ MessageFlowInv'
    BY <1>1, <2>1, CallFramePreservesEventTrace, CallFramePreservesMessageFlow
       DEF StrongInv
  <2>3. StructuralInv' /\ NotFailed'
    BY <1>1, SMTT(90)
    DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
        RuntimeVars, ChannelVars, CallVars,
        StrongInv, StructuralInv, TypeOK, RuntimeStates, ChannelStates,
        CallStates, EventKinds, StatusKinds, SingleRuntime, ChannelSentinelEquivalence,
        CallSentinelEquivalence, UnusedCallsAreEmpty, ChannelLifecycleInv,
        CallLifecycleInv, TerminalStatusEquivalence, HasStatus, NotFailed,
        UsedChannels, UsedCalls, ActiveChannels, ActiveCalls,
        ActiveChannelStates, ActiveCallStates, ChannelsOf, CallsOf,
        IsUnusedCall, IsActiveCall, IsTerminalCall
  <2>4. QED BY <2>2, <2>3 DEF StrongInv
<1>2. QED BY <1>1

THEOREM ChannelOnlyPreservesStrongInv ==
    StrongInv /\ NotFailed /\ NextSafeChannelOnly => StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafeChannelOnly
      PROVE StrongInv' /\ NotFailed'
  <2>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
  <2>1. UNCHANGED <<RuntimeVars, CallVars>>
    BY <1>1, NextSafeChannelOnlyFrame
  <2>2. EventTraceInv' /\ MessageFlowInv'
    BY <1>1, <2>1, CallFramePreservesEventTrace, CallFramePreservesMessageFlow
       DEF StrongInv, RuntimeVars, CallVars
  <2>3. StructuralInv' /\ NotFailed'
    BY <1>1, <2>1, SMTT(90)
    DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
        RuntimeVars, ChannelVars, CallVars,
        StrongInv, StructuralInv, TypeOK, RuntimeStates, ChannelStates,
        CallStates, EventKinds, StatusKinds, SingleRuntime, ChannelSentinelEquivalence,
        CallSentinelEquivalence, UnusedCallsAreEmpty, ChannelLifecycleInv,
        CallLifecycleInv, TerminalStatusEquivalence, HasStatus, NotFailed,
        UsedChannels, UsedCalls, ActiveChannels, ActiveCalls,
        ActiveChannelStates, ActiveCallStates, ChannelsOf, CallsOf,
        IsUnusedCall, IsActiveCall, IsTerminalCall
  <2>4. QED BY <2>2, <2>3 DEF StrongInv
<1>2. QED BY <1>1

THEOREM ChannelCallPreservesStrongInv ==
    StrongInv /\ NotFailed /\ NextSafeChannelCall => StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafeChannelCall
      PROVE StrongInv' /\ NotFailed'
  <2>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
  <2>1. PICK chId \in ChannelIds : ChannelFinishClosing(chId)
    BY <1>1 DEF NextSafeChannelCall
  <2>2. UNCHANGED RuntimeVars
    BY <2>1 DEF ChannelFinishClosing
  <2>3. UNCHANGED <<channel_runtime, call_channel, submitted, sent,
                    received, delivered, send_closed>>
    BY <2>1 DEF ChannelFinishClosing
  <2>4. /\ channel_state' = [channel_state EXCEPT ![chId] = "closed"]
        /\ channel_state[chId] = "closing"
    BY <2>1 DEF ChannelFinishClosing
  \* Calls that are not active on the closing channel keep their state.
  <2>5. \A c \in CallIds :
            ~(call_channel[c] = chId /\ IsActiveCall(c)) =>
                /\ call_state'[c] = call_state[c]
                /\ events_delivered'[c] = events_delivered[c]
                /\ status_pending'[c] = status_pending[c]
    BY <1>1, <2>1, SMTT(90)
    DEF ChannelFinishClosing, StrongInv, StructuralInv, TypeOK,
        IsActiveCall, ActiveCallStates, HasStatus
  \* An affected call is cancelled: an active call carries no status yet
  \* (CallLifecycleInv), so the update appends CANCELLED or installs the
  \* two-event stream.
  <2>6. \A c \in CallIds :
            call_channel[c] = chId /\ IsActiveCall(c) =>
                /\ call_state'[c] = "terminal"
                /\ ~status_pending'[c]
                /\ ~HasStatus(c)
                /\ events_delivered[c] = <<>> =>
                       events_delivered'[c] = <<"INITIAL_METADATA", "CANCELLED">>
                /\ events_delivered[c] # <<>> =>
                       events_delivered'[c] = Append(events_delivered[c], "CANCELLED")
                /\ HasStatus(c)'
                /\ IsCancelled(c)'
    BY <1>1, <2>1, SMT
    DEF ChannelFinishClosing, StrongInv, StructuralInv, TypeOK, CallStates,
        EventKinds, StatusKinds, CallLifecycleInv, TerminalStatusEquivalence,
        IsActiveCall, ActiveCallStates, IsTerminalCall, IsUnusedCall,
        HasStatus, IsCancelled, UsedCalls
  <2>7. TypeOK'
    BY <1>1, ChannelCallPreservesTypeOK DEF StrongInv, StructuralInv
  <2>8a. SingleRuntime' /\ ChannelSentinelEquivalence' /\ ChannelLifecycleInv'
    BY <1>1, <2>2, <2>3, <2>4, SMTT(90)
    DEF RuntimeVars, StrongInv, StructuralInv, TypeOK, RuntimeStates,
        ChannelStates, SingleRuntime, ChannelSentinelEquivalence,
        ChannelLifecycleInv, UsedChannels, ActiveChannelStates,
        ActiveChannels
  <2>8b. CallSentinelEquivalence' /\ UnusedCallsAreEmpty' /\ CallLifecycleInv'
    BY <1>1, <2>3, <2>4, <2>5, <2>6, <2>7, SMTT(90)
    DEF StrongInv, StructuralInv, TypeOK, ChannelStates, CallStates,
        EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
        CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
        UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
        ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
  <2>8. StructuralInv'
    BY <2>7, <2>8a, <2>8b, SMTT(90) DEF StructuralInv
  <2>9. EventTraceInv'
    <3>0. /\ UNCHANGED delivered
          /\ delivered \in [CallIds -> Seq(Messages)]
          /\ events_delivered \in [CallIds -> Seq(EventKinds)]
      BY <1>1, <2>1, SMT
      DEF ChannelFinishClosing, StrongInv, StructuralInv, TypeOK
    <3>1. EventStreamShape'
      BY <1>1, <2>3, <2>5, <2>6, <2>7, AppendTerminalKeepsShape,
         StatusKindsExpansion, SMTT(90)
      DEF StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, CallLifecycleInv,
          TerminalStatusEquivalence, HasStatus, UsedCalls, ActiveCallStates,
          IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>2. MessageEventsMatchDelivered'
      <4>1. ASSUME NEW c \in CallIds, c \in UsedCalls'
            PROVE  Len(events_delivered'[c]) =
                       (IF Len(events_delivered'[c]) > 0 THEN 1 ELSE 0)
                       + Len(delivered'[c])
                       + (IF HasStatus(c)' THEN 1 ELSE 0)
        <5>0. c \in UsedCalls =>
                  Len(events_delivered[c]) =
                      (IF Len(events_delivered[c]) > 0 THEN 1 ELSE 0)
                      + Len(delivered[c])
                      + (IF HasStatus(c) THEN 1 ELSE 0)
          BY <1>1, Zenon
          DEF StrongInv, EventTraceInv, MessageEventsMatchDelivered
        <5>1. CASE call_channel[c] = chId /\ IsActiveCall(c)
          <6>0. c \in UsedCalls
            BY <5>1, Zenon
            DEF UsedCalls, IsUnusedCall, IsActiveCall, ActiveCallStates
          <6>1. CASE events_delivered[c] = <<>>
            BY <1>1, <2>6, <3>0, <5>0, <5>1, <6>0, <6>1, SMTT(120)
            DEF HasStatus, StatusKinds, EventKinds
          <6>2. CASE events_delivered[c] # <<>>
            <7>1. /\ events_delivered'[c]
                        = Append(events_delivered[c], "CANCELLED")
                  /\ ~HasStatus(c)
                  /\ HasStatus(c)'
              BY <2>6, <5>1, <6>2, Zenon
            <7>2. Len(events_delivered'[c]) = Len(events_delivered[c]) + 1
              BY <3>0, <7>1, AppendProperties, SMT DEF EventKinds
            <7>3. QED
              BY <3>0, <5>0, <6>0, <6>2, <7>1, <7>2, SMTT(120)
          <6>3. QED BY <6>1, <6>2
        <5>2. CASE ~(call_channel[c] = chId /\ IsActiveCall(c))
          <6>1. /\ events_delivered'[c] = events_delivered[c]
                /\ call_state'[c] = call_state[c]
            BY <2>5, <5>2, Zenon
          <6>2. c \in UsedCalls
            BY <4>1, <6>1, Zenon DEF UsedCalls, IsUnusedCall
          <6>3. HasStatus(c)' = HasStatus(c)
            BY <6>1, Zenon DEF HasStatus
          <6>4. QED BY <3>0, <5>0, <6>1, <6>2, <6>3, SMTT(120)
        <5>3. QED BY <5>1, <5>2
      <4>2. QED
        BY <4>1, Zenon DEF MessageEventsMatchDelivered, UsedCalls, IsUnusedCall
    <3>3. QED BY <3>1, <3>2 DEF EventTraceInv
  <2>10. MessageFlowInv'
    BY <1>1, <2>3, <2>5, <2>6, <2>7, SMTT(90)
    DEF StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
        EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
        CompleteDelivery, IsPrefix, TerminalStatusEquivalence, HasStatus,
        IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
        IsActiveCall, IsTerminalCall
  <2>11. NotFailed'
    BY <1>1, <2>2 DEF RuntimeVars, NotFailed
  <2>12. QED BY <2>8, <2>9, <2>10, <2>11 DEF StrongInv
<1>2. QED BY <1>1

THEOREM CallOnlyPreservesStrongInv ==
    StrongInv /\ NotFailed /\ NextSafeCallOnly => StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafeCallOnly
      PROVE StrongInv' /\ NotFailed'
  <2>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds
  <2>1. UNCHANGED <<RuntimeVars, ChannelVars>>
    BY <1>1, NextSafeCallOnlyFrame
  \* The channel and runtime conjuncts of StructuralInv read only variables
  \* this group leaves unchanged, and TypeOK has its own group lemma, so
  \* the per-action cases below argue about the three call conjuncts only.
  <2>2. TypeOK'
    BY <1>1, CallOnlyPreservesTypeOK DEF StrongInv, StructuralInv
  <2>3. SingleRuntime' /\ ChannelSentinelEquivalence' /\ ChannelLifecycleInv'
    BY <1>1, <2>1, SMT
    DEF RuntimeVars, ChannelVars, StrongInv, StructuralInv, TypeOK,
        RuntimeStates, ChannelStates, SingleRuntime,
        ChannelSentinelEquivalence, ChannelLifecycleInv,
        UsedChannels, ActiveChannelStates, ActiveChannels
  <2>4. CallSentinelEquivalence' /\ UnusedCallsAreEmpty' /\ CallLifecycleInv'
    <3>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
      BY <1>1, <2>1, <3>1, SMTT(90)
      DEF CallStart,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>2. CASE \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
      BY <1>1, <2>1, <3>2, SMTT(90)
      DEF SendMessage,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>3. CASE \E cId \in CallIds : EndSend(cId)
      BY <1>1, <2>1, <3>3, SMT
      DEF EndSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>4. CASE \E cId \in CallIds : NetworkSend(cId)
      BY <1>1, <2>1, <3>4, SMT
      DEF NetworkSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
      BY <1>1, <2>1, <3>5, StatusKindsExpansion, SMT
      DEF NetworkReceive,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
      BY <1>1, <2>1, <3>6, SMT
      DEF ReceiveStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
      BY <1>1, <2>1, <3>7, SMTT(90)
      DEF DeliverInitialMetadata,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>8. CASE \E cId \in CallIds : DeliverMessage(cId)
      BY <1>1, <2>1, <3>8, StatusKindsExpansion, SMTT(90)
      DEF DeliverMessage,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>9. CASE \E cId \in CallIds : DeliverStatus(cId)
      BY <1>1, <2>1, <3>9, SMTT(90)
      DEF DeliverStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>10. CASE \E cId \in CallIds : CallCancel(cId)
      BY <1>1, <2>1, <3>10, SMT
      DEF CallCancel,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
          EventKinds, StatusKinds, CallSentinelEquivalence, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedChannels, UsedCalls, ActiveChannels, ActiveChannelStates,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>11. QED BY <1>1, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>7, <3>8, <3>9, <3>10 DEF NextSafeCallOnly
  <2>5. EventTraceInv'
    <3>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
      BY <1>1, <2>1, <3>1, SMT
      DEF MessageEventsMatchDelivered, HasStatus, CallStart,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>2. CASE \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
      BY <1>1, <2>1, <3>2, SMT
      DEF MessageEventsMatchDelivered, HasStatus, SendMessage,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>3. CASE \E cId \in CallIds : EndSend(cId)
      BY <1>1, <2>1, <3>3, SMT
      DEF MessageEventsMatchDelivered, HasStatus, EndSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>4. CASE \E cId \in CallIds : NetworkSend(cId)
      BY <1>1, <2>1, <3>4, SMTT(90)
      DEF MessageEventsMatchDelivered, HasStatus, NetworkSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
      BY <1>1, <2>1, <3>5, StatusKindsExpansion, IsPrefixAppendRight, SMT
      DEF MessageEventsMatchDelivered, HasStatus, NetworkReceive,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
      BY <1>1, <2>1, <3>6, SMT
      DEF MessageEventsMatchDelivered, HasStatus, ReceiveStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
      BY <1>1, <2>1, <3>7, SMTT(90)
      DEF MessageEventsMatchDelivered, HasStatus, DeliverInitialMetadata,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>8. CASE \E cId \in CallIds : DeliverMessage(cId)
      <4>1. ASSUME NEW cId \in CallIds, DeliverMessage(cId)
            PROVE  EventTraceInv'
        <5>1. MessageEventsMatchDelivered'
          BY <1>1, <2>1, <4>1, DeliverKeepsAlignment, Zenon
          DEF StrongInv, StructuralInv, EventTraceInv
        <5>2. QED
          BY <1>1, <2>1, <4>1, <5>1, StatusKindsExpansion, SMTT(90)
          DEF HasStatus, DeliverMessage,
              RuntimeVars, ChannelVars, CallVars,
              StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
              EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
              CallLifecycleInv, TerminalStatusEquivalence,
              UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
              IsTerminalCall
      <4>2. QED BY <3>8, <4>1
    <3>9. CASE \E cId \in CallIds : DeliverStatus(cId)
      BY <1>1, <2>1, <3>9, AppendTerminalKeepsShape, SMT
      DEF MessageEventsMatchDelivered, HasStatus, DeliverStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
          EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
          IsTerminalCall
    <3>10. CASE \E cId \in CallIds : CallCancel(cId)
      <4>1. ASSUME NEW cId \in CallIds, CallCancel(cId)
            PROVE  EventTraceInv'
        <5>1. MessageEventsMatchDelivered'
          BY <1>1, <2>1, <4>1, CancelKeepsAlignment, Zenon
          DEF StrongInv, StructuralInv, EventTraceInv
        <5>2. QED
          BY <1>1, <2>1, <4>1, <5>1, AppendTerminalKeepsShape,
             StatusKindsExpansion, SMTT(300)
          DEF HasStatus, CallCancel,
              RuntimeVars, ChannelVars, CallVars,
              StrongInv, StructuralInv, EventTraceInv, TypeOK, CallStates,
              EventKinds, StatusKinds, EventStreamShape, UnusedCallsAreEmpty,
              CallLifecycleInv, TerminalStatusEquivalence,
              UsedCalls, ActiveCallStates, IsUnusedCall, IsActiveCall,
              IsTerminalCall
      <4>2. QED BY <3>10, <4>1
    <3>11. QED BY <1>1, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>7, <3>8, <3>9, <3>10 DEF NextSafeCallOnly
  <2>6. MessageFlowInv'
    <3>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
      BY <1>1, <2>1, <3>1, SMT
      DEF CallStart,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>2. CASE \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
      BY <1>1, <2>1, <3>2, SMTT(90)
      DEF SendMessage,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>3. CASE \E cId \in CallIds : EndSend(cId)
      BY <1>1, <2>1, <3>3, SMT
      DEF EndSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>4. CASE \E cId \in CallIds : NetworkSend(cId)
      BY <1>1, <2>1, <3>4, SMTT(90)
      DEF NetworkSend,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
      <4>1. SubmittedPrefixOfSent'
        BY <1>1, <3>5, SMT
        DEF NetworkReceive, RuntimeVars, ChannelVars, CallVars,
            StrongInv, StructuralInv, MessageFlowInv, TypeOK,
            SubmittedPrefixOfSent, IsPrefix, UsedCalls, IsUnusedCall,
            IsActiveCall, ActiveCallStates
      <4>2. ReceivedPrefixOfDelivered'
        <5>1. PICK cId \in CallIds, msg \in Messages :
                  NetworkReceive(cId, msg)
          BY <3>5
        <5>2. /\ received'[cId] = Append(received[cId], msg)
              /\ \A c \in CallIds :
                     c # cId => received'[c] = received[c]
          BY <1>1, <5>1, SMT
          DEF NetworkReceive, StrongInv, StructuralInv, TypeOK
        <5>3. delivered' = delivered /\ call_state' = call_state
          BY <5>1, SMT DEF NetworkReceive, RuntimeVars, ChannelVars
        <5>4. IsPrefix(delivered[cId], received[cId])
          BY <1>1, <5>1, SMT
          DEF NetworkReceive, StrongInv, StructuralInv, MessageFlowInv,
              ReceivedPrefixOfDelivered, UsedCalls, IsUnusedCall,
              IsActiveCall, ActiveCallStates
        <5>5. IsPrefix(delivered[cId], received'[cId])
          BY <1>1, <5>2, <5>4, IsPrefixAppendRight, SMT
          DEF StrongInv, StructuralInv, TypeOK, IsPrefix
        <5>6. QED
          BY <1>1, <5>2, <5>3, <5>5, SMT
          DEF StrongInv, StructuralInv, MessageFlowInv,
              ReceivedPrefixOfDelivered, UsedCalls, IsUnusedCall
      <4>3. CompleteDelivery'
        BY <1>1, <3>5, SMT
        DEF NetworkReceive, RuntimeVars, ChannelVars, CallVars,
            StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
            CompleteDelivery, CallLifecycleInv, TerminalStatusEquivalence,
            HasStatus, IsCancelled, UsedCalls, IsUnusedCall,
            IsActiveCall, ActiveCallStates, IsTerminalCall
      <4>4. QED BY <4>1, <4>2, <4>3 DEF MessageFlowInv
    <3>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
      BY <1>1, <2>1, <3>6, SMT
      DEF ReceiveStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
      BY <1>1, <2>1, <3>7, SMTT(90)
      DEF DeliverInitialMetadata,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>8. CASE \E cId \in CallIds : DeliverMessage(cId)
      BY <1>1, <2>1, <3>8, StatusKindsExpansion, SMTT(90)
      DEF DeliverMessage,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>9. CASE \E cId \in CallIds : DeliverStatus(cId)
      BY <1>1, <2>1, <3>9, SMT
      DEF DeliverStatus,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>10. CASE \E cId \in CallIds : CallCancel(cId)
      BY <1>1, <2>1, <3>10, SMT
      DEF CallCancel,
          RuntimeVars, ChannelVars, CallVars,
          StrongInv, StructuralInv, MessageFlowInv, TypeOK, CallStates,
          EventKinds, StatusKinds, SubmittedPrefixOfSent, ReceivedPrefixOfDelivered,
          CompleteDelivery, IsPrefix, UnusedCallsAreEmpty,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>11. QED BY <1>1, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>7, <3>8, <3>9, <3>10 DEF NextSafeCallOnly
  <2>7. NotFailed'
    BY <1>1, <2>1 DEF RuntimeVars, NotFailed
  <2>8. QED BY <2>2, <2>3, <2>4, <2>5, <2>6, <2>7 DEF StrongInv, StructuralInv
<1>2. QED BY <1>1

THEOREM SafeNextPreservesHealthyStrongInv ==
    StrongInv /\ NotFailed /\ NextSafe => StrongInv' /\ NotFailed'
<1>1. ASSUME StrongInv, NotFailed, NextSafe
      PROVE StrongInv' /\ NotFailed'
  <2>1. CASE NextSafeRuntimeOnly
    BY <1>1, <2>1, RuntimeOnlyPreservesStrongInv
  <2>2. CASE NextSafeRuntimeChannel
    BY <1>1, <2>2, RuntimeChannelPreservesStrongInv
  <2>3. CASE NextSafeChannelOnly
    BY <1>1, <2>3, ChannelOnlyPreservesStrongInv
  <2>4. CASE NextSafeChannelCall
    BY <1>1, <2>4, ChannelCallPreservesStrongInv
  <2>5. CASE NextSafeCallOnly
    BY <1>1, <2>5, CallOnlyPreservesStrongInv
  <2>6. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5 DEF NextSafe
<1>2. QED BY <1>1

THEOREM FailNextLeavesHealthyRegion ==
    StrongInv /\ NextFail => TypeOK' /\ SingleRuntime' /\ ~NotFailed'
<1>1. QED
    BY SMT DEF NextFail, RuntimeFail, RuntimeVars, ChannelVars, CallVars,
               StrongInv, StructuralInv, TypeOK,
               RuntimeStates, SingleRuntime, NotFailed

\* In the failed region every runtime transition is disabled: the failed
\* slot blocks RuntimeCreate's quiescence guard, and SingleRuntime leaves
\* no other slot in a state RuntimeBeginShutdown, RuntimeRelease or
\* RuntimeFail could fire from.
THEOREM FailedRegionPreserved ==
    ASSUME TypeOK, SingleRuntime, ~NotFailed, Next
    PROVE  TypeOK' /\ SingleRuntime' /\ ~NotFailed'
<1>1. TypeOK'
    BY NextPreservesTypeOK
<1>2. UNCHANGED runtime_state
  <2>1. CASE NextSafeRuntimeOnly \/ NextSafeRuntimeChannel \/ NextFail
    BY <2>1
    DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextFail,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease, RuntimeFail,
        TypeOK, RuntimeStates, SingleRuntime, NotFailed
  <2>2. CASE NextSafeChannelOnly \/ NextSafeChannelCall \/ NextSafeCallOnly
    BY <2>2, NextSafeChannelOnlyFrame, NextSafeChannelCallFrame,
       NextSafeCallOnlyFrame, SMTT(90) DEF RuntimeVars
  <2>3. CASE NextExplicitStutter
    BY <2>3, NextExplicitStutterFrame, SMTT(90) DEF vars
  <2>4. QED
    BY <2>1, <2>2, <2>3, NextDecomposition DEF NextByFootprint, NextSafe
<1>3. SingleRuntime' /\ ~NotFailed'
    BY <1>2 DEF SingleRuntime, NotFailed
<1>4. QED BY <1>1, <1>3

THEOREM IndInvPreserved == IndInv /\ [Next]_vars => IndInv'
<1>1. ASSUME IndInv, [Next]_vars
      PROVE IndInv'
  <2>1. CASE UNCHANGED vars
    <3>1. QED BY <1>1, <2>1, StutteringPreservesIndInv
  <2>2. CASE Next /\ ~NotFailed
    <3>1. TypeOK /\ SingleRuntime BY <1>1 DEF IndInv
    <3>2. TypeOK' /\ SingleRuntime' /\ ~NotFailed'
           BY <2>2, <3>1, FailedRegionPreserved
    <3>3. QED BY <3>2 DEF IndInv
  <2>3. CASE Next /\ NotFailed /\ NextSafe
    <3>1. StrongInv BY <1>1, <2>3 DEF IndInv
    <3>2. StrongInv' /\ NotFailed'
           BY <2>3, <3>1, SafeNextPreservesHealthyStrongInv
    <3>3. QED BY <3>2 DEF IndInv, StrongInv, StructuralInv
  <2>4. CASE Next /\ NotFailed /\ NextFail
    <3>1. StrongInv BY <1>1, <2>4 DEF IndInv
    <3>2. TypeOK' /\ SingleRuntime' /\ ~NotFailed'
           BY <2>4, <3>1, FailNextLeavesHealthyRegion
    <3>3. QED BY <3>2 DEF IndInv
  <2>5. CASE Next /\ NotFailed /\ NextExplicitStutter
    <3>1. UNCHANGED vars
           BY <2>5, NextExplicitStutterFrame
    <3>2. QED BY <1>1, <3>1, StutteringPreservesIndInv
  <2>6. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5,
                  NextDecomposition DEF NextByFootprint
<1>2. QED BY <1>1

THEOREM SafetyBehaviorEstablishesIndInv ==
    Init /\ [][Next]_vars => []IndInv
<1>1. Init => IndInv
    BY InitEstablishesIndInv
<1>2. IndInv /\ [Next]_vars => IndInv'
    BY IndInvPreserved
<1>3. QED BY <1>1, <1>2, PTL

(***************************************************************************)
(* NOMINAL-WORLD ENTRY                                                     *)
(***************************************************************************)

THEOREM InitEstablishesHealthyStrongInv ==
    Init => StrongInv /\ NotFailed
<1>1. USE InitEstablishesIndInv
<1>2. QED
    BY SMT DEF Init, IndInv, NotFailed, RuntimeStates

\* The nominal world entered from Init: what a reader-facing fairness
\* requirement can assume without mentioning StrongInv.
THEOREM NominalBehaviorEstablishesStrongInv ==
    Init /\ [][NextSafe]_vars => []StrongInv
<1>1. Init => StrongInv /\ NotFailed
    BY InitEstablishesHealthyStrongInv
<1>2. StrongInv /\ NotFailed /\ [NextSafe]_vars => StrongInv' /\ NotFailed'
  <2>1. CASE NextSafe
    <3>1. QED BY <2>1, SafeNextPreservesHealthyStrongInv
  <2>2. CASE UNCHANGED vars
    <3>1. QED BY <2>2, StutteringPreservesHealthyStrongInv
  <2>3. QED BY <2>1, <2>2
<1>3. QED BY <1>1, <1>2, PTL

(***************************************************************************)
(* LIVENESS LIFTING ACROSS FAILURE                                         *)
(*                                                                         *)
(* Two facts carry every nominal leads-to into the full model: failure is  *)
(* absorbing, and a failure-free behavior projects onto the nominal        *)
(* world.  TLAPS cannot instantiate a lifting schema by citation, so each  *)
(* property's lifting cites these two facts and closes by PTL.             *)
(***************************************************************************)

THEOREM NextFailEntersFailedRegion ==
    TypeOK /\ NextFail => ~NotFailed'
<1>1. ASSUME TypeOK, NextFail
      PROVE ~NotFailed'
  <2>1. PICK rtId \in RuntimeIds : RuntimeFail(rtId)
    BY <1>1 DEF NextFail
  <2>2. runtime_state'[rtId] = "FAILED_UNQUIESCED"
    BY <1>1, <2>1, SMTT(90) DEF TypeOK, RuntimeFail
  <2>3. QED BY <2>1, <2>2 DEF NotFailed
<1>2. QED BY <1>1

THEOREM HealthyNextProjectsToSafe ==
    TypeOK /\ [Next]_vars /\ NotFailed /\ NotFailed' => [NextSafe]_vars
<1>1. ASSUME TypeOK, [Next]_vars, NotFailed, NotFailed'
      PROVE [NextSafe]_vars
  <2>1. CASE UNCHANGED vars
    <3>1. QED BY <2>1 DEF vars
  <2>2. CASE NextSafe
    <3>1. QED BY <2>2
  <2>3. CASE NextFail
    <3>1. ~NotFailed'
      BY <1>1, <2>3, NextFailEntersFailedRegion
    <3>2. QED BY <1>1, <3>1
  <2>4. CASE NextExplicitStutter
    <3>1. UNCHANGED vars BY <2>4, NextExplicitStutterFrame
    <3>2. QED BY <3>1
  <2>5. QED BY <1>1, <2>1, <2>2, <2>3, <2>4,
                  NextDecomposition DEF NextByFootprint
<1>2. QED BY <1>1

THEOREM AlwaysHealthyFullBehaviorProjectsToNominal ==
    /\ []IndInv
    /\ [][Next]_vars
    /\ []NotFailed
    => /\ []StrongInv
       /\ [][NextSafe]_vars
<1>1. ASSUME []IndInv, [][Next]_vars, []NotFailed
      PROVE []StrongInv /\ [][NextSafe]_vars
  <2>1. []StrongInv BY <1>1, PTL DEF IndInv
  <2>2. [][NextSafe]_vars
    BY <1>1, HealthyNextProjectsToSafe, PTL DEF IndInv
  <2>3. QED BY <2>1, <2>2
<1>2. QED BY <1>1

\* Failure is prefix-absorbing on full-model behaviors.
THEOREM FailedStateStepIsAbsorbing ==
    IndInv /\ ~NotFailed /\ [Next]_vars => ~NotFailed'
<1>1. ASSUME IndInv, ~NotFailed, [Next]_vars
      PROVE ~NotFailed'
  <2>1. CASE UNCHANGED vars
    <3>1. QED BY <1>1, <2>1 DEF vars, NotFailed
  <2>2. CASE Next
    <3>1. TypeOK /\ SingleRuntime BY <1>1 DEF IndInv
    <3>2. TypeOK' /\ SingleRuntime' /\ ~NotFailed'
      BY <1>1, <2>2, <3>1, FailedRegionPreserved
    <3>3. QED BY <3>2
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* Everything a lifted liveness proof consumes from Spec, packaged once:
\* the inductive invariant, the absorption of failure, the projection of a
\* failure-free behavior onto the nominal world, and the fairness block.
THEOREM SpecLiftingFacts ==
    Spec => /\ []IndInv
            /\ [](~NotFailed => ~NotFailed')
            /\ ([]NotFailed => []StrongInv /\ [][NextSafe]_vars)
            /\ Fairness
<1>1. Spec => []IndInv
    BY SafetyBehaviorEstablishesIndInv, PTL DEF Spec
<1>2. IndInv /\ ~NotFailed /\ [Next]_vars => ~NotFailed'
    BY FailedStateStepIsAbsorbing
<1>3. Spec => [](~NotFailed => ~NotFailed')
    BY <1>1, <1>2, PTL DEF Spec
<1>4. Spec => ([]NotFailed => []StrongInv /\ [][NextSafe]_vars)
    BY <1>1, AlwaysHealthyFullBehaviorProjectsToNominal, PTL DEF Spec
<1>5. QED BY <1>1, <1>3, <1>4, PTL DEF Spec

(***************************************************************************)
(* FAIRNESS PROGRESS: METADATA                                             *)
(*                                                                         *)
(* The WF1 pattern every liveness proof follows: an unless lemma, an       *)
(* enabledness lemma, an achieve lemma, then a PTL close against the       *)
(* single WF conjunct the argument consumes.                               *)
(***************************************************************************)

THEOREM MetadataWaitingEnablesDelivery ==
    \A cId \in CallIds :
        StrongInv /\ MetadataWaiting(cId) =>
            ENABLED <<DeliverInitialMetadata(cId)>>_vars
<1>1. QED
    BY ExpandENABLED
    DEF StrongInv, StructuralInv, TypeOK, CallStates, EventKinds, StatusKinds,
        MetadataWaiting, MetadataDelivered, DeliverInitialMetadata,
        vars, RuntimeVars, ChannelVars, CallVars,
        IsActiveCall, ActiveCallStates

THEOREM MetadataWaitingUnlessDone ==
    \A cId \in CallIds :
        StrongInv /\ MetadataWaiting(cId) /\ [NextSafe]_vars =>
            MetadataWaiting(cId)' \/ MetadataDelivered(cId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, CallStates, ChannelStates,
               EventKinds, StatusKinds,
               NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
               NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
               RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
               ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
               CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
               ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
               DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars,
               MetadataWaiting, MetadataDelivered, ActiveCallStates,
               IsActiveCall, IsUnusedCall, HasStatus, vars

THEOREM DeliverInitialMetadataCompletesMetadata ==
    \A cId \in CallIds :
        StrongInv /\ MetadataWaiting(cId) /\
        <<DeliverInitialMetadata(cId)>>_vars => MetadataDelivered(cId)'
<1>1. QED
    BY SMT DEF StrongInv, StructuralInv, TypeOK, EventKinds, StatusKinds,
               MetadataWaiting, MetadataDelivered, DeliverInitialMetadata,
               RuntimeVars, ChannelVars, CallVars, vars

\* The hypothesis names the single WF conjunct the argument consumes, not
\* the whole Fairness block, so the dependency stays readable at the call
\* site.
THEOREM MetadataProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => (MetadataWaiting(cId) ~> MetadataDelivered(cId))
<1>1. StrongInv /\ MetadataWaiting(cId) =>
          ENABLED <<DeliverInitialMetadata(cId)>>_vars
    BY MetadataWaitingEnablesDelivery
<1>2. StrongInv /\ MetadataWaiting(cId) /\ [NextSafe]_vars =>
          MetadataWaiting(cId)' \/ MetadataDelivered(cId)'
    BY MetadataWaitingUnlessDone
<1>3. StrongInv /\ MetadataWaiting(cId) /\
        <<DeliverInitialMetadata(cId)>>_vars => MetadataDelivered(cId)'
    BY DeliverInitialMetadataCompletesMetadata
<1>4. QED BY <1>1, <1>2, <1>3, PTL

(***************************************************************************)
(* FAIRNESS PROGRESS: CALL TERMINATION                                     *)
(*                                                                         *)
(* OPEN.  ReceiveStatus latches status_pending, which disables             *)
(* NetworkReceive and freezes the receive stream; PendingStatusPhase then  *)
(* names the three phases the call walks through to reach COMPLETED.  The     *)
(* backlog phase cites DeliveryProgressSafeFor at the frozen stream        *)
(* length instead of carrying its own induction.                           *)
(***************************************************************************)

(***************************************************************************)
(* FAIRNESS PROGRESS: RUNTIME SHUTDOWN                                     *)
(*                                                                         *)
(* OPEN.  RuntimeBeginShutdown already moved every open channel of the     *)
(* runtime to "closing"; neither ChannelCreate nor CallStart applies to a  *)
(* STOPPING runtime, so the set of channels to drain is fixed.  Each       *)
(* closing channel reaches the stable "closed" state by WF1; a finite      *)
(* conjunction over ChannelsOf(rtId) then enables RuntimeRelease.          *)
(***************************************************************************)

\* Each closing channel reaches the stable closed state on its own: a
\* plain WF1 on ChannelFinishClosing, independent of the runtime.
THEOREM ChannelClosesFor ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           => ((channel_state[chId] = "closing") ~>
                   (channel_state[chId] = "closed"))
<1>1. StrongInv /\ channel_state[chId] = "closing" /\ [NextSafe]_vars =>
          (channel_state[chId] = "closing")' \/
          (channel_state[chId] = "closed")'
    BY SMT DEF StrongInv, StructuralInv, TypeOK, ChannelStates,
        RuntimeStates, ChannelSentinelEquivalence, ChannelLifecycleInv,
        UsedChannels, ActiveChannelStates, ActiveChannels,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars
<1>2. StrongInv /\ channel_state[chId] = "closing" =>
          ENABLED <<ChannelFinishClosing(chId)>>_vars
  <2>1. TypeOK /\ channel_state[chId] = "closing" =>
            ENABLED <<ChannelFinishClosing(chId)>>_vars
    BY ExpandENABLED, SMT
    DEF TypeOK, ChannelStates, CallStates, EventKinds, StatusKinds,
        ChannelFinishClosing, HasStatus, IsActiveCall, ActiveCallStates,
        RuntimeVars, ChannelVars, CallVars, vars
  <2>2. QED BY <2>1 DEF StrongInv, StructuralInv
<1>3. StrongInv /\ channel_state[chId] = "closing" /\
          <<ChannelFinishClosing(chId)>>_vars =>
              (channel_state[chId] = "closed")'
    BY SMT DEF StrongInv, StructuralInv, TypeOK, ChannelStates,
        ChannelFinishClosing, RuntimeVars, ChannelVars, CallVars, vars
<1>4. QED BY <1>1, <1>2, <1>3, PTL

\* Once every channel of a STOPPING runtime is closed, RuntimeRelease is
\* enabled: ClosedChannelNoCalls supplies the terminal-calls guard.
THEOREM AllClosedEnablesRelease ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ StrongInv
           /\ ShutdownWaiting(rtId)
           /\ \A chId \in ChannelsOf(rtId) : channel_state[chId] = "closed"
           => ENABLED <<RuntimeRelease(rtId)>>_vars
<1>0. USE NoneNotInChannelIds, NoneNotInCallIds
      DEF NoneNotInChannelIds, NoneNotInCallIds
<1>1. /\ TypeOK
      /\ CallSentinelEquivalence
      /\ CallLifecycleInv
      => \A chId \in ChannelIds :
             channel_state[chId] = "closed" =>
                 \A cId \in CallsOf(chId) : call_state[cId] = "terminal"
  <2>1. SUFFICES ASSUME TypeOK, CallSentinelEquivalence, CallLifecycleInv,
                        NEW chId \in ChannelIds,
                        channel_state[chId] = "closed",
                        NEW cId \in CallIds,
                        call_channel[cId] = chId
                 PROVE call_state[cId] = "terminal"
    BY SMT DEF CallsOf
  <2>2. call_state[cId] # "none"
    BY <2>1 DEF CallSentinelEquivalence, IsUnusedCall
  <2>3. ~IsActiveCall(cId)
    BY <2>1, <2>2, SMT DEF CallLifecycleInv, UsedCalls, IsUnusedCall,
        IsActiveCall, ActiveCallStates, ActiveChannels, ActiveChannelStates
  <2>4. QED
    BY <2>1, <2>2, <2>3, SMT DEF TypeOK, CallStates, IsActiveCall,
        ActiveCallStates
<1>2. QED
    BY <1>1, ExpandENABLED, SMT
    DEF StrongInv, StructuralInv, TypeOK, RuntimeStates, ChannelStates,
        ShutdownWaiting, RuntimeRelease, ChannelsOf, CallsOf,
        RuntimeVars, ChannelVars, CallVars, vars

\* Structural facts of a stopping runtime: its channels are closing or
\* closed, channel ownership is frozen in both directions, and a closed
\* channel stays closed.
THEOREM StoppingChannelFacts ==
    ASSUME NEW rtId \in RuntimeIds, NEW c \in ChannelIds
    PROVE  /\ StrongInv /\ ShutdownWaiting(rtId) =>
               (channel_runtime[c] = rtId =>
                    channel_state[c] = "closing" \/
                    channel_state[c] = "closed")
           /\ StrongInv /\ ShutdownWaiting(rtId) /\
                  channel_runtime[c] # rtId /\ [NextSafe]_vars =>
               channel_runtime'[c] # rtId
           /\ StrongInv /\ channel_runtime[c] = rtId /\ [NextSafe]_vars =>
               channel_runtime'[c] = rtId
           /\ StrongInv /\ channel_state[c] = "closed" /\ [NextSafe]_vars =>
               channel_state'[c] = "closed"
<1>0. USE NoneNotInRuntimeIds, NoneNotInChannelIds
      DEF NoneNotInRuntimeIds, NoneNotInChannelIds
<1>1. StrongInv /\ ShutdownWaiting(rtId) =>
          (channel_runtime[c] = rtId =>
               channel_state[c] = "closing" \/
               channel_state[c] = "closed")
    BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, RuntimeStates,
        ChannelStates, ShutdownWaiting, ChannelSentinelEquivalence,
        ChannelLifecycleInv, UsedChannels
<1>2. StrongInv /\ ShutdownWaiting(rtId) /\
          channel_runtime[c] # rtId /\ [NextSafe]_vars =>
              channel_runtime'[c] # rtId
    BY SMT DEF StrongInv, StructuralInv, TypeOK, RuntimeStates,
        ChannelStates, ShutdownWaiting,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars
<1>3. StrongInv /\ channel_runtime[c] = rtId /\ [NextSafe]_vars =>
          channel_runtime'[c] = rtId
    BY SMT DEF StrongInv, StructuralInv, TypeOK, ChannelStates,
        ChannelSentinelEquivalence, UsedChannels,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars
<1>4. StrongInv /\ channel_state[c] = "closed" /\ [NextSafe]_vars =>
          channel_state'[c] = "closed"
    BY SMT DEF StrongInv, StructuralInv, TypeOK, ChannelStates,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars
<1>5. QED BY <1>1, <1>2, <1>3, <1>4

\* The same facts under [], proved in a WF-free context: LS4 refuses to
\* necessitate sibling facts when a non-boxed hypothesis such as a WF
\* disjunction is in scope.
THEOREM StoppingChannelFactsBoxed ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ []ShutdownWaiting(rtId)
           => /\ [](channel_runtime[chId] = rtId =>
                        channel_state[chId] = "closing" \/
                        channel_state[chId] = "closed")
              /\ [](channel_runtime[chId] # rtId =>
                        (channel_runtime[chId] # rtId)')
              /\ [](channel_runtime[chId] = rtId =>
                        (channel_runtime[chId] = rtId)')
              /\ [](channel_state[chId] = "closed" =>
                        (channel_state[chId] = "closed")')
<1>1. StrongInv /\ ShutdownWaiting(rtId) =>
          (channel_runtime[chId] = rtId =>
               channel_state[chId] = "closing" \/
               channel_state[chId] = "closed")
    BY StoppingChannelFacts
<1>2. StrongInv /\ ShutdownWaiting(rtId) /\ [NextSafe]_vars =>
          (channel_runtime[chId] # rtId =>
               (channel_runtime[chId] # rtId)')
    BY StoppingChannelFacts, SMT
<1>3. StrongInv /\ ShutdownWaiting(rtId) /\ [NextSafe]_vars =>
          (channel_runtime[chId] = rtId =>
               (channel_runtime[chId] = rtId)')
    BY StoppingChannelFacts, SMTT(90)
<1>4. StrongInv /\ ShutdownWaiting(rtId) /\ [NextSafe]_vars =>
          (channel_state[chId] = "closed" =>
               (channel_state[chId] = "closed")')
    BY StoppingChannelFacts, SMT
<1>5. QED BY <1>1, <1>2, <1>3, <1>4, PTL

\* Each channel of a stopping runtime eventually stays closed; channels of
\* other owners satisfy the goal vacuously, forever.
THEOREM ChannelSettlesFor ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           /\ []ShutdownWaiting(rtId)
           => <>[](channel_runtime[chId] = rtId =>
                       channel_state[chId] = "closed")
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(ChannelFinishClosing(chId)),
             []ShutdownWaiting(rtId)
      PROVE <>[](channel_runtime[chId] = rtId =>
                     channel_state[chId] = "closed")
  <2>11. [](channel_runtime[chId] = rtId =>
                channel_state[chId] = "closing" \/
                channel_state[chId] = "closed")
    BY <1>1, StoppingChannelFactsBoxed, PTL
  <2>12. [](channel_runtime[chId] # rtId =>
                (channel_runtime[chId] # rtId)')
    BY <1>1, StoppingChannelFactsBoxed, PTL
  <2>13. [](channel_runtime[chId] = rtId =>
                (channel_runtime[chId] = rtId)')
    BY <1>1, StoppingChannelFactsBoxed, PTL
  <2>14. [](channel_state[chId] = "closed" =>
                (channel_state[chId] = "closed")')
    BY <1>1, StoppingChannelFactsBoxed, PTL
  <2>21. /\ []StrongInv
         /\ [][NextSafe]_vars
         /\ WF_vars(ChannelFinishClosing(chId))
         => ((channel_state[chId] = "closing") ~>
                 (channel_state[chId] = "closed"))
    \* Instantiates the schema before any propositional-temporal step:
    \* LS4 cannot instantiate a cited ASSUME NEW theorem itself.
    BY ChannelClosesFor, IsaT(600)
  <2>2. (channel_state[chId] = "closing") ~>
            (channel_state[chId] = "closed")
    BY <1>1, <2>21, PTL
  <2>3. <>(channel_state[chId] = "closed") =>
            <>[](channel_runtime[chId] = rtId =>
                     channel_state[chId] = "closed")
    BY <2>12, <2>13, <2>14, PTL
  <2>4. [](channel_runtime[chId] = rtId =>
               <>(channel_state[chId] = "closed"))
    BY <2>11, <2>13, <2>2, PTL
  <2>5. QED BY <2>3, <2>4, PTL
<1>2. QED BY <1>1, PTL

\* Quantified weak fairness is invariant, so it can be consumed at any
\* suffix of the behavior.
THEOREM BoxedChannelFairness ==
    (\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
    <=> [](\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
<1>1. [](\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
      <=> \A chId \in ChannelIds : [](WF_vars(ChannelFinishClosing(chId)))
    OBVIOUS
<1>2. ASSUME NEW chId \in ChannelIds
      PROVE [](WF_vars(ChannelFinishClosing(chId)))
            <=> WF_vars(ChannelFinishClosing(chId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600)

\* Finite accumulation: the per-channel eventually-stable facts combine
\* into one eventually-always over the whole constant channel set.
THEOREM AllChannelsSettle ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  (\A chId \in ChannelIds :
                <>[](channel_runtime[chId] = rtId =>
                         channel_state[chId] = "closed"))
           => <>[](\A chId \in ChannelIds :
                       channel_runtime[chId] = rtId =>
                           channel_state[chId] = "closed")
<1>0. USE FiniteChannelIds DEF FiniteChannelIds
<1> DEFINE G(c) == channel_runtime[c] = rtId => channel_state[c] = "closed"
           K(c) == <>[]G(c)
           I(T) == (\A chId \in T : K(chId)) =>
                       <>[](\A chId \in T : G(chId))
<1>1. I({})
  <2>1. \A chId \in {} : G(chId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET ChannelIds, NEW x \in ChannelIds \ T
       PROVE <>[](\A chId \in T : G(chId)) /\ <>[]G(x) =>
                 <>[](\A chId \in T \cup {x} : G(chId))
  <2>1. (\A chId \in T : G(chId)) /\ G(x) =>
            (\A chId \in T \cup {x} : G(chId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET ChannelIds, IsFiniteSet(T), I(T),
             NEW x \in ChannelIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A chId \in T \cup {x} : K(chId)) =>
            (\A chId \in T : K(chId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A chId \in T : G(chId)) /\ <>[]G(x) =>
            <>[](\A chId \in T \cup {x} : G(chId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(ChannelIds)
    BY <1>1, <1>2, FS_Induction, IsaT(600)
<1>4. QED BY <1>3, Zenon DEF I

THEOREM ReleaseReaches ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  StrongInv /\ <<RuntimeRelease(rtId)>>_vars =>
               ShutdownReleased(rtId)'
<1>1. QED
    BY SMT DEF StrongInv, StructuralInv, TypeOK, RuntimeStates,
        RuntimeRelease, ShutdownReleased,
        RuntimeVars, ChannelVars, CallVars, vars

THEOREM ShutdownWaitingUnlessReleasedSafe ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  StrongInv /\ ShutdownWaiting(rtId) /\ [NextSafe]_vars =>
               ShutdownWaiting(rtId)' \/ ShutdownReleased(rtId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, RuntimeStates,
        ShutdownWaiting, ShutdownReleased,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars

\* Release-side facts under [], again in a WF-free context.
THEOREM ReleaseFactsBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           => /\ [](ShutdownWaiting(rtId) =>
                        ShutdownWaiting(rtId)' \/ ShutdownReleased(rtId)')
              /\ [](ShutdownWaiting(rtId) /\
                        (\A chId \in ChannelIds :
                             channel_runtime[chId] = rtId =>
                                 channel_state[chId] = "closed") =>
                        ENABLED <<RuntimeRelease(rtId)>>_vars)
              /\ [](<<RuntimeRelease(rtId)>>_vars =>
                        ShutdownReleased(rtId)')
<1>1. ASSUME []StrongInv, [][NextSafe]_vars
      PROVE /\ [](ShutdownWaiting(rtId) =>
                      ShutdownWaiting(rtId)' \/ ShutdownReleased(rtId)')
            /\ [](ShutdownWaiting(rtId) /\
                      (\A chId \in ChannelIds :
                           channel_runtime[chId] = rtId =>
                               channel_state[chId] = "closed") =>
                      ENABLED <<RuntimeRelease(rtId)>>_vars)
            /\ [](<<RuntimeRelease(rtId)>>_vars =>
                      ShutdownReleased(rtId)')
  <2>1. StrongInv /\ ShutdownWaiting(rtId) /\ [NextSafe]_vars =>
            ShutdownWaiting(rtId)' \/ ShutdownReleased(rtId)'
    BY ShutdownWaitingUnlessReleasedSafe, SMTT(90)
  <2>2. StrongInv /\ ShutdownWaiting(rtId) /\
            (\A chId \in ChannelIds :
                 channel_runtime[chId] = rtId =>
                     channel_state[chId] = "closed") =>
            ENABLED <<RuntimeRelease(rtId)>>_vars
    <3>1. StrongInv /\ ShutdownWaiting(rtId) /\
              (\A chId \in ChannelIds :
                   channel_runtime[chId] = rtId =>
                       channel_state[chId] = "closed") =>
              \A chId \in ChannelsOf(rtId) :
                  channel_state[chId] = "closed"
      BY SMTT(90) DEF ChannelsOf
    <3>2. QED BY <3>1, AllClosedEnablesRelease
  <2>3. StrongInv /\ <<RuntimeRelease(rtId)>>_vars =>
            ShutdownReleased(rtId)'
    BY ReleaseReaches, SMTT(90)
  <2>4. QED BY <1>1, <2>1, <2>2, <2>3, PTL
<1>2. QED BY <1>1, PTL

\* Under a behavior that keeps the runtime STOPPING forever, every
\* channel settles closed, RuntimeRelease becomes continuously enabled,
\* and weak fairness fires it.  Standalone so its statement can be
\* necessitated at any suffix by the final assembly.
THEOREM ShutdownPersistentWaitingReleases ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ [](\A chId \in ChannelIds :
                     WF_vars(ChannelFinishClosing(chId)))
           /\ []ShutdownWaiting(rtId)
           => <>ShutdownReleased(rtId)
<1> DEFINE G(c) == channel_runtime[c] = rtId => channel_state[c] = "closed"
           AllG == \A chId \in ChannelIds : G(chId)
           SW == ShutdownWaiting(rtId)
           SR == ShutdownReleased(rtId)
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(RuntimeRelease(rtId)),
             [](\A chId \in ChannelIds :
                    WF_vars(ChannelFinishClosing(chId))),
             []ShutdownWaiting(rtId)
      PROVE <>SR
  <2>1. \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
    BY <1>1, PTL
  <2>2. \A chId \in ChannelIds :
            /\ []StrongInv
            /\ [][NextSafe]_vars
            /\ WF_vars(ChannelFinishClosing(chId))
            /\ []SW
            => <>[]G(chId)
    BY ChannelSettlesFor, IsaT(600) DEF ShutdownWaiting
  <2>3. (\A chId \in ChannelIds : <>[]G(chId)) => <>[]AllG
    BY AllChannelsSettle, SMTT(90)
  <2>4. /\ []StrongInv
        /\ [][NextSafe]_vars
        /\ (\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
        /\ []SW
        => \A chId \in ChannelIds : <>[]G(chId)
    BY <2>2, IsaT(600)
  <2>5. /\ [](SW => SW' \/ SR')
        /\ [](SW /\ AllG => ENABLED <<RuntimeRelease(rtId)>>_vars)
        /\ [](<<RuntimeRelease(rtId)>>_vars => SR')
    BY <1>1, ReleaseFactsBoxed, IsaT(600)
  <2>6. QED BY <1>1, <2>1, <2>3, <2>4, <2>5, PTL
<1>2. QED BY <1>1, PTL

\* The persistent-waiting fact, pre-boxed in a context free of unboxed
\* hypotheses: inside the consuming proof the raw WF atoms block the
\* necessitation, so it happens here where every hypothesis is boxed.
THEOREM ShutdownPersistentWaitingReleasesBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ []WF_vars(RuntimeRelease(rtId))
           /\ [](\A chId \in ChannelIds :
                     WF_vars(ChannelFinishClosing(chId)))
           => []([]ShutdownWaiting(rtId) => <>ShutdownReleased(rtId))
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             []WF_vars(RuntimeRelease(rtId)),
             [](\A chId \in ChannelIds :
                    WF_vars(ChannelFinishClosing(chId)))
      PROVE  []([]ShutdownWaiting(rtId) => <>ShutdownReleased(rtId))
  <2>1. [][]StrongInv
    BY <1>1, PTL
  <2>2. [][][NextSafe]_vars
    BY <1>1, PTL
  <2>3. [][]WF_vars(RuntimeRelease(rtId))
    BY <1>1, PTL
  <2>4. [][](\A chId \in ChannelIds :
                 WF_vars(ChannelFinishClosing(chId)))
    BY <1>1, BoxedChannelFairness, PTL
  <2>45. /\ []StrongInv
         /\ [][NextSafe]_vars
         /\ WF_vars(RuntimeRelease(rtId))
         /\ [](\A chId \in ChannelIds :
                   WF_vars(ChannelFinishClosing(chId)))
         /\ []ShutdownWaiting(rtId)
         => <>ShutdownReleased(rtId)
    \* Instantiates the schema before the boxed modus ponens below.
    BY ShutdownPersistentWaitingReleases, IsaT(600)
  <2>5. QED
    BY <2>1, <2>2, <2>3, <2>4, <2>45, PTL
<1>2. QED
    BY <1>1

THEOREM ShutdownProgressSafeFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           => (ShutdownWaiting(rtId) ~> ShutdownReleased(rtId))
<1> DEFINE SW == ShutdownWaiting(rtId)
           SR == ShutdownReleased(rtId)
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(RuntimeRelease(rtId)),
             \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
      PROVE SW ~> SR
  <2>1. [](\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
    BY <1>1, BoxedChannelFairness, IsaT(600)
  <2>2. /\ [](SW => SW' \/ SR')
        /\ [](SW /\ (\A chId \in ChannelIds :
                         channel_runtime[chId] = rtId =>
                             channel_state[chId] = "closed") =>
                  ENABLED <<RuntimeRelease(rtId)>>_vars)
        /\ [](<<RuntimeRelease(rtId)>>_vars => SR')
    BY <1>1, ReleaseFactsBoxed, IsaT(600)
  <2>3. []WF_vars(RuntimeRelease(rtId))
    BY <1>1, PTL
  <2>4. /\ []StrongInv
        /\ [][NextSafe]_vars
        /\ WF_vars(RuntimeRelease(rtId))
        /\ [](\A chId \in ChannelIds :
                  WF_vars(ChannelFinishClosing(chId)))
        /\ []SW
        => <>SR
    BY ShutdownPersistentWaitingReleases, IsaT(600)
  <2>5. [](SW => SW' \/ SR') => [](SW => (<>SR \/ []SW))
    BY PTL
  <2>6. []([]SW => <>SR)
    BY <1>1, <2>1, <2>3, ShutdownPersistentWaitingReleasesBoxed, PTL
  <2>7. QED BY <2>2, <2>5, <2>6, PTL
<1>2. QED BY <1>1, PTL

\* The three state and action facts that turn the lifted disjunct
\* "released, or some runtime failed" into the strong per-slot form
\* EventualShutdown asks for.

THEOREM ShutdownWaitingImpliesNotFailed ==
    \A rtId \in RuntimeIds :
        SingleRuntime /\ ShutdownWaiting(rtId) => NotFailed
<1>1. QED
    BY SMTT(90) DEF SingleRuntime, ShutdownWaiting, NotFailed

THEOREM ShutdownWaitingUnlessSettled ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ ShutdownWaiting(rtId) /\ [Next]_vars =>
               ShutdownWaiting(rtId)' \/ ShutdownSettled(rtId)'
<1>1. ASSUME TypeOK, ShutdownWaiting(rtId), [Next]_vars
      PROVE ShutdownWaiting(rtId)' \/ ShutdownSettled(rtId)'
  <2>1. CASE NextSafeRuntimeOnly \/ NextSafeRuntimeChannel \/ NextFail
    BY <1>1, <2>1
    DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextFail,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease, RuntimeFail,
        TypeOK, RuntimeStates, ShutdownWaiting, ShutdownSettled
  <2>2. CASE NextSafeChannelOnly \/ NextSafeChannelCall \/ NextSafeCallOnly
    BY <1>1, <2>2, NextSafeChannelOnlyFrame, NextSafeChannelCallFrame,
       NextSafeCallOnlyFrame
    DEF RuntimeVars, ShutdownWaiting
  <2>3. CASE NextExplicitStutter
    BY <1>1, <2>3, NextExplicitStutterFrame DEF vars, ShutdownWaiting
  <2>4. CASE UNCHANGED vars
    BY <1>1, <2>4 DEF vars, ShutdownWaiting
  <2>5. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4,
       NextDecomposition DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

THEOREM ShutdownSettledStable ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ ShutdownSettled(rtId) /\ [Next]_vars =>
               ShutdownSettled(rtId)'
<1>1. ASSUME TypeOK, ShutdownSettled(rtId), [Next]_vars
      PROVE ShutdownSettled(rtId)'
  <2>1. CASE NextSafeRuntimeOnly \/ NextSafeRuntimeChannel \/ NextFail
    BY <1>1, <2>1
    DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextFail,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease, RuntimeFail,
        TypeOK, RuntimeStates, ShutdownSettled
  <2>2. CASE NextSafeChannelOnly \/ NextSafeChannelCall \/ NextSafeCallOnly
    BY <1>1, <2>2, NextSafeChannelOnlyFrame, NextSafeChannelCallFrame,
       NextSafeCallOnlyFrame
    DEF RuntimeVars, ShutdownSettled
  <2>3. CASE NextExplicitStutter
    BY <1>1, <2>3, NextExplicitStutterFrame DEF vars, ShutdownSettled
  <2>4. CASE UNCHANGED vars
    BY <1>1, <2>4 DEF vars, ShutdownSettled
  <2>5. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4,
       NextDecomposition, SMTT(90) DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

(***************************************************************************)
(* FAIRNESS PROGRESS: SUBMITTED REACHES THE WIRE                           *)
(*                                                                         *)
(* OPEN.  While no status is latched NetworkSend is enabled and the        *)
(* argument is well-founded on i - Len(sent); once status_pending holds,   *)
(* NetworkSend is disabled and TerminalProgressSafeFor supplies the        *)
(* IsTerminalCall escape disjunct.                                         *)
(***************************************************************************)

\* SubmitProgressSafeFor is proved at the end of the module: it cites
\* PendingProgressSafeFor, which itself builds on the Delivery descent.

(***************************************************************************)
(* FAIRNESS PROGRESS: RECEIVED REACHES THE CONSUMER                        *)
(*                                                                         *)
(* OPEN.  The only real induction of the liveness proofs: a well-founded   *)
(* descent on i - Len(delivered).  DeliverMessage needs INITIAL_METADATA   *)
(* first (MetadataProgressSafeFor), DeliverStatus cannot fire while a      *)
(* backlog remains, and a cancel reaches the escape disjunct directly.     *)
(***************************************************************************)

\* A waiting call is active: an unused call has an empty stream
\* (UnusedCallsAreEmpty makes the backlog impossible) and a terminal call
\* either carries CANCELLED (excluded) or delivered everything
\* (CompleteDelivery contradicts the backlog).
THEOREM DeliveryWaitingIsActive ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  StrongInv /\ DeliveryWaitingAt(cId, i) => IsActiveCall(cId)
<1>1. SUFFICES ASSUME StrongInv, DeliveryWaitingAt(cId, i)
               PROVE  IsActiveCall(cId)
    OBVIOUS
<1>2. received[cId] # <<>>
    BY <1>1, SMTT(90) DEF StrongInv, StructuralInv, TypeOK,
        PositiveNaturals, DeliveryWaitingAt
<1>3. ~IsUnusedCall(cId)
    BY <1>1, <1>2, SMT DEF StrongInv, StructuralInv,
        UnusedCallsAreEmpty, IsUnusedCall
<1>4. ~IsTerminalCall(cId)
    BY <1>1, <1>3, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, PositiveNaturals, DeliveryWaitingAt, CompleteDelivery,
        IsCancelled, HasStatus, UsedCalls, IsUnusedCall, IsTerminalCall
<1>5. QED
    BY <1>1, <1>3, <1>4, SMT DEF StrongInv, StructuralInv, TypeOK,
        CallStates, IsUnusedCall, IsTerminalCall, IsActiveCall,
        ActiveCallStates

THEOREM DeliveryWaitingUnlessDone ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  StrongInv /\ DeliveryWaitingAt(cId, i) /\ [NextSafe]_vars =>
               DeliveryWaitingAt(cId, i)' \/ DeliveryDoneAt(cId, i)'
<1>0. SUFFICES ASSUME StrongInv, DeliveryWaitingAt(cId, i), [NextSafe]_vars
               PROVE  DeliveryWaitingAt(cId, i)' \/ DeliveryDoneAt(cId, i)'
    OBVIOUS
<1>1. CASE NextSafeRuntimeOnly
    BY <1>0, <1>1, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease
<1>2. CASE NextSafeRuntimeChannel
    BY <1>0, <1>2, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeRuntimeChannel, RuntimeBeginShutdown
<1>3. CASE NextSafeChannelOnly
    BY <1>0, <1>3, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeChannelOnly, ChannelCreate, ChannelStartClosing
<1>4. CASE NextSafeChannelCall
    BY <1>0, <1>4, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeChannelCall, ChannelFinishClosing
<1>5. CASE NextSafeCallOnly
    BY <1>0, <1>5, SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeCallOnly, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus, CallCancel
<1>6. CASE UNCHANGED vars
    BY <1>0, <1>6, SMT DEF StrongInv, StructuralInv, MessageFlowInv,
        TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars
<1>7. QED
    BY <1>0, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF NextSafe

THEOREM DeliveryBacklogEnablesDeliver ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  StrongInv /\ DeliveryWaitingAt(cId, i) /\ MetadataDelivered(cId)
               => ENABLED <<DeliverMessage(cId)>>_vars
<1>1. StrongInv /\ DeliveryWaitingAt(cId, i) => IsActiveCall(cId)
    BY DeliveryWaitingIsActive
<1>2. QED
    BY <1>1, ExpandENABLED, SMTT(90)
    DEF StrongInv, StructuralInv, EventTraceInv, EventStreamShape,
        MessageFlowInv, TypeOK, CallStates, EventKinds, StatusKinds, PositiveNaturals,
        DeliveryWaitingAt, MetadataDelivered, CallLifecycleInv,
        TerminalStatusEquivalence, ReceivedPrefixOfDelivered, IsPrefix,
        CompleteDelivery, SubmittedPrefixOfSent,
        HasStatus, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        DeliverMessage, RuntimeVars, ChannelVars, CallVars, vars

\* One rung of the descent: with the head delivered and n + 1 positions
\* missing, fairness of DeliverMessage reaches the goal or leaves exactly
\* n positions missing.  Standalone so the induction below never mixes an
\* action formula with its induction hypothesis (an LS4 limitation).
THEOREM DeliveryDescentFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals, NEW n \in Nat
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverMessage(cId))
           => ((/\ DeliveryWaitingAt(cId, i)
                /\ MetadataDelivered(cId)
                /\ i - Len(delivered[cId]) = n + 1)
               ~> (\/ DeliveryDoneAt(cId, i)
                   \/ /\ DeliveryWaitingAt(cId, i)
                      /\ MetadataDelivered(cId)
                      /\ i - Len(delivered[cId]) = n))
<1> DEFINE P == /\ DeliveryWaitingAt(cId, i)
                /\ MetadataDelivered(cId)
                /\ i - Len(delivered[cId]) = n + 1
           Q == \/ DeliveryDoneAt(cId, i)
                \/ /\ DeliveryWaitingAt(cId, i)
                   /\ MetadataDelivered(cId)
                   /\ i - Len(delivered[cId]) = n
<1>1. StrongInv /\ P /\ [NextSafe]_vars => P' \/ Q'
  <2>0. SUFFICES ASSUME StrongInv, P, [NextSafe]_vars
                 PROVE  P' \/ Q'
      OBVIOUS
  <2>1. CASE NextSafeRuntimeOnly
      BY <2>0, <2>1, SMT DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease
  <2>2. CASE NextSafeRuntimeChannel
      BY <2>0, <2>2, SMT DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeRuntimeChannel, RuntimeBeginShutdown
  <2>3. CASE NextSafeChannelOnly
      BY <2>0, <2>3, SMT DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeChannelOnly, ChannelCreate, ChannelStartClosing
  <2>4. CASE NextSafeChannelCall
      BY <2>0, <2>4, SMT DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeChannelCall, ChannelFinishClosing
  <2>5. CASE NextSafeCallOnly
      BY <2>0, <2>5, SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars,
        NextSafeCallOnly, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus, CallCancel
  <2>6. CASE UNCHANGED vars
      BY <2>0, <2>6, SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, DeliveryWaitingAt, DeliveryDoneAt,
        MetadataDelivered, UnusedCallsAreEmpty, CallLifecycleInv,
        TerminalStatusEquivalence, CompleteDelivery,
        ReceivedPrefixOfDelivered, SubmittedPrefixOfSent, IsPrefix,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        RuntimeVars, ChannelVars, CallVars, vars
  <2>7. QED
      BY <2>0, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6 DEF NextSafe
<1>2. StrongInv /\ P => ENABLED <<DeliverMessage(cId)>>_vars
    BY DeliveryBacklogEnablesDeliver
<1>3. StrongInv /\ P /\ <<DeliverMessage(cId)>>_vars => Q'
    BY SMT DEF StrongInv, StructuralInv, MessageFlowInv, TypeOK,
        CallStates, EventKinds, StatusKinds, PositiveNaturals,
        DeliveryWaitingAt, DeliveryDoneAt, MetadataDelivered,
        ReceivedPrefixOfDelivered, IsPrefix, CompleteDelivery,
        CallLifecycleInv, TerminalStatusEquivalence,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        DeliverMessage, RuntimeVars, ChannelVars, CallVars, vars
<1>4. QED BY <1>1, <1>2, <1>3, PTL

\* The two arithmetic bridges of the induction, pre-boxed in a context
\* free of unboxed hypotheses: inside the induction step the hypothesis
\* Ind(n) is not a boxed formula, which blocks necessitation there.
THEOREM DeliveryBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals, NEW n \in Nat
    PROVE  LET P == DeliveryWaitingAt(cId, i) /\ MetadataDelivered(cId)
               M == i - Len(delivered[cId])
           IN  []StrongInv =>
                   /\ [](P /\ M <= n + 1 =>
                             (P /\ M <= n) \/ (P /\ M = n + 1))
                   /\ [](P /\ M = n => P /\ M <= n)
<1> DEFINE P == DeliveryWaitingAt(cId, i) /\ MetadataDelivered(cId)
           M == i - Len(delivered[cId])
<1>1. StrongInv /\ P /\ M <= n + 1 =>
          (P /\ M <= n) \/ (P /\ M = n + 1)
    BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        DeliveryWaitingAt, MetadataDelivered
<1>2. StrongInv /\ P /\ M = n => P /\ M <= n
    BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        DeliveryWaitingAt, MetadataDelivered
<1>3. QED BY <1>1, <1>2, PTL

THEOREM DeliveryProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))
<1> DEFINE P == DeliveryWaitingAt(cId, i) /\ MetadataDelivered(cId)
           M == i - Len(delivered[cId])
\* Phase one: the head arrives while the backlog persists.
<1>1. StrongInv /\ DeliveryWaitingAt(cId, i) =>
          MetadataWaiting(cId) \/ MetadataDelivered(cId)
  <2>1. StrongInv /\ DeliveryWaitingAt(cId, i) => IsActiveCall(cId)
    BY DeliveryWaitingIsActive
  <2>2. QED
    BY <2>1, SMT DEF StrongInv, StructuralInv, TypeOK, EventKinds, StatusKinds,
        MetadataWaiting, MetadataDelivered
<1>2. StrongInv /\ DeliveryWaitingAt(cId, i) /\ [NextSafe]_vars =>
          DeliveryWaitingAt(cId, i)' \/ DeliveryDoneAt(cId, i)'
    BY DeliveryWaitingUnlessDone
<1>3. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(DeliverInitialMetadata(cId))
      => (MetadataWaiting(cId) ~> MetadataDelivered(cId))
    BY MetadataProgressSafeFor, IsaT(600)
<1>4. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(DeliverInitialMetadata(cId))
      => (DeliveryWaitingAt(cId, i) ~> (P \/ DeliveryDoneAt(cId, i)))
    BY <1>1, <1>2, <1>3, PTL
\* Phase two: induction on an upper bound of the number of missing
\* positions.  The bound i is a rigid constant, so no existential
\* elimination is ever needed: the induction is instantiated at i.
<1>5. TypeOK /\ DeliveryWaitingAt(cId, i) => M >= 1 /\ M <= i
    BY SMT DEF TypeOK, PositiveNaturals, DeliveryWaitingAt
<1> DEFINE Ind(n) == /\ []StrongInv
                     /\ [][NextSafe]_vars
                     /\ WF_vars(DeliverMessage(cId))
                     => ((P /\ M <= n) ~> DeliveryDoneAt(cId, i))
<1>6. Ind(0)
  <2>1. TypeOK => ~(P /\ M <= 0)
    BY <1>5, SMT DEF TypeOK, PositiveNaturals, DeliveryWaitingAt
  <2>2. QED BY <2>1, PTL DEF StrongInv, StructuralInv
<1>7. ASSUME NEW n \in Nat, Ind(n)
      PROVE Ind(n + 1)
  <2>s. SUFFICES ASSUME []StrongInv,
                        [][NextSafe]_vars,
                        WF_vars(DeliverMessage(cId))
        PROVE (P /\ M <= n + 1) ~> DeliveryDoneAt(cId, i)
    BY IsaT(600)
  <2>2. (P /\ M = n + 1) ~> (DeliveryDoneAt(cId, i) \/ (P /\ M = n))
    BY <2>s, DeliveryDescentFor, PTL
  <2>4. (P /\ M <= n) ~> DeliveryDoneAt(cId, i)
    BY <1>7, <2>s, PTL
  <2>5. [](P /\ M <= n + 1 => (P /\ M <= n) \/ (P /\ M = n + 1))
    BY <2>s, DeliveryBoundSplit, PTL
  <2>6. [](P /\ M = n => P /\ M <= n)
    BY <2>s, DeliveryBoundSplit, PTL
  <2>7. QED BY <2>2, <2>4, <2>5, <2>6, PTL
<1> HIDE DEF Ind
<1>8. \A n \in Nat : Ind(n)
    BY <1>6, <1>7, NatInduction, IsaT(600)
<1>9. Ind(i)
    BY <1>8 DEF PositiveNaturals
<1>10. QED BY <1>4, <1>5, <1>9, PTL DEF Ind, StrongInv, StructuralInv

\* An active call with a latched status only stops being so by reaching a
\* terminal event, and while latched its received stream is frozen
\* (NetworkReceive requires ~status_pending).
THEOREM PendingUnlessDone ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ StrongInv
           /\ IsActiveCall(cId) /\ status_pending[cId]
           /\ [NextSafe]_vars
           => \/ /\ IsActiveCall(cId)' /\ status_pending'[cId]
                 /\ received'[cId] = received[cId]
              \/ HasStatus(cId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered, SubmittedPrefixOfSent,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars

\* Same unless fact without the frozen length, for the phases that do not
\* track it.
THEOREM PendingUnlessDonePlain ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ StrongInv
           /\ IsActiveCall(cId) /\ status_pending[cId]
           /\ [NextSafe]_vars
           => \/ IsActiveCall(cId)' /\ status_pending'[cId]
              \/ HasStatus(cId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered, SubmittedPrefixOfSent,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars

\* With everything received also delivered, the latched status is ready to
\* go out.
THEOREM ReadyEnablesDeliverStatus ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ IsActiveCall(cId)
           /\ status_pending[cId]
           /\ Len(events_delivered[cId]) >= 1
           /\ events_delivered[cId][1] = "INITIAL_METADATA"
           /\ ~HasStatus(cId)
           /\ Len(delivered[cId]) = Len(received[cId])
           => ENABLED <<DeliverStatus(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF TypeOK, ChannelStates, CallStates, EventKinds, StatusKinds, DeliverStatus,
        HasStatus, IsActiveCall, ActiveCallStates,
        RuntimeVars, ChannelVars, CallVars, vars

THEOREM DeliverStatusReaches ==
    ASSUME NEW cId \in CallIds
    PROVE  StrongInv /\ <<DeliverStatus(cId)>>_vars => HasStatus(cId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, EventKinds, StatusKinds, CallStates,
        DeliverStatus, HasStatus, RuntimeVars, ChannelVars, CallVars, vars

\* The guards of ReadyEnablesDeliverStatus, derived from the invariant.
THEOREM ReadyGuardsFromInv ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ StrongInv
           /\ IsActiveCall(cId)
           /\ status_pending[cId]
           /\ MetadataDelivered(cId)
           /\ ReceiveDebt(cId) = 0
           => /\ TypeOK
              /\ Len(events_delivered[cId]) >= 1
              /\ events_delivered[cId][1] = "INITIAL_METADATA"
              /\ ~HasStatus(cId)
              /\ Len(delivered[cId]) = Len(received[cId])
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, EventTraceInv, EventStreamShape,
        MessageFlowInv, ReceivedPrefixOfDelivered, IsPrefix, TypeOK,
        CallStates, EventKinds, StatusKinds, CallLifecycleInv, TerminalStatusEquivalence,
        MetadataDelivered, ReceiveDebt, HasStatus, UsedCalls,
        ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall

\* The shared heart of Terminal and Submit: once a status is latched, the
\* consumer chain alone drains the head, the frozen backlog, then the
\* status itself.  Phase two eliminates the state-dependent stream length
\* with the for-all-n leads-to recipe; each fixed n cites the Delivery
\* descent at the rigid index n.
THEOREM PendingProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => ((IsActiveCall(cId) /\ status_pending[cId]) ~> HasStatus(cId))
<1> DEFINE W == IsActiveCall(cId) /\ status_pending[cId]
           R == HasStatus(cId)
           MD == MetadataDelivered(cId)
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(DeliverInitialMetadata(cId)),
             WF_vars(DeliverMessage(cId)),
             WF_vars(DeliverStatus(cId))
      PROVE W ~> R
  <2>1. StrongInv /\ W /\ [NextSafe]_vars => W' \/ R'
    BY PendingUnlessDonePlain
  \* Phase one: the response head arrives.
  <2>2. StrongInv /\ W => MetadataWaiting(cId) \/ MD
    BY SMT DEF StrongInv, StructuralInv, TypeOK, EventKinds, StatusKinds,
        MetadataWaiting, MetadataDelivered, IsActiveCall, ActiveCallStates
  <2>3. /\ []StrongInv
        /\ [][NextSafe]_vars
        /\ WF_vars(DeliverInitialMetadata(cId))
        => (MetadataWaiting(cId) ~> MD)
    BY MetadataProgressSafeFor, IsaT(600)
  <2>4. W ~> ((W /\ MD) \/ R)
    BY <1>1, <2>1, <2>2, <2>3, PTL
  \* Phase two: the receive stream is frozen while the status is latched;
  \* at each fixed positive length i the Delivery descent empties the
  \* backlog, and a zero-length stream has no backlog at all.
  <2>5. ASSUME NEW i \in PositiveNaturals
        PROVE (W /\ MD /\ Len(received[cId]) = i) ~>
                  ((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R)
    <3> DEFINE P == W /\ MD /\ Len(received[cId]) = i
    <3>1. StrongInv /\ P /\ [NextSafe]_vars => P' \/ R'
      <4>1. StrongInv /\ MD /\ [NextSafe]_vars /\ IsActiveCall(cId)' => MD'
        BY SMT DEF StrongInv, StructuralInv, TypeOK, CallStates,
            EventKinds, StatusKinds, MetadataDelivered,
            NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
            NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
            RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
            ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
            CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
            ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
            DeliverStatus, CallCancel, IsActiveCall, ActiveCallStates,
            IsUnusedCall, HasStatus,
            RuntimeVars, ChannelVars, CallVars, vars
      <4>2. QED BY <4>1, PendingUnlessDone, SMTT(90)
    <3>2. StrongInv /\ P =>
              \/ W /\ MD /\ ReceiveDebt(cId) = 0
              \/ DeliveryWaitingAt(cId, i)
      BY StatusKindsExpansion, SMTT(90)
      DEF StrongInv, StructuralInv, MessageFlowInv,
          ReceivedPrefixOfDelivered, IsPrefix, CompleteDelivery, TypeOK,
          CallStates, PositiveNaturals, DeliveryWaitingAt, ReceiveDebt,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall, MetadataDelivered
    <3>3. /\ []StrongInv
          /\ [][NextSafe]_vars
          /\ WF_vars(DeliverInitialMetadata(cId))
          /\ WF_vars(DeliverMessage(cId))
          => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))
      BY DeliveryProgressSafeFor, IsaT(600)
    <3>4. StrongInv /\ P /\ DeliveryDoneAt(cId, i) =>
              (W /\ MD /\ ReceiveDebt(cId) = 0) \/ R
      BY StatusKindsExpansion, SMT
      DEF StrongInv, StructuralInv, MessageFlowInv,
          ReceivedPrefixOfDelivered, IsPrefix, TypeOK, CallStates,
          PositiveNaturals, DeliveryDoneAt, ReceiveDebt,
          CallLifecycleInv, TerminalStatusEquivalence, HasStatus,
          IsCancelled, UsedCalls, ActiveCallStates, IsUnusedCall,
          IsActiveCall, IsTerminalCall
    <3>5. DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i)
      BY <1>1, <3>3, PTL
    <3>6. QED BY <1>1, <3>1, <3>2, <3>4, <3>5, PTL
  \* Existential elimination over the frozen positive length; the empty
  \* stream reaches the ready state without waiting.
  <2>6. (W /\ MD) ~> ((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R)
    <3>1. StrongInv /\ W /\ MD =>
              \/ W /\ MD /\ ReceiveDebt(cId) = 0
              \/ \E i \in PositiveNaturals : Len(received[cId]) = i
      BY SMT DEF StrongInv, StructuralInv, MessageFlowInv,
          ReceivedPrefixOfDelivered, IsPrefix, TypeOK, CallStates,
          PositiveNaturals, ReceiveDebt, UsedCalls, IsUnusedCall,
          IsActiveCall, ActiveCallStates, MetadataDelivered
    <3>2. \A i \in PositiveNaturals :
              [](/\ W /\ MD /\ Len(received[cId]) = i
                 => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R))
      <4>1. ASSUME NEW i \in PositiveNaturals
            PROVE [](/\ W /\ MD /\ Len(received[cId]) = i
                     => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R))
        BY <2>5, PTL
      <4>2. QED BY <4>1
    <3>3. (\A i \in PositiveNaturals :
              [](/\ W /\ MD /\ Len(received[cId]) = i
                 => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R)))
          => [](\A i \in PositiveNaturals :
                  /\ W /\ MD /\ Len(received[cId]) = i
                  => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R))
      OBVIOUS
    <3>4. (\A i \in PositiveNaturals :
              /\ W /\ MD /\ Len(received[cId]) = i
              => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R))
          => (/\ W /\ MD
              /\ (\E i \in PositiveNaturals : Len(received[cId]) = i)
              => <>((W /\ MD /\ ReceiveDebt(cId) = 0) \/ R))
      OBVIOUS
    <3>5. QED BY <1>1, <3>1, <3>2, <3>3, <3>4, PTL
  \* Phase three: the status goes out.
  <2>7. (W /\ MD /\ ReceiveDebt(cId) = 0) ~> R
    <3> DEFINE P == W /\ MD /\ ReceiveDebt(cId) = 0
    <3>1. StrongInv /\ P /\ [NextSafe]_vars => P' \/ R'
      <4>1. StrongInv /\ MD /\ [NextSafe]_vars /\ IsActiveCall(cId)' => MD'
        BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, CallStates,
            EventKinds, StatusKinds, MetadataDelivered,
            NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
            NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
            RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
            ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
            CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
            ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
            DeliverStatus, CallCancel, IsActiveCall, ActiveCallStates,
            IsUnusedCall, HasStatus,
            RuntimeVars, ChannelVars, CallVars, vars
      <4>2. StrongInv /\ P /\ [NextSafe]_vars =>
                (W /\ ReceiveDebt(cId) = 0)' \/ R'
        BY SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
            EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
            UnusedCallsAreEmpty, CallLifecycleInv,
            TerminalStatusEquivalence, CompleteDelivery,
            ReceivedPrefixOfDelivered, SubmittedPrefixOfSent,
            IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
            IsUnusedCall, IsActiveCall, IsTerminalCall, ReceiveDebt,
            MetadataDelivered,
            NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
            NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
            RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
            ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
            CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
            ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
            DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars,
            vars
      <4>3. QED BY <4>1, <4>2
    <3>2. StrongInv /\ P => ENABLED <<DeliverStatus(cId)>>_vars
      BY ReadyGuardsFromInv, ReadyEnablesDeliverStatus
    <3>3. StrongInv /\ P /\ <<DeliverStatus(cId)>>_vars => R'
      BY DeliverStatusReaches
    <3>4. QED BY <1>1, <3>1, <3>2, <3>3, PTL
  <2>8. QED BY <1>1, <2>1, <2>4, <2>6, <2>7, PTL
<1>2. QED BY <1>1, PTL

\* An active call is either still waiting for its status to be observed
\* (ReceiveStatus is enabled) or already carries one.
THEOREM ActiveEnablesReceiveStatus ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsActiveCall(cId) /\ ~status_pending[cId]
               => ENABLED <<ReceiveStatus(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF TypeOK, ChannelStates, CallStates, EventKinds, StatusKinds, ReceiveStatus,
        IsActiveCall, ActiveCallStates,
        RuntimeVars, ChannelVars, CallVars, vars

THEOREM ActiveUnlessDone ==
    ASSUME NEW cId \in CallIds
    PROVE  StrongInv /\ IsActiveCall(cId) /\ [NextSafe]_vars =>
               IsActiveCall(cId)' \/ HasStatus(cId)'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered, SubmittedPrefixOfSent,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars

THEOREM ReceiveStatusLatches ==
    ASSUME NEW cId \in CallIds
    PROVE  StrongInv /\ IsActiveCall(cId) /\ <<ReceiveStatus(cId)>>_vars =>
               (IsActiveCall(cId) /\ status_pending[cId])'
<1>1. QED
    BY SMTT(90) DEF StrongInv, StructuralInv, TypeOK, CallStates, EventKinds, StatusKinds,
        ReceiveStatus, IsActiveCall, ActiveCallStates,
        RuntimeVars, ChannelVars, CallVars, vars

THEOREM TerminalProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(ReceiveStatus(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (TerminalWaiting(cId) ~> TerminalReached(cId))
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(ReceiveStatus(cId)),
             WF_vars(DeliverInitialMetadata(cId)),
             WF_vars(DeliverMessage(cId)),
             WF_vars(DeliverStatus(cId))
      PROVE TerminalWaiting(cId) ~> TerminalReached(cId)
  <2>1. StrongInv /\ TerminalWaiting(cId) /\ [NextSafe]_vars =>
            TerminalWaiting(cId)' \/ TerminalReached(cId)'
    BY ActiveUnlessDone DEF TerminalWaiting, TerminalReached
  \* WF1 on ReceiveStatus latches the status; already-latched and
  \* already-reached states pass through.
  <2>2. StrongInv /\ (TerminalWaiting(cId) /\ ~status_pending[cId]) =>
            ENABLED <<ReceiveStatus(cId)>>_vars
    BY ActiveEnablesReceiveStatus
       DEF StrongInv, StructuralInv, TerminalWaiting
  <2>3. StrongInv /\ (TerminalWaiting(cId) /\ ~status_pending[cId]) /\
            [NextSafe]_vars =>
              (TerminalWaiting(cId) /\ ~status_pending[cId])' \/
              (TerminalWaiting(cId) /\ status_pending[cId])' \/
              TerminalReached(cId)'
    BY ActiveUnlessDone DEF TerminalWaiting, TerminalReached
  <2>4. StrongInv /\ (TerminalWaiting(cId) /\ ~status_pending[cId]) /\
            <<ReceiveStatus(cId)>>_vars =>
              (TerminalWaiting(cId) /\ status_pending[cId])'
    BY ReceiveStatusLatches DEF TerminalWaiting
  <2>5. (TerminalWaiting(cId) /\ ~status_pending[cId]) ~>
            ((TerminalWaiting(cId) /\ status_pending[cId]) \/
             TerminalReached(cId))
    BY <1>1, <2>2, <2>3, <2>4, PTL
  <2>6. /\ []StrongInv
        /\ [][NextSafe]_vars
        /\ WF_vars(DeliverInitialMetadata(cId))
        /\ WF_vars(DeliverMessage(cId))
        /\ WF_vars(DeliverStatus(cId))
        => ((IsActiveCall(cId) /\ status_pending[cId]) ~> HasStatus(cId))
    BY IsaT(180), PendingProgressSafeFor
  <2>7. QED BY <1>1, <2>1, <2>5, <2>6, PTL
       DEF TerminalWaiting, TerminalReached
<1>2. QED BY <1>1, PTL


(***************************************************************************)
(* FAIRNESS PROGRESS: SUBMITTED REACHES THE WIRE (proof)                   *)
(*                                                                         *)
(* While no status is latched the wire drains submitted positions, one    *)
(* NetworkSend at a time; the moment a status latches, NetworkSend is     *)
(* disabled for good and PendingProgressSafeFor carries the call to a     *)
(* terminal event, which is the escape disjunct of the property.          *)
(***************************************************************************)

THEOREM SubmitWaitingIsActive ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  StrongInv /\ SubmitWaitingAt(cId, i) => IsActiveCall(cId)
<1>1. QED
    BY SMT DEF StrongInv, StructuralInv, MessageFlowInv, TypeOK,
        CallStates, PositiveNaturals, SubmitWaitingAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        SubmittedPrefixOfSent, IsPrefix,
        HasStatus, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall

THEOREM BacklogEnablesNetworkSend ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ IsActiveCall(cId)
           /\ ~status_pending[cId]
           /\ Len(sent[cId]) < Len(submitted[cId])
           => ENABLED <<NetworkSend(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF TypeOK, ChannelStates, CallStates, EventKinds, StatusKinds, NetworkSend,
        IsActiveCall, ActiveCallStates,
        RuntimeVars, ChannelVars, CallVars, vars

\* One rung of the send descent: with n + 1 positions missing and no
\* status latched, fairness of NetworkSend reaches position i, leaves n
\* positions missing, or a status latches.
THEOREM SubmitDescentFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals, NEW n \in Nat
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(NetworkSend(cId))
           => ((/\ SubmitWaitingAt(cId, i)
                /\ ~status_pending[cId]
                /\ i - Len(sent[cId]) = n + 1)
               ~> (\/ SubmitDoneAt(cId, i)
                   \/ /\ SubmitWaitingAt(cId, i)
                      /\ ~status_pending[cId]
                      /\ i - Len(sent[cId]) = n
                   \/ /\ SubmitWaitingAt(cId, i)
                      /\ status_pending[cId]))
<1> DEFINE P == /\ SubmitWaitingAt(cId, i)
                /\ ~status_pending[cId]
                /\ i - Len(sent[cId]) = n + 1
           Q == \/ SubmitDoneAt(cId, i)
                \/ /\ SubmitWaitingAt(cId, i)
                   /\ ~status_pending[cId]
                   /\ i - Len(sent[cId]) = n
                \/ /\ SubmitWaitingAt(cId, i)
                   /\ status_pending[cId]
<1>1. StrongInv /\ P /\ [NextSafe]_vars => P' \/ Q'
    BY SMTT(90) DEF StrongInv, StructuralInv, MessageFlowInv, EventTraceInv,
        EventStreamShape, TypeOK, CallStates, ChannelStates, EventKinds, StatusKinds,
        PositiveNaturals, SubmitWaitingAt, SubmitDoneAt,
        UnusedCallsAreEmpty, CallLifecycleInv, TerminalStatusEquivalence,
        CompleteDelivery, ReceivedPrefixOfDelivered, SubmittedPrefixOfSent,
        IsPrefix, HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        NextSafe, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, CallCancel, RuntimeVars, ChannelVars, CallVars, vars
<1>2. StrongInv /\ P => ENABLED <<NetworkSend(cId)>>_vars
  <2>1. StrongInv /\ P =>
            /\ TypeOK
            /\ IsActiveCall(cId)
            /\ ~status_pending[cId]
            /\ Len(sent[cId]) < Len(submitted[cId])
    BY SubmitWaitingIsActive, SMT
    DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        SubmitWaitingAt
  <2>2. QED BY <2>1, BacklogEnablesNetworkSend
<1>3. StrongInv /\ P /\ <<NetworkSend(cId)>>_vars => Q'
    BY SMT DEF StrongInv, StructuralInv, MessageFlowInv, TypeOK,
        CallStates, EventKinds, StatusKinds, PositiveNaturals,
        SubmitWaitingAt, SubmitDoneAt, SubmittedPrefixOfSent, IsPrefix,
        CallLifecycleInv, TerminalStatusEquivalence,
        HasStatus, IsCancelled, UsedCalls, ActiveCallStates,
        IsUnusedCall, IsActiveCall, IsTerminalCall,
        NetworkSend, RuntimeVars, ChannelVars, CallVars, vars
<1>4. QED BY <1>1, <1>2, <1>3, PTL

\* The arithmetic bridges of the send induction, pre-boxed.
THEOREM SubmitBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals, NEW n \in Nat
    PROVE  LET P == SubmitWaitingAt(cId, i) /\ ~status_pending[cId]
               M == i - Len(sent[cId])
           IN  []StrongInv =>
                   /\ [](P /\ M <= n + 1 =>
                             (P /\ M <= n) \/ (P /\ M = n + 1))
                   /\ [](P /\ M = n => P /\ M <= n)
<1> DEFINE P == SubmitWaitingAt(cId, i) /\ ~status_pending[cId]
           M == i - Len(sent[cId])
<1>1. StrongInv /\ P /\ M <= n + 1 =>
          (P /\ M <= n) \/ (P /\ M = n + 1)
    BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        SubmitWaitingAt
<1>2. StrongInv /\ P /\ M = n => P /\ M <= n
    BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        SubmitWaitingAt
<1>3. QED BY <1>1, <1>2, PTL

THEOREM SubmitProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(NetworkSend(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i))
<1> DEFINE P == SubmitWaitingAt(cId, i) /\ ~status_pending[cId]
           M == i - Len(sent[cId])
           L == SubmitWaitingAt(cId, i) /\ status_pending[cId]
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(NetworkSend(cId)),
             WF_vars(DeliverInitialMetadata(cId)),
             WF_vars(DeliverMessage(cId)),
             WF_vars(DeliverStatus(cId))
      PROVE SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i)
  \* The latched branch: the call reaches a terminal event, which is the
  \* escape disjunct of the property.
  <2>1. L ~> SubmitDoneAt(cId, i)
    <3>1. StrongInv /\ L => IsActiveCall(cId) /\ status_pending[cId]
      BY SubmitWaitingIsActive
    <3>2. /\ []StrongInv
          /\ [][NextSafe]_vars
          /\ WF_vars(DeliverInitialMetadata(cId))
          /\ WF_vars(DeliverMessage(cId))
          /\ WF_vars(DeliverStatus(cId))
          => ((IsActiveCall(cId) /\ status_pending[cId]) ~> HasStatus(cId))
      BY IsaT(180), PendingProgressSafeFor
    <3>3. StrongInv /\ HasStatus(cId) => SubmitDoneAt(cId, i)
      BY SMT DEF StrongInv, StructuralInv, TypeOK, CallStates, EventKinds, StatusKinds,
          SubmitDoneAt, UnusedCallsAreEmpty, CallLifecycleInv,
          TerminalStatusEquivalence, HasStatus, UsedCalls,
          ActiveCallStates, IsUnusedCall, IsActiveCall, IsTerminalCall
    <3>4. QED BY <1>1, <3>1, <3>2, <3>3, PTL
  \* The free branch: induction on the missing-position bound.
  <2> DEFINE Ind(n) == (P /\ M <= n) ~> SubmitDoneAt(cId, i)
  <2>2. Ind(0)
    <3>1. StrongInv => ~(P /\ M <= 0)
      BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
          SubmitWaitingAt
    <3>2. QED BY <1>1, <3>1, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE Ind(n + 1)
    <3>1. (/\ SubmitWaitingAt(cId, i)
           /\ ~status_pending[cId]
           /\ i - Len(sent[cId]) = n + 1)
          ~> (\/ SubmitDoneAt(cId, i)
              \/ /\ SubmitWaitingAt(cId, i)
                 /\ ~status_pending[cId]
                 /\ i - Len(sent[cId]) = n
              \/ L)
      BY <1>1, SubmitDescentFor, PTL
    <3>2. [](P /\ M <= n + 1 => (P /\ M <= n) \/ (P /\ M = n + 1))
      BY <1>1, SubmitBoundSplit, PTL
    <3>3. [](P /\ M = n => P /\ M <= n)
      BY <1>1, SubmitBoundSplit, PTL
    <3>4. L ~> SubmitDoneAt(cId, i)
      BY <2>1
    <3>5. (P /\ M = n + 1) ~> SubmitDoneAt(cId, i)
      BY <2>3, <3>1, <3>3, <3>4, PTL
    <3>6. QED BY <2>3, <3>2, <3>5, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(180)
  <2>5. Ind(i)
    BY <2>4 DEF PositiveNaturals
  <2>6. StrongInv /\ SubmitWaitingAt(cId, i) => (P /\ M <= i) \/ L
    BY SMT DEF StrongInv, StructuralInv, TypeOK, PositiveNaturals,
        SubmitWaitingAt
  <2>7. QED BY <1>1, <2>1, <2>5, <2>6, PTL DEF Ind
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* TOP-LEVEL ASSEMBLY                                                      *)
(***************************************************************************)

(***************************************************************************)
(* SAFETY                                                                  *)
(***************************************************************************)

THEOREM SafetyTheorem == Spec => []SafetyInvariant
<1>1. Init => IndInv
    BY InitEstablishesIndInv
<1>2. IndInv /\ [Next]_vars => IndInv'
    BY IndInvPreserved
<1>3. IndInv => SafetyInvariant
    BY IndInvImpliesSafetyInvariant
<1>4. QED
    BY <1>1, <1>2, <1>3, PTL DEF Spec

(***************************************************************************)
(* FAIRNESS REQUIREMENTS, ONE THEOREM PER GUARANTEE                        *)
(*                                                                         *)
(* Each theorem states a guarantee in the failure-free world against the   *)
(* WF conjuncts it actually consumes, listed one by one.  Read a single    *)
(* theorem to answer: what has to keep happening for this guarantee to     *)
(* hold, and what does not matter to it?                                   *)
(*                                                                         *)
(* Level 0 has no demand variable, so every Deliver* action is guarded by  *)
(* availability alone.  A WF on one of them is a continuing obligation of  *)
(* the caller, not a promise of the library.  The trigger of each          *)
(* guarantee (CallStart, SendMessage, RuntimeBeginShutdown) is             *)
(* deliberately unfair and sits in the antecedent of the leads-to: the     *)
(* library promises what follows a trigger, never that a trigger occurs.   *)
(*                                                                         *)
(* Shutdown is the only guarantee with no Deliver* conjunct: once the      *)
(* caller has asked for shutdown, completion does not depend on the        *)
(* caller reading anything.  The level-1 refinement must discharge         *)
(* ShutdownFairness with binding-owned threads alone.                      *)
(***************************************************************************)

\* The runtime hands over the response head.
THEOREM MetadataFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => (MetadataWaiting(cId) ~> MetadataDelivered(cId))
<1>1. Init /\ [][NextSafe]_vars => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED BY <1>1, MetadataProgressSafeFor, PTL

\* Status is observed, then the head, the backlog and the status itself
\* are drained.  NetworkSend is absent: an unsent message never blocks
\* termination.
THEOREM TerminalFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(ReceiveStatus(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (TerminalWaiting(cId) ~> TerminalReached(cId))
<1>1. Init /\ [][NextSafe]_vars => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED BY <1>1, TerminalProgressSafeFor, PTL

\* No Deliver* at all: draining a channel cancels its calls outright, so
\* shutdown never waits on the caller.
THEOREM ShutdownFairnessRequirement ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           => (ShutdownWaiting(rtId) ~> ShutdownReleased(rtId))
<1>1. Init /\ [][NextSafe]_vars => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED BY <1>1, ShutdownProgressSafeFor, PTL

\* The wire drains submitted messages.  ReceiveStatus is absent, and the
\* delivery chain is present: a latched status disables NetworkSend, and
\* the only way out of that branch is the call reaching terminal.
THEOREM SubmitFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(NetworkSend(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           => (SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i))
<1>1. Init /\ [][NextSafe]_vars => []StrongInv
    BY NominalBehaviorEstablishesStrongInv, SMTT(90)
<1>2. QED BY <1>1, SubmitProgressSafeFor, PTL

\* The caller alone, plus the head that gates message delivery.
THEOREM DeliveryFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))
<1>1. Init /\ [][NextSafe]_vars => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED BY <1>1, DeliveryProgressSafeFor, PTL

(***************************************************************************)
(* LIVENESS                                                                *)
(* Each proof extracts the WF conjuncts its guarantee consumes, cites the  *)
(* nominal leads-to at the fixed instance, and lets PTL lift it across     *)
(* failure using SpecLiftingFacts.                                         *)
(***************************************************************************)

THEOREM EventualMetadataAt ==
    ASSUME NEW cId \in CallIds
    PROVE  Spec => (MetadataPending(cId) ~> MetadataAnswered(cId))
<1>. DEFINE F(c) == WF_vars(DeliverInitialMetadata(c))
<1>2. Fairness => \A c \in CallIds : F(c)
\* Only Isabelle can read a WF conjunct: SMT calls the expression unsupported
\* and Zenon calls the operator unsupported, both immediately.  Opening the
\* bundle takes it about twenty-five seconds, so the budget is stated rather
\* than left to the default and a stretch factor - at the default it closes
\* with no margin, which is a step that fails on a busy machine.
    BY IsaT(120) DEF Fairness
<1>. HIDE DEF F
<1>3. Fairness => F(cId)
    BY <1>2
<1>4. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(DeliverInitialMetadata(cId))
      => (MetadataWaiting(cId) ~> MetadataDelivered(cId))
    BY MetadataProgressSafeFor, PTL
<1>5. QED
    BY SpecLiftingFacts, <1>3, <1>4, PTL
    DEF F, MetadataPending, MetadataAnswered

THEOREM EventualMetadataHolds == Spec => EventualMetadata
<1>1. SUFFICES ASSUME NEW cId \in CallIds
      PROVE Spec => (MetadataPending(cId) ~> MetadataAnswered(cId))
    BY DEF EventualMetadata
<1>2. QED BY EventualMetadataAt

THEOREM EventualTerminalAt ==
    ASSUME NEW cId \in CallIds
    PROVE  Spec => (TerminalPending(cId) ~> TerminalAnswered(cId))
<1>. DEFINE F(c) == /\ WF_vars(ReceiveStatus(c))
                    /\ WF_vars(DeliverInitialMetadata(c))
                    /\ WF_vars(DeliverMessage(c))
                    /\ WF_vars(DeliverStatus(c))
<1>2. Fairness => \A c \in CallIds : F(c)
\* Only Isabelle can read a WF conjunct: SMT calls the expression unsupported
\* and Zenon calls the operator unsupported, both immediately.  Opening the
\* bundle takes it about twenty-five seconds, so the budget is stated rather
\* than left to the default and a stretch factor - at the default it closes
\* with no margin, which is a step that fails on a busy machine.
    BY IsaT(120) DEF Fairness
<1>. HIDE DEF F
<1>3. Fairness => F(cId)
    BY <1>2
<1>4. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(ReceiveStatus(cId))
      /\ WF_vars(DeliverInitialMetadata(cId))
      /\ WF_vars(DeliverMessage(cId))
      /\ WF_vars(DeliverStatus(cId))
      => (TerminalWaiting(cId) ~> TerminalReached(cId))
    BY TerminalProgressSafeFor, PTL
<1>5. QED
    BY SpecLiftingFacts, <1>3, <1>4, PTL
    DEF F, TerminalPending, TerminalAnswered

THEOREM EventualTerminalHolds == Spec => EventualTerminal
<1>1. SUFFICES ASSUME NEW cId \in CallIds
      PROVE Spec => (TerminalPending(cId) ~> TerminalAnswered(cId))
    BY DEF EventualTerminal
<1>2. QED BY EventualTerminalAt

THEOREM SubmitProgressHolds == Spec => SubmitProgress
<1>1. SUFFICES ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
      PROVE Spec => ((SubmitWaitingAt(cId, i) /\ NotFailed) ~>
                         (SubmitDoneAt(cId, i) \/ ~NotFailed))
    BY DEF SubmitProgress, SubmitProgressAt
<1>. DEFINE F(c) == /\ WF_vars(NetworkSend(c))
                    /\ WF_vars(DeliverInitialMetadata(c))
                    /\ WF_vars(DeliverMessage(c))
                    /\ WF_vars(DeliverStatus(c))
<1>2. Fairness => \A c \in CallIds : F(c)
\* Only Isabelle can read a WF conjunct: SMT calls the expression unsupported
\* and Zenon calls the operator unsupported, both immediately.  Opening the
\* bundle takes it about twenty-five seconds, so the budget is stated rather
\* than left to the default and a stretch factor - at the default it closes
\* with no margin, which is a step that fails on a busy machine.
    BY IsaT(120) DEF Fairness
<1>. HIDE DEF F
<1>3. Fairness => F(cId)
    BY <1>2
<1>4. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(NetworkSend(cId))
      /\ WF_vars(DeliverInitialMetadata(cId))
      /\ WF_vars(DeliverMessage(cId))
      /\ WF_vars(DeliverStatus(cId))
      => (SubmitWaitingAt(cId, i) ~> SubmitDoneAt(cId, i))
    BY SubmitProgressSafeFor, PTL
<1>5. QED BY SpecLiftingFacts, <1>3, <1>4, PTL DEF F

THEOREM DeliveryProgressHolds == Spec => DeliveryProgress
<1>1. SUFFICES ASSUME NEW cId \in CallIds, NEW i \in PositiveNaturals
      PROVE Spec => ((DeliveryWaitingAt(cId, i) /\ NotFailed) ~>
                         (DeliveryDoneAt(cId, i) \/ ~NotFailed))
    BY DEF DeliveryProgress, DeliveryProgressAt
<1>. DEFINE F(c) == /\ WF_vars(DeliverInitialMetadata(c))
                    /\ WF_vars(DeliverMessage(c))
<1>2. Fairness => \A c \in CallIds : F(c)
\* Only Isabelle can read a WF conjunct: SMT calls the expression unsupported
\* and Zenon calls the operator unsupported, both immediately.  Opening the
\* bundle takes it about twenty-five seconds, so the budget is stated rather
\* than left to the default and a stretch factor - at the default it closes
\* with no margin, which is a step that fails on a busy machine.
    BY IsaT(120) DEF Fairness
<1>. HIDE DEF F
<1>3. Fairness => F(cId)
    BY <1>2
<1>4. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(DeliverInitialMetadata(cId))
      /\ WF_vars(DeliverMessage(cId))
      => (DeliveryWaitingAt(cId, i) ~> DeliveryDoneAt(cId, i))
    BY DeliveryProgressSafeFor, PTL
<1>5. QED BY SpecLiftingFacts, <1>3, <1>4, PTL DEF F

\* EventualShutdown asks for more than the lifting produces: settlement of
\* this very slot rather than "or some runtime failed".  Three state and
\* action facts close the gap: STOPPING and a failure cannot coexist
\* (SingleRuntime), STOPPING only exits to RELEASED or FAILED, and the
\* settled states are stable.
THEOREM EventualShutdownHolds == Spec => EventualShutdown
<1>1. SUFFICES ASSUME NEW rtId \in RuntimeIds
      PROVE Spec => (ShutdownWaiting(rtId) ~> ShutdownSettled(rtId))
    BY DEF EventualShutdown
<1>. DEFINE F(r) == WF_vars(RuntimeRelease(r))
            G == \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
<1>2a. Fairness => (\A r \in RuntimeIds : F(r)) /\ G
    BY IsaT(120) DEF Fairness
<1>. HIDE DEF F, G
<1>2. Fairness => F(rtId) /\ G
    BY <1>2a
<1>3. /\ []StrongInv
      /\ [][NextSafe]_vars
      /\ WF_vars(RuntimeRelease(rtId))
      /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
      => (ShutdownWaiting(rtId) ~> ShutdownReleased(rtId))
    BY ShutdownProgressSafeFor, PTL
<1>4. IndInv /\ ShutdownWaiting(rtId) => NotFailed
    BY ShutdownWaitingImpliesNotFailed DEF IndInv
<1>5. TypeOK /\ ShutdownWaiting(rtId) /\ [Next]_vars =>
          ShutdownWaiting(rtId)' \/ ShutdownSettled(rtId)'
    BY ShutdownWaitingUnlessSettled
<1>6. TypeOK /\ ShutdownSettled(rtId) /\ [Next]_vars =>
          ShutdownSettled(rtId)'
    BY ShutdownSettledStable
<1>7. ShutdownReleased(rtId) => ShutdownSettled(rtId)
    BY DEF ShutdownReleased, ShutdownSettled
<1>8. IndInv => TypeOK
    BY SMTT(90) DEF IndInv
<1>9. QED
    BY SpecLiftingFacts, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8,
       PTL DEF Spec, F, G

THEOREM LivenessTheorem == Spec => LivenessProperties
<1>1. QED
    BY EventualTerminalHolds, EventualShutdownHolds, SubmitProgressHolds,
       DeliveryProgressHolds, EventualMetadataHolds
    DEF LivenessProperties

=============================================================================
