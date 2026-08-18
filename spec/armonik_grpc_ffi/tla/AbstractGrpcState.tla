-------------------------- MODULE AbstractGrpcState -------------------------
(***************************************************************************)
(* The shared state of the ArmoniK gRPC FFI specifications: constant       *)
(* parameters, their assumptions, and the level-0 state variables.         *)
(*                                                                         *)
(* Declarations only, so that every refinement level EXTENDS this module   *)
(* and the state is declared exactly once: AbstractGrpc defines the        *)
(* level-0 machinery over it, and each refinement level adds its own       *)
(* variables next to it and reaches the machinery of the level above       *)
(* through an INSTANCE.  Nothing is ever redeclared, so there is exactly   *)
(* one runtime_state (and so on) across all levels.                        *)
(***************************************************************************)

EXTENDS FiniteSets

CONSTANTS
    Messages,       \* The universe of possible messages
    CallIds,        \* Finite set of call identifiers
    ChannelIds,     \* Finite set of channel identifiers
    RuntimeIds      \* Finite set of runtime identifiers (typically singleton)

(***************************************************************************)
(* "none" must not be a member of any identifier set - it is used as a     *)
(* sentinel value for uninitialized ownership fields.                      *)
(***************************************************************************)

ASSUME NoneNotInRuntimeIds == "none" \notin RuntimeIds
ASSUME NoneNotInChannelIds == "none" \notin ChannelIds
ASSUME NoneNotInCallIds == "none" \notin CallIds

ASSUME FiniteCallIds == IsFiniteSet(CallIds)
ASSUME FiniteChannelIds == IsFiniteSet(ChannelIds)
ASSUME FiniteRuntimeIds == IsFiniteSet(RuntimeIds)

(***************************************************************************)
(* LEVEL-0 STATE VARIABLES                                                 *)
(***************************************************************************)

VARIABLES
    runtime_state,      \* [RuntimeIds -> RuntimeStates]
    channel_state,      \* [ChannelIds -> ChannelStates]
    channel_runtime,    \* [ChannelIds -> RuntimeIds \union {"none"}]
    call_state,         \* [CallIds -> CallStates]
    call_channel,       \* [CallIds -> ChannelIds \union {"none"}]
    submitted,          \* [CallIds -> Seq(Messages)]
    sent,               \* [CallIds -> Seq(Messages)]
    received,           \* [CallIds -> Seq(Messages)]
    delivered,          \* [CallIds -> Seq(Messages)]
    events_delivered,   \* [CallIds -> Seq(EventKinds)]
    send_closed,        \* [CallIds -> BOOLEAN]
    status_pending      \* [CallIds -> BOOLEAN]

=============================================================================
