# TASKS — .NET ArmoniK Client on Native Rust gRPC Channel

## Philosophy

We start from main. We build the shortest path to a .NET unary call → real gRPC server.
Each task is a functional commit that adds a tested capability. The existing PR stack
(#711–#747) is a source of code to pick from, not a prerequisite to integrate in bulk.

The deliverable is a functional PR stack. Nothing is merged into main before complete
end-to-end validation. Small bugfix PRs directly on main are possible.

The TLA+ proof comes before the FFI implementation.

---

## Context: existing PRs

The current stack (#711 → #747) builds `armonik-transport` incrementally. The code is
good but organized for the old architecture (FFI transport HTTP/2, no gRPC layer).

**What we pick from**: the transport code (proxy, TLS, serde, connector, tests) when we
need it. **What we don't take as-is**: the organization as a stack piled across 18 PRs,
the FFI skeleton/client (#744–#747), the reexports (#746).

Each task below indicates whether it picks from the stack or writes from scratch.

---

## Phase 0 — TLA+ (before any FFI implementation)

**The invariant names in T0.1-T0.3 are the pre-model sketch and are not the contract.**
They were written before the specifications existed and most did not survive the
modelling: ten of T0.1's fifteen names exist, one of T0.3's four, and none of T0.2's
eight.  What each level guarantees is the manifests in `DotNetBinding_defs.tla`,
`FfiGrpc_defs.tla` and `AbstractGrpc.tla`, bound to design.md's property lists by
`ci/check_property_manifest.py`.  Read the tasks below for what was asked, the
manifests for what holds.

### T0.1: Level 0 TLA+ spec (AbstractGrpc)

**Prerequisite**: None
**Commit**: Create `spec/armonik_grpc_ffi/tla/AbstractGrpc.tla`
- Variables: runtime_state, channels, calls, 4 sequences (submitted/sent/received/delivered),
  events_delivered, send_closed
- Safety invariants: MetadataFirst, StatusLast, NoEventAfterStatus, UniqueTerminal,
  SendAfterEndSend, MonotoneRuntime, SingleRuntime, ChannelOwnership, CallOwnership,
  CreateRequiresRunning, StoppingClosesChannels, ReleasedNoCalls, SubmittedPrefixOfSent,
  ReceivedPrefixOfDelivered, OrderPreserved
- Liveness: EventualTerminal, EventualShutdown, SubmitProgress, DeliveryProgress,
  EventualMessage

**Deliverable**: TLA+ spec explorable by TLC. Structure ready for TLAPS.
**Status**: done. `AbstractGrpcState`, `AbstractGrpc`, `AbstractGrpc_defs`,
`AbstractGrpcTheorems` and four TLC configurations.

### T0.2: Level 1 TLA+ spec (FfiGrpc)

**Prerequisite**: T0.1
**Commit**: Create `spec/armonik_grpc_ffi/tla/FfiGrpc.tla`
- Added variables: handles, callbacks_in_flight, start_gate, at_ffi_boundary_send/recv
- Invariants: HandleValidity, BorrowedLifetime, CallbackSerialization, SingleSendInFlight,
  SingleRecvInFlight, GateClosed, ReleasedImpliesQuiescent, FfiBoundaryOrder
- Refinement mapping to AbstractGrpc
- Decomposition of Level 0 fairness into local fairness

**Deliverable**: Level 1 TLA+ spec. Refinement verified by TLC.
**Status**: done, and the refinement is proved rather than only checked:
`RefinesInit`, `RefinesNext` disjunct by disjunct, and the fairness lifts.
The refusals' enabledness lives in `FfiGrpcEnabledTheorems`, apart because a
statement that writes ENABLED cannot be reached through an instance.

### T0.3: Level 2 TLA+ spec (DotNetBinding)

**Prerequisite**: T0.2
**Commit**: Create `spec/armonik_grpc_ffi/tla/DotNetBinding.tla`
- Variables: call_states (GCHandle), host_queue, tcs_state, gc_roots, dispose_state
- Invariants: RootSurvivesCallbacks, GCHandleAllocBeforeStart, ContinuationsAsync,
  DisposeAwaitsReleased
- Refinement mapping to FfiGrpc
- Fairness justification "callback returns" (bounded trampoline)

**Deliverable**: Level 2 TLA+ spec. Refinement verified.
**Status**: done. `Spec => L1!Spec` proved, so level 1's guarantees are inherited
rather than restated.  Six TLC configurations plus two witnesses, whose targets
are stated negatively so a violation trace is the result - without them either
branch could be dead code.

### T0.4: TLAPS proofs (3 levels)

**Prerequisite**: T0.3
**Commit**: Prove by TLAPS:
- Safety invariants of all 3 levels (induction)
- Refinements (simulation): Level 2 → Level 1 → Level 0
- Fairness decomposition

**Deliverable**: `spec/armonik_grpc_ffi/tla/proofs/`. TLAPS validates.
**Status**: done.  The proofs are `*Theorems_proofs.tla` beside their interfaces
rather than a `proofs/` subdirectory - the layout the statement contract needs,
since each proofs module restates its interface verbatim.  Verified with the
fingerprint cache disabled, which is the only run that says anything about the
text as it stands: 1809, 23, 11421 and 28818 obligations, no failure.  A green
run over a warm cache says only that the obligations were once discharged.

The TLAPS proof is verified manually and will stay that way: building tlapm with its
backends costs more per pull request than the team is willing to spend.  This is a
decision, not a gap waiting to be filled, and the consequence is worth stating plainly:
`ci/check.sh` checks the declarations, the footprints, the parses and the bindings
between this document and the manifests; nothing checks that the proofs still close.

Before merging anything that touches a `*_proofs.tla` or a definition under it, on a
machine with tlapm:

```
cd spec/armonik_grpc_ffi/tla
for m in AbstractGrpcTheorems_proofs FfiGrpcEnabledTheorems_proofs \
         FfiGrpcTheorems_proofs DotNetBindingTheorems_proofs; do
  TLAPM=<path-to-tlapm> bash ci/verify_proofs.sh $m.tla
done
```

`verify_proofs.sh` passes `--nofp`, which is the whole point: a run over a warm cache
says the obligations were once discharged by a text that may since have changed.


---

## Phase 1 — Minimal E2E unary call (the shortest path)

The bare minimum for a unary call: a plain HTTP/2 connector (no TLS, no proxy, no retry),
gRPC framing, minimal FFI, minimal .NET binding.

### T1.1: Add the `grpc` module to `armonik-transport` — framing + unary

**Prerequisite**: None (parallelizable with Phase 0)
**Source**: from scratch, picking the plain HTTP connector from the existing stack
**Commit**: Add `packages/rust/armonik-transport/src/grpc/` beside the `http2` module
the crate already carries:
- Dependencies: hyper, hyper-util, http, bytes, tokio
- `GrpcChannel::new(endpoint, executor)` — connects in plain HTTP/2 (no TLS)
- gRPC framing (encode/decode length-prefixed)
- `start_call(method, metadata)` → `GrpcCall`
- `GrpcCall`: send_message, end_send, next_message → RecvResult (Message | End(GrpcStatus))
- Executor trait

**Deliverable**: Rust integration test: unary call to a local gRPC server (plain HTTP/2).
**Status**: done.  The crate also gains the `http2` module this task assumed it already had.

### T1.2: Create `armonik-transport-ffi` — minimal unary ABI

**Prerequisite**: T1.1, T0.2 (FFI spec proved or at least written)
**Source**: from scratch per the design. The crate of that name on `wip/rust-all` is not the
source: it delegates every gRPC semantic to tonic and exposes a polling ABI, where the design
calls for an engine that owns its framing under a callback ABI.
**Commit**: Create `packages/rust/armonik-transport-ffi/`:
- `ak_runtime_create`, `ak_runtime_status`, `ak_runtime_begin_shutdown`, `ak_runtime_destroy`
- `ak_channel_create` (minimal JSON config: just endpoint), `ak_channel_release`
- `ak_call_start`, `ak_get_call_buffer`, `ak_call_send_message` (zero-copy),
  `ak_return_call_buffer`, `ak_call_end_send`, `ak_call_cancel`, `ak_event_consumed`
- `ak_call_debt_of`, `ak_runtime_memory_usage`, `ak_abi_version`
- Events: WRITE_DONE, INITIAL_METADATA, MESSAGE, STATUS, SHUTDOWN_COMPLETE,
  RESOURCES_RELEASED
- SlotMap registry, owned Tokio runtime, callback with call_ctx
- One send in flight, one non-consumed event per call

There is no `ak_call_release`: design.md removed it deliberately and says why. This list
followed the design where the two disagreed.

**Deliverable**: C or Rust FFI test: unary call via the ABI to a local gRPC server.
**Status**: done.

### T1.3: Create `ArmoniK.Api.Client.RustGrpcChannel` — .NET unary binding

**Prerequisite**: T1.2
**Source**: from scratch per the design, patterns from the spike (`wk/spike/ffi-http2-handler`)
**Commit**: Create `packages/csharp/ArmoniK.Api.Client.RustGrpcChannel/`:
- P/Invoke for all entry points (DllImport, Cdecl)
- NativeRuntime + NativeChannel (SafeHandle)
- Trampoline (static delegate, GCHandle runtime_ctx)
- HostQueue + Dispatcher
- CallState (GCHandle call_ctx, TCS for metadata/status, Channel<> for messages)
- NativeCallInvoker: BlockingUnaryCall + AsyncUnaryCall
- Pin buffer for zero-copy send, await WRITE_DONE, unpin
- Cancellation via CancellationToken → ak_call_cancel

**Deliverable**: .NET E2E test: unary call to a local gRPC server (plain HTTP/2).

**Status**: done.  Four items of the commit list above are stale against design.md, which wins:
the handles are generational 64-bit tokens the runtime reclaims itself, so no `SafeHandle`; the
ring is the queue, so no dispatcher; payloads go in a buffer the engine lends out of the call's
arena, so nothing is pinned; and the delivery ring replaces `Channel<>`, which design.md rules out
by name (`IAsyncSignal RingSignal; // latched auto-reset, never SemaphoreSlim`).

### T1.4: Integrate into `ArmoniK.Api.Client` — injectable CallInvoker

**Prerequisite**: T1.3
**Commit**: Make the CallInvoker injectable into the existing client.
Minimal options mapping (just endpoint for now).
Explicit error if native DLL is missing.

**Deliverable**: An existing ArmoniK client test passes with the native CallInvoker (unary, plain HTTP).

**Status**: done.  `ArmoniK.Api.Client` gains nothing, and that is the finding: the generated stubs
already take a `ChannelBase`, so "injectable" needed no change there and the client keeps no
knowledge of the native engine - a consumer that wants it references the package, one that does
not, does not.  What was missing was the other two clauses.  The options mapping is
`options.Endpoint`, passed to `NativeRuntimeFactory.Channel`, which is the whole of "just endpoint
for now"; mapping it inside the binding would have meant depending on `ArmoniK.Api.Client` for one
POCO.  And a missing engine now raises `RustEngineMissingException`, naming the word size, where
the search looked, and which of the two supply routes was expected to answer - .NET's own message
names a bare library and no reason.

The test is `ArmoniKClientTests`, and it starts its own `ArmoniK.Api.Mock` the way the echo tests
start their own server, on ports the fixture picks.  Readiness is read from the mock's HTTP root -
the one thing it serves that is not gRPC - because it takes its ports from configuration and
announces nothing.  So it runs wherever the suite runs, and a mock that never listens fails by
name with the port it was waited on.

---

### T1.5: Architectures and runtimes — x86, x64, arm, and .NET Framework

**Prerequisite**: T1.4
**Why it is here and not in the original plan**: phase 1 was written for one architecture and one
runtime. The requirement is x86, x64 and arm — arm on .NET only, .NET Framework being Windows
x86/x64 — and the binding must be exercised from .NET Framework 4.7.2, 4.8 and .NET 8.0 client
processes. That is a scope addition rather than a detail of T1.3, so it gets its own task.

**Commit**:
- `tests/layout.rs` in pointer widths rather than absolute x64 numbers, and run for i686 as well as
  x86_64. Done; the receipt is in its own commit.
- Multi-target cargo builds: `--target` per RID in the binding's build step, which today builds the
  host architecture alone, plus the cross toolchains for each.
- `runtimes/<rid>/native` packaging, so .NET resolves the engine with no code of ours; plus the
  `build/*.targets` and the `NativeMethods` static constructor that .NET Framework needs, having no
  RID probing. One loading path for every Framework consumer: both architectures are copied beside
  the application and `IntPtr.Size` picks, AnyCPU and an explicit `PlatformTarget` alike. The loader
  is a no-op where that folder is absent, which is the .NET case, so it is the same code there.
- The echo server as its own `net8.0` executable, because Kestrel and Grpc.AspNetCore do not run on
  .NET Framework, and the tests multi-targeted `net4.7;net4.8;net8.0` dialling it — which is how
  `ArmoniK.Api.Client.Test` and `ArmoniK.Api.Mock` already work (`test.yml:165-181`).
- `test.yml` as a matrix over target framework and architecture, with a Rust toolchain in the C# job.

**Deliverable**: the unary E2E test green from a .NET Framework 4.7.2, a 4.8 and a .NET 8.0 process,
on x86 and x64, with arm64 built and packaged.

**Status**: done, apart from arm64, which has no runner here to build or run on - the mapping and
the packaging carry it, and the first CI job on an arm host will say whether that is enough. The
matrix is 15 tests over three runtimes and two architectures, and `test.yml` runs seven
combinations of runtime, architecture and operating system.

### Phase 1 closing — the reviews, and what is deliberately left

Two items this section listed as open are closed. The `/simplify` findings are applied. And the
x86 flake - "seen once as 11 failures and never reproduced" - is diagnosed and fixed: snafu 0.9
generates a `backtrace` field with `Backtrace::force_capture()` rather than `capture()`, so every
error built one whatever `RUST_BACKTRACE` said, and on 32-bit Windows the `dbghelp` stack walk
that triggers can enter a cycle and never return. Three write-only `backtrace` fields are gone,
which removes the cycle and the per-error cost. It is layout-sensitive - it shows on a large debug
binary, vanishes in release, and any edit to the enclosing function makes it go away - so a run
that does not reproduce it proves nothing, and the rate has to be measured over tens of runs.

Phase 1 was reviewed four times over: compliance to the TLA+ model, `/simplify` across the whole
of `packages/rust` and then once per crate and per dll, a code-quality pass, and compliance to the
model again. The second model pass is the one that earned its keep. It found two breaks of proved
level-0 invariants that the first pass and three cleanup passes had all walked past:

- A call cancelled while its reader was parked on a delivery credit dropped the message it was
  carrying and then forwarded whatever the peer had decided, so a host could be told the call
  completed while an event of it was thrown away. `CompleteDelivery` forbids it, and once
  cancellation is latched the model admits one terminal: CANCELLED.
- A closing channel waited for its last call to be *reclaimed* rather than to reach its terminal,
  so `SHUTDOWN_COMPLETE` and `GRPC_STOPPED` were observable with a channel still CLOSING - false
  for `IsRuntimeDrained` and for `ReleasedNoChannels`. The header sentence and a test had locked
  in the wrong reading; both are corrected with the code.

Two questions the model settled rather than the code:

- `ak_call_start` on a stopped runtime answers `AK_STATUS_INVALID_STATE`, not
  `AK_STATUS_HANDLE_STALE`. `IsRuntimeDrained` requires every channel closed and
  `ChannelStateMatchesNative` requires a disposed channel to read closing or closed, so the
  channel has to stay nameable; staling its handle at the drain made both unrepresentable.
- The budget wait owes no `RetryRefusedBudget` action. A refused retry changes no variable, so it
  is stuttering by construction, and `BudgetWaitEndsWhenHopeless` says in its own comment that the
  wait promises nothing else.

**Deliberately left, and why:**

- arm64 is mapped and packaged but has never been executed. There is no runner here; the first CI
  job on an arm host is what will say whether the packaging is enough.
- The happy-eyeballs behaviour that motivated adopting `hyper_util`'s connector has no test:
  nothing in the suite presents a dual-stack host with one dead address family.
- Five minor divergences from the second model pass. The largest gap is not a divergence but an
  absence: the model's reader machine (`consumer_phase`, `reader_state`, the `BeginMoveNext`
  family) has no implementation, because it is phase 2's, so the invariants over it are vacuous
  today. That is T2.2's starting point rather than a debt - the machine is specified and proved,
  and `design.md` carries the `MoveNext` it asks for.

---

## Phase 2 — Streaming (the 3 other cardinalities)

### T2.1: Client streaming (Rust channel + FFI + .NET)

**Prerequisite**: T1.3
**Commit**: Multiple send_message before end_send. Same FFI mechanics (WRITE_DONE per message).
.NET side: AsyncClientStreamingCall with write stream.

**Deliverable**: .NET E2E test: client streaming.

### T2.2: Server streaming (Rust channel + FFI + .NET)

**Prerequisite**: T1.3
**Commit**: next_message / event_consumed loop until RecvResult::End.
.NET side: AsyncServerStreamingCall with read stream.

**Deliverable**: .NET E2E test: server streaming.

### T2.3: Bidi streaming (Rust channel + FFI + .NET)

**Prerequisite**: T2.1, T2.2
**Commit**: Concurrent send and recv. .NET side: AsyncDuplexStreamingCall.

**Deliverable**: .NET E2E test: bidi streaming.

---

## Phase 3 — TLS and secure connection

### T3.1: TLS with system roots

**Prerequisite**: T1.1
**Source**: pick the TLS config from the existing stack (#725, #726)
**Commit**: Add TLS to the connector (`CaSource::System`). Endpoint `https://`.

**Deliverable**: Test: unary call over HTTPS with system CA.

### T3.2: Explicit PEM CA

**Prerequisite**: T3.1
**Source**: pick from #726
**Commit**: `CaSource::PemFile`. Option in the JSON config.

**Deliverable**: Test: connection with custom CA.

### T3.3: mTLS (PEM + PKCS12)

**Prerequisite**: T3.2
**Source**: pick from #726, #730
**Commit**: `IdentitySource::PemFiles` and `IdentitySource::Pkcs12`. Load + inject into rustls.

**Deliverable**: mTLS tests: PEM pair and P12.

### T3.4: Effective OverrideTargetName

**Prerequisite**: T3.1
**Source**: pick from `wk/fix/rust-override-target-server-name`
**Commit**: Override the ServerName in the rustls handshake.

**Deliverable**: Test: override target, handshake succeeds with a different name.

### T3.5: WindowsStore (CA and client identity)

**Prerequisite**: T3.3
**Commit**: `CaSource::WindowsStore` and `IdentitySource::WindowsStore`. Resolution on Rust side.

**Deliverable**: Test (Windows CI): mTLS from the store.

### T3.6: Insecure (unverified connection)

**Prerequisite**: T3.1
**Commit**: `CaSource::Insecure`. Explicit opt-in.

**Deliverable**: Test: connection without certificate verification.

---

## Phase 4 — Proxy

### T4.1: Explicit proxy

**Prerequisite**: T1.1
**Source**: pick from #711, #712
**Commit**: `ProxySource::ExplicitUri` and `ExplicitWithCredentials`. CONNECT tunnel.

**Deliverable**: Test: unary via explicit HTTP proxy.

### T4.2: Environment proxy

**Prerequisite**: T4.1
**Source**: pick from #716
**Commit**: `ProxySource::Environment`. Read HTTP_PROXY/HTTPS_PROXY/NO_PROXY.

**Deliverable**: Test: proxy via env.

### T4.3: Windows system proxy

**Prerequisite**: T4.1
**Source**: existing code in the stack
**Commit**: `ProxySource::WindowsSystem`. Async WinHTTP resolver with timeout.

**Deliverable**: Test (Windows CI): system proxy.

---

## Phase 5 — Retry, deadline, robustness

### T5.1: Deadline (local + grpc-timeout)

**Prerequisite**: T1.1
**Commit**: Local timer per call. `grpc-timeout` header transmitted to the server. Expiration →
cancel + RecvResult::End(DEADLINE_EXCEEDED). default_deadline option in GrpcChannelConfig.

**Deliverable**: Test: deadline expires → status DEADLINE_EXCEEDED.

### T5.2: Automatic retry (unary)

**Prerequisite**: T5.1
**Source**: pick the types from #732 (RetryConfig)
**Commit**: RetryConfig in GrpcChannelConfig. Exponential backoff. Retryable codes.
Transparent unary retry (no buffer needed — single message, replayable).

**Deliverable**: Test: retry on UNAVAILABLE, succeeds on 2nd attempt.

### T5.3: Streaming retry (replay buffer)

**Prerequisite**: T5.2, T2.1
**Commit**: Configurable replay buffer. Client streaming retryable if ≤ buffer.
Bidi retryable if no response received and ≤ buffer. Commitment detection.

**Deliverable**: Tests: streaming retry ≤ buffer OK, streaming retry > buffer → committed (no retry).

### T5.4: Complete FFI runtime shutdown

**Prerequisite**: T1.2, T0.4 (safety proofs)
**Commit**: begin_shutdown closes the start gate. Drains/cancels calls. Awaits quiescence.
Joins Tokio. SHUTDOWN_COMPLETE callback. Poll status → RELEASED.

**Deliverable**: Test: create/shutdown/RELEASED cycles. No thread in flight after RELEASED.

### T5.5: Eager connection (option)

**Prerequisite**: T1.1
**Commit**: `eager_connect` option in GrpcChannelConfig. If true, HTTP/2 connection at create.

**Deliverable**: Test: with eager, the connection is established before the first call.

### T5.6: TCP keepalive and idle timeout

**Prerequisite**: T1.1
**Source**: pick from #741
**Commit**: Keepalive and idle timeout options in config. Applied to the Hyper pool.

**Deliverable**: Test: idle connection is closed after timeout.

---

## Phase 6 — Config, packaging, final integration

### T6.1: Complete JSON schema and C# generation

**Prerequisite**: All options implemented (T3.x, T4.x, T5.x)
**Source**: pick from #728, #745
**Commit**: Regenerate the JSON schema from the final Rust types (schemars). Generate C#
types (RustChannelOptions). Schema freshness test. C# → JSON → Rust round-trip test.

**Deliverable**: Schema committed. C# types generated. No drift.

### T6.2: Complete mapping of existing ArmoniK client options

**Prerequisite**: T6.1, T1.4
**Commit**: Map all `GrpcChannel` options (ArmoniK.Api.Common.Options) to
`RustChannelOptions`: endpoint, TLS, proxy, timeout, retry. Correspondence test.

**Deliverable**: The native provider accepts all existing client options.

### T6.3: Cross-platform build and NuGet package

**Prerequisite**: T1.2
**Commit**: CI build of the native DLL for win-x64, win-x86, linux-x64, linux-x86, linux-arm64.
Multi-RID NuGet package with `runtimes/{rid}/native/`. Automatic resolution.

**Deliverable**: `dotnet pack` produces a functional NuGet. DLL resolved on each platform.

### T6.4: .NET Framework 4.7.2 and 4.8 E2E tests

**Prerequisite**: T6.3, T2.3
**Commit**: Test project targeting net472 and net48. Same suite as net6.0/net8.0.

**Deliverable**: All tests pass on .NET Framework. Identical behavior.

### T6.5: Comparative benchmarks campaign

**Prerequisite**: T6.4
**Commit**: Unary latency benchmarks (P50/P95/P99), streaming throughput, memory overhead.
Native vs managed. net48 and net8.0. Results documented.

**Deliverable**: Performance baseline established.

### T6.6: Documentation and cleanup

**Prerequisite**: T6.5
**Commit**: README, migration guide. Close obsolete PRs from the stack. Dead code cleanup.

**Deliverable**: Clean repo. PR stack ready to merge into main.

---

## Phase 7 — Rust ArmoniK Client on armonik-transport

### T7.1: Adapt the Rust ArmoniK client to use the `grpc` module

**Prerequisite**: T1.1, T2.3 (functional Rust channel with all 4 cardinalities)
**Commit**: Replace the direct Tonic dependency in the Rust ArmoniK client with an adapter
that consumes `armonik-transport`'s `grpc` module. The generated Tonic stubs work via a
`Channel` adapter
that delegates to `GrpcChannel`. The Rust client and the .NET binding share the same native
gRPC engine.

**Deliverable**: Rust ArmoniK client tests pass using `armonik-transport` instead of
Tonic directly. Same functional behavior.

---

## Phase 8 — Open subjects, after V1

### T8.1: Bound the replay buffers of a whole process

**Prerequisite**: T5.2 (retry works and its cost is measurable)
**Commit**: study, then whatever it concludes.

`RetryConfig::max_buffer_size` is configured per channel and consumed per call, so a channel
holds that size times the number of retryable calls in flight and nothing bounds the product. A
process with several channels multiplies it again, and the post-V1 per-call retry override would
let a call set its own size, so no single configuration value can be read as the ceiling. V1
states the limit as per-call and accepts it.

What to settle, in this order, because each answer constrains the next:

- **What happens when the ceiling is reached.** Refusing a `send_message` and silently making a
  call non-retryable are two different contracts, and the second one changes what a caller can
  conclude from a failed call. This is the decision, not an implementation detail: pick it first.
- **Where the ceiling lives.** A byte budget owned by the runtime and lent to channels, or a
  pool whose exhaustion is itself the limit. A pool is tempting because it also answers the
  allocation question, but it couples a resilience policy to an allocator.
- **How a per-call size interacts with it.** A call asking for more than the channel's share
  either borrows from the runtime, is refused, or is admitted as non-retryable.
- **What the host can observe.** If a call quietly stops being retryable, a binding needs to be
  able to say so, which probably means a diagnostic rather than a new event.

**Deliverable**: a decision recorded in the design, and either an implementation or a stated
reason for keeping the per-call bound.

---

## Dependency graph

```text
                T0.1 → T0.2 → T0.3 → T0.4 (TLA+, parallel to everything else)
                                        │
T1.1 ─────────────→ T1.2 ←─────────────┘ (FFI after proof)
  │                    │
  │                  T1.3 → T1.4
  │                    │
  ├── T2.1, T2.2 → T2.3
  │
  ├── T3.1 → T3.2 → T3.3 → T3.4, T3.5, T3.6
  │
  ├── T4.1 → T4.2, T4.3
  │
  ├── T5.1 → T5.2 → T5.3
  │     T5.4, T5.5, T5.6
  │
  └── T6.1 → T6.2 → T6.3 → T6.4 → T6.5 → T6.6
```

---

## Parallelization

- **Phase 0** (TLA+) advances in parallel with Phase 1 (Rust code)
- **T1.1** (Rust channel) has no blocking prerequisite — starts immediately
- **T1.2** (FFI) waits for T0.2 (FFI spec written) — the full proof (T0.4) is ideal but the
  written spec is enough to start implementation
- **Phases 3, 4, 5** are independent of each other — parallelizable after Phase 1
- **T6.3** (cross-platform build) can start as soon as T1.2
