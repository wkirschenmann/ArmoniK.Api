# SPEC — ArmoniK .NET Client on Native Rust gRPC Channel

## Objectives

1. Replace the managed gRPC channel (`Grpc.Net.Client` / `WinHttpHandler`) with a common native
   Rust gRPC channel, exposed to .NET via a thin C FFI.
2. **Unify client behavior** regardless of the .NET runtime (Framework 4.7.2, 4.8, .NET 6+,
   .NET 8+). The native transport eliminates behavioral divergences between `WinHttpHandler`,
   `SocketsHttpHandler`, `GrpcWebHandler`, and conditional fallbacks.
3. Make currently inoperative options (`RequestTimeout`, `OverrideTargetName`) effective.
4. Preserve the client's public API: same surface, same generated stubs. Only the underlying
   `CallInvoker` changes.

---

## Layered Architecture

```text
┌─────────────────────────────────────────────────────────┐
│  5. ArmoniK.Api.Client                                  │  C# — netstandard2.0
│     Generated gRPC stubs, ArmoniK services              │
│     Injectable CallInvoker (managed OR native)          │
├─────────────────────────────────────────────────────────┤
│  4. ArmoniK.Api.Client.RustGrpcChannel                  │  C# — netstandard2.0
│     Native CallInvoker, binding runtime,                │  (multitarget if justified)
│     trampoline, host queue, SafeHandles, shutdown       │
├─────────────────────────────────────────────────────────┤
│  3. armonik-grpc-channel-ffi                            │  Rust → DLL/SO C
│     Versioned C ABI, owned Tokio runtime,               │
│     handles, callbacks, registries, shutdown            │
├─────────────────────────────────────────────────────────┤
│  2. armonik-grpc-channel                                │  Rust — lib crate
│     Functional gRPC channel: framing, metadata,         │
│     status, retry, deadline, flow control, 4 cardinal.  │
├─────────────────────────────────────────────────────────┤
│  1. armonik-transport                                   │  Rust — lib crate
│     Network connector: endpoint, TCP, TLS/mTLS,         │
│     proxy, connect timeout, DNS                         │
└─────────────────────────────────────────────────────────┘
```

### .NET Multitargeting

The `ArmoniK.Api.Client.RustGrpcChannel` package targets `netstandard2.0` as its base. Multitargeting
(e.g., `net6.0`, `net8.0`) is permitted if it provides a concrete benefit (e.g., `LibraryImport` to
reduce P/Invoke overhead, `NativeMemory`, span-based APIs). Functional behavior remains identical
regardless of the TFM — that is precisely the unification objective.

---

## Layer 1 — `armonik-transport` (complete)

### Current State

The crate exists and provides:
- Configuration and endpoint/origin resolution
- TCP and socket options
- TLS, mTLS, explicit CA and native roots
- Server-name override (declared but not effective)
- HTTP CONNECT proxy
- Connect timeout
- A `Connector` consumable by Hyper/Tonic

### Missing Features

- **Stable public contract for the `Connector`** — The boundary between what `armonik-transport`
  exposes and what `armonik-grpc-channel` consumes is not formalized. This is the prerequisite
  for everything else: the public trait/type of the connector, its guarantees, its error contract.

- **Extraction of options that don't belong to it** — `RetryConfig`, `Http2Config`, request timeout
  and pool options are currently declared here but applied above. They must migrate to
  `armonik-grpc-channel`. The residual configuration for `armonik-transport` is: endpoint,
  TLS/mTLS, TCP, proxy, connect timeout.

- **Effective `OverrideTargetName`** — The option exists but does not modify the name verified by
  rustls. It must act on the `ServerName` of the TLS handshake.

- **`ProxySource::WindowsSystem`** — Reading the Windows system proxy via WinHTTP (resolver only,
  not network engine). Async resolution with timeout and cancellation; must never block under the
  loader lock or within an FFI callback.

- **Structured network errors** — Error messages must not contain secrets (private key paths,
  passwords) or internal debug information in release mode. Error codes must be programmatically
  discriminable.

---

## Layer 2 — `armonik-grpc-channel` (create)

### Responsibility

Pure Rust functional gRPC engine. No notion of C ABI, callback, or foreign handle. Consumable
directly by a Rust client via a Tonic adapter, and by the FFI layer.

### gRPC Model — Mapping onto HTTP/2

A gRPC call is an HTTP/2 POST request:

