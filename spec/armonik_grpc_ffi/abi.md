# ABI — `armonik-transport-ffi`

The normative C ABI: its principles, the reasons behind its entry points, the configuration
document, the sequences a host follows, and what it does not carry yet. How the crate implements it is
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

The runtime's options have a schema of their own, `runtime.schema.json`, generated from
`armonik-transport`'s `options::RuntimeOptions` under its `schema` feature: `Endpoint`, the server a
channel created with an empty endpoint reaches, the two memory ceilings, `ChannelDefaults`, and
`Logging`, whose `Filter` selects the engine's logs. It
is the document `ak_runtime_create_from` loads from the sources an `ak_config` lists - files, the
environment, pairs and documents, a later one over an earlier one (configuration-loading.md) - and
the one the `armonik` client loads; `ak_runtime_create` takes the ceilings and the channel defaults
as the fields of `ak_runtime_config`, the cases of one document with no endpoint and no filter, and
its zero ceiling is the default where a loaded document refuses a zero. `RuntimeOptions.g.cs` is generated
from it with its encoding, which the .NET binding's `LoadConfigFromObject` writes. A source is a
document judged whole against the schema: a key no option declares, the root's included, a value of
the wrong type or out of the bounds the schema states and a missing mandatory field are refused by
their path, in every source and in a channel's own document. A file or a document is judged on the
section the `ak_config`'s prefix names, and the names of the environment and of pairs that start
with it; everything else is never looked at. The prefix is always the one given: an empty one takes
everything, so that the whole file is the engine's. `ak_config` defines no flag, so any bit is
refused, and it has no default prefix.

`ChannelDefaults` is a channel document every channel of the runtime is merged over, option by
option and the channel's winning, which `ak_runtime_config` carries as `channel_defaults_json`. A
struct merges field by field, but for one with a mandatory field, which is stated whole; an
alternative - how the server is verified, who the client is, which proxy - is an enum, which merges
as its payload does over the same alternative, and which is taken whole over another, so no
merge combines two alternatives into one neither stated. Options that only
bound one another, such as the two backoff bounds, merge as any option, and a merge where they
disagree is refused as a document stating both would be. Its schema is the channel's, so the
generator renders `RuntimeOptions.g.cs` with `--reuse` of the channel schema and refers to the
classes `ChannelOptions.g.cs` declares.

Note: `RetryConfig` appears both in `GrpcChannelConfig` (channel default) and, post-V1, as a
per-call override. Only the type is shared with the schema; the per-call override travels as
an ABI field like the rest of `CallStartOptions`.

The schema is committed at `packages/rust/armonik-transport/options.schema.json`.

### Where the declarations are

The declarations are `packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h`,
rendered by cbindgen from the crate's `src/abi.rs` and `src/lib.rs`, with each item's contract
in the doc comment it is rendered from. The header states what each entry point does and what it
refuses; the sections below state why, which proofs the promises rest on, and what is specified
and not built yet. For a declaration the header is the reference, and for its reasons this
document.

#### Status codes

One prefix for the whole enum, `AK_STATUS_`: a value called `AK_RUNTIME_BUSY` would read as an
`ak_runtime_state` member, and one called `SLOT_BUSY` would read as nothing at all.

The refusals divide by whether waiting helps. `AK_STATUS_SLOT_BUSY` and `AK_STATUS_BUDGET_BUSY`
are backpressure, and the budget is not necessarily held by others: a call's own sends in flight
hold it too. `AK_STATUS_MESSAGE_TOO_LARGE` asks the same ceiling for more than it is, so it is
permanent where `AK_STATUS_BUDGET_BUSY` is transient. Only a lend refused with it is woken
by `AK_EVENT_BUDGET_WAKE`; a resize's refusal is not (see Calls). `AK_STATUS_INVALID_STATE` is a guard that
refused - a destroy before quiescence, a start while stopping, a lend or a send on a call that
is over or cancelled, a second lend while the call's one buffer is held (see Calls), a second end
of sending - which is not a fault, and calling it `AK_STATUS_INTERNAL` would blame the runtime.
`AK_STATUS_INTERNAL` is the fault the ABI cannot attribute, a genuine allocator failure included.

#### Errors

A status says whether a call worked and, when it did not, whether waiting would help. It cannot
carry a message or name a family, which requirements 11.1, 11.2 and 11.5 ask for, so every entry
point that can refuse takes an `ak_error *out_error` as its last argument.

