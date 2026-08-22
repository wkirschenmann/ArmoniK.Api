---- MODULE DotNetBinding_MC_TTrace_1787354789 ----
EXTENDS Sequences, TLCExt, DotNetBinding_MC, Toolbox, Naturals, TLC, DotNetBinding_MC_TEConstants

_expression ==
    LET DotNetBinding_MC_TEExpression == INSTANCE DotNetBinding_MC_TEExpression
    IN DotNetBinding_MC_TEExpression!expression
----

_trace ==
    LET DotNetBinding_MC_TETrace == INSTANCE DotNetBinding_MC_TETrace
    IN DotNetBinding_MC_TETrace!trace
----

_inv ==
    ~(
        TLCGet("level") = Len(_TETrace)
        /\
        buffer_send = ((call1 :> (b1 :> 0 @@ b2 :> 0)))
        /\
        buffer_state = ((call1 :> (b1 :> "none" @@ b2 :> "none")))
        /\
        runtime_root_live = (TRUE)
        /\
        channel_state = ((ch1 :> "none"))
        /\
        delivered = ((call1 :> <<>>))
        /\
        buffers_held_by_host = ((call1 :> 0))
        /\
        runtime_dispose_state = ("destroyed")
        /\
        write_dones_emitted = ((call1 :> 0))
        /\
        channel_runtime = ((ch1 :> "none"))
        /\
        status_pending = ((call1 :> FALSE))
        /\
        events_delivered = ((call1 :> <<>>))
        /\
        send_closed = ((call1 :> FALSE))
        /\
        cancel_requested = ((call1 :> FALSE))
        /\
        buffer_charge = ((<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0))
        /\
        consumer_phase = ((call1 :> "prologue"))
        /\
        retry_state = ((call1 :> "idle"))
        /\
        handle_released = ((call1 :> FALSE))
        /\
        runtime_state = ((rt1 :> "RELEASED"))
        /\
        resources_released_emitted = ((rt1 :> FALSE))
        /\
        received = ((call1 :> <<>>))
        /\
        memory_used = (0)
        /\
        call_state = ((call1 :> "none"))
        /\
        runtime_destroyed = ((rt1 :> TRUE))
        /\
        sent = ((call1 :> <<>>))
        /\
        second_event_owed = ((rt1 :> FALSE))
        /\
        call_root_live = ((call1 :> FALSE))
        /\
        last_lend_status = ((call1 :> "NONE"))
        /\
        write_done_callback_running = ((call1 :> FALSE))
        /\
        call_token_published = ((call1 :> FALSE))
        /\
        call_channel = ((call1 :> "none"))
        /\
        shutdown_callback_running = ((rt1 :> FALSE))
        /\
        submitted = ((call1 :> <<>>))
        /\
        delivery_callback_running = ((call1 :> FALSE))
        /\
        resources_released_callback_running = ((rt1 :> FALSE))
        /\
        call_dispose_state = ((call1 :> "active"))
        /\
        shutdown_event_emitted = ((rt1 :> TRUE))
        /\
        buffer_length = ((<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0))
        /\
        pending_continuations = ((call1 :> 0))
        /\
        payloads_consumed_by_host = ((call1 :> 0))
    )
----

