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
/// It knows neither HTTP/2 nor gRPC — it is a network dial.
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
    /// Loads trust roots from the configured source.
    /// Returns None for System (rustls uses its own roots).
    pub fn load(&self) -> Result<Option<CertificateDer<'static>>, ConfigError>;
}

pub struct ProxyConfig {
    pub source: ProxySource,
    pub credentials: Option<ProxyCredentials>,
}

pub enum ProxySource {
    /// No proxy — direct connection. Deserialized from "none", "disabled" or equivalent.
    /// Implies that NO_PROXY is checked (if the target matches NO_PROXY, we stay direct
    /// even if another source is configured elsewhere). When this variant is chosen
    /// explicitly, NO_PROXY does not apply — it is an unconditional refusal.
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

pub struct ProxyConfig {
    pub source: ProxySource,
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
    pub eager_connect: bool,    // default = false
}

pub struct Http2Config {
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    pub max_concurrent_streams: Option<u32>,
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
}
```

### GrpcChannel

```rust
pub struct GrpcChannel { /* ... */ }

impl GrpcChannel {
    /// Creation. If eager_connect = true, initiates the connection immediately
    /// via the provided executor. Otherwise, the first connection is lazy.
    pub fn new(
        config: GrpcChannelConfig,
        executor: impl Executor,
    ) -> Result<Self, ConfigError>;

    /// Starts a gRPC call.
    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError>;

    /// Closes the channel: refuses new calls, cancels active calls.
    pub fn close(&self);
}
```

### GrpcCall

```rust
pub struct GrpcCall { /* ... */ }

impl GrpcCall {
    /// Sends a message (serialized protobuf bytes).
    /// In pure Rust usage: copies the buffer and returns when accepted by the framing layer.
    /// In FFI usage: the caller keeps the buffer valid until the WRITE_DONE callback.
    /// Only one send in flight per call (the next must wait for the previous one to complete).
    pub async fn send_message(&self, msg: Bytes) -> Result<(), CallError>;

    /// Signals end of sending (END_STREAM on the request body).
    pub async fn end_send(&self) -> Result<(), CallError>;

    /// Retrieves the initial metadata (HTTP/2 headers from the response).
    /// Blocks until reception or terminal.
    pub async fn recv_initial_metadata(&self) -> Result<Metadata, CallError>;

    /// Retrieves the next message or the terminal status.
    /// Each call implicitly constitutes a request for one message (natural backpressure).
    /// Returns End(GrpcStatus) when the stream is finished — this is the call's terminal.
    pub async fn next_message(&self) -> Result<RecvResult, CallError>;

    /// Cancels the call (sends RST_STREAM).
    pub fn cancel(&self);
}

/// Reception result: a message or the end of the stream (status + trailing metadata).
pub enum RecvResult {
    /// A received gRPC message.
    Message(OwnedMessage),
    /// End of stream — gRPC status + trailing metadata. Terminal, nothing after this.
    End(GrpcStatus),
}

/// Terminal status of a gRPC call.
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
    pub trailing_metadata: Metadata,
}