A host that passes NULL pays nothing for it: the message is rendered only when there is an
`ak_error` to write it into. A constant message crosses as itself, with a NULL owner, so the
refusals a host meets on its hot path - backpressure, a stale handle - allocate nothing either
way; only a message built from a cause chain, a refused document's for one, is allocated.

`AK_ERROR_NONE` is the family of the refusals the status already says everything about:
backpressure, a request larger than the ceiling, a fault the library cannot attribute.
`AK_ERROR_USAGE` is the host's misuse of the ABI - a null pointer, a stale handle, a downcall its
object refuses at that moment - kept apart from the families that describe the network, because a
retry may cure those and only a change of code cures this one.

The message carries no source location: the engine's errors record one for tracing, and
requirement 11.4 keeps it out of what crosses the ABI. Nor does it carry the credentials an
endpoint's userinfo may hold: a message that names the endpoint names it without them.

No release callback travels in `ak_error`. The host would copy a live code pointer into its own
memory, and only quiescence permits unloading this library: a host that retires the runtime
before formatting the message would call into an unmapped page. `ak_error_release` is a symbol
the host's own loader resolved, which keeps the module referenced for as long as its stub
exists. It is not `ak_event_consumed`: a refusal is not a delivery, and takes no delivery
credit.

#### Options records

A record the host fills - `ak_runtime_config`, `ak_call_start_options` - starts with
`struct_size`, `version`, `flags` and `reserved`, which is what requirement 13.5 asks of a record
that evolves. `ak_config` starts with the first three, and its `source_count` in place of
`reserved`.

The size is a minimum. A record longer than this library's definition is read up to that
definition's end and the rest ignored, so a field appended to a record serves a newer host on an
older library. The minimum is the size of these first definitions and stays it: a later library
that appends a field reads it as absent from a host compiled before the field existed, and a
shorter record than that was never valid.

`version` and `reserved` are refused when they are not zero, and `flags` when it sets a flag
this library does not define for the record, rather than ignored. A flag asks for a behaviour, and
a library that lacks it has to say so rather than run without it; `version` and `reserved` stay
free for a change no appended field can express. A flag that reads a field is refused too on a
record whose `struct_size` stops before that field, which would otherwise read as zero.

`ak_call_start_options` ends with `timeout_ns`, past its first definition, and read only under
`AK_CALL_HAS_DEADLINE`: a host compiled before the field states no deadline, and the channel's
default applies. The field is relative - the nanoseconds from `ak_call_start` - because an instant
of the host's clock means nothing in the library's, and zero is a deadline already passed rather
than none, which is what the flag is for.

`AK_CALL_ONE_REQUEST` declares that the request is exactly one message, as on a unary or a
server-streaming method. Its commit also ends the sending, `ak_call_end_send` is refused, and no
WRITE_DONE comes: the send is settled at the commit, and a lend after it is refused with
`AK_STATUS_INVALID_STATE` rather than `SLOT_BUSY`. The call sends nothing before its commit - the
lent buffer keeps room for the gRPC prefix ahead of what the host writes, and the commit makes it
the request's whole body - and nothing watches its deadline before it: a commit past the deadline
is accepted, and the call ends `DEADLINE_EXCEEDED`.

`AK_CALL_ONE_RESPONSE` declares that the response is at most one message, as on a unary or a
client-streaming method. A second one ends the call `INTERNAL` before anything decodes it or the
runtime is charged for it, and the status after the message is read past the runtime's first
memory threshold: a host may hold the message until the status is in, as the .NET binding's reader
does.

`AK_CALL_WAIT_FOR_READY` declares that the call waits for the channel to open a connection rather
than end `UNAVAILABLE` when it cannot reach its server, as gRPC's wait-for-ready has it and
`CallOptions.WithWaitForReady` sets it. It reads no field, so the record keeps its layout, and a
host built before the flag simply never sets it. The channel dials again, backing off as gRPC's
connection backoff does, and the call goes out on the first connection it opens. The wait ends
at the call's deadline (`DEADLINE_EXCEEDED`), on its cancellation, or when its channel is
released; otherwise it lasts as long as the server stays out of reach, so a call that must not
wait for good states a deadline - a failure that persists, such as a certificate the server does
not present, is waited out too. The flag does not help a call that reached a connection: what
breaks it after that ends it as it ends any other. `AK_ABI_VERSION` stays 1: the flag is additive,
and a library that predates it refuses it as an unknown flag.

A record this library fills, `ak_error` among them, has a fixed layout instead: `ak_abi_version()`
is the agreement, and the two sides agree at load time or they do not run.

