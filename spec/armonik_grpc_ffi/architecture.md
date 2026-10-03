# Architecture — the Rust engine and the .NET binding

How [contract.md](contract.md) and [abi.md](abi.md) are met: the internals of each layer,
from the engine to the ArmoniK client. What binds is in those two documents; this is how
this repository meets it, and why.

## Layer 1 — `armonik-transport`, module `http2`

### Relationship with existing code

The `armonik-transport` crate already has a functional `TlsConfig` and `Identity` (serde flat
options, eager loading of crypto material on config read). The design here follows the same
approach: configure what you want to obtain (paths, options), and the material is
loaded/validated immediately upon connector construction. The Rust types contract.md states are
an evolution of the existing code, not a from-scratch replacement.

## Layer 2 — `armonik-transport`, module `grpc`

### Which runtime drives the engine

The channel is handed a `tokio::runtime::Handle` and spawns on it. Naming the runtime rather
than taking the ambient one is what the C ABI needs: the runtime that drives the engine is one
the host never enters, and `tokio::spawn` would look for a context the calling thread does not
have.

There is no executor trait. The engine is written against `tokio` throughout - `tokio::sync` for
every channel and semaphore, `tokio::time` for the deadlines, `tokio::net` for the connector -
so an abstraction over the spawner alone would name a portability the rest of the crate does not
offer. A task handle for cancellation is not needed either: a task is stopped by its call being
cancelled or its channel closed, both of which the task itself watches.

The one adapter that remains is `Spawner`, which puts the handle in the shape
`hyper::rt::Executor` asks for.

### Consumption by the Rust ArmoniK client

The Rust ArmoniK client (`armonik::Client<T>`) uses the `grpc` module via an adapter
compatible with the generated Tonic stubs. `tonic::transport::Channel` is a concrete type
and cannot be implemented; the trait a generated stub actually requires is `GrpcService`,
which any `tower::Service<http::Request<Body>>` satisfies. That is the boundary:

```rust
/// Adapter that allows Tonic stubs to consume a GrpcChannel.
pub struct TonicAdapter {
    channel: GrpcChannel,
}

impl tower::Service<http::Request<BoxBody>> for TonicAdapter {
    type Response = http::Response<BoxBody>;
    type Error = tonic::Status;
    type Future = ...;
}
```

A `tower::Service` speaks HTTP bodies; `GrpcChannel` speaks messages. An adapter between the
two is not thin: it has to take the request body apart into messages and put the response
messages back together as a body, including trailers, compression flags and the length-prefixed
framing - a second implementation of the gRPC wire format layered on the first. The middle is
where the framing gets implemented twice, and there are two coherent answers either side of it:

- the Rust engine exposes an HTTP/2 service that Tonic consumes directly, and the
  message-level API is what the FFI is built on;
- or the ArmoniK Rust clients use message-level stubs generated against `GrpcChannel`, and
  Tonic is not in the picture at all.

**Decided (2026-09-28): the first**, and the engine already has its shape - tonic's client over an
HTTP/2 service of the channel's own. What it means for the Rust client work already under way
elsewhere is analysed when T7.1 starts. The .NET binding is unaffected either way: it consumes the
message-level API through the FFI, and both paths share the same engine - retry, deadline, pool,
flow control.

## Layer 3 — `armonik-transport-ffi`

### Internal architecture

The level-1 model is an actor model, and the implementation is built as one. Every FFI
variable of `FfiGrpc` belongs to exactly one owner, and that owner is the only writer:

```text
AkRuntime                       owns: runtime state, task group, callback + runtime_ctx
  |                                   shutdown_event_emitted, shutdown_callback_running
  +-- AkChannel (per channel)    owns: GrpcChannel, the channel state
  |     |
  |     +-- CallActor (per call) owns: everything per-call the model names
  |           |                        write_dones_emitted, *_callback_running,
  |           |                        payloads_consumed_by_host, the observed
  |           |                        cancel/release requests
  |           +-- send window     MaxSendsInFlight entries, FIFO
  |           +-- delivery window DeliveryCredits outstanding payloads
  |
  +-- payload allocator          the only structure touched by ak_event_consumed
```

**One task per call owns the delivery side, and it is the only writer of it.** The call
actor is a Tokio task owning the call's delivery state. Every *data* callback for that
call - metadata, message, terminal - is invoked from it, and every downcall reaches it as
a command. This is what makes the level-1 guards implementable without a single
cross-thread lock: `DeliverMessage` tests `~IsCancelRequested(cId)`, and the task that
reads the flag is the task that emits the callback, so nothing can slip in between the
test and the emission. A downcall never emits a callback and never blocks on one.

**WRITE_DONE is a second domain, and deliberately so.** The ABI states it may arrive in
parallel with a data callback for the same call, precisely so that acquitting a send is
never held behind a slow message handler. So there are two callback producers per call,
not one, and the model says the same thing: `EmitWriteDone` and the deliveries are
separate actions with separate running flags, each serialized within itself and neither
with the other. A host must therefore assume a data callback and a WRITE_DONE can be on
two threads at once for one call - which is exactly why WRITE_DONE stays out of the
delivery ring on the managed side, leaving that ring single-producer.

**Downcalls are commands, not state changes.** `ak_call_cancel` sets an atomic flag and
wakes the actor; it returns immediately, and `RequestCallCancellation` linearizes when the
actor *observes* the flag rather than when the downcall returns. The window in between is
level-1 stutter, which is why the model never had to promise anything about it: committed
callbacks may still arrive after `ak_call_cancel` returns, exactly as the ABI documents.

**Reclaiming a call is the runtime's own step, not a downcall.** `ReleaseCallHandle`
linearizes when the actor observes the last debt cleared on a terminal call: no payload
owed, no buffer out, its own callbacks returned. Every one of those is an event the
runtime already sees, so nothing had to be asked of the host to evaluate them, and the
guarantee that follows - the arena goes back with nothing of it in the host's hands - is
unconditional rather than contingent on the host calling something.

**That is the difference between this and `ak_runtime_destroy`, which stays a downcall.**
Destroy answers a question only the host can ask - may I unload the library - so the host
needs a verdict it can act on, and a refcount is no substitute. Reclaiming a call answers
a question the runtime resolves for itself, and the verdict is of no use to the host.
Giving both the same downcall shape would force an unobservable condition (has the
callback finished unwinding) to be argued *out* of a precondition; the right shape is no
precondition at all.

**The send window** bounds the memory the call's arena lends out: at most
`MaxSendsInFlight` buffers at a time, counting both those the host is still filling and
those already committed and awaiting their WRITE_DONE. The slot is charged when the
buffer is lent rather than when the message is committed, because the allocation is what
costs memory. Acceptance must be synchronous - the ABI refuses with `AK_STATUS_SLOT_BUSY` rather
than blocking - so occupancy is an atomic counter the caller thread bumps with a bounded
compare-and-swap: the model's `HasFreeSendSlot` guard *is* that CAS, and
`LendSendBuffer` linearizes at its success. Committing moves a buffer from one side of
that count to the other without changing it, which is why `ak_call_send_message` needs
no bound of its own.

WRITE_DONE is emitted in send order, so the counter and the queue head are all the
bookkeeping needed; this is exactly the level-1 argument that one monotone counter
identifies which sends are acquitted.

**The buffers carry identities in the model.** Level 1 names every allocation, drawing
from a constant set `BufferIds` per call, because the ABI says a buffer is given back
exactly once and returns are unordered - a count cannot say which buffer came back, which
is the gap the names close. The set is finite so that the per-buffer arguments are finite
inductions. Its size is a modelling bound: identities are never reused, so a level-1
call accepts at most `Cardinality(BufferIds)` sends and the freshness a lend needs
depends on it - a configured depth is reachable in the model only if the space is at
least that large. It is not `MaxSendsInFlight`,
which bounds how many allocations are outstanding at once and is what makes a returned
buffer's send debt bounded by a constant.

**The slot goes back at emission, not at return**, and the distinction is load-bearing.
Two counts live here and they are distinct: `SendWindowOccupancy`, which
`MaxSendsInFlight` bounds and which shrinks when WRITE_DONE is emitted, and the
acquittal callback still on the stack, which only quiescence and the terminal care
about. Freeing at return would mean a host woken by WRITE_DONE could ask for a buffer,
be refused with `AK_STATUS_SLOT_BUSY`, and have already spent the wakeup that was going to free
it - a deadlock whose window is microseconds, which is to say one that passes tests and
fails under load.

The rule behind it is the one that also removed `ak_call_release` from the ABI: **the
return of a callback is not observable by the host, so nothing the host must wait for may
be gated on it.** `WriteDoneFreesASlot` is that rule made checkable - a WRITE_DONE really hands
a slot back - and it belongs to a family worth stating deliberately, because nothing
forces it otherwise: what the ABI permits, it does not then withdraw. The receive side
has the same property and got it by accident, because the `DeliverMessage` fairness
lift needed it; the send side is host-driven, no lift needed anything, and the property
went unstated until it was violated.