```text
HTTP/2 Request:
  :method = POST
  :path = /{service}/{method}
  :scheme = http | https
  content-type = application/grpc
  te = trailers
  grpc-timeout = {deadline}         ← optional, transmitted to server
  {metadata key: value}             ← user metadata = additional HTTP headers
  {metadata key-bin: base64(bytes)} ← binary metadata

  [body] = sequence of framed gRPC messages:
    [1 byte compressed flag][4 bytes big-endian length][message bytes]

HTTP/2 Response:
  :status = 200
  content-type = application/grpc
  {initial metadata}                ← HTTP headers of the response

  [body] = sequence of framed gRPC messages (same format)

  HTTP/2 Trailers (trailing headers):
    grpc-status = {code}
    grpc-message = {message}        ← optional
    {trailing metadata}             ← user metadata
```

**Metadata** = HTTP headers of the request/response. They are passed in the HTTP/2 headers of the
request (for request metadata) and read from the HTTP/2 headers of the response (initial metadata)
and from the HTTP/2 trailers (trailing metadata also containing `grpc-status`).

**Half-close** (`end_send`) = the client signals that it will send no more messages on this call.
Concretely, this is an `END_STREAM` on the HTTP/2 request body. For a unary or client-streaming
call, this is what triggers server-side processing. The term `half_close` comes from the gRPC
protocol; in our API we call it `end_send` for clarity.

### Connection Options

The channel supports:
- **Lazy connection** (default) — the first physical connection is established on the first call
- **Eager connection** (configuration option) — an HTTP/2 connection is established upon channel
  creation, without waiting for the first call

### V1 Features

- Hyper HTTP/2 client/pool, lazy or eager connection (configurable)
- gRPC framing/deframing (length-prefixed messages)
- All 4 cardinalities (unary, client streaming, server streaming, bidi streaming)
- Request metadata → HTTP/2 request headers
- Initial metadata ← HTTP/2 response headers
- Trailing metadata ← HTTP/2 trailers (including `grpc-status` and `grpc-message`)
- gRPC status extracted from trailers
- Flow control (demand-driven reads)
- `end_send` (half-close: END_STREAM on the request body)
- Local cancellation and RST_STREAM propagation
- Local deadline + `grpc-timeout` header transmitted to server
- Retry/backoff (5 attempts, initial 1s, max 5s, multiplier 1.5, codes UNAVAILABLE/ABORTED/UNKNOWN)
- Notion of committed call (no retry after sending data in streaming)
- HTTP/2 pool with keepalive and idle timeout
- HTTP/2 options (max concurrent streams, initial window size, etc.)
- No owned Tokio runtime — uses an `Executor` trait provided by the caller

### Out of V1

- Hedging, load balancing, compression, interceptors, channelz, connectivity state

### Executor Trait

```rust
/// The channel spawns tasks via this executor.
/// In normal Rust: direct Tokio adapter.
/// In FFI: adapter that registers into the FFI runtime's task group.
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskHandle;
}
```

---

## Layer 3 — `armonik-grpc-channel-ffi` (create)

### Responsibility

C projection of layer 2. Owns foreign isolation: Tokio runtime, handles, callbacks, registries,
task tracking, shutdown and safe unload.

### ABI Surface (sketch — will be finalized in DESIGN.MD)

```c
// === Runtime lifecycle ===
ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_runtime_handle *out_runtime);
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);
ak_status ak_runtime_begin_shutdown(ak_runtime_handle runtime);

// === Channel ===
ak_status ak_channel_create(ak_runtime_handle runtime,
                            ak_bytes_in config_json,
                            ak_channel_handle *out_channel);
void ak_channel_release(ak_channel_handle channel);
// Release is synchronous. In-flight calls on this channel receive a
// terminal error (CANCELLED). The channel retains its internal resources
// until its calls reach quiescence, but the handle is no longer usable
// for starting new calls.

// === gRPC Call ===
ak_status ak_call_start(ak_channel_handle channel,
                        const ak_call_start_options *options,
                        ak_operation_token token,
                        ak_call_handle *out_call);
ak_status ak_call_send_message(ak_call_handle call, ak_bytes_in message);
ak_status ak_call_end_send(ak_call_handle call);
ak_status ak_call_request_messages(ak_call_handle call, uint32_t n);
ak_status ak_call_cancel(ak_call_handle call);
void ak_call_release(ak_call_handle call);

// === Utilities ===
int ak_abi_version(void);
void ak_bytes_release(ak_bytes bytes);
```

### Callback (Trampoline Contract)

