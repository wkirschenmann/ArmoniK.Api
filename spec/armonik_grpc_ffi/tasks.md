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

The stack (#711 → #747) builds `armonik-transport` incrementally. **It is discarded**, and
what is wanted is taken from it first: the option machinery (`config_utils`, the units, the
schema, the `Secret` type), the proxy, the TLS beyond what this crate already carries, and
their tests. It was organized for the old architecture - an FFI transport over HTTP/2 with no
gRPC layer - so nothing of its shape survives, only its code.

Phase 3 is where most of that harvest happens, because the options are what the stack is
mostly about.

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
  absence at the time: the model's reader machine (`consumer_phase`, `reader_state`, the
  `BeginMoveNext` family) had no implementation, so the invariants over it were vacuous. T2.2
  implemented it and they are not.

---

## Phase 2 — Streaming (the 3 other cardinalities)

### T2.1: Client streaming (Rust channel + FFI + .NET)

**Prerequisite**: T1.3
**Commit**: Multiple send_message before end_send. Same FFI mechanics (WRITE_DONE per message).
.NET side: AsyncClientStreamingCall with write stream.

**Deliverable**: .NET E2E test: client streaming.
**Status**: done.  The engine and the ABI already carried several sends - `send_message` waits
for room, and the ABI acquits each message with its own WRITE_DONE - and nothing exercised it,
both echo servers being unary only.  What the binding had to learn is that **a write completes
at its WRITE_DONE and not at its commit**, which is what makes a window of one enough for a
stream: the emission that completes one write has already freed the slot the next lend asks for.
design.md states it and level 2 names it `ManagedWriterNeverObservesSlotBusy`; removing the wait
makes the writer observe `SLOT_BUSY`, which is the proof it is load-bearing.

### T2.2: Server streaming (Rust channel + FFI + .NET)

**Prerequisite**: T1.3
**Commit**: next_message / event_consumed loop until RecvResult::End.
.NET side: AsyncServerStreamingCall with read stream.

**Deliverable**: .NET E2E test: server streaming.
**Status**: done, and it is the task that changed the most.  The read side is now the machine
design.md specifies rather than a background task draining eagerly into one response: one
consumer of the delivery ring, taken by a transition and never by a peek, the registration
disarmed before the winner is decided, one `CompareExchange` per read arbitrating between the
token, the result and a decode failure, and the payload acquitted after that decision in a
guaranteed block.  Two defects of T1.3 fell out of it: the count of unacquitted sends sat behind
a `try { return ... } finally` and had never run - the compiler had been saying so, `CS0162` -
and `Cancel` ended the call without handing the ring to a drain, so a response stream abandoned
half read left the channel's disposal waiting for good.

### T2.3: Bidi streaming (Rust channel + FFI + .NET)

**Prerequisite**: T2.1, T2.2
**Commit**: Concurrent send and recv. .NET side: AsyncDuplexStreamingCall.

**Deliverable**: .NET E2E test: bidi streaming.
**Status**: done, and it asked for nothing new in the binding: the writer landed with T2.1 and
the reader with T2.2, and the two halves share no state - a write waits for an acquittal the
engine delivers off the ring, a read takes the ring.  The work was the fixture, because
interleaving is what the cardinality is for: a bidi service that answered only at the half-close
would let a batching client pass and prove nothing.  The interleaved test passed without a
change to the binding, which is the evidence that the halves are independent.

### Phase 2 closing — what the cardinalities settled, and what is left

**Every cardinality reads the same way.**  A call that answers once - unary, client streaming -
is the streaming reader reduced to a single: one message, then a terminal, and anything else is
a server that did not honour the cardinality.  There is no second read path to keep consistent
with the first, which is what lets the model's `reader_state` cover all four rather than one.
What differs between the cardinalities is what they send, not how they read.

**Deliberately left, and why:**

- `WriteOptions` is accepted and ignored: the ABI carries no per-write flag, and inventing one
  for a value no caller sets would be a field to keep true rather than a feature.
- No retry on a stream.  That is T6.4 and its replay buffer, and it is the reason it is a task of
  its own rather than a clause of the retry that precedes it.
