# ABI — `armonik-transport-ffi`

The normative C ABI: its principles, its entry points, the configuration document, the
sequences a host follows, and what it does not carry yet. How the crate implements it is
[architecture.md](architecture.md); where each entry point acts in the formal model is
[formal-model.md](formal-model.md).

## Layer 3 — `armonik-transport-ffi`

### Principles

- The FFI runtime owns a Tokio runtime and hands its handle to the GrpcChannel
- Every spawned task is registered in a task group (joinable at shutdown)
- Handles are `uint64_t` tokens validated in an internal registry. Each is drawn from a
  monotonic counter and never handed out twice, so a token whose object has been reclaimed
  names nothing rather than aliasing whatever came after it. The three kinds draw from
  disjoint ranges of the same 64 bits - runtimes below 2^32, channels to 2^63, calls above -
  so a handle of one kind is absent from the others' tables, and telling the kinds apart is
  a range test
  The August design specified a slot map - index plus per-slot generation, chained free
  list - chosen for O(1) allocation. That was reversed on 2026-09-05. The generation was
  never a goal: it is the repair for the aliasing that reusing an index causes, and reusing
  an index is what bounds the memory of an array addressed by a counter. Three of the
  defects found reviewing this crate lived in that machinery - a generation that wrapped
  after 2^32 reuses of one slot, a publish that could land on a freed slot, a free list
  that could take one index twice. A counter has none of those paths, and the O(1) it gives
  up is a hash lookup on a path that runs a handful of times per call, against a network
  round trip. The event path never touches the registry at all: payloads and lent buffers
  are identified by a tagged owner pointer, so the place where O(1) would have earned its
  keep was already not using it.
- A released channel leaves its table once it is CLOSED, and its handle names nothing from
  then on. No level has a step for it: the refinement maps a channel handle the table no
  longer holds to the closed state it left in, which a history variable recording the release
  carries. A channel the shutdown closed stays in the table, because the host still names it -
  `ak_call_start` on it answers `AK_STATUS_INVALID_STATE`, as a closed channel's does
- **No deadlock crosses the ABI.** A deadlock needs a cycle, and one through the boundary has
  three ways to form; each is closed:
  1. Rust holding a lock while it calls the host. The callback may make a downcall that needs
     that lock, or a host thread in a downcall may wait for it while the callback waits for that
     thread. No callback starts with a lock of this crate held: in a debug build every lock is
     counted per thread, and emitting an event with one held panics, so the test suite checks
     the rule at every event.
  2. The host holding a lock the trampoline needs. The binding's trampoline takes none: the
     ring's signal swaps its task atomically, and a WRITE_DONE is a counter and a completion.
  3. A downcall waiting for a callback. The header forbids it, and the one downcall that
     blocks, `ak_runtime_destroy`, is accepted only at QUIESCENT, when every callback has
     returned. An end of sending waits only for a send downcall still queueing, and that send
     waits on nothing.
  The other locks - the handle tables, a channel's phase, the runtime's tokio and teardown
  slots - are held for a few instructions; the runtime's gate is held across a channel's
  creation and a call's start, and neither emits. None is held across a callback, so they cost
  waits and not deadlocks.

- Received message payloads are **owned**: the host receives an `ak_bytes` that it must
  release. This prepares for future zero-copy (the host will be able to deserialize directly
  from the native buffer before releasing).

### JSON configuration schema

The JSON schema is generated from `options::ChannelOptions` and `options::TransportOptions`,
the option document rather than the engine's own configuration. `CallStartOptions`
is deliberately outside it: it carries a `Deadline`, whose `Absolute(Instant)` variant is a
process-local monotonic point with no portable serialization and no meaning in another
address space. Per-call options cross the ABI as fields, not as JSON, and a serialized
deadline - in a retry policy for instance - is always a relative `Duration`.
The schema is the source of truth for:
- C# options (generated from the schema)
- Options documentation
- Rust-side validation at channel creation

Note: `RetryConfig` appears both in `GrpcChannelConfig` (channel default) and, post-V1, as a
per-call override. Only the type is shared with the schema; the per-call override travels as
an ABI field like the rest of `CallStartOptions`.

The schema is committed at `packages/rust/armonik-transport/options.schema.json`.

### FFI entry points (complete V1 list)