```c
typedef ak_callback_result (*ak_callback)(
    void *runtime_ctx,
    ak_operation_token token,
    const ak_event *event);  // payload borrowed during the call
```

Events delivered per call via the callback:

| Event | Payload | When |
|-------|---------|------|
| `AK_EVENT_INITIAL_METADATA` | Metadata (HTTP/2 response headers) | Upon receiving response headers |
| `AK_EVENT_MESSAGE` | gRPC message bytes (already deframed) | For each message requested via `request_messages` |
| `AK_EVENT_STATUS` | gRPC status + trailing metadata | **Terminal** — last event of the call |

Runtime events:

| Event | Payload | When |
|-------|---------|------|
| `AK_EVENT_SHUTDOWN_COMPLETE` | Shutdown status | Last callback of the runtime |

`AK_EVENT_STATUS` is the call terminal. It contains the `grpc-status`, the `grpc-message`, and the
trailing metadata (extracted from HTTP/2 trailers). All call information — initial metadata,
messages, status — is delivered via the callback. Messages arrive only in response to
`ak_call_request_messages` (demand-driven).

### Semantics of `ak_channel_release`

- Synchronous and non-blocking
- The handle is no longer valid for `ak_call_start` after the call
- In-flight calls are cancelled (receive an `AK_EVENT_STATUS` with code CANCELLED)
- Internal channel resources are freed after call quiescence
- The runtime tracks the channel until quiescence (no leak)

### Safety Properties

- Opaque handles, validated in a per-runtime registry (generation-tagged)
- `ptr+len` inputs borrowed during the downcall, copied by Rust
- Callbacks serialized per call, concurrent between calls
- A single terminal (`AK_EVENT_STATUS`) per accepted call
- No callback after a call's terminal
- Start gate closed → no new children
- `AK_RELEASED` certifies quiescence (unload permitted)

### Runtime Lifecycle

```text
AK_NOT_INITIALIZED → AK_RUNNING → AK_STOPPING → AK_DRAINING → AK_RELEASED
                                                      ↓
                                          AK_FAILED_UNQUIESCED
```

### Configuration

Channel configuration is passed as UTF-8 JSON. The JSON schema is the single source of truth for
options (shared across all layers). The schema lives in the FFI crate and is generated from the Rust
types of `armonik-grpc-channel` + `armonik-transport`. It is committed in the repo so that bindings
can generate their code.

---

## Layer 4 — `ArmoniK.Api.Client.RustGrpcChannel` (create)

### Responsibility

C# NuGet package that implements `Grpc.Core.CallInvoker` on top of the FFI. Contains the .NET
binding runtime.

### Internal Components

| Component | Role |
|-----------|------|
| `NativeRuntime` | DLL loading, version check, `SafeHandle` on `ak_runtime` |
| `NativeChannel` | `SafeHandle` on `ak_channel`, JSON config |
| `NativeCallInvoker` | Implements `CallInvoker` (5 methods) |
| `Trampoline` | Static rooted delegate, copies payload, enqueues `CompletionRecord` |
| `OperationRegistry` | Token → `CallState`, publication before `ak_call_start` |
| `HostQueue` | `ConcurrentQueue<CompletionRecord>` + signal |
| `Dispatcher` | Dedicated thread/task, routes completions to TCS |
| `ShutdownCoordinator` | `begin_shutdown` → poll `AK_RELEASED` → dispose |

### Configuration

The C# options class (`RustChannelOptions` or equivalent) is **generated from the JSON schema** of
the FFI configuration. This guarantees consistency between C# options and Rust options without
manual maintenance.

### Constraints

- `netstandard2.0` as the base TFM (compatible with Framework 4.7.2+)
- Multitarget `net6.0`/`net8.0` permitted if measurable benefit (e.g., `LibraryImport`)
- `DllImport` with `CallingConvention.Cdecl` on netstandard2.0
- `GCHandle` for `runtime_ctx`, published before any `ak_call_start`
- `TaskCompletionSource` with `RunContinuationsAsynchronously`
- Cancellation via `CancellationToken` → `ak_call_cancel`
- No user code in the trampoline

### Native Packaging

Native binary per RID included in the NuGet package:

| RID | Platform |
|-----|----------|
| `win-x64` | Windows 64-bit |
| `win-x86` | Windows 32-bit |
| `linux-x64` | Linux 64-bit |
| `linux-x86` | Linux 32-bit |
| `linux-arm64` | Linux ARM 64-bit |

Structure: `runtimes/{rid}/native/armonik_grpc_channel_ffi.{dll|so}`

