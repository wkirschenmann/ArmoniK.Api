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
    pub proxy: ProxyConfig,         // disabled | explicit | system
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

/// hyper's defaults, written out: no PING, 20 s, 2 MiB, 5 MiB, 1 MiB. Beside them,
/// ours: 16 KiB gathered per write.
pub struct Http2Config {
    pub keep_alive_interval: Option<Duration>,
    pub keep_alive_timeout: Duration,
    pub keep_alive_while_idle: bool,
    /// What the peer may send ahead of what is read: `Fixed { stream,
    /// connection }`, what one stream and the whole connection may have
    /// unread - a call its host does not read holds up to its stream window,
    /// and the connection's is at least 65535, since only an increase is
    /// announced - or `Adaptive`, hyper's windows that start at 65535 and
    /// grow with the bandwidth-delay product its PINGs measure, up to 16 MiB.
    pub receive_windows: ReceiveWindows,
    /// How long a session stays open with no call on it before the channel
    /// closes it; none keeps it open. Each session has its own.
    pub idle_timeout: Option<Duration>,
    /// How many calls one session carries at once: a call that finds every
    /// session full opens another, as many as the calls in flight need,
    /// taking the fullest with room first. A call counts from its dispatch to
    /// the end of its response and of its request, so that calls follow one
    /// another on a session but never share it beyond the limit. A session
    /// never carries more than its server's SETTINGS_MAX_CONCURRENT_STREAMS
    /// either, which is the only bound when this is none.
    pub simultaneous_calls_per_connection: Option<usize>,
    /// How many bytes a write to the connection may gather while the work
    /// already ready adds to it; 0 writes at once.
    pub write_coalescing: usize,
    /// How many bytes of one stream's request may be queued in the session,
    /// waiting to be written, before its next part is handed over, whole. At
    /// least 1, and at most u32::MAX: hyper panics past it.
    pub send_buffer: usize,
    /// How many DATA frames of the peer's largest size one queued part of a
    /// request may span, written one after the other in one write. At 1 a
    /// part is one frame, as in stock h2; more needs the engine built against
    /// h2-batch's patch, and is refused otherwise. At most 256.
    pub frames_per_write: usize,
    // Not built: max_frame_size, and the advertised SETTINGS_MAX_CONCURRENT_STREAMS,
    // which bounds the streams the *peer* may open (RFC 9113 s5.1.2) - for a client,
    // server pushes. It is not a cap on outgoing calls; simultaneous_calls_per_connection
    // and the server's own SETTINGS_MAX_CONCURRENT_STREAMS bound those per session.
}

pub struct ProxyConfig {
    pub source: ProxySource,
    /// `Basic` credentials, empty when unset. Beside the environment's proxy,
    /// each half set here takes the place of the one its URL carries; beside
    /// the one Windows' settings name, they are its credentials.
    pub username: String,
    pub password: SecretString,
}

pub enum ProxySource {
    /// No proxy, unconditionally: a direct connection whatever NO_PROXY says.
    /// The engine's default; the options' is `System`. NO_PROXY is consulted
    /// by `System`, which is where it belongs: making it apply here too would
    /// mean "no proxy" sometimes yields to configuration.
    Disabled,
    /// An `http://` URI without userinfo: the options move credentials written
    /// in the URL into the fields above.
    Explicit(Uri),
    /// ALL_PROXY, HTTPS_PROXY, HTTP_PROXY and NO_PROXY, read when the channel
    /// is created; a loopback endpoint is dialled directly. On Windows, the
    /// user's network settings when the environment names no proxy, a PAC
    /// script resolved by WinHTTP on a blocking thread for each dial.
    System,
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
    /// How many requests start in a window of time, a retry's included; a
    /// request over it waits for the next window, except a retry the policy
    /// chose, which is skipped to its next backoff and counts as an attempt.
    /// None starts them as made.
    pub rate_limit: Option<RateLimitConfig>,
    pub default_deadline: Option<Duration>,
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
    /// What a call's messages are compressed with, `grpc-encoding`. Default
    /// None: they go as they are. A message that would not be smaller goes out
    /// flagged uncompressed. The send limit is on the message as written;
    /// the receive limit bounds the frame as it arrives and the message once
    /// it is inflated. The channel reads `grpc-accept-encoding` on every
    /// response head of a gRPC response, a Trailers-Only one included; a head
    /// without it, or with it blank, says nothing. While the last head that
    /// has it leaves this encoding out, calls that start send as they are, and
    /// the channel logs one warning in all (`tracing`, target
    /// `armonik_transport`). A later head that lists it has calls compress
    /// again, logged at debug as a further refusal is. Behind a balancer whose
    /// backends differ, the state follows whichever answered last. A call
    /// chooses its encoding when its first attempt has taken its turn at the
    /// rate limit, and compresses nothing before: one that waits for its turn
    /// and is ended there has compressed nothing. It keeps the encoding it
    /// chose, so its messages and its `grpc-encoding` agree through a retry;
    /// one sent compressed to a server
    /// that does not accept it ends UNIMPLEMENTED and is not sent again. A
    /// server that states no `grpc-accept-encoding` is not learned from.
    pub send_encoding: Option<Encoding>,
    /// The encodings besides identity the channel accepts in an answer, named
    /// in `grpc-accept-encoding` in this order with identity last; a repeated
    /// one counts at its first place, and a message compressed in any other
    /// ends INTERNAL. Default empty.
    pub accept_encodings: Vec<Encoding>,
    // No eager_connect flag: connecting is GrpcChannel::connect().await. The
    // option document's Transport.ConnectEagerly is the FFI's, which calls connect()
    // once ak_channel_create has registered the channel.
}