_init ==
    /\ call_dispose_state = _TETrace[1].call_dispose_state
    /\ last_lend_status = _TETrace[1].last_lend_status
    /\ send_closed = _TETrace[1].send_closed
    /\ events_delivered = _TETrace[1].events_delivered
    /\ buffers_held_by_host = _TETrace[1].buffers_held_by_host
    /\ submitted = _TETrace[1].submitted
    /\ buffer_send = _TETrace[1].buffer_send
    /\ runtime_state = _TETrace[1].runtime_state
    /\ delivered = _TETrace[1].delivered
    /\ second_event_owed = _TETrace[1].second_event_owed
    /\ cancel_requested = _TETrace[1].cancel_requested
    /\ write_done_callback_running = _TETrace[1].write_done_callback_running
    /\ buffer_state = _TETrace[1].buffer_state
    /\ channel_state = _TETrace[1].channel_state
    /\ retry_state = _TETrace[1].retry_state
    /\ call_channel = _TETrace[1].call_channel
    /\ call_root_live = _TETrace[1].call_root_live
    /\ call_token_published = _TETrace[1].call_token_published
    /\ write_dones_emitted = _TETrace[1].write_dones_emitted
    /\ sent = _TETrace[1].sent
    /\ runtime_destroyed = _TETrace[1].runtime_destroyed
    /\ handle_released = _TETrace[1].handle_released
    /\ runtime_root_live = _TETrace[1].runtime_root_live
    /\ shutdown_callback_running = _TETrace[1].shutdown_callback_running
    /\ runtime_dispose_state = _TETrace[1].runtime_dispose_state
    /\ shutdown_event_emitted = _TETrace[1].shutdown_event_emitted
    /\ channel_runtime = _TETrace[1].channel_runtime
    /\ resources_released_emitted = _TETrace[1].resources_released_emitted
    /\ buffer_length = _TETrace[1].buffer_length
    /\ call_state = _TETrace[1].call_state
    /\ buffer_charge = _TETrace[1].buffer_charge
    /\ pending_continuations = _TETrace[1].pending_continuations
    /\ consumer_phase = _TETrace[1].consumer_phase
    /\ payloads_consumed_by_host = _TETrace[1].payloads_consumed_by_host
    /\ resources_released_callback_running = _TETrace[1].resources_released_callback_running
    /\ status_pending = _TETrace[1].status_pending
    /\ received = _TETrace[1].received
    /\ memory_used = _TETrace[1].memory_used
    /\ delivery_callback_running = _TETrace[1].delivery_callback_running
----