A third kind, `ak_stats`, is sized by the host and filled by the library. It starts with the same
four fields, and the host sets `struct_size` to the size of its own definition, at least those
four, and the others to zero, which are refused otherwise. The library writes the eight-byte words that
lie within that size and sets `struct_size` to what it wrote, so a host built before a field
appended to the record reads that field as the zero it left, and a host built after it reads a
library that lacks it as the shorter answer it gets. Its fields past the head are all eight bytes,
in steps of eight, so that its layout is one on x86, x64 and arm whatever each aligns an eight-byte
integer to.

#### Runtime lifecycle

`ak_runtime_status` is the guarantee criterion, and no callback can be: a callback runs on the
runtime's own thread, so it is by construction delivered while the runtime still has one.
`AK_RUNTIME_QUIESCENT` is the only observation that means everything is gone, that thread
included. The events are notifications; the status is the gate.

`ak_runtime_destroy` is refused before quiescence and for no other reason. Quiescence already
means the host has given everything back, so no separate memory check is made there, and a host
that still holds something reads `AK_RUNTIME_GRPC_STOPPED`, which says the same thing earlier and
says why. Live handles do not block it: a handle names runtime-owned state, so the runtime may
reclaim it, while a payload or a lent buffer is memory the host may still be reading or writing,
so only the host can end it. That no handle of a destroyed runtime is accepted afterwards is
proved, not asserted: `DestroyedRuntimeRejectsHandles` says no downcall on a call of a destroyed
runtime is ever enabled again. Unloading the library is safe from `AK_RUNTIME_QUIESCENT` on, and
the destroy frees the runtime's own allocation on top of that. From
`AK_RUNTIME_FAILED_UNQUIESCED` the destroy is refused outright, because nothing can promise the
outstanding memory is idle.

#### Runtime states

Stopped and destructible are two facts - the first about the runtime's own activity, the second
about what the host has given back - and one value cannot carry both, so there are two.
`AK_RUNTIME_GRPC_STOPPED` is the functional shutdown: no channels, no connections, no gRPC task
running, and it needs nothing back from the host. It does not mean no thread is left: the
teardown thread survives to carry `AK_EVENT_RESOURCES_RELEASED` when that is owed.
`AK_RUNTIME_QUIESCENT` is stopped plus an empty ledger: every payload consumed, every buffer
given back and released.

There is no draining state between stopping and stopped. It had no observable boundary distinct
from `AK_RUNTIME_GRPC_STOPPING`, and a status the model does not define is one no two
implementations would return at the same moment. Nor is there one after quiescence: the handle
stops existing at `ak_runtime_destroy` rather than entering a released state that is still legal
to query, and from then on it reads `AK_RUNTIME_NONE`, the answer for every handle this library
does not know.

Stopped and quiescent both refine one level-0 state, `RELEASED`. Level 0 has no notion of a
handle to free, so the distinction is carried entirely by level-1 variables, which is the only
shape the refinement rule allows: level 1 never writes level-0 state.

#### The shutdown's two events

`ak_host_debt` is an enum and not a bitmask: one fact with two exclusive values, and a bitmask
would invite a second flag that does not exist. `AK_HOST_NOTHING_TO_RETURN` is zero so that a
zero-initialized event reads as nothing outstanding: a runtime that forgot to set the field would
make the host destroy too early and be refused, which is diagnosable, where the opposite default
would make it wait for an event that never comes.

Neither value permits destroying. The field answers one question - has the host work to do - and
it is read inside the callback, before the runtime is quiescent; the permission is
`ak_runtime_status` answering `AK_RUNTIME_QUIESCENT`, and nothing else. `AK_HOST_NOTHING_TO_RETURN`
does not even mean everything is freed: it is computed from what the host holds, and the runtime
may still be releasing the bytes of buffers given back earlier.

`AK_EVENT_RESOURCES_RELEASED` is a kind of its own rather than a second
`AK_EVENT_SHUTDOWN_COMPLETE`, because a host has to tell the two apart to know when its context
may go: one that freed on the first of two identically tagged events would hand the second a
dangling pointer.

#### Handles and contexts

A handle's layout is opaque and must not be interpreted: only the values the ABI hands out are
valid, and `AK_HANDLE_NONE` is the null token. One type, `ak_handle`, serves runtimes, channels
and calls. Each kind is counted in a range of its own and no value is reissued -
[decisions.md](decisions.md), "Exact handle format" - so ABA cannot arise, and a handle of the
wrong kind is refused as stale. A runtime's handle ends at `ak_runtime_destroy`, a channel's at
`ak_channel_release` once its last call has ended, and a call reclaims its own.

