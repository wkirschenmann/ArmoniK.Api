# Contract — .NET ArmoniK Client on Native Rust gRPC Channel

What layers 1 and 2 promise the code that uses them. The C ABI built on them is
[abi.md](abi.md); how each promise is kept is [architecture.md](architecture.md). The
numbering of layers and levels is the one [design.md](design.md) states.

## Layer 1 — `armonik-transport`, module `http2`

Layers 1 and 2 are two contracts carried by one crate: `armonik-transport` holds
`http2` for the connector and `grpc` for the gRPC engine.  The engine needs the
connector and nothing else needs the engine, so a crate boundary between them would
carry no dependency the module boundary does not.  The numbers stay because the two
contracts stay.

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

### TransportConfig

```rust
pub struct TransportConfig {
    pub endpoint: Uri,              // http:// in the clear, https:// over TLS
    pub connect_timeout: Duration,
    pub tls: TlsConfig,             // read for https://, refused on http:// unless default
    pub tcp: TcpConfig,             // the socket's keepalive
    pub http2: Http2Config,         // the session's PING keepalive and windows
    pub proxy: ProxyConfig,         // disabled | explicit; env is T5.2's, windows_system T5.3's
}

/// Loaded material, which a handshake uses as it stands. Built from the paths a
/// document names by `options::TlsOptions::load`, which reads the files - a PEM
/// pair or a PKCS#12 bundle for the identity - or, on Windows, resolves them from
/// a certificate store.
pub struct TlsConfig {
    pub roots: Vec<CertificateDer<'static>>, // empty: the system's
    pub accept_any_server: bool,            // verifies nothing (opt-in)
    pub identity: Option<ClientIdentity>,   // for mTLS
    pub server_name: Option<String>,        // verified and sent as SNI instead of the host
}

pub struct ClientIdentity {
    pub chain: Vec<CertificateDer<'static>>, // the leaf first, then its issuers
    pub key: PrivateKeyDer<'static>,         // never printed
}

/// Off unless keepalive is set. Whole seconds, which is what the socket holds.
pub struct TcpConfig {
    pub keepalive: Option<Duration>,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_retries: Option<u32>,      // not applied on Windows
}

/// hyper's defaults, written out: no PING, 20 s, 2 MiB, 5 MiB.
pub struct Http2Config {
    pub keep_alive_interval: Option<Duration>,
    pub keep_alive_timeout: Duration,
    pub keep_alive_while_idle: bool,
    /// What one stream may have unread.
    pub stream_window: u32,
    /// What the connection may have unread, shared by every stream of the
    /// channel: a call its host does not read holds up to its stream window
    /// of it. At least 65535, since only an increase is announced.
    pub connection_window: u32,
    // Not built: max_frame_size, and the advertised SETTINGS_MAX_CONCURRENT_STREAMS,
    // which bounds the streams the *peer* may open (RFC 9113 s5.1.2) - for a client,
    // server pushes. It is not a cap on outgoing calls; that one is
    // max_calls_in_flight, and it lives in PoolConfig.
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
    // Resolution and connection are one outcome: the dial is `hyper_util`'s `HttpConnector`,
    // which races address families rather than walking them in turn.
    Connect,
    TlsHandshake,
    /// The stream was connected and no HTTP/2 session could be established on
    /// it. The module is named for that protocol and owns the handshake, so
    /// the failure is named here rather than in the gRPC engine above.
    Http2Handshake,
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

## Layer 2 — `armonik-transport`, module `grpc`

### GrpcChannelConfig

```rust
pub struct GrpcChannelConfig {
    pub transport: TransportConfig,
    pub retry: Option<RetryConfig>,
    pub default_deadline: Option<Duration>,
    pub pool: Option<PoolConfig>,
    pub user_agent: Option<String>,
    /// The largest message the engine reassembles, refused on the length the
    /// peer announces. It is what bounds the reassembly buffer: the HTTP/2
    /// window bounds what is in flight and is released as each frame is
    /// taken, so a message that never completes would grow that buffer
    /// without the window ever being exceeded. 4 MiB, as gRPC has it.
    pub max_recv_message_size: usize,
    /// How many buffers one call may have out at once before a send waits.
    /// Default 1.
    pub max_sends_in_flight: usize,
    // No eager_connect flag: connecting is GrpcChannel::connect().await.
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
    /// The error is the engine's own and not `ConfigError`, which belongs to the
    /// environment-driven client: a caller reading a refused window has no reason
    /// to meet a vocabulary about `GrpcClient__CertPem` and PEM parsing, and it was
    /// the one edge tying `grpc` to `config`.
    pub fn new(
        config: GrpcChannelConfig,
        spawner: tokio::runtime::Handle,
    ) -> Result<Self, GrpcChannelConfigError>;

    /// Establishes the connection and reports how it went. Optional: the first
    /// call connects lazily otherwise. This replaces the eager_connect flag,
    /// which asked a synchronous constructor to start asynchronous work and
    /// had nowhere to report its failure. The error is ChannelError rather
    /// than TransportError because a closed channel opens no connection, and
    /// that is an outcome of this call rather than of the network.
    pub async fn connect(&self) -> Result<(), ChannelError>;

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
// neither can two readers. This is the Rust-side twin of what level 2 holds
// by construction: one reader_state and one writer_state per call.
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
    /// Retrieves the response head: the initial metadata (HTTP/2 headers
    /// from the response) and where it came from - the peer's headers, a
    /// response that delivered none (Trailers-Only), or no response at all.
    /// Blocks until reception or terminal. Without headers from the peer the
    /// metadata is empty rather than an error - see the normalization note
    /// in abi.md.
    pub async fn recv_head(&mut self) -> Result<&ResponseHead, CallError>;

    /// Retrieves the next message or the terminal status.
    /// Each call implicitly constitutes a request for one message (natural backpressure).
    /// Returns End(GrpcStatus) when the stream is finished - this is the call's terminal.
    pub async fn next_message(&mut self) -> Result<RecvResult, CallError>;
}

/// A call's response head, first on every call.
pub struct ResponseHead {
    /// Empty unless the origin is Wire.
    pub metadata: Metadata,
    pub origin: HeadOrigin,
}

/// Where a response head came from.
pub enum HeadOrigin {
    /// The peer's response headers were delivered.
    Wire,
    /// A response arrived and delivered no head - the Trailers-Only shape, or
    /// an answer refused before its body. The status is the call's: the
    /// peer's, unless the call was stopped here first.
    TrailersOnly,
    /// No response reached the call: it failed or was cancelled first.
    NoResponse,
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
would produce a client that retries at the wrong times and hides server pushback. T6.3 and
T6.4 carry it.

---

## What is missing

**Protocol surface not yet contractualized.** Message and metadata size limits and what a
violation produces on each side; gRPC compression (`grpc-encoding`,
`grpc-accept-encoding`, per-message compressed flag); `grpc-timeout` derivation from the
deadline and what happens when both a channel default and a call deadline exist; `-bin`
metadata keys and their base64 encoding; `grpc-message` percent-encoding; GOAWAY handling
beyond the code of a stream it ends, and stream re-attempt; and the Trailers-Only response,
which the ABI normalizes but whose status mapping is not written down. Each is a place where
two implementations would diverge silently.
