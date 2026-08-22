--------------------------- MODULE DotNetBinding_MCdirected ---------------------------
(***************************************************************************)
(* TLC model checking configuration for DotNetBinding.                     *)
(* Exploration and debugging, not evidence - but during convergence it is  *)
(* the dead-action detector: TLAPS happily proves that an action that can  *)
(* never fire preserves everything, so before any proof exists, TLC's      *)
(* per-action coverage statistics are what certifies every action fires.   *)
(* Run with -coverage 1 and read the action counts.                        *)
(***************************************************************************)

EXTENDS DotNetBinding_defs, TLC

(***************************************************************************)
(* FINITE EXPLORATION BOUND                                                *)
(***************************************************************************)

\* The managed variables are bounded by the level-0 streams and the finite
\* phase machines, so bounding the streams is enough for a finite graph.
StateConstraint ==
    /\ \A cId \in CallIds : Len(submitted[cId]) <= 2
    /\ \A cId \in CallIds : Len(received[cId]) <= 2
    /\ \A cId \in CallIds : pending_continuations[cId] <= 3

\* Overrides MessageLength: a .cfg constant assignment cannot carry a
\* function literal.  One byte per message - the ceiling is the subject,
\* not a spread of sizes.
MC_MessageLength == [msg \in Messages |-> 1]

\* The level-1 safety aggregate, under a cfg-citable name.
MC_L1Safety == F!SafetyInvariant

(***************************************************************************)
(* TLC WORKAROUND                                                          *)
(* TLC cannot resolve doubly-instantiated variable tuples (F!l0_vars is    *)
(* L0!vars seen through two INSTANCE layers).  Both tuple definitions are  *)
(* overridden in every configuration with flat lists of the shared         *)
(* variables, exactly as FfiGrpc_MC overrides l0_vars.                     *)
(***************************************************************************)

MC_l0_vars == <<runtime_state, channel_state, channel_runtime,
                call_state, call_channel, submitted, sent, received,
                delivered, events_delivered, send_closed, status_pending>>

MC_ffi_vars == <<buffers_held_by_host, write_dones_emitted,
                 write_done_callback_running, delivery_callback_running,
                 payloads_consumed_by_host, handle_released,
                 cancel_requested, shutdown_event_emitted,
                 shutdown_callback_running, runtime_destroyed,
                 buffer_state, buffer_send, second_event_owed,
                 last_lend_status, resources_released_emitted,
                 resources_released_callback_running,
                 buffer_charge, buffer_length, memory_used>>

MC_l1_vars == <<MC_l0_vars, MC_ffi_vars>>

(***************************************************************************)
(* DIRECTED REACHABILITY - the second-event chain                          *)
(* Random walks return a lent buffer almost immediately, so the behaviour  *)
(* where the debt survives to the shutdown is unreachable in practice for  *)
(* a blind search.  The directed configuration prunes every state where a  *)
(* buffer was returned before the shutdown event, which collapses the      *)
(* graph onto exactly the paths of interest; the "invariant" below is the  *)
(* target stated negatively - its violation trace IS the witness that      *)
(* AK_EVENT_RESOURCES_RELEASED is reachable at level 2.                    *)
(***************************************************************************)

DebtSurvivesToShutdown ==
    \/ \E rtId \in RuntimeIds : shutdown_event_emitted[rtId]
    \/ \A cId \in CallIds, b \in BufferIds :
           buffer_state[cId][b] # "returned"

SecondEventNeverEmitted ==
    \A rtId \in RuntimeIds : ~resources_released_callback_running[rtId]

===============================================================================
