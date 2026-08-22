# DESIGN — .NET ArmoniK Client on Native Rust gRPC Channel

## Introduction

This document details the technical decisions for each layer of the architecture defined in
[SPEC.MD](SPEC.MD), in response to the requirements in [requirements.md](requirements.md).

It establishes the APIs, types, sequences, error contracts, and the foundations of the TLA+
formal model. Implementation choices (specific crates, internal algorithms) remain free as long
as they comply with the contracts described here.

---

## Layer 1 — `armonik-transport`

### Public contract: `tower::Service<Uri>`

The connector exposes a `tower::Service<Uri, Response = Connection>` where `Connection`
implements the Hyper traits required (AsyncRead + AsyncWrite + Connection info).

```rust
/// The connector produces TCP connections (+ TLS if configured) to a URI.
/// It knows neither HTTP/2 nor gRPC - it is a network dial.
pub struct TransportConnector { /* ... */ }

impl tower::Service<Uri> for TransportConnector {
    type Response = TransportConnection;
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn call(&mut self, target: Uri) -> Self::Future;
}
```

### Connector guarantees

- `poll_ready` does not perform blocking I/O (only availability check)
- `call` performs DNS resolution, TCP dial, TLS handshake if configured
- The connect timeout applies to the entire sequence (DNS + TCP + TLS + proxy CONNECT)
- An error is a structured `TransportError` (see below)
- The connector is `Clone + Send + Sync` (shareable between tasks)

### Relationship with existing code

The `armonik-transport` crate already has a functional `TlsConfig` and `Identity` (serde flat
options, eager loading of crypto material on config read). The design here follows the same
approach: configure what you want to obtain (paths, options), and the material is
loaded/validated immediately upon connector construction. The Rust types below are an evolution
of the existing code, not a from-scratch replacement.

### TransportConfig

```rust
pub struct TransportConfig {
    pub endpoint: Endpoint,         // target URI
    pub tls: Option<TlsConfig>,    // None = plain HTTP
    pub tcp: TcpConfig,            // keepalive, nodelay, etc.
    pub proxy: ProxyConfig,        // disabled | explicit | env | windows_system
    pub connect_timeout: Duration,
}

pub struct TlsConfig {
    pub ca: CaSource,              // System | PemFile(path) | WindowsStore | Insecure
    pub client_identity: Option<IdentitySource>,
    pub override_target_name: Option<String>,
}

/// Where the trust roots come from (config-time, serializable to JSON schema).
pub enum CaSource {
    System,                         // OS roots
    PemFile(PathBuf),              // explicit CA from file
    #[cfg(windows)]
    WindowsStore { subject_name: Option<String>, friendly_name: Option<String> },
    Insecure,                       // no verification (opt-in)
}

/// Where the client identity for mTLS comes from (config-time, serializable to JSON schema).
/// Same pattern as ProxySource: describes how to obtain the material, not the material itself.
pub enum IdentitySource {
    PemFiles { cert: PathBuf, key: PathBuf },
    Pkcs12 { path: PathBuf, password: Option<SecretString> },
    #[cfg(windows)]
    WindowsStore { subject_name: Option<String>, friendly_name: Option<String> },
}

/// Loaded material (runtime, after source resolution). Not serializable.
pub struct Identity {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl IdentitySource {
    /// Loads crypto material from the configured source.
    pub fn load(&self) -> Result<Identity, ConfigError>;
}

impl CaSource {
    /// Loads trust roots from the configured source. A PEM file may hold a
    /// chain or several roots, so this is a set, not one certificate.
    /// Empty for System: rustls uses its own roots.
    pub fn load(&self) -> Result<Vec<CertificateDer<'static>>, ConfigError>;
}

pub struct ProxyConfig {
    pub source: ProxySource,
}

pub enum ProxySource {
    /// No proxy, unconditionally: a direct connection whatever NO_PROXY says.
    /// Deserialized from "none" or "disabled". NO_PROXY is consulted by the
    /// Environment variant, which is where it belongs; making it apply here
    /// too would mean this variant sometimes yields to configuration, and
    /// "no proxy" would stop meaning one thing.
    None,
    /// Read proxy from environment (HTTP_PROXY, HTTPS_PROXY, NO_PROXY).
    Environment,
    /// Windows system proxy (WinHTTP resolver).
    #[cfg(windows)]
    WindowsSystem,
    /// Clean URI (without userinfo) + mandatory separate credentials.
    ExplicitWithCredentials { uri: CleanUri, username: String, password: SecretString },
    /// Raw URI (may contain credentials in the authority, or not).
    /// No-auth case: URI without userinfo. Inline-auth case: user:pass@host in the URI.
    ExplicitUri(Uri),
}

/// URI guaranteed without userinfo (no user:password@). Validated by construction.
pub struct CleanUri(Uri);

impl CleanUri {
    pub fn new(uri: Uri) -> Result<Self, ConfigError>;
}
```

### TransportError

```rust
pub enum TransportErrorKind {
    DnsResolution,
    TcpConnect,
    TlsHandshake,
    ProxyConnect,
    Timeout,
    Configuration,
}

pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,            // cause chain, without secrets
}
```

---

## Layer 2 — `armonik-grpc-channel`

### GrpcChannelConfig

```rust
pub struct GrpcChannelConfig {
    pub transport: TransportConfig,
    pub http2: Option<Http2Config>,
    pub retry: Option<RetryConfig>,
    pub default_deadline: Option<Duration>,
    pub pool: Option<PoolConfig>,
    pub user_agent: Option<String>,
    // No eager_connect flag: connecting is GrpcChannel::connect().await.
}

pub struct Http2Config {
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    /// The HTTP/2 SETTINGS value this endpoint advertises, which bounds the
    /// streams the *peer* may open (RFC 9113 s5.1.2) - for a client, server
    /// pushes. It is not a cap on outgoing calls; that one is
    /// max_calls_in_flight, and it lives in PoolConfig.
    pub advertised_max_concurrent_streams: Option<u32>,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_timeout: Duration,
}

pub struct RetryConfig {
    pub max_attempts: u32,          // total (initial + retries)
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
    pub retryable_status_codes: Vec<GrpcStatusCode>,
    pub max_buffer_size: usize,     // replay buffer size (bytes)
}

pub struct PoolConfig {
    pub idle_timeout: Duration,
    pub max_connections: Option<usize>,
    /// Local cap on calls this channel may have open at once, enforced by
    /// refusing start_call. Distinct from the HTTP/2 SETTINGS value above,
    /// which bounds what the peer may open.
    pub max_calls_in_flight: Option<u32>,
}
```

### GrpcChannel

```rust
pub struct GrpcChannel { /* ... */ }

impl GrpcChannel {
    /// Creation. Validates the configuration and performs no I/O, so its only
    /// failure is a bad configuration and it cannot block.
    pub fn new(
        config: GrpcChannelConfig,
        executor: impl Executor,
    ) -> Result<Self, ConfigError>;

    /// Establishes the connection and reports how it went. Optional: the first
    /// call connects lazily otherwise. This replaces the eager_connect flag,
    /// which asked a synchronous constructor to start asynchronous work and
    /// had nowhere to report its failure.
    pub async fn connect(&self) -> Result<(), TransportError>;

    /// Starts a gRPC call.
    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError>;

    /// Closes the channel: refuses new calls, cancels active calls.
    pub fn close(&self);
}
```

### GrpcCall

```rust
pub struct GrpcCall { /* ... */ }

// A call splits into a writer, a reader and a control. The halves take
// &mut self, so send order and read order are facts about the type rather
// than a rule in prose: two concurrent sends cannot be expressed, and
// neither can two readers. This is the Rust-side twin of the level-2
// SingleStreamConsumer obligation.
impl GrpcCall {
    /// Splits into the three. Each may be dropped independently.
    pub fn split(self) -> (SendHalf, RecvHalf, CallControl);
}

/// Cancels a call from anywhere. Cloneable and thread-safe on purpose:
/// cancelling is exactly what a third party does - a CancellationToken
/// registration, a deadline timer - and putting it on the halves instead
/// would strand it as soon as one of them is consumed.
#[derive(Clone)]
pub struct CallControl { /* ... */ }
// CallControl: Send + Sync

impl CallControl {
    /// Cancels the call (sends RST_STREAM). Idempotent.
    pub fn cancel(&self);
}

impl SendHalf {
    /// Sends a message (serialized protobuf bytes).
    /// In pure Rust usage: takes the buffer and returns when accepted by the framing layer.
    /// In FFI usage the bytes are already ours - they came from the call's arena - so
    /// nothing is borrowed and nothing is copied.
    /// At most max_sends_in_flight buffers out of one arena (a channel option, default 1);
    /// the next must wait for a WRITE_DONE to free a slot.
    pub async fn send_message(&mut self, msg: Bytes) -> Result<(), CallError>;

    /// Signals end of sending (END_STREAM on the request body). Takes self:
    /// there is no send after the end of sending, and &mut self would have
    /// left that as a rule to remember rather than one to obey.
    pub async fn end_send(self) -> Result<(), CallError>;
}

impl RecvHalf {
    /// Retrieves the initial metadata (HTTP/2 headers from the response).
    /// Blocks until reception or terminal. On a Trailers-Only response the
    /// server sends no headers, and this yields an empty Metadata rather
    /// than an error - see the normalization note in the ABI section.
    pub async fn recv_initial_metadata(&mut self) -> Result<Metadata, CallError>;

    /// Retrieves the next message or the terminal status.
    /// Each call implicitly constitutes a request for one message (natural backpressure).
    /// Returns End(GrpcStatus) when the stream is finished - this is the call's terminal.
    pub async fn next_message(&mut self) -> Result<RecvResult, CallError>;
}

/// Reception result: a message or the end of the stream (status + trailing metadata).
pub enum RecvResult {
    /// A received gRPC message.
    Message(OwnedMessage),
    /// End of stream - gRPC status + trailing metadata. Terminal, nothing after this.
    End(GrpcStatus),
}

/// Terminal status of a gRPC call.
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
    pub trailing_metadata: Metadata,
}

/// Received message. Owned - the caller frees when dropped. Bytes is a
/// refcounted view on the buffer the transport already owns, so the receive
/// path copies nothing; across the FFI the same allocation is what ak_bytes
/// points at, and ak_event_consumed is the explicit drop.
pub struct OwnedMessage {
    pub data: Bytes,
}
```

### CallStartOptions

```rust
pub struct CallStartOptions {
    pub method: String,             // e.g.: "/armonik.api.grpc.v1.Sessions/CreateSession"
    pub metadata: Metadata,         // request metadata -> HTTP/2 headers
    pub deadline: Option<Deadline>, // override of the channel default
    /// Reserved post-V1: override of the retry policy for this call.
    /// In V1, must be None - the channel default applies.
    pub reserved_retry: Option<RetryConfig>,
}

/// Absolute or relative deadline.
pub enum Deadline {
    Absolute(Instant),
    Timeout(Duration),
}
```

Note: the cardinality is not declared at start. It is implicit in the call usage (a unary does
send_message + end_send + next_message + status; a server streaming does send_message +
end_send + next_message in a loop). The channel does not need to know it to drive the HTTP/2
connection.

### Executor trait

```rust
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskHandle;
}

/// Handle to a spawned task. Allows cancellation.
pub struct TaskHandle { /* ... */ }
impl TaskHandle {
    pub fn cancel(&self);
}
```

### Consumption by the Rust ArmoniK client

The Rust ArmoniK client (`armonik::Client<T>`) uses `armonik-grpc-channel` via an adapter
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

**Open, and it is a boundary decision rather than a detail.** A `tower::Service` speaks HTTP
bodies; `GrpcChannel` speaks messages. An adapter between the two is not thin: it has to take
the request body apart into messages and put the response messages back together as a body,
including trailers, compression flags and the length-prefixed framing. That is a second
implementation of the gRPC wire format layered on the first. Two coherent answers exist and
V1 must pick one, because the middle is where the framing gets implemented twice:

- the Rust engine exposes an HTTP/2 service that Tonic consumes directly, and the
  message-level API is what the FFI is built on;
- or the ArmoniK Rust clients use message-level stubs generated against `GrpcChannel`, and
  Tonic is not in the picture at all.

The .NET binding is unaffected either way: it consumes the message-level API through the FFI.
Both paths share the same engine - retry, deadline, pool, flow control. T7.1 assumes the
first and has to say so explicitly.

### Retry — commitment point

A call is retryable while **both** hold: no response header has been seen, and what it has
sent still fits the replay buffer. Committing on either alone is wrong - the table below
reads as if one sufficed, which is why each row names both.

| Situation | Retryable? |
|-----------|------------|
| Unary: error before Response-Headers, request within budget | Yes |
| Client streaming: error before Response-Headers, data sent ≤ budget | Yes (replay) |
| Client streaming: data sent > budget | No (committed) |
| Bidi: error before Response-Headers, data ≤ budget | No response seen, so yes |
| Bidi: Response-Headers or a message received | No (committed) |
| Server streaming: error before Response-Headers | Yes |
| Remaining deadline < backoff | No |

**Under-specified for V1, and knowingly.** The table is the commitment point only. The
retry contract of gRFC A6 also carries: jitter on the backoff, `grpc-retry-pushback-ms`,
`grpc-previous-rpc-attempts` on each attempt, transparent retries for `REFUSED_STREAM` and
post-GOAWAY streams (which are not attempts and are not throttled), and a per-channel retry
throttle. A channel-wide policy is also the wrong granularity: gRPC configures retry per
method. None of that is specified here yet, and shipping the commitment point without it
would produce a client that retries at the wrong times and hides server pushback. It needs
a task of its own in phase 5, alongside T5.2.

---

## Layer 3 — `armonik-grpc-channel-ffi`

### Principles

- The FFI runtime owns a Tokio runtime and uses it as Executor for the GrpcChannel
- Every spawned task is registered in a task group (joinable at shutdown)
- Handles are `uint64_t` tokens validated in an internal registry, implemented
  as a slot map (index + generation, chained free list for O(1) allocation).
  A token from a reused slot fails validation instead of aliasing the new one
- Received message payloads are **owned**: the host receives an `ak_bytes` that it must
  release. This prepares for future zero-copy (the host will be able to deserialize directly
  from the native buffer before releasing).

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
and the level-2 refinement owes that order as a proof obligation
(`PayloadsReleasedInOrder`). In exchange the model asks strictly less of the host: one
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
is the guarantee: a callback runs on the runtime's own thread, so it cannot report that
the thread is gone. `ak_runtime_status` returning QUIESCENT is the guarantee, and from
there unloading the library, destroying the runtime and starting a new one are all safe.

Both STOPPED and QUIESCENT refine one level-0 state. Freeing a handle is not a gRPC
concept, so level 0 has nothing to say about the difference, and level 1 carries it
entirely in its own variables - the only shape the refinement rule allows.

