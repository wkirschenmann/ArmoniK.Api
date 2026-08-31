--------------------------- MODULE AbstractGrpc ----------------------------
(***************************************************************************)
(* Level 0 - Abstract specification for ArmoniK gRPC FFI.                  *)
(* Models the observable behavior of the runtime, channels, calls, and     *)
(* message delivery as seen by a client.                                   *)
(*                                                                         *)
(* All state is modeled as total functions over finite identifier sets.    *)
(* RELEASED is where a runtime ends.  Level 1 refines it into two          *)
(* observable statuses - AK_RUNTIME_GRPC_STOPPED and AK_RUNTIME_QUIESCENT -*)
(* whose difference is what the host still holds; that ownership question  *)
(* does not exist at this level.                                           *)
(***************************************************************************)

EXTENDS AbstractGrpcState, Naturals, Sequences

(***************************************************************************)
(* The constants, their assumptions and the twelve state variables are     *)
(* declared once in AbstractGrpcState, shared with the refinement levels.  *)
(***************************************************************************)

(***************************************************************************)
(* Derived constants                                                       *)
(***************************************************************************)

RuntimeStates == {"NOT_INIT", "RUNNING", "STOPPING", "RELEASED", "FAILED_UNQUIESCED"}
ChannelStates == {"none", "open", "closing", "closed"}
CallStates    == {"none", "started", "sending", "half_closed", "terminal"}
StatusKinds   == {"COMPLETED", "CANCELLED"}
EventKinds    == {"INITIAL_METADATA", "MESSAGE"} \union StatusKinds

\* --- DOMAIN-DRIVEN GROUPINGS ---
RuntimeVars == <<runtime_state>>
ChannelVars == <<channel_state, channel_runtime>>
CallVars    == <<call_state, call_channel, submitted, sent, received, delivered,
                 events_delivered, send_closed, status_pending>>

vars == <<runtime_state, channel_state, channel_runtime,
          call_state, call_channel, submitted, sent, received, delivered,
          events_delivered, send_closed, status_pending>>

(***************************************************************************)
(* HELPERS                                                                 *)
(***************************************************************************)