**Rust never reclaims a lent buffer.** Not on cancellation, not on channel close, not on
shutdown: the buffer comes back only through `ak_call_send_message` or
`ak_return_call_buffer`, both host calls. That is what removes the race between a thread
serializing into the buffer and a thread cancelling the call - there is no moment at
which two parties may touch the allocation, so no lock is needed and no cancellation
token is load-bearing for memory safety. A `try/catch` around the write would not be an
alternative: on .NET Core and later an access violation is a corrupted-state exception
the runtime does not let you catch.

The counterpart is a fairness hypothesis, `WF(HostReturnsBuffer(c, b))`, and it has to be
per buffer rather than per call: returns are unordered, so a per-call conjunct would let a
host cycle buffers indefinitely while starving one. That is the one asymmetry with the
receive side, where `WF(HostConsumesEvent(c))` per call suffices *because* release is FIFO
- the counter there identifies which payload is owed, and a buffer count identifies
nothing. The host is therefore asked for two things, and they are symmetric: give back
what you consumed, give back what you borrowed.

**The delivery window** is the exact mirror. At most `DeliveryCredits` payloads may be
outstanding, and the host releases them **in delivery order**, so one counter - payloads
released - is all the bookkeeping needed, against `events_delivered` the way the send
counter runs against `submitted`. `ak_event_consumed` still takes the payload rather than
the call, because the `owner` field is what identifies the allocation to free; the order
is a contract, not a lookup. It runs on the host's thread: it frees the allocation and
posts a credit back to the actor, which is the only place a credit is ever created.
Terminals use the relaxed guard (`HasFreeDeliverySlotForTerminal`), so a terminal is never
blocked behind unread messages.

FIFO release is what lets level 1 count payloads instead of tracking their identities,
and level 2 carries that order by representation rather than as an obligation: its release
counter can only advance by one, and `ConsumerHandoffPreservesTail` is the theorem for the
hand-off. In exchange the model asks strictly less of the host: one
fairness conjunct per call rather than one per payload, since consuming past a payload
without consuming it is no longer a behavior the ABI admits.

**Quiescence.** The runtime keeps a task group; `ak_runtime_begin_shutdown` closes the
start gate and asks every channel to close, which cancels its calls. Each call actor
drains on its own - the proved `CancellationCompletes` says it needs nothing from the
host - and unregisters. When the group is empty and no callback is on the host stack, the
runtime emits `AK_EVENT_SHUTDOWN_COMPLETE`; only after that callback returns does the
state become `AK_RUNTIME_GRPC_STOPPED`. Payloads the host still owes do not hold that back:
`ak_event_consumed` stays legal afterwards. The chain therefore depends on no *ownership
return* - neither `HostConsumesEvent` nor `HostReturnsBuffer` appears in
`ShutdownEventEmitted` or in the `RuntimeRelease` lift - which is what discharges the
level-0 directive on `ShutdownFairness`. It does depend on the callbacks already
dispatched returning: the lift consumes `ShutdownCallbackReturns` and
`DeliveryCallbackReturns`, and `ShutdownEmitFairnessRequirement` consumes
`WriteDoneReturns` as well. For the .NET binding those returns are a property of a
trampoline that is total and never runs user code inline; for a generic FFI host they are
obligations the ABI imposes. `AK_RUNTIME_GRPC_STOPPED` is the functional shutdown, and it
is one of the two refinements of level-0 `RELEASED`.

Stopped is not destructible. The first says the runtime has nothing left to run; the
second says the host has nothing left to give back, and one state cannot carry both - so
there are two, and `AK_RUNTIME_QUIESCENT` is the second. It is checkable rather than
trusted: the runtime counts the payloads and buffers it handed out, and publishes
QUIESCENT only when that count reaches zero and its own arena work is done. The count is
internal and has no ABI of its own; `ak_runtime_status` is the single place the host reads
the answer.

That count is why the order matters. A host that waits for QUIESCENT before releasing
waits for a condition it is itself the only obstacle to, which is a deadlock - so the
host-debt field on `AK_EVENT_SHUTDOWN_COMPLETE` tells it, at that moment, whether the ball
is in its court. `AK_EVENT_RESOURCES_RELEASED` then says its part is done. Neither event
is the guarantee: a callback runs on one of the runtime's own threads, so it cannot report
that the threads are gone. `ak_runtime_status` returning QUIESCENT is the guarantee, and from
there unloading the library, destroying the runtime and starting a new one are all safe.

**A thread per channel.** Each channel runs on a thread of its own, a current-thread tokio
runtime that carries the channel's connection, the driver of each of its calls and the
call's own tasks, so that a call never moves between threads (`decisions.md` gives what that
was measured against). The calls of one channel share that thread for all their work -
framing, encoding and decoding, the host's callbacks - and channels run in parallel. The
runtime keeps one worker of its own, which delivers AK_EVENT_SHUTDOWN_COMPLETE. A channel's
thread stops with the channel, once the host has released it and its last call has been
reclaimed, or with the teardown; either way it first gives the close of the connection up to a
second to finish, so that the peer sees the connection closed rather than dropped. The teardown
waits for every channel's thread before it finishes - so QUIESCENT still means that no thread
of the runtime is left, and may come up to that second later. What a callback costs is its
channel's to pay: one that blocks stops that channel's connection and every call on it.

Both STOPPED and QUIESCENT refine one level-0 state. Freeing a handle is not a gRPC
concept, so level 0 has nothing to say about the difference, and level 1 carries it
entirely in its own variables - the only shape the refinement rule allows.

Handles are not part of that count. Destroy invalidates every handle of the runtime at
once, and a token is never reissued, so a later use is a refusal rather than a fault. The line
is between what the runtime owns and what the host might still be touching: a handle names
runtime state, a payload or a lent buffer is memory under the host's hands. Only the second
kind can hold destruction back - the alternative, refusing to destroy until every call
handle is released, turns a host bug that today leaks a slot into one that hangs at
teardown, and buys no safety in exchange.

**Failure.** A runtime fails when its shutdown can no longer finish: the task that drives
the shutdown panicked, or the thread that drops the tokio runtime from outside it could not be
started. Both happen during a shutdown. Elsewhere a panic is contained where it happens - a
downcall answers `AK_STATUS_INTERNAL`, a call's own task ends that call with a status - and
the runtime goes on. Two defensive readings report it too: `ak_runtime_status` itself
faulting, and a stored state it cannot read. The state is then `AK_RUNTIME_FAILED_UNQUIESCED`:
`ak_runtime_destroy` is refused and unloading is forbidden, because nothing can promise the
outstanding memory is idle.

A shutdown that only fails to end - a callback the host never returns from, a buffer never
given back - is not a failure. The runtime stays `GRPC_STOPPED`, which refuses the destroy just
the same, and the host's patience is the host's to bound. The model lets a runtime fail from
RUNNING as well, which is wider than the code and the safe direction to be wrong in: every
guarantee is stated outside the failure.

**Failure suspends the nominal contract, and the document must not promise past it.**
The models make their safety and liveness guarantees conditional on no runtime having
failed, so after a failure there is no promise that callbacks stop, that a terminal
arrives, that cleanup completes, or that any downcall behaves as documented. Two things
do survive as *invariants*, and they are deliberate rather than incidental: the failed
state is absorbing - the public `FailedRuntimeAbsorbing` carries it as a theorem, stated
outside `NotFailed` on purpose: it is the one promise that holds exactly when the other
guarantees' escape hatch has fired - `RuntimeFail` requires a running or stopping runtime, so nothing
returns to nominal - and `FfiCallInv` together with `BufferStateInv` is proved outside the
failure envelope, so the send and delivery counts and the buffer identities hold whatever
happens. The fairness lifts need those disciplines on the whole behaviour, failure
included, so moving them under the envelope would break the refinement.

Six *liveness* guarantees are also stated without a failure escape, and that is a claim
worth reading twice: the four callback returns, `BufferEventuallyFreed` and
`CallEventuallyReclaimed` promise progress even after a failure. They can, because every
action they rest on is untouched by one - returning a buffer, freeing its bytes and
returning from a callback are all steps whose guards a failed runtime does not disable. It
is the guarantees that read a *runtime state* that carry the escape, because level-0
safety is asserted only outside the failed state: `ShutdownEventEmitted`,
`RuntimeEventuallyQuiescent` and `ResourcesReleasedEventually` each end in
`\/ ~L0!NotFailed`. So the honest summary is not "no promise past a failure" but "no
promise that reads a runtime state past a failure".
It is also the one state in which reclamation may never happen: no terminal will ever
arrive, so the debt of an active call is never settled and its arena is stranded. That is
a leak on a path that has already given up, and preferable to reclaiming memory the host
may still be reading.

### Zero-copy (integrated from V1)

**Send (host → Rust)**:
- `ak_get_call_buffer(handle, len, &buf)` — Rust lends writable native memory
- the host serializes into it and commits with `ak_call_send_message(handle, buf)`, or gives
  it back unused with `ak_return_call_buffer(buf)`
- at most `MaxSendsInFlight` buffers out of one arena (natural backpressure, on top of HTTP/2
  flow control)