Handles are not part of that count. Destroy invalidates every handle of the runtime at
once, and the generational tokens make a later use a refusal rather than a fault. The line
is between what the runtime owns and what the host might still be touching: a handle names
runtime state, a payload or a lent buffer is memory under the host's hands. Only the second
kind can hold destruction back - the alternative, refusing to destroy until every call
handle is released, turns a host bug that today leaks a slot into one that hangs at
teardown, and buys no safety in exchange.

**Failure.** On a runtime failure the binding makes one best-effort pass to reclaim what
it can and stops, and reports `AK_RUNTIME_FAILED_UNQUIESCED`. `ak_runtime_destroy` is
refused and unloading is forbidden, because nothing can promise the outstanding memory
is idle.

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

### Where each level-1 action happens

The table is the contract between the proof and the code. Every action of `FfiGrpc` has
exactly one linearization point; a change on either side that breaks a row breaks the
refinement.

| Level-1 action | Linearization point |
|----------------|---------------------|
| `RuntimeCreate` | `ak_runtime_create` publishes the runtime as RUNNING |
| `ChannelCreate` | `ak_channel_create` publishes the channel as open |
| `ChannelStartClosing` | `ak_channel_release`, or the runtime's shutdown closing the gate |
| `ChannelFinishClosing` | the last call of a closing channel reaches its terminal |
| `CallStart` | `ak_call_start` registers the actor and returns `AK_STATUS_OK` |
| `LendSendBuffer` | the bounded CAS on the slot counter succeeds, inside `ak_get_call_buffer`. Its three refusals - `AK_STATUS_SLOT_BUSY` for this call's window, `AK_STATUS_BUDGET_BUSY` for the runtime-wide ceiling, `AK_STATUS_MESSAGE_TOO_LARGE` for a request past it - are the model actions `RefuseLendForSlot`, `RefuseLendForBudget` and `RefuseLendTooLarge`, linearizing at the check that fails; each writes the call's last-lend status and nothing else |
| `HostReturnsBuffer` | `ak_return_call_buffer` gives a lent buffer back unused |
| `FreeReturnedBuffer` | the actor drops the allocation, once no unacquitted send lives in it. Not a downcall: giving a buffer back is the host's step, releasing its bytes is the runtime's |
| `SendMessage` | `ak_call_send_message` hands the filled buffer to the actor |
| `EndSend` | `ak_call_end_send`: the actor takes the END_STREAM command off its queue |
| `EmitWriteDone` | the actor invokes the callback with `AK_EVENT_WRITE_DONE` |
| `WriteDoneReturns` | that callback returns to the actor |
| `DeliverInitialMetadata` / `DeliverMessage` / `DeliverStatus` / `DeliverCancelled` | the actor invokes the data callback, having taken a credit |
| `DeliveryCallbackReturns` | that callback returns to the actor |
| `HostConsumesEvent(c)` | `ak_event_consumed` frees the oldest outstanding payload, identified by its `owner` |
| `RequestCallCancellation` | `ak_call_cancel`: the actor observes the flag, not the downcall's return |
| `ReleaseCallHandle` | the actor observes the last debt cleared on a terminal call: no payload owed, no buffer out, its own callbacks returned. Not a downcall |
| `RuntimeBeginShutdown` | `ak_runtime_begin_shutdown` closes the start gate |
| `EmitShutdownComplete` | the runtime task invokes the callback with `AK_EVENT_SHUTDOWN_COMPLETE` |
| `ShutdownCallbackReturns` | that callback returns |
| `RuntimeRelease` | the runtime publishes `AK_RUNTIME_GRPC_STOPPED`, or `AK_RUNTIME_QUIESCENT` when nothing of it is outstanding. Both refine the level-0 RELEASED state; which one the host reads is the release signal, not a level-0 distinction |
| `EmitResourcesReleased` | the runtime task invokes the callback with `AK_EVENT_RESOURCES_RELEASED`, owed only when `SHUTDOWN_COMPLETE` carried `AK_HOST_MUST_RETURN` |
| `ResourcesReleasedCallbackReturns` | that callback returns, which completes the resources branch. It does not by itself make the status `AK_RUNTIME_QUIESCENT`: the order against `RuntimeRelease` is free, so the level-0 transition may still be owed |
| `RuntimeDestroy` | `ak_runtime_destroy` accepts, its precondition checked |
| `NetworkSend` / `NetworkReceive` / `ReceiveStatus` | internal to `armonik-grpc-channel`, not observable at the ABI |
| `RuntimeFail` | any unrecoverable runtime fault, including a genuine allocator failure inside `ak_get_call_buffer` - but not reaching the configured ceiling, which is a refusal; the model leaves the state that follows unconstrained |
| `RemainFailed` / `RemainReleased` | explicit stutter, so a terminal runtime state has a step and the temporal proofs need no special case |

#### Which ABI argument becomes what

A function's arguments are as much of the contract as its name, and an argument the model
drops is a decision rather than an omission. This table is the record, and
`ci/check_abi_coverage.py` fails the build if an argument of an acting function has no row.

| ABI argument | in the model |
|---|---|
| every `ak_*_handle` | the identifier parameter: `rtId`, `chId`, `cId` |
| `ak_get_call_buffer`'s `*out` | `b` in `LendSendBuffer(cId, b)` - the allocation lent |
| `ak_call_send_message`'s `buffer` | `b` in `SendMessage(cId, msg, b)`. An argument, not a choice made inside the action: the host names the allocation it commits, and letting the model pick would make `buffer_send` a record of nondeterminism rather than of what the caller passed |
| `ak_return_call_buffer`'s `buffer` | `(cId, b)` in `HostReturnsBuffer(cId, b)` - a buffer determines its call, so the pair *is* the buffer |
| `ak_event_consumed`'s `payload` | **not modelled.** Release is FIFO by ABI rule, so the release count already says which payload is owed. That makes `ReleasesNeverExceedDeliveries` conservation of a count under a conformance assumption rather than a proof about identities - the one place the send side is now stronger than the receive side, and an open item rather than an oversight |
| `ak_get_call_buffer`'s `len` | The model takes the length directly: `LendSendBuffer(cId, b, len, charge)`, with `charge` the size the allocator returned. The lend sees only a length, exactly as the C function does; the message identity is born at the commit, where `SendMessage` requires `MessageLength[msg] = buffer_length` for the buffer it sends. `IsLendable(len)` is the request being in range, `IsMemoryAvailable(charge)` the ceiling admitting what backs it, and `CoversRequest(charge, len)` ties the two - including that an empty request charges nothing. Level 0 carries no sizes: its send window counts allocations |
| `config`, `config_json`, `options` | **not modelled.** Configuration reaches the model as the constants `MaxSendsInFlight`, `DeliveryCredits`, `Ceiling` and `MessageLength`; the rest does not change what the ABI guarantees |
| `callback`, `runtime_ctx`, `call_ctx` | **not modelled at level 1.** They are identity plumbing, and what must hold of them is level 2: `TokenPublishedBeforeStart` and `RootSurvivesCallbacks` |
| every other `*out` | **not modelled.** A returned handle is the identifier the action already quantifies over |

Two rows carry the ownership argument, and they are asymmetric. `EmitWriteDone` is the
only producer of WRITE_DONE and it is per-actor, so a send really is acquitted exactly
once and `WriteDonesNeverExceedSends` proves it. `HostConsumesEvent` is the other half,
and there the proof is weaker than the sentence one would like to write: the model counts
releases, it does not track which `owner` a release names, so `ReleasesNeverExceedDeliveries`
proves no over-consumption *given* that the host releases the right owner, once, in
order. That assumption sits with the fairness hypotheses, and the level-2 obligations are
where the binding pays it. A defensive ABI that rejected a duplicate token would need the
identities in the model; this design chooses assume-guarantee instead.

`ReleaseCallHandle` linearizes where the actor observes the last debt clear, not at a
downcall - the ABI has none for it. What it establishes - `ReleasedCallIsClean` - is
proved to survive every later step, since nothing can lend or deliver on a retired call.
That is the formal content of "at the end of a call, whatever the ending, everything is
back".

### JSON configuration schema

The JSON schema is generated from `GrpcChannelConfig` + `TransportConfig`. `CallStartOptions`
is deliberately outside it: it carries a `Deadline`, whose `Absolute(Instant)` variant is a
process-local monotonic point with no portable serialization and no meaning in another
address space. Per-call options cross the ABI as fields, not as JSON, and a serialized
deadline - in a retry policy for instance - is always a relative `Duration`.
The schema is the source of truth for:
- C# options (generated from the schema)
- Options documentation
- Rust-side validation at channel creation and at the start of each call

Note: `RetryConfig` appears both in `GrpcChannelConfig` (channel default) and, post-V1, as a
per-call override. Only the type is shared with the schema; the per-call override travels as
an ABI field like the rest of `CallStartOptions`.

The schema is committed at `packages/rust/armonik-grpc-channel-ffi/include/channel_config.schema.json`.

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
// generational tokens are for. The distinction is deliberate: a handle names
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
                            ak_bytes_in config_json,
                            ak_channel_handle *out);

// Frees the channel. In-progress calls are cancelled (CANCELLED).
// The handle is no longer valid after this call.
void ak_channel_release(ak_channel_handle channel);

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
// When the allocation is freed is Rust's business and is not observable here: a
// call still within its replay buffer keeps the bytes so it can send them again,
// and frees them when it commits. WRITE_DONE therefore says the slot is free,
// nothing about the memory.
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
// distinguish the two reads it off the terminal: Trailers-Only carries its
// status in the trailing metadata with no headers preceding it.
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
// the generational token makes safe rather than faulting. And abandoning a call
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
// that one read returns one coherent instant. See "What is actually verified".
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
// default 1) - while the host owes that many, the runtime withholds the next data
// callback; only a terminal may still go out with every credit spent.
// The payload pointer identifies the allocation to free; the host MUST release
// in delivery order, so with several credits the oldest outstanding payload is
// always the next one to be consumed.
// The terminal does not invalidate payloads already handed over: calling
// ak_event_consumed remains legal after the terminal, and is in fact required
// before the call can be reclaimed.  After a runtime failure the binding cleans
// up on a best-effort basis: no promise that reads a runtime state survives, but
// this call does - ak_event_consumed stays legal and BufferEventuallyFreed still
// holds, because a failed runtime disables neither returning memory nor freeing it.
void ak_event_consumed(ak_bytes payload);
```

### Detailed ABI surface

```c
// === Handles ===
// Handles are tokens, not pointers. Each is a slot index plus a generation
// counter, so a handle from a freed slot is detected and refused with
// AK_STATUS_HANDLE_STALE rather than dereferenced - which is what lets a downcall on a
// reclaimed call report a status instead of faulting, and what closes ABA when
// a slot is reused. Their layout is opaque and must not be interpreted; only
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
    AK_EVENT_INITIAL_METADATA   = 1,  // payload = metadata blob (owned)
    AK_EVENT_MESSAGE            = 2,  // payload = message bytes (owned)
    AK_EVENT_STATUS             = 3,  // terminal - payload = status + trailing metadata (owned)
    AK_EVENT_WRITE_DONE         = 4,  // the accepted send is settled; its slot is already free
    AK_EVENT_SHUTDOWN_COMPLETE  = 5,  // the runtime has stopped running
    AK_EVENT_RESOURCES_RELEASED = 6,  // and now nothing of it is outstanding
} ak_event_kind;

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
    int32_t          status_code;  // grpc status (AK_EVENT_STATUS only)
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
  See Zero-copy below.
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
the writer it wakes can act on it straight away. Retry costs no copy at all: the
bytes are already Rust's, so a retryable call simply keeps the allocation until it commits.
The slot budget and the replay buffer (`RetryConfig::max_buffer_size`) are two independent
budgets - the first bounds how many buffers are outstanding, the second how many bytes are
retained for replay - and neither has to be traded against the other.

**Receive (Rust → host)**:
- `ak_event.payload` is an owned `ak_bytes` (reference-counted Rust buffer)
- The host can deserialize directly from `payload.ptr` via `Span<byte>` or unsafe
- The host calls `ak_event_consumed` when done — Rust deallocs + arms the next one
- No copy if the host consumes the native pointer directly

**Memory fragmentation**:
- Send side: every send buffer comes out of the call's arena, bounded by
  `MaxSendsInFlight` buffers at a time, and the arena is dropped in one piece when the call
  is released - which the release precondition guarantees is safe. Retained replay bytes are
  the same allocations, held past their WRITE_DONE and bounded separately by
  `max_buffer_size`. Arenas are a natural fit for a pool held by the channel, so the same
  memory serves every call the channel carries and the steady state costs no allocation.
- Receive side: Rust buffers are allocated by Hyper (similar size classes,
  well-managed by jemalloc/system allocator). If fragmentation is measured in production,
  a pool of pre-allocated buffers can be added without ABI change.
- Neither pool is visible across the ABI, so either can be added or removed later without
  touching a binding.

**A global ceiling above the per-call budgets.** `max_buffer_size` bounds one call, and
`MaxSendsInFlight` bounds one call's outstanding buffers; nothing bounded their product across
the calls a process carries. The runtime therefore holds a byte budget shared by every channel,
handed out by `ak_get_call_buffer`.

**Reaching the ceiling and failing to allocate are two different events, and only the
second is a fault.** The ceiling is a configured accounting limit. Reaching it means some
live allocation - possibly from this call - is holding bytes right now, and the runtime
knows exactly what recredits that capacity: `FreeReturnedBuffer`. Waiting demonstrably
helps. A real allocator failure is the
other case, and there waiting has no evidence behind it - a process that cannot allocate a
send buffer has no reason to believe it can allocate a retry path or the string a log line
needs.

So `ak_get_call_buffer` has four modeled lend outcomes - `OK`, `SLOT_BUSY`,
`BUDGET_BUSY`, `MESSAGE_TOO_LARGE`; the remaining ABI results (`HANDLE_STALE`,
`INVALID_STATE`, `INVALID_ARG`, and `INTERNAL` for a fault the ABI cannot attribute)
are the ABI matrix's rows, outside the backpressure sub-machine level 1 formalizes.
It lends, with `MESSAGE_TOO_LARGE` refused permanently when `len` exceeds the ceiling
itself; or it refuses with
`AK_STATUS_SLOT_BUSY` because this call's window is full, whose wake-up is WRITE_DONE; or it
refuses with `AK_STATUS_BUDGET_BUSY` because the runtime-wide ceiling is reached, which is
not necessarily this call's doing - with a window deeper than one or replay bytes
retained, its own sends hold budget too - so a WRITE_DONE of this call is a wake-up,
never an exhaustive one: the budget is runtime-wide and anyone's free recredits it.
Only a genuine allocator failure is `RuntimeFail` with `AK_STATUS_INTERNAL`.