\* Channels that have been created (state != "none")
UsedChannels == {chId \in ChannelIds : channel_state[chId] # "none"}

\* Channels in an active (non-terminal) state
ActiveChannelStates == {"open", "closing"}
ActiveChannels == {chId \in ChannelIds : channel_state[chId] \in ActiveChannelStates}

\* Calls that have been created
IsUnusedCall(cId) == call_state[cId] = "none"
UsedCalls == {cId \in CallIds : ~IsUnusedCall(cId)}

\* Calls in an active (non-terminal) state
ActiveCallStates == {"started", "sending", "half_closed"}
IsActiveCall(cId) == call_state[cId] \in ActiveCallStates
ActiveCalls == {cId \in CallIds : IsActiveCall(cId)}

\* s1 is a prefix of s2
IsPrefix(s1, s2) ==
    /\ Len(s1) <= Len(s2)
    /\ SubSeq(s2, 1, Len(s1)) = s1

HasStatus(cId) ==
    /\ Len(events_delivered[cId]) > 0
    /\ events_delivered[cId][Len(events_delivered[cId])] \in StatusKinds

IsCancelled(cId) ==
    /\ Len(events_delivered[cId]) > 0
    /\ events_delivered[cId][Len(events_delivered[cId])] = "CANCELLED"

IsTerminalCall(cId) == call_state[cId] = "terminal"

\* No runtime is in a failed state - when this is false, no property holds
NotFailed == \A rtId \in RuntimeIds : runtime_state[rtId] # "FAILED_UNQUIESCED"

\* Channels owned by a runtime
ChannelsOf(rtId) == {chId \in ChannelIds : channel_runtime[chId] = rtId}

\* Calls on a channel
CallsOf(chId) == {cId \in CallIds : call_channel[cId] = chId}

(***************************************************************************)
(* TYPE INVARIANT                                                          *)
(***************************************************************************)

TypeOK ==
    /\ runtime_state \in [RuntimeIds -> RuntimeStates]
    /\ channel_state \in [ChannelIds -> ChannelStates]
    /\ channel_runtime \in [ChannelIds -> RuntimeIds \union {"none"}]
    /\ call_state \in [CallIds -> CallStates]
    /\ call_channel \in [CallIds -> ChannelIds \union {"none"}]
    /\ submitted \in [CallIds -> Seq(Messages)]
    /\ sent \in [CallIds -> Seq(Messages)]
    /\ received \in [CallIds -> Seq(Messages)]
    /\ delivered \in [CallIds -> Seq(Messages)]
    /\ events_delivered \in [CallIds -> Seq(EventKinds)]
    /\ send_closed \in [CallIds -> BOOLEAN]
    /\ status_pending \in [CallIds -> BOOLEAN]

(***************************************************************************)
(* INITIAL STATE                                                           *)
(***************************************************************************)

Init ==
    /\ runtime_state = [rtId \in RuntimeIds |-> "NOT_INIT"]
    /\ channel_state = [chId \in ChannelIds |-> "none"]
    /\ channel_runtime = [chId \in ChannelIds |-> "none"]
    /\ call_state = [cId \in CallIds |-> "none"]
    /\ call_channel = [cId \in CallIds |-> "none"]
    /\ submitted = [cId \in CallIds |-> <<>>]
    /\ sent = [cId \in CallIds |-> <<>>]
    /\ received = [cId \in CallIds |-> <<>>]
    /\ delivered = [cId \in CallIds |-> <<>>]
    /\ events_delivered = [cId \in CallIds |-> <<>>]
    /\ send_closed = [cId \in CallIds |-> FALSE]
    /\ status_pending = [cId \in CallIds |-> FALSE]

(***************************************************************************)
(* ACTIONS - Runtime lifecycle                                             *)
(***************************************************************************)

RuntimeCreate(rtId) ==
    /\ runtime_state[rtId] = "NOT_INIT"
    /\ \A other \in RuntimeIds :
        other # rtId => runtime_state[other] \in {"NOT_INIT", "RELEASED"}
    /\ runtime_state' = [runtime_state EXCEPT ![rtId] = "RUNNING"]
    /\ UNCHANGED <<ChannelVars, CallVars>>

RuntimeBeginShutdown(rtId) ==
    /\ runtime_state[rtId] = "RUNNING"
    /\ runtime_state' = [runtime_state EXCEPT ![rtId] = "STOPPING"]
    /\ channel_state' = [chId \in ChannelIds |->
        IF channel_runtime[chId] = rtId /\ channel_state[chId] = "open"
        THEN "closing"
        ELSE channel_state[chId]]
    /\ UNCHANGED <<channel_runtime, CallVars>>

RuntimeRelease(rtId) ==
    /\ runtime_state[rtId] = "STOPPING"
    /\ \A chId \in ChannelsOf(rtId) : channel_state[chId] = "closed"
    /\ \A chId \in ChannelsOf(rtId) :
        \A cId \in CallsOf(chId) : call_state[cId] = "terminal"
    /\ runtime_state' = [runtime_state EXCEPT ![rtId] = "RELEASED"]
    /\ UNCHANGED <<ChannelVars, CallVars>>

RuntimeFail(rtId) ==
    /\ runtime_state[rtId] \in {"RUNNING", "STOPPING"}
    /\ runtime_state' = [runtime_state EXCEPT ![rtId] = "FAILED_UNQUIESCED"]
    /\ UNCHANGED <<ChannelVars, CallVars>>