_next ==
    /\ \E i,j \in DOMAIN _TETrace:
        /\ \/ /\ j = i + 1
              /\ i = TLCGet("level")
        /\ call_dispose_state  = _TETrace[i].call_dispose_state
        /\ call_dispose_state' = _TETrace[j].call_dispose_state
        /\ last_lend_status  = _TETrace[i].last_lend_status
        /\ last_lend_status' = _TETrace[j].last_lend_status
        /\ send_closed  = _TETrace[i].send_closed
        /\ send_closed' = _TETrace[j].send_closed
        /\ events_delivered  = _TETrace[i].events_delivered
        /\ events_delivered' = _TETrace[j].events_delivered
        /\ buffers_held_by_host  = _TETrace[i].buffers_held_by_host
        /\ buffers_held_by_host' = _TETrace[j].buffers_held_by_host
        /\ submitted  = _TETrace[i].submitted
        /\ submitted' = _TETrace[j].submitted
        /\ buffer_send  = _TETrace[i].buffer_send
        /\ buffer_send' = _TETrace[j].buffer_send
        /\ runtime_state  = _TETrace[i].runtime_state
        /\ runtime_state' = _TETrace[j].runtime_state
        /\ delivered  = _TETrace[i].delivered
        /\ delivered' = _TETrace[j].delivered
        /\ second_event_owed  = _TETrace[i].second_event_owed
        /\ second_event_owed' = _TETrace[j].second_event_owed
        /\ cancel_requested  = _TETrace[i].cancel_requested
        /\ cancel_requested' = _TETrace[j].cancel_requested
        /\ write_done_callback_running  = _TETrace[i].write_done_callback_running
        /\ write_done_callback_running' = _TETrace[j].write_done_callback_running
        /\ buffer_state  = _TETrace[i].buffer_state
        /\ buffer_state' = _TETrace[j].buffer_state
        /\ channel_state  = _TETrace[i].channel_state
        /\ channel_state' = _TETrace[j].channel_state
        /\ retry_state  = _TETrace[i].retry_state
        /\ retry_state' = _TETrace[j].retry_state
        /\ call_channel  = _TETrace[i].call_channel
        /\ call_channel' = _TETrace[j].call_channel
        /\ call_root_live  = _TETrace[i].call_root_live
        /\ call_root_live' = _TETrace[j].call_root_live
        /\ call_token_published  = _TETrace[i].call_token_published
        /\ call_token_published' = _TETrace[j].call_token_published
        /\ write_dones_emitted  = _TETrace[i].write_dones_emitted
        /\ write_dones_emitted' = _TETrace[j].write_dones_emitted
        /\ sent  = _TETrace[i].sent
        /\ sent' = _TETrace[j].sent
        /\ runtime_destroyed  = _TETrace[i].runtime_destroyed
        /\ runtime_destroyed' = _TETrace[j].runtime_destroyed
        /\ handle_released  = _TETrace[i].handle_released
        /\ handle_released' = _TETrace[j].handle_released
        /\ runtime_root_live  = _TETrace[i].runtime_root_live
        /\ runtime_root_live' = _TETrace[j].runtime_root_live
        /\ shutdown_callback_running  = _TETrace[i].shutdown_callback_running
        /\ shutdown_callback_running' = _TETrace[j].shutdown_callback_running
        /\ runtime_dispose_state  = _TETrace[i].runtime_dispose_state
        /\ runtime_dispose_state' = _TETrace[j].runtime_dispose_state
        /\ shutdown_event_emitted  = _TETrace[i].shutdown_event_emitted
        /\ shutdown_event_emitted' = _TETrace[j].shutdown_event_emitted
        /\ channel_runtime  = _TETrace[i].channel_runtime
        /\ channel_runtime' = _TETrace[j].channel_runtime
        /\ resources_released_emitted  = _TETrace[i].resources_released_emitted
        /\ resources_released_emitted' = _TETrace[j].resources_released_emitted
        /\ buffer_length  = _TETrace[i].buffer_length
        /\ buffer_length' = _TETrace[j].buffer_length
        /\ call_state  = _TETrace[i].call_state
        /\ call_state' = _TETrace[j].call_state
        /\ buffer_charge  = _TETrace[i].buffer_charge
        /\ buffer_charge' = _TETrace[j].buffer_charge
        /\ pending_continuations  = _TETrace[i].pending_continuations
        /\ pending_continuations' = _TETrace[j].pending_continuations
        /\ consumer_phase  = _TETrace[i].consumer_phase
        /\ consumer_phase' = _TETrace[j].consumer_phase
        /\ payloads_consumed_by_host  = _TETrace[i].payloads_consumed_by_host
        /\ payloads_consumed_by_host' = _TETrace[j].payloads_consumed_by_host
        /\ resources_released_callback_running  = _TETrace[i].resources_released_callback_running
        /\ resources_released_callback_running' = _TETrace[j].resources_released_callback_running
        /\ status_pending  = _TETrace[i].status_pending
        /\ status_pending' = _TETrace[j].status_pending
        /\ received  = _TETrace[i].received
        /\ received' = _TETrace[j].received
        /\ memory_used  = _TETrace[i].memory_used
        /\ memory_used' = _TETrace[j].memory_used
        /\ delivery_callback_running  = _TETrace[i].delivery_callback_running
        /\ delivery_callback_running' = _TETrace[j].delivery_callback_running

\* Uncomment the ASSUME below to write the states of the error trace
\* to the given file in Json format. Note that you can pass any tuple
\* to `JsonSerialize`. For example, a sub-sequence of _TETrace.
    \* ASSUME
    \*     LET J == INSTANCE Json
    \*         IN J!JsonSerialize("DotNetBinding_MC_TTrace_1787354789.json", _TETrace)

=============================================================================

 Note that you can extract this module `DotNetBinding_MC_TEExpression`
  to a dedicated file to reuse `expression` (the module in the 
  dedicated `DotNetBinding_MC_TEExpression.tla` file takes precedence 
  over the module `DotNetBinding_MC_TEExpression` below).