The order of those checks is what keeps `AK_STATUS_INTERNAL` rare. The ceiling is tested
*before* anything is allocated, so a runtime at its budget refuses with `AK_STATUS_BUDGET_BUSY`
and never reaches an allocation that could fail. Past that check, every allocation on this path
uses the fallible form - `try_reserve`, `try_with_capacity` - so a failure returns rather than
aborting, and that return is what `AK_STATUS_INTERNAL` reports. The infallible `Vec` and `Bytes`
APIs are what abort; the emission path does not use them.

**The budget covers the emission path and only it.** What it governs is the memory this
runtime allocates against a quota of its own and can therefore refuse: the buffers
`ak_get_call_buffer` lends. Receive-side memory is not in it. Those bytes belong to hyper and
are governed by the HTTP/2 flow-control window rather than by anything the ABI exposes, and
there is no refusal to expose: a genuine allocation failure in Rust runs the allocation error
hook and aborts, so `Vec` and `Bytes` offer no `Result` to turn into a status. A fallible
receive path would be a different design - reserving from a bounded pool and resetting the
stream with `RESOURCE_EXHAUSTED` when the reservation fails - and it is not this one. This is
why the model's budget is welded to the send-buffer lifecycle alone, and why
`BudgetEventuallyHasRoomFor` is a statement about lending rather than about all memory.

**What a buffer charges against the budget** is the size the allocator returned, not the size
the host asked for: `charge(b)` is what actually backs `b`, `len` is the request it must
cover, and `bytes_used` is the sum of `charge(b)` over every buffer lent and not yet freed.
The two refusals are then

```
AK_STATUS_MESSAGE_TOO_LARGE  <=>  len > ceiling
AK_STATUS_BUDGET_BUSY        <=>  len <= ceiling  and  bytes_used + charge > ceiling
```

Charging the request instead is the obvious alternative, and it fails at the one thing the
ceiling exists for. A budget that counts what was asked for bounds an accounting fiction;
the memory that has to fit is what the allocator handed out. Size-class rounding and per-arena
slack would sit outside the ceiling, and the ceiling would be wrong by however much they come
to - silently, and in the direction that matters.

What that costs is the predictability of one refusal, and only one. `MESSAGE_TOO_LARGE` stays
a predicate the host can evaluate *before* it calls, because it reads `len` and the ceiling
and nothing else - which is also why its permanence is **derived** in the model rather than
asserted: `IsLendable(len)` mentions no charge, so no sequence of frees can turn the answer
around. A host holding a 4 MiB message against a 4 MiB ceiling still knows which answer it
will get. `BUDGET_BUSY` is not predictable from the host's own numbers, and does not need to
be: it is transient, it has a wake-up, and retrying on it is the correct response.

The residue is small but it is not zero: **the ceiling bounds
the bytes the allocator reported, not the process's resident memory.** Allocator metadata, the
arena's own structures and fragmentation between arenas sit outside it. No bound relating the
two is asserted here: it is not known that the relation even has the shape of a factor plus a
constant, and writing one down before there is data to fit it would be a number with no
standing. Configuring the ceiling as though it were an RSS limit is the concrete mistake this
paragraph exists to prevent, and stating that much does not require the bound.

Two of the refusals are transient and each needs its own wake-up. `AK_STATUS_SLOT_BUSY`
has WRITE_DONE. `AK_STATUS_BUDGET_BUSY` has none of its own - a call refused for the
ceiling has no send in flight, so nothing of that call can wake it - and that is why
`ak_runtime_memory_usage` exists: the host polls the runtime's accounting instead. The third
refusal, `AK_STATUS_MESSAGE_TOO_LARGE`, needs no wake-up because waiting cannot help, and
`AK_STATUS_INVALID_STATE` needs none either: it reports a guard, not a shortage.

The budget wake-up is a poll and not a signal on purpose, for now. A signal would have to
fire at the moment the native budget is genuinely recredited rather than when the host hands
something back - committing a buffer moves bytes from the host to the runtime without
freeing any - and a signal on the wrong edge is a wake-up that never comes. A
signal or an epoch can be added later without changing the contract, since the poll remains
correct in its presence. What the poll does *not* give on its own is freedom from
starvation: another call can win the capacity between the observation and the retry. Level 2
states the honest contract there: the wait is cancellable and nothing more - cancellation
and dispose end it, the successful lend ends it, and no repetition of attempts, cadence or
backoff is promised, because a promise of attempts would prove nothing (a refused retry
changes nothing observable) while acquisition would need the arbitration discussed above.
The cadence and the backoff are implementation choices, verified by review and tests.

A fatal ceiling - the rejected alternative - would be wrong, not merely pessimistic:
`AK_RUNTIME_FAILED_UNQUIESCED` is absorbing and `ak_runtime_destroy` is refused from it
forever, so a normal burst would leave the runtime permanently undestroyable - a
mechanism introduced to bound memory turning the runtime itself unreclaimable.

An implementation that refuses more often than the specification permits produces a subset
of the modelled behaviours: no proved liveness rests on the lend being taken. The
alternative shapes - reserving the budget at call admission and refusing `ak_call_start`, or a
runtime permit acquired before the lend and released on the real recredit - remain open and
are the level-2 material for turning a non-blocking refusal into a fair asynchronous wait.
See T8.1.

---

## Layer 4 — `ArmoniK.Api.Client.RustGrpcChannel`

### Internal architecture

The disposable object is the channel; the invoker is a view over it.

```csharp
await using var channel = new NativeGrpcChannel(options);
CallInvoker invoker = channel.CreateCallInvoker();
```

```text
+------------------------------------------+
|  NativeGrpcChannel : ChannelBase         |
|    +-- NativeChannel (SafeHandle)        |
|    +-- RuntimeLease -> RuntimeState      |
|    +-- Trampoline (static, unmanaged)    |
|    +-- CallState (per call)              |
|          +-- delivery ring + signals     |
+------------------------------------------+
        |
        +-- CreateCallInvoker() -> NativeCallInvoker (a view, no state)

No queue and no thread between the two: the trampoline publishes into the call's own
ring and the consumer reads it directly.  The RuntimeState is the process's, never a
channel's: what the channel holds is a lease.
```

**Lifecycle: the lease belongs to the channel.** A `CallInvoker` is a stateless view -
`CreateCallInvoker()` hands one out and owns nothing - so the unit of borrowing is the
channel, the object an application creates and disposes. The first channel materializes
the process runtime (the factory allocates the shared `RuntimeState` root, calls
`ak_runtime_create`, then creates the channel - all before the constructor returns);
every later channel takes a lease on the materialized runtime and creates only its own
`ak_channel`. `DisposeAsync` on a channel settles its own calls - and no one else's,
ownership being `call_channel`, the level-0 relation - releases its `ak_channel`, then
its lease; the last lease released is what starts the native shutdown, and only then.
The runtime is reusable: after a full teardown, the next channel materializes a fresh
generation. The lease count is the implementation's refcount; the model derives "last"
from the set of channels not yet settled rather than from a counter.

**Dispose is asynchronous, and its task means something.** `NativeGrpcChannel` is
`IAsyncDisposable`: the task returned by `DisposeAsync` completes once this channel's
calls are settled and its handle released - and, when it held the last lease, once
`ak_runtime_destroy` has returned. A synchronous `Dispose` would have to block on the
network and on host callbacks, which is why the surface does not offer one. The call
wrappers keep gRPC's own shape and expose `Dispose`; the binding's extension is the
channel's asynchronous one, and `await using` is the documented pattern.

### Trampoline

```csharp
// Rooted for the runtime's lifetime. Runs on a Tokio thread: it publishes
// one ring slot and returns. No user code, no allocation, and no path that
// can throw - which is what makes it total.
[UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
private static unsafe void OnEvent(void* runtimeCtx, void* callCtx, ak_event* evt)
{
    // Both runtime events carry runtime_ctx and no call_ctx. Neither frees the
    // root: every callback of every kind carries runtime_ctx, so the root is
    // released after ak_runtime_destroy returns, where nothing can be in
    // flight. Freeing it here on the shutdown event would be a use-after-free
    // whenever host_debt said the host must return something.
    if (evt->kind == AK_EVENT_SHUTDOWN_COMPLETE)
    {
        var rt = RuntimeState.From(runtimeCtx);
        rt.ShutdownTcs.TrySetResult(evt->host_debt);   // Dispose needs the tag
        return;
    }

    if (evt->kind == AK_EVENT_RESOURCES_RELEASED)
    {
        var rt = RuntimeState.From(runtimeCtx);
        rt.ResourcesReleasedSignal.Set();
        return;
    }

    var s = CallState.From(callCtx);

    if (evt->kind == AK_EVENT_WRITE_DONE)
    {
        // No payload, and the ABI says it may arrive in parallel with a data
        // callback for the same call: it must not queue behind one.  This is
        // where the pending write completes: TrySetResult only signals, the
        // continuation runs elsewhere.  One writer per call, and a task
        // created only after the previous one completed, make this WRITE_DONE
        // unambiguously that write's.
        Interlocked.Exchange(ref s.WriteTcs, null)?.TrySetResult();
        return;
    }

    // Metadata, message, terminal: every event that carries a payload takes
    // the next slot. The slot index is the delivery order, so the consumer
    // releases in that order with nothing to arrange.
    ref var slot = ref s.Ring[(int)(s.Head & s.Mask)];
    slot.Payload = evt->payload;
    slot.Kind    = evt->kind;
    slot.Status  = evt->status_code;
    Volatile.Write(ref s.Head, s.Head + 1);   // publishes the slot with it
    s.RingSignal.Set();

    // Last callback of the call, and this is its last access: the native
    // root goes here. Managed references keep the object alive, so this
    // collects nothing - it just stops the ABI from resolving a call_ctx
    // that no longer names anything.
    if (evt->kind == AK_EVENT_STATUS)
        s.SelfHandle.Free();
}
```

The trampoline is the level-1 callback boundary: it runs on a native thread, and its
return is what the model calls `DeliveryCallbackReturns` (or `WriteDoneReturns`). Keeping
it allocation-free and lock-free is not an optimization but the reason the native actor
can promise to make progress without the host: the proved liveness assumes the callback
returns, and nothing else.

Treating metadata as an ordinary payload is what makes that literal. Decoding it here
would allocate, and it would need a `finally` to avoid leaking the payload on a malformed
header blob - a failure path across the FFI boundary, which is the worst place to have
one. Publishing a slot cannot fail, so the trampoline has no exception path at all and
the whole leak-on-throw class disappears rather than being handled.

The trampoline never copies a payload. It publishes the owned `ak_bytes` and the consumer
releases it after parsing, on a managed thread.

No registry: the `call_ctx` is directly a `GCHandle` to the call's `CallState`, allocated
before `ak_call_start`.

**Who frees that root, and when, is not the consumer's business.** Reclamation is the
runtime's own step and the host is not told when it happens, so tying the root's lifetime
to it is not even an option. Two rules remove the question instead:

- every callback resolves `call_ctx` into a strong local reference before touching
  anything, so the object stays reachable for the whole callback regardless of the root;
- the terminal callback frees `SelfHandle` itself, after its last access to `CallState`.
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
    // NextPow2(DeliveryCredits + 2): the ABI never leaves more than
    // DeliveryCredits + 1 payloads outstanding, plus one slot so full and
    // empty stay distinguishable. It therefore cannot fill.
    Slot[] Ring; int Mask;
    long Head;                     // published by the actor thread
    long Tail;                     // private to the consumer
    IAsyncSignal RingSignal;       // latched auto-reset, never SemaphoreSlim

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
type this binding uses for every cancelled path - see the frontier section, where that
decision is stated.

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
disposed call leaves no managed waiter - reader, writer, headers, status - and level 2
states it (`DisposeLeavesNoManagedWaiter`).

WRITE_DONE must never queue behind a slow message handler - the ABI states it may arrive
in parallel with data callbacks for the same call - so it stays out of the delivery ring
and frees its slot on the spot. That is safe because it runs no user code: a counter
increment and a latched signal.

### No dispatcher: the ring is the queue

There is no dispatcher thread and no process-wide host queue. The trampoline publishes
into the call's own ring and the consumer reads it directly, so an event crosses one
buffer instead of two and nothing is allocated to describe it.

What used to justify the dedicated thread does not survive inspection: the release
(`ak_event_consumed`) happens in the application's parse, on the pool, whichever way the
event was routed. A dispatcher never protected `PayloadsEventuallyConsumed` - that
hypothesis rests on the application in both designs. What it did protect was routing
under pool pressure, and that is bought more cheaply by keeping every wakeup off the
Tokio thread.

**Nothing may run user code on the callback's thread.** With no dispatcher standing
between the trampoline and the application, this is the *only* thing that keeps the
native actor free to make progress, so it is a rule and not a preference:

- every `TaskCompletionSource` is built with `RunContinuationsAsynchronously`;
- `RingSignal` is a latched auto-reset signal whose `Set` never runs a
  waiter inline - `ManualResetValueTaskSourceCore<bool>` with
  `RunContinuationsAsynchronously = true`, or a `TaskCompletionSource`-based
  `AsyncAutoResetEvent` where that type is unavailable;
- **never `SemaphoreSlim`.** Its `Release` can complete a `WaitAsync` waiter inline
  depending on the runtime version, which would put application code on the Tokio thread
  through the back door the two flags close at the front.

The signal only says "something may have changed"; the truth is `Head != Tail`, so merged
wakeups cost nothing and the consumer drains what is there before waiting again. Latched
is what matters: a `Set` with no waiter must be remembered, or the window between finding
the ring empty and arming the wait loses the event.

```csharp
private bool TryPeek(out Slot slot)
{
    if (Volatile.Read(ref _head) == _tail) { slot = default; return false; }
    slot = _ring[(int)(_tail & _mask)];   // borrowed: the payload is still the runtime's
    return true;                          // _tail does NOT move here
}

// The tail advances with the release, never before it: _tail is exactly
// the runtime's consumed count, which is what lets the model read the
// ring's tail straight off payloads_consumed_by_host.  Moving it at the
// copy would put the managed index one ahead for the whole parse and
// make that mapping false.
private async ValueTask<T> ReadNextAsync<T>(Func<Slot, T> parse,
                                            CancellationToken ct)
{
    while (!TryPeek(out var slot))
        await _ringSignal.WaitAsync(ct);
    try     { return parse(slot); }        // decode under the borrow
    finally { ak_event_consumed(slot.Payload.owner); _tail++; }
}
```

**Slot 0 is always the metadata**, and level 0 proves it: `EventStreamShape` says the
first delivered event of a used call is `INITIAL_METADATA`, and the ABI never skips it,
not even on a call cancelled before the response head. So the host needs no inspection to
know what it is taking. It is consumed by whoever asks first - awaiting the headers, or
reading the first message - through an idempotent `EnsureHeadersAsync`, which is also
what stops the two from overlapping. `ResponseHeadersAsync` therefore completes on
arrival rather than on the application deciding to read, which is what gRPC promises.

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
  `Dispose` does not free `SelfHandle` either - the terminal callback already did, on the
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
Rust types (TransportConfig, GrpcChannelConfig)
    │ derive(schemars::JsonSchema) on the *Source types (serializable)
    ▼