---

## Layer 5 — `ArmoniK.Api.Client` (complete)

### Current State

- Targets `netstandard2.0`
- Uses `Grpc.Net.Client` or `Grpc.Core` depending on platform/runtime
- `GrpcChannelProvider` provides a `ChannelBase`
- Generated stubs call a `CallInvoker`
- Divergent behavior between .NET Framework (WinHttpHandler/GrpcWeb) and modern .NET
  (SocketsHttpHandler)
- `RequestTimeout` does not set any deadline (warning only)
- `OverrideTargetName` has no effect (warning only)

### Missing Features

- **Injectable CallInvoker** — Ability to provide an external `CallInvoker` (the
  `RustCallInvoker`) without going through `ChannelBase`. This is the integration point between
  layers 4 and 5.

- **Native provider** — A `RustGrpcChannelProvider` (or adaptation of the existing
  `GrpcChannelProvider`) that builds the native channel from existing options.

- **Options mapping** — Existing configuration options (`GrpcChannel`, `TlsOptions`, etc.) must be
  able to produce the JSON expected by the FFI. The mapping must be explicit and tested.

- **Effective deadline** — `RequestTimeout` must produce a real deadline transmitted to the server
  via `grpc-timeout` and applied locally.

- **Effective OverrideTargetName** — The option must modify the server name verified by TLS.

- **Clean shutdown/dispose** — `IAsyncDisposable` that triggers the native runtime shutdown and
  waits for `AK_RELEASED` before allowing unload.

- **Opt-in managed fallback** — The native channel is the default when the native DLL is available.
  If it is absent or if the user requests it, fall back to the current managed behavior. This must
  not break existing deployments.

- **Unified behavior** — Regardless of the .NET runtime (4.7.2, 4.8, 6, 8), options produce the
  same network effect, because the transport is identical (native Rust).

---

## TLA+ Formal Model

### Location

The TLA+ specifications live in:

```text
spec/armonik_grpc_ffi/tla/
├── AbstractGrpc.tla          channel, calls, terminal, cancel, retry (functional spec)
├── CallLifecycle.tla         start, send, end_send, recv, terminal, quiescence
├── GrpcFlowControl.tla       message demand, send admission, bounds
├── Retry.tla                 attempts, commitment, backoff, deadline
├── FfiRuntime.tla            gates, children, tasks, callbacks, shutdown
├── GenerationalIds.tla       allocation, validation, tombstones
├── CallbackProtocol.tla      publication, trampoline, result, barrier
├── MemoryOwnership.tla       borrowed, copied, retained, released
├── FailureContainment.tla    fault latch, quarantine, quiescence
├── FfiGrpc.tla               concrete composition FFI + gRPC
├── FfiRefinement.tla         mapping FfiGrpc → AbstractGrpc
├── DotNetBinding.tla         roots, queue, TCS, cancellation, dispose
└── DotNetRefinement.tla      mapping DotNetBinding → FfiGrpc
```

### Role

The TLA+ models verify safety properties (invariants) and conditional liveness (progress under
fairness assumptions). They do not prove performance, code conformance, or termination of arbitrary
foreign code.

The TLA+ spec will be detailed in DESIGN.MD. What is fixed here:
- The abstract spec (`AbstractGrpc.tla`) is the target of the first refinement
- The FFI and the .NET binding are successive refinement levels
- Safety properties do not depend on any fairness assumption
- Liveness properties are conditional on declared fairness (scheduler, callback, consumer, peer)

---

## V1 Scope

### Included

- HTTP and HTTPS endpoints
- Native HTTP/2 gRPC (not gRPC-Web)
- Lazy connection (default) or eager (option)
- All 4 gRPC cardinalities
- Request metadata (HTTP/2 request headers)
- Initial metadata (HTTP/2 response headers)
- Trailing metadata + gRPC status (HTTP/2 trailers)
- Cancellation
- Deadline actually applied and transmitted (`grpc-timeout`)
- Retry (5 attempts, 1s→5s, ×1.5, UNAVAILABLE/ABORTED/UNKNOWN)
- TLS with system roots
- Explicit PEM CA
- Unverified connection (explicit opt-in)
- mTLS (P12/PFX or PEM+key)
- Effective OverrideTargetName
- Proxy: disabled, explicit, environment, Windows system
- Basic proxy auth
- Connect timeout
- TCP keepalive, idle timeout
- Deterministic shutdown and safe unload
- Identical behavior on .NET Framework 4.7.2/4.8 and .NET 6/8+