- The streaming tests run against this repository's echo fixture, not against ArmoniK's own
  contracts.  `ArmoniK.Api.Mock` implements three streaming RPCs - `Events.GetEvents`,
  `Results.DownloadResultData` and `Results.UploadResultData` - and `ArmoniKClientTests` already
  starts it, so exercising the generated stubs of those is available and not yet done.
- arm64 is unchanged from phase 1: mapped and packaged, never executed.

---

## Phase 3 — The option surface, end to end

The options were planned last, as T6.1 and T6.2. They come first instead: nothing in TLS, in
proxy or in retry is reachable from a .NET caller until the JSON carries it, and the schema is
the only dependency the configuration code has. Implementing three phases no consumer can use is
the order this corrects.

**Where each side's authority lies.** Rust owns the option types, their defaults, their
validation and the schema derived from them. .NET owns the loading: appsettings, environment and
command line, in the order its own configuration system layers them. That division is forced
rather than chosen - the files and the command line are .NET's, so a separate environment read on
the Rust side would sit outside that ordering and break the precedence between the three. So the
JSON handed to `ak_channel_create` is complete and authoritative, and **the engine consults no
environment variable on the FFI path**. `from_env` stays for the crate's own Rust consumers.
Empty means unset means take the default; it no longer means look at the environment.

**The options have two shapes, and only one of them is flat.** Across the FFI the JSON is
structured and typed: units nest as objects, a boolean is a JSON boolean and not the string
`"true"`, a number is a number. That is the shape a generated C# type serializes to and the shape
that says the most about what a value is, so it is the one the boundary carries. An environment
variable has no structure and no type at all, so there the same options are flat names under a
prefix - `GrpcClient__TcpKeepAliveInterval` - and every value is text. `embed_prefixed!` and
`schema_with_prefix` serve that second shape: they are how a unit is read under a prefix and how
its schema is rewritten to match, and on the FFI path neither is needed because nothing is
flattened there. Producing the flat shape for a .NET caller is .NET's, which binds its own
configuration sources; producing it for a Rust caller is `from_env`'s, which stays.

The readers are shared whichever shape carried the value: `text` accepts a real boolean or a real
number as readily as a string, so one vocabulary interprets both and the two shapes cannot
disagree about what a value means.

**Left for later:** giving the Rust crate the same layering .NET has - files, environment,
command line, bound onto the config types in one declared order - rather than `from_env` alone.
The types this phase declares are what it would bind onto, so the question is worth asking after
they exist and not before.

### T3.1: Harvest `config_utils`

**Prerequisite**: none
**Source**: the #7xx stack, which is discarded once what is wanted has been taken from it
**Commit**: the from-string readers (`text`, `secret_text`, `boolean`, `optional_duration`,
`optional_parsed` and the rest), `strip_rust_details`, `schema_with_prefix`, the `embed_prefixed!`
macro and the `Secret` type. Domain-free machinery only: `endpoint`, `rate_limit` and `user_agent`
are transport vocabulary and stay out, which is what keeps a later lift into its own crate a
directory move.

**Deliverable**: the module in place with its tests, and no domain option inside it.

### T3.2: The option units, as live types

**Prerequisite**: T3.1
**Source**: the #7xx stack
**Commit**: `tls`, `proxy`, `retry`, `http2`, `tcp_keepalive` as nested types reached as
`config.retry.max_attempts`, never flattened into plain fields - and nested in the JSON too, as
objects, rather than flattened into it. The prefix belongs to whoever
embeds a unit, not to the unit, so one can be embedded twice. Names come from the mechanism -
field name, `rename_all = "PascalCase"`, prefix - and nowhere from a per-field rename.

The `retry` unit carries **two** replay ceilings rather than one: what a call that answers once
may hold, and what a stream may. They are different quantities - the first bounds a single
message kept in case it has to go again, the second a whole sent prefix - so one value would
either starve the stream or let a unary call reserve a stream's worth. T6.1 settles what happens
when either is reached; that they are configuration, and that there are two, is settled here.

**Deliverable**: `grep -c rename` on each unit answers 1, the container attribute. Both ceilings
appear in the schema and reach the engine.

### T3.3: The schema, and the C# type, as build artefacts