\* A failed runtime is the sole active runtime; all other runtime slots must be
\* unused or already released.  This explicit stutter preserves deadlock freedom
\* after failure without permitting another runtime to become active.
RemainFailed(rtId) ==
    /\ runtime_state[rtId] = "FAILED_UNQUIESCED"
    /\ \A other \in RuntimeIds :
        other # rtId => runtime_state[other] \in {"NOT_INIT", "RELEASED"}
    /\ UNCHANGED vars

\* Only a fully released model may stutter here.  NOT_INIT slots still have
\* RuntimeCreate work available and therefore do not satisfy this guard.
RemainReleased ==
    /\ \A rtId \in RuntimeIds : runtime_state[rtId] = "RELEASED"
    /\ UNCHANGED vars

(***************************************************************************)
(* ACTIONS - Channel lifecycle                                             *)
(***************************************************************************)

ChannelCreate(chId, rtId) ==
    /\ runtime_state[rtId] = "RUNNING"
    /\ channel_state[chId] = "none"
    /\ channel_state' = [channel_state EXCEPT ![chId] = "open"]
    /\ channel_runtime' = [channel_runtime EXCEPT ![chId] = rtId]
    /\ UNCHANGED <<RuntimeVars, CallVars>>

ChannelStartClosing(chId) ==
    /\ channel_state[chId] = "open"
    /\ channel_state' = [channel_state EXCEPT ![chId] = "closing"]
    /\ UNCHANGED <<RuntimeVars, channel_runtime, CallVars>>

ChannelFinishClosing(chId) ==
    /\ channel_state[chId] = "closing"
    \* Cancel all active calls on this channel and close
    /\ call_state' = [cId \in CallIds |->
        IF call_channel[cId] = chId /\ IsActiveCall(cId)
        THEN "terminal"
        ELSE call_state[cId]]
    /\ events_delivered' = [cId \in CallIds |->
        IF call_channel[cId] = chId /\ IsActiveCall(cId)
        THEN IF events_delivered[cId] = <<>>
             THEN <<"INITIAL_METADATA", "CANCELLED">>
             ELSE IF ~HasStatus(cId)
                  THEN Append(events_delivered[cId], "CANCELLED")
                  ELSE events_delivered[cId]
        ELSE events_delivered[cId]]
    /\ status_pending' = [cId \in CallIds |->
        IF call_channel[cId] = chId /\ IsActiveCall(cId)
        THEN FALSE
        ELSE status_pending[cId]]
    /\ channel_state' = [channel_state EXCEPT ![chId] = "closed"]
    /\ UNCHANGED <<RuntimeVars, channel_runtime,
                   call_channel, submitted, sent, received, delivered, send_closed>>

(***************************************************************************)
(* ACTIONS - Call lifecycle                                                *)
(***************************************************************************)

CallStart(cId, chId) ==
    /\ channel_state[chId] = "open"
    /\ runtime_state[channel_runtime[chId]] = "RUNNING"
    /\ IsUnusedCall(cId)
    /\ call_state' = [call_state EXCEPT ![cId] = "started"]
    /\ call_channel' = [call_channel EXCEPT ![cId] = chId]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   submitted, sent, received, delivered, events_delivered,
                   send_closed, status_pending>>

SendMessage(cId, msg) ==
    /\ call_state[cId] \in {"started", "sending"}
    /\ send_closed[cId] = FALSE
    /\ ~status_pending[cId]
    /\ submitted' = [submitted EXCEPT ![cId] = Append(@, msg)]
    /\ call_state' = [call_state EXCEPT ![cId] = "sending"]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_channel, sent, received, delivered, events_delivered,
                   send_closed, status_pending>>

EndSend(cId) ==
    /\ call_state[cId] \in {"started", "sending"}
    /\ send_closed[cId] = FALSE
    /\ ~status_pending[cId]
    /\ send_closed' = [send_closed EXCEPT ![cId] = TRUE]
    /\ call_state' = [call_state EXCEPT ![cId] = "half_closed"]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_channel, submitted, sent, received, delivered,
                   events_delivered, status_pending>>