channel_config.schema.json  <- committed, source of truth
    │ generation tool (NJsonSchema, or custom)
    ▼
RustChannelOptions.g.cs     <- generated, C# types to configure the channel
    │ .ToJson()
    ▼
UTF-8 JSON passed to ak_channel_create
    │ serde::Deserialize on Rust side
    ▼
GrpcChannelConfig (Rust types, with *Source)
    │ .load() / .resolve()
    ▼
Effective material (Identity, CA certs, proxy route, etc.)
```

The `*Source` types (IdentitySource, CaSource, ProxySource) are serializable (serde +
schemars). The loaded material (Identity, CertificateDer) is not. The JSON schema is
generated from the source types, which guarantees consistency between Rust options and C#
options without manual maintenance.

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
var options = new RustChannelOptions { Endpoint = "https://armonik:5001" };
await using var channel = new NativeGrpcChannel(options);
var client = new Sessions.SessionsClient(channel.CreateCallInvoker());
```

### Existing options mapping

The current client options (`GrpcChannel` in ArmoniK.Api.Common.Options) must be able to
produce a `RustChannelOptions`. The mapping is explicit and tested:

| Existing option | RustChannelOptions field |
|-----------------|--------------------------|
| `Address` | `Endpoint` |
| `CaCert` | `Tls.CaCertPath` |
| `ClientCert` / `ClientKey` | `Tls.ClientIdentity` (PEM) |
| `ClientP12` | `Tls.ClientIdentity` (PKCS12) |
| `AllowUnsafeConnection` | `Tls.CaSource = Insecure` |
| `OverrideTargetName` | `Tls.OverrideTargetName` |
| `Proxy` | `Proxy.Source` |
| `ProxyUsername` / `ProxyPassword` | `Proxy.Credentials` |
| `RequestTimeout` | `DefaultDeadline` |
| `MaxAttempts` | `Retry.MaxAttempts` |
| `InitialBackOff` etc. | `Retry.*` |

---

## TLA+ Formal Model

### Structure and location

TLA+ files live in `spec/armonik_grpc_ffi/tla/`. The model is structured in three
refinement levels:

```text
Level 0 - Abstract spec (what the user observes)
    AbstractGrpc.tla

Level 1 - FFI spec (what happens at the C boundary)
    FfiGrpc.tla  refines  AbstractGrpc

Level 2 - .NET binding spec (what happens on the managed side)
    DotNetBinding.tla  refines  FfiGrpc
```

### Level 0 — AbstractGrpc

State variables:
- `runtime_state`: NOT_INIT | RUNNING | STOPPING | RELEASED | FAILED_UNQUIESCED.
  `RELEASED` is the abstract "this runtime is finished" and the ABI's
  `AK_RUNTIME_QUIESCENT` is what refines it. Destroying the runtime has no level-0
  counterpart at all - freeing a handle is not a gRPC concept - so it appears only at
  level 1, exactly like `ReleaseCallHandle`
- `channels`: set of channels (open | closed)
- `calls`: set of calls with their state
- `send_closed`: boolean per call (end_send called)
- Per call, 4 message sequences:
  - `submitted`: messages submitted by the client to the library (via send_message)
  - `sent`: messages actually sent over the network (HTTP/2)
  - `received`: messages received from the network (HTTP/2)
  - `delivered`: messages delivered to the client by the library (via callback/next_message)
- `events_delivered`: ordered sequence of events per call (INITIAL_METADATA, MESSAGE*, STATUS)

#### Safety invariants (to be proved by TLAPS)

Every name below is a conjunct of `SafetyCore` in `AbstractGrpc.tla`, and
`ci/check_property_manifest.py` fails the build if this list and that conjunction
diverge in either direction. Safety is required only while no runtime has entered the
deliberately unconstrained failed state: `SafetyInvariant == NotFailed => SafetyCore`.

**Event sequencing per call:**
- **MetadataFirst**: the first event of `events_delivered` is INITIAL_METADATA
- **NoEventAfterStatus**: a status event is the last event, which also makes it unique -
  two would each have to be last
- **TerminalStatusEquivalence**: a call is terminal exactly when it carries a status
  event, so "the call is over" and "the host has been told" are one fact and not two
- **EventStreamShape**: the whole event grammar in one predicate: the first event is
  INITIAL_METADATA, every later one is MESSAGE or a status kind, and every interior one
  is MESSAGE - a status can only sit last. The two bullets above are its ends, kept as
  their own conjuncts; the shape is what the event counting builds on
- **MessageEventsMatchDelivered**: one MESSAGE callback per delivered message, stated as
  a count: the events of a used call number one metadata, plus one per delivered
  message, plus one status once it has arrived. With the shape this pins the event
  stream to the delivery sequence - no MESSAGE event without its message, none missing,
  none doubled

**Message integrity (prefix invariants, liveness of equality):**
- **SubmittedPrefixOfSent**: `sent` is a prefix of `submitted` at all times
- **ReceivedPrefixOfDelivered**: `delivered` is a prefix of `received` at all times
- **CompleteDelivery**: a call that reached its terminal without being cancelled
  delivered everything it received. This is safety, not liveness: it constrains the
  terminal step rather than promising one
- No property equates `sent` with `submitted`, even on success: a server may answer
  without reading everything, and a cancellation abandons accepted sends. The only
  safety relation between the two is `SubmittedPrefixOfSent`, and the liveness one is
  positional with termination as an escape. What is owed for an abandoned send is its
  WRITE_DONE, not its transmission

**Send-side sequencing:**
- **SendAfterEndSend**: once `end_send` is called the call is half-closed or terminal,
  and `SendMessage` is guarded on neither, so no send can follow

**Ownership - everything belongs to a runtime:**
- **SingleRuntime**: at most one runtime with state ∈ {RUNNING, STOPPING,
  FAILED_UNQUIESCED} at all times. This is a modelling restriction, not an ABI rule:
  nothing in the C surface forbids two runtimes. The binding keeps a single shared
  runtime for the process, which every `NativeGrpcChannel` leases rather than owning -
  the diagram above shows the invoker's view, not a per-invoker runtime. It bounds the
  state space and lets the shutdown chain be stated per runtime without quantifying over
  interleavings; a second runtime would need it lifted and the shutdown proofs redone
- **ChannelOwnership**: a created channel names a runtime
- **CallOwnership**: a started call names a created channel, and therefore a runtime

**Channel ↔ runtime link:**
- **ActiveChannelImpliesActiveRuntime**: an open or closing channel's runtime is
  RUNNING, STOPPING or FAILED_UNQUIESCED - never RELEASED
- **StoppingClosesChannels**: runtime STOPPING ⇒ each of its channels is closing or closed
- **ReleasedNoChannels**: runtime RELEASED ⇒ each of its channels is closed

**Call ↔ channel ↔ runtime link:**
- **ActiveCallImpliesActiveChannel**: an active call sits on an open or closing channel
- **ClosedChannelNoCalls**: a closed channel has only terminal calls
- **ReleasedNoCalls**: runtime RELEASED ⇒ every call of every channel is terminal. It
  says nothing about a callback still unwinding on the host stack; that is
  `ShutdownSignalInv` at level 1, and it is deliberately a separate claim

**Enforced by the action guards, not stated as invariants.** These are true of every
behaviour of the model, but by construction rather than by an inductive proof, so they
carry no theorem and this document must not imply one:
- monotone runtime transitions NOT_INIT → RUNNING → STOPPING → RELEASED, with
  STOPPING → FAILED_UNQUIESCED as the only branch: each action's guard names the state
  it leaves, and `RemainReleased` and `RemainFailed` are the only steps enabled from the
  two terminal states
- a channel is created only from a RUNNING runtime, and a call started only on an open
  channel: `ChannelCreate` and `CallStart` are guarded on exactly that
- no channel or call exists outside a runtime: the sentinel equivalences
  (`ChannelSentinelEquivalence`, `CallSentinelEquivalence`) make "unused" and "no owner"
  the same state, and they are conjuncts of the inductive invariant rather than of the
  safety contract

#### Liveness (conditional on fairness)

The four `~>` guarantees and `EventualMetadata` are the conjuncts of
`LivenessProperties` in `AbstractGrpc.tla`; the same checker binds this list to it.
Every antecedent embeds a trigger the model leaves unfair - `CallStart`, `SendMessage`,
`RuntimeBeginShutdown` - so the library promises what follows a trigger, never that one
occurs. Each is discharged only while no runtime has failed, with `~NotFailed` as the
escape.

- **EventualMetadata**: a started call eventually gets its response head
  (under: scheduler fairness, network progresses)
- **EventualTerminal**: a started call eventually reaches STATUS. Applied to a stopping
  runtime this is what drains it: closing latches cancellation on the channel's active
  calls, so the drain asks the host for no ownership return - though it does need the
  callbacks already dispatched to return
  (under: scheduler fairness, network progresses, client and server each produce a
  finite number of messages)
- **EventualShutdown**: runtime STOPPING ⇒ ◇ RELEASED
  (under: callbacks return, peer responds or timeout. Not under anything the host
  consumes: shutdown deliberately does not wait for payloads to be released)
- **SubmitProgress**: for every position i ≤ Len(submitted), eventually
  `sent[i] = submitted[i]` or the call terminated
- **DeliveryProgress**: for every position i ≤ Len(received), eventually
  `delivered[i] = received[i]` or the call terminated. This is the guarantee a received
  message eventually reaches the host, under WF on the delivery actions and on
  `ak_event_consumed`

Progress is stated per position, not per value: `SubmitProgressAt(c, i)` says the i-th
submitted message reaches the wire at position i with `sent[c][i] = submitted[c][i]`, and
`DeliveryProgressAt(c, i)` is its mirror. Two identical messages are therefore distinct
obligations, which membership in a sequence could not express.

### Level 1 — FfiGrpc

The state is the level-0 state (shared through `AbstractGrpcState`, never redeclared),
two FFI constants — `MaxSendsInFlight` and `DeliveryCredits`, the pipelining depths of
the ABI contract, each assumed a positive natural, plus the buffer identity space — and
nineteen FFI variables:

- `buffers_held_by_host`: per call, the number of buffers `ak_get_call_buffer` has lent
  and that have not come back. Committing one moves it out of this count and into
  `submitted`, so `SendWindowOccupancy(c) == buffers_held_by_host[c] + Len(submitted[c])
  - write_dones_emitted[c]` is what `MaxSendsInFlight` bounds: the memory the call's arena
  holds for the send path, whichever side is looking at it. It counts against the
  *emitted* WRITE_DONEs, not the returned ones, so the slot goes back when the event goes
  out. `WriteDonesReturned(c) == write_dones_emitted[c] - (1 if the acquittal callback is
  running)` is the other count, and it says which sends are acquitted - the terminal and
  quiescence read it, the window does not
- `write_dones_emitted`: per call, the monotone count of WRITE_DONEs emitted. A send is
  identified by the index of its message in the level-0 `submitted` sequence; WRITE_DONE
  acquits in send order, so this one counter says exactly which sends are acquitted
- `write_done_callback_running`: per call, a WRITE_DONE callback is on the host stack
- `delivery_callback_running`: per call, a delivery callback is on the host stack
- `payloads_consumed_by_host`: per call, the monotone count of payloads released. A
  payload is identified by the index of its event in `events_delivered`; release follows
  delivery order, so this one counter says exactly which payloads the host still owes -
  the mirror of the send side, and what the host owes is the gap to `events_delivered`
- `handle_released`: per call, the runtime has reclaimed the call - the handle is stale and
  the arena may go. Set by `ReleaseCallHandle`, which is the actor's own step and not a
  downcall: the ABI has no `ak_call_release`
- `cancel_requested`: per call, cancellation latched - by `ak_call_cancel` or by a
  channel or runtime closing. Reclamation does not latch it: it only happens past the
  terminal, so by then there is nothing left to cancel. The latching step models the moment the call's delivery
  task takes the request into account, not the downcall's return: the window in
  between is level-1 stutter (the level-2 command queue), like the send path, so the
  guard `~IsCancelRequested` on `DeliverMessage` costs the implementation one test in
  the task's own loop, never a cross-thread lock
- `shutdown_event_emitted`, `shutdown_callback_running`: per runtime, the
  SHUTDOWN_COMPLETE discipline
- `second_event_owed`: per runtime, the tag SHUTDOWN_COMPLETE carried. The event
  says the runtime stopped running and reports whether the host still holds any of its
  memory; the answer is recorded because the second event is owed only when it was yes.
  A host told nothing was outstanding is spared the work of returning anything; it
  still reaches quiescence through the status, which is the only permission to
  destroy or unload
- `resources_released_emitted`, `resources_released_callback_running`: per runtime, the
  RESOURCES_RELEASED discipline. This is where the difference between the two observable
  statuses lives: `AK_RUNTIME_GRPC_STOPPED` and `AK_RUNTIME_QUIESCENT` are two refinements of
  the one level-0 RELEASED state, so the level-0 model cannot carry it and level 1 never
  writes level-0 state
- `runtime_destroyed`: per runtime, `ak_runtime_destroy` happened. `RuntimeDestroy` is
  guarded on `IsRuntimeQuiescent` - released, neither callback on the stack, the second
  event out if the first one said it was owed, nothing owed or lent across any call of
  the runtime, and no bytes given back but not yet released - and it stutters on the
  level-0 state,
  because destroying a handle is not a gRPC concept. Like every other downcall it carries
  no fairness. It is also what makes the handles stale: `IsRuntimeOfCallDestroyed` guards
  `ReleaseCallHandle` and `RequestCallCancellation`, the only two downcalls a finished
  call could otherwise still accept
- `buffer_state`: per call and per buffer, where that allocation is in its life -
  `none`, `lent`, `returned`, `freed`. Monotone, so an identity is used once. This is
  the one place the model carries a name the counters cannot, and it is tied back to
  `buffers_held_by_host` by `LentCountMatchesBufferStates`, which is what keeps every
  proof that reads the counter standing. `returned` and `freed` are two states because
  they are two events: giving the buffer back is the host's, releasing the memory is the
  runtime's, and the replay buffer is the gap
- `buffer_send`: per call and per buffer, the index of the send living in that
  allocation, zero for none. Keyed by the buffer because that is how
  `ak_call_send_message` keys it, so both questions the model asks are lookups - may
  these bytes go, and are they still needed. Without it nothing forbade releasing the
  memory of a message still on its way to the wire