`ak_call_ctx` is whatever the host wants there: a `GCHandle` in .NET, a `GlobalRef` in Java, an
id in Python. It has no native lifecycle; the host manages what it points at.

#### Buffers and payloads

`owner`, not `ptr`, identifies an allocation, because the view may point into the middle of a
larger, reference-counted one; the host passes it back unchanged. This library never reclaims a
lent buffer on its own - not on cancellation, not on channel close - which is what removes the
race between a writing thread and a cancelling one. The same holds of the buffer a resize
hands back.

The payload of `AK_EVENT_WRITE_DONE`, `AK_EVENT_BUDGET_WAKE`, `AK_EVENT_SHUTDOWN_COMPLETE` and
`AK_EVENT_RESOURCES_RELEASED` is still present, as the empty and unowned value, so
`owner == NULL` is the single test for there being nothing to give back, and `ak_event_consumed`
on it is a no-op rather than an error. Reading `len` instead of `owner` is how a call would
silently stop receiving: a zero-length payload that took a delivery credit - the synthesized head
of a Trailers-Only response is exactly that - must be consumed, because the credit comes back
with the acquittal and not with the bytes.

#### The callback's context

`runtime_ctx` is valid, as the ABI requires it, until the runtime's last event:
`AK_EVENT_SHUTDOWN_COMPLETE` when its `host_debt` says `AK_HOST_NOTHING_TO_RETURN`,
`AK_EVENT_RESOURCES_RELEASED` otherwise. Freeing it on the shutdown event without reading that
field is a use-after-free. That is the floor, not the policy: the .NET binding holds it longer
and releases it after `ak_runtime_destroy` returns, which needs no reasoning about which event
was last.

#### The log callback

The engine's logs reach the host through a callback given when the runtime is created, in the
fields `log_callback` and `log_ctx` appended to `ak_runtime_config` and to `ak_config`
(observability.md has the reasons). It is never replaced or removed. Unlike the runtime's context
it is not tied to the runtime's last event: `log_ctx` and the function stay valid until
`ak_runtime_destroy` returns, because the engine's threads are gone by quiescence and the destroy
waits for a delivery a host's own thread has under way, and delivers nothing after. A refused
creation delivers what its configuration's load logged, then the same: nothing after the call
returns. The events a creation logs are delivered on the host's own thread, inside the call. The
callback must not call this library, and an event logged from inside it is dropped.

#### Calls

`ak_call_start` emits no callback for a `call_ctx` whose start it refused.

`ak_get_call_buffer` takes the most the host will write, which the generated marshaller knows
before the first byte - it calls `SetPayloadLength(CalculateSize())` - so no growable writer is
needed. The host says how many bytes it wrote when it commits, `ak_call_send_message(handle,
buf, written)`, and only those are sent, so the host must have written them: the buffer is not
zeroed, and nothing of it past `written` is read. A commit of more than the lend, or a
write past its end that changed the bytes the library put after it, which the commit,
`ak_return_call_buffer` and `ak_resize_call_buffer` check, is `AK_STATUS_CORRUPTED`: the memory around the buffer may be
corrupted, so the buffer is taken back without being freed and the runtime shuts down. The lend is refused with `AK_STATUS_INVALID_STATE` once the call is over or its cancellation
requested, and its handle is stale once it is reclaimed; being refused on a call that has just ended is normal
and not an error, and the same race exists on `ak_call_send_message`. Lending only on a live call
is also what makes destruction sound: a released runtime has no live call, so nothing can hand
its memory back out.

`AK_STATUS_INVALID_STATE` answers a second lend as well: a call has one buffer, and a lend is
refused while the host holds it. That is a host bug and not backpressure, and so is a lend from
another thread while a lend or a resize of the same call has not returned. A host woken by
`AK_EVENT_WRITE_DONE` or `AK_EVENT_BUDGET_WAKE` is not lending beside another: on a live call
neither event reaches it while a lend of the call is still being answered, so a refused lend
has given back everything it took, the call's one buffer included, and the host may ask again
at once.

A length of zero is refused with `AK_STATUS_INVALID_ARG`: an empty message needs no buffer, and
`ak_call_send_message` sends one when given the empty buffer, owner NULL and len 0. That send takes
a slot of the window and gets its `AK_EVENT_WRITE_DONE` like any other.