/// A message encoding, by its gRPC name: `gzip` (RFC 1952), `deflate` (the
/// zlib structure of RFC 1950, as gRPC means it) and `zstd` (RFC 8878).
#[non_exhaustive]
pub enum Encoding { Gzip, Deflate, Zstd }

/// At most `calls` requests start in a window of `per`; a request over it
/// waits for the next window, except a retry the policy chose, which is
/// skipped. The windows are fixed, so up to twice `calls` can start within `per`
/// across a boundary. Refused when the channel is created if `calls` is 0 or
/// `per` is zero.
pub struct RateLimitConfig {
    pub calls: usize,
    pub per: Duration,
}

pub struct RetryConfig {
    pub max_attempts: u32,          // total (initial + retries)
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
    pub retryable_codes: Vec<GrpcStatusCode>, // default: UNAVAILABLE alone; RetryConfig::grpc_client() has the three of GrpcClient
    pub call_replay_bytes: usize,    // what one call keeps for a replay, whatever it sends
    pub channel_replay_bytes: usize, // what every call of the channel keeps together
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
    /// the next must wait for a WRITE_DONE to free a slot. A call that sends one request has
    /// no SendHalf: `prepare_one_request_call` gives it a `OneRequest`, which takes the request
    /// framed in place, once, and settles it there.
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
    pub deadline: Option<Deadline>, // replaces the channel default; sent as grpc-timeout
    pub one_response: bool,         // the response is at most one message
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

Note: a call may declare at start that its response is at most one message, and the engine
then refuses a second and reads the status past the runtime's first threshold. Otherwise the
cardinality is implicit in the call usage (a unary does send_message + end_send + next_message +
status; a server streaming does send_message + end_send + next_message in a loop). The channel
does not need to know it to drive the HTTP/2 connection.

### Retry — commitment point

A call is retryable while **both** hold: no response header has been seen, and what it has
sent still fits the replay buffer - its own ceiling, and the channel's total, which its next
message would not pass. Committing on either alone is wrong - the table below reads as if one
sufficed, which is why each row names both. A committed call goes on; it is no longer retried.

| Situation | Retryable? |
|-----------|------------|
| Unary: error before Response-Headers, request within budget | Yes |
| Client streaming: error before Response-Headers, data sent ≤ budget | Yes (replay) |
| Client streaming: data sent > budget | No (committed) |
| Any call whose next message would pass the channel's replay total | No (committed) |
| Bidi: error before Response-Headers, data ≤ budget | No response seen, so yes |
| Bidi: Response-Headers or a message received | No (committed) |
| Server streaming: error before Response-Headers | Yes |
| Remaining deadline < backoff | No |
| Refused, or past a GOAWAY's last stream, with every message kept | Yes, at once, once a call, and not as an attempt |
| Never sent, its connection closing under it, with every message kept | Yes, at once, once a call, and not as an attempt |

**Beyond the commitment point.** The backoff is drawn uniformly below its bound, the server's
`grpc-retry-pushback-ms` replaces it or refuses the retry, and each attempt carries
`grpc-previous-rpc-attempts`, as gRFC A6 has them. A stream the peer's HTTP/2 layer refused with
`REFUSED_STREAM`, or that its GOAWAY left unprocessed, goes again at once, whatever the policy:
it is A6's transparent retry, once a call, counted as no attempt and in no
`grpc-previous-rpc-attempts`. A request hyper drops before sending it, its connection closing
under it, goes again the same way, also once a call: A6 allows until the deadline, which a call
with none would turn into a loop of dials. Both replay what the call kept, so a call with no policy,
which keeps none, goes again only if it had sent nothing. Not specified yet: the per-channel
retry throttle, which A6 makes optional. The policy is the channel's for every
method, as `GrpcClient` configures it, where gRPC would allow one per method; its codes are
`UNAVAILABLE` unless `Grpc.Retry.Adaptive.Codes` says otherwise, and `GrpcClient`'s three are its `GrpcClient`
preset.

**Where an attempt ended.** Several origins share one code: `UNAVAILABLE` is the server's own, a
proxy's 503, a dial that failed, a refused stream and a GOAWAY. So each attempt that goes out and
fails carries its `Origin` beside its status (see its documentation), and the `Pushback` its server
stated, which is read after the head too. A stream a GOAWAY named as processed ends with the
connection's own error, and its origin is the connection's. A caller sees nothing of it: the
origin is logged at `debug` and told to a test by `hooks::on_attempt`.

---

## What is missing

**Protocol surface not yet contractualized.** Message and metadata size limits and what a
violation produces on each side; `-bin`
metadata keys and their base64 encoding; `grpc-message` percent-encoding; GOAWAY handling
beyond the code of a stream it ends, and stream re-attempt; and the Trailers-Only response,
which the ABI normalizes but whose status mapping is not written down. Each is a place where
two implementations would diverge silently.