/// Received message. Owned — the caller frees when done.
/// In V1, it's a Vec<u8>. In future zero-copy, it will be a handle to
/// a Rust buffer with explicit release.
pub struct OwnedMessage {
    pub data: Bytes,
}
```

### CallStartOptions

```rust
pub struct CallStartOptions {
    pub method: String,             // e.g.: "/armonik.api.grpc.v1.Sessions/CreateSession"
    pub metadata: Metadata,         // request metadata → HTTP/2 headers
    pub deadline: Option<Deadline>, // override of the channel default
    /// Reserved post-V1: override of the retry policy for this call.
    /// In V1, must be None — the channel default applies.
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
compatible with the generated Tonic stubs. The adapter implements `tonic::transport::Channel`
(or the `GrpcService<BoxBody>` trait) by delegating to `GrpcChannel`:

```rust
/// Adapter that allows Tonic stubs to consume a GrpcChannel.
pub struct TonicAdapter {
    channel: GrpcChannel,
}

impl tower::Service<http::Request<BoxBody>> for TonicAdapter {
    type Response = http::Response<BoxBody>;
    type Error = tonic::Status;
    type Future = ...;
    // Decodes the Tonic request, routes it to GrpcChannel, reconstructs the response.
}
```

The Rust client and the .NET binding share the same native `GrpcChannel`. Rust uses it via
`TonicAdapter` (no FFI). .NET uses it via the FFI layer + CallInvoker. Both benefit from the
same engine (retry, deadline, pool, flow control).

### Retry — commitment point

| Situation | Retryable? |
|-----------|------------|
| Unary: error before response | Yes |
| Client streaming: error, data sent ≤ buffer | Yes (replay from buffer) |
| Client streaming: error, data sent > buffer | No (committed) |
| Bidi: error, no response received, data ≤ buffer | Yes |
| Bidi: response received (initial metadata or message) | No (committed) |
| Server streaming: error before initial metadata | Yes |
| Remaining deadline < backoff | No |

---

## Layer 3 — `armonik-grpc-channel-ffi`

### Principles

- The FFI runtime owns a Tokio runtime and uses it as Executor for the GrpcChannel
- Every spawned task is registered in a task group (joinable at shutdown)
- Handles are opaque identifiers validated in an internal registry, implemented
  as a SlotMap (index + generation, chained free list for O(1) allocation)
- Received message payloads are **owned**: the host receives an `ak_bytes` that it must
  release. This prepares for future zero-copy (the host will be able to deserialize directly
  from the native buffer before releasing).

### JSON configuration schema

The JSON schema is generated from `GrpcChannelConfig` + `TransportConfig` + `CallStartOptions`.
It is the source of truth for:
- C# options (generated from the schema)
- Options documentation
- Rust-side validation at channel creation and at the start of each call

Note: `RetryConfig` appears both in `GrpcChannelConfig` (channel default) and in
`CallStartOptions` (per-call override, post-V1). The schema covers both usages so that the
generated C# type is reusable in both contexts.

The schema is committed at `packages/rust/armonik-grpc-channel-ffi/include/channel_config.schema.json`.

### FFI entry points (complete V1 list)

```c
// === Runtime lifecycle ===

// Creates a runtime. Synchronous. The runtime transitions to RUNNING.
// callback + runtime_ctx remain valid until AK_EVENT_SHUTDOWN_COMPLETE.
ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_runtime_handle *out);

// Returns the current state of the runtime. Synchronous, non-blocking, thread-safe.
// The handle remains valid for this call even after RELEASED — this is the
// unload condition. The host polls until AK_RUNTIME_RELEASED before unloading the DLL.
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);

// Triggers shutdown. Closes the start gate, drains/cancels calls.
// The terminal AK_EVENT_SHUTDOWN_COMPLETE arrives via the callback.
// Idempotent — a second call is a no-op.
ak_status ak_runtime_begin_shutdown(ak_runtime_handle runtime);

// === Channel ===

// Creates a channel from a config JSON. Synchronous (no I/O unless eager).
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
// If start fails (return != AK_OK), no callback will be emitted for this call_ctx.
ak_status ak_call_start(ak_channel_handle channel,
                        const ak_call_start_options *options,
                        void *call_ctx,
                        ak_call_handle *out);

// Sends a message. Rust reads directly the bytes pointed to by message.
// The host MUST keep the pointed buffer valid (pinned) until reception of the
// AK_EVENT_WRITE_DONE callback for this call. Only one send in flight per call at a time:
// the host must not call send_message again before receiving WRITE_DONE.
// The terminal STATUS may arrive instead of WRITE_DONE (error/cancel).
// WRITE_DONE says both that the buffer is released and that the slot is free. A retryable
// call is copied into the replay buffer first, which is invisible here.
ak_status ak_call_send_message(ak_call_handle call, ak_bytes_in message);

// Signals end of sending (END_STREAM). No more send_message after this.
ak_status ak_call_end_send(ak_call_handle call);

// Requests the next message. Produces a MESSAGE or STATUS callback.
// Only one can be in flight at a time.
ak_status ak_call_request_next_message(ak_call_handle call);

// Cancels the call. Produces a STATUS callback with code CANCELLED.
// Idempotent — a second call is a no-op.
ak_status ak_call_cancel(ak_call_handle call);

// Frees the handle. Cancels the call if not already terminated.
// The terminal callback still arrives after release.
void ak_call_release(ak_call_handle call);

// === Utilities ===

// ABI version. To compare with AK_ABI_VERSION compiled into the binding.
int ak_abi_version(void);

// Signals that the host has consumed the payload of an event. Dual semantics:
// 1. Frees the native memory (Rust deallocs the buffer)
// 2. Arms reception of the next event for this call (demand signal)
// Only one non-consumed event per call at a time — as long as the host has not
// called ak_event_consumed, the runtime does not deliver the next one.
void ak_event_consumed(ak_bytes payload);
```

### Detailed ABI surface

```c
// === Opaque types ===
// Lifecycle: created by ak_runtime_create, freed implicitly at AK_RELEASED.
typedef struct ak_runtime_s *ak_runtime_handle;

// Lifecycle: created by ak_channel_create, freed by ak_channel_release.
// In-progress calls are cancelled. Internal resources survive
// until call quiescence then are freed by the runtime.
typedef struct ak_channel_s *ak_channel_handle;

// Lifecycle: created by ak_call_start, freed by ak_call_release.
// Release requests cancellation but the terminal callback still arrives
// — the handle is invalid for new downcalls after release.
typedef struct ak_call_s    *ak_call_handle;

// Token chosen by the host, passed to ak_call_start, returned in each callback
// for this call. It is an opaque void* — Rust never dereferences it.
// The host puts whatever it wants there: GCHandle (.NET), GlobalRef (Java), id (Python).
// No native lifecycle — the host manages the pointed object.
// Must remain valid until reception of the terminal (AK_EVENT_STATUS) for the call.
typedef void *ak_call_ctx;

// === Buffers ===

// Borrowed by Rust during the downcall only. The host remains owner.
// No native lifecycle — the host manages the pointed memory.
typedef struct {
    const uint8_t *ptr;
    size_t len;
} ak_bytes_in;

// Owned by the host after reception. The host MUST call ak_event_consumed
// exactly once when it has finished consuming the data.
//
// owner: opaque handle to the underlying Rust allocation. The ptr/len
// is a read-only view on bytes that may be a subset of a larger allocation
// (e.g., an Arc<Vec<u8>>). It is owner that identifies what to free —
// ptr alone is not enough because it may point into the middle of a
// reference-counted allocation. The host passes owner unchanged to
// ak_event_consumed.
typedef struct {
    const uint8_t *ptr;     // read-only view
    size_t len;             // number of readable bytes at ptr
    void *owner;            // opaque — passed as-is to ak_bytes_release
} ak_bytes;

// === Events ===
typedef enum {
    AK_RUNTIME_RUNNING           = 1,  // operational, accepts channels and calls
    AK_RUNTIME_STOPPING          = 2,  // start gate closed, channels closing
    AK_RUNTIME_DRAINING          = 3,  // awaiting quiescence of tasks and callbacks (FFI level 1 refinement)
    AK_RUNTIME_RELEASED          = 4,  // quiescent, unload authorized
    AK_RUNTIME_FAILED_UNQUIESCED = 5,  // quiescence impossible, unload forbidden
} ak_runtime_state;
// NOT_INITIALIZED is not an observable state: before a successful ak_runtime_create,
// the host has no handle. After RELEASED, the handle is only valid for
// ak_runtime_status (which continues to return RELEASED).

typedef enum {
    AK_EVENT_INITIAL_METADATA  = 1,  // payload = metadata blob (owned)
    AK_EVENT_MESSAGE           = 2,  // payload = message bytes (owned)
    AK_EVENT_STATUS            = 3,  // terminal — payload = status + trailing metadata (owned)
    AK_EVENT_SHUTDOWN_COMPLETE = 4,  // runtime terminal
    AK_EVENT_WRITE_DONE        = 5,  // the send buffer has been consumed by the network, host can unpin
} ak_event_kind;

// Passed on the stack in the callback — no own lifecycle.
// The payload field is owned and must be released by the host.
typedef struct {
    ak_event_kind kind;
    ak_bytes payload;               // owned — host must call ak_event_consumed
    int32_t status_code;            // grpc status (relevant only for AK_EVENT_STATUS)
} ak_event;

// === Callback ===
// Lifecycle of the function pointer: must remain valid for the runtime's lifetime.
// Lifecycle of runtime_ctx: managed by the host (GCHandle), must remain valid until
// reception of AK_EVENT_SHUTDOWN_COMPLETE.
typedef void (*ak_callback)(
    void *runtime_ctx,
    void *call_ctx,
    const ak_event *event);
// The callback receives an event whose payload is owned.
// The host MUST call ak_event_consumed on event->payload
// after consuming the data. This call frees the memory AND arms the next one.
// The callback is serialized per call, concurrent between calls.
```

### Unary call sequence

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
callState = new CallState(...)
gcHandle = GCHandle.Alloc(callState)
ak_call_start(channel, opts,       → validates channel, creates GrpcCall,
              gcHandle, &handle)      registers in task group
                                     returns handle
pin(msg_buffer)                    // pin the managed buffer
ak_call_send_message(handle, msg)  → Rust reads msg in place, and copies it into the
                                     replay buffer while the call is still retryable
                                   ... network: Rust sends over HTTP/2 ...
                          callback(runtime_ctx, gcHandle, &evt_w) ←
                            evt_w.kind = WRITE_DONE     [buffer released, slot free]
unpin(msg_buffer)                  // safe: Rust has finished reading
ak_call_end_send(handle)           → signal end_send
                                   ... network ...
                          callback(runtime_ctx, gcHandle, &evt1) ←
                            evt1.kind = INITIAL_METADATA  [auto, before any message]
                            evt1.payload = ak_bytes{ptr, len, owner}
ak_event_consumed(evt1.payload)    // free + arm next
                          callback(runtime_ctx, gcHandle, &evt2) ←
                            evt2.kind = MESSAGE
                            evt2.payload = ak_bytes{ptr, len, owner}
// host can deserialize directly from evt2.payload.ptr (zero-copy recv)
ak_event_consumed(evt2.payload)    // free + arm next
                          callback(runtime_ctx, gcHandle, &evt3) ←
                            evt3.kind = STATUS  [terminal, end of stream]
                            evt3.payload = ak_bytes{ptr, len, owner}
                            evt3.status_code = 0 (OK)
ak_event_consumed(evt3.payload)    // free (no next, this is the terminal)
ak_call_release(handle)
gcHandle.Free()                    // safe because terminal received
```

FFI note:
- **Send**: the buffer passed to `send_message` is read directly by Rust. The host MUST keep
  it pinned/valid until `AK_EVENT_WRITE_DONE`. Only one send in flight per call. The terminal
  `AK_EVENT_STATUS` may arrive instead of WRITE_DONE (error/cancel — the host can then unpin).
  Whether Rust took a copy before answering depends on whether the call is still retryable, and
  the host cannot tell: one event, one moment to unpin, either way. See Zero-copy below.
- **Receive (demand via consumed)**: the `ak_bytes` payload is owned. The host consumes
  (deserializes directly from the native pointer) then calls `ak_event_consumed`. This
  call frees the memory AND arms reception of the next event. Only one non-consumed event
  per call at a time — this is the backpressure mechanism.
The terminal `AK_EVENT_STATUS` may arrive instead of a requested message (end of stream or error).

### Shutdown sequence

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
ak_runtime_begin_shutdown(rt)      → closes the start gate
                                     cancels/drains calls
                                     awaits task quiescence
                                     joins the Tokio executor
                          callback(ctx, 0, SHUTDOWN_COMPLETE) ←  [last callback]
                                     thread exits the trampoline
                                     transitions to AK_RELEASED
loop:
  state = ak_runtime_status(rt)
  if state == AK_RELEASED: break
  yield/spinwait
// Safe unload
```

### Zero-copy (integrated from V1)

**Send (host → Rust)**:
- `ak_call_send_message(handle, ak_bytes_in)` — Rust reads the host buffer directly
- The host pins the buffer before the send and unpins after receiving `AK_EVENT_WRITE_DONE`
- Only one send in flight per call (natural backpressure via HTTP/2 flow control)
- On the .NET side: `GCHandle.Alloc(buffer, GCHandleType.Pinned)` or POH (.NET 5+)

`AK_EVENT_WRITE_DONE` carries two meanings at once: Rust no longer reads the host buffer, and
the send slot is free for the next message. They are one event on purpose, so a binding has one
rule to follow and one moment to unpin.

That is what decides where the replay buffer's copy happens. A retryable call has to be able to
send its messages again, so something must hold them, and it cannot be the host buffer: the host
unpins at `WRITE_DONE`, long before the call commits. Deferring `WRITE_DONE` until the message
leaves the replay window would keep the send zero-copy, but it would either stall the stream on
the retry window or split the event in two, and the host would then hold up to
`RetryConfig::max_buffer_size` of pinned bytes per call. On .NET Framework, which has no pinned
object heap, that trades one copy for lasting fragmentation of the collected generations.

So while a call is still retryable, Rust copies the message into the replay buffer and emits
`WRITE_DONE` as soon as the copy is taken. Once the call is committed, no copy is needed and
`WRITE_DONE` follows the network.

The commitment rule is what makes that cheap. A call stays retryable only while what it has sent
fits in `max_buffer_size`, so a message small enough to be replayed is small enough to copy, and
a message that exceeds the buffer commits the call and is read in place. Zero-copy therefore
survives exactly where it pays, on the large payloads, and the copy is bounded by a configured
size rather than by message volume. With `max_buffer_size = 0` nothing is retryable past the
first message and nothing is ever copied.

**Receive (Rust → host)**:
- `ak_event.payload` is an owned `ak_bytes` (reference-counted Rust buffer)
- The host can deserialize directly from `payload.ptr` via `Span<byte>` or unsafe
- The host calls `ak_event_consumed` when done — Rust deallocs + arms the next one
- No copy if the host consumes the native pointer directly

**Memory fragmentation**:
- Send side: Rust allocates only what the replay buffer holds, never more than
  `max_buffer_size` per retryable call, and nothing at all once a call is committed. Because
  those copies are all bounded by one configured size, they are a natural fit for a pool of
  reusable buffers held by the channel: a replayed message is released as soon as the call
  commits, so the same buffers serve every call the channel carries and the steady state costs
  no allocation.
- Receive side: Rust buffers are allocated by Hyper (similar size classes,
  well-managed by jemalloc/system allocator). If fragmentation is measured in production,
  a pool of pre-allocated buffers can be added without ABI change.
- Neither pool is visible across the ABI, so either can be added or removed later without
  touching a binding.

**Open: `max_buffer_size` bounds a call, not a process.** The option is configured per channel
and consumed per call, so what a channel holds is that size times the number of retryable calls
it carries at once, and nothing bounds the product. A process with several channels multiplies it
again, and the post-V1 per-call retry override would let one call set its own size, so the bound
cannot simply be read off one configuration value. V1 states the limit as per-call and accepts
it. A global ceiling is wanted, and the shape it should take is undecided: a byte budget the
runtime hands out and refuses when exhausted, admission control that makes a call
non-retryable rather than refusing it, or a pool whose exhaustion is itself the ceiling. What
happens when the budget is gone is the part that decides the rest, because refusing a send and
silently committing a call are very different contracts. See T8.1.

---

## Layer 4 — `ArmoniK.Api.Client.RustGrpcChannel`

### Internal architecture

```text
┌──────────────────────────────────────────┐
│  NativeCallInvoker : CallInvoker         │
│    ├── NativeRuntime (SafeHandle)        │
│    ├── NativeChannel (SafeHandle)        │
│    ├── Trampoline (static delegate)      │
│    ├── HostQueue (ConcurrentQueue)       │
│    └── Dispatcher (background task)      │
└──────────────────────────────────────────┘
```

### Trampoline

```csharp
// Static delegate rooted for the runtime's lifetime.
// Executes on a Tokio thread — MUST be minimal.
private static unsafe void OnEvent(void* runtimeCtx, void* callCtx, ak_event* evt)
{
    // 1. Cast callCtx → GCHandle → CallState
    // 2. Build a CompletionRecord (callState + kind + owned payload ref)
    // 3. Enqueue into HostQueue
    // 4. Signal (ManualResetEventSlim or SemaphoreSlim)
    // 5. Return — NO user code, NO exception
}
```

The trampoline does NOT copy the payload in V1. It stores the `ak_bytes` reference (owned) in
the CompletionRecord. The copy (or direct deserialization) happens in the Dispatcher, on a
managed thread. The release (`ak_event_consumed`) happens after consumption.

No more registry: the `call_ctx` is directly a `GCHandle` to the call's `CallState`,
allocated before `ak_call_start` and freed after receiving the terminal.

### CallState (per call)

```csharp
// Allocated and GCHandle.Alloc'd BEFORE ak_call_start.
// The GCHandle is passed as call_ctx. Freed after the terminal.
class CallState
{
    TaskCompletionSource<Metadata> InitialMetadataTcs;  // RunContinuationsAsynchronously
    Channel<OwnedMessage> MessageChannel;               // demand-driven via ak_call_request_next_message
    TaskCompletionSource<GrpcStatus> StatusTcs;         // terminal
    CancellationTokenRegistration CancelRegistration;
    GCHandle SelfHandle;                                // the GCHandle passed as call_ctx
}
```

### Dispatcher

The dispatcher is a background task that drains the HostQueue:

```text
while (queue.TryDequeue(out record) || await signal):
    state = registry.Lookup(record.Token)
    switch record.Kind:
        INITIAL_METADATA → state.InitialMetadataTcs.SetResult(decode(record.Payload))
                           ak_event_consumed(record.Payload)
        MESSAGE          → state.MessageChannel.Write(record.Payload)  // consumed after deserialization
        STATUS           → state.StatusTcs.SetResult(decode(record.Payload))
                           ak_event_consumed(record.Payload)
                           registry.Remove(record.Token)
```

### NativeCallInvoker — CallInvoker mapping

The 5 `CallInvoker` methods translate as follows:

| CallInvoker method | Implementation |
|--------------------|----------------|
| `BlockingUnaryCall` | start + send + end_send + await status (blocks the thread) |
| `AsyncUnaryCall` | start + send + end_send + return Task wrapper |
| `AsyncClientStreamingCall` | start + expose write stream + return Task<response> |
| `AsyncServerStreamingCall` | start + send + end_send + expose read stream |
| `AsyncDuplexStreamingCall` | start + expose write stream + expose read stream |

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
channel_config.schema.json  ← committed, source of truth
    │ generation tool (NJsonSchema, or custom)
    ▼
RustChannelOptions.g.cs     ← generated, C# types to configure the channel
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
using var invoker = new NativeCallInvoker(options);
var client = new Sessions.SessionsClient(invoker);
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
Level 0 — Abstract spec (what the user observes)
    AbstractGrpc.tla

Level 1 — FFI spec (what happens at the C boundary)
    FfiGrpc.tla  refines  AbstractGrpc

Level 2 — .NET binding spec (what happens on the managed side)
    DotNetBinding.tla  refines  FfiGrpc
```

### Level 0 — AbstractGrpc

State variables:
- `runtime_state`: NOT_INIT | RUNNING | STOPPING | RELEASED | FAILED_UNQUIESCED
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

**Event sequencing per call:**
- **MetadataFirst**: ∀ call: the first event in `events_delivered` is INITIAL_METADATA (if the call receives events)
- **StatusLast**: ∀ call: STATUS is the last event (terminal), it arrives exactly once
- **NoEventAfterStatus**: ∀ call: no event after STATUS

**Message integrity (prefix invariants, liveness of equality):**
- **SubmittedPrefixOfSent**: ∀ call, at all times: `sent` is a prefix of `submitted`
- **ReceivedPrefixOfDelivered**: ∀ call, at all times: `delivered` is a prefix of `received`
- **SentEqualsSubmitted**: ∀ call terminated successfully: `sent` = `submitted` (liveness)
- **DeliveredEqualsReceived**: ∀ call terminated successfully: `delivered` = `received` (liveness)
- **SubmitProgress**: ∀ message m ∈ `submitted`: ◇ (m ∈ `sent` ∨ call terminated/cancelled)
- **DeliveryProgress**: ∀ message m ∈ `received`: ◇ (m ∈ `delivered` ∨ call terminated/cancelled)

**Uniqueness and termination:**
- **UniqueTerminal**: ∀ call: |{evt ∈ events | evt.kind = STATUS}| ≤ 1
- **SendAfterEndSend**: ∀ call: send_closed[call] ⇒ ¬∃ future send_message(call)

**Runtime lifecycle — monotone transitions:**
- **SingleRuntime**: at most one runtime with state ∈ {RUNNING, STOPPING} at all times
- **MonotoneRuntime**: transitions follow NOT_INIT → RUNNING → STOPPING → RELEASED (no going back). STOPPING → FAILED_UNQUIESCED is also allowed.
- **ReleasedTerminal**: runtime_state = RELEASED ⇒ no future transition
- **FailedTerminal**: runtime_state = FAILED_UNQUIESCED ⇒ no future transition

**Ownership — everything belongs to a runtime:**
- **ChannelOwnership**: ∀ channel: ∃! runtime such that channel ∈ runtime.channels
- **CallOwnership**: ∀ call: ∃! channel such that call ∈ channel.calls (and therefore ∃! runtime)
- **NoOrphan**: no channel or call exists outside a runtime

**Channel ↔ runtime link:**
- **CreateRequiresRunning**: a channel can only be created if runtime_state = RUNNING
- **StoppingClosesChannels**: runtime_state = STOPPING ⇒ ∀ channel: channel.state ∈ {closing, closed}
- **ReleasedNoChannels**: runtime_state = RELEASED ⇒ ∀ channel: channel.state = closed

**Call ↔ runtime link:**
- **StartRequiresOpenChannel**: a call can only be started if channel.state = open (and therefore runtime = RUNNING)
- **StoppingTerminatesCalls**: runtime_state = STOPPING ⇒ ∀ active call: ◇ STATUS delivered
- **ReleasedNoCalls**: runtime_state = RELEASED ⇒ no active call, no callback in flight

#### Liveness (conditional on fairness)

- **EventualTerminal**: 
  ∀ call started: ◇ STATUS delivered
  (under: scheduler fairness, network progresses, client and server
  each produce a finite number of messages)
- **EventualShutdown**: 
  runtime_state = STOPPING ⇒ ◇ runtime_state = RELEASED
  (under: consumer drains, callbacks return, peer responds or timeout)
- **EventualMessage**: 
  request_next_message called ∧ server still has messages ⇒ ◇ MESSAGE delivered
  (under: network progresses)

### Level 1 — FfiGrpc

Added variables:
- `handles`: registry (id → resource, generation)
- `callbacks_in_flight`: counter per call
- `start_gate`: open | closed
- Per call, 2 intermediate message states added to Level 0 sequences:
  - `at_ffi_boundary_send`: messages accepted by the FFI but not yet passed to the Rust channel
    (between `submitted` and `sent`)
  - `at_ffi_boundary_recv`: messages received from the Rust channel but not yet delivered to the host
    via callback (between `received` and `delivered`)

Additional invariants:
- **HandleValidity**: every accepted downcall uses a valid handle in the registry
- **BorrowedLifetime**: send buffer not read by Rust after emission of WRITE_DONE or STATUS
- **CallbackSerialization**: callbacks_in_flight[call] ≤ 1
- **SingleSendInFlight**: ∀ call: at most one send awaiting WRITE_DONE at a time
- **SingleRecvInFlight**: ∀ call: at most one non-consumed event at a time (the runtime does
    not deliver the next one until the previous is consumed)
- **GateClosed**: start_gate = closed ⇒ ¬∃ new child created
- **ReleasedImpliesQuiescent**: runtime_state = RELEASED ⇒
    callbacks_in_flight = 0 ∧ handles = ∅ ∧ tasks = ∅
- **FfiBoundaryOrder**: ∀ call: `at_ffi_boundary_send` is a suffix of `submitted` and a
    prefix of `sent`; `at_ffi_boundary_recv` is a suffix of `received` and a prefix of
    `delivered`

#### Liveness proof strategy (fairness decomposition)

Level 0 liveness properties (EventualTerminal, EventualShutdown, SubmitProgress, DeliveryProgress)
are **not re-proved** at Level 1. Instead, we prove that Level 1's local fairness assumptions
imply Level 0's fairness assumptions:

```text
Level 0 fairness              Decomposed into Level 1 local fairness
────────────────              ──────────────────────────────────────
"network progresses"       =  "Tokio scheduler fair" ∧ "HTTP/2 connection progresses"
"client produces finite"   =  "host calls send_message a finite number of times"
"server produces finite"   =  "peer sends END_STREAM" ∧ "Tokio reads frames"
"scheduler fairness"       =  "Tokio repolls woken tasks"
                              ∧ "callback trampoline returns"
                              ∧ "host dispatcher routes events"
```

Each local fairness is justifiable by a layer:
- "Tokio repolls" → Tokio guarantee (OS grants CPU)
- "callback returns" → refined at Level 2 (.NET binding: bounded trampoline, no user code)
- "host dispatcher routes" → dedicated managed thread, signaled

The refinement proves:
1. Level 1 safety invariants ⇒ Level 0 safety invariants
2. Level 1 local fairness ⇒ Level 0 fairness
3. Therefore Level 0 liveness is inherited by composition, not re-proved

Refinement mapping to AbstractGrpc:
- An accepted `ak_call_start` ↔ a call started
- `AK_EVENT_MESSAGE` delivered ↔ message_received added
- `AK_EVENT_STATUS` delivered ↔ status set (terminal)
- `ak_call_end_send` ↔ transition to half_closed
- `ak_channel_release` ↔ channel closed

### Level 2 — DotNetBinding

Added variables:
- `call_states`: GCHandle → CallState (allocated before start, freed after terminal)
- `host_queue`: sequence of CompletionRecords
- `tcs_state`: per call, state of each TaskCompletionSource
- `gc_roots`: set of live GCHandles
- `dispose_state`: active | disposing | disposed

Additional invariants:
- **RootSurvivesCallbacks**: ∀ active call: gc_root(runtime_ctx) ∈ gc_roots
- **TokenPublishedBeforeStart**: ∀ call: GCHandle(call_ctx) allocated before ak_call_start
- **ContinuationsAsync**: TCS completed ⇒ non-inline continuation
- **DisposeAwaitsReleased**: dispose completed ⇒ runtime_state = RELEASED

Refinement mapping to FfiGrpc:
- `GCHandle.Alloc(callState)` before start ↔ publication before start
- `OnEvent` trampoline ↔ callback reception
- `Dispatcher` routes ↔ event consumed
- `ak_event_consumed` called ↔ payload released + demand signal
- `DisposeAsync` completed ↔ RELEASED observed

### TLAPS Proof

The proof strategy is:
1. Prove safety invariants of each level independently (induction)
2. Level 0 liveness is proved with that level's fairness assumptions
3. Prove refinement between levels (simulation)
4. If possible, refinement liveness is proved by showing that the current level's fairness + safety implement the upper level's fairness.

Proof files will be in `spec/armonik_grpc_ffi/tla/proofs/`.

---

## Open decisions (to be resolved during implementation)

| Question | Options | Impact |
|----------|---------|--------|
| Crate for X509Store Windows | Direct native APIs / `schannel` crate / `windows` crate | Layer 1 |
| Exact handle format (opaque pointer vs index + generation) | Performance vs safety | Layer 3 |
| Default size of the replay buffer | 0 (no streaming retry) vs 4KB vs 64KB | Layer 2 config. Sets how much a retryable call is copied and how much a pool has to hold: it is the one knob between replayability and copy cost |
| Host queue signal mechanism | `ManualResetEventSlim` vs `SemaphoreSlim` vs custom | Layer 4 perf |
| Source generator vs T4 for C# options | Build tooling | Layer 4 |
| Connection pool management (idle eviction) | Internal timer vs lazy check | Layer 2 |

---

## References

- [gRPC over HTTP/2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
- [gRPC Retry Design](https://github.com/grpc/proposal/blob/master/A6-client-retries.md)
- [tower::Service](https://docs.rs/tower/latest/tower/trait.Service.html)
- [Grpc.Core.CallInvoker](https://grpc.github.io/grpc/csharp-dotnet/api/Grpc.Core.CallInvoker.html)
- [TLA+ Proof System](https://tla.msr-inria.inria.fr/tlaps/content/Home.html)