**Prerequisite**: T3.2
**Commit**: `schemars` on the Rust types, emitted by a cargo target. The schema describes the
structured shape - nested objects, booleans as booleans, numbers as numbers - because that is
what the generated C# type has to serialize to, and a schema that said `string` everywhere would
generate a class that says nothing about what it holds. The binding's csproj already
shells out to `cargo build` for the engine; it runs the emitter and the C# generation in the same
step, so the generated options type is produced from the schema at every build of the native
library. Nothing is committed and nothing is diff-checked: staleness is not detected, it is made
impossible. The build fails loudly if the generation does not run.

**Deliverable**: the hand-written `ChannelOptions` is deleted and its replacement is generated.
Round-trip test C# -> JSON -> Rust over every option.

### T3.4: The .NET side loads, and only loads

**Prerequisite**: T3.3
**Commit**: bind `IConfiguration` - appsettings, environment, command line - onto the generated
type, in .NET's own precedence order, and serialize the whole of it. No option is read on the
Rust side of the FFI boundary.

**Deliverable**: a test setting the same option in two layers and asserting .NET's order decides;
a test that an option set only in the environment reaches the engine.

### T3.5: Align the vocabulary by configuration

**Prerequisite**: T3.4
**Commit**: the prefixes and the structure are configurable, so the names line up with the
existing client's where a counterpart exists rather than through a hand-written table. The
generated type is a superset: there are many more options here than `GrpcClient` carries today,
and the ones without a counterpart simply have none.

**Deliverable**: every `GrpcClient` option reaches its generated counterpart, and the options
with no counterpart are listed rather than silently extra.

---

## Phase 4 — TLS and secure connection

### T4.1: The engine takes the real connector

**Prerequisite**: T3.5, T1.1
**Commit**: widen `GrpcChannelConfig` past `{ transport, user_agent, max_sends_in_flight,
max_recv_message_size }` and let `connect.rs::https_connector` replace the plain connector of
`http2.rs`, which refuses `https://` by construction.

This one task delivers what the August plan cut into five, because they are branches of one
function that already exists and is tested: system roots, an explicit PEM CA, `OverrideTargetName`
with its IPv6 and its option-naming error, the insecure opt-in, and mTLS from a PEM pair. It also
carries what the old T5.6 asked for - keepalive, nodelay, keepalive interval and retries, connect
timeout - because the same connector sets them.

**Deliverable**: a unary call over HTTPS, and one test per branch of the connector reached from
the engine rather than from the connector's own tests.

### T4.2: Client identity from a PKCS#12 bundle

**Prerequisite**: T4.1
**Source**: the #7xx stack
**Deliverable**: mTLS with a P12 bundle and its password, the password read as a `Secret`.

### T4.3: The whole certificate chain

**Prerequisite**: T4.2
**Source**: the #7xx stack
**Commit**: present the chain and not the leaf alone.

**Deliverable**: a handshake a server accepts only when given an intermediate.

### T4.4: WindowsStore, for the CA and for the identity

**Prerequisite**: T4.2
**Commit**: resolution on the Rust side. Genuinely new, and platform-specific.

**Deliverable**: mTLS from the store, on the Windows CI.

---

## Phase 5 — Proxy

### T5.1: Explicit proxy, with and without credentials

**Prerequisite**: T4.1
**Source**: the #7xx stack
**Commit**: the proxy types and the CONNECT tunnel. The stack's `Secret` takes the password
fields, and its URI handling replaces the `safe_endpoint` this crate carries.

**Deliverable**: a unary call through an explicit HTTP proxy, and no credential in any message.

### T5.2: Proxy from the environment

**Prerequisite**: T5.1
**Source**: the #7xx stack
**Commit**: `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`.

Read on the Rust side, unlike every other option, because these three are the operating system's
convention rather than this library's vocabulary: a caller that sets none of them still expects
them honoured, and .NET has no counterpart to bind. That is the one deliberate exception to
phase 3's rule, and it is recorded here rather than discovered later.

**Deliverable**: a call through a proxy named only by the environment.

### T5.3: Windows system proxy

**Prerequisite**: T5.1
**Commit**: the asynchronous WinHTTP resolver, with its timeout.

**Deliverable**: a call through the system proxy, on the Windows CI.

---

## Phase 6 — Deadline, retry, and what bounds them

### T6.1: What happens when a replay ceiling is reached

**Prerequisite**: T3.5
**Commit**: study, then whatever it concludes.

This was T8.1, after V1. It comes before the retry it bounds, because its own first question is a
contract and not an implementation detail, and choosing it once per-call buffers exist means
retrofitting rather than designing.

