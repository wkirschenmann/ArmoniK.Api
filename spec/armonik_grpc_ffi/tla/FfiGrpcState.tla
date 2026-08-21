---------------------------- MODULE FfiGrpcState ----------------------------
(***************************************************************************)
(* The level-1 state: the shared level-0 state plus the FFI constants and  *)
(* variables.                                                              *)
(*                                                                         *)
(* Declarations only, following AbstractGrpcState: FfiGrpc defines the     *)
(* level-1 machinery over it, and the next refinement level extends this   *)
(* module, adds its own variables next to it, and reaches the level-1      *)
(* machinery through an INSTANCE of FfiGrpcTheorems.                       *)
(***************************************************************************)

EXTENDS AbstractGrpcState, Naturals, FiniteSets

(***************************************************************************)
(* FFI CONSTANTS - the pipelining depths of the ABI contract               *)
(***************************************************************************)

CONSTANTS
    MaxSendsInFlight, \* pinned send buffers a call may hold at once
    DeliveryCredits,  \* unconsumed payloads a call may owe the host
    BufferIds,        \* the send-buffer identity space, per call
    Ceiling,          \* the runtime-wide byte limit on lent send memory
    MessageLength     \* the size in bytes of each abstract message

ASSUME MaxSendsInFlightIsPositive == MaxSendsInFlight \in Nat \ {0}

ASSUME DeliveryCreditsArePositive == DeliveryCredits \in Nat \ {0}

\* Positive so that a lend of the whole ceiling is representable.  A zero
\* total does not mean an empty outstanding set: an empty message charges
\* nothing, and the accounting counts charges, not buffers.
ASSUME CeilingIsPositive == Ceiling \in Nat \ {0}

\* A message has a size.  Only the commit reads it, to check that the message
\* fits the buffer it was given; the budget charges what the allocator handed
\* out, which is at least what the host asked for and may be more.
ASSUME MessageLengthIsNat == MessageLength \in [Messages -> Nat]

\* Buffers are named, unlike payloads, because ak_buffer carries an owner
\* and the ABI says a buffer is given back exactly once.  A payload needs
\* no name: release is FIFO, so the release count already says which one
\* is owed.  Buffer returns are unordered, so a count says nothing about
\* which buffer came back, and that is the gap this space closes.
\* Finite so that the per-buffer arguments are finite inductions; nonempty
\* so that a lemma about one buffer has something to name.
ASSUME BufferIdsAreAFiniteNonemptySet ==
    /\ IsFiniteSet(BufferIds)
    /\ BufferIds # {}