```c
// === Status ===
// Returned by every entry point that can fail. The exceptions are ak_event_consumed
// and ak_return_call_buffer, which are void because a wrong token is a host bug the
// ABI cannot report anywhere useful, and ak_runtime_status and ak_abi_version, which
// return their answer. One prefix for the whole enum: a
// value called AK_RUNTIME_BUSY would read as an ak_runtime_state member, and a
// value called SLOT_BUSY would read as nothing at all.
typedef enum {
    AK_STATUS_OK            = 0,
    AK_STATUS_HANDLE_STALE  = 1,  // the object is gone; the token names nothing
    AK_STATUS_SLOT_BUSY     = 2,  // this call's send window is full - backpressure,
                                  // not an error; retry when a WRITE_DONE arrives
    AK_STATUS_INVALID_ARG   = 3,  // a null pointer, or a struct whose size prefix
                                  // does not match any known version
    AK_STATUS_INTERNAL      = 4,  // a fault the ABI cannot attribute, including a
                                  // genuine allocator failure
    AK_STATUS_BUDGET_BUSY   = 5,  // the runtime-wide byte ceiling is reached - not
                                  // necessarily by others: this call's own in-flight
                                  // sends hold budget too; poll
                                  // ak_runtime_memory_usage and retry
    AK_STATUS_INVALID_STATE = 6,  // a valid handle at the wrong moment: destroy
                                  // before quiescence, a start while stopping, a
                                  // send after the terminal, a second end_send. A
                                  // guard refused, which is not a fault - calling it
                                  // INTERNAL would blame the runtime
    AK_STATUS_MESSAGE_TOO_LARGE = 7,  // the request cannot be satisfied at any
                                  // moment: len exceeds the ceiling itself, so no
                                  // return by anyone will ever make room. Permanent
                                  // where BUDGET_BUSY is transient - do not retry
} ak_status;

// === Runtime lifecycle ===

// Creates a runtime. Synchronous. The runtime transitions to RUNNING.
// callback + runtime_ctx remain valid until the last event of the runtime:
// AK_EVENT_SHUTDOWN_COMPLETE when host_debt says AK_HOST_NOTHING_TO_RETURN,
// AK_EVENT_RESOURCES_RELEASED otherwise. A binding may hold it longer - see the
// callback typedef - but not shorter.
ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_runtime_handle *out);

// Returns the current state of the runtime. Synchronous, non-blocking, thread-safe.
// The handle stays valid for this call until ak_runtime_destroy.
//
// This is the guarantee criterion, and no callback can be: a callback runs on the
// runtime's own thread, so it is by construction delivered while the runtime still
// has one. AK_RUNTIME_QUIESCENT is the only observation that means everything is
// gone - including that thread. The events are notifications; this is the gate.
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);

// Triggers shutdown. Closes the start gate, drains/cancels calls.
// The terminal AK_EVENT_SHUTDOWN_COMPLETE arrives via the callback.
// Idempotent - a second call is a no-op.
ak_status ak_runtime_begin_shutdown(ak_runtime_handle runtime);

// Destroys the runtime and frees everything it owns. Refused before
// AK_RUNTIME_QUIESCENT, and that is the only reason: quiescence already means
// the host has given everything back, so there is no separate memory check
// here. A host that still holds something sees AK_RUNTIME_GRPC_STOPPED from
// ak_runtime_status, which says the same thing earlier and says why - so no
// "busy" status is needed on this path, and none is defined.
//
// Live call and channel handles do NOT block it: destroy invalidates every
// handle of this runtime atomically, and a later downcall on one returns
// AK_STATUS_HANDLE_STALE rather than touching freed memory - which is what the
// unreissued tokens are for. The distinction is deliberate: a handle names
// runtime-owned state, so the runtime may reclaim it; a payload or a lent
// buffer is memory the host may still be reading or writing, so only the host
// can end it. There is nothing to forget on the handle side - the runtime
// reclaims a call by itself - while forgetting ak_event_consumed or
// ak_return_call_buffer keeps the runtime alive.
//
// The invalidation is proved, not merely asserted: DestroyedRuntimeRejectsHandles
// says no downcall on a call of a destroyed runtime is ever enabled again.
//
// After it returns AK_STATUS_OK the handle is invalid for every call including
// ak_runtime_status. Unloading the library is safe from AK_RUNTIME_QUIESCENT
// onwards; this call frees the runtime's own allocation on top of that.
// The escape is the same as elsewhere: from AK_RUNTIME_FAILED_UNQUIESCED it is
// refused outright, because nothing can promise the outstanding memory is idle.
ak_status ak_runtime_destroy(ak_runtime_handle runtime);

// === Channel ===

// Creates a channel from a config JSON. Synchronous and performs no I/O:
// connecting is a separate step, so creation fails only on a bad config.
// Lifecycle: freed by ak_channel_release.
ak_status ak_channel_create(ak_runtime_handle runtime,
                            ak_bytes_in endpoint,
                            ak_bytes_in config_json,
                            ak_channel_handle *out);

// Frees the channel. In-progress calls are cancelled (CANCELLED). The handle
// is reclaimed once the last call has reached its terminal; a channel the
// runtime's shutdown closed stays nameable until it is released.
void ak_channel_release(ak_channel_handle channel);

// How far along a channel's closing is; NONE for a handle this library does not
// know, which a released channel is once reclaimed. Observational: what ends
// CLOSING is this library's own bookkeeping - the last call of the channel
// reaching its terminal - so a host following the drain has nothing to do but
// read.
ak_channel_state ak_channel_status(ak_channel_handle channel);

// === Call ===

// Starts a gRPC call. call_ctx is returned in each callback for this call.
// The host MUST allocate call_ctx before this call and keep it valid until the terminal.
// If start fails (return != AK_STATUS_OK), no callback will be emitted for this call_ctx.
ak_status ak_call_start(ak_channel_handle channel,
                        const ak_call_start_options *options,
                        void *call_ctx,
                        ak_call_handle *out);

// Lends the host a buffer out of the call's arena to serialize into. The exact
// length is known before the first byte is written - the generated marshaller calls
// SetPayloadLength(CalculateSize()) - so no growable writer is needed.
// At most MaxSendsInFlight buffers out of one arena at a time (a channel option,
// default 1), counting both those the host is filling and those already committed
// and awaiting their WRITE_DONE: beyond that the downcall is refused with
// AK_STATUS_SLOT_BUSY, whose wake-up is this call's next WRITE_DONE. A second,
// unrelated refusal is AK_STATUS_BUDGET_BUSY: the runtime-wide byte ceiling is
// reached - not necessarily by others, this call's own in-flight sends hold
// budget too. No single event announces room, so the host polls
// ak_runtime_memory_usage and retries. Neither is an error.
// Retrying only makes sense while the request could ever fit. If len exceeds the
// ceiling itself, no return by anyone will ever make room, and the refusal is
// AK_STATUS_MESSAGE_TOO_LARGE - permanent, and not to be retried. In every refusal
// no buffer is lent and *out is untouched.
// A genuine allocator failure is none of these: it is AK_STATUS_INTERNAL and the
// runtime fails.
// Refused with AK_STATUS_INVALID_STATE once the call is over - a terminal call has
// nothing left to send -
// once cancellation has been requested, and once the call is reclaimed. Being
// refused on a call that has just ended is normal and not an error: the same
// race exists on ak_call_send_message.
// Lending only on a live call is also what makes destruction sound: a released
// runtime has no live call, so nothing can hand its memory back out.
ak_status ak_get_call_buffer(ak_call_handle call, size_t len, ak_buffer *out);

// Commits a lent buffer as the next message. Ownership passes back to Rust, which
// reads it in place. Refused once cancellation has been requested or the trailers
// have been received - the buffer must then go back through
// ak_return_call_buffer.
// When the allocation is freed is Rust's business and is not observable here: the
// engine copies the message out when it encodes it, and the allocation goes then.
// WRITE_DONE therefore says the slot is free, nothing about the memory.
// AK_EVENT_WRITE_DONE settles an accepted send and frees its slot, from the moment
// the event is emitted - not when the callback returns. It says nothing about the
// network: the message may have been written to the transport, or abandoned because
// the call was cancelled, the peer terminated it, or the connection closed. The
// acquittal is owed either way, which is what lets a cancelled call reach its
// terminal without leaving a send unaccounted for.
// The slot goes back to the send window on emission, which WriteDoneFreesASlot
// proves: a host woken by it may ask for a buffer immediately, from inside the
// callback if it wants to, and the window will not be what refuses it. The lend
// still has its own preconditions - the call active, no cancellation latched, the
// handle live - so this is capacity returned, not an allocation promised. Gating
// the slot on the callback's return would gate it on something the host cannot
// observe, which is how a lost wakeup becomes a deadlock.
// The callback still on the stack is a separate matter, and only two things depend
// on it: the runtime is not quiescent while one is running, and the terminal waits
// for every WRITE_DONE of the call to have returned.
// It always arrives, exactly once per accepted send, in send order, and always
// before the terminal event, even when the call fails or is cancelled.
ak_status ak_call_send_message(ak_call_handle call, ak_buffer buffer);

// Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the
// only exit for a buffer whose send is refused, and the call is not reclaimed
// until it happens.
// The owner field identifies the allocation, exactly as for ak_event_consumed.
void ak_return_call_buffer(ak_buffer buffer);

// Signals end of sending (END_STREAM). No more send_message after this.
ak_status ak_call_end_send(ak_call_handle call);

// Cancels the call. Produces a STATUS callback with code CANCELLED.
// Idempotent while the call is live - a second call is a no-op. Once the call
// has been reclaimed the handle is stale, so it returns AK_STATUS_HANDLE_STALE rather
// than nothing: the call is over, there is nothing left to cancel, and the host
// does not choose the moment reclamation happens.
// Cancellation is asynchronous: the request takes effect when the call's delivery
// task observes it (its next iteration), so MESSAGE callbacks already committed may
// still arrive after this downcall returns. No lock is required: the same task reads
// the request and emits the callbacks. From that observation point, received messages
// not yet delivered are dropped by Rust; no callback is emitted for them.
// INITIAL_METADATA is never skipped: a call cancelled before the response
// head still receives it before the CANCELLED terminal.
ak_status ak_call_cancel(ak_call_handle call);

// NORMALIZATION, not a property of the wire. gRPC allows a Trailers-Only
// response, where the server sends trailers and no headers at all; a cancelled
// or failed call may see nothing either. The ABI still emits exactly one
// INITIAL_METADATA per used call, first, synthesizing an empty one when the
// wire carried none. Hosts therefore need no special case, and the property the
// binding relies on - the first event of a call is always the metadata - holds
// at this boundary rather than being inherited from HTTP/2. A host that must
// tell them apart reads the event's status_code, an ak_head_origin: the peer's
// headers (zero, so a host that ignores the field takes every head for the
// peer's), a response that delivered none - Trailers-Only, whose trailers the
// terminal carries - or no response at all.
// The synthesized event is empty but not free: it takes a delivery credit like
// any other, so it carries a real owner and must be consumed. len == 0 with a
// non-NULL owner is the normal shape here.

// There is no ak_call_release, on purpose. Every resource a call lends out is
// given back by an event the runtime already observes - ak_event_consumed for a
// payload, ak_call_send_message or ak_return_call_buffer for a buffer, the
// return of its own callback - so the runtime knows when a terminal call owes
// nothing, and reclaims the handle and the arena itself. A downcall would only
// have restated a verdict the runtime already holds.
//
// Two consequences the host must know. The handle goes stale at a moment the
// host does not choose; a later downcall on it returns AK_STATUS_HANDLE_STALE, which
// an unreissued token makes safe rather than faulting. And abandoning a call
// is still ak_call_cancel, then consume through to the terminal - reclamation
// waits for what the host holds, so dropping a payload on the floor leaks the
// arena exactly as it did before.
//
// Reports what the call still owes. Purely observational: it changes nothing,
// and it is legal to never call it. It exists because without a release downcall
// nothing reports a forgotten ak_return_call_buffer synchronously, and an
// obligation with no way to check it is one that rots.
// Conformance tests and host assertions are the intended callers.
// AK_STATUS_HANDLE_STALE means the call is already reclaimed, which is to say the host
// owes nothing; a debug build keeps the slot as a tombstone so that answer is
// distinguishable from a garbage token.
typedef struct {
    uint32_t payloads_owed;       // delivered, not yet ak_event_consumed
    uint32_t buffers_lent;        // out of the arena, not yet given back
    uint32_t callbacks_in_flight; // the runtime's own, informational
    int      terminal_delivered;  // 0 or 1
} ak_call_debt;

ak_status ak_call_debt_of(ak_call_handle call, ak_call_debt *out);

// What the runtime-wide byte ceiling is holding. Both forms are synchronous,
// non-blocking and observational: they change nothing the model carries.
//
// The base form is what a retry needs, and it needs nothing else. A buffer occupies
// the ceiling from ak_get_call_buffer until the runtime frees its bytes, and
// committing it with ak_call_send_message does not free anything - it hands the same
// bytes from the host to the runtime. So only a fall in the total proves capacity
// came back, and a single number carries that.
typedef struct {
    uint64_t bytes_used;      // occupied against the ceiling, atomic snapshot
    uint64_t ceiling;         // the configured limit
} ak_memory_usage;

ak_status ak_runtime_memory_usage(ak_runtime_handle runtime, ak_memory_usage *out);

// The detailed form is for observability, not for progress: it says why the ceiling
// is held, so an operator can tell a stuck host from a slow network. The three
// categories are the buffer lifecycle, and each says who has to move next:
//
//   bytes_host_lent      the host holds these and has neither committed nor
//                        returned them. No runtime step will move them; the host's
//                        own code must.
//   bytes_send_in_flight committed, and the send they carry is not acquitted yet.
//                        The transport still needs the bytes; a WRITE_DONE moves
//                        them to the next category.
//   bytes_runtime_held   given back and not yet freed - returned unused, or carrying
//                        a send already acquitted. The host has nothing left to do
//                        here. In the model FreeReturnedBuffer is enabled on all of
//                        these and weakly fair, so the category drains on its own;
//                        an implementation that keeps an acquitted send's bytes for
//                        replay until the commitment point holds part of it longer,
//                        which is why the name says held rather than freeable.
//
// The first two fields of ak_memory_usage_detailed are the base struct's, in the
// same order, so a host upgrades by changing the call and the type and re-reading
// nothing.
//
// Normative: the snapshot is coherent - all five numbers are read from one instant
// of the runtime's accounting - and
//     bytes_host_lent + bytes_send_in_flight + bytes_runtime_held == bytes_used
//     bytes_used <= ceiling
// hold exactly on every returned snapshot, not merely eventually. A host may
// therefore compare fields across categories without a second call.
//
// Normative here means an ABI obligation, checked by the ABI tests. The two
// identities are proved at level 1 (MemoryAccountingExact, CategoriesPartitionTotal,
// MemoryWithinCeiling); what stays a test obligation is the snapshot itself -
// that one read returns one coherent instant. See "What is actually verified" in
// formal-model.md.
typedef struct {
    uint64_t bytes_used;
    uint64_t ceiling;
    uint64_t bytes_host_lent;
    uint64_t bytes_send_in_flight;
    uint64_t bytes_runtime_held;
} ak_memory_usage_detailed;

ak_status ak_runtime_memory_usage_detailed(ak_runtime_handle runtime,
                                           ak_memory_usage_detailed *out);

// Both answer on a failed runtime - a host wants the accounting there most of all -
// and both return AK_STATUS_HANDLE_STALE after ak_runtime_destroy, the handle
// naming nothing by then.

// === Utilities ===

// ABI version. To compare with AK_ABI_VERSION compiled into the binding.
int ak_abi_version(void);

// Signals that the host has consumed the payload of an event. Dual semantics:
// 1. Frees the native memory (Rust deallocs the buffer)
// 2. Arms reception of the next event for this call (demand signal)
// At most DeliveryCredits non-consumed payloads per call (a channel option,
// `delivery_credits` in the channel's config JSON, default 1) - while the host owes that many, the runtime withholds the next data
// callback; only a terminal may still go out with every credit spent.
// The payload pointer identifies the allocation to free; the host MUST release
// in delivery order, so with several credits the oldest outstanding payload is
// always the next one to be consumed.
// The terminal does not invalidate payloads already handed over: calling
// ak_event_consumed remains legal after the terminal, and is in fact required
// before the call can be reclaimed.  After a runtime failure no promise that
// reads a runtime state survives, but this call does - ak_event_consumed stays
// legal and BufferEventuallyFreed still holds, because a failed runtime disables
// neither returning memory nor freeing it.
void ak_event_consumed(ak_bytes payload);
```