Two of its four questions are already answered: the ceilings are configuration, and there are two
of them - one for a call that answers once, one for a stream - which T3.2 declares. What is left
is what they mean.

- **What happens at the ceiling.** Refusing a `send_message` and quietly making a call
  non-retryable are two different contracts, and the second changes what a caller may conclude
  from a failed call. This is the decision; pick it first.
- **Whether anything bounds the product.** A ceiling consumed per call means a channel holds it
  times the retryable calls in flight, and several channels multiply it again. Either that
  product is accepted and stated, or something bounds it - and the runtime already owns a byte
  budget it lends for payloads, `Ledger`, reported by `ak_runtime_memory_usage`, which is the
  first candidate rather than a new mechanism.
- **What the host can observe.** A call that quietly stops being retryable is something a binding
  has to be able to say, which is probably a diagnostic rather than a new event.

**Deliverable**: a decision recorded in the design, and either an implementation or a stated
reason for accepting the unbounded product.

### T6.2: Deadline

**Prerequisite**: T1.1
**Commit**: a local timer per call, the `grpc-timeout` header transmitted, expiry cancelling the
call and answering `DEADLINE_EXCEEDED`.

It also lifts `MustCarryNoDeadline`, which is today the only `Unimplemented` the binding opposes
to an ordinary caller.

**Deliverable**: a deadline expires and the status says so; a caller's `CallOptions.Deadline` is
no longer refused.

### T6.3: Retry for a call that answers once

**Prerequisite**: T6.1, T6.2
**Source**: the retry types from the #7xx stack
**Commit**: exponential backoff, retryable codes. A single message is replayable without a
buffer, so this cardinality needs none.

**Deliverable**: a retry on UNAVAILABLE that succeeds on the second attempt.

### T6.4: Retry for a stream

**Prerequisite**: T6.3, T2.3
**Commit**: the replay buffer, bounded as T6.1 decided. A client stream is retryable while it
fits; a bidi one while nothing has been answered and it fits. Commitment detection.

**Deliverable**: a stream within the buffer retries, one past it is committed and does not.

### T6.5: Eager connection, as an option

**Prerequisite**: T3.5
**Commit**: the behaviour exists - `GrpcChannel::connect()`, and
`connecting_up_front_reports_what_a_call_would_have_reported` tests it - so this is the option
that reaches it and nothing else.

**Deliverable**: with the option set, the connection is established before the first call.

### T6.6: Packaging, finished

**Prerequisite**: T4.1
**Commit**: the Linux runtime identifiers in CI, `dotnet pack` producing a package that resolves
on each platform, and arm64 executed at last rather than only mapped and packed.

**Deliverable**: a package that works on every runtime identifier it claims.

### T6.7: Benchmarks

**Prerequisite**: T6.6
**Commit**: unary latency at P50, P95 and P99, streaming throughput, memory overhead. Native
against managed, on net4.8 and net8.0.

**Deliverable**: a baseline recorded.

### T6.8: Integration into `ArmoniK.Api.Client`

**Prerequisite**: T3.5, T6.6
**Commit**: let a consumer choose the transport by configuration rather than by which factory it
calls, and split the assemblies so that choosing costs only what it uses.

Today the seam is `ChannelBase`: every generated ArmoniK stub takes one, `NativeChannel` is one,
and a consumer picks by calling `NativeRuntimeFactory.Channel` instead of
`GrpcChannelFactory.CreateChannel`. That works and is what phase 1 deliberately settled for -
`ArmoniK.Api.Client` knows nothing of the native engine, so a consumer that does not want it does
not carry it.

Two facts constrain whatever replaces it, and neither is a matter of taste:

- `GrpcChannelFactory.CreateChannel` returns `GrpcChannel`, grpc-dotnet's concrete type, not
  `ChannelBase`. It can never hand back a `NativeChannel` without a breaking signature change.
- A selector living inside `ArmoniK.Api.Client` makes that package depend on the native one, so
  the cdylib and its architectures enter every consumer's build, including those that chose the
  managed transport. The candidates that keep both properties are a third thin package that may
  reference both, or the consumer's own dependency wiring.