(***************************************************************************)
(* ACTIONS - Network (internal progress)                                   *)
(***************************************************************************)

NetworkSend(cId) ==
    /\ IsActiveCall(cId)
    /\ ~status_pending[cId]
    /\ Len(sent[cId]) < Len(submitted[cId])
    /\ LET nextIdx == Len(sent[cId]) + 1
       IN sent' = [sent EXCEPT ![cId] = Append(@, submitted[cId][nextIdx])]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_state, call_channel, submitted, received, delivered,
                   events_delivered, send_closed, status_pending>>

NetworkReceive(cId, msg) ==
    /\ IsActiveCall(cId)
    /\ ~status_pending[cId]
    /\ received' = [received EXCEPT ![cId] = Append(@, msg)]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_state, call_channel, submitted, sent, delivered,
                   events_delivered, send_closed, status_pending>>

ReceiveStatus(cId) ==
    /\ IsActiveCall(cId)
    /\ ~status_pending[cId]
    /\ status_pending' = [status_pending EXCEPT ![cId] = TRUE]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_state, call_channel, submitted, sent, received,
                   delivered, events_delivered, send_closed>>

(***************************************************************************)
(* ACTIONS - Event delivery                                                *)
(***************************************************************************)

DeliverInitialMetadata(cId) ==
    /\ IsActiveCall(cId)
    /\ events_delivered[cId] = <<>>
    /\ events_delivered' = [events_delivered EXCEPT ![cId] = Append(@, "INITIAL_METADATA")]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_state, call_channel, submitted, sent, received,
                   delivered, send_closed, status_pending>>

DeliverMessage(cId) ==
    /\ IsActiveCall(cId)
    /\ Len(events_delivered[cId]) >= 1
    /\ events_delivered[cId][1] = "INITIAL_METADATA"
    /\ ~HasStatus(cId)
    /\ Len(delivered[cId]) < Len(received[cId])
    /\ LET nextIdx == Len(delivered[cId]) + 1
       IN delivered' = [delivered EXCEPT ![cId] = Append(@, received[cId][nextIdx])]
    /\ events_delivered' = [events_delivered EXCEPT ![cId] = Append(@, "MESSAGE")]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_state, call_channel, submitted, sent, received,
                   send_closed, status_pending>>

DeliverStatus(cId) ==
    /\ IsActiveCall(cId)
    /\ status_pending[cId]
    /\ Len(events_delivered[cId]) >= 1
    /\ events_delivered[cId][1] = "INITIAL_METADATA"
    /\ ~HasStatus(cId)
    \* All received messages must have been delivered before status
    /\ Len(delivered[cId]) = Len(received[cId])
    /\ events_delivered' = [events_delivered EXCEPT ![cId] = Append(@, "COMPLETED")]
    /\ call_state' = [call_state EXCEPT ![cId] = "terminal"]
    /\ status_pending' = [status_pending EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_channel, submitted, sent, received, delivered, send_closed>>

CallCancel(cId) ==
    /\ IsActiveCall(cId)
    /\ IF events_delivered[cId] = <<>>
       THEN events_delivered' = [events_delivered EXCEPT ![cId] = <<"INITIAL_METADATA", "CANCELLED">>]
       ELSE /\ ~HasStatus(cId)
            /\ events_delivered' = [events_delivered EXCEPT ![cId] = Append(@, "CANCELLED")]
    /\ call_state' = [call_state EXCEPT ![cId] = "terminal"]
    /\ status_pending' = [status_pending EXCEPT ![cId] = FALSE]
    /\ UNCHANGED <<RuntimeVars, ChannelVars,
                   call_channel, submitted, sent, received, delivered, send_closed>>

(***************************************************************************)
(* NEXT STATE RELATION                                                     *)
(***************************************************************************)