### Detailed ABI surface

```c
// === Handles ===
// Handles are tokens, not pointers. Each names one object and is never handed
// out again, so a handle whose object is gone is detected and refused with
// AK_STATUS_HANDLE_STALE rather than dereferenced - which is what lets a downcall on a
// reclaimed call report a status instead of faulting. ABA cannot arise: nothing
// comes back to a name. Their layout is opaque and must not be interpreted; only
// the values the ABI hands out are valid, and AK_HANDLE_NONE is the null token.
// ak_runtime_destroy stales every handle of the runtime at once, including the
// call handles: no downcall on any of them is accepted afterwards, which is
// DestroyedRuntimeRejectsHandles at level 1.
typedef uint64_t ak_runtime_handle;   // freed by ak_runtime_destroy
typedef uint64_t ak_channel_handle;   // freed by ak_channel_release
typedef uint64_t ak_call_handle;      // reclaimed by the runtime

#define AK_HANDLE_NONE ((uint64_t)0)

// Token chosen by the host, passed to ak_call_start, returned in each callback
// for this call. It is an opaque void* - Rust never dereferences it.
// The host puts whatever it wants there: GCHandle (.NET), GlobalRef (Java), id (Python).
// No native lifecycle - the host manages the pointed object.
// Must remain valid until reception of the terminal (AK_EVENT_STATUS) for the call.
typedef void *ak_call_ctx;

// === Buffers ===

// Lent to the host by ak_get_call_buffer, out of the call's arena. The host
// writes len bytes into it and gives it back exactly once, either by
// ak_call_send_message or by ak_return_call_buffer. Rust never reclaims a
// lent buffer on its own - not on cancellation, not on channel close - which
// is what removes any race between a writing thread and a cancelling one.
// owner identifies the allocation, as in ak_bytes.
typedef struct {
    uint8_t *ptr;           // writable, len bytes
    size_t len;             // exactly the length asked for
    void *owner;            // opaque - passed back as-is
} ak_buffer;

// Owned by the host after reception. The host MUST call ak_event_consumed
// exactly once when it has finished consuming the data.
//
// owner: opaque handle to the underlying Rust allocation. The ptr/len
// is a read-only view on bytes that may be a subset of a larger allocation
// (e.g., an Arc<Vec<u8>>). It is owner that identifies what to free -
// ptr alone is not enough because it may point into the middle of a
// reference-counted allocation. The host passes owner unchanged to
// ak_event_consumed.
typedef struct {
    const uint8_t *ptr;     // read-only view
    size_t len;             // number of readable bytes at ptr
    void *owner;            // opaque - passed as-is to ak_event_consumed
} ak_bytes;

// === Errors ===

// Why a fallible entry point refused. A status says whether a call worked and,
// when it did not, whether waiting would help; it cannot carry a message or
// name a family, which is what requirements 11.1, 11.2 and 11.5 ask for.
typedef enum {
    AK_ERROR_CONFIG     = 1,  // the configuration document, before any socket
    AK_ERROR_CONNECTION = 2,  // DNS, TCP, TLS handshake
    AK_ERROR_TRANSPORT  = 3,  // HTTP/2 or gRPC framing, after a connection
    AK_ERROR_TIMEOUT    = 4,
    AK_ERROR_CANCELLED  = 5,
} ak_error_kind;

// Filled by this library, read by the host. Fixed layout: the two sides agree
// through ak_abi_version() at load time, so there is no size prefix to read -
// that mechanism serves the records the host fills, and this one travels the
// other way.
typedef struct {
    int32_t kind;      // ak_error_kind
    ak_bytes detail;   // UTF-8, the cause chain flattened into one message.
                       // detail.owner == NULL means there is nothing to free,
                       // which is how a constant message crosses without an
                       // allocation. Released by ak_error_release, never by
                       // ak_event_consumed: a refusal is not a delivery, and it
                       // takes no delivery credit.
} ak_error;

// No release callback travels in the struct. The host would copy a live code
// pointer into its own memory, and only quiescence permits unloading this
// library: a host that retires the runtime before formatting the message would
// call into an unmapped page. A symbol the host's own loader resolved keeps the
// module referenced for as long as its stub exists.
//
// A no-op when detail.owner is NULL, so a host may route every error through it.
void ak_error_release(ak_bytes detail);

// === Events ===
typedef enum {
    AK_RUNTIME_RUNNING           = 1,  // operational, accepts channels and calls
    AK_RUNTIME_GRPC_STOPPING     = 2,  // start gate closed, channels closing
    AK_RUNTIME_GRPC_STOPPED      = 3,  // Hyper and Tonic are done; a dispatch
                                       // thread may still carry one last event
    AK_RUNTIME_QUIESCENT         = 4,  // and nothing of it is outstanding either
    AK_RUNTIME_FAILED_UNQUIESCED = 5,  // quiescence impossible, destroy refused
} ak_runtime_state;
// Stopped and destructible are two different facts - the first is about the
// runtime's own activity, the second about what the host has given back - and
// one enum value cannot carry both, so there are two. STOPPED is the functional
// shutdown: no channels, no connections, no gRPC task running, and it needs no
// ownership return from the host. It does not mean no thread is left: the dispatch
// thread survives to carry AK_EVENT_RESOURCES_RELEASED when that is owed. QUIESCENT is STOPPED plus an empty ledger: every payload
// consumed, every buffer given back and released. Only QUIESCENT permits
// ak_runtime_destroy or unloading the library; a new runtime additionally
// requires the old one destroyed, so its handle and its counters are gone.
//
// The host reaches QUIESCENT by acting, not by waiting: while it still holds
// something the status stays STOPPED, and the AK_EVENT_SHUTDOWN_COMPLETE
// callback says so through its host_debt field. Polling for QUIESCENT before
// returning what it holds is therefore a deadlock, and that field is what stops a host
// from writing one.
//
// There is no DRAINING between STOPPING and STOPPED. It had no observable
// boundary distinct from STOPPING - "awaiting quiescence" is what STOPPING
// already means - and a status the model does not define is one no two
// implementations would return at the same moment.
// NOT_INITIALIZED is not an observable state: before a successful ak_runtime_create,
// the host has no handle. There is no state after QUIESCENT either: the handle
// stops existing at ak_runtime_destroy rather than entering a released state
// that is still legal to query.
//
// Both STOPPED and QUIESCENT refine one level-0 state, RELEASED. The level-0
// model has no notion of a handle to free, so the distinction between them is
// carried entirely by level-1 variables - which is also the only shape the
// refinement rule allows, level 1 never writing level-0 state.

typedef enum {
    AK_EVENT_INITIAL_METADATA   = 1,  // payload = metadata blob (owned), status_code = its origin
    AK_EVENT_MESSAGE            = 2,  // payload = message bytes (owned)
    AK_EVENT_STATUS             = 3,  // terminal - payload = status + trailing metadata (owned)
    AK_EVENT_WRITE_DONE         = 4,  // the accepted send is settled; its slot is already free
    AK_EVENT_SHUTDOWN_COMPLETE  = 5,  // the runtime has stopped running
    AK_EVENT_RESOURCES_RELEASED = 6,  // and now nothing of it is outstanding
} ak_event_kind;

// Carried by AK_EVENT_INITIAL_METADATA in status_code: where the head came
// from - see the normalization note at ak_call_cancel. Zero is the peer's
// headers, so a host that ignores the field takes every head for the peer's.
typedef enum {
    AK_HEAD_RECEIVED      = 0,  // the peer's response headers
    AK_HEAD_TRAILERS_ONLY = 1,  // a response came and delivered no head; the
                                // terminal is the call's status, the peer's
                                // unless the call was stopped here first
    AK_HEAD_NO_RESPONSE   = 2,  // no response reached the call
} ak_head_origin;

// Carried by AK_EVENT_SHUTDOWN_COMPLETE and by nothing else: whether the host
// still holds memory of this runtime - a payload not yet consumed, or a buffer
// not yet given back.
//
// An enum and not a bitmask: it is one fact with two exclusive values, and a
// bitmask would invite a second flag that does not exist. AK_HOST_NOTHING_TO_RETURN
// is zero so that a zero-initialized event reads as "nothing outstanding": a
// runtime that forgot to set the field would make the host destroy too early
// and be refused, which is diagnosable, where the opposite default would make
// it wait for an event that never comes.
typedef enum {
    AK_HOST_NOTHING_TO_RETURN = 0,  // nothing of ours is in your hands; no second event
    AK_HOST_MUST_RETURN       = 1,  // consume the payloads, return the buffers;
                                    // AK_EVENT_RESOURCES_RELEASED follows
} ak_host_debt;
// Neither value permits destroying. The field answers one question - has the
// host work to do - and it is read inside the callback, where the status is
// still STOPPING because ak_runtime_release has not run yet. The permission is
// ak_runtime_status() == AK_RUNTIME_QUIESCENT and nothing else, in both cases.
// AK_HOST_NOTHING_TO_RETURN does not even mean everything is freed: it is
// computed from what the host holds, and the runtime may still be releasing the
// bytes of buffers returned earlier.
// AK_EVENT_RESOURCES_RELEASED is emitted only when host_debt said
// AK_HOST_MUST_RETURN. When it said AK_HOST_NOTHING_TO_RETURN there is nothing
// left to announce, and the
// host has already been told everything it needs.
//
// It is a distinct kind rather than a second AK_EVENT_SHUTDOWN_COMPLETE because
// a host has to be able to tell the two apart to know when its context may go:
// one that freed on the first of two identically-tagged events would hand the
// second a dangling pointer.
// WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED carry no payload. The
// field is still present and is the empty, unowned value: ptr == NULL,
// len == 0, owner == NULL.
// ak_event_consumed on it is a no-op rather than an error, so a host may route
// every event through one path without a special case, and owner == NULL is the
// single test for "there is nothing to give back".
//
// owner == NULL means unowned, never "empty". A zero-length payload that took a
// delivery credit - the synthesized INITIAL_METADATA of a Trailers-Only
// response is exactly that - carries a non-NULL owner and MUST be consumed,
// because the credit comes back with the acquittal and not with the bytes.
// Reading len instead of owner is how a call would silently stop receiving.

// Passed on the stack in the callback - no own lifecycle.
// The payload field is owned and must be released by the host.
typedef struct {
    ak_event_kind    kind;
    ak_bytes         payload;      // owned - host must call ak_event_consumed
    int32_t          status_code;  // grpc status (AK_EVENT_STATUS), ak_head_origin
                                   // (AK_EVENT_INITIAL_METADATA), zero otherwise
    ak_host_debt     host_debt;    // AK_EVENT_SHUTDOWN_COMPLETE only
} ak_event;

// === Callback ===
// Lifecycle of the function pointer: must remain valid for the runtime's lifetime.
// Lifecycle of runtime_ctx, as the ABI requires it: valid until the last event of
// the runtime - AK_EVENT_SHUTDOWN_COMPLETE when host_debt says
// AK_HOST_NOTHING_TO_RETURN, AK_EVENT_RESOURCES_RELEASED when it says
// AK_HOST_MUST_RETURN. Freeing it on the shutdown event without reading that field
// is a use-after-free. That is the minimum; the .NET binding holds it longer and
// releases it after ak_runtime_destroy returns, which needs no reasoning about
// which event was last. Both satisfy the ABI - the rule here is the floor, not
// the policy.
typedef void (*ak_callback)(
    void *runtime_ctx,
    void *call_ctx,
    const ak_event *event);
// The callback receives an event whose payload is owned when there is one.
// The host MUST call ak_event_consumed on every payload whose owner is not NULL,
// after consuming the data: that frees the memory AND arms the next event.
// WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED carry no payload and owe
// nothing.
// Data callbacks (INITIAL_METADATA, MESSAGE, STATUS) are serialized per call and
// concurrent between calls. WRITE_DONE may arrive in parallel with any of them,
// including for the same call: a per-call lock in the handler would hold the
// slot release hostage behind a slow message handler.
```