---- MODULE DotNetBinding_MC_TEExpression ----
EXTENDS Sequences, TLCExt, DotNetBinding_MC, Toolbox, Naturals, TLC, DotNetBinding_MC_TEConstants

expression == 
    [
        \* To hide variables of the `DotNetBinding_MC` spec from the error trace,
        \* remove the variables below.  The trace will be written in the order
        \* of the fields of this record.
        call_dispose_state |-> call_dispose_state
        ,last_lend_status |-> last_lend_status
        ,send_closed |-> send_closed
        ,events_delivered |-> events_delivered
        ,buffers_held_by_host |-> buffers_held_by_host
        ,submitted |-> submitted
        ,buffer_send |-> buffer_send
        ,runtime_state |-> runtime_state
        ,delivered |-> delivered
        ,second_event_owed |-> second_event_owed
        ,cancel_requested |-> cancel_requested
        ,write_done_callback_running |-> write_done_callback_running
        ,buffer_state |-> buffer_state
        ,channel_state |-> channel_state
        ,retry_state |-> retry_state
        ,call_channel |-> call_channel
        ,call_root_live |-> call_root_live
        ,call_token_published |-> call_token_published
        ,write_dones_emitted |-> write_dones_emitted
        ,sent |-> sent
        ,runtime_destroyed |-> runtime_destroyed
        ,handle_released |-> handle_released
        ,runtime_root_live |-> runtime_root_live
        ,shutdown_callback_running |-> shutdown_callback_running
        ,runtime_dispose_state |-> runtime_dispose_state
        ,shutdown_event_emitted |-> shutdown_event_emitted
        ,channel_runtime |-> channel_runtime
        ,resources_released_emitted |-> resources_released_emitted
        ,buffer_length |-> buffer_length
        ,call_state |-> call_state
        ,buffer_charge |-> buffer_charge
        ,pending_continuations |-> pending_continuations
        ,consumer_phase |-> consumer_phase
        ,payloads_consumed_by_host |-> payloads_consumed_by_host
        ,resources_released_callback_running |-> resources_released_callback_running
        ,status_pending |-> status_pending
        ,received |-> received
        ,memory_used |-> memory_used
        ,delivery_callback_running |-> delivery_callback_running
        
        \* Put additional constant-, state-, and action-level expressions here:
        \* ,_stateNumber |-> _TEPosition
        \* ,_call_dispose_stateUnchanged |-> call_dispose_state = call_dispose_state'
        
        \* Format the `call_dispose_state` variable as Json value.
        \* ,_call_dispose_stateJson |->
        \*     LET J == INSTANCE Json
        \*     IN J!ToJson(call_dispose_state)
        
        \* Lastly, you may build expressions over arbitrary sets of states by
        \* leveraging the _TETrace operator.  For example, this is how to
        \* count the number of times a spec variable changed up to the current
        \* state in the trace.
        \* ,_call_dispose_stateModCount |->
        \*     LET F[s \in DOMAIN _TETrace] ==
        \*         IF s = 1 THEN 0
        \*         ELSE IF _TETrace[s].call_dispose_state # _TETrace[s-1].call_dispose_state
        \*             THEN 1 + F[s-1] ELSE F[s-1]
        \*     IN F[_TEPosition - 1]
    ]

=============================================================================



Parsing and semantic processing can take forever if the trace below is long.
 In this case, it is advised to uncomment the module below to deserialize the
 trace from a generated binary file.

\*
\*---- MODULE DotNetBinding_MC_TETrace ----
\*EXTENDS IOUtils, DotNetBinding_MC, TLC, DotNetBinding_MC_TEConstants
\*
\*trace == IODeserialize("DotNetBinding_MC_TTrace_1787354789.bin", TRUE)
\*
\*=============================================================================
\*

---- MODULE DotNetBinding_MC_TETrace ----
EXTENDS DotNetBinding_MC, TLC, DotNetBinding_MC_TEConstants