`ak_resize_call_buffer(buffer, new_len, keep, out)` is for a host that finds the length it asked
for was not the length it needed, whichever way: it exchanges the buffer for one of `new_len`
bytes and keeps the first `keep` bytes the host wrote, which must be at most what was lent and at
most `new_len`. It takes no call handle, as `ak_return_call_buffer` does: the buffer names its
call. On success `*out` is the new buffer, which the host gives back in its turn, and `buffer` is
the host's no longer - its memory may be lent again at once, so nothing is read or written
through it, and the one buffer a call holds, its slot of the window and its debt are the new
buffer's. The ledger sees the difference alone: the old charge is replaced by the new in one
step, so the two are never both counted and the room the old buffer held is not offered to
another call in between, and the ceiling is asked only for a growth. The ceiling counts the
charge and not the instant: the old arena is still allocated while the bytes are copied.

A refusal leaves `buffer` lent, charged and the host's, and `*out` as it was:
`AK_STATUS_BUDGET_BUSY` when the ceiling has no room for the growth now, `AK_STATUS_MESSAGE_TOO_LARGE`
when `new_len` is past the ceiling, `AK_STATUS_INVALID_STATE` on a call that is over or cancelled,
or one-request and committed, `AK_STATUS_INVALID_ARG` for a `new_len` of zero, a `keep` past it, a
null `out` or a buffer that is not lent, and `AK_STATUS_INTERNAL` for an allocator failure or for a
contained panic (below). A `keep` past what was lent, or a write past the end of the buffer that
changed the bytes the library put after it, is the overrun of a commit: `AK_STATUS_CORRUPTED`,
the buffer taken back unfreed, nothing carried over, the runtime shutting down.
`AK_STATUS_BUDGET_BUSY` here records no wait and owes no `AK_EVENT_BUDGET_WAKE`: a wait is the
lend's, made by a host that holds nothing, and one that holds a buffer while it waits is room the
others wait for. A host that waits gives the buffer back and lends the new length.
`ak_call_debt_of` counts one buffer lent before, during and after an exchange.

A panic the library contains in `ak_get_call_buffer`, `ak_call_send_message` or
`ak_resize_call_buffer` is answered by what the operation had done when it happened, and the
buffer is as that answer says:

- Before the buffer is used up - for a lend, before the host holds it; for a commit, before its
  arena is taken to be the message; for a resize, before the exchange is made, which includes the
  ceiling's charge, moved in one step or not at all - the answer is `AK_STATUS_INTERNAL`, a
  refusal like an allocator failure. A buffer the host held stays lent, charged and the host's,
  and `*out` as it was, so the host may retry or give the buffer back. A lend refused by a
  contained panic holds nothing: nothing is charged, no slot of the window is spent, the call's one
  buffer is free, no wait for room is recorded and `*out` is untouched, so the host may ask again.
- Once the operation is made - the message queued, or on a one-request call given; the exchange
  made - the answer is `AK_STATUS_OK`, whatever a panic in the rest of it does.
- Between the two, where the buffer is gone and the operation was not made - a commit whose arena
  was taken to be the message and is not queued, or an overrun being taken back - the answer is
  `AK_STATUS_CORRUPTED`: the buffer is taken back and the runtime shuts down.

A buffer that is gone has its debt paid, which is what lets that shutdown complete.
`ak_return_call_buffer` has no answer: its debt is paid whatever a panic meets, and a panic while
it reads the bytes after the end of the buffer treats the buffer as an overrun, taken back without
being freed, with the runtime shutting down.

A genuine allocator failure is none of these: it is `AK_STATUS_INTERNAL`, and the lend is
refused as the others are - nothing charged, no slot spent - while the runtime carries on. A
refused lend, whether for an allocator failure or a panic, pays what it took part by part,
whatever a panic in one part does to the others, so a runtime whose lend was refused still reaches
`QUIESCENT` with nothing owed and no byte charged.

A lend refused with `AK_STATUS_BUDGET_BUSY` leaves the call waiting on that length until a lend
of it succeeds or the call ends, and every release that gives bytes back meanwhile owes the call
an `AK_EVENT_BUDGET_WAKE`. While it waits, every call of the runtime reads only below the ceiling
lowered by that length, so the host must try again when woken or cancel the call: a send given up
on a live call holds reception lowered for the whole runtime. The event may arrive in parallel
with the call's data callbacks, like `AK_EVENT_WRITE_DONE`, and before its terminal; it promises
no room, since another call may take it first.