- `buffer_charge`: per call and per buffer, the bytes the allocator handed out for that
  allocation. It is what the budget counts, and it is written once on a fresh buffer and
  never rewritten - the discipline `buffer_send` already follows
- `buffer_length`: per call and per buffer, the bytes `ak_buffer.len` exposes. Distinct from
  the charge because the allocator may round a request up: `FitsInBuffer` reads the length,
  so a commit must fit the view the host was given, while the ceiling counts what was really
  taken. `CoversRequest` ties them at the lend and nothing relates them afterwards
- `memory_used`: the runtime-wide counter, moved by the lend and the free the way an
  implementation moves it rather than evaluated as a sum on demand. Typed `Int`, the free
  being the one action that subtracts; `MemoryAccountingExact` is what makes it non-negative
  and what makes the ceiling mean anything
- `last_lend_status`: per call, what its last `ak_get_call_buffer` returned - `OK` or one
  of the three refusals. Nothing else reads it, which is the point: it is the observable
  frontier of the downcall, the state a level-2 binding refines its retry decisions
  against, without giving any other action a new way to be blocked. It records the
  backpressure sub-machine only - `OK`, `MESSAGE_TOO_LARGE`, `SLOT_BUSY`, `BUDGET_BUSY`;
  the rest of the downcall's result matrix (`HANDLE_STALE`, `INVALID_STATE`,
  `INVALID_ARG`, `INTERNAL`, and `*out` untouched on every refusal) is the ABI matrix's
  rows and the conformance tests' burden, not this variable's. An identity of
  attempts finer than the call - positions, tickets - is level 2's to introduce if its
  retry model needs one

Deliberately absent: no handle registry (validity is modeled, not indices and
generations, so the slot map's generation counter is an implementation of handle validity
rather than a modelled object), no read-credit variable (`ak_event_consumed` frees and arms in one
gesture, so credits available + payloads owed = `DeliveryCredits` on a live call and
one variable suffices), no start gate (derivable from `runtime_state`), no boundary
message lists (the in-flight gaps are the derived differences between the level-0
sequences).

Every action either refines a level-0 action (conjoining FFI guards and updates onto
the instantiated `L0!` action) or stutters on the level-0 variables; the level-0
machinery is the only writer of the level-0 state.

Guards, invariants and properties are written through named state predicates
(`HasFreeSendSlot`, `IsCancelRequested`, `IsRuntimeDrained`, ...); reading a
variable directly is reserved to update expressions, `Init` and `TypeOK`.

Three naming rules hold throughout, and they are written here because a reader who
has to induce them cannot tell a predicate from a step:

- **A stative verb is a predicate, a dynamic verb is an action.** `HostHoldsNoBuffer`,
  `HostOwnsNoPayload` and `RuntimeOwesFree` describe a state; `HostReturnsBuffer`,
  `HostConsumesEvent` and `RuntimeRelease` are steps. The party prefix says whose
  obligation or whose step it is, and never which of the two it is - the verb does that.
  There is no exception in either module.
- **`Is…` describes the object its argument names, `Has…` describes the call.**
  `IsLentBuffer(c, b)` is about the buffer, `HasFreeSendSlot(c)` about the call.
- **`Deliver…` carries a payload and spends a delivery credit; `Emit…` carries neither.**
  That is why WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED are emitted and the
  four data events are delivered. Note that `DeliverStatus` and `DeliverCancelled` both
  produce `AK_EVENT_STATUS`: two model actions for one event kind, distinguished by the
  status the payload carries. Every delivery action adds exactly one owned event and arms
  exactly one callback - an `ak_callback` carries one `ak_event`, and the model counts the
  host's debt off `events_delivered`, so an action that appended two events under one
  callback would owe the host a payload no callback carries. `DeliverCancelled` therefore
  requires the initial metadata to be out already: a call cancelled before anything was
  delivered gets two serialized callbacks, INITIAL_METADATA first - `DeliverInitialMetadata`
  fires on its own weak fairness - and the cancellation second.

Additional invariants (the FFI conjuncts of the level-1 inductive invariant):
- **SubmittedOccurrencesGloballyUnique**: an occurrence token is committed at most once,
  across every call - the submitted sequences are jointly injective, `NeverSubmitted`
  guarding the commit and this invariant making the guard citable. It is what lets a
  submitted message name its send, and the k-th WRITE_DONE name its message
- **ReceivedOccurrencesGloballyUnique**: the receive side of the same discipline - one
  reception per token across every call, `NetworkReceive` being guarded on a token never
  received anywhere
- **DirectionsShareNoToken**: a token names one occurrence in one direction, so a received
  token never reappears in emission nor an emitted one in reception - across all calls.
  Position orders each direction; the tokens are what `buffer_send` and the k-th
  WRITE_DONE key on, and what makes "the same bytes twice" two occurrences rather than
  one ambiguous value. Payload content, were it modelled, would be a separate function of
  the token, as `MessageLength` already is
- **UnusedCallsAreFfiClean**: no FFI state before `ak_call_start`
- **ReleasedCallIsClean**: a released call is terminal, every payload consumed, every
  buffer given back and freed, no delivery callback on the stack and no send in
  flight - the release's full postcondition, carried as an invariant so a client can
  cite what the reclaim guaranteed rather than re-deriving it from the guard. And it
  stays that way, because nothing can lend or deliver afterwards. This is the end-of-call guarantee: whatever the ending, Rust has everything
  back before the arena goes. The reclaiming step also waits for the call's own delivery
  callback to return and for every buffer given back to be released, both conditions only
  the runtime can read, which is why they moved into the guard when reclamation stopped
  being a downcall the host could be asked to establish
  it, and the runtime defers its teardown instead
- **SendsInFlightWithinLimit**: never more buffers out of one call's arena than
  `MaxSendsInFlight`, counting those the host is filling and those awaiting WRITE_DONE
- **WriteDonesNeverExceedSends / RunningWriteDoneWasEmitted**: the send-side
  no-double-free — never more acquittals than accepted sends
- **ReleasesNeverExceedDeliveries**: the host never releases more payloads than were
  delivered. This is conservation of a count, not of identities: the model does not track
  which `owner` a release names, so two releases of the same payload are indistinguishable
  from the correct release of two. It therefore proves *no over-consumption under a
  conformance assumption* - the host releases the right owner, once, in order - and that
  assumption sits beside the fairness hypotheses rather than being discharged here. A
  defensive ABI that detects a duplicate token would need the identities modelled; this
  design chooses assume-guarantee instead, and the level-2 obligations are where the
  binding pays it
- **TerminalCallHasNoSendInFlight**: WRITE_DONE always precedes the terminal
- **ClosingChannelCallsCancelRequested**: both paths into closing latch cancellation on the
  channel's active calls, so a closing channel drains without the host
- **ActiveCallPayloadsWithinCredits / PayloadsOwnedWithinCreditsPlusOne**: at most
  `DeliveryCredits` payloads owed while the call is active, one more only when the last
  is the terminal
- **LentCountMatchesBufferStates**: the bridge between the two views of the send buffers -
  the count `buffers_held_by_host` and the per-buffer state - is that the count is the
  cardinality of the lent ones. Both are kept on purpose: the count is what the send window
  and every proof about it read, the states are what carry an identity, and without this
  conjunct they could drift. That is the same failure as the send window and the ABI comment
  describing it coming apart, which is why it is stated rather than assumed
- **UnusedCallsHaveFreshBuffers**: a call that has not started has allocated nothing, the
  per-buffer form of `UnusedCallsAreFfiClean`
- **BufferSendIndicesExist**: a send index recorded against a buffer names a send that
  exists. `buffer_send[c][b]` is the send living in allocation `b`, zero for none - the
  identity `ak_call_send_message` takes and the counters could not express. Keyed by the
  buffer rather than by the send index, on purpose: both questions the model asks are then
  lookups, and one allocation carrying two sends is unrepresentable instead of being
  something an invariant has to forbid
- **CommittedBuffersAreGivenBack**: a buffer that carries a send is `returned` or `freed`,
  never lent again. Committing is one of the two ways to give a buffer back, so this is what
  stops one allocation being handed out twice
- **EverySendHasItsBuffer / SendsLiveInOneBuffer**: each accepted send lives in exactly
  one allocation. Keying `buffer_send` by the buffer makes one allocation carrying two
  sends unrepresentable, but it says nothing about the other two directions: that no send
  exists without a buffer, and that no two buffers claim the same send. Both are true by
  construction - a commit records the index the sequence is about to reach, which is above
  every index already recorded - and both are now stated rather than left to be read off
  the actions
- **UnacquittedSendKeepsItsBytes**: the bytes of an unacquitted send are still there. This
  is the conjunct that closes a real hole: without the send-to-buffer link, nothing forbade
  releasing the memory of a message the transport had not read yet, and no counter could
  have caught it because a counter does not know which allocation carries which send
- **NoDeliveryImpliesNoDebt**: a call that has never received a delivery owes no payload
  and has no delivery callback running. The delivery debt is created by the same step
  that appends the first event, which is what gives the initial metadata its credit
  without any help from the host
- **ActiveCallHasNoStatus / UnusedCallHasNoEvents**: the status event terminates the call
  in the step that appends it, so an active call never carries one and a call that has
  not started carries nothing. Both are guard-based, so they survive a failure - the
  cancellation drain needs them there
- **ShutdownSignalInv**: two halves, stated apart because they are preserved by
  different arguments. `ShutdownSignalCore` says SHUTDOWN_COMPLETE is emitted exactly
  once, from a drained runtime, and that release waits for its callback to return.
  `ReleaseSignalInv` says the second event is owed before it is sent, its callback
  never outlives it, the tag is accurate - a runtime whose event said nothing was
  outstanding really had an empty ledger - and the second event is honest: once
  `AK_EVENT_RESOURCES_RELEASED` has gone out, nothing of the runtime is in the host's
  hands and nothing the host gave back is still waiting to be freed. That last conjunct
  is what makes the event mean what the ABI says it means rather than only arrive; the
  two that read a ledger are the ones that need a drained runtime's ledger not to grow,
  which is why the split keeps each preservation obligation the size it was
- **MemoryAccountingExact**: the runtime-wide counter equals the charges of the send
  buffers actually out. This is the accounting claim with content, and it can fail - a lend
  that forgets its increment, a free that forgets its decrement or subtracts the wrong
  charge, a second credit for one buffer. Defined as the sum it would be a tautology, which
  is why `memory_used` is a variable the actions move rather than an expression evaluated
  on demand: that is how the implementation keeps it, and it is the number
  `ak_runtime_memory_usage` publishes. The three categories of the detailed observer are
  definitions over the same charges, and that they add up to the total is
  `CategoriesPartitionTotal` - a lemma and not an invariant, since it holds of any state.
  Both sit outside the `NotFailed` umbrella: failing changes neither the counter nor any
  charge, so a host still gets its memory back afterwards and the observers still answer
- **MemoryWithinCeiling**: the counter never passes the ceiling. Carried by the lend's
  guard alone, the free only ever subtracting

- **DestroyedRuntimeIsClean**: a destroyed runtime is quiescent, and stays quiescent -
  the whole gate, not half of it, so the invariant's name and `ak_runtime_destroy`'s
  precondition are the same sentence. Quiescence is absorbing for six separate reasons:
  RELEASED is a level-0 end state, neither runtime callback can be re-entered because
  the conjunct forbidding it is also what its writer's guard demands, the tag is frozen
  and the second event is a latch, and neither ledger can refill once the calls are
  quiet. It is kept apart from `ShutdownSignalInv` because it is the only runtime-level
  conjunct that reaches into the calls

Usability, proved of the ABI itself and assuming nothing of the host - what it permits it
does not then withdraw, and what it withdraws it withdraws completely:
- **DeliveryCallbacksReturn / WriteDoneCallbacksReturn / ShutdownCallbacksReturn /
  ResourcesReleasedCallbacksReturn**: every callback the runtime hands out comes back.
  These are the obligations the ABI imposes on the host, stated as guarantees so the
  fairness conjuncts they rest on are not hypotheses with nothing to buy. The last one
  is what makes `AK_EVENT_RESOURCES_RELEASED` safe to owe: without it the runtime could
  wait forever on a callback it dispatched and never reach `AK_RUNTIME_QUIESCENT`
- **DestroyedRuntimeRejectsHandles**: after `ak_runtime_destroy`, no downcall on any call
  of that runtime is enabled - not release, not cancel, not lend, not any of the three
  lend refusals (not even a status is written), not send, not end_send, not returning a
  buffer. Two of them are refused by a guard; the rest follow from what
  destruction already required, a released runtime having no live call and nothing of its
  memory outstanding. This is the formal content of "destroy invalidates every handle of
  the runtime": asserted here, and carried by a theorem rather than by the prose alone
- **WriteDoneFreesASlot**: `EmitWriteDone(c) ⇒ HasFreeSendSlot(c)'`. A host woken by a
  WRITE_DONE and asking for a buffer is never refused for want of a slot - cancellation, a
  closed send side or a retired handle each still refuse one on their own grounds. Nothing
  forced this to be stated -
  the send side is host-driven, so no fairness lift needed it - and its absence is what
  let the slot accounting drift from the ABI it documents. The receive side has the same
  property and got it by accident, because the `DeliverMessage` lift needed it

New liveness guarantees:
- **CancellationCompletes**: a cancelled call reaches its terminal without any ownership return from the application - the callbacks already in flight still return, which stays a host hypothesis
- **SendsEventuallyAcquitted**: per send — the k-th accepted send is acquitted by its
  in-order WRITE_DONE, whose callback returns
- **PayloadsEventuallyConsumed**: per payload — each payload handed over is
  individually consumed; rests only on the per-call host hypothesis
  `WF(HostConsumesEvent(call))`, which carries every payload of the call because
  release is FIFO
- **ShutdownEventEmitted**: a stopping runtime emits SHUTDOWN_COMPLETE
- **EventualChannelClosed**: a channel told to close closes, its calls cancelled and
  drained - what `ak_channel_release` promises. It rests on the runtime's fairness and
  on the host hypotheses too: the callbacks already dispatched must return for the
  drain to finish. Previously
  derivable and citable by nobody; the theorem makes it part of the interface
- **BufferEventuallyFreed**: every buffer the arena lends out is given back and then
  released. Two rungs with two owners: the host returns it, per buffer because returns are
  unordered, and the runtime releases the bytes once the send they carry is acquitted. The
  replay buffer is the gap between the two
- **CallEventuallyReclaimed**: a terminal call is reclaimed - handle retired, arena gone -
  without the host doing anything beyond giving back what it holds. This is what removing
  `ak_call_release` from the ABI buys: the guarantee is unconditional where a downcall the
  host might never make could not be. The escape is `ak_runtime_destroy`, which takes the
  arena with the runtime; there is deliberately no failure escape, because every action the
  drain rests on is untouched by a runtime failure