Next ==
    \/ \E rtId \in RuntimeIds : RuntimeCreate(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeBeginShutdown(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeRelease(rtId)
    \/ \E rtId \in RuntimeIds : RuntimeFail(rtId)
    \/ \E rtId \in RuntimeIds : RemainFailed(rtId)
    \/ RemainReleased
    \/ \E chId \in ChannelIds, rtId \in RuntimeIds : ChannelCreate(chId, rtId)
    \/ \E chId \in ChannelIds : ChannelStartClosing(chId)
    \/ \E chId \in ChannelIds : ChannelFinishClosing(chId)
    \/ \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
    \/ \E cId \in CallIds, msg \in Messages : SendMessage(cId, msg)
    \/ \E cId \in CallIds : EndSend(cId)
    \/ \E cId \in CallIds : NetworkSend(cId)
    \/ \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    \/ \E cId \in CallIds : ReceiveStatus(cId)
    \/ \E cId \in CallIds : DeliverInitialMetadata(cId)
    \/ \E cId \in CallIds : DeliverMessage(cId)
    \/ \E cId \in CallIds : DeliverStatus(cId)
    \/ \E cId \in CallIds : CallCancel(cId)

(***************************************************************************)
(* SAFETY INVARIANTS                                                       *)
(***************************************************************************)

MetadataFirst ==
    \A cId \in UsedCalls :
        Len(events_delivered[cId]) > 0 => events_delivered[cId][1] = "INITIAL_METADATA"

NoEventAfterStatus ==
    \A cId \in UsedCalls :
        \A i \in 1..Len(events_delivered[cId]) :
            events_delivered[cId][i] \in StatusKinds => i = Len(events_delivered[cId])

SubmittedPrefixOfSent ==
    \A cId \in UsedCalls : IsPrefix(sent[cId], submitted[cId])

ReceivedPrefixOfDelivered ==
    \A cId \in UsedCalls : IsPrefix(delivered[cId], received[cId])

TerminalStatusEquivalence ==
    \A cId \in UsedCalls : IsTerminalCall(cId) <=> HasStatus(cId)

SendAfterEndSend ==
    \A cId \in UsedCalls :
        send_closed[cId] => call_state[cId] \in {"half_closed", "terminal"}

SingleRuntime ==
    \A rt1, rt2 \in RuntimeIds :
        /\ runtime_state[rt1] \in {"RUNNING", "STOPPING", "FAILED_UNQUIESCED"}
        /\ runtime_state[rt2] \in {"RUNNING", "STOPPING", "FAILED_UNQUIESCED"}
        => rt1 = rt2

ChannelOwnership ==
    \A chId \in UsedChannels : channel_runtime[chId] \in RuntimeIds

CallOwnership ==
    \A cId \in UsedCalls : call_channel[cId] \in UsedChannels

\* Active call implies active channel
ActiveCallImpliesActiveChannel ==
    \A cId \in ActiveCalls : call_channel[cId] \in ActiveChannels

\* Closed channel implies all its calls are terminal
ClosedChannelNoCalls ==
    \A chId \in ChannelIds :
        channel_state[chId] = "closed" =>
            \A cId \in CallsOf(chId) : call_state[cId] = "terminal"

\* Active channel implies runtime is running, stopping or failed
ActiveChannelImpliesActiveRuntime ==
    \A chId \in ActiveChannels :
        channel_runtime[chId] \in RuntimeIds /\
        runtime_state[channel_runtime[chId]] \in {"RUNNING", "STOPPING", "FAILED_UNQUIESCED"}

StoppingClosesChannels ==
    \A rtId \in RuntimeIds :
        runtime_state[rtId] = "STOPPING" =>
            \A chId \in ChannelsOf(rtId) : channel_state[chId] \in {"closing", "closed"}

ReleasedNoChannels ==
    \A rtId \in RuntimeIds :
        runtime_state[rtId] = "RELEASED" =>
            \A chId \in ChannelsOf(rtId) : channel_state[chId] = "closed"

ReleasedNoCalls ==
    \A rtId \in RuntimeIds :
        runtime_state[rtId] = "RELEASED" =>
            \A chId \in ChannelsOf(rtId) :
                \A cId \in CallsOf(chId) : call_state[cId] = "terminal"

\* --- Message completeness (safety: non-cancelled terminal implies all received were delivered) ---

CompleteDelivery ==
    \A cId \in UsedCalls :
        (IsTerminalCall(cId) /\ ~IsCancelled(cId)) => Len(delivered[cId]) = Len(received[cId])

\* Safe for traces of length zero and one: no zero-based sequence access.
EventStreamShape ==
    \A cId \in UsedCalls :
        /\ Len(events_delivered[cId]) > 0 =>
              events_delivered[cId][1] = "INITIAL_METADATA"
        /\ \A i \in 2..Len(events_delivered[cId]) :
              events_delivered[cId][i] \in {"MESSAGE"} \union StatusKinds
        /\ \A i \in 2..Len(events_delivered[cId]) :
              i < Len(events_delivered[cId]) =>
                  events_delivered[cId][i] = "MESSAGE"

\* A MESSAGE callback carries exactly one message: given the stream shape,
\* the event count is one metadata, one event per delivered message, and one
\* terminal if it has arrived.  This is the link between the sequence that
\* carries the FFI debt and the sequence of data.
MessageEventsMatchDelivered ==
    \A cId \in UsedCalls :
        Len(events_delivered[cId]) =
            (IF Len(events_delivered[cId]) > 0 THEN 1 ELSE 0)
            + Len(delivered[cId])
            + (IF HasStatus(cId) THEN 1 ELSE 0)

SafetyCore ==
    /\ TypeOK
    /\ MetadataFirst
    /\ NoEventAfterStatus
    /\ SubmittedPrefixOfSent
    /\ ReceivedPrefixOfDelivered
    /\ TerminalStatusEquivalence
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
    /\ CompleteDelivery
    /\ EventStreamShape
    /\ MessageEventsMatchDelivered

\* Safety is required only while no runtime has entered the deliberately
\* unconstrained failed state.
SafetyInvariant == NotFailed => SafetyCore

(***************************************************************************)
(* LIVENESS PROPERTIES                                                     *)
(*                                                                         *)
(* Each guarantee is stated over a named waiting/reached pair, and the     *)
(* proof reuses those names, so nothing ever has to be rewritten under a   *)
(* leads-to.                                                               *)
(*                                                                         *)
(* Every antecedent embeds a trigger the model deliberately leaves unfair: *)
(* CallStart, SendMessage, RuntimeBeginShutdown.  The library promises     *)
(* what follows a trigger, never that a trigger occurs.                    *)
(***************************************************************************)

PositiveNaturals == Nat \ {0}

\* A started call still waiting for its response head.
MetadataWaiting(cId) ==
    /\ IsActiveCall(cId)
    /\ events_delivered[cId] = <<>>

MetadataDelivered(cId) == Len(events_delivered[cId]) > 0

\* An active call has not reached a terminal event yet.
TerminalWaiting(cId) == IsActiveCall(cId)

TerminalReached(cId) == HasStatus(cId)

\* The pending/answered forms carry the failure escape inside the
\* name: a quantified leads-to is strippable only when both operands
\* are bare named applications.
TerminalPending(cId) == TerminalWaiting(cId) /\ NotFailed

TerminalAnswered(cId) == TerminalReached(cId) \/ ~NotFailed

MetadataPending(cId) == MetadataWaiting(cId) /\ NotFailed

MetadataAnswered(cId) == MetadataDelivered(cId) \/ ~NotFailed

\* A runtime the caller has asked to stop.
ShutdownWaiting(rtId) == runtime_state[rtId] = "STOPPING"

ShutdownReleased(rtId) == runtime_state[rtId] = "RELEASED"

ShutdownSettled(rtId) ==
    runtime_state[rtId] \in {"RELEASED", "FAILED_UNQUIESCED"}

\* Submitted position i has not reached the wire.
SubmitWaitingAt(cId, i) ==
    /\ i <= Len(submitted[cId])
    /\ Len(sent[cId]) < i
    /\ ~IsTerminalCall(cId)

SubmitDoneAt(cId, i) ==
    \/ /\ i <= Len(sent[cId])
       /\ sent[cId][i] = submitted[cId][i]
    \/ IsTerminalCall(cId)

\* Received position i has not reached the consumer.
DeliveryWaitingAt(cId, i) ==
    /\ i <= Len(received[cId])
    /\ Len(delivered[cId]) < i
    /\ ~IsCancelled(cId)

DeliveryDoneAt(cId, i) ==
    \/ /\ i <= Len(delivered[cId])
       /\ delivered[cId][i] = received[cId][i]
    \/ IsCancelled(cId)

\* Every active call eventually terminates, unless a runtime fails.
EventualTerminal ==
    \A cId \in CallIds :
        TerminalPending(cId) ~> TerminalAnswered(cId)

\* A runtime asked to stop always settles, and in this very slot.  This is
\* stronger than what the generic failure lifting produces, which only
\* yields "or some runtime failed".
EventualShutdown ==
    \A rtId \in RuntimeIds :
        ShutdownWaiting(rtId) ~> ShutdownSettled(rtId)

\* Every submitted position eventually reaches the wire with its value,
\* unless the call terminates or a runtime fails.
SubmitProgressAt(cId, i) ==
    (SubmitWaitingAt(cId, i) /\ NotFailed) ~>
        (SubmitDoneAt(cId, i) \/ ~NotFailed)

SubmitProgress ==
    \A cId \in CallIds :
        \A i \in PositiveNaturals : SubmitProgressAt(cId, i)

\* Every received position eventually reaches the consumer with its value,
\* unless the call is cancelled or a runtime fails.
DeliveryProgressAt(cId, i) ==
    (DeliveryWaitingAt(cId, i) /\ NotFailed) ~>
        (DeliveryDoneAt(cId, i) \/ ~NotFailed)

DeliveryProgress ==
    \A cId \in CallIds :
        \A i \in PositiveNaturals : DeliveryProgressAt(cId, i)

\* A started call eventually gets its initial metadata.
EventualMetadata ==
    \A cId \in CallIds :
        MetadataPending(cId) ~> MetadataAnswered(cId)

\* The five guarantees under one name, so the top-level proof can cite them
\* as a single obligation.
LivenessProperties ==
    /\ EventualTerminal
    /\ EventualShutdown
    /\ SubmitProgress
    /\ DeliveryProgress
    /\ EventualMetadata

(***************************************************************************)
(* FAIRNESS AND SPEC                                                       *)
(*                                                                         *)
(* Two kinds of conjuncts live here.  NetworkSend, ReceiveStatus,          *)
(* RuntimeRelease and ChannelFinishClosing are promises of the library:    *)
(* the FFI refinement must discharge them with its own                     *)
(* threads.  The Deliver* conjuncts are continuing obligations of the      *)
(* caller: level 0 has no demand variable, so delivery is guarded by       *)
(* availability alone, and a caller that stops reading falsifies them.     *)
(* The trigger actions (CallStart, SendMessage, RuntimeBeginShutdown)      *)
(* carry no fairness at all: the library never promises that a trigger     *)
(* occurs, only what follows one.                                          *)
(***************************************************************************)

Fairness ==
    /\ \A cId \in CallIds : WF_vars(NetworkSend(cId))
    /\ \A cId \in CallIds : WF_vars(ReceiveStatus(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverMessage(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverInitialMetadata(cId))
    /\ \A cId \in CallIds : WF_vars(DeliverStatus(cId))
    /\ \A rtId \in RuntimeIds : WF_vars(RuntimeRelease(rtId))
    /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))

Spec == Init /\ [][Next]_vars /\ Fairness

=============================================================================