When `ak_call_send_message`'s allocation is freed is this library's business and is not
observable: a call that may be retried keeps it for a replay, past its WRITE_DONE, so
`AK_EVENT_WRITE_DONE` says the slot is free and nothing about the memory. The acquittal is
owed whatever became of the message - written, or abandoned because the call was cancelled, the
peer ended it or the connection closed - which is what lets a cancelled call reach its terminal
without leaving a send unaccounted for. The slot goes back to the window on emission, which
`WriteDoneFreesASlot` proves: a host woken by it may ask for a buffer at once, from inside the
callback if it wants, and the window will not be what refuses it. The lend keeps its own
preconditions - the call active, no cancellation latched, the handle live - so this is capacity
returned, not an allocation promised. Gating the slot on the callback's return would gate it on
something the host cannot observe, which is how a lost wake-up becomes a deadlock. Only two
things depend on the callback still being on the stack: the runtime is not quiescent while one
runs, and the terminal waits for every `AK_EVENT_WRITE_DONE` of the call to have returned.

`ak_call_cancel` is idempotent while the call is live. Once the call is reclaimed its handle is
stale, so the answer is `AK_STATUS_HANDLE_STALE` rather than nothing: the call is over, nothing
is left to cancel, and the host does not choose when reclamation happens. No lock guards the
request: the call's own task reads it and emits the callbacks, and from the moment it observes
the request, received messages not yet delivered are dropped without a callback.

#### Normalization of the head

The ABI emits exactly one `AK_EVENT_INITIAL_METADATA` per call, first, and synthesizes an empty
one when no head came: gRPC allows a Trailers-Only response, where the server sends trailers and
no headers, and a cancelled or failed call may see nothing at all. So hosts need no special case,
and the property the binding relies on - the first event of a call is its metadata - holds at
this boundary rather than being inherited from HTTP/2. A host that must tell the cases apart
reads the event's `status_code`, an `ak_head_origin`. The synthesized event is empty but not
free: it takes a delivery credit like any other, so it carries a real owner and must be
consumed, and `len == 0` with a non-NULL owner is the normal shape here.

#### No release downcall for a call

Every resource a call lends out comes back through something the runtime already observes -
`ak_event_consumed` for a payload, `ak_call_send_message` or `ak_return_call_buffer` for a
buffer, the return of its own callback - so the runtime knows when a terminal call owes nothing,
and reclaims the handle and the arena itself. A release downcall would only restate a verdict
the runtime already holds.

`ak_call_debt_of` exists because, without a release downcall, nothing reports a forgotten
`ak_return_call_buffer` synchronously, and an obligation with no way to check it is one that
rots; conformance tests and host assertions are its intended callers. Its
`AK_STATUS_HANDLE_STALE` means the handle names no call - reclaimed, for a handle the host was
given - which is to say the host owes nothing.

#### A channel's delivery window

`ak_channel_delivery_window(channel, &window, &error)` writes the delivery window the channel was
created with: `Grpc.Host.Receive.Window` as the channel's own document and the runtime's channel
defaults settle it between them, or the engine's own (four) when neither names one (decided
2026-10-07, T6.8). A host sizes what it holds per call from the window, and a runtime created from
sources states its `ChannelDefaults` where the host cannot see them, so the host reads the window
back instead of assuming it. It is fixed for the life of the channel; `AK_STATUS_HANDLE_STALE`
names a channel the library does not know, and a null `out` is `AK_STATUS_INVALID_ARG`. Additive:
`AK_ABI_VERSION` stays 1, as it did for `ak_runtime_create_from`: an addition is within the
version, so a binding that calls it against a library of ABI 1 built before it fails with a missing
export (an `EntryPointNotFoundException` in .NET), after which the .NET binding releases the channel
it had made and lets the exception through.

#### Memory usage

`ak_runtime_memory_usage` is the runtime's accounting, one number against the ceiling: the bytes
of the buffers lent, of the messages received and not yet given back, and of the compressed
copies of sent messages while they are held - a copy the ceiling has no room for is dropped, and
the message goes out uncompressed. A retry after a lend
refused with `AK_STATUS_BUDGET_BUSY` does not read it - `AK_EVENT_BUDGET_WAKE` says when a
release gave bytes back - but an operator does. A buffer occupies the ceiling from `ak_get_call_buffer` until the
runtime frees its bytes, and committing it frees nothing - it hands the same bytes from the host
to the runtime - so only a fall in the total proves capacity came back. The number may pass the
ceiling, the first threshold, by a message per call admitted to read below it, and never the
second. It answers
on a failed runtime, where a host wants the accounting most, and with `AK_STATUS_HANDLE_STALE`
after `ak_runtime_destroy`. The detailed form, under "Specified, not built" below, says why the
ceiling is held.