- **RuntimeEventuallyQuiescent**: a runtime that has stopped running reaches
  `AK_RUNTIME_QUIESCENT`, so a host polling `ak_runtime_status` is not waiting for
  nothing, and may then destroy or unload - and once destroyed, start a new
  runtime. Note the shape - the
  runtime promises the permission, never the destruction, because destroying is the
  host's call. It is stronger than the host's ledger emptying: it also carries the last
  callback having returned, which is the only thing that can say the trampoline thread
  is gone, and no callback could ever report that about itself. It carries the same
  failure escape as `ShutdownEventEmitted`: what makes a released runtime's calls
  terminal is a level-0 invariant, and level-0 safety is asserted only while no runtime
  sits in the failed state. The drains themselves survive a failure - they rest on
  `FfiCallInv` and `BufferStateInv`, both outside that umbrella - so the escape covers
  the premise, not the mechanism

- **ResourcesReleasedEventually**: a runtime whose `SHUTDOWN_COMPLETE` carried
  `AK_HOST_MUST_RETURN` does emit `AK_EVENT_RESOURCES_RELEASED`. This is the tag's own
  promise, and it is why a host may wait for the second callback rather than poll from
  the first. It is stated on the tag rather than on the runtime state because the tag is
  what the host reads, and it is derived rather than re-proved: the level-0 shutdown
  settles, the runtime quiesces from there, and quiescence with the tag set is the event
  having gone out. Level 2 refines this one rather than re-deriving it

- **BudgetEventuallyHasRoomFor**: the counter eventually has `len` bytes free, for every
  lendable `len`. The name is deliberate: it promises room in the model's accounting, not
  that the allocator admits the request - the allocator picks the charge, its size classes
  are its own, and whether a realizable class fits is the implementation's contract,
  carried by the ABI matrix and its tests. `IsLendable(len)` is the request the ABI
  considers at all - no larger than the ceiling, an empty serialized message being valid.
  `HasAccountingRoomForSomeCharge(len)` - some charge in the model's range covers the request and
  fits - is the existential the property closes on; a `AK_STATUS_BUDGET_BUSY` refusal
  denies one charge, not all of them, and a smaller one may already fit. Proved from the
  drain: every
  buffer out is eventually freed, so the outstanding set empties, the accounting makes the
  counter zero, and at zero the request is its own witness. It says nothing about who is
  served: lending carries no fairness, a competing caller may win every race, and no
  per-request grant is promised at this level. A guarantee that a *specific* refused
  request is eventually served would need an arbitration the ABI does not have - a FIFO
  waiter or a reserved permit - and a fairness on the host's retry *invocation*, not on
  the successful lend; stating it on the success would assume the very selection it
  claims to prove. What a retry loop is owed is level 2's `BudgetCancellationStopsRetry`

#### Fairness

Nineteen weak-fairness conjuncts, all individual, and they do not all belong to the same
party. Which side owes each one is the whole point of listing them, because the ones the
host owes are exactly the obligations a level-2 binding has to discharge.

| Owed by | Conjuncts | What it means |
| --- | --- | --- |
| Rust runtime | `NetworkSend`, `ReceiveStatus`, `EmitWriteDone`, `RuntimeRelease`, `EmitShutdownComplete`, `EmitResourcesReleased`, `ChannelFinishClosing`, `FreeReturnedBuffer`, `ReleaseCallHandle` | Its own threads and its own allocator. Nothing outside the library can stall them |
| FFI layer | `DeliverInitialMetadata`, `DeliverMessage`, `DeliverStatus`, `DeliverCancelled` | An event that reaches the queue reaches the host |
| Host (binding + application) | `DeliveryCallbackReturns`, `WriteDoneReturns`, `ShutdownCallbackReturns`, `ResourcesReleasedCallbackReturns`, `HostConsumesEvent`, `HostReturnsBuffer` | Six hypotheses the ABI imposes and cannot enforce |

The four callback-return conjuncts say that the callback the *host* installed eventually
returns. For our own .NET binding that is a property we implement and can point at; for
any other host it is an obligation the ABI imposes. The model is right to assume it -
nothing can make progress otherwise - but calling it a promise of the binding overstates
what is ours to guarantee.

The next two are about giving memory back. `HostConsumesEvent` is per call, which suffices
because payload release is FIFO: consuming past a payload without consuming it is not a
behavior the ABI admits. `HostReturnsBuffer` is per *buffer*, because buffer returns are
unordered - a per-call conjunct would let a host cycle some buffers while starving one.

The remaining downcalls (`CallStart`, `LendSendBuffer`, `SendMessage`, `EndSend`,
`RequestCallCancellation`, `RuntimeBeginShutdown`, `RuntimeDestroy`) carry no fairness:
the model never promises the host acts, only what follows when it does. `ReleaseCallHandle`
is not among them, because it is not a downcall - the runtime reclaims a settled call
itself, which is what removing `ak_call_release` from the ABI buys. Runtime shutdown
completes without any ownership return from the host, which discharges the level-0
directive on
`ShutdownFairness`.

Level-0 safety and the five level-0 liveness guarantees are not re-proved: the
refinement mapping is the identity on the level-0 variables, and `Spec => L0!Spec` is
proved with tlapm by lifting each level-0 fairness conjunct to the level-1 machinery.

### Level 2 — DotNetBinding

`DotNetBinding.tla` refines `FfiGrpc`: the state space, the actions, the fairness and
the properties below are the specification as written, and they now cover the three
mechanisms the design promises - the channel-held lease on the reusable shared runtime,
the write machine, and the managed completions.  No proof exists yet: the modules are
TLC-vetted (thirty-two of the thirty-three actions fire; the thirty-third is dead by
design, below) and in pre-proof review.

**Scope.** The model is the generic bidirectional-streaming call.  The five
`CallInvoker` methods are refinements of it that fix the number of messages in each
direction, not separate machines, so none is modelled - the unary shapes compose the
same reader and the same terminal consumer.  The start/send command queue is level-1
stutter.  The .NET async runtime is not modelled: completions signal and continuations
run elsewhere - `RunContinuationsAsynchronously` on every TCS is an implementation rule
verified by review and tests, not a theorem, and the model carries no counter pretending
otherwise.  The memory model of the ring stays a coding rule for review, outside every
level.

**Reuse is direct.** `DotNetBindingState` extends `FfiGrpcState`, so the level-0 and
level-1 variables are the same variables, not mapped copies; the level-1 machinery is
reached through `F == INSTANCE FfiGrpcTheorems`.  A coupled action conjoins the level-1
action it rides on; a managed-only action leaves `F!vars` unchanged; the runtime's own
actions pass through untouched.  `L2!Spec => F!Spec` is therefore the only refinement to
prove - `L0!Spec` follows from level 1's `RefinesSpec` by transitivity.  Ownership needs
no new relation either: `call_channel` already says which channel a call belongs to, and
`ChannelIds` is the lease identity space, so no `InvokerIds` and no refcount appear -
"the last lease released" is `AllLeasesReleased`, every channel unopened or disposed.

**The ring has no variables.** The trampoline publishes inside the delivery callback, so
the published prefix IS `events_delivered` and the head is its length; a release is
`ak_event_consumed`, so the tail IS `payloads_consumed_by_host`.  Both indexes are
level-1 state read through level-2 names (`RingHead`, `RingTail`), and two of the
obligations the ring owes come free of proof: release order is held by representation -
advancing a counter can only release the oldest - and a payload cannot be released twice
for the same reason.  What level 2 adds about the ring is who consumes it, and what the
reader and the writer are doing.

Added variables - all discipline, no capacity:
- `call_token_published`, `call_root_live`, `runtime_root_live`: the GCHandle plumbing -
  token and root born with the start, roots outliving the callbacks
- `current_runtime`, `runtime_dispose_state`: the shared runtime by generation -
  `absent` before the first channel and again after a full teardown, so a promise about
  the destroy attaches to the generation that is current, not to some earlier one
- `channel_dispose_state`: per channel, `unopened` / `constructing` / `active` /
  `disposing` / `disposed`.  The lease is live exactly between the first two and the
  last; the teardown's guard reads this function, never a counter
- `consumer_phase`, `reader_state`: which class of consumer has the ring, and what the
  application's read is doing - `idle`, `waiting` (a suspended `MoveNext`) or `parsing`
- `writer_state`, `retry_len`: the write machine and the length its budget wait
  remembers
- `headers_completion`, `status_completion`: the public objects that must never be left
  pending
- `call_dispose_state`: the call's own machine, driving the drain

**The factory and the channels.** `CreateRuntime(rt, ch)` is the first channel's
constructor reaching an unmaterialized factory: the shared root and `ak_runtime_create`
in one step, with that channel now `constructing`.  `AcquireLease(ch)` is a later
channel taking its lease; `CreateChannel(ch)` is its `ak_channel_create`, after which
the constructor returns and the object is exposed - binding-owed, a constructor that
began completes.  `BeginDisposeChannel` remembers the public `DisposeAsync`;
`DisposeCallForChannel` settles the calls of that channel and no others;
`FinishDisposeChannel` closes the native channel once they are all disposed and drops
the lease.  `BeginRuntimeShutdown` fires only when every lease is gone, then
`FinishDisposeRuntime`, then `FreeRuntimeRoot` - which re-arms the factory to `absent`,
so the next channel materializes a fresh generation.

**The reader ends.** A consumed terminal leaves the reader finished, and every later
`MoveNext` answers false at once, as `IAsyncStreamReader` requires - it never waits for
anything, and nothing in the model represents such a call because it touches no state.
A finished reader holds nothing, so the drain takes the ring from it as it would from an
idle one; only a parse in flight or a suspended `MoveNext` makes the drain wait.
That is also why "every call is disposed in the end" is stated as the application's
obligation rather than derived: an application looping on a false answer would satisfy a
read-or-dispose disjunction forever without disposing, so the model asks for the dispose
directly, as the API does.

**The reader, precisely.** `BeginMoveNext` commits the read whether or not a payload
exists; `BeginParse` wakes it when a payload arrives and only while the call is still
active - a dispose that linearized first wins the race, and the waiter then resolves
through `CancelWaiter`, never parsing a ring the drain already claimed.
`FinishConsumePayload` conjoins `F!HostConsumesEvent` when the parse completes, and it
is where the status is resolved if the slot it just decoded was the terminal one.

**The writer, precisely.** `WriteLendSucceeds` enters `serializing`;
`WriteRefusedBudget` enters the cancellable wait, `RetryLendSucceeds` leaves it,
`CancelWriterWait` resolves it on cancellation or dispose; `WriteRefusedTooLarge` faults
without waiting; `CommitWrite` sends and enters `awaiting_write_done`; `WriteAborted` is
the disposable wrapper closing over a throwing marshaller or a refused commit;
`WriteDoneCompletes` conjoins the level-1 callback return and completes the write.
There is no slot wait: with completion at WRITE_DONE and one writer per call, the next
lend always finds the window open, which `ManagedWriterNeverObservesSlotBusy` states.

**Fairness comes in three tiers, and the tiers are the point of the level.**
- *Runtime-owed* (`RuntimeOwedFairness`): the thirteen level-1 families the runtime and
  the FFI dispatch owe, taken verbatim - same actions, same tuple.  The binding
  restricts none of them, so their extraction from `Spec` toward `F!Fairness` is
  citation, not proof.
- *Binding-owed* (`BindingOwedFairness`): the callbacks return after bounded work, the
  waiter wakes or resolves, the drain drains, the hand-off happens, both dispose chains
  and the whole teardown complete, the constructor finishes, and serialization settles -
  a disjunction, because whether it commits or aborts is the marshaller's business
  while *that it settles* is the binding's promise.  These derive the six host conjuncts
  of `F!Fairness`: the six hypotheses level 1 imposed and could not enforce become
  theorems here.  Two of them cross user code and carry the one stated hypothesis: user
  serialization and parsing terminate.  The wrapper covers success and exception;
  nothing covers code that never comes back.
- *Application-owed* (`ApplicationOwedFairness`): one weak fairness per call - eventually
  begin the next read or dispose the call.  Beside it sit two conformity hypotheses of
  safety, not progression: `MoveNext` calls are serialized (`IAsyncStreamReader`) and
  writes are serialized (`IClientStreamWriter`), both encoded by the single
  `reader_state` and `writer_state` values.  Nothing else is asked: not feeding the
  request stream (sends are triggers, never owed), not completing it, not any cadence.

#### Level-2 safety invariants (to be proved by TLAPS)

Every name below is a conjunct of `ManagedSafety` in `DotNetBinding_defs.tla`, and
`ci/check_property_manifest.py` fails the build if this list and that conjunction
diverge in either direction.  `ManagedTypeOK` is also a conjunct, structural like
`TypeOK` in level 0's `SafetyCore`, with its own public theorem.

- **TokenPublishedBeforeStart**: no used call without its token - born in the same step
  as `ak_call_start`
- **RootSurvivesCallbacks**: a delivery or WRITE_DONE callback in flight resolves its
  `call_ctx` to a live root - freed by the terminal callback's own return, its last
  access
- **RuntimeRootSurvivesCallbacks**: every callback of every kind resolves `runtime_ctx`
  to a live root - the call callbacks included, freed only after `ak_runtime_destroy`
  returned
- **ConsumerPhaseMatchesDispose**: the phase machine and the call's dispose machine never
  disagree
- **AtMostOneReaderOutstanding**: the reader - waiting or parsing - exists only where
  the application reads; one value per call is what makes it unique
- **DrainNeverOverlapsApplicationConsumer**: the drain never runs beside an application
  read, suspended or parsing
- **WaitingWriterHoldsNoBuffer**: a write waiting on the budget holds no lent buffer -
  waiting for capacity while holding capacity is the deadlock level 1 cannot see
- **SerializingWriterHoldsTheBuffer**: exactly the serializing state holds a buffer, and
  it holds one - the lend/return discipline as an equation
- **WaitMatchesRefusal**: a waiting write's last lend result is BUDGET_BUSY
- **ManagedWriterNeverObservesSlotBusy**: no lend of this binding is ever refused for a
  slot - the consequence of completing at WRITE_DONE with a single writer, stated so a
  defect would show
- **RetryLenMatchesWait**: the remembered length exists exactly while the wait does
- **DisposeAwaitsDestroy**: the teardown reaching `destroyed` means `ak_runtime_destroy`
  returned for the **current** generation
- **RuntimeManagerCoherent**: a materialized generation has an identity and a root, an
  absent one has neither - the manager never claims a runtime it does not hold
- **LiveChannelUsesCurrentRuntime**: a live channel hangs off the current generation,
  never an earlier destroyed one
- **ManagedShutdownHasNoHostDebt**: the shutdown never owes the second event. Every call
  is settled before its channel releases, and every channel releases before the
  teardown, so `SHUTDOWN_COMPLETE` finds no host debt and
  `AK_EVENT_RESOURCES_RELEASED` is unreachable at this level - the level-1 machinery
  stays modelled and passed through for the refinement