- nothing to pin on the .NET side: no `GCHandle`, no pinned object heap, no fragmentation of
  the collected generations

The exact length is known before the first byte, so a plain `len` suffices and no growable
writer is needed: the generated marshaller calls `context.SetPayloadLength(message.CalculateSize())`
and only then writes.

`AK_EVENT_WRITE_DONE` now says one thing, the slot is free - and free at emission, so
the writer it wakes can act on it straight away. The bytes leave the arena once, when tonic's
encoder copies the message into the request body, and the allocation is released there. **The
replay cache is the engine's copy**, not the arena original: a retry resends what the engine
holds, which is a copy it needs whatever the send path does.

So the slot budget and the replay buffer (`RetryConfig`'s replay bytes) charge two different
buffers - the arena allocation while it is lent or queued, the engine's copy while a retry may
still send it - and they do not constrain each other: a replay takes no slot in the send window,
which bounds what the host serializes at once and nothing a replay does.

**Receive (Rust → host)**:
- `ak_event.payload` is an owned `ak_bytes` (reference-counted Rust buffer)
- The host can deserialize directly from `payload.ptr` via `Span<byte>` or unsafe
- The host calls `ak_event_consumed` when done — Rust deallocs + arms the next one
- No copy if the host consumes the native pointer directly

**Memory fragmentation**:
- Send side: every send buffer comes out of the call's arena, bounded by
  `MaxSendsInFlight` buffers at a time, and the arena is dropped in one piece when the call
  is released - which the release precondition guarantees is safe. Retained replay bytes are
  not these allocations but the engine's copies, bounded separately, per call and per channel,
  by the retry unit's replay bytes.
  Arenas are a natural fit for a pool held by the channel, so the same memory serves every call
  the channel carries and the steady-state fast path allocates nothing the binding controls - no
  payload, no event object - which is a budget to measure, not an absolute: task completions,
  scheduling, exception paths and arbitrary marshallers allocate.
- Receive side: Rust buffers are allocated by Hyper (similar size classes,
  well-managed by jemalloc/system allocator). If fragmentation is measured in production,
  a pool of pre-allocated buffers can be added without ABI change.
- Neither pool is visible across the ABI, so either can be added or removed later without
  touching a binding.

**A global ceiling above the per-call budgets.** `MaxSendsInFlight` bounds one call's
outstanding buffers; nothing bounded their product across the calls a process carries. The
runtime therefore holds a byte budget shared by every channel, handed out by
`ak_get_call_buffer`. Replay copies are not charged to it: a channel's total of replay bytes bounds
them, past which a call is committed rather than any send refused.

**Reaching the ceiling and failing to allocate are two different events, and only the
second is a fault.** The ceiling is a configured accounting limit. Reaching it means some
live allocation - possibly from this call - is holding bytes right now, and the runtime
knows exactly what recredits that capacity: `FreeReturnedBuffer`. Waiting demonstrably
helps. A real allocator failure is the other case, and there waiting has no evidence behind
it, so it is no backpressure: it is `AK_STATUS_INTERNAL`, and a host does not retry the same
length. It does not fail the runtime. What failed is this request, refused with everything it
took given back, and what the runtime holds for its other calls is allocated already.

So `ak_get_call_buffer` has four modeled lend outcomes - `OK`, `SLOT_BUSY`,
`BUDGET_BUSY`, `MESSAGE_TOO_LARGE`; the remaining ABI results (`HANDLE_STALE`,
`INVALID_STATE`, `INVALID_ARG`, and `INTERNAL` for a fault the ABI cannot attribute)
are the ABI matrix's rows, outside the backpressure sub-machine level 1 formalizes.
It lends, with `MESSAGE_TOO_LARGE` refused permanently when `len` exceeds the ceiling
itself; or it refuses with
`AK_STATUS_SLOT_BUSY` because this call's window is full, whose wake-up is WRITE_DONE; or it
refuses with `AK_STATUS_BUDGET_BUSY` because the runtime-wide ceiling is reached, which is
not necessarily this call's doing - with a window deeper than one, its own sends hold
budget too - so a WRITE_DONE of this call is a wake-up,
never an exhaustive one: the budget is runtime-wide and anyone's free recredits it.
A genuine allocator failure is `AK_STATUS_INTERNAL`, one of those rows: a refused lend that
changes nothing level 1 carries, and not `RuntimeFail`.

The order of those checks is what keeps `AK_STATUS_INTERNAL` rare. The ceiling is tested
*before* anything is allocated, so a runtime at its budget refuses with `AK_STATUS_BUDGET_BUSY`
and never reaches an allocation that could fail. Past that check, the arena - the one
allocation the host sizes - is reserved with the fallible form, `try_reserve_exact`, so its
failure returns rather than aborting, and that return is what `AK_STATUS_INTERNAL` reports. The infallible `Vec` and `Bytes`
APIs are what abort; the emission path does not use them.

**The budget covers both directions, with two thresholds.** One count of bytes holds what
`ak_get_call_buffer` lends and what the engine receives: a message is charged its length from
the moment it is decoded until the host gives it back with `ak_event_consumed`, or until its call
ends without having delivered it. The slack a message may keep alive in tonic's decode buffer is
taken as negligible. Nothing on the receive side allocates against a refusal, which is what made
a fallible receive path look necessary: a count of messages already decoded needs none. The
decision is taken before the read instead - a call is admitted to read its next message only
below the first threshold, `memory_ceiling`, and held back above it, HTTP/2 flow control then
stopping the peer as it does when a call is out of delivery credits. The second threshold,
`memory_hard_ceiling`, is where the engine stops: calls admitted together below the first may
pass it by a message each, and a decoded message that would take the count past the second ends
its call with `RESOURCE_EXHAUSTED` and is freed at once. Level 1 models the two steps of the read
as two actions, `AdmitRead` and `NetworkReceive`, because an atomic read would hide the very
overshoot the second threshold bounds.

**A send refused for room is served before new reads.** While one waits, the threshold where
reads stop is lowered by the largest length waiting, and goes back once that send is served or
its call ends. Without it, received bytes would keep the count from falling under steady
traffic, and a refused send could wait forever - level 1's `RefusedSendEventuallyHasRoom` is
proved from that hold. The length and not the charge: a refused charge may exceed the first
threshold, while a length past it is `MESSAGE_TOO_LARGE`, so the lowered threshold is never below
zero.

**Sends in flight hold reads back too.** The count holds a committed message until its WRITE_DONE,
and HTTP/2 flow control can hold that send until the peer reads, which on a bidirectional call may
wait for this side to read. With the count at the first threshold, neither moves until another
call gives room back or a deadline ends the call; a BUDGET_WAKE owed to that call waits as well,
since its writer raises it once the send returns. The ceiling is therefore sized above what the
calls a process runs at once keep in flight. The .NET binding keeps at most one, and holds none
when it lends.

**The engine decides when a call reads; the runtime supplies the rule.** The admission is a
`ReadGate` the call starts with, which `armonik-transport` waits on before it reads each message
off the stream. The runtime's gate opens once the call has delivered the message it read last and
the ceiling admits a read - level 1's two conditions for `AdmitRead`. Waiting there, a call is
still ended by what ends any of its waits - its deadline, its cancellation, its channel closing -
as grpc-java and grpc-dotnet end a call whose reader is not reading. A status the peer sends is
read where its messages are, behind the gate, as both deliver a status only after the messages
before it: a held-back call learns how its peer ended it once there is room. Level 1 receives a
status ungated; a call that waits there is the back-pressure the budget exists to apply, and the
host ends it by giving back what it holds or by cancelling.

**What a buffer charges against the budget** is the capacity of the allocation that backs it,
not merely the size the host asked for: `charge(b)` is what backs `b`, known before the lend,
`len` is the request it must cover, and `bytes_used` is the sum of `charge(b)` over every buffer
lent and not yet freed, plus the length of every message received and not yet given back. Each lend gets a fresh allocation of exactly `len`, so today the charge
is the request; a pooled arena would charge the capacity of the buffer it hands out.
The two refusals are then

```
AK_STATUS_MESSAGE_TOO_LARGE  <=>  len > ceiling
AK_STATUS_BUDGET_BUSY        <=>  len <= ceiling  and  bytes_used + charge > ceiling
```

`ceiling` is the one in force, not the one the host typed: the ABI has bounds of its own -
the gRPC length prefix is four bytes, and no allocation exceeds half an address space -
and a budget above them is one no single lend could ever draw on. `Ledger::limit` caps
what was configured by what this library can lend, and `ak_runtime_memory_usage` reports
that. One number, so `Ceiling` in the model is that number and the equivalence above is
the whole story.

Charging the request, whatever backs it, is the obvious alternative, and it fails at the one
thing the ceiling exists for. A budget that counts what was asked for bounds an accounting
fiction; the memory that has to fit is what backs the buffer, and per-arena slack would sit
outside the ceiling, wrong by however much it comes to - silently, and in the direction that
matters. The allocator's own size-class rounding stays outside regardless: Rust's stable
allocation interface does not report it, and it is small against a message.

What the ceiling bounds is what the engine lends and what it has received and not had back.
The copy tonic's encoder makes of each message is outside it, and so is what hyper buffers below
the decoder within the flow-control window; neither is bounded runtime-wide.

What that costs is the predictability of one refusal, and only one. `MESSAGE_TOO_LARGE` stays
a predicate the host can evaluate *before* it calls, because it reads `len` and the ceiling
and nothing else - which is also why its permanence is **derived** in the model rather than
asserted: `IsLendable(len)` mentions no charge, so no sequence of frees can turn the answer
around. A host holding a 4 MiB message against a 4 MiB ceiling still knows which answer it
will get. `BUDGET_BUSY` need not be predictable from the host's own numbers - it happens to be
while the charge is the request - because it is transient, it has a wake-up, and retrying on it
is the correct response.

The residue is small but it is not zero: **the ceiling bounds
the bytes the allocator reported, not the process's resident memory.** Allocator metadata, the
arena's own structures and fragmentation between arenas sit outside it. No bound relating the
two is asserted here: it is not known that the relation even has the shape of a factor plus a
constant, and writing one down before there is data to fit it would be a number with no
standing. Configuring the ceiling as though it were an RSS limit is the concrete mistake this
paragraph exists to prevent, and stating that much does not require the bound.

Two of the refusals are transient and each needs its own wake-up. `AK_STATUS_SLOT_BUSY`
has WRITE_DONE. `AK_STATUS_BUDGET_BUSY` has `AK_EVENT_BUDGET_WAKE`: a call refused for the
ceiling has no send in flight, so nothing of that call frees room, and the event comes from the
releases of others. The third refusal, `AK_STATUS_MESSAGE_TOO_LARGE`, needs no wake-up because
waiting cannot help, and `AK_STATUS_INVALID_STATE` needs none either: it reports a guard, not a
shortage.

The wake-up fires where the count falls - a send buffer's release, at its WRITE_DONE or when it
is given back unsent, and a received message's at `ak_event_consumed` - and never at a commit,
which moves bytes from the host to the runtime without freeing any: a signal on that edge would
be a wake-up that never comes. It wakes every call refused since the last release, carries no
payload and takes no delivery credit, as WRITE_DONE takes none. Waking every one rather than one
is what keeps the wake-up from being lost: a single call woken that does not try again would
leave the others asleep with room available. The engine raises it from the task that also
delivers the call's terminal, which waits for it, so the terminal stays the call's last event.

A host woken is obliged to try the send again or to cancel the call, as it is obliged to give a
payload back: one that gives up a send and keeps its call holds reception lowered for the whole
runtime. What the wake-up does *not* give is freedom from starvation: another call can take the
room between the release and the retry. Level 2 states the honest contract there: the wait is
cancellable and nothing more - cancellation and dispose end it, the successful lend ends it, and
no acquisition is promised, because that would need an arbitration the ABI does not have.

A fatal ceiling - the rejected alternative - would be wrong, not merely pessimistic:
`AK_RUNTIME_FAILED_UNQUIESCED` is absorbing and `ak_runtime_destroy` is refused from it
forever, so a normal burst would leave the runtime permanently undestroyable - a
mechanism introduced to bound memory turning the runtime itself unreclaimable.

An implementation that refuses more often than the specification permits produces a subset
of the modelled behaviours: no proved liveness rests on the lend being taken. The
alternative shapes - reserving the budget at call admission and refusing `ak_call_start`, or a
runtime permit acquired before the lend and released on the real recredit - remain open and
are the level-2 material for turning a non-blocking refusal into a fair asynchronous wait.
See T6.1.

---

## Layer 4 — `ArmoniK.Api.Client.RustGrpcChannel`

### Internal architecture

The disposable object is the channel; the invoker is a view over it.

```csharp
await using var runtime = NativeRuntime.Create();
await using var channel = runtime.Channel(endpoint, options);
CallInvoker invoker = channel.CreateCallInvoker();
```

```text
+--------------------------------------------+
|  NativeRuntime                             |
|    +-- Trampoline (static delegate)        |
|    +-- NativeChannel : ChannelBase         |
|          +-- the channel's ulong handle    |
|          +-- NativeCall<T> (per call)      |
|                +-- delivery ring + signal  |
+--------------------------------------------+
        |
        +-- CreateCallInvoker() -> NativeCallInvoker (a view, no state)

No queue and no thread between the two: the trampoline publishes into the call's own
ring and the consumer reads it directly.  The runtime is the application's own object;
a channel is made from it and kept by it.
```

**Lifecycle: the runtime is an object, and it is the caller's.** `NativeRuntime.Create`
starts the engine and `DisposeAsync` stops it, so a host declares how long it wants one
rather than having that derived from something else. A channel is made by
`runtime.Channel(endpoint)` and the runtime keeps it; a `CallInvoker` is a stateless view
that `CreateCallInvoker()` hands out and that owns nothing.

`DisposeAsync` on a channel settles its own calls - and no one else's, ownership being
`call_channel`, the level-0 relation - releases its `ak_channel`, and tells the runtime to
forget it. It stops nothing else. `DisposeAsync` on the runtime disposes every channel it
still holds and then retires the engine: begin the shutdown, wait for quiescence, destroy.
Both orders are ordinary - a caller that disposed its channels first finds the set empty,
one that disposed neither has them disposed for it - because a channel disposed twice is a
no-op, and because an order a caller has to remember is not a guarantee.

**Why this is not a refcount.** The engine admits one runtime per process (requirement
14.9) and gives that claim back only on a destroy that succeeded, so an implicit lifetime
- the first channel materializing a runtime, the last releasing it - forces a decision on
what a creation does while a destruction is in flight. There is no good answer: refusing
breaks "dispose everything and start again", and waiting blocks a thread on a teardown
that needs the thread pool to advance, which is a starvation deadlock under load. Owning
the runtime dissolves the question rather than answering it. A second `Create` while one
lives is refused at once, by the engine and on its own authority, and nothing waits.

Membership is a set behind a lock rather than a concurrent one: what has to hold is that a
channel is never added after the disposal has swept, and under the lock either the creation
wins and the sweep finds it or the creation reads the disposal and refuses. A concurrent
set needs a second read afterwards, and a channel added between the two holds a handle
nobody closes.

**Dispose is asynchronous, and its task means something.** Both are `IAsyncDisposable`:
a channel's task completes once its calls are settled and its handle released, and the
runtime's once every channel it made has gone that way and `ak_runtime_destroy` has
returned. A synchronous `Dispose` would have to block on the
network and on host callbacks, which is why the surface does not offer one - and why
neither type implements `IDisposable`. That absence is what makes `await using` a rule
rather than a recommendation: `using var` on a type that is only `IAsyncDisposable` does not
compile (CS8418), so a caller who forgets is told by the compiler instead of by a channel that
never drains. The call wrappers keep gRPC's own shape and expose `Dispose`; the binding's
extension is the channel's asynchronous one.

### Trampoline

```csharp
// Rooted for the runtime's lifetime as a static delegate: netstandard2.0 has no
// UnmanagedCallersOnly. Runs on a Tokio thread, hands the event to whoever it
// names, and returns. No user code, and no binding-managed payload allocation on
// the measured fast path.
private static readonly unsafe NativeMethods.ak_runtime_create_callback_delegate Trampoline = OnEvent;

internal static unsafe void OnEvent(void* runtimeCtx, void* callCtx, ak_event* evt)
{
    // A call's events carry its call_ctx; the runtime's two carry runtime_ctx alone.
    // A root that resolves to nothing is an event nobody can take.
    object? target;
    try { target = GCHandle.FromIntPtr((IntPtr)(callCtx != null ? callCtx : runtimeCtx)).Target; }
    catch { NativeMethods.ak_event_consumed(evt->payload); return; }

    var call  = target as ICallSink;
    var taken = false;
    try
    {
        if (call is not null)
            // Metadata, message, terminal: the next ring slot, published with a
            // release store on the head. WRITE_DONE: the armed write's acquittal,
            // which takes no slot and must not queue behind a data callback.
            taken = call.Publish(evt->kind, evt->payload, evt->status_code);
        else if (target is NativeRuntime runtime)
            // SHUTDOWN_COMPLETE or RESOURCES_RELEASED: a wake-up, and the waiter
            // reads the state again. Neither frees the runtime's root.
            runtime.announced_.Set();
    }
    catch
    {
        // Nothing may unwind into the engine.
    }
    finally
    {
        // What the ring did not take is given back here, and only here.
        if (!taken)
            NativeMethods.ak_event_consumed(evt->payload);

        // The call's last callback: its root goes, whatever the publish did.
        // Managed references keep the object alive, so this collects nothing -
        // it stops the ABI from resolving a call_ctx that no longer names anything.
        if (call is not null && evt->kind == ak_event_kind.AK_EVENT_STATUS)
            call.TerminalReturned();
    }
}
```

The acquittal completes the armed write without taking it out of its field, because the
same field is the one-write claim and the writer releases it itself. That the acquittal is
that write's, and not a later one's, is the ABI's promise rather than the binding's
check: WRITE_DONE arrives exactly once per accepted send, in send order, and a caller
honouring the one-writer contract has no second write armed until the first returned.

The trampoline is the level-1 callback boundary: it runs on a native thread, and its
return is what the model calls `DeliveryCallbackReturns` (or `WriteDoneReturns`). Keeping
it allocation-free on its measured fast path and lock-free is not an optimization but the reason the native actor
can promise to make progress without the host: the proved liveness assumes the callback
returns, and nothing else.

Treating metadata as an ordinary payload is what makes that literal. Decoding it here
would allocate, and it would need a `finally` to avoid leaking the payload on a malformed
header blob - a failure path across the FFI boundary, which is the worst place to have
one. Publishing a slot cannot fail. The trampoline catches everything all the same,
because nothing may unwind into the engine, and what a throw would have leaked its
`finally` gives back regardless: a payload the ring did not take, and on the terminal the
call's root.

The trampoline never copies a payload. It publishes the owned `ak_bytes` and the consumer
releases it after parsing, on a managed thread.

No registry: the `call_ctx` is directly a `GCHandle` to the call, allocated before
`ak_call_start`.

**Who frees that root, and when, is not the consumer's business.** Reclamation is the
runtime's own step and the host is not told when it happens, so tying the root's lifetime
to it is not even an option. Two rules remove the question instead:

- every callback resolves `call_ctx` into a strong local reference before touching
  anything, so the object stays reachable for the whole callback regardless of the root;
- the terminal callback frees that root itself, after its last access to the call.
  It is the last callback of the call, so that is where native use ends - and the managed
  side keeps its own references, so freeing the native root collects nothing.

The `runtime_ctx` root does **not** follow that shape, and the difference is the whole point.
It is released after `ak_runtime_destroy` returns, never inside a callback. Every callback of
every kind carries `runtime_ctx`, so there is no last one to hand the free to: releasing it on
`SHUTDOWN_COMPLETE` would be a use-after-free whenever `host_debt` says the host still owes a
return. `ak_runtime_destroy` returning is the only point at which nothing can be in flight,
which is what the ABI's "valid until the last event of the runtime" rule amounts to.

### CallState (per call)

```csharp
// Allocated and GCHandle.Alloc'd BEFORE ak_call_start.
// The GCHandle is passed as call_ctx, and the terminal callback frees it itself
// after its last access to CallState - not the consumer.
class CallState
{
    // The delivery ring is the stream queue: metadata, messages and the
    // terminal all ride it, so there is one buffer per call, not two.
    // NextPow2(DeliveryCredits + 1): the ABI never leaves more than
    // DeliveryCredits + 1 payloads outstanding, and Head and Tail are
    // monotonic counters rather than wrapped indexes, so occupancy is
    // Head - Tail and empty is already distinct from full without a slot
    // spent to tell them apart. It therefore cannot fill.
    Slot[] Ring; int Mask;
    long Head;                     // published by the actor thread
    long Tail;                     // private to the consumer
    IAsyncSignal RingSignal;       // wakes the waits taken before a Set, never SemaphoreSlim

    Task HeadersTask;              // slot 0, driven by whoever asks first
    Metadata Headers;              // decoded on the pool, from slot 0
    TaskCompletionSource<GrpcStatus> StatusTcs;         // RunContinuationsAsynchronously
    CancellationTokenRegistration CancelRegistration;
    GCHandle SelfHandle;                                // the GCHandle passed as call_ctx

    TaskCompletionSource WriteTcs; // the pending WriteAsync, completed by its
                                   // WRITE_DONE; RunContinuationsAsynchronously.
                                   // No slot counter and no send signal: one writer
                                   // completing at WRITE_DONE never finds the window
                                   // full, so there is nothing to wait for
    // Buffers lent by ak_get_call_buffer and not yet given back. Every one
    // of them must be returned, or the call is never reclaimed.
    ConcurrentBag<ak_buffer> LentBuffers;               // native depth allows
                                   // MaxSendsInFlight; this binding exercises one, the
                                   // writer being single and completing at WRITE_DONE
}

struct Slot { public ak_bytes Payload; public int Kind; public int Status; }
```

`Head` is published with `Volatile.Write` and read with `Volatile.Read`; that pair is
what makes the slot's fields visible, so the fields themselves need no volatility of
their own. `Tail` needs no barrier at all: the producer never observes the consumer,
because the credit bound removes any need to test for fullness. On x64 the pair costs
nothing; on ARM64 it is the difference between correct and not.

### The send side: giving back is the host's half of the contract

The host never owns send memory: `ak_get_call_buffer` lends it, protobuf serializes
straight into it, and `ak_call_send_message` gives it back. There is nothing to pin and
nothing to copy. What the host does owe is the return, exactly once, on every path:
a refused send, a thrown serializer, a cancelled call, a disposed stream writer all end
with `ak_return_call_buffer`. A `using` on the lent buffer is the whole discipline.

That obligation is not decorative. A call is not reclaimed while a buffer is out, so a
binding that forgets one leaks the call's arena rather than corrupting it - a leak instead
of a fault. It is also the obligation that lost its synchronous check when
`ak_call_release` left the ABI: nothing now refuses at the moment of the mistake.
`ak_call_debt_of` is what puts that check back, in tests and assertions rather than on the
hot path, and `BufferEventuallyFreed` is what the model asks of the host in exchange.

A refusal (`AK_STATUS_SLOT_BUSY`) is backpressure and not an error, with exactly one
cause - this call's window - so WRITE_DONE is a wake-up the host can rely on. That is a
property of the ABI and of level 1, for a host that pipelines deeper: this binding never
meets it, because a write completes at its WRITE_DONE and the window is therefore always
open at the next lend (`ManagedWriterNeverObservesSlotBusy`).

**The write machine.** `WriteAsync` is a per-call state machine, and its linearization
is fixed: **a write completes at its WRITE_DONE**, not at the commit. One writer per
call is `IClientStreamWriter`'s own contract - no concurrent `WriteAsync`, no
`CompleteAsync` beside a pending write - so the machine has one value per call:

- *idle*: no write pending. `WriteAsync` begins with the lend;
- *serializing*: the lend succeeded, the marshaller writes into the lent buffer - the
  one state that holds a buffer, closed by the disposable wrapper on success and
  exception alike;
- *waiting_budget*: `BUDGET_BUSY` - the cancellable wait, remembering the refused
  length;
- *awaiting_write_done*: the commit was accepted; the write's task completes when its
  WRITE_DONE callback runs;
- *closed*: `CompleteAsync` was called (`end_send`) - legal only from idle.

**There is no slot wait, and that is a consequence, not an omission.** The window frees
at the WRITE_DONE's *emission*, and the pending write completes at that same
WRITE_DONE; a conformant caller cannot start the next `WriteAsync` before its previous
one completed, so the next lend always finds a free slot - whatever the native depth.
`AK_STATUS_SLOT_BUSY` therefore never reaches a conformant managed writer, and level 2
states exactly that (`ManagedWriterNeverObservesSlotBusy`) rather than modelling a wait
no behaviour can enter. The status stays in the ABI and in level 1, where a
deeper-pipelining host is admitted; a SLOT_BUSY observed by this binding is a defect in
it, and the invariant is where that shows. The slot counter and the send signal leave
`CallState` with the wait.

There is no slot wait on this surface: with a single writer completing at its
WRITE_DONE, the emission that completes one write has already freed the window, so the
next lend finds it open - `SLOT_BUSY` is unreachable here and the model says so.
`MESSAGE_TOO_LARGE` faults the write synchronously and enters no wait: the refusal is
permanent, retrying it would poll against a constant. Cancellation or dispose resolves
a waiting writer exceptionally, exactly as they resolve a waiting reader; a write
already committed settles through its WRITE_DONE, which level 1 guarantees before the
terminal.

**Managed completions.** Three public objects must never be left pending: the headers
(`ResponseHeadersAsync`), the status (`StatusTcs`), and the pending write above.

The headers resolve when the prologue consumes slot 0; a dispose before the metadata
faults them with an `RpcException` carrying `StatusCode.Cancelled`, the one exception
type this binding uses for every cancelled path - see "What no level of the specification
covers" in formal-model.md, where that decision is stated.

**The status is resolved by whoever consumes the terminal slot, never by the callback.**
The terminal callback copies `ak_bytes`, the kind and the code into the ring, publishes
the head and returns - it decodes nothing, which is what keeps it total and off the
user's path. But the ABI's status payload carries the message and the trailing
metadata, so the public status cannot be built from the slot's code alone: the terminal
consumer - the application's reader, or the drain when a dispose got there first -
parses the payload, resolves `StatusTcs` and the wrappers that hang off it, and only
then calls `ak_event_consumed` on it. A dispose therefore completes *after* that
resolution and never depends on bytes it already returned. This is also grpc-dotnet's
own discipline: a streaming call's status is settled once the response stream and its
trailers have been read, not when the response head arrives.

The unary shapes are compositions of the same machine rather than a fourth object: the
one-shot reads its single message through the reader and then its status through the
terminal consumer above, so `Task<TResponse>` succeeds exactly when both did. A
settled call leaves no managed waiter - reader, writer, headers, status - and level 2
states it (`DisposeLeavesNoManagedWaiter`).

WRITE_DONE must never queue behind a slow message handler - the ABI states it may arrive
in parallel with data callbacks for the same call - so it stays out of the delivery ring
and frees its slot on the spot. That is safe because it runs no user code: a counter
increment and a signal.

### No dispatcher: the ring is the queue

There is no dispatcher thread and no process-wide host queue. The trampoline publishes
into the call's own ring and the consumer reads it directly, so an event crosses one
buffer instead of two and nothing is allocated to describe it.

A dedicated dispatcher thread would buy nothing here. The release
(`ak_event_consumed`) happens in the application's parse, on the pool, however the event
was routed, so no dispatcher can protect `PayloadsEventuallyConsumed` - that hypothesis
rests on the application either way. What a dispatcher does protect is routing under pool
pressure, and keeping every wakeup off the Tokio thread buys the same thing more cheaply.

**Nothing may run user code on the callback's thread.** With no dispatcher standing
between the trampoline and the application, this is the *only* thing that keeps the
native actor free to make progress, so it is a rule and not a preference:

- every `TaskCompletionSource` is built with `RunContinuationsAsynchronously`;
- `RingSignal` wakes every wait taken before a `Set` and keeps nothing for one
  taken after, and its `Set` never runs a waiter inline and takes no lock - a
  `TaskCompletionSource` built with `RunContinuationsAsynchronously`, swapped out
  atomically and completed by a `Set` that has a wait to wake;
- **never `SemaphoreSlim`.** Its `Release` can complete a `WaitAsync` waiter inline
  depending on the runtime version, which would put application code on the Tokio thread
  through the back door the two flags close at the front.

The signal only says "something may have changed"; the truth is `Head != Tail`, so merged
wakeups cost nothing and the consumer drains what is there before waiting again. The order
is what matters: every consumer of the ring - the prologue, a read, the drain - takes its
wait before it looks at the ring and at its own phase, and awaits it only when the look
found nothing, so a publication in the window between the look and the await sets the wait
already taken, and one before the look the look saw. A signal that kept a wake-up for the
next wait would close that window for one consumer and open it for another: whichever
waited next would take it, and a read that looked, stalled and then waited could find the
drain had taken the one that told it it was cancelled.

```text
MoveNext(ct), every step in this order:

1. publish the read - reader waiting, this ReadOp the call's current read, in one store -
   then register ct, whose callback is ReadOp.Fire
   // TLA: BeginMoveNext
   the token: if the read is still undecided, cancel the call - CancelAndDrain, idempotent
   and non-blocking - and never wait for the marshaller
   // TLA: RequestReadCancellation, then CancelWaitingRead / CancelParsingRead
2. await EnsureHeadersAsync under this read's registration: slot 0 is the prologue's
   // TLA: ConsumeHeader, and CancelWaitingRead on the token's side
3. loop: take the ring's wait, then claim - TryBeginParse moves the reader from waiting
   to parsing; Acquired breaks, Lost ends as Cancelled, Empty awaits the wait taken first
   // TLA: BeginParseEvent against HandoffToDrain
   an exit before ownership disarms what is armed and retracts the read -
   AbortWaitingRead(op). A wait the call's cancellation ended answers Cancelled, and no
   drain: whatever cancelled the call ran CancelAndDrain already. Any other such exit - a
   lost claim, a header decode, an internal fault - calls CancelAndDrain as well
4. own the slot: branch on the sum - a terminal decodes the status, a message the response
   type - and remember the outcome rather than throw it
   // TLA: ParsingReadOwnsItsSlot
   a terminal whose decode failed makes a synthetic Internal status here, before the
   release, for step 6 to resolve
5. disarm - DisposeAsync - and only then decide: op.TryWin, one CompareExchange
6. in one block that runs even when step 5 throws, and that nothing leaves early: resolve
   the status, ak_event_consumed(owner), advance the tail, PublishIdleOrFinished -
   republish, and pump a drain that is owed
   // TLA: FinishConsumePayload if this read won, FinishCancelledParse
   // if the token did, then HandoffToDrain
7. answer the winner: Cancelled if the token won; a terminal answers from the status -
   false on OK, the same RpcException otherwise, a failed decode included; a message that
   failed to decode faults the call - CancelAndDrain - and its exception is rethrown with
   its stack intact; a message becomes Current, and true
```

Two rules the steps do not show. **The tail advances with the release, never before it:**
the ring's tail is exactly the runtime's consumed count, which is what lets the model read
it straight off `payloads_consumed_by_host`, and moving it at the claim would put the managed
index one ahead for the whole parse and make that mapping false. **The helpers of the
acquittal block are total, and only `ak_event_consumed` touches the native payload:** what
the decode produced is the managed copy, so nothing handed to the others after the release
still reads the bytes it gives back. They never throw, because an exception would jump out of a
sequence the machine performs as one step - `ResolveStatus` is a `TrySet`,
`StatusFromDecodeFailure` only wraps, `PublishIdleOrFinished` publishes and pumps,
`ak_event_consumed` is the void downcall - and `AbortWaitingRead` and `CancelAndDrain` are
idempotent, non-blocking and non-throwing besides, because a token callback runs them.

**The steps above are normative rather than illustrative**, and each names the action of
the machine it realizes - which is how a divergence in ordering becomes visible at all. The read's state is
published *before* `ct.Register`, because `Register` invokes the callback inline when the
token is already cancelled and it must find this read rather than the previous one.
`PublishWaiting` is that store: it makes the reader `waiting` and installs this `ReadOp` as
the call's current read, and it must be one publication rather than two - a callback that
observes the new state but the old op, or the reverse, is the stale-attribution bug the
whole ordering exists to prevent. The
registration is disarmed, awaited, *before* the result is decided - `DisposeAsync` returns
only once no callback of that registration is running or ever will, so a decision taken
after it cannot be overturned, whereas a test taken before it can. And success and
cancellation share one linearization point: a single `CompareExchange` per read, whose
loser does nothing. A last-moment check of the token is not a weaker version of this - it
is a different, wrong thing, because the window between the check and the task's
completion is exactly where the callback lands.

The metadata is consumed inside the read, not before it: `EnsureHeadersAsync` runs under
this read's own registration, so a token firing while the headers are outstanding faults
the read and the headers together instead of finding no operation to cancel. And the slot
is taken by a transition, not by a peek: `TryBeginParse` moves the reader from `waiting` to
`parsing` and that move is what confers ownership, so the drain's handoff - which takes the
ring only from an idle or finished reader - and this read cannot both believe they hold the
tail.

The payload goes back *after* the winner is known, and it goes back on every path. This is
the ordering the model fixes and the one an implementation is most likely to get wrong:
`FinishConsumePayload` and `FinishCancelledParse` are each a single step that acquits the
slot, resolves the terminal status if that is what it held, and republishes the reader.
Releasing before the decision splits that step in two, and the state in between - tail
advanced, result undecided, reader still owning the operation - is one the machine does not
have. No level below will ever formalize it, so the code must not create it. The mirror
mistake is as easy: moving the release out of a guaranteed block to fix the ordering leaves
a throwing marshaller holding the slot for good, and `InFlightPayloadEventuallyReleased`
then has no implementation. So the decode's outcome is *remembered* rather than thrown -
value, end, or failure - and the release runs unconditionally once the marshaller has
returned control, however it returned it. Only a marshaller that never returns at all is
left to the stated hypothesis.

**A read has four outcomes, and the branch on which comes first.** `BeginParseEvent` takes
an *event*, and the ring carries a sum: a message, or the terminal one bearing the status
and the trailers. The terminal payload is not a message of the response type, so the branch
has to precede any marshaller - decoding it as `T` reads the wrong format, and `GetStatus`
loses the only copy of the status when the slot is released. After the release the winner
decides: a message becomes `Current` and `true`; a clean end becomes `false`; a failing end
becomes the stable `RpcException`; and a token that won becomes `Cancelled` whatever the
slot held, the status still resolved if the slot was terminal.

**The registration stays armed until the marshaller has returned.** That is the whole
point of `parsing_cancelled`: `RequestReadCancellation` is enabled for any read in flight,
a parse included, and `CancelParsingRead` is weakly fair, so a token firing during the
decode must reach `ReadOp.Fire`. Disarming earlier deletes that behaviour - the trace
`RequestReadCancellation` then `CancelParsingRead` then `FinishCancelledParse` simply has
no implementation, `TryWin` always succeeds once the slot is claimed, and a marshaller that
never returns can no longer even be cancelled at the transport. So the acquisition phase
carries no `finally`: a `finally` runs on the *normal* exit too, which would disarm the
registration the instant the slot is claimed. Pre-ownership exits disarm in their own catch
clauses, and the post-ownership path disarms once, where the decision needs it.

**Leaving `waiting` is the reaction's job, not the waiter's.** A wait cancelled by
`_callCancelled` leaves as the binding's one public rule, `RpcException(StatusCode.Cancelled)`
and never `OperationCanceledException` - no caller has to ask which token fired. What moves
the reader out of `waiting` is `CancelAndDrain`, and its contract is per state, which its
name does not say: on a `waiting` reader it performs the reaction of `CancelWaitingRead` -
cancel the call, fault the read, fault the headers if they are still pending, take the
consumer to the drain, and set the ring's signal so the wait observes it; on a `parsing`
reader it cancels the call and marks the drain owed but never takes the slot, which stays
the marshaller's until it returns; on an idle or finished reader it cancels the call and
drains. It is idempotent, non-blocking and non-throwing in all three, because the token's
callback runs it and `DisposeAsync` may already be waiting for that callback - a
`CancelAndDrain` that waited for the marshaller would close the cycle.

**A decode failure and the token share one arbiter.** They can cross, so the order must be
decided rather than left to chance, and one `CompareExchange` per read decides both: if the
token won, the call is already cancelled and `Cancelled` is the truthful answer, the failed
bytes being of no further interest. If the read won, the decode failure is published and it
cancels the call too - a stream whose bytes do not decode cannot continue. One arbiter, one
winner, no second policy to keep consistent.

**`EnsureHeadersAsync` is total about slot 0.** The prologue owns the metadata, and a decode
that throws while holding it is the one exit where retracting the read is not enough: the
reader is gone, the call is still active, `HeadersTask` is faulted for good, and slot 0 is
still owed. Every later `MoveNext` re-observes the same faulted task while the call can
neither progress nor settle without the application disposing it. So the contract is the
terminal's: **when the header decode returns control, normally or by exception, slot 0 is
either acquitted exactly once or handed to a drain that is actually scheduled.** The sketch
takes the second form - the pre-ownership catch faults the headers, retracts the read and
calls `CancelAndDrain`, whose handoff collects the slot - which keeps the acquittal in one
place rather than two. A failing header decode therefore makes the call unusable and drains
it, which is the honest outcome: nothing can be read from a stream whose metadata did not
parse.

**A read that fails before it owns anything must still be retracted.** Publishing the
operation before `ct.Register` is what makes an already-cancelled token find the right read,
but it also means the publication can outlive the attempt: `Register` throws
`ObjectDisposedException` when the caller's source has already been disposed, and at that
point nothing is armed to disarm - yet a read stands published with no one to run it, and
the next `MoveNext` sees a phantom concurrent read. The same hole opens on any pre-ownership
exit: disarming the registration stops a late callback, it does not retract the operation.
So there is one primitive for it, `AbortWaitingRead(op)`: if `op` is still the current read
and the reader is still `waiting` it removes it, and if a dispose or the drain has already
taken the consumer it changes nothing at all. It never touches a slot, and it is idempotent
against a concurrent `CancelAndDrain`. Moving the publication after `Register` is not the
alternative - that loses the already-cancelled token, which is the case the order exists
for.

**Leaving a parse must wake the drain that is owed.** `CancelAndDrain` on a parsing reader
cancels the call and marks the drain owed without taking the slot, which is right - the
marshaller keeps its borrow. But the model then gets `HandoffToDrain` from weak fairness the
moment the reader is no longer parsing, and code has no spontaneous fairness: an action that
becomes enabled runs only if something schedules it. The gap matters most exactly where it
is hardest to see, on a parse that held the *terminal*: after its release no further native
event will ever arrive to set the ring's signal, so a drain owed to a bit and nothing else
is owed forever, and `DisposeAsync` never completes. `PublishIdleOrFinished` therefore
publishes *and* pumps: it observes the owed flag in the same atomic step as the
republication and schedules exactly one drain continuation, single-flight, with no user code
inline and no waiting under the atomic. The ring's signal is a data wake-up and must not
double as an ownership-change wake-up; conflating them is how this stall hides.

**A terminal always yields a terminal outcome, even when its decode fails.** This is the
one place where an exception cannot simply be reported: once the slot is released the event
is gone and its trailers with it, so nothing can produce the status afterwards - not the
drain, which finds an empty ring, and not a later read.  `GetStatus`, the settlement and
`DisposeAsync` would wait on a status no step can still resolve, which is
`StatusEventuallyResolved` failing in the code while holding in the model.  So a terminal
whose decode throws resolves a stable synthetic status - `Internal`, carrying the decode
error - before the release, and every later read and `GetStatus` answers from that same
value - and so does the read that consumed it.  That last point is the one easy to get
wrong: answering the raw decode exception there while every later read answers the synthetic
`RpcException` gives two different terminal results for one stream, where
`IAsyncStreamReader` promises the same answer every time.  So the terminal branch answers
from the status, always; a token that won still reports `Cancelled`, and the status is
resolved either way.  Cancelling the call afterwards is not a substitute - `ak_call_cancel`
on a call that has already ended produces nothing to decode.

**An empty ring is not a lost race.** After the metadata is consumed it is entirely normal
for no message or terminal to have been published yet, and the model simply stays in
`waiting` until `BeginParseEvent` becomes enabled. A boolean `Try` cannot say which
happened, so the claim is three-valued: acquired, empty, lost. Empty waits on the signal;
lost means the drain holds the consumer and the read ends exceptionally. The signal is a
wake-up and confers nothing - only the transition does - and the read takes its wait before
the claim, or a publication landing between the empty observation and the wait is lost and
the read hangs on a stream that has already spoken.

These are what the model states: `ReadCancellationSettled` says a posted request is not
carried away by the normal end of a read, `LiveRequestOnlyDischargedByReaction` says only
the binding's reaction may discharge it, and `ParsingReadOwnsItsSlot` says the borrow lasts
exactly as long as the parse. An implementation that tests the token last, releases before
deciding, or releases only on the paths that did not throw, satisfies none of them.

**The directed tests this path owes**, each asserting the same three things - the task
resolves exactly once, `ak_event_consumed` runs exactly once, and no cancellation is
attributed to a later read.

On the races: a token already cancelled before `MoveNext`; cancellation while waiting for
the metadata; cancellation before and after the claim; cancellation during the decode;
cancellation between the disarm and the decision; a callback already running when
`DisposeAsync` begins; the read winning just before the callback; a previous read's token
firing during the next read. The last two are what the single `CompareExchange` exists for,
and the only ones that fail silently without it.

On the outcomes: a message; a clean end, which must answer `false`; a failing end, which
must answer the same `RpcException` every time; Trailers-Only, where the terminal is the
first event after the metadata; and an empty ring - metadata consumed, nothing published
for a controlled interval, then a message - where `MoveNext` must stay pending and deliver
that message rather than reporting cancellation.

On the decode: a message marshaller that throws; a trailers decoder that throws; each of
those crossing the read token; and each crossing a concurrent `Dispose`. Every one of them
must show the tail advanced by exactly one and a single `ak_event_consumed`. A failed
terminal decode is checked twice over: the read that consumed it and several reads after it,
plus `GetStatus`, must all report the same synthetic status.

On the exits that own nothing: a token taken from a source disposed before `MoveNext`, so
`Register` throws; an injected failure in `EnsureHeadersAsync`; an injected failure in the
signal wait; each crossed with a concurrent `Dispose`. After every one of them no read may
remain published - the next `MoveNext` must not be refused as concurrent - and no callback
may reach a later read. The header failure is checked further, because it is the only one
holding a slot: inject after slot 0 is acquired and before the `Metadata` is built, then
assert `HeadersTask` faulted exactly once, `ak_event_consumed` exactly once for that owner
whether immediately or through the drain, the tail advanced by exactly one, the call drained
with no further act from the application, `DisposeAsync` completed, and no second attempt
decoding bytes already returned.

And the one that decides whether the drain has a mechanism at all: the reader holds the
terminal, a token or a dispose wins while the marshaller is deliberately blocked, and no
further native callback is allowed. When the marshaller is released, `ak_event_consumed`
must be exactly one, the drain must take the consumer, and `DisposeAsync` must complete.
Nothing but an explicit wake-up makes that trace pass; a flag alone hangs it.

**Slot 0 is always the metadata**, and level 0 proves it: `EventStreamShape` says the
first delivered event of a used call is `INITIAL_METADATA`, and the ABI never skips it,
not even on a call cancelled before the response head. So the host needs no inspection to
know what it is taking. It is consumed by whoever asks first - awaiting the headers, or
reading the first message - through an idempotent `EnsureHeadersAsync`, which is also
what stops the two from overlapping. `ResponseHeadersAsync` therefore completes on
arrival rather than on the application deciding to read, which is what gRPC promises.

**What the headers answer is grpc-dotnet's**, read off the head's origin: the peer's
headers when they came; for a response that delivered no head - the Trailers-Only shape,
or an answer refused before its body - that response's trailers, whatever its status, as
grpc-dotnet returns the headers of any HTTP response once it arrives and before it reads
it; and, when no response reached the call, the call's status as an `RpcException`.
Whoever takes the head records its origin, and the terminal answers the headers the head
did not. The prologue waits for that terminal, the only event that follows such a head,
before it consumes slot 0, so the headers are answered without a read - the model's
`ConsumeHeader`, which leaves the outcome open because it does not carry the origin. A
call that ends first leaves the head to the reader or the drain, and the terminal answers
the headers there. grpc-go's `Header()` answers a call no response reached with no
headers and no error, and leaves the error to the read; the binding does not, because
.NET code is written against grpc-dotnet and may take answered headers as a sign that a
response came.

The ring carries a sum, not just messages - the managed mirror of the Rust
`RecvResult::Message | RecvResult::End`. The terminal takes its place *in* the ring
instead of being released on a side path, so whoever drains it releases the payloads in
the order they were delivered.

Deserialization is lazy and zero-copy **on the fast path**: the native buffer travels to
`MoveNext`, protobuf parses straight out of it through a `DeserializationContext` over
the native span (`PayloadAsReadOnlySequence`), and the `using` releases it at the end of
that parse. Nothing copies, and no payload ever outlives the parse that reads it - so no
finalizer and no GC ordering question ever enters the picture.

That path is not universal, and claiming it would be is the kind of promise that breaks
in the field. A `CallInvoker` receives whatever `Marshaller<T>` the stub was generated
with. The contextual marshallers protobuf generates today take the fast path on both
sides: `SetPayloadLength(CalculateSize())` then a write into the lent buffer, and a parse
straight off the native sequence. A marshaller that produces a `byte[]`, or a
deserializer that calls `PayloadAsNewBuffer()`, cannot: the binding copies once, in the
managed direction, and everything else - the credits, the release order, the send window -
is unchanged. Both paths must exist and be tested; only the first is zero-copy.

Unary is not a special case. Its single response travels the same ring, and the
`Task<TResponse>` is completed by a one-shot reader that parses and releases exactly as
`MoveNext` does. Parsing the lone response on a side path would put a second consumer on
the ring for one call shape out of five, which is the one thing the release order cannot
survive. One path, one consumer, and the obligations below hold for every shape rather
than for most of them.

This is also what keeps `DeliveryCredits` meaningful. Because a payload is released only
when the application has parsed it, the native side genuinely withholds the next message
until the reader has caught up: the credit is application backpressure, not an accounting
detail absorbed by a managed buffer.

#### Release ordering is an obligation, not an intention

The native accounting is a counter, so the host owes releases **in delivery order**. Four
rules make that hold, and they are the level-2 proof obligations:

- **The ring index is the order.** `Tail` advances by one per release, and the slot it
  names is the payload the native counter is about to free. There is nothing to arrange:
  the order is the data structure, not a property of whoever drains it.
- **One consumer at a time**, in three phases: the header prologue owns slot 0, then the
  application while the call is live - the one-shot reader for the unary shapes - then
  the drain after `Dispose`. Each hands over on completion, never concurrently; two
  consumers would interleave releases and break the order with no way to detect it.
- **`Dispose` drains in order, and that is all it does.** Disposing the call requests
  cancellation and hands the ring to the drain, which releases what is left from `tail`
  up. `CancellationCompletes` proves the terminal arrives without any further host
  action, so the drain reaches it and releases its payload last. There is nothing to call
  afterwards: the runtime reclaims the call when that last release clears the debt.
  `Dispose` does not free the call's root either - the terminal callback already did, on the
  native side, after its own last access. A parse already in flight on the application thread completes
  first; the drain starts behind it, never beside it.
- **Exactly once.** `OwnedMessage` releases on its first disposal and is inert afterwards.
  The native side never re-issues an index, so a payload freed twice could only come from
  the host - and that is the one failure the ABI cannot detect.

No callback may resolve a context after its root is freed, and none does. The two roots
have different lives: a call's is freed by that call's terminal callback, which is its
last; the runtime's is freed after `ak_runtime_destroy` returns, later than every event of
every kind. Inside any callback the strong local keeps the object alive whatever happens to
the root, which is what makes the rule checkable rather than a matter of timing. That is
`ShutdownSignalInv`'s managed counterpart, stated at level 2 as `RootSurvivesCallbacks`.

### NativeCallInvoker — CallInvoker mapping

The 5 `CallInvoker` methods translate as follows:

| CallInvoker method | Implementation |
|--------------------|----------------|
| `BlockingUnaryCall` | start + send + end_send + await the one-shot reader (blocks the thread) |
| `AsyncUnaryCall` | start + send + end_send + return the one-shot reader's Task |
| `AsyncClientStreamingCall` | start + expose write stream + return Task<response> |
| `AsyncServerStreamingCall` | start + send + end_send + expose read stream |
| `AsyncDuplexStreamingCall` | start + expose write stream + expose read stream |

The five shapes differ only in what is wrapped around the per-call ring; none of them
bypasses it.

Each call:
1. Serializes the request to bytes (protobuf, done by the stub)
2. Allocates a `CallState`, does `GCHandle.Alloc` on it
3. Calls `ak_call_start` with the GCHandle as `call_ctx`
4. Returns the appropriate object (AsyncUnaryCall, etc.) that wraps the CallState's TCSs

### Configuration — generation chain

The complete chain is:

```text
armonik_transport::options::{ChannelOptions, and the units it nests}
    │ derive(schemars::JsonSchema), under the `schema` feature
    ▼
options.schema.json      <- committed beside the crate; a test fails when it is not
    │                       what the types describe
    │ ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator, a build-time tool
    │ over Corvus.Json.CodeGeneration's TypeDeclaration model
    ▼
ChannelOptions.g.cs      <- committed; the build compares it with what the schema
    │                       renders, and never rewrites it
    │ ChannelOptions.Encode()
    ▼
UTF-8 JSON, ak_channel_create's config_json - the endpoint is its own argument
    │ serde_json into ChannelOptions, every bound checked again; each unit's
    │ conversion, in armonik-transport, reads the certificate files it names
    ▼
GrpcChannelConfig and TransportConfig, the engine's own
```

A path crosses the ABI and a certificate does not: the binding writes the path a caller gave,
and the engine reads the file, so one reader serves every host and a file that names nothing
usable is refused at `ak_channel_create`, by the option's name and never by its path.

The schema is the generator's only input, so the vocabulary lives in one place. T3.3 settled
the shape every option takes, and gives the reasons:

- a duration is a number of seconds, and the option's name carries the unit -
  `ConnectTimeoutSeconds`;
- a count is an `int`, and its range is a constraint of the schema rather than of an unsigned
  type;
- every constraint that can be said in the schema is said there;
- a default is stated in its option's description and nowhere else in the schema: applying it
  is the reader's, and a test compares the two;
- nothing is nullable: unset is absent;
- `additionalProperties: false` everywhere, so an unknown option is refused rather than ignored;
- nothing is required, and `{}` is a valid configuration.

A binding may narrow what the schema admits, and cannot widen it: the engine checks every bound
again. The schema states what the engine can honour, and a binding that sizes something of its
own from an option bounds it by what that costs on its side. The .NET binding does so once. Each
call allocates its delivery ring at the next power of two above `DeliveryCredits`, so it refuses
a window past `NativeRuntime.MaxDeliveryCredits`, 32768 - 65536 slots, two megabytes per call in
a 64-bit process - where the schema admits 536870910, just under what a tokio semaphore holds
on a 32-bit target. `MaxSendsInFlight` sizes nothing on the .NET side, and keeps the schema's
bound.

Options that name material - a certificate, an identity, a proxy - arrive with the tasks that
read them, from T4.1 on.

---

## Layer 5 — `ArmoniK.Api.Client`

### Integration point

```csharp
// The existing client accepts an injectable CallInvoker:
public class SessionsClient
{
    public SessionsClient(CallInvoker callInvoker) { ... }
}

// Usage with the native channel:
await using var runtime = NativeRuntime.Create();
await using var channel = runtime.Channel("http://armonik:5001");
var client = new Sessions.SessionsClient(channel.CreateCallInvoker());
```

### Existing options mapping

The current client options (`GrpcChannel` in ArmoniK.Api.Common.Options) must be able to
produce a `ChannelOptions`. The mapping is explicit and tested:

| Existing option | ChannelOptions field |
|-----------------|--------------------------|
| `Address` | `Endpoint` |
| `CaCert` | `Transport.Tls.CaCertPath` |
| `ClientCert` / `ClientKey` | `Transport.Tls.CertPem` / `Transport.Tls.KeyPem` |
| `ClientP12` | `Transport.Tls.CertP12`; its password, `Transport.Tls.CertP12Password`, has no counterpart |
| `AllowUnsafeConnection` | `Transport.Tls.AllowUnsafeConnection` |
| `OverrideTargetName` | `Transport.Tls.OverrideTargetName` |
| `Proxy` | `Transport.Proxy.Address` |
| `ProxyUsername` / `ProxyPassword` | `Transport.Proxy.Username` / `Transport.Proxy.Password` |
| `RequestTimeout` | `DefaultDeadlineSeconds` |
| `MaxAttempts` | `Retry.MaxAttempts` |
| `InitialBackOff` etc. | `Retry.*` |

---

## What is missing

**The `CallInvoker` mapping owes `CallOptions` in full**: per-call credentials, host override,
write options and method type; the deadline, the cancellation token and the request metadata are
carried. The table above maps the five call shapes and stops there.