### Out of V1

- OpenTelemetry telemetry
- Hedging
- Generic load balancing
- Configurable compression
- Generic interceptors
- gRPC-Web
- Java/Python/C++ bindings

---

## Design Decisions

1. **MsQuic-style callback toward official trampoline** — no completion queue exposed at the ABI
2. **A single native gRPC channel** — no retry/framing duplication in the binding
3. **`CallInvoker`** as .NET integration point (not `HttpMessageHandler`) — avoids dependency on
   `Grpc.Net.Client` for transport
4. **Consumer token published before start** — closes the fast-callback race
5. **Tokio runtime owned by the FFI** — no process-wide singleton
6. **Generation-tagged handles** — eliminates the ABA problem
7. **Immutable configuration after creation** — snapshot at the start of each call
8. **`AK_RELEASED` as the sole unload condition** — safety before liveness
9. **Synchronous `ak_channel_release` with call cancellation** — simplifies host lifecycle
10. **JSON schema as source of truth** — C# options generated from the schema, not the reverse
11. **Optional eager connection** — configurable, lazy by default
12. **`end_send` rather than `half_close`** — naming clarity in the API

---

## Validation Criteria

### Core Rust (layers 1+2)

- Configuration without opening a socket (lazy mode)
- Eager connection if configured
- All 4 cardinalities against a real server
- Request/response/trailing metadata, trailers-only, non-OK status
- Large messages exceeding the HTTP/2 window
- Slow consumer and bounded flow control (demand-driven)
- Cancellation before/after headers
- Timeout before/after headers
- end_send and terminal races
- Retries and commitment by cardinality
- Concurrent calls on a channel
- Channel closure during active calls
- Proxy (explicit, env, Windows system)
- TLS, CA, mTLS, target override

### FFI (layer 3)

- Fast callback before start returns (consumer token published beforehand)
- Callbacks serialized per call, concurrent between calls
- Stale/wrong-owner/wrong-generation tokens rejected
- Unique terminal (`AK_EVENT_STATUS`), no late event
- Release concurrent with callback/cancel/terminal
- Inputs copied before return
- Panic containment
- Start refused after gate closure
- Empty task group after shutdown
- Callback barrier without late callback
- Poll `AK_RELEASED` certifies quiescence
- `ak_channel_release` cleanly cancels in-flight calls

### .NET (layers 4+5)

- Tests on net472 and net48
- Tests on net6.0 and net8.0 (identical behavior)
- Generated stubs using the native `CallInvoker`
- Blocking and async unary
- Client, server, and bidirectional streaming
- CancellationToken and effective deadline
- Non-inline continuations (`RunContinuationsAsynchronously`)
- Delegate rooting under forced GC
- Dispose during active call
- Repeated create/shutdown/status cycles → `AK_RELEASED`
- Unload only after demonstrated quiescence
- Existing options preserved and effective
- Managed fallback works if native DLL is absent

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| .NET Framework 4.7.2 incompatibility | Blocking | CI tests on net472 from earliest development |
| Trampoline + host queue performance | Latency | Benchmark vs managed baseline early |
| Shutdown complexity under GC/finalizers | Memory corruption | TLA+ model of binding, no finalizer that forces unload |
| Multi-RID cross-compilation | Packaging | CI matrix from the start |
| Windows proxy options (PAC/WPAD) | Timeout/hang | WinHTTP async resolver with timeout |
| JSON schema / C# options divergence | Drift | C# generation from schema, round-trip test |

---

## References

- `packages/rust/armonik-transport-ffi/TOWARDS_ARMONIK_GRPC.MD` — Transition summary
- `packages/rust/armonik-transport-ffi/DESIGN.md` — Async FFI isolation design
- `packages/rust/armonik-transport-ffi/PROTOCOL.md` — Current transport ABI contract
- `packages/rust/armonik-transport-ffi/STATE_OF_THE_ART.md` — Async FFI state of the art
- `packages/rust/armonik-transport-ffi/BASELINE_COMPARISON.md` — Current FFI / C# spike comparison
- `packages/rust/armonik-transport-ffi/include/NativeMethods.g.cs` — Current v1 ABI surface
- `packages/rust/armonik-transport-ffi/include/http_config.schema.json` — Current config schema

---

## Next Steps

- **DESIGN.MD** — Finalization of Rust and C APIs, exact types, error contracts, TLA+ specification
- **TASKS.MD** — Ordered implementation plan (the public contract of `armonik-transport` being an
  identified prerequisite)