- **LiveChannelKeepsRuntimeAlive**: a channel holding a lease keeps the runtime
  materialized - a channel is never left pointing at a torn-down runtime
- **NoRuntimeShutdownWhileLeased**: the native shutdown never starts while any lease is
  out
- **ChannelStateMatchesNative**: the channel machine and the native channel agree - no
  managed channel active without its `ak_channel`, none exposed before it
- **DisposeLeavesNoManagedWaiter**: a disposed call has its reader idle, its writer
  settled, its headers resolved and its status resolved
- **RingNeverOverflows**: `RingHead - RingTail <= DeliveryCredits + 1`.  Inherited, not
  re-proved: level 1's `PayloadsOwnedWithinCreditsPlusOne` read through the derived
  indexes, which is what lets the trampoline publish without a fullness test

#### Level-2 liveness (conditional on fairness)

The conjuncts of `ManagedLiveness` in `DotNetBinding_defs.tla`, bound by the same
checker.  Every promise crossing the native runtime carries the `~NotFailed` escape,
like every level-0 and level-1 promise: a failed runtime is the contract's one admitted
way out, and no termination is guaranteed past a failure.

- **BudgetCancellationStopsRetry**: a write waiting on the budget stops waiting once
  cancellation or dispose arrives.  The conditional shape is the whole property: the
  deadline is optional, cancellation may never come, and acquisition is deliberately not
  guaranteed since another call can always win the capacity - a behaviour polling
  forever is admitted.  Promising acquisition would need an arbitration the ABI does not
  have (a FIFO of waiters), and a fair `ak_get_call_buffer` cannot stay synchronous and
  non-blocking; that escalation stays available at no cost, since a poll remains correct
  once a signal exists
- **PendingWriteEventuallySettled**: a write that reached the buffer settles - it
  commits or aborts, and a committed one completes at its WRITE_DONE, which level 1
  guarantees before the terminal
- **ChannelConstructionCompletes**: a constructor that began completes - the channel
  reaches its exposed state, unless the runtime failed under it
- **CallDisposeCompletes**: a disposed call settles - the drain reaches the terminal,
  releases everything and resolves the status
- **ChannelLeaseEventuallyReleased**: a disposing channel gives its lease back - its
  calls settled, its handle released
- **ChannelDisposeCompletes**: the public `DisposeAsync` task completes. For a channel
  that was not the last holder it completes with its own release; for the last one it
  waits for the destroy it triggered, which is what its task promised - and
  `LastChannelDisposeAwaitsDestroy`, an action theorem, is that ordering made citable
- **RuntimeDisposeCompletes**: a teardown that began completes - destroy, then the root,
  then the factory re-armed
- **CallRootEventuallyFreed / RuntimeRootEventuallyFreed**: every allocated root dies -
  the call's at its terminal callback, the generation's after destroy
- **InFlightPayloadEventuallyReleased**: a parse completes and its slot is released -
  under the stated hypothesis that user parsing terminates
- **WaitingReaderEventuallyResolved**: a suspended `MoveNext` is resolved by payload or
  dispose, never abandoned
- **PublishedCallEventuallyDisposed**: every call the application created is disposed in
  the end.  It is a theorem of this model rather than only a norm of the API, and it
  needs the whole stack to be one: `EventualTerminal` and the delivery fairness bring
  every call to its terminal and drain its ring, the finite globally-unique token
  universe stops any reader from reading forever, and the application's own conjunct
  then has only the dispose left to take.  A physical system with an unbounded stream
  keeps the norm without the theorem
- **HeadersEventuallyResolved / StatusEventuallyResolved**: the public completions are
  never left pending - the prologue or the dispose resolves the headers, the terminal
  consumer resolves the status

#### Held by construction, not stated as invariants

Like level 0's monotone transitions: true of every behaviour, by the shape of the
actions rather than by induction, and this document must not imply a theorem exists.

- **ContinuationsAsync**: no completion runs a continuation on the callback's thread.
  In the model, no action both returns a callback and performs an application step; in
  the implementation, every TCS is `RunContinuationsAsynchronously` and every signal is
  latched - an implementation rule verified by review and tests, deliberately not
  restated as a counter the model would prove things about.  With no dispatcher between
  the trampoline and the application this is the only thing keeping user code off the
  Tokio thread, and it is what makes the callback-return fairness the binding's to
  promise
- **MessageTooLargeIsNotRetried**: no action enters a wait from MESSAGE_TOO_LARGE - the
  refusal is permanent by construction, so a retry would poll against a constant.  The
  absence is the mechanism
- **PayloadsReleasedInOrder / ReleasedAtMostOnce**: the release counter can only advance
  by one, so the order is the data structure and a double release cannot be expressed.
  This is the price level 1's counter abstraction charged, paid by representation - and
  the hand-off preserves the tail for the same reason: `HandoffToDrain` touches no
  level-1 state, and `ConsumerHandoffPreservesTail` states it as a public theorem
- **SingleStreamConsumer / SingleStreamWriter**: one `reader_state` and one
  `writer_state` per call, every consuming or writing action guarded by them - two
  concurrent `MoveNext`, or two concurrent `WriteAsync`, are unrepresentable.  The
  representation encodes what the API requires of the caller: the serialization is the
  application's conformity hypothesis, not a guarantee the binding manufactures
- **NoDowncallAfterDestroy**: the call downcalls carry `BindingMayDowncall`, the channel
  downcalls their own channel-machine guards, and `BeginRuntimeShutdown` requires every
  lease released - an ordering on dispose, not a safety net.  Level 1's
  `DestroyedRuntimeRejectsHandles` is the runtime's side of the same fact
- **The second event is dead at this level.** `AK_EVENT_RESOURCES_RELEASED` exists in
  the ABI and level 1 for a host that still owes memory when the shutdown starts.  This
  binding never does: a call is disposed only with its ring drained and its writer
  settled, and the teardown starts only when every channel is settled, so
  SHUTDOWN_COMPLETE always finds no debt.  `ResourcesReleasedReturns` is therefore the
  one action of the model that never fires - checked, not assumed: a directed
  configuration prunes buffer returns before the shutdown event and completes with no
  violation of "the second event is never emitted".  The action stays for refinement
  completeness, and its emptiness is a property of the dispose discipline
- **BuffersAlwaysReturned / ReleasedEventually**: not invariants but fairness conjuncts -
  the disposable wrapper's WF and the begin-or-dispose WF respectively
- **RetainedBytesAreEventuallyFreed** stays a native-Rust obligation: the retention is
  the runtime's own decision, no managed code observes it, and nothing level 2 models
  can discharge it.  It is the one place where the implementation is deliberately slower
  than the model rather than the reverse

The public interface, `DotNetBindingTheorems.tla`, declares the obligations the freeze
requires discharged - `RefinesInit`/`RefinesNext`/`RefinesSpec`, the six host
discharges, `ManagedTypeOKHolds`, `ManagedSafetyHolds`,
`ConsumerHandoffPreservesTail`, one theorem per liveness promise and their aggregate -
none of them proved today.

Refinement mapping, by direct reuse:
- the first `new NativeGrpcChannel(options)` ↔ `CreateRuntime` then `CreateChannel` -
  the shared root and `ak_runtime_create`, then this channel's `ak_channel_create`,
  the constructor returning only afterwards
- a later channel ↔ `AcquireLease` then `CreateChannel`
- the call constructor ↔ `StartCall` - `GCHandle.Alloc`, `ak_call_start` and exposure in
  one step
- `OnEvent` publishes a slot ↔ `OnEventReturns`; the terminal one is
  `TerminalCallbackReturns`, which frees the call root and decodes nothing
- `MoveNext`, its suspension and its parse ↔ `BeginMoveNext`, `BeginParse`,
  `FinishConsumePayload` - the last conjoining `F!HostConsumesEvent` and resolving the
  status when the slot it decoded was the terminal
- `WriteAsync` ↔ `WriteLendSucceeds` or a refusal, `CommitWrite` or `WriteAborted`,
  then `WriteDoneCompletes`; `CompleteAsync` ↔ `CloseWriter`
- `Dispose` on a call ↔ `BeginDisposeCall`, `CancelWaiter` for a suspended read,
  `HandoffToDrain` behind any parse in flight, `DrainRelease` until drained, then
  `FinishDisposeCall`
- `DisposeAsync` on the channel ↔ `BeginDisposeChannel`, `DisposeCallForChannel` per
  owned call, `FinishDisposeChannel`; and when it held the last lease,
  `BeginRuntimeShutdown`, `FinishDisposeRuntime`, `FreeRuntimeRoot`

Level 2 re-proves none of the window reasoning: with `Spec => F!Spec` established the
same way level 1 established `Spec => L0!Spec`, the send bound, the credit bound, the
per-object liveness, the end-of-call cleanliness and both no-double-free invariants are
inherited rather than restated.

#### What no level of the specification covers

The models assume state updates are atomic and sequentially consistent; concrete memory
ordering, encodings and the code the models abstract on purpose sit outside that
assumption.  This is the closed frontier: twelve subjects deliberately outside every
level, each with what verifies it instead. The list exists so that none of them is mistaken for a gap - a
proof obligation nobody wrote - and so that none is reopened as one. No further level of
refinement would help with any of them: they are properties of concrete memory, of
encodings, or of code the models abstract on purpose.

| Subject | Verified by |
|---------|-------------|
| **The ring's memory model** - `Volatile` pairing, acquire/release, false sharing | Code review and a race test, ARM64 included |
| **No inline continuation** - every TCS `RunContinuationsAsynchronously`, every signal latched | Review, plus a test that no user code runs on a callback thread |
| **Serialized bytes** - arbitrary `Marshaller<T>` round-trips | Protobuf round-trip tests |
| **Exact metadata, status and trailers**, and the .NET exception mapping | gRPC conformance tests |
| **The five `CallInvoker` shapes' cardinalities** - the model is the generic bidirectional call | A test per shape |
| **`CallOptions` in full** - deadline, credentials, headers | Still declared missing work, not a hidden claim |
| **The lease refcount's algorithm** and the singleton's publication | A concurrent create/dispose race test |
| **The handles' concrete encoding** - widths, allocation, type discrimination | ABI header work and stale-handle tests |
| **Budget polling's cadence, backoff and starvation** | Nothing: deliberately not guaranteed, and the model says so |
| **Payload owner identity** - FIFO release is a conformance hypothesis | `ak_call_debt_of` in assertions and tests |
| **The `WriteTcs` publication race** around the send downcall | A directed race test |
| **A compilable C ABI**, layouts, versioning, protocol encodings, tri-language tests | The header and conformance phase of the binding plan |

**The memory model is the price of the ring.** A missing `Volatile.Write` on `head`
produces a ring that violates everything proved above, and neither level 1 nor level 2
will catch it. The release/acquire pairing is a coding rule, and it belongs in review
rather than among the proof obligations, where listing it would suggest a coverage that
does not exist. It is the price of a zero-copy SPSC ring, and it is worth paying, but it
is worth naming.

Four of the twelve carry a decision the implementation must not improvise, so the
decision is here rather than in the code that will need it.

**Cancellation faults with `RpcException`.** A call disposed or cancelled before its
metadata arrives resolves `ResponseHeadersAsync` - and every other pending managed object
of that call - with an `RpcException` carrying `StatusCode.Cancelled`. One rule, one
exception type, whatever the path: no `ThrowOperationCanceledOnCancellation` option is
ported, so calling code stays in the `RpcException` world grpc-dotnet callers already
handle. The model states that nothing is left pending
(`DisposeLeavesNoManagedWaiter`); which exception carries the failure is this decision.

**The lease refcount is a lock and a counter.** The factory's lock guards one pair - the
current runtime and the number of leases out - so acquisition and last release are
decided under the same lock, and `ak_runtime_destroy` is called outside it once the
counter reached zero. It costs one uncontended lock per channel construction and
disposal, never anything on a hot path, and it is obviously correct where an
`Interlocked` counter would need an argument about resurrection that the model does not
supply: level 2 derives "the last release" from the set of settled channels, and the code
must reach the same conclusion by counting.

**The write TCS is published before the commit, and rolled back on refusal.** The
WRITE_DONE callback may run the moment `ak_call_send_message` accepts, so the TCS has to
be reachable from `CallState` before the downcall - a callback that finds nothing would
lose the completion the write is waiting on. If the commit is refused instead, the same
`using` scope that returns the buffer withdraws the TCS and faults it. Publishing after
the acceptance would need a landing slot for a callback that arrived early, which is more
machinery for the same guarantee.

**The handles' encoding is a property, not yet a layout.** Three properties must hold, and
the widths that carry them belong with the C header, where `ak_abi_version` and the struct
layouts are chosen and where the tri-language tests live:

- a stale handle is refused, never dereferenced - the generation counter is the
  implementation of the released and destroyed states the models carry, and nothing else;
- a live handle of the wrong kind is refused as an argument error rather than resolved
  against the wrong object: index spaces are per kind, so a channel's index is plausibly a
  live call's index and the generations of two spaces climb in parallel. Either a kind tag
  in the handle or a kind field in the slot discriminates them; the slot field is the
  cheaper of the two, and the choice belongs with the slot map's design;
- generation wraparound is impossible by construction, not merely improbable: a slot whose
  generation would saturate is retired instead of reused. One comparison at allocation
  buys a structural argument where a width alone would only buy a large number.

The arithmetic that will size them, recorded so it is not redone: an index space covers
*simultaneous* objects - a runtime, a handful of channels, the concurrent calls, and per
call at most `MaxSendsInFlight` buffers and `DeliveryCredits + 1` payload owners - while a
generation counts a slot's *reuses over the whole process lifetime*. Sustaining a hundred
thousand calls a second for ten years is some three times ten to the thirteenth calls; over
four thousand slots that is under two to the thirty-third reuses each. The index wants
twelve to sixteen bits, the generation something above thirty-two, and the two together
leave room in a machine word for the kind.

### TLAPS Proof

The proof strategy, applied to both written levels:
1. Prove the safety invariants of each level independently, by induction on `Next`
2. Prove that level's liveness from that level's fairness assumptions
3. Prove the refinement to the level above (simulation on the identity mapping)
4. Prove refinement liveness by lifting: the current level's fairness and safety
   implement each fairness conjunct of the level above, so the upper level's
   liveness is inherited rather than re-proved

Each level is split in four modules: the state (`*State.tla`, the variables and the
constant assumptions), the specification (`*.tla`), the invariants and the theorem
*statements* (`*_defs.tla`, `*Theorems.tla`), and the proofs (`*Theorems_proofs.tla`).
The split lets `check_theorem_statements.py` verify that every proved theorem restates
its declaration verbatim, so a proof can never quietly weaken what it claims. No proof
step is `OMITTED` at either level.