### Unary call sequence

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
callState = new CallState(...)
gcHandle = GCHandle.Alloc(callState)
ak_call_start(channel, opts,       -> validates channel, creates GrpcCall,
              gcHandle, &handle)      registers in task group
                                     returns handle
ak_get_call_buffer(handle, n, &buf) -> lends n bytes out of the call arena
serialize into buf.ptr             // protobuf writes straight into native memory
ak_call_send_message(handle, buf)  -> ownership of buf passes back to Rust
                                   ... network: Rust sends over HTTP/2 ...
                          callback(runtime_ctx, gcHandle, &evt_w) <-
                            evt_w.kind = WRITE_DONE     [slot free]
ak_call_end_send(handle)           -> signal end_send
                                   ... network ...
                          callback(runtime_ctx, gcHandle, &evt1) <-
                            evt1.kind = INITIAL_METADATA  [auto, before any message]
                            evt1.payload = ak_bytes{ptr, len, owner}
ak_event_consumed(evt1.payload)    // free + arm next
                          callback(runtime_ctx, gcHandle, &evt2) <-
                            evt2.kind = MESSAGE
                            evt2.payload = ak_bytes{ptr, len, owner}
// host can deserialize directly from evt2.payload.ptr (zero-copy recv)
ak_event_consumed(evt2.payload)    // free + arm next
                          callback(runtime_ctx, gcHandle, &evt3) <-
                            evt3.kind = STATUS  [terminal, end of stream]
                            evt3.payload = ak_bytes{ptr, len, owner}
                            evt3.status_code = 0 (OK)
                            [this callback frees gcHandle after its
                             last access - it is the call's last]
ak_event_consumed(evt3.payload)    // free (no next, this is the terminal)
                            [the actor sees the debt cleared and reclaims
                             the handle and the arena; the host does
                             nothing, and its handle is now stale]
```

FFI note:
- **Send**: the host serializes into a buffer lent by `ak_get_call_buffer` and gives it back
  exactly once, by `ak_call_send_message` or `ak_return_call_buffer`. At most
  `MaxSendsInFlight` buffers out of one arena (default 1); WRITE_DONE acquits in send order,
  always arrives, exactly once per accepted send, and always before the terminal event, even
  on error or cancellation. There is nothing to pin: the memory is Rust's from the start.
  See Zero-copy in architecture.md.
- **Receive (demand via consumed)**: the `ak_bytes` payload is owned. The host consumes
  (deserializes directly from the native pointer) then calls `ak_event_consumed`. This
  call frees the memory AND arms reception of the next event. At most `DeliveryCredits`
  non-consumed payloads per call (default 1) — this is the backpressure mechanism.
The terminal `AK_EVENT_STATUS` may arrive instead of a next MESSAGE (end of stream or error).

### Shutdown sequence

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
ak_runtime_begin_shutdown(rt)      -> closes the start gate
                                     cancels/drains calls
                                     awaits gRPC quiescence
            callback(ctx, 0, SHUTDOWN_COMPLETE, host_debt) <-
                                     the callback returns
                                     |
     +-----------------------------------+
     |  these two are independent, in either order:  |
     |                                   |
     |  the runtime publishes AK_RUNTIME_GRPC_STOPPED
     |  (Hyper and Tonic are done; the dispatch thread is not)
     |                                   |
     |  if host_debt == AK_HOST_MUST_RETURN:
     |    the host consumes every payload and returns every buffer,
     |    the runtime frees what came back, and then
     |    callback(ctx, 0, RESOURCES_RELEASED) <-
     +-----------------------------------+
                                     |
                                     both done -> AK_RUNTIME_QUIESCENT
loop:                                  // mandatory in both host_debt cases
  state = ak_runtime_status(rt)
  if state == AK_RUNTIME_QUIESCENT: break
  yield/spinwait
ak_runtime_destroy(rt)             -> AK_STATUS_OK
free the runtime_ctx root          // after destroy, never in a callback
// Safe unload; destroy has run, so a new runtime may start
```

Releasing comes before polling, and that is the whole point of the host-debt field. The
functional shutdown waits for the callbacks to return, never for unconsumed payloads: an
unconsumed payload does not hold `AK_RUNTIME_GRPC_STOPPED` back, and `ak_event_consumed` stays
legal throughout, so the shutdown chain still completes without the host doing anything.
What an unconsumed payload does hold back is `AK_RUNTIME_QUIESCENT` - the memory gate is
in the status, not in `ak_runtime_destroy`, which now refuses for one reason only. A host
that polled first and released second would wait forever, which is why the runtime says
so in the event rather than leaving it to be discovered.
`SHUTDOWN_COMPLETE` is emitted only once every channel is closed, every delivery callback
has returned and every accepted send has had its WRITE_DONE delivered and that callback
returned too: it is the last callback of the functional shutdown, and the last one
outright when its `host_debt` field says `AK_HOST_NOTHING_TO_RETURN`. A buffer merely lent and never committed is not an accepted send and holds
nothing back here - it holds back the call's reclamation, and through it
`ak_runtime_destroy`.

## What is missing

**The header is the contract; two things it needs are still missing.** The header lives at
`packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h` and is committed so an ABI
change shows up in review; T3.6 makes it generated from `abi.rs` and the commit verified by
regenerating and comparing, which is the arrangement `options.schema.json` already has - review
sees the diff either way, and only the hand-written arrangement can go stale in silence.

It settles what this section used to list as undecided: `ak_bytes_in` as the borrowed mirror of
`ak_bytes`, a size prefix on every options struct the host fills, `ak_runtime_config` and
`ak_call_start_options`, the metadata blob as a length-prefixed key/value sequence, and the
`AK_EVENT_STATUS` payload as a length-prefixed reason followed by the trailing metadata - the
code itself is `ak_event.status_code`.

That prefix is a `uint32_t struct_size` alone, compared for exact equality, so a host
compiled against any other revision is refused in both directions and no addition can ever be
additive. Requirement 13.3 promises additivity and 13.5 asks for `size` + `version` + `flags`
+ reserved validated to zero with the size checked as a minimum, which is what the record the
host fills needs; the record this library fills has a fixed layout and `ak_abi_version()` for
its agreement.

What is still owed:

- an ownership matrix: for each ABI object, who allocates, who frees, and when it stops
  being legal to touch. The header states each rule against its own entry point; nothing
  gathers them;
- conformance tests exercised from C and C# against the same header, because an ABI that
  only its author's binding uses is not an ABI. `tests/layout.rs` pins the sizes and offsets
  a C compiler produces for the header and checks the declarations and the exports name the
  same set, which is not the same thing: nothing in this workspace compiles the header.

**What the ABI does not yet implement.** `ak_runtime_memory_usage_detailed` and its
five-field struct: its three categories need each buffer's position in its lifecycle
tracked, and it is an observability tool rather than one a retry needs. `AK_RUNTIME_FAILED_UNQUIESCED` has two producers, a shutdown
task that dies under its guard and a shutdown that cannot get the thread quiescence is
defined as, and two defensive readings: `ak_runtime_status` faulting, and a stored state it
cannot read.

`ak_error` and `ak_error_release` are specified above and implemented nowhere: every failing
entry point answers with a status alone, `From<ChannelError> for ak_status` sends everything
but `Closed` to `AK_STATUS_INVALID_ARG`, and the configuration reader returns an `Option`, so
it discards the reason for a refusal before anything could report it. The engine's error types
are careful and no host can read one.