#### Stats

`ak_runtime_stats(runtime, &stats, &error)` writes the counters and gauges of the runtime's
channels as one `ak_stats`, and `ak_channel_stats(channel, &stats, &error)` writes those of the
channel's endpoint as another of the same record. The runtime keeps one registry for each host
and port a channel was created on: two channels there read the same numbers, what a closed one
counted stays in them, and the runtime's read is the sum of the endpoints' and of what no endpoint
owns, the waits and refusals at the memory ceiling, which are zero in a channel's record.
`ak_channel_endpoint(channel, buffer, capacity, &length, &error)` writes that host and port in
UTF-8, as much as fits, and sets `length` to the whole of it, which a host that finds it above
`capacity` asks again with a larger buffer for; a null `buffer` with a capacity of zero asks only
the length. A handle that names no channel is `AK_STATUS_HANDLE_STALE`. The record holds calls
started and ended per status, messages, retries per what failed, resends, dials, sessions closed per reason, streams the peer reset per HTTP/2 error code, bytes,
the waits and refusals at the memory ceiling, and the throttle's gauges (observability.md has what
each counts, and how the .NET binding derives from them). It is observational and outside the
formal model, synchronous and non-blocking, and counts nothing itself: what it reads the engine
has kept, and it reads it once for the call.

It is always in the header. A library built without the `metrics` feature of
`armonik-transport-ffi` answers `AK_STATUS_OK` with `AK_STATS_COUNTING` clear in `flags` and every
counter zero, and never a status that says it is not supported: a host built once serves either
build, tells them apart by the flag - or by passing the four fields of the head alone - and has
nothing to register when the flag is clear. The counters only grow, and a call that was running
when the structure was read is counted in what it has done so far.

Additive: `AK_ABI_VERSION` stays 1. A host that calls it against
a library built before it fails with a missing export, an `EntryPointNotFoundException` in .NET.

#### Payload consumption

`ak_event_consumed` does two things at once: it frees the native memory, and it arms the next
event of the call, the demand signal. At most `Grpc.Host.Receive.Window` payloads of a call are
outstanding at once, a channel option whose default is 4: while the host owes that many, the
runtime withholds the next data callback, and only a terminal may still go out with every credit -
every place of the window - spent.

The payload's owner identifies the allocation to free, and the header has the host give a
call's payloads back in delivery order. Nothing in this library refuses another order: it
frees whichever owner it is given. The order is what the formal model's accounting rests on.
Level 1 counts releases rather than tracking which payload each one names, and that count
says which payloads the host still owes only when releases come in order -
[formal-model.md](formal-model.md) calls it the conformance assumption the count rests on.
The .NET binding meets it by representation, its ring releasing at its tail, which level 2
proves.

The terminal does not invalidate payloads already handed over: `ak_event_consumed` stays legal
after it, and is required before the call can be reclaimed. After a runtime failure no promise
that reads a runtime state survives, but this one does: `ak_event_consumed` stays legal and
`BufferEventuallyFreed` still holds, because a failed runtime disables neither returning memory
nor freeing it.

### Specified, not built

What this document specifies and nothing builds yet keeps its declarations here, until the code
that builds it carries them in the header.

The detailed form of the memory usage, an observability tool rather than one a retry needs. Its
`ak_runtime_handle` is the header's `ak_handle`:

```c
// The detailed form is for observability, not for progress: it says why the ceiling
// is held, so an operator can tell a stuck host from a slow network. The five
// categories are the buffer lifecycle and the received messages' own, and each says
// who has to move next:
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
//   bytes_host_received  messages delivered and not yet consumed. The host's own
//                        code must give them back with ak_event_consumed.
//   bytes_runtime_received
//                        messages decoded and not yet delivered, at most one per
//                        call, each waiting for a delivery credit. The host frees
//                        them by consuming what it already holds.
//
// The compressed copies of sent messages that the engine holds are counted in
// bytes_used and in none of these categories: the identity below holds for a
// runtime that holds none, and the detailed form is not built.
//
// The first two fields of ak_memory_usage_detailed are the base struct's, in the
// same order, so a host upgrades by changing the call and the type and re-reading
// nothing.
//
// Normative: the snapshot is coherent - all seven numbers are read from one instant
// of the runtime's accounting - and
//     bytes_host_lent + bytes_send_in_flight + bytes_runtime_held
//         + bytes_host_received + bytes_runtime_received == bytes_used
//     bytes_used <= memory_hard_ceiling, as ak_runtime_config set it
// hold exactly on every returned snapshot, not merely eventually. A host may
// therefore compare fields across categories without a second call. bytes_used may
// pass ceiling, the first threshold: calls admitted to read below it may cross it
// together, by a message each.
//
// Normative here means an ABI obligation, checked by the ABI tests. The identities
// are proved at level 1 (MemoryAccountingExact, CategoriesPartitionTotal,
// ReceivedCategoriesPartitionTotal, MemoryWithinHardCeiling); what stays a test
// obligation is the snapshot itself - that one read returns one coherent instant.
// See "What is actually verified" in formal-model.md.
typedef struct {
    uint64_t bytes_used;
    uint64_t ceiling;
    uint64_t bytes_host_lent;
    uint64_t bytes_send_in_flight;
    uint64_t bytes_runtime_held;
    uint64_t bytes_host_received;
    uint64_t bytes_runtime_received;
} ak_memory_usage_detailed;

ak_status ak_runtime_memory_usage_detailed(ak_runtime_handle runtime,
                                           ak_memory_usage_detailed *out,
                                           ak_error *out_error);
```

### Unary call sequence

Every fallible downcall below also takes `out_error`, left out for width.

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
ak_call_send_message(handle, buf, n) -> the n bytes written; buf passes back to Rust
                                   ... network: Rust sends over HTTP/2 ...
                          callback(runtime_ctx, gcHandle, &evt_w, 1) <-
                            evt_w.kind = WRITE_DONE     [slot free]
ak_call_end_send(handle)           -> signal end_send
                                   ... network ...
                          callback(runtime_ctx, gcHandle, evts, 3) <-
                            evts[0].kind = INITIAL_METADATA  [always first]
                            evts[1].kind = MESSAGE
                            evts[2].kind = STATUS  [terminal, end of stream]
                            evts[i].payload = ak_bytes{ptr, len, owner}
                            evts[2].status_code = 0 (OK)
                            [what is read a round of the runtime
                             apart comes in one callback; a head
                             sent ahead of its message comes alone]
                            [this callback frees gcHandle after its
                             last access - it is the call's last]
// host can deserialize directly from evts[1].payload.ptr (zero-copy recv)
ak_events_consumed(payloads, 3)    // free all three (no next, the terminal is in)
                            [the last debt cleared, this downcall
                             reclaims the handle and the arena; the host
                             asks nothing more, and its handle is now
                             stale]
```

FFI note:
- **Send**: the host serializes into a buffer lent by `ak_get_call_buffer` and gives it back
  exactly once, by `ak_call_send_message` or `ak_return_call_buffer`, or exchanged for another
  by `ak_resize_call_buffer`, which is then the one to give back. One unfilled buffer at a
  time, and at most `Grpc.Host.Send.Window` out of one arena (default 1), counting those committed
  and awaiting their WRITE_DONE; WRITE_DONE acquits in send order,
  always arrives, exactly once per accepted send, and always before the terminal event, even
  on error or cancellation - but on a call that declared one request, whose commit settles its
  send with no WRITE_DONE. There is nothing to pin: the memory is Rust's from the start.
  See Zero-copy in architecture.md.
- **Receive (demand via consumed)**: the `ak_bytes` payload is owned. The host consumes
  (deserializes directly from the native pointer) then calls `ak_event_consumed`, or
  `ak_events_consumed` for several in delivery order. This frees the memory AND arms
  reception of the next event. One callback carries the data events of a call that were
  ready together or that its delivery waited to gather, up to `Grpc.Host.Receive.CoalescingBytes`, and
  every other event alone. At most `Grpc.Host.Receive.Window`
  non-consumed payloads per call (default 4) — this is the backpressure mechanism.
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
     |  (Hyper and Tonic are done; the teardown thread is not)
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
`ak_bytes`, a versioned head on every options struct the host fills, `ak_runtime_config` and
`ak_call_start_options`, the metadata blob as a length-prefixed key/value sequence, and the
`AK_EVENT_STATUS` payload as a length-prefixed reason followed by the trailing metadata - the
code itself is `ak_event.status_code`.

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