`HttpMessageHandler` is the existing precedent for naming a transport in a string option, and
the generated options type is a superset of `GrpcClient`, so the two vocabularies meet here or
nowhere.

**Deferred until the Rust bridge on the other side is built**, so that both directions are
designed together rather than one constrained by the other. Recorded now so the constraints above
are not rediscovered.

**Deliverable**: a consumer switches transport by configuration, and one that does not want the
native engine does not build it.

### T6.9: Documentation and cleanup

**Prerequisite**: T6.8
**Commit**: README and migration guide. The #7xx stack discarded once nothing more is wanted from
it. Dead code removed.

**Deliverable**: a clean repository.

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

## Phase 9 — What belongs upstream

### T9.1: The tunnel fixes hyper-util has not had

**Prerequisite**: T5.1
**Commit**: carry upstream what this repository has measured and pinned.

`hyper_util::client::legacy::connect::proxy::Tunnel` is what opens a CONNECT tunnel, and the
proxy work does not write its own. As shipped in 0.1.20 it gets four cases wrong, each of them
measured here and pinned by a `known_issue_*` test that is meant to fail the day a release fixes
it - so a dependency bump turning CI red is the notice, not a regression:

- **Only an exact `200` opens the tunnel**, where RFC 9110 says any `2xx` should. A proxy
  answering `201` or `204` reads as a refusal.
- **A status line split across two reads is rejected**, though the connection is fine. Legal
  HTTP, and likelier with a slow proxy or a small MSS.
- **An `HTTP/1.0 407` is not recognised as a request for credentials.** Only `HTTP/1.1 407` is,
  so the message naming which two options to set is not shown for the older version.
- **A target with no port is dialled on `443` whatever its scheme**, rather than the scheme
  deciding between `80` and `443`. ArmoniK deployments always name a port, so this one is
  unlikely to matter in practice.

The first two are already tracked by [hyperium/hyper-util#300](https://github.com/hyperium/hyper-util/pull/300)
and by ArmoniK.Api issue #702. The tests and the reproductions exist; what is missing is the
patch and the pull request.

It is a phase of its own and not a clause of T5.1 because it is work in another repository, on
another project's review cycle, and neither its schedule nor its outcome is ours. Nothing here
waits on it: the tripwires are what makes waiting safe.

**Deliverable**: a pull request per defect, or a stated reason for not carrying one.

---

## Dependency graph

```text
                T0.1 → T0.2 → T0.3 → T0.4 (TLA+, parallel to everything else)
                                        │
T1.1 ─────────────→ T1.2 ←─────────────┘ (FFI after proof)
  │                    │
  │                  T1.3 → T1.4 → T1.5
  │                    │
  │                  T2.1, T2.2 → T2.3
  │
  └── T3.1 → T3.2 → T3.3 → T3.4 → T3.5   (the options, and everything waits on them)
                                     │
              ┌──────────────────────┼──────────────────────┐
              │                      │                      │
        T4.1 → T4.2 → T4.3     T5.1 → T5.2, T5.3      T6.1, T6.5, T6.6 → T6.7 → T6.8 → T6.9
          └──→ T4.4                                     │
                                                  T6.2 → T6.3 → T6.4
```

T4.1 is what unblocks phases 4 and 5 alike: the proxy needs the same connector the TLS work
gives the engine. T6.1 decides a ceiling before T6.4 builds what it bounds, and T6.2 stands on
T1.1 alone, so it can run early.

---

## Parallelization

- **Phase 0** (TLA+) advanced in parallel with Phase 1 (Rust code)
- **T1.1** (Rust channel) had no blocking prerequisite
- **T1.2** (FFI) waited for T0.2 (FFI spec written) — the full proof (T0.4) is ideal but the
  written spec is enough to start implementation
- **Phase 3 is the bottleneck and is not parallel with anything.** Every option of phases 4, 5
  and 6 reaches a .NET caller through the schema it produces, so the three phases after it are
  parallelizable with each other and none of them with it.
- **T6.6** (packaging) can start as soon as T4.1, since what it packages is the engine
- **T6.2** (deadline) stands on T1.1 alone and can run at any point
- **Phase 7 waits, though its prerequisites are met.** T7.1 needs T1.1 and T2.3 and both are
  done, so it could start now. It does not: phases 3 to 6 may still move the surface it would
  adapt to, and adapting twice costs more than waiting once.