A second checker binds this document to those modules. `ci/check_property_manifest.py`
compares each level's property list here with the conjunctions that level proves -
`SafetyCore` and `LivenessProperties` at level 0, `FfiCallInv`, the extra conjuncts of
`SafetyInvariant` and `LivenessProperties` at level 1 - and fails in both directions: a
property claimed here that no manifest carries is a promise nothing proves, and a proved
conjunct absent here is a guarantee nobody can find. Both run in `ci/check.sh`. The
send-window deadlock got in through exactly that gap - the accounting and the ABI comment
describing it drifted apart with nothing comparing them - so the checker is part of the
fix rather than housekeeping.

#### What is actually verified, as of this revision

The document describes the target specification; the verified artefact trails it while a
change is in flight. This table is the honest reading, and it is meant to be updated with
the artefact rather than left to rot:

| Element | Status |
|---------|--------|
| Specification described in this document | Current |
| Model-checking configurations | Nine configurations exist - five at level 1, four at level 0 - and running them is not part of this gate: every property they would check is proved by tlapm, over unbounded constants where the configurations would fix `Ceiling = 3` and unit messages. They are kept for exploration and debugging - a checker that prints a counterexample trace is the fastest way to understand a broken draft - not as evidence |
| Level 1, one pass at `--stretch 1` | **11444 obligations, all proved, 12m8s at `--threads 12`**, this revision. A single pass is the whole verification: with the optimized tlapm build (`qdelamea-aneo/tlapm`, `/root/tlapm-opt-wil`) it is fast enough to iterate on, and it is the only count free of the obligations two adjacent windows would both cover |
| Level 0, one pass at `--stretch 1` | **1805 obligations, all proved, 2m13s at `--threads 12`**, this revision - the event-trace conjuncts `EventStreamShape` and `MessageEventsMatchDelivered` joined `SafetyCore`, so the level-0 module changed and was re-proved in full |
| A scatter of failures clustered by *backend* is a resource signature | At `--threads 4` on a machine where other provers were running, the same module returned 12 failures and **every one of them named `Isa`** - including steps untouched for weeks and unrelated to each other. Isabelle is the first backend to exhaust its budget under contention. Read the failing lines before theorizing about the goals they carry: the cluster was diagnosed twice as a property of `Fairness` before anyone looked at the method column. Every Isabelle call in the module carries `IsaT(600)` - a ceiling and not a cost, so a step needing two seconds still takes two, and an Isabelle failure now means a proof defect rather than contention |
| Where Isabelle is irreducible | Extracting one weak-fairness conjunct at a fixed identifier needs a backend that can instantiate a lemma whose conclusion is a conjunction of `WF_` atoms. `PTL` cannot instantiate; **Zenon cannot read `WF_` at all**. Four `QED` steps that were only doing modus ponens on a quantifier-free antecedent moved to `PTL`; the seven citations of `FairnessAtCall` and its siblings cannot move, and the three `QED`s whose antecedent crosses a bounded quantifier cannot either |
| `ExpandENABLED` and `TypeOK` | Never expand `TypeOK` in the `BY` of an `ExpandENABLED` call. `FreeBufferEnabled` resisted every backend, budgets to 300s and `--stretch 5` while its DEF list carried `TypeOK`: the expansion piles one membership conjunct per variable onto a goal that is already an existential over every primed variable, and the solver stops finding the witness. Use `TypeOK` only in the step that establishes `vars' # vars` beforehand - here a prime-free disequality on the `EXCEPT` - and cite it as an opaque fact in the `ExpandENABLED` step. The same proof then closes at `--stretch 1`. It surfaced when the free began writing a variable of its own, because while a variable is unconstrained the solver refutes "nothing changed" by varying it and never walks the long path |
| `ci/check_theorem_statements.py` | 72 declarations - 71 theorems and one public lemma - each restated verbatim in its proofs module |
| `ci/check_action_footprints.py`, `check_abi_coverage.py`, `check_proofs_present.py`, `check_arity.py` | Green |
| SANY, on the ten SANY-clean modules | Green |
| `ci/check_property_manifest.py` | Green: this document's property lists and the manifests name the same properties |
| The two memory observers' normative invariants | **Covered at level 1.** `buffer_charge` holds the bytes each lent buffer was granted and `memory_used` the runtime-wide total; `MemoryAccountingExact` states `memory_used = BytesOutstanding` and `MemoryWithinCeiling` that the total never passes `Ceiling`. Both are in `IndInv` and proved inductive. The four category totals - `BytesHostLent`, `BytesSendInFlight`, `BytesRuntimeHeld`, `BytesOutstanding` - are sums over the pairs each state selects, and `CategoriesPartitionTotal` is the snapshot identity the observers must report |
| Level 2 | **Drafted and TLC-vetted, no proofs.** The DotNetBinding modules cover the whole managed contract - the channel-held lease on the reusable shared runtime, the reader, the write machine, the completions - SANY-clean and registered in `ci/check.sh`; the property manifests are bound to this document by the manifest checker, and `DotNetBindingTheorems` declares the freeze's obligations. TLC (INIT/NEXT with per-action coverage, runs bounded by construction): thirty-two of the thirty-three actions fire across two configurations, one broad and one directed at the call path; `ResourcesReleasedReturns` is dead by design, and a third, directed configuration completes with no violation of "the second event is never emitted", which is the check of that claim. No invariant violation was found. No obligation has been given to TLAPS: nothing level-2 is proved |

There is an objection to modelling any of this, and it is half right, so it is worth stating.
The partition identity is close to true by construction: `BytesOutstanding` is a sum over the
pairs `buffer_state` selects, so `CategoriesPartitionTotal` discriminates no design and would
catch no defect on its own. Where the objection stops holding is `MemoryAccountingExact`, which
is not of that kind. It relates a counter
the actions update by arithmetic - `memory_used' = memory_used + charge` on the lend,
`- buffer_charge[<<cId, b>>]` on the free - to a sum over a set those same actions reshape, and
nothing makes the two agree except the actions being written correctly. It is what makes the
ceiling mean anything: without it `MemoryWithinCeiling` bounds a number with no stated relation
to the memory that is out. It is also the only reason `memory_used` can be typed `Int` and still
be known non-negative, the free being the one action that subtracts.

The price the objection names is real and is paid: the sums over sets are the expensive
part of these proofs. `SumFunctionOnSet` from the standard `Functions` module and its theory in
`FunctionTheorems` carry it - `SumFunctionOnSetAddIndex` for the lend, `SumFunctionOnSetRemoveIndex`
for the free, `SumFunctionOnSetEqual` where a state moves without changing a charge. A fold taking
its summand as an *operator* parameter has no citable primed form, which is the trap that shape
walks into; `SumFunctionOnSet` is first order in both arguments and the prime distributes.

The refusals are modelled, and as their own actions: `RefuseLendTooLarge`,
`RefuseLendForSlot` and `RefuseLendForBudget` write `last_lend_status` and nothing else. They
carry no fairness - refusing is never owed - and every property proved of the other actions
crosses them, since the state they touch is read by none of them.

Modelling them is not decoration: the refusals are the observable frontier of the downcall,
the states a level-2 binding refines its retry decisions against, and what the ABI's status
codes mean is defined by which model action wrote them. `RefuseLendForBudget` takes the
charge as a parameter: it records that *this* one did not fit, not the claim that none would
- a smaller charge may already fit when the refusal lands.

A dead action would satisfy every safety proof - TLAPS happily proves that an action that
can never fire preserves everything - so each status carries its own conditional
enabledness theorem: `TooLargeRefusalEnabled`, `SlotRefusalEnabled` and
`BudgetRefusalEnabled` state that at any eligible state the refusal whose guard holds is
`ENABLED`. Conditional is the honest word: they do not prove the hypotheses reachable
from `Init` - `SlotRefusalEnabled`'s full window, in particular, is reachable only when
`Cardinality(BufferIds) >= MaxSendsInFlight` - but the first exhibits its own rigid
witness, `Ceiling + 1` being in the request domain and never lendable. That witness is
why the
refusals quantify over `RequestLengths == 0..(Ceiling + 1)` rather than over the lendable
sizes: one representative above the ceiling stands for every larger request, and without it
`RefuseLendTooLarge` would be unsatisfiable and every proof about it vacuously true. The
budget refusal's charge ranges over `CandidateCharges`, the same domain, for the same
reason.

What stays true is the shape of the refusal. **A refusal is not a state of the runtime**: it
records what a downcall returned, not a condition the runtime is in. A runtime-level
`RESOURCE_EXHAUSTED` state would be the same mistake as the fatal ceiling that preceded this
design, and promoting a refusal to a state is what made the runtime undestroyable.

What does deserve a model is the retry protocol, and it is level 2's because it is about the
binding's own scheduling rather than about bytes. One boolean per call - retrying or not -
carries `BudgetCancellationStopsRetry` and `MessageTooLargeIsNotRetried` with no counter
anywhere, and it carries the deadlock that level 1 cannot see: level 1 *assumes* the host
gives back what it holds, so a host blocked polling for capacity while holding a lent buffer
is admitted there and fatal in practice. See `WaitingWriterHoldsNoBuffer`.

Nothing above is `OMITTED`, nothing fails, and both obligation counts were measured on the
model as it stands here rather than carried over. The proof's framing rests on thirteen
framing lemmas - `OnlyCallStartWritesCallChannel`, `EveryStepEitherLendsOrKeepsBuffers` and
their siblings, each naming the writers of one variable - over four projection lemmas
(`FfiOnlyStutters`, `StutterProjects`, `FfiOnlyStepsKeepL0`,
`RuntimeAndChannelStepsKeepCalls`), which trade one large obligation for several small
ones. The shape is load-bearing: frames that enumerate the whole action alphabet put
every new action in every frame obligation, and past nineteen frames no solver timeout
closes them - naming each variable's writers is what lets the alphabet keep growing.

---

## What this document is not, and what is missing

This file mixes four registers - a normative contract, the reasoning that led to it,
implementation sketches, and the proof strategy - and that is why contradictions in it are
hard to see. It should be split: the functional and protocol contract, the normative ABI
with its state machines and ownership matrix, the Rust and .NET implementation
architecture, the formal model and its mapping to the code, and a decision log. Until that
split happens, read the ABI blocks as normative and the rest as justification.

**The ABI is not yet a contract you can compile against.** What is specified here is the
shape and the ownership rules; what is missing is everything a second implementer would
need, and none of it is decided by omission:

- a real C header that compiles, with `ak_status` enumerated, `ak_runtime_config`,
  `ak_call_start_options` and the metadata and status payload encodings given as byte
  layouts rather than described;
- calling convention, struct alignment and padding rules, and a `size`/`version` prefix on
  every options struct so the ABI can grow without breaking callers;
- an ownership matrix: for each ABI object, who allocates, who frees, and when it stops
  being legal to touch;
- conformance tests exercised from Rust, C and C# against the same header, because an ABI
  that only its author's binding uses is not an ABI.

**Protocol surface not yet contractualized.** Message and metadata size limits and what a
violation produces on each side; gRPC compression (`grpc-encoding`,
`grpc-accept-encoding`, per-message compressed flag); `grpc-timeout` derivation from the
deadline and what happens when both a channel default and a call deadline exist; `-bin`
metadata keys and their base64 encoding; `grpc-message` percent-encoding; GOAWAY handling
and stream re-attempt; and the Trailers-Only response, which the ABI normalizes but whose
status mapping is not written down. Each is a place where two implementations would
diverge silently.

**The `CallInvoker` mapping owes `CallOptions` in full**: deadline, cancellation token,
request metadata, per-call credentials, host override, write options and method type. The
table above maps the five call shapes and stops there.

## Open decisions (to be resolved during implementation)

| Question | Options | Impact |
|----------|---------|--------|
| Crate for X509Store Windows | Direct native APIs / `schannel` crate / `windows` crate | Layer 1 |
| Exact handle format | **Slot map: index plus generation.** A stale handle is refused with a status instead of dereferenced, which is what makes runtime-driven reclamation safe: the host may still hold a token for a call already reclaimed | Layer 3, decided |
| Default replay buffer (`max_buffer_size`) | 0 (no streaming retry) vs 4KB vs 64KB | Layer 2 config. Sets how many sent bytes an arena retains past their WRITE_DONE, which is the one knob between replayability and memory held |
| Host queue signal mechanism | **Moot: there is no host queue.** The per-call ring replaces it, and its signal must be latched auto-reset - never `SemaphoreSlim`, whose `Release` can run a waiter inline on the callback's thread | Layer 4, decided |
| Generator for the C# options | **Roslyn source generator.** T4 needs an external tool and a build step of its own; a generator runs in-compilation and stays in step with the Rust JSON schema by construction | Layer 4, decided |
| Connection pool management (idle eviction) | Internal timer vs lazy check | Layer 2 |
| Command delivery to a call actor | Per-actor channel vs atomic flags + notify | Layer 3. Cancel is a flag the actor polls; send and end_send carry data. Reclamation is neither - it is the actor's own step when the debt counters reach zero, so it is a wake rather than a command. A single channel is simpler, two mechanisms are faster |
| Send memory on .NET | **Moot: the host never owns it.** `ak_get_call_buffer` lends native memory, protobuf serializes straight into it, `ak_call_send_message` gives it back. Nothing to pin, nothing to copy, and no managed heap to fragment | Layer 4, decided |
| Payload allocation shape | One `Vec` per event vs slices of a pooled `Arc<Vec<u8>>` | Layer 3. The ABI already carries `owner` separately from `ptr`, so both fit without changing the contract |
| `ak_channel_release` returns void | Keep void as an idempotent no-op / return `AK_STATUS_HANDLE_STALE` like the other downcalls | Layer 3. The general promise says every downcall on a dead handle reports HANDLE_STALE; the release is the one exception, and either the signature or the promise must move |
| Low-memory probe | Warn when the memory the system has available drops below `ceiling` / no probe | Layer 3. The ceiling bounds what this runtime lends, not what the machine has left; a runtime configured near the machine's limit refuses nothing and is killed instead. Deferred until there is operational data to set a threshold against |
| A ceiling for the receive path | Configurable capacity, reserved from a bounded pool / unbounded as now | Layer 3. Would make the receive side refusable the way emission is - a different design from this one, and one that needs data on real receive footprints before it is worth the ABI surface |

---

## References

- [gRPC over HTTP/2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
- [gRPC Retry Design](https://github.com/grpc/proposal/blob/master/A6-client-retries.md)
- [tower::Service](https://docs.rs/tower/latest/tower/trait.Service.html)
- [Grpc.Core.CallInvoker](https://grpc.github.io/grpc/csharp-dotnet/api/Grpc.Core.CallInvoker.html)
- [TLA+ Proof System](https://tla.msr-inria.inria.fr/tlaps/content/Home.html)