trace == 
    <<
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> FALSE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "active",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "NOT_INIT"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> FALSE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "active",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "NOT_INIT"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> FALSE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "active",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "RUNNING"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> FALSE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "destroying",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "STOPPING"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> FALSE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "destroying",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "STOPPING"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> TRUE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> TRUE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "destroying",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "STOPPING"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> TRUE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "destroying",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "RELEASED"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> FALSE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> TRUE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)]),
    ([buffer_send |-> (call1 :> (b1 :> 0 @@ b2 :> 0)),buffer_state |-> (call1 :> (b1 :> "none" @@ b2 :> "none")),runtime_root_live |-> TRUE,channel_state |-> (ch1 :> "none"),delivered |-> (call1 :> <<>>),buffers_held_by_host |-> (call1 :> 0),runtime_dispose_state |-> "destroyed",write_dones_emitted |-> (call1 :> 0),channel_runtime |-> (ch1 :> "none"),status_pending |-> (call1 :> FALSE),events_delivered |-> (call1 :> <<>>),send_closed |-> (call1 :> FALSE),cancel_requested |-> (call1 :> FALSE),buffer_charge |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),consumer_phase |-> (call1 :> "prologue"),retry_state |-> (call1 :> "idle"),handle_released |-> (call1 :> FALSE),runtime_state |-> (rt1 :> "RELEASED"),resources_released_emitted |-> (rt1 :> FALSE),received |-> (call1 :> <<>>),memory_used |-> 0,call_state |-> (call1 :> "none"),runtime_destroyed |-> (rt1 :> TRUE),sent |-> (call1 :> <<>>),second_event_owed |-> (rt1 :> FALSE),call_root_live |-> (call1 :> FALSE),last_lend_status |-> (call1 :> "NONE"),write_done_callback_running |-> (call1 :> FALSE),call_token_published |-> (call1 :> FALSE),call_channel |-> (call1 :> "none"),shutdown_callback_running |-> (rt1 :> FALSE),submitted |-> (call1 :> <<>>),delivery_callback_running |-> (call1 :> FALSE),resources_released_callback_running |-> (rt1 :> FALSE),call_dispose_state |-> (call1 :> "active"),shutdown_event_emitted |-> (rt1 :> TRUE),buffer_length |-> (<<call1, b1>> :> 0 @@ <<call1, b2>> :> 0),pending_continuations |-> (call1 :> 0),payloads_consumed_by_host |-> (call1 :> 0)])
    >>
----


=============================================================================

---- MODULE DotNetBinding_MC_TEConstants ----
EXTENDS DotNetBinding_MC

CONSTANTS m1, call1, ch1, rt1, b1, b2

=============================================================================

---- CONFIG DotNetBinding_MC_TTrace_1787354789 ----
CONSTANTS
    Messages = { m1 }
    CallIds = { call1 }
    ChannelIds = { ch1 }
    RuntimeIds = { rt1 }
    MaxSendsInFlight = 1
    DeliveryCredits = 1
    BufferIds = { b1 , b2 }
    Ceiling = 3
    MessageLength <- MC_MessageLength
    l0_vars <- [ FfiGrpc ] MC_l0_vars
    ffi_vars <- [ FfiGrpc ] MC_ffi_vars
    vars <- [ FfiGrpc ] MC_l1_vars
    l1_vars <- MC_l1_vars
    rt1 = rt1
    call1 = call1
    m1 = m1
    b2 = b2
    b1 = b1
    ch1 = ch1

INVARIANT
    _inv

CHECK_DEADLOCK
    \* CHECK_DEADLOCK off because of PROPERTY or INVARIANT above.
    FALSE

INIT
    _init

NEXT
    _next

CONSTANT
    _TETrace <- _trace

ALIAS
    _expression
=============================================================================
\* Generated on Sat Aug 22 01:26:46 CEST 2026