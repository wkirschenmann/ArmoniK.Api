# TASKS — .NET ArmoniK Client on Native Rust gRPC Channel

## Philosophy

We start from main. We build the shortest path to a .NET unary call → real gRPC server.
Each task is a functional commit that adds a tested capability. The existing PR stack
(#711–#747) is a source of code to pick from, not a prerequisite to integrate in bulk.

The deliverable is a functional PR stack. Nothing is merged into main before complete
end-to-end validation, except a bug fix that stands on its own, which may go to main directly.

The TLA+ proof comes before the FFI implementation.

---

## Context: existing PRs

The stack (#711 → #747) builds `armonik-transport` incrementally. **It is discarded**, and
what is wanted is taken from it by the task that needs it, when it needs it: the option units,
the proxy, the TLS beyond what this crate already carries, and their tests. It was organized for the old architecture - an FFI transport over HTTP/2 with no
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

The nine checks it runs are outside CI for the same reason, decided rather than pending:
they need a JVM and `tla2tools.jar` on every pull request of a repository whose other
work never touches this specification. So they are the author's step, run before a change
to these documents, to the TLA+ modules or to the C header - which is also why each of
them prints what it counted rather than only whether it passed.

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
**Status**: done.  The crate also gains the `http2` module this task assumed it already had. Two
items of the list above did not survive: there is no executor trait - a `Spawner` puts a runtime
handle in the shape hyper asks for, and design.md says why - and the framing written here is
tonic's client since 2026-09-28.

### T1.2: Create `armonik-transport-ffi` — minimal unary ABI

**Prerequisite**: T1.1, T0.2 (FFI spec proved or at least written)
**Source**: from scratch per the design. The crate of that name on `wip/rust-all` is not the
source: it exposes a polling ABI, where the design calls for a callback ABI.
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

**Status**: done, apart from arm64, which has no runner here to build or run on. The mapping
carries it; the packaging packs its engine only if one was built, and silently leaves it out
otherwise, since each asset is packed under an `Exists()` condition. The first CI job on an arm
host will say whether the mapping is enough. The
matrix is 15 tests over three runtimes and two architectures, and `test.yml` runs seven
combinations of runtime, architecture and operating system.

The set delivered here is not the set the package promises. Requirement 8.1 names eleven runtime
identifiers; `RustTargets.props` declares seven of them and omits `linux-arm` and the three musl
ones, while `linux-x86` - which the requirement used to ask for - is gone, .NET publishing no
runtime for it. Of the eleven, CI builds three: win-x64, win-x86, linux-x64. Closing that gap is
its own task, T6.6: each musl triple needs its own toolchain, and the CI matrix, the Framework copy
step and the loader have to derive from the props table instead of repeating it.

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

- arm64 is mapped, but has never been built, packaged or executed. There is no runner here; the first CI
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
- arm64 is unchanged from phase 1: mapped, never built or executed.

---

## Phase 3 — The option surface, and the artefacts that carry it

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
prefix - `GrpcClient__TcpKeepaliveInterval` - and every value is text. Reading a unit under a
prefix, and rewriting its schema to match, is that second shape's work, and the FFI path needs
neither because nothing is flattened there. Producing the flat shape for a .NET caller is .NET's, which binds its own
configuration sources; producing it for a Rust caller is `from_env`'s, which stays.

**And the two shapes are read with different strictness, on purpose.** The JSON is typed by its
schema: a boolean field takes a JSON boolean, a number takes a number, and the string `"true"` is
refused there. A schema that promises a type and then accepts anything spelled like it is not a
contract, and the generated C# type has no reason to write anything else.

The wide spellings belong to the sources that carry no type at all - the command line,
environment variables, and any configuration file the Rust side may read one day. `1`, `True`,
`on`, `y` and their opposites are what a shell, a C++ program, a Python script or an INI file
writes, and refusing them would make an option unusable from whichever of those a deployment
happens to use. `read_env_bool` reads that vocabulary.

So one set of types is read two ways, and how is an open question rather than a settled one:
serde attributes are fixed on the type, and the lenient readers cannot simply be attached to
fields that must also deserialize strictly. Whether that is two representations, a deserializer
chosen per source, or something else is T3.2's to answer, and it should be answered before the
units are written rather than after.

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

**Status**: done, then undone on 2026-09-28. The module landed with its tests ahead of its
consumers - T3.2 declared the channel's options with plain derives, and of its five hundred lines
one reader was ever called, the boolean vocabulary `read_env_bool` reads. The rule since is that
what is used once is written where it is used: that vocabulary is `read_env_bool`'s own, and the
module is gone with the three dependencies it alone held. What T4.1 and T5.1 need of it they write
when they need it.

### T3.2: The channel's own options, as the first unit

**Prerequisite**: T3.1
**Commit**: the options the engine already has - the endpoint, the user agent, the connect
timeout, the message size it accepts, the delivery credits and the sends it allows in flight -
declared as types and read strictly from a structured JSON, replacing the hand-written
`ChannelSettings` the FFI crate carries today.

**No other unit arrives here.** TLS, proxy, retry and the TCP and HTTP/2 knobs each come with the
feature that reads them: a unit landing three phases before anything consumes it is the same
mistake as machinery landing before its first reader, and the proxy unit alone is twelve hundred
lines nothing would call. Each of those tasks brings its unit, its entry in the schema and the
property generation puts on the C# type, so the option surface grows a feature at a time and is
never wider than what works.

What this task settles for all of them, being the first: how a nested unit is declared, how its
constraints are stated so the schema carries them, and how its flat name falls out of its nesting
path. The reading is strict and only strict - the text sources never cross the FFI, so the
question T3.1 left open does not arise here. Names come from the mechanism, field name and
`rename_all = "PascalCase"`, and nowhere from a per-field rename.

**Deliverable**: the FFI crate's hand-written settings type is gone, the engine's options travel
as structured JSON, and `grep -c rename` on the unit answers 1, the container attribute.

**Status**: done in 5ee33e44, and two items of the deliverable read differently from what shipped.
The FFI crate keeps a `ChannelSettings`, now a wrapper over the typed `ChannelOptions` that checks
again the bounds a host may not have validated. And `grep -c rename` answers one container
attribute per struct - two, `ChannelOptions` and `TransportOptions` - and no per-field rename.

### T3.3: The schema, and the C# type, as build artefacts

**Prerequisite**: T3.2
**Commit**: `schemars` on the Rust types, emitted by a cargo target, and **the schema is
committed**. A Roslyn source generator turns it into the options class, driven by an attribute
that names it in the source rather than in a project file:

```csharp
[GenerateFromJsonSchema("options.schema.json")]
partial class ChannelOptions { }
```

The schema is committed because a source generator runs at design time too, and a schema that
only existed after a cargo build would leave a fresh clone with no type and a red IDE. The price
is that it can go stale, so the build checks it: regenerating from the Rust types and comparing
is a step, and a difference fails the build. Staleness is detected rather than impossible - the
trade the attribute is worth.

An earlier console tool exists, `ArmoniK.Api.TransportOptionsGenerator`, and is where this
starts: `--schema`/`--output`, deterministic by design - "the same schema always gives the same
bytes" - with its documentation generation and its fixture tests. What it assumes is the shape
this plan discarded: one flat vocabulary, every property a `string`, `anyOf` branches unioned.
Retargeting it at the structured shape is the work; its machinery and its tests survive.

**Status**: done, and **the generator is not a Roslyn one** - the only part of this task that did
not survive being tried.  Three measurements settled it:

- A Roslyn generator has to read JSON, and an analyzer loads inside the compiler, where
  `System.Text.Json` and its transitive assemblies conflict with what MSBuild already holds.  The
  documented failure (roslyn#41785, #66124) is green under `dotnet build` and red under Visual
  Studio - the one shape no CI catches, and one no measurement here could clear either.
- `Corvus.Json.SourceGenerator` has that solved (zero dependencies on netstandard2.0), but the
  types it emits are `readonly partial struct` readers over a `JsonElement`.  T3.4 binds
  `IConfiguration`, which needs a class with settable properties, so its output would have been
  wrapped by a hand-written mutable facade - leaving Corvus buying only a validation the Rust
  reader already performs, at the price of `NodaTime` and five more packages in every consumer of
  `ArmoniK.Api.Client`.
- Generators cannot read each other's output (roslyn#55104): they all receive the same input
  compilation, so no generator could have read a Corvus-generated struct in the same assembly.
  And a facade generated from the struct would carry neither the bounds nor the documentation,
  which live in Corvus's emitted validation code and in XML comments, not in metadata.

So `ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator` is a net8.0 tool that runs at build time
and reads the schema with **Corvus's `TypeDeclaration` model** - the resolution engine without the
emitter.  Corvus resolves `$ref`, `$defs` and the draft's rules; this repository decides the C#.
Nothing of Corvus reaches a consumer, since the tool never ships.

What that costs against the plan is the decorator and the `partial`: the class is named by the
tool's `--output` rather than by an attribute.  What it keeps is everything the decorator was
for - the schema committed, the C# generated from it and nothing else, and a build that fails on
a stale file.  `CheckGeneratedOptionsMatchTheSchema` renders and compares rather than rewriting,
so a build never edits the tree it is building.

**And a duration names its unit.**  `ConnectTimeoutSeconds`, not `ConnectTimeout`: see the draft
list above for why ISO-8601 was refused.

**The round trip this task asks for is covered in three places rather than one.**  A single test
spanning C# and Rust would need the channel to accept a `ChannelOptions`, which is T3.4's to add -
`NativeRuntimeFactory.Channel` still takes a window and nothing else.  What holds meanwhile:

- `ChannelOptionsTests` asserts the exact JSON each option produces, `{}` for a document that
  names nothing included.
- `armonik-transport-ffi`'s `config` tests read those documents back, so the far side of the
  boundary is exercised on the same bytes.
- and `every_option_the_schema_declares_is_one_serde_reads` closes the gap neither of those can
  see.  The chain from the Rust types to the C# is two generated steps, each with a freshness
  test, so a misspelling cannot survive it - but `schemars` and `serde` are two separate readings
  of the same fields, and this crate makes them differ on purpose with `schemars(with = "i32")`.
  That test builds a document from the schema's own properties and hands it to `serde`, where
  `deny_unknown_fields` refuses any name the two stopped agreeing on.

The one test that starts a channel with every option set is T3.4's.

The schema describes the structured shape - nested objects, booleans as booleans, numbers as
numbers - because that is what the generated C# type has to serialize to, and a schema that said
`string` everywhere would generate a class that says nothing about what it holds. It is also what
makes the JSON strict: the type in the schema is the type the reader enforces.

**The shape a draft settled**, so it is written here rather than rediscovered:

- **A duration is a number of seconds, and the option's name carries the unit** -
  `ConnectTimeoutSeconds`, not `ConnectTimeout`. A `Duration` derives `{ secs, nanos }`, which is
  a memory layout rather than anything a document writes. ISO-8601 was weighed and refused: it is
  the registered `format: duration`, and it would justify a `TimeSpan`, but `exclusiveMinimum` is
  a numeric keyword that a string instance makes *ignored* rather than violated - so the schema
  would silently stop stating that zero is refused, against the rule that every constraint which
  can be said in the schema is said there. It also admits `P1Y`, which is not a fixed duration, so
  a conforming validator would accept documents the engine refuses. The unit therefore lives in
  the name, where it costs no converter on either side.
- **A count is an `int`, not a `uint`, and a check is what states the constraint.** `uint`
  excludes a negative but not zero, and zero is the value that actually breaks a window or a
  credit; it buys half the check while costing CLS compliance and a binder that handles `int`
  naturally. The constraint lives in the schema instead: `#[schemars(range(min = 1))]` emits
  `"type": "integer", "format": "int32", "minimum": 1`, the generated class checks it, and the
  transport refuses by option name regardless - one constraint, stated where a reader looks and
  enforced where it matters.
- **Every constraint that can be said in the schema is said there.** `minimum` and `maximum` for
  a range, `pattern` for a shape, `enum` for a closed set, `minLength` for what may not be empty.
  A constraint written in the schema reaches the generated class, the generated documentation and
  any other consumer of the vocabulary at once; one written only in the transport reaches none of
  them and is discovered at run time.
- **The schema is the generator's every input.** Nothing is passed beside it: the type and its
  format give the C# type, `minimum` gives the check, `description` gives the XML doc, `$defs`
  and `$ref` give the nested classes, and a property absent from `required` is an optional one.
  A generator that needed a second input would be a second place for the vocabulary to live.
- Nothing is nullable. Unset is absent, never `null`, so the C# side writes with
  `JsonIgnoreCondition.WhenWritingNull` and the schema offers no null branch to generate against.
- `additionalProperties: false` everywhere: an unknown option is refused rather than ignored, or
  a stale spelling fails silently and late.
- Nothing is required, and `{}` is a valid configuration. That holds because the endpoint - the
  one value a channel cannot be created without - crosses the ABI as its own argument instead of
  as an option that happens to be mandatory. `ak_channel_create` gains it, which is a change to
  the header and therefore to the contract; `layout.rs` is what notices.

**Documentation rides the schema, and that costs four things.** A rustdoc comment becomes the
`description` of its property or its type, so an option is documented once, in Rust, beside the
field. Turning that into XML doc comments is the generator's work:

- The first paragraph becomes `<summary>` and the rest `<remarks>`; the blank line rustdoc and
  the schema both preserve is the separator.
- Markdown becomes XML: backticks are everywhere in this repository's style and have to reach the
  IDE as `<c>`, not as backticks.
- Intra-doc links resolve to nothing in C#. The harvested `strip_rust_details` did this for the
  flat names of the discarded design and has to be adapted: to the generated property's name when
  the link names an option, and to plain text when it names anything else.
- And a rule on the writing rather than the generator: **an option's doc comment describes the
  option, not the implementation that reads it.** It lands in a .NET consumer's tooltip, where
  this crate's memory layout is noise. Whatever is about the implementation goes in an ordinary
  comment beside the code.

**What is typed and what is not.** A type is used where it says something true and a string
where the truth is stated elsewhere. Durations, counts, sizes and booleans are typed: their shape
is nearly all of their constraint, and a `bool` cannot be misspelled at all - which is why the
wide boolean vocabulary belongs to the text sources and not here. An endpoint, a certificate
path, a proxy address, a rate limit spelled `100/1s`: strings, because their constraint is
semantic and the transport is the only place that can state it. Typing does not have to be
complete to be worth having - it moves a class of errors to the .NET binder, which names the
configuration path, and leaves the rest to the transport, which names the option.

**Deliverable**: the hand-written `ChannelOptions` is deleted and its replacement is generated,
with its documentation. Round-trip test C# -> JSON -> Rust over every option, and a stale schema
fails the build.

### T3.4: The .NET side loads, and only loads

**Prerequisite**: T3.3
**Commit**: bind `IConfiguration` - appsettings, environment, command line - onto the generated
type, in .NET's own precedence order, and serialize the whole of it. No option is read on the
Rust side of the FFI boundary.

**The flat name of an option is its nesting path, joined by `__`.** .NET's configuration already
maps that separator onto a section, which the repository relies on today with
`GrpcClient__Endpoint`, so a nested unit costs nothing to reach:
`GrpcClient__Transport__ConnectTimeoutSeconds` binds to `Transport.ConnectTimeoutSeconds` with no code of ours.
The Rust side derives the same name from the same path rather than declaring a prefix per
embedding - two embeddings of one unit have two paths by construction, which is what declaring
them was for.

**And this is what dissolves the strict-versus-lenient question on this path.** The text sources
never reach Rust across the FFI: .NET binds them into the typed object and serializes structured
JSON, so the FFI deserialization is strict and only strict. The lenient readers serve the crate's
own Rust consumers, which hold a config rather than a document. The question returns only if the
Rust side takes the same layering, which is the item left for later.

**Deliverable**: a test setting the same option in two layers and asserting .NET's order decides;
a test that an option set only in the environment reaches the engine.

**Status**: done.  `NativeRuntimeFactory.Channel` has three doors now - a `ChannelOptions`, an
`IConfiguration`, and the delivery window alone - and the first is what the other two build.  The
configuration one takes a *section* rather than a root: which sources a configuration is composed
from and which of them wins is .NET's to resolve and the host's to compose, so a caller hands over
`configuration.GetSection(...)` and this binds what is there.  Nothing of ours sits between the
layers, which is what the two-layer test asserts.  The environment test sets all five options
and makes a call, so it is also the test T3.3 owed: an option the engine stops reading under its
name refuses the channel there.

**`ChannelOptions` is public.**  A caller fills it in, and T6.8 will construct one from another
assembly; the generator emits `public sealed class` for that reason.  Which makes the
documentation requirement of T3.3 load-bearing rather than tidy: these are the tooltips a .NET
caller reads.

**And a bound was reachable by one door only.**  `MaxDeliveryCredits` - far tighter than the
schema's, because every call sizes a ring from the number - was checked in the window overload
and nowhere else, so an option arriving through a configuration was bounded only by the schema.
It is checked wherever a window arrives now.

**What the binder does with a name no option matches is drop it.**  That is .NET's behaviour and
not this binding's to change, so a misspelled *configuration key* is ignored while a misspelled
*option in the document* is refused by `additionalProperties: false` - the path a Rust or C++ host
takes.  Stated as a test, because the difference is easy to assume away.

**And nothing of the options surface is written by hand any more.**  T3.3 left a partial beside
the generated file holding the `JsonSerializerContext` and `Encode`, on the grounds that how a
document crosses the ABI is not something a schema says.  Both are mechanical given the root
group's name, and leaving them out cost two hand edits the day the classes became public - so the
generator renders them, the partial is deleted, and the classes are `sealed` rather than
`partial`: a second part would be something the schema does not decide, and the place for that is
the generator.

### T3.5: Align the vocabulary by configuration

**Prerequisite**: T3.4
**Commit**: the prefixes and the structure are configurable, so the names line up with the
existing client's where a counterpart exists rather than through a hand-written table. The
generated type is a superset: there are many more options here than `GrpcClient` carries today,
and the ones without a counterpart simply have none.

**Deliverable**: every `GrpcClient` option reaches its generated counterpart, and the options
with no counterpart are listed rather than silently extra.

**Status**: done, and the first half of that deliverable had nothing to do.  Measured rather than
assumed: of `GrpcClient`'s twenty options, **none** has a counterpart in this vocabulary today.
Seventeen wait on a phase that has not run - `AllowUnsafeConnection`, `CaCert`,
`OverrideTargetName`, `CertPem` and `KeyPem` on T4.1, `CertP12` on T4.2, the three proxy ones on
T5.1, the four retry ones on T6.3, `RequestTimeout` on T6.2, and the three keepalive and idle
ones on T4.1 - and three are answered by something that is not an option at all: the endpoint
crosses the ABI as `ak_channel_create`'s own argument, `HttpMessageHandler` names the handler
grpc-dotnet should use and this engine *is* the handler, and `ReusePorts` is a socket option of
that handler.  In the other direction all five of this channel's options are its own.

So there is no table of names to align, and configurable prefixes have nothing to configure - the
caller hands over the section, so the prefix is the caller's, and the structure is the schema's.
What ships is the deliverable's second half, as a gate rather than a document:
`OptionVocabularyTests` refuses an option on either side that has no entry, and each entry either
maps a name, names the phase that will, or says what answers it instead.  That is what makes
phases 4 to 6 move a name out of `Awaited` rather than quietly leave it there.

It earned itself on the first run, finding `CertP12` - which a grep over `GrpcClient.cs` had
missed, because the pattern excluded digits.

---

### T3.6: One ABI, generated from the Rust

**Prerequisite**: T3.3, whose arrangement this copies. **Before T4.0.**
**Commit**: `cbindgen` emits `include/armonik_transport_ffi.h` from `abi.rs` and `csbindgen` emits
the P/Invoke half of `NativeMethods.cs`; both artefacts stay committed, and the build regenerates
and compares, so a difference fails it. The prose the header carries - the two rules, the
ownership sentences, the per-symbol contracts - moves into `cbindgen.toml`'s `header` and into the
Rust doc comments, which is where a generator can carry it.

One C ABI is written out five times today: `abi.rs`, the header, `NativeMethods.cs`,
`tests/layout.rs` and `AbiLayoutTests.cs`. It is here rather than after phase 4 because T4.0
changes seventeen declarations and two structs at once, and making that change by hand is the
fifth transcription of one contract. Generating first makes the ABI break the generator's first
proof instead of its first casualty. The header's own comment - "written by hand and committed, so
an ABI change shows up in review rather than in a generated file nobody reads" - is answered by
the comparison step and not by the hand: review sees a diff either way, and only one of the two
arrangements can drift in silence.

The generated declarations are what the binding then loads, so they are verified rather than
trusted:

- **The .NET layout tests stay.** `Marshal.SizeOf` and `Marshal.OffsetOf` measure what the CLR
  does with the declarations, per target framework and per architecture. A generator proves the
  declarations agree with the Rust, not that the runtime lays them out as Rust does.
  `tests/layout.rs` is the Rust-side half and can be derived; `AbiLayoutTests` is a measurement,
  and it runs on x64 alone today where requirement 8.1 promises eleven runtime identifiers.
- **What csbindgen does with the shapes this ABI actually uses**: a struct passed and returned by
  value, `ak_bytes` as a by-value argument, the delivery callback as a function pointer, and every
  enum as an `int`. Each is a place where a declaration can be right in the header and wrong in
  C#.
- **What is not in the Rust at all**: the calling convention, `SetLastError`, and the netstandard
  half's `unsafe` signatures. Whatever the generator does not emit is what this repository's own
  template adds, and the template is then the thing under review.

**Deliverable**: the header and the P/Invoke declarations regenerate byte for byte from `abi.rs`,
checked by a build step as `CheckGeneratedOptionsMatchTheSchema` is; the layout tests pass
unchanged on net4.7, net4.8 and net8.0.

**And design.md is split by register once this lands.** With the ABI's reference in the Rust,
the declarations the Rust carries leave design.md; what it specifies and nothing builds yet stays
until it is built, `ak_error` first, which T4.0 builds from it. What remains mixes a normative
contract, the reasoning that led to it, implementation sketches and the proof strategy, as the
document says of itself near its end, and it splits as that passage says: the contract, the
normative ABI, the implementation architecture, the formal model and a decision log. Layer 4 then
keeps the obligations a reader has to hold rather than the walkthrough of one method.
`check_abi_coverage.py` moves with the declarations: it compares design.md's with the header's,
and would report every function that left as one the header declares and design.md does not
describe. What it keeps is the rest - every acting function in the level-1 mapping table, and the
table of which argument becomes what, wherever the split puts them - and `NOT_BUILT` still names
what design.md specifies and nothing builds, `ak_error_release` among it. The other gates under
`tla/ci` that read design.md by section - the derived invariants, the property manifest and the
sketch actions - move with what they read.

---

## Phase 4 — TLS and secure connection

### T4.0: The ABI says why it refused, and becomes additive

**Prerequisite**: T3.4
**Why it is here and not in the original plan**: the five branches of T4.1's connector fail in ways
only a message tells apart - a CA file that is not there, a PEM that does not parse, a handshake the
peer refused, a certificate name no override can satisfy. A status code carries none of them, so
requirements 11.1, 11.2 and 11.5 are unreachable through this ABI until it carries a message. This
comes before the branches that need it rather than after.

**Commit**: `ak_error` and `ak_error_release` as the ABI section states them, a nullable
`ak_error *out_error` on every entry point that can fail, and the errors the engine already builds
flattened into `detail` instead of collapsing into `AK_STATUS_INVALID_ARG`. The configuration reader
stops answering with an `Option`, which is where the reason for a refusal is discarded today.
`snafu::Location` leaves the message that crosses the ABI - requirement 11.4 - and stays in the
tracing record.

In the same change, while it is still free: `ak_runtime_config` and
`ak_call_start_options` gain `version`, `flags` and reserved fields validated to zero, and their
size check becomes a minimum instead of an equality. That is what makes T6.2's deadline field an
addition rather than a break of every host compiled before it, and it is why this task precedes
T6.2 as well.

`AK_ABI_VERSION` stays 1. The ABI is not published, so no host outside this repository is
compiled against it and a change is not yet a new version. That stops being true the day the
library ships, and so does changing a record's layout for free.

**Deliverable**: a configuration document refused over a named key produces that key's name in the
message, read from C and from .NET; a host passing NULL for `out_error` causes no allocation; and a
struct one field longer than this library knows is accepted with the unknown tail ignored.

### T4.1: The engine takes the real connector

**Prerequisite**: T4.0, T3.5, T1.1
**Commit**: widen `GrpcChannelConfig` past `{ transport, user_agent, max_sends_in_flight,
max_recv_message_size }` and let `connect.rs::https_connector` replace the plain connector of
`http2.rs`, which refuses `https://` by construction.

This one task delivers what the August plan cut into five, because they are branches of one
function that already exists and is tested: system roots, an explicit PEM CA, `OverrideTargetName`
with its IPv6 and its option-naming error, the insecure opt-in, and mTLS from a PEM pair. It also
carries what the old T5.6 asked for - keepalive, nodelay, keepalive interval and retries, connect
timeout - because the same connector sets them.

It brings the `tls`, `tcp_keepalive` and `http2` units with it, in the shape T3.2 settled: their
entries in the schema, and the properties generation puts on the C# type.

**Deliverable**: a unary call over HTTPS, and one test per branch of the connector reached from
the engine rather than from the connector's own tests. Every option of those three units reaches
the engine from a structured JSON.

**And the flow-control windows become options.** They belong to the `http2` unit this task
brings, as audit-response.md's H-009 decided. Their defaults are hyper's, 2 MiB for a stream and
5 MiB for the connection, stated in each option's description and applied by the engine's reader
as design.md has it for every default, so that a change of hyper's does not change them; T6.7's
benchmarks may choose others. The options state a hazard the host manages, not a rule a default
keeps: the connection's window is shared by every stream of a channel, a call its host does not
read holds up to one stream window of it, and nothing bounds how many calls a host leaves unread.
Enough of them stop the others on the channel receiving.

**And whether a duration reaches .NET as a `TimeSpan` is settled here**, before this task adds
the keepalive and idle durations beside `ConnectTimeoutSeconds`. A caller setting the option in
code holds a `TimeSpan`, and a `double?` of seconds lets `TotalMilliseconds` compile as readily
as `TotalSeconds`. But the configuration binder reads a `TimeSpan` as `d.hh:mm:ss`, where "5" is
five days and "2.5" fails, and a configuration is where most callers set it. `GrpcClient` already
exposes its six durations as `TimeSpan`, `KeepAliveTime` and `RequestTimeout` among them, so that
reading is the one its callers meet today. The document keeps its number of seconds, as design.md
has it for every duration; what is settled is the C# type.

**The options generator gains each shape when an option first needs it.** It emits five
scalar shapes today, and TLS's options may want an enumeration, a list or a choice between groups.
Each arrives with the option that uses it, and not ahead of it.

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
**Commit**: the proxy types and the CONNECT tunnel, with the `proxy` unit that configures them -
the largest of the units, and it arrives here because this is where something reads it. `secrecy`
takes the password fields, and the stack's URI handling replaces the `safe_endpoint` this crate
carries.

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
of them - one for a call that answers once, one for a stream. Neither is declared yet: they arrive
with the `retry` unit T6.3 brings. What is left is what they mean.

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

**A fourth question belongs here, because it shares the subject: whether this binding starts using
the send window's depth.** The window is a memory bound on the arena and it is live for any host -
at most `MaxSendsInFlight` buffers lent at once, charged at the lend, refused past it with
`AK_STATUS_SLOT_BUSY`. What a depth above one buys is pipelining, and not of the network: the
serialization of message N+1 overlaps the transmission of N, which is a gain on a saturated link
as much as an idle one.

This binding exercises one, and that is decided rather than missing - design.md says "native depth
allows MaxSendsInFlight; this binding exercises one, the writer being single and completing at
WRITE_DONE".

That decision lives at level 2 and constrains nothing above it. `DotNetBinding` describes this
binding, which is why its writer is one state machine with no `SLOT_BUSY` action; level 1 already
carries the general case - `HasFreeSendSlot`, `RefuseLendForSlot`, and a `submitted` sequence that
`HasNoSendInFlight` compares against the acquittals emitted. So the ABI's guarantees do not move
if this binding starts using the depth.

Changing it is three artefacts in this order: that layer-4 decision, then level 2's writer becomes
multi-slot and its refinement proof is redone, then the binding returns at the commit instead of the
acquittal and `LentBuffer`'s default branch becomes a wait.

**The window and the replay cache do not constrain each other** (2026-09-28, superseding the
decision that the window had to be the larger). That decision rested on a replay resending the
arena original through the send window. The engine takes tonic's encoder instead, which copies
each message out of the arena, and a replay resends that copy: it takes no slot in the window, so
the two values are chosen independently and no unit relates them.

### T6.2: Deadline

**Prerequisite**: T4.0, T1.1
**Commit**: a local timer per call, the `grpc-timeout` header transmitted, expiry cancelling the
call and answering `DEADLINE_EXCEEDED`.

T4.0 comes first because the deadline is a field of `ak_call_start_options`, and a field changes
that struct's size: while the size is checked for equality, adding it refuses every host compiled
before it.

It also lifts `MustCarryNoDeadline`, which is today the only `Unimplemented` the binding opposes
to an ordinary caller.

**Deliverable**: a deadline expires and the status says so; a caller's `CallOptions.Deadline` is
no longer refused.

### T6.3: Retry for a call that answers once

**Prerequisite**: T6.1, T6.2
**Source**: the retry types from the #7xx stack
**Commit**: exponential backoff, retryable codes, and the `retry` unit that configures them. A
single message is replayable without a buffer, so this cardinality needs none of its own - but
the unit declares **two** replay ceilings, one for a call that answers once and one for a stream,
because they bound different things: a single message kept in case it has to go again, against a
whole sent prefix. One value would either starve the stream or let a unary call reserve a
stream's worth. T6.1 has already settled what happens when either is reached.

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
**Commit**: the eleven runtime identifiers of requirement 8.1 in `RustTargets.props` - it gains
`linux-arm` and the three musl ones - with the CI matrix, the .NET Framework copy step and
`NativeMethods.EngineDirectory` derived from that table rather than each carrying its own list.
`dotnet pack` producing a package that resolves on each platform, and arm64 executed at last
rather than only mapped and packed.

**Where the work actually is: the two armv7 targets.** GitHub offers hosted arm64 runners for public
repositories - `ubuntu-24.04-arm` and `windows-11-arm` - and this workspace uses `ubuntu-latest` and
`windows-latest` and nothing else. Where those runners are available, win-arm64, linux-arm64 and
linux-musl-arm64 are built and executed natively with no cross toolchain at all, which closes the
oldest gap this document carries: T1.5's arm64, mapped and never once built or run. `osx-x64`
and `osx-arm64` have hosted runners too. What no hosted runner covers is 32-bit armv7 - `linux-arm`
and `linux-musl-arm` are cross-compiled and stay unexecuted, and that is the honest tier boundary.

Musl costs less than it looks, and for three reasons that are independent of each other - none of
them the C ABI, which rustc emits on its own.

**Why musl is a separate asset at all**: `std` is compiled per `target_env`, and `target_env` is
part of the triple - `rustc --print cfg` answers `gnu` for one and `musl` for the other. What this
library holds is `std`, tokio and hyper, whose reactor wants `epoll`, whose pool wants pthreads and
whose sockets and clock go the same way, so the C library is linked whatever the crypto does. A
shared object linked against glibc does not load on Alpine. This reason survives dropping TLS and
survives a pure-Rust crypto backend.

**Why every target needs a C compiler**: `rustls` reaches `ring`, which is not written in Rust
alone - 107 C and assembly files its build script hands to `cc`. That is also the whole reason a
Windows build needs `cl.exe`. The crypto itself is nearly freestanding, including two libc headers
and no allocation, but `cc` still has to produce code for the target, so a musl target needs a
musl-targeting compiler rather than the host's. And `ring` is not libc-free on the two arm targets:
it reads CPU capabilities through `getauxval`, and branches on the libc flavour to do it.

**Why the musl builds need a flag**: `crt-static` is on by default on those targets and on no gnu
one, which statically links the C runtime into a `cdylib`. They need
`-C target-feature=-crt-static`.

`EngineDirectory` picks between `x64` and `x86` on `IntPtr.Size` alone, which answers for the two
platforms .NET Framework runs on and for nothing else; the table has to reach it.

**Deliverable**: a package that works on every runtime identifier it claims, and one table that
every consumer of the list reads.

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
and a consumer picks by calling `NativeRuntime.Channel` instead of
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

**And `ArmoniK.Api.Common` has to be split before this ships.** T3.4 took a reference to it for
`ConfigurationExt.GetRequiredValue`, which is four lines - and Common also compiles 28 `.proto`
files, the whole ArmoniK message set, plus `Grpc.Net.Client`, `ArmoniK.Utils` and
`System.Diagnostics.DiagnosticSource`. The reference is a real package dependency of the produced
nupkg, on both target frameworks, so every consumer of the transport pulls the API surface the
transport is meant to be independent of - `design.md` says the channel serves any generated stub,
and `ChannelOptions.g.cs` is rendered for a trimmed or native-AOT host.

What that asks for is a small assembly holding what both sides need and nothing else: the
configuration reader, and whatever else turns out to be shared once the two vocabularies meet.
Measured rather than assumed - `dotnet sln`'s own nuspec is where the dependency was read.

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

### T6.10: Received messages count against the memory ceiling

**Prerequisite**: none among the tasks above. The level-1 model changes first, as for anything the
ABI promises.

**Why**: the runtime's ceiling bounds only the buffers a host fills to send. A message the engine
receives and lends to the host is counted for quiescence and not in bytes, so what a runtime holds
on the receive side is bounded per call - (`DeliveryCredits` plus the few messages the engine reads
ahead of them) times `MaxReceiveMessageSize` - and not at all across calls. A client downloading
large chunks on many calls at once can exhaust the process's memory with every bound respected.

**Commit**: two thresholds over the one count of bytes that sends and receives then share.

- A received message is charged its length, from the moment it is decoded until the host gives it
  back with `ak_event_consumed`. The slack a message may keep alive in tonic's decode buffer is
  taken as negligible.
- The first threshold is where work waits. A call about to read its next message while the count
  is past it stops reading, and HTTP/2 flow control stops the peer, as when a call is out of
  delivery credits; it reads again when a release takes the count back below. A send buffer is
  admitted against this threshold as it is against the ceiling today: `AK_STATUS_BUDGET_BUSY`
  while the lends in flight leave no room, `AK_STATUS_MESSAGE_TOO_LARGE` when the message alone
  exceeds it.
- The second threshold is where the engine stops, so that the process does not run out of memory.
  Calls that each found the count below the first threshold before reading can pass it together,
  by up to one message each. A decoded message that would take the count past the second ends its
  call with `RESOURCE_EXHAUSTED`, and is freed at once.
- A call refused with `AK_STATUS_BUDGET_BUSY` is woken by an event at the next release that could
  make room, a send buffer's or a received message's. It fires where the count falls - a send
  buffer's release, at its WRITE_DONE or when it is given back unsent, and a received message's
  at `ak_event_consumed` - and never at a commit, which moves bytes from the host to the runtime
  without freeing any: design.md kept a poll until now because a signal on that edge is a
  wake-up that never comes. The .NET binding's poll of `ak_runtime_memory_usage`, every 2 ms,
  goes.
- The memory categories gain one, the received messages the host holds, and the partition level
  1 proves (`CategoriesPartitionTotal`) gains it too.
- design.md is rewritten where it says the budget covers the emission path only, that a fallible
  receive path is not this design, and that the budget's wake-up is a poll. Its reasons were
  that receive-side bytes belong to hyper and that a failed allocation aborts; neither holds
  against a count of messages already decoded, which needs no fallible allocation.
- The runtime's own options - its worker count and its thresholds, however the second is set -
  join the generated vocabulary: a schema, a default stated in each description, and a binding
  from `IConfiguration`, as a channel's options have. Today the worker count and the one ceiling
  are two bare parameters of `NativeRuntime.Create`.
- `AK_ABI_VERSION` does not change, for the reason T4.0 gives.

The models change first: level 1 charges a payload its length and gains the two thresholds and the
event, and level 2 replaces the binding's poll by the event.

What to settle:

- how the second threshold is set - a second field of `ak_runtime_config` beside
  `memory_ceiling`, or derived from the first - and both defaults; today's ceiling defaults to
  four gigabytes, or half the address space where that is smaller;
- the event: its name, whether it wakes every call refused since the last release or one of them,
  and that it carries no payload and takes no delivery credit, as WRITE_DONE does not;
- whether a caller must tell the second threshold from a message past `MaxReceiveMessageSize`.
  Both end the call with `RESOURCE_EXHAUSTED`, the first transient and runtime-wide, the second
  permanent, and only the status message tells them apart - which a retry policy reading the
  code, T6.3's, does not see.

**Deliverable**: many calls receiving messages the host does not read end with some of them
waiting and none past the second threshold, read from `ak_runtime_memory_usage`; a message that
would cross the second ends its call with `RESOURCE_EXHAUSTED`; a send refused with
`AK_STATUS_BUDGET_BUSY` resumes on the event when a received message is given back, with no poll;
and the runtime's options are read from a configuration as a channel's are.

---

## Phase 7 — Rust ArmoniK Client on armonik-transport

### T7.1: Adapt the Rust ArmoniK client to use the `grpc` module

**To revisit before it starts** (2026-09-28): design.md decides that the client drives the
engine's HTTP/2 service directly, so the adapter the commit line below describes is not the plan.
What that decision means for the Rust client code on Florian's branch is to be analysed first.

**Prerequisite**: T1.1, T2.3 (functional Rust channel with all 4 cardinalities)
**Commit**: Replace the direct Tonic dependency in the Rust ArmoniK client with an adapter
that consumes `armonik-transport`'s `grpc` module. The generated Tonic stubs work via a
`Channel` adapter
that delegates to `GrpcChannel`. The Rust client and the .NET binding share the same native
gRPC engine.

**Deliverable**: Rust ArmoniK client tests pass using `armonik-transport` instead of
Tonic directly. Same functional behavior.

This is also where the crate stops carrying two disjoint stacks. Until here the Rust client
compiles fifteen mandatory dependencies where it compiled eight - `h2`, `http`, `http-body-util`,
`bytes`, `base64`, `tokio` with five features and `tower-service` - for an engine it
does not use, and the only code the two stacks share is `chain` and `safe_endpoint`. That is paid
deliberately rather than gated: a feature gate over the engine would be removed by this task, and
a feature position nothing exercises rots before then.

**And the client offers what it means to.** `pub use armonik_transport as transport` makes the
whole engine part of the Rust client's public API; it gives way to the items the client offers.
And the client's configuration converts into the engine's: `TransportConfig` carries an endpoint
and a connect timeout, and nothing converts into it. What becomes of an option the engine has not
got by then is settled here - `RateLimit`, which no task builds, and whichever of
`TcpNagleAlgorithm` and `Http2MaxHeaderListSize` T4.1's units leave out: each is built, or
refused by the client, and none is read and ignored.

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

### T9.2: What tonic's client answers against the gRPC documents

**Prerequisite**: none
**Commit**: open a tracking issue in this repository, as #702 is for T9.1, then carry upstream
what this repository has measured and pinned.

The engine is tonic's client over an HTTP/2 session of the channel's own, and tonic 0.14.6 gets
eight cases wrong for a client. Each is worked around in `grpc/channel.rs`, `grpc/driver.rs` or
`grpc/status.rs`, and each workaround is pinned by a test that fails without it:

- **A message past the receive limit is OUT_OF_RANGE.** The gRPC status table gives
  RESOURCE_EXHAUSTED, and lists OUT_OF_RANGE among the codes the library never generates.
  Remapped by matching tonic's message, the only thing telling it from a status the peer sent.
- **Trailers that arrive inside a message end the call with their status**, so a message the peer
  cut short is dropped under an OK. Answered with INTERNAL by following the length prefixes.
- **Any status in the response head is taken for Trailers-Only**, so messages behind it are
  dropped under that status. Answered with UNKNOWN by reading the body to its end first.
- **A non-200 answer's body is decoded as gRPC**, so a proxy's HTML error page is reported as an
  invalid compression flag rather than through PROTOCOL-HTTP2's HTTP-to-gRPC table.
- **A malformed `grpc-status-details-bin` panics.** `Status::from_header_map` decodes it with an
  `expect`, which ends the task driving the call. The header is removed before tonic reads it.
- **An announced length is reserved before the limit can refuse it**, so on a 32-bit target with
  no limit a peer's length past what an address spans panics the reserve. The limit tonic is given
  is held under half the address space.
- **A GOAWAY's reason is read as a reset's**, under tonic's `server` feature: h2 hands it to every
  stream the GOAWAY ends, so a stream a server going away cleanly never processed is INTERNAL,
  where PROTOCOL-HTTP2 has the client take it as UNAVAILABLE and retry. The engine's table reads
  no reason from a peer's GOAWAY, pinned by
  `a_stream_a_peers_goaway_left_unprocessed_is_unavailable`, and keeps the reason of one h2 sent
  itself, pinned by `a_stream_ended_by_a_goaway_this_side_sent_keeps_its_reason`.
- **The RST_STREAM reason is read only under tonic's `server` feature**, so a client built without
  it reports every reset as UNKNOWN. The engine keeps its own table.

The last one is also a trap here: the Rust test build links tonic with `server` through a
dev-dependency, so the Rust suite cannot see whether the engine's table is needed. The .NET suite
loads the library as it ships, and `AStreamThePeerResetsCarriesTheReasonItWasResetWith` is what
pins it.

The workarounds are what makes waiting safe, so nothing here waits on upstream, and a pull request
to tonic is not this project's work of the moment.

**Deliverable**: an upstream issue or pull request per defect, or a stated reason for not carrying
one - and, as each is fixed in a release, the workaround it made unnecessary removed.

---

## Phase 10 — Logs and observation across the ABI

### T10.1: What a host can see of the engine

**Prerequisite**: T3.5
**Commit**: study, then whatever it concludes.

The ABI lets a host observe two things - `ak_call_debt_of` and `ak_runtime_memory_usage` - and
tells it nothing else. The engine has `tracing` spans and events inside it that reach nobody, so a
deployment that misbehaves gives a .NET operator no more than a status code, and the questions an
operator actually asks - which endpoint, which call, how long, why it retried - have no answer on
that side of the boundary.

It is recorded here rather than left implicit because the ABI is a contract: adding to it later
is a header change, and the shape it takes should be chosen once rather than grown by accident.

What to settle:

- **How an event crosses.** A callback per event is the shape the rest of the ABI already uses,
  and it has the same rule: it must not allocate on the host's behalf, and it must be total. The
  alternative is a drained queue like the delivery ring, which costs a thread and buys batching.
- **What an event carries.** A level, a target, a message, and structured fields - and fields are
  where a C ABI gets expensive, because a `tracing` event's fields are typed and dynamic. A
  rendered line is cheap and lossy; a field array is faithful and costs an allocation per event.
- **Who filters, and where.** Filtering on the host side means every event crosses the boundary
  including the ones nobody wants; filtering in the engine means the host has to be able to say
  what it wants, which is another option and another call.
- **What is measured rather than logged.** Calls in flight, bytes in the ledger, dials, retries:
  counters a host can poll are cheaper than events it must consume, and the two observation
  points that exist are already that shape.
- **What a secret must never reach.** The engine holds endpoints, proxy credentials and
  certificate paths. `safe_endpoint` exists because a URI can carry a password; a logging path
  that bypassed it would undo that.
- **The binding's own surface, which is the same decision.** `ArmoniK.Api.Client.RustGrpcChannel`
  has no `ILogger`, no `EventSource` and no trace anywhere in it. Most of what it catches it does
  report - a decode failure reaches the reader, an unreadable header reaches
  `ResponseHeadersAsync`, a teardown that failed reaches whoever awaits the disposal and is kept
  in `refused_` - and the two exceptions are both in the trampoline: a context handle that no
  longer names a target, and a `Publish` that threw. Those two catches do not go, and are not the
  defect: the trampoline runs on a tokio thread and an exception crossing back into Rust is
  undefined behaviour, so the catch is the boundary. What is missing is that the boundary says
  nothing, and a call stranded there is the one failure with no observer at all. Whether that is
  an `EventSource` or an optional `ILoggerFactory` on `NativeRuntime.Create` is chosen
  with the engine's own crossing rather than beside it - a host with two unrelated diagnostic
  channels for one call is what deciding twice produces.

**Deliverable**: a decision recorded in the design, and the ABI extension it calls for.

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
                       └── T3.6 → T4.0   (the ABI generated, then broken once)
                                     │
              ┌──────────────────────┼──────────────────────┐
              │                      │                      │
        T4.1 → T4.2 → T4.3     T5.1 → T5.2, T5.3      T6.1, T6.5, T6.6 → T6.7 → T6.8 → T6.9
          └──→ T4.4                                     │
                                                  T6.2 → T6.3 → T6.4
```

T4.1 is what unblocks phases 4 and 5 alike: the proxy needs the same connector the TLS work
gives the engine. T6.1 decides a ceiling before T6.4 builds what it bounds. T4.0 is upstream of
both T4.1 and T6.2 for two different reasons: T4.1's failures need a message to be distinguishable
at all, and T6.2 adds a field to a struct whose size is checked for equality. T3.6 is upstream of
T4.0 and of nothing else: it stands on T3.3 for the arrangement and not on the option surface, so
it is the one task of phase 3 that runs in parallel with T3.4 and T3.5.

---

## Parallelization

- **Phase 0** (TLA+) advanced in parallel with Phase 1 (Rust code)
- **T1.1** (Rust channel) had no blocking prerequisite
- **T1.2** (FFI) waited for T0.2 (FFI spec written) — the full proof (T0.4) is ideal but the
  written spec is enough to start implementation
- **Phase 3 is the bottleneck and is not parallel with anything.** Every option of phases 4, 5
  and 6 reaches a .NET caller through the schema it produces, so the three phases after it are
  parallelizable with each other and none of them with it. The exception is T3.6, which touches
  no option: it needs T3.3's regenerate-and-compare arrangement and T4.0 needs it.
- **T6.6** (packaging) can start as soon as T4.1, since what it packages is the engine
- **T6.2** (deadline) stands on T1.1 and T4.0: nothing about a timer waits on the option surface,
  but the field it adds waits on the struct being able to grow
- **Phase 7 comes after 3, 4, 5 and 6.** T7.1's declared prerequisites, T1.1 and T2.3, are met,
  so it could start at any time; it does not, by decision. Adapting the Rust client to a surface
  those phases are still moving would mean adapting it twice, and a client that could not offer
  TLS would be adapted before it was useful - the engine speaks in the clear until T4.1.