(***************************************************************************)
(* FFI STATE VARIABLES                                                     *)
(* Both pipelines are FIFO, and each is identified by one monotone         *)
(* counter against a level-0 sequence.  A send is the index of its         *)
(* message in submitted, acquitted by WRITE_DONE in send order; a payload  *)
(* is the index of its event in events_delivered, released in delivery     *)
(* order.  In both cases the counter names the settled prefix, and the     *)
(* difference with the sequence length is what the host still holds.       *)
(*                                                                         *)
(* Send buffers come out of the call's arena, so the send side needs one   *)
(* level on top of its counter: how many buffers are lent out and not yet  *)
(* committed.  A lent buffer leaves that level either by carrying a        *)
(* message (it becomes a send) or by being returned unused, which is what  *)
(* lets the runtime insist that nothing is outstanding before it reclaims  *)
(* the call.                                                               *)
(*                                                                         *)
(* buffer_state is that level again, but per buffer rather than counted,   *)
(* and it is the one place the model carries an identity the counters      *)
(* cannot: none -> lent -> returned -> freed, monotone, never reused.      *)
(* The count and the states are tied by LentCountMatchesBufferStates, and  *)
(* keeping both is what makes the identities additive - every proof that   *)
(* reads the counter stands unchanged.  "returned" and "freed" are two     *)
(* states because they are two events: giving the buffer back is the       *)
(* host's, releasing the memory is the runtime's, and the replay buffer    *)
(* is the gap between them.                                                *)
(*                                                                         *)
(* buffer_send is the link the ABI has and the counters did not, and it is *)
(* keyed the way the ABI keys it: ak_call_send_message names the buffer, so*)
(* a buffer knows which send lives in it, zero meaning none.  Keyed by the *)
(* buffer rather than by the send index, both questions the model asks are *)
(* lookups - may this buffer go, and are these bytes still needed - and one*)
(* allocation carrying two sends is unrepresentable rather than something  *)
(* an invariant has to forbid.  Without the link nothing could forbid      *)
(* releasing the memory of a message still on its way to the wire, which   *)
(* is a use-after-free no counter could have caught.                       *)
(***************************************************************************)

VARIABLES
    buffers_held_by_host,        \* per call: buffers lent, not yet committed
    write_dones_emitted,         \* per call: WRITE_DONEs emitted, monotone
    write_done_callback_running, \* per call: WRITE_DONE callback on stack
    delivery_callback_running,   \* per call: delivery callback on stack
    payloads_consumed_by_host,   \* per call: payloads released, monotone
    handle_released,             \* per call: the runtime retired the handle
    cancel_requested,            \* per call: cancellation latched
    shutdown_event_emitted,      \* per runtime: SHUTDOWN_COMPLETE went out
    shutdown_callback_running,   \* per runtime: its callback on stack
    runtime_destroyed,           \* per runtime: ak_runtime_destroy happened
    buffer_state,                \* per call, per buffer: its lifecycle state
    buffer_send,                 \* per call, per buffer: the send it carries
\* The release signal.  SHUTDOWN_COMPLETE says the runtime stopped running and
\* carries whether the host still holds any of its memory; that answer is
\* recorded because the second event is owed only when it was yes.  Two
\* observable statuses refine one level-0 RELEASED: STOPPED when something is
\* still out, QUIESCENT when nothing is - and the difference lives here, level 1
\* never writing level-0 state.
    second_event_owed,    \* per runtime: the tag SHUTDOWN_COMPLETE carried
    resources_released_emitted,  \* per runtime: RESOURCES_RELEASED went out
    resources_released_callback_running, \* per runtime: its callback on stack

\* Two quantities, because they are not the same number.  buffer_length is what
\* ak_buffer.len exposes and what a commit must fit inside; buffer_charge is
\* what the allocator handed out and what the budget counts.  The allocator may
\* round the request up, so charging the length would bound a fiction, and
\* checking a commit against the charge would admit a message the view cannot
\* hold.  CoversRequest ties them at the lend and nothing relates them after.
\* The emission budget, in bytes.  buffer_charge records what the allocator
\* handed out for a buffer, written once when the buffer is lent and read by
\* the commit and the free; memory_used is the runtime-wide counter, kept the
\* way the implementation keeps it - an independent quantity moved by the lend
\* and the free, not a sum evaluated on demand.
\* Independent is the point.  MemoryAccountingExact ties the counter to the
\* charges of the buffers actually out, and it can fail: a path that forgets
\* an increment, forgets a decrement, credits twice, or publishes a snapshot
\* between two updates breaks it.  Defined as the sum it would be a tautology;
\* the ABI publishes this counter, so a host builds on it, and a published
\* number that can drift is a promise someone will rely on.
\* The emission path only.  Receive-side memory belongs to hyper and is
\* governed by the HTTP/2 flow control window, not by anything the ABI can
\* refuse against.  On that side an allocation failure runs Rust's allocation
\* error hook and aborts, so there is no refusal to model; the emission path
\* uses the fallible allocator APIs and reports AK_STATUS_INTERNAL instead.
\* Global rather than per runtime: RuntimeCreate requires every other runtime
\* destroyed, so at most one is ever outstanding, and the
\* counter reaching zero at destroy is proved rather than assumed - quiescence
\* leaves no buffer out, and the accounting reads the counter off that.
    last_lend_status,            \* per call and message: the lend's answer
    buffer_charge,               \* per call, per buffer: the bytes allocated
    buffer_length,               \* per call, per buffer: the bytes exposed
    memory_used                  \* runtime-wide: bytes lent and not yet freed

=============================================================================
