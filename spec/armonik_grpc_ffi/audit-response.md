# Response to the external audit of `wk/feat/phase1`

An external audit of this branch delivered 1 617 findings: 32 blocker, 375 major, 671 minor,
539 nit, of which 118 are pre-existing. This file records what was done with them, one line per
finding touched, so a reader can tell an applied fix from a refused one from a deferred one without
reading the audit.

**The inventory is authoritative over the narrative.** The generated `inventaire.json` counts 32
blockers and 375 majors where the audit's own README says 33 and 374, and its component reports
contradict its transverse report in places. Every finding below was re-derived from the code before
being acted on; where the audit's stated mechanism turned out to be wrong, that is recorded too,
because a wrong mechanism with a right conclusion is still a finding to fix.

Statuses used here:

| | |
|---|---|
| **applied** | reproduced, fixed, and the fix proved by a test or a command |
| **answered** | a decision settles it; the decision is in `requirements.md` or `design.md` |
| **refused** | re-derived and it does not hold; the measurement that says so is given |
| **deferred** | holds, and belongs to a task that is not this one; the task is named |
| **open** | holds, and what settles it is a decision nobody has taken; the decision is in design.md's open-decisions table with its cost on both sides |

## The blockers, at a glance

| | count | |
|---|---|---|
| applied | 18 | the four version tags, the lock file, a path Linux cannot read, the archive's schema, two numbers a host may send, the prologue that answers the headers with its two test gaps, an internal type that stopped escaping, a send ceiling that is waited on, and a panic that no longer strands what its task promised |
| answered by a decision | 5 | the platform set, the ABI's error channel, its size check, and two the absence of a publication channel dissolves |
| re-derived, refused or downgraded | 2 | the HTTP/2 window, whose evidence holds and whose number does not; and the shutdown's debt decision, whose race cannot happen |
| open | 8 | |

Of the eight open, none is a hang or a crash any more. What is left is one release path, one
pre-existing defect in a component this branch does not touch, two refactors, one test gap, and
two that wait on decisions recorded above.

**What the re-derivation is finding, over twenty-five blockers so far: the audit's evidence lines hold
and its conclusions need redoing.** Three of its claims were wrong on the number or the consequence
while right about the mechanism, one described a race the code already closes, and one - the
headers deadlock - was under-described rather than over: it missed that the formal model already
prescribed the fix and that an invariant was vacuous against the code. So each finding is reproduced before it is believed, and what the reproduction
says is written down beside it.

---

## Answered by the five decisions

Recorded in `requirements.md` 8.1, 11.6, 13.5 and 14.9, in design.md's open-decisions table, and in
tasks.md T3.1, T4.0 and T6.6. Commit: "the audit's four decisions, taken as five".

| Finding | What it said | Status |
|---|---|---|
| L-019 | the RID table claims seven identifiers, CI builds three | **answered** - requirement 8.1 names eleven, `RustTargets.props` owns the table and T6.6 makes the others derive from it |
| R-013 | the ABI has no channel for an error message | **answered** - `ak_error` in design.md's ABI section, built by T4.0 |
| R-024 | `read_versioned` demands an exact size, so no addition is additive | **answered** - the minimum check and requirement 13.5's fields, in T4.0, before T6.2 adds a field |
| Z-005, N-002 | `ClientConfig`'s public fields are retyped on a crate published as a patch release | **refused as stated** - nothing publishes these crates: `publish.yml` carries jobs for C#, Python, C++, Java and npm and none for cargo. The retyping is real and costs nothing today. It becomes Z-005 again the day a cargo publication channel exists, which is its own task |
| the audit's D4 | "the choice between `connect.rs` and a second TLS path appears in no task" | **refused** - tasks.md T4.1 states it verbatim, and the audit's own `rapport-spec-docs.md` quotes that task |
| not in the audit | one runtime per process was enforced in code and stated in the C header but in neither steering document | **applied** - requirement 14.9 |

---

## Applied: the build is green again

Commit: "the version tags, the lock file, and a path Linux cannot read".

| Finding | What it said | Proof |
|---|---|---|
| L-001, K-079, K-080, K-081 | the four new `.csproj` carry neither `<Version>` nor `<PackageVersion>`, which `verify-versions` requires of every `.csproj` it globs | `npm run verify-versions`: FATAL on the first of the four before, `Found 3.29.2 for all projects` after |
| L-003 | `update-versions` rewrites `Cargo.toml` and never `Cargo.lock`, and every CI cargo invocation passes `--locked` | bumped both version lines as the script does: `cargo metadata --locked` exit 101, "cannot update the lock file ... because --locked was passed". After the fix, `npm run update-versions 3.30.0` refreshes the lock and `cargo metadata --locked` exits 0 |
| K-058 | the two `AssemblyMetadata` paths use backslashes, which reach .NET unnormalised | `EchoServerProcess.ServerAssembly` calls `Path.GetFullPath` then `File.Exists`, so on the matrix's one ubuntu leg the whole tail is a single filename and every test needing the echo server throws `FileNotFoundException`. Forward slashes, as the sibling project's own comment already prescribes |

The audit preferred a different fix for the version tags - teach `_contants.ts` to skip
`IsPackable=false` projects. Measured against that: `ArmoniK.Api.Client.Tests` and
`ArmoniK.Api.Worker.Tests` are both `IsPackable=false` and both carry the two tags, so the
convention does not key on packability, and that fix would have dropped two pre-existing projects
out of the version sweep for no gain.

One defect found beside L-003 rather than in it: `_readAndReplace` writes the result and reports
success without ever checking that the pattern matched, so a moved or renamed version tag stops
being versioned silently. It fails now. The Rust dependency line is the case that mattered - its
pattern is a lookaround anchored on one exact string, and nothing else would have noticed.

---

## Applied: two numbers a host may send

Commit: "two numbers a host may send that end the call, or the process".

| Finding | What it said | Proof |
|---|---|---|
| A1-001 | `From<Seconds> for Duration` panics for a negative, non-finite or over-large value, and nothing bounds the option above | `{"Transport":{"ConnectTimeoutSeconds":1e300}}` is admitted by the reader and panics at `library/core/src/time.rs:964`. The conversion is `TryFrom` now and the reader holds the `Duration` it built; 9 passed in `config::tests` against 2 failures |
| A2-001 | `worker_threads` goes to tokio's builder with no upper bound | reproduced at the audit's own number: `memory allocation of 34359738360 bytes failed`, exit code 9 - the process ends, so `catch_unwind` never sees it. `AK_MAX_WORKER_THREADS` is 1024, in `abi.rs` and in the header, pinned by a layout test |

The audit's probe for A1-001 named `Seconds(-1.0)` alongside `Seconds(1e300)`. Only the second is
reachable through the ABI: the reader already refused `<= 0.0`. The finding held on its other half.

1024 is the one number here that is chosen rather than measured. A worker is an OS thread, so the
useful range is the machine's core count; past the bound the failures are thread exhaustion or that
allocation, and a host can act on neither.

Deferred from the same neighbourhood, with its mechanism: the schema states no maximum for
`ConnectTimeoutSeconds`, so the generated `Validate()` accepts a value the engine then refuses -
an error found late rather than early. That is the audit's "one bound, one place" (its lot 13),
which moves bounds into the schema and derives both sides from it, and it is a task of its own.

---

## H-009, the HTTP/2 window: the observation holds, the number does not

The audit's evidence is right and its conclusion is not, so this one is recorded rather than fixed.

**What it says.** "The HTTP/2 client is handshaked with no flow-control tuning, so it runs on
hyper's 64 KiB stream and connection windows with adaptive_window off. A 64 KiB window caps one
stream at window/RTT: ~64 MB/s at 1 ms RTT, ~6.4 MB/s at 10 ms - and .NET's SocketsHttpHandler
scales its own window to 16 MiB by default, so on any non-LAN path this engine is slower than the
managed client it exists to beat." Severity blocker, confidence high.

**The evidence half is true**: `http2::Builder::new(executor).handshake(io)` calls none of
`initial_stream_window_size`, `initial_connection_window_size` or `adaptive_window`, anywhere in
the crate.

**The number is wrong.** hyper 1.10.1, which `Cargo.lock` resolves, defaults its HTTP/2 *client* to

    src/proto/h2/client.rs:48  const DEFAULT_CONN_WINDOW: u32 = 1024 * 1024 * 5;   // 5mb
    src/proto/h2/client.rs:49  const DEFAULT_STREAM_WINDOW: u32 = 1024 * 1024 * 2; // 2mb

65 535 is `SPEC_WINDOW_SIZE`, and in hyper it is what `adaptive_window(true)` *sets* as its BDP
starting point - not a default. So the untuned engine has a 2 MiB stream window, and the same
arithmetic gives 2 GB/s at 1 ms and 200 MB/s at 10 ms rather than 64 and 6.4 MB/s. The conclusion
that the engine is slower than the managed client on any non-LAN path does not follow from it.

**What is left of the finding, which is real.** The window is a library default nobody here chose,
and the comparison with .NET is the other way round at the start and possibly the audit's way round
at the limit: `SocketsHttpHandler` starts a stream at 64 KiB and scales dynamically to 16 MiB, so
this engine is ahead of it immediately and behind it on a long fat pipe. Which of the two shapes is
wanted is a policy this repository has not chosen, and enabling `adaptive_window` is not free: it
drops the starting window from 2 MiB to 64 KiB in exchange for growing past it.

**Decided: the window becomes configuration, and later.** Not a number this repository picks - the
right one depends on the deployment's latency and on how many calls are in flight at once, and both
are the operator's to know rather than ours. The options belong to the `http2` unit T4.1 already
brings with it, in the shape T3.2 settled, so the schema declares them and both sides derive from
it; T6.7's benchmarks are what say which default to ship. Severity as re-derived here is major, not
blocker.

---

## E-001 and E-008: the model already prescribes the fix, and an invariant is vacuous

The audit found this one and under-described it. Recorded here before it is built, because what it
turns out to be is not what it was reported as.

**What it says.** E-001: "`headers_` is only ever resolved from inside `MoveNext`, so on
server-streaming and duplex calls `ResponseHeadersAsync` cannot complete unless the caller is
already pumping the reader." E-008: ArmoniK's own shipped `WaitForResultsAsync`
(`EventsClientExt.cs:140`) awaits `ResponseHeadersAsync` on a server-streaming call before its
first `MoveNext` - so the binding cannot serve the client it exists for. Both verified; both hold.

**What the audit missed: `DotNetBinding.tla` already specifies the shape**, and has since phase 0.

    ConsumerPhases == {"prologue", "application", "drain", "done"}
    Init: consumer_phase = [c \in CallIds |-> "prologue"]

    ConsumeHeader(cId) ==                          \* an action of its own
        /\ consumer_phase[cId] = "prologue"
        /\ RingTail(cId) = 0
        /\ L1!HostConsumesEvent(cId)               \* the credit comes back here
        /\ headers_completion' = [... EXCEPT ![cId] = "succeeded"]
        /\ consumer_phase' = [... EXCEPT ![cId] = "application"]

    BeginParseEvent(cId) == /\ consumer_phase[cId] = "application"   \* slot 0 is not the app's

`ConsumeHeader` is enabled by the metadata arriving, not by a read. So the model resolves the
headers without the application pumping anything, which is exactly what E-008 needs.

**And the implementation has no `consumer_phase`.** `NativeCall.cs:362` says it plainly: "`Phase`
is the model's `reader_state` extended with the drain". The model has two reader variables and the
binding implements one. `grep -i prologue` over the whole binding returns a single line - a comment.
So `PrologueReaderOnlyWaits` and the `DotNetBinding_MCwitnessPrologue` witness hold over a variable
nothing implements: **vacuous against the code**. This is the same class of gap T1.5's status
recorded one level up ("the model's reader machine had no implementation, so the invariants over it
were vacuous"), and T2.2 closed that one by implementing `reader_state` while leaving
`consumer_phase` behind.

**The fold was deliberate, and its reason is real.** `NativeCall.cs:648`: "The prologue is consumed
inside a read, under that read's own registration, so a token firing while the head is outstanding
faults the read and the headers together rather than finding no operation to cancel." And the model
knows that race exists - `PrologueReadCancellationUnreachable` is stated negatively so its witness
run is *expected* to violate it, which is the trace proving a cancel can be pending while a call is
in the prologue with the reader waiting.

So both shapes owe an answer for a cancel with no read to fault, and the model already gives it:
`BeginDisposeCall` "faults a headers task still pending: no managed waiter survives a dispose", and
the binding has `FailHead` for it already.

**Applied, in the model's shape.** There was no decision to take: the model outranks an
implementation that improvised around it, and the improvisation is what made ArmoniK's own client
hang. `Phase` gains `Prologue` as its initial value, so the arbiter stays one word - holding the
ring is what makes the prologue exclusive, `MoveNext` finding it waits for the phase rather than
publishing behind it, `HandoffToDrain` refuses it as it refuses a parse in flight, and
`CancelAndDrain` faults the headers without moving the transition. A task rather than part of
`Publish`, because the header forbids parsing on the callback's thread.

    before: Failed TheResponseHeadArrivesWithoutAnyRead        [10 s]  (server streaming)
            Failed TheResponseHeadArrivesWithoutAnyRead        [10 s]  (duplex)
            Failed TheEventStreamAnswersItsHeadBeforeItIsRead  [10 s]  (ArmoniK's own stub)
    after:  all three passed; the suite is 81 where it was 78

Each fails at its full timeout rather than on a value, which is what a deadlock looks like. The
echo server gains `HeadOnly` and `HeadThenChat` - response headers and no message at all - because
a stream with nothing to read is what tells a read apart from no read. This is the change the audit
predicted the echo server would need.

Sequencing note: A3-056 (effort L) wants this 1036-line class split into a ring, a read machine, a
sender and a thin call. The prologue landed inside it, because a split moves code rather than
changing it while a deadlock is not deferrable.

K-027 - "`ResponseHeadersAsync` is tested for the unary cardinality only; the three streaming ones,
where the branch's known deadlock lives, have no test" - closes with it, and its own note that the
echo server needed changing first was right.

---

## Applied: an internal type stops reaching application code

| Finding | What it said | Proof |
|---|---|---|
| C-001, K-029 | `IClientStreamWriter<T>.WriteAsync` returns the call's task unchanged, so the binding's internal `CallEnded` reaches application code instead of an `RpcException`; and no test writes to a request stream after the call's terminal, which is the only path that raises it | the echo server gains `CollectRefused`, a client-streaming call refused without reading. Before: `Expected: instance of <Grpc.Core.RpcException> But was: <...Calls.CallEnded>`. After: passed |

---

## Applied: the send ceiling is waited on, not allocated around

| Finding | What it said | Proof |
|---|---|---|
| I-001 | on `AK_STATUS_BUDGET_BUSY` the binding allocates the refused bytes on the managed heap and copies them into the arena at commit, so the ceiling arbitrated by the engine's ledger is overridden by the host | `spilled_ ??= new byte[length]` is gone from the ceiling's branch. The refusal is met in `SetPayloadLength`, which is called with the exact length before a byte is written, so the whole attempt restarts: `NoRoomYet` out of the serialization, the send loop waits for room and serializes again |

**Decided: the ceiling is backpressure, so nothing is allocated on the .NET side.** `BUDGET_BUSY`
says wait, the way `SLOT_BUSY` says wait for a call's window, and answering a wait with an
allocation answers it with the one thing it exists to refuse. What made the allocation look
necessary is that the refusal arrives inside a `SerializationContext` override, which cannot await;
what makes it unnecessary is that the same override is handed the exact length up front, so the
attempt is restartable rather than half-done.

`spilled_` stays for `SerializationContext.Complete(byte[])`, where a marshaller hands over an array
allocated before this binding saw it. design.md's decided row claimed "nothing to copy, and no
managed heap to fragment" while the ceiling branch did both; it now names that one exception.

No test exercises the ceiling, and the audit says so itself in A3-146: nothing drives the memory
ceiling to exhaustion or the send window to its limit. Such a test needs a ceiling sized to one
message and two sends in flight, in a runtime of its own - `Configure` is refused while one exists.
Named here rather than left implied.

**And the same principle leaves a question, whose first answer was wrong.** `AK_STATUS_SLOT_BUSY` is
the other backpressure status, and `LentBuffer.Take` drops it into its default branch as
`RpcException(Internal)` - a failure where the header says "backpressure, not an error; retry when a
WRITE_DONE arrives". It is unreachable as this host is built, because `WriteAsync` returns only at
the acquittal and there is never more than one send in flight.

Two readings were recorded here before this one and both were wrong. The first was that
`MaxSendsInFlight` has no effect and is worth removing. The second was that the formal model
forbids using it and would have to change first. What design.md actually says settles both.

**The window is a memory bound, and it is live.** design.md: "The send window bounds the memory the
call's arena lends out: at most `MaxSendsInFlight` buffers at a time, counting both those the host
is still filling and those already committed and awaiting their WRITE_DONE. The slot is charged when
the buffer is lent rather than when the message is committed, because the allocation is what costs
memory." So the option bounds an arena for any host, and `AK_STATUS_SLOT_BUSY` is how the ABI
refuses past it, synchronously and without blocking.

**What a depth above one buys is pipelining**, and it is not the network that gains: the
serialization of message N+1 overlaps the transmission of N. That is a gain on a saturated link as
much as an idle one.

**And this binding exercising one is a stated decision, not a gap.** design.md, in the layer-4
sketch: "native depth allows MaxSendsInFlight; this binding exercises one, the writer being single
and completing at WRITE_DONE", and beside the writer's completion source, "No slot counter and no
send signal: one writer completing at WRITE_DONE never finds the window full, so there is nothing to
wait for". So `LentBuffer`'s default branch is unreachable by a decision that is written down.

**What that decision does not do is constrain what may be built, and reading it as though it did was
a category error made twice here.** `DotNetBinding` is level 2: it *describes* this binding, which
is why its writer is one state machine and why it has no `SLOT_BUSY` action. The model that
constrains is level 1, the ABI's, and it already carries the general case:

    HasFreeSendSlot(cId) == SendWindowOccupancy(cId) < MaxSendsInFlight
    RefuseLendForSlot(cId, len) == ... /\ ~HasFreeSendSlot(cId)
                                      /\ last_lend_status' = "SLOT_BUSY"
    HasNoSendInFlight(cId) == /\ write_dones_emitted[cId] = Len(submitted[cId])

`submitted` is a sequence, so several messages in flight are modelled, and the `SLOT_BUSY` refusal
with them. Nothing about the ABI's guarantees moves if this binding starts using the depth.

Revisiting it is therefore three artefacts in order: design.md's layer-4 decision, then level 2's
writer becomes multi-slot and its refinement proof is redone - level 1 needs nothing - then the
binding returns at the commit instead of the acquittal and `SLOT_BUSY` becomes a wait.

**Whether that happens now is the user's call.** It is a feature the ABI already permits, it
interacts with the replay ceiling phase 6 has to settle - a retry needs the sent bytes kept past
their acquittal - and none of it is a correction. Recorded against T6.1, which owns that subject.

---

## Applied: a panic in a task no longer strands what the task promised

| Finding | What it said | Proof |
|---|---|---|
| R-053 | the spawned reader, writer and shutdown tasks are not wrapped in `catch_unwind`, so a panic there kills the task silently and the call never reaches its terminal, the channel never closes and the runtime never quiesces - a hang rather than the error requirement 14.8 promises | `guarded` in `lib.rs`, and each task's tail now runs on both paths. 3 tests on the guard itself, the load-bearing one being a body that panics *after* a suspension - which a catch around the whole future would never see, and which is why the entry points' synchronous `guard` cannot serve here. 201 Rust tests where there were 198 |

The reader's terminal moved out of the guarded body, so a panic while reading answers `Internal`
and the terminal still goes out. The writer's acquittal likewise: the reader waits on it before the
terminal, and a panic that skipped it used to leave that wait to the oneshot's drop - the same
outcome by accident rather than on purpose.

**And the shutdown task gets `AK_RUNTIME_FAILED_UNQUIESCED` its first producer.** A panic there is
the worst of the three: no `SHUTDOWN_COMPLETE` goes out and no teardown thread starts, so the
runtime answers STOPPED for the life of the process and refuses every destroy. The header already
defines the state for exactly this - "quiescence impossible, destroy refused" - and design.md
recorded that nothing set it. Now something does, which is also what the .NET side's status polling
needs to stop waiting.

Two things this does not recover, named rather than implied: a panic between a message reaching the
wire and its acquittal leaks that send's window permit and its charge against the ledger, so a
runtime that meets it may not empty its ledger again; and a panic inside a call cannot know what
that call owed. Removing the hangs is what this lot does.

---

## Applied: the vocabulary gate watches the third reader

| Finding | What it said | Proof |
|---|---|---|
| N-021 | the `GrpcClient__*` environment namespace has two disagreeing readers - `ClientConfigArgs::from_env` and the .NET `Options.GrpcClient` - and the branch's new `OptionVocabularyTests` compares the .NET one against the schema while never looking at the Rust one | measured: the Rust reader knows 18 names, `Options.GrpcClient` declares 20, and the intersection is 6 - `AllowUnsafeConnection`, `CaCert`, `CertPem`, `Endpoint`, `KeyPem`, `OverrideTargetName`. Exactly the audit's count. The gate reads the reader's own calls now, and the divergence is pinned: removing one entry fails with `But was: < "TcpKeepalive" >` |

**The divergence is pinned rather than reconciled, and deliberately.** Some of the twelve
Rust-only names are the same concept under another spelling - `TcpKeepalive` against
`KeepAliveTime`, `Timeout` against `RequestTimeout` - and which spelling wins is the #736
vocabulary question, not this branch's. What the gate now catches is a name moving on one side
alone, which matters because an unknown option is ignored rather than refused: drift there fails
late and silently everywhere else.

The names are read from the reader's `read_env` calls rather than mirrored in the test, because a
list kept in the test is the drift it exists to catch. The audit suggested a `pub const NAMES` in
Rust for the same purpose; reading the calls needs no second source of truth to keep in step.

One thing worth recording because it was nearly missed: a memory of this repository said these
divergences were "recorded in the crate README". They are - in the #7xx stack, which has not
landed. On this branch nothing recorded them, which is why the gate is the right place for them
rather than prose.

---

## Refused: A2-002, the shutdown's debt decision

**What it said.** "The shutdown's host-debt decision reads only `Ledger::empty()`, which a lend
already committed to inside `fill` has not yet incremented." Fix proposed: fold every call's
`debt.buffers`/`debt.payloads` into the decision, and gate `ak_get_call_buffer` on `pass_the_gate`.

**The window it names is real.** `lend` claims `debt.buffers` with a compare-exchange before it
calls `fill`, and `fill` is what charges the ledger, so there is an interval where a call shows a
buffer claim and the ledger shows nothing. And `finished()`, which the shutdown awaits per call,
waits on `Debt::quiet()` - the terminal and the callbacks - not on `Debt::settled()`, which is what
reads `buffers` and `payloads`. So the shutdown does read the ledger with that window open.

**It still cannot be wrong, for two reasons that are in the code.** The shutdown cancels every call
*before* it awaits any of them, and `fill` opens with `accepts_work()`, which is
`live() && !cancelled`. So a lend inside the window fails, charges nothing, and `lend` puts
`debt.buffers` back - the host is refused and holds nothing to return. And the terminal's own
payload is charged by `lend_payload` before `debt.terminal` is stored, so the flag the shutdown
waits on is released after the charge it would otherwise miss.

A lend that *succeeds* between the gate closing and the calls being cancelled charges the ledger,
and the read that follows reports `MUST_RETURN` - correct, and why gating `ak_get_call_buffer` is
not needed either.

**Status: refused, both halves.** No test is added for a race that cannot happen; what would earn
its place is a test that `quiet()` and `settled()` stay different, which is the property this rests
on - noted for the test-gap sweep rather than done here.

---

## The writer's fix, and the worse defect it nearly carried

The obvious version of this fix would have introduced a worse defect than the one it cures.
Answering from `await call_.TerminalAsync`, as the unary path does through its drained task, waits
on a read the caller may never make: a writer that only writes would hang inside `WriteAsync`
instead of hearing why it failed. So the terminal is used only when a read has already consumed
one, and `Unavailable` otherwise - which is what `NativeCall.Start` already answers for the same
two refusals.

---

## Applied: the archive holds the file its own tests read

| Finding | What it said | Proof |
|---|---|---|
| L-007 | `options.schema.json` is committed beside the crate and read by `include_str!`, but the crate's `include` list does not name it | `cargo package -p armonik-transport --list`, diffed before and after: exactly one line gained, `options.schema.json`. The reader is `src/options.rs:149`, inside `the_committed_schema_is_the_one_the_types_describe`, so what an archive without it breaks is that test rather than a consumer's build - narrower than the finding implies, and still an archive whose own tests do not compile |

---

## Decided in discussion, and written down rather than left there

Six things were settled in conversation and existed nowhere in the repository. They are in
design.md's open-decisions table and in tasks.md now; the rows below cite them, and this section
carries the reasoning a table cell cannot hold.

**The premise that dismissed four of the audit's findings was mine.** I had ranked its
off-the-shelf findings against a rule that the engine avoids tonic. No requirement says that, no
design section says it and no task says it - `armonik-transport`'s own manifest depends on tonic
for `channel` and `codegen`, and design.md's T7.1 adapter is typed `type Error = tonic::Status`.
Withdrawing the premise moves F-002, F-004, F-005 and F-003 from out-of-mandate to real, and
leaves one argument standing where I had four.

**That argument is the send buffer's owner, and it reaches the codec alone.** The zero-copy send
has the host serialize straight into a Rust arena allocation, and **that original is the replay
cache**, not a copy taken from it. tonic's `Encoder` writes into a buffer tonic owns, so adopting
it means one copy per message into the arena and a replay cache that is either a second copy or
the wrong bytes. None of that touches the receive side, where `Decoder<Item = Bytes>` is the shape
`Payload` already carries, nor the metadata map, nor the status decoding, nor the pool. I had the
direction of that copy backwards - claiming a throughput gain for it - and the direction is the
whole argument.

**Re-derived, each of the three tonic findings is smaller than it looks, and one is a reason not
to reuse.** For the metadata map, tonic's two base64 engines and its five-name reserved list are
both `pub(crate)`, so what reuse buys is `MetadataMap` with `get_bin`/`append_bin` and not the
constants the finding quotes as duplication - and three behaviours this branch built are absent
from it. For the status decoding, `Status::from_header_map` **panics** on a
`grpc-status-details-bin` the peer wrote badly, which is precisely the class of thing this library
spent a commit removing from its own tasks. For the pool it holds outright, and it is the largest
of the three. The rows below carry each measurement.

**No correctness objection survives against `System.Threading.Channels`.** I had three and all
three were wrong. A bounded `Channel<Slot>` does not allocate per payload - the element is a
reference to a native buffer, not the buffer. A queue that removes the item at the take does not
lose the release, because `ak_event_consumed` names the payload it frees, so the engine discharges
whatever comes back; the header caps how many payloads a call may owe and asks for no order among
them. And level 2's `RingHead`/`RingTail` are not a constraint: that level describes this binding,
and a refinement needs a mapping, which may be fictional. What is left is a trade with a cost on
each side, which is why it is an open decision and not a fix or a refusal.

**The ABI stops being transcribed by hand.** Generated from the Rust with the artefact committed
and verified the way `options.schema.json` is. It was proposed early and argued down, on the
ground the header's own comment gives - that a hand-written file shows up in review where a
generated one does not - and the comparison step answers that better than the hand does, because
review sees the diff either way and only the hand can drift in silence. T3.6 states it and sits
before T4.0, because T4.0 changes seventeen declarations and two structs at once and making that
change by hand is the fifth transcription of one contract.

**The binding's diagnostics belong to T10.1** rather than to a fix of their own. That task already
asks how the engine's `tracing` events cross the ABI, and the host's own surface is the same
question: a binding with two unrelated diagnostic channels for one call is what deciding twice
produces.

**And the channel's synchronous `Dispose` goes.** design.md says a synchronous dispose would have
to block on the network and on host callbacks and that the surface therefore offers none;
`NativeChannel` implements `IDisposable` all the same, so the path the document rules out is there
for a caller to find. Dropping it makes `await using` compiler-enforced instead of documented, and
separating `ShutdownAsync` from disposal goes with it.

---

## The majors, as they close

Of the 287 the inventory holds that are neither spec drift nor pre-existing, 101 are
accounted for in the table below - in 85 rows, because several rows carry two or three findings
that share one mechanism, one row carries a decision the audit never made, and A3-056 among the
ids is the blocker whose structural half these refactors closed.

Forty-nine rows are applied, nine of them in part - and what became of the other part is named in
each; thirteen are refused outright, one of those by a decision that is the user's; six are deferred - two to T4.0, where
their own fix points, one each to T6.10 and T10.1, one to the tasks that build what it lacks, and
one to T6.6; six are answered by a requirement, a decision or a row above that now states what they said was
unstated; four are open - they hold, and what settles them is a decision this document does not
take alone, so it is in design.md's open-decisions table with its cost on both sides; five were
already closed by an earlier fix that crossed the audit, and one by two rows above it; and one holds with its fix recorded for
whoever next touches what it lives in. Each applied row's proof is the transcript in the commit
that applied it.

| Finding | What it said | Status |
|---|---|---|
| Z-002 | the refusal message interpolates the endpoint as given, and an endpoint carrying `user:password@` is exactly what the engine refuses, so that message is the leak's own path | **applied** - `NativeChannel.Safely` renders `scheme://host[:port]`, which is what `safe_endpoint` renders on the other side |
| E-003, R-021 | every `CallInvoker` override takes a per-call `host` that overrides the channel's authority, and nothing reads it | **applied** - refused beside the credentials and the propagation token, for the reason the invoker's own doc comment already gives. Without the check the call *succeeds*, against a host nobody asked for: `Expected: <RpcException> But was: null` |
| D-008 | `DrainAsync` runs on a task nobody awaits and `settled_` is the first thing the settler waits on, so an exception leaving it is a call that never settles and a channel whose disposal waits for it | **applied** as a shape, not as a live bug: everything outside the old inner try is a `TaskCompletionSource` read, an index masked to a power of two, or a `TrySetResult`. The drain is the one path with nobody to hear it, so it is the one that must not rest on nothing throwing |
| D-006 | `CancelWith` reads `ending_.IsCancellationRequested` and then stores `cancellation_`, so a settler running `ending_.Cancel(); cancellation_.Dispose();` between the two leaves a live registration on the caller's token - and that holds the call, its ring and its buffers for as long as the caller's token source lives | **applied** - the read and the store are one step under `disarm_`, and the settler claims the field under it before disposing, so either order disposes exactly once. Two accessors of the field, both inside the lock. No test: forcing an interleaving between two adjacent statements needs a hook in the method, and that hook would be test-only code in the cancellation path |
| R-045 | the configuration door binds with `Get<T>()`, which drops a misspelled option in silence, while the same misspelling in the document is refused by `deny_unknown_fields` | **applied** - `ErrorOnUnknownConfiguration`, and the message names the key. Before: `Expected: <System.InvalidOperationException> But was: null` - the door opened a channel configured by nobody. The section holds the channel's options and nothing else now, which is worth knowing when the integration settles how ArmoniK's twenty `GrpcClient` options map onto these five (R-003) |
| B-001 | the endpoint was taken out of the `ClientConfig` debug span to keep a `user:password@` out of the log, and `override_target_name` - refused for `@` by the same function, a hundred lines below the span - was left in | **applied** - out of the span, and the comment says both fields now. The guard is a test that reads the span's own argument list, since what has to hold is a field's absence and a subscriber sees only the fields that are there. Putting it back: `FAILED ... no_field_the_userinfo_guard_refuses_is_recorded_in_the_span` |
| A1-004 | `GrpcChannel::new` checks only that `max_sends_in_flight` is non-zero, so a window past the semaphore's limit is accepted and panics later, at the first call | **applied** - refused against `LARGEST_WINDOW`, which the crate already names and already tests against `Semaphore::MAX_PERMITS`. The FFI door refuses the same range out of a document; this door takes a number and refused nothing. Removing it: `assertion failed: matches!(refused(LARGEST_WINDOW as usize + 1), ...)` |
| A2-003, D-002 | `ConnectTimeoutSeconds` is bounded only from below, so NaN and any value past `Duration`'s range reach `from_secs_f64` and panic | **already closed** by A1-001's fix: the conversion is `Duration::try_from(seconds).ok()?` in `config::parse`, and `<= 0.0` is false for NaN, which the `try_from` then refuses. Two of the audit's findings for one mechanism, both answered by the blocker's proof |
| I-003, G-012, R-047 | `config::parse` restates by hand every bound the schema declares, making a second copy of a rule the schema owns, and no test compares the two | **applied** as a test, three findings for one mechanism. The copy stays: nothing obliges a host to have validated its document, so the reader checks rather than trusts - which is what the function's own doc comment already says. What was missing is the comparison, and it now reads the bounds out of the schema and drives the reader at each edge. Removing one check: `MaxReceiveMessageSize is admitted below the minimum the schema states`. The two windows share `LARGEST_WINDOW` as a constant, so their maxima cannot drift numerically; the three `minimum: 1`s, the `minLength: 1` and the `exclusiveMinimum: 0.0` are literals on both sides and could |
| R-042 | the engine drops a response header whose key its alphabet refuses, and the reason given is that the .NET binding's `Metadata.Add` would refuse it - a host-specific policy in the shared engine | **applied to the reason, refused for the policy.** The rule is not .NET's: gRPC's Header-Name grammar is narrower than the HTTP token `HeaderName` accepts, so a key outside it is one no gRPC consumer can carry, and the sibling comment on `validate_key` already said so. What was host-specific was the justification, which named one witness as if it were the rule. The comment states the grammar now |
| F-030 | the `ak_status` to gRPC `StatusCode` mapping is written out in five places with five slightly different tables | **refused.** They are not five copies of one table; they are four different questions. `InvalidState` at `ak_call_start` is the channel going away under a call that raced its disposal, so `Unavailable`; the same status while serializing is *this* call having ended, so `CallEnded`, which the invoker resolves into the terminal the caller has to see; `MessageTooLarge` maps only where a size is refused, which is the lend, because `ak_call_send_message` cannot answer it; and the runtime and channel doors answer `InvalidOperationException` because there is no call to answer. Checked for the divergence the finding asserts: no status gets two different answers for one situation. What is spelled more than once is the pair `InvalidState or HandleStale`, four times, and naming it would trade a self-describing pattern for a predicate |
| R-029, I-006 | the ABI's "one unfilled buffer at a time whatever `max_sends_in_flight` says" is the .NET binding's single-writer model promoted into the contract, and it forecloses the only use of a send window above one | **refused, both.** The two are different quantities. What is capped at one is the number of buffers a host may hold *unfilled*; the window is how many messages may be sent and unacquitted. `commit` decrements `debt.buffers` while the permit stays spent until WRITE_DONE, so a host lends again immediately and serializes message N+1 while N is on the wire - which is exactly the pipelining the window exists for, and what design.md states at layer 4: "native depth allows MaxSendsInFlight; this binding exercises one". On the status: holding two unfilled buffers is a contract violation and answers INVALID_STATE, while a full window answers SLOT_BUSY from `window.try_acquire` - the backpressure code is already there, for the case that is backpressure |
| R-011, A2-023, A2-100 | one runtime per process is enforced by a process-global `LIVE` flag and three global registries, a restriction that appears in no requirement and in no design section | **answered.** It appears in requirement 14.9 now, added when the decision was taken: with one runtime, there is one channel factory, and several tokio runtimes in a process is what the decision refuses. The three findings ask for the constraint to be lifted rather than for it to be stated, which is the decision itself |
| N-033 | the userinfo refusal is implemented twice in one crate, with two error types, two messages and two different levels of correctness | **refused, and it is the same answer as B-003.** The two run at two different stages and must: `config.rs` tests the raw string because it runs *before* the parse, whose error carries the endpoint it could not parse, and `http2.rs` scopes the test to the authority because it is handed a `Uri` that parsed. Merging them means choosing one stage, and the earlier one cannot parse without putting a password in a message |
| R-022 | five exception messages interpolate the raw `ak_status` name, and one of them the endpoint | **half applied, half refused.** The endpoint is redacted, which was Z-002. The status name stays: it is the one token in the sentence that says which refusal happened, and a host reading `no buffer to serialize into (MessageTooLarge)` learns more from the name than from its absence |
| F-006 | the four cardinality overrides repeat the same four-line preamble and the same four-lambda tail verbatim | **applied to the preamble, refused for the tail.** The preamble is the duplication that already cost something: the per-call host had to be refused in four places by hand, and a refusal reaching three of them is one a caller dodges by choosing a cardinality. It is one `Started` method now. The tail is four delegates each constructor takes separately; hoisting them into a generic holder trades sixteen self-describing lambda lines for a class plus sixteen references |
| K-054, K-056 | four fixtures repeat the same server block verbatim, and `MockServerProcess` and `EchoServerProcess` are the same file twice | **applied to what is one thing twice.** The fixtures inherit the echo server's lifecycle, 204 lines out of six files against 111 in two new ones. The assembly lookup is one method keyed by name - the duplication that had already cost a double fix, since the recorded path is relative and both copies needed `Path.GetFullPath` - with `Kill` beside it. The two process classes stay apart: their readiness protocols differ, one polling an HTTP root for a port chosen in the test and the other reading the port the server printed, and the mock's own comment says why |
| G-005 | `DrainAsync` duplicates `MoveNext`'s slot interpretation minus the marshaller, and the two copies have already diverged | **applied to the divergence, refused for the merge.** They differ on an unreadable metadata blob: the read path faults the call, the drain swallows it, so `ResponseHeadersAsync` answers `Metadata.Empty` on a call that ended OK - "no headers" for "headers nobody could read". Unreachable today, because the blob is written by `blob.rs` and read by `RawMetadata`, and the keys are filtered to what both accept, so only an engine defect produces one. The drain faults them now, through the `FailHead` that already marks such an exception observed, and the sentence a terminal nobody could decode reports is written once instead of twice. Merging the blocks is refused: one arbitrates a read against a token and publishes a phase, the other latches the settlement, and a shared helper would take three parameters and a callback to serve both |
| C-007 | `MaxDeliveryCredits` is a bound only the .NET ring imposes and the schema does not state, so the same document is accepted by a C or Rust host and refused by this one | **applied** in c32d3ef4, and the asymmetry is correct. A binding may narrow what the ABI admits, and design.md now says so beside the options: each call allocates its ring at the next power of two above the window, so the binding's 32768 is 65536 slots, two megabytes per call in a 64-bit process, where the schema's bound sizes nothing on this side |
| I-007 | three timing policies - a 30 s shutdown ceiling, a 2 ms budget poll and a 1 ms quiesce poll - are private constants in the binding rather than options | **already closed** for the one with a consequence, by the A3-022 row below: the 30 s ceiling deciding entry into an absorbing state is gone, and the binding's wait for quiescence has no deadline. The intervals left decide nothing a host would want to set - a 2 ms poll for room against the ceiling, which T6.10 replaces by an event, a 1 ms poll for a thread join, and a 100 ms poll for a failure the engine stores without announcing it |
| I-012, R-049 | the runtime's own options bypass the schema pipeline entirely - no schema entry, no generated class, no `Validate`, no `IConfiguration` door | **deferred to T6.10**, which gives the runtime a second threshold and brings its options into the generated vocabulary. Both say the same thing: the runtime's configuration surface was built by hand while the channel's is generated. Making them agree means a second schema and a second generated class, which is the options generator's work and not a fix |
| L-088 | the .NET Framework deployment layout is written in the targets file and re-derived in C# | **deferred to T6.6**, which derives the .NET Framework copy step and `NativeMethods.EngineDirectory` from one table of runtime identifiers (the I-005 row below). Of the two prose copies the finding also counts, the exception's message reads `EngineDirectory` rather than restating it, and the csproj's comment names .NET's own `runtimes/<rid>/native` convention rather than this package's layout |
| R-043 | the metadata blob has two hand-written codecs with no round-trip test across the ABI | **applied** in 9164a7b3: `MetadataCrossesBothCodecsAsItWasSent` sends text, empty, repeated and binary entries, has the echo server list what it read and copy the entries into its head, and so reads each codec against the other in both directions. Measured on the way: grpc-dotnet's own `Metadata` would join a repeated key's values and lose an empty value before the wire, so the server copies header to header |
| R-034, M-055 | `check_abi_coverage.py` enumerates the ABI out of design.md rather than out of the header, so the gate compares that document to itself and can see neither a symbol the header declares and the design forgot, nor a declaration the design carries and nothing implements | **applied**, and the same defect filed twice. Measured: the header declares 17 functions and `lib.rs` exports exactly those 17, while design.md declared 18 - `ak_channel_status` missing, `ak_error_release` and `ak_runtime_memory_usage_detailed` extra. The gate reads the header now and checks four directions, and the first run found two facts nobody had: `ak_channel_status` is in the header, in `lib.rs` and in `NativeMethods.cs` with no description in design.md, and `ak_channel_create`'s `endpoint` - the one value a channel cannot be created without - was in neither design.md's declaration nor the argument table. Both fixed. The gate was also **red on this branch** and had been since the error-channel commit, which is the cost of M-054 measured on my own change |
| M-056 | the gate's `OBSERVATIONAL` list names `ak_runtime_memory_usage_detailed`, which no ABI declares, and its "listed as observational but the ABI does not declare it" check passes only because the ABI it reads is design.md | **applied, and its dependency dissolved.** It was waiting on the audit's M-007 to remove the function from design.md; that is not needed. design.md specifies it and says in its own words that it is not implemented, so the gate carries a `NOT_BUILT` list naming what builds each such function - `T4.0` for `ak_error_release` - and reports an entry that stops being true, the way the observational list already did. `ak_channel_status` joins `OBSERVATIONAL`, where it belongs: it reports a state and linearizes nothing |
| M-058 | a superseded copy of `check_theorem_statements.py` sits at the top of `tla/`, is run by nothing, misses two of the four declaration/proof pairs and exits 1 with two false STATEMENT DRIFT lines | **applied** - deleted. The live one is in `ci/`, which `check.sh` runs; it reports 111 declarations across four pairs where design.md's table still claimed 72 across three, so that number is corrected too |
| M-054 | nothing in `.github/workflows/` invokes `ci/check.sh` or any gate under `tla/ci/`, so the nine checks the documents call a build gate run only when someone runs them by hand | **answered by decision: they stay out of CI.** They need a JVM and `tla2tools.jar` on every pull request of a repository whose other work never touches this specification, and that is more than the gates are worth - the same argument tasks.md already records for tlapm itself. What was wrong was the documents' word, not the arrangement: three places said a check "fails the build", and tasks.md now states that the nine are the author's step, run before a change to these documents, to the modules or to the C header |
| A3-056 | `NativeCall` is one 1036-line class holding 22 instance fields, 2 enums, 2 nested classes and a nested struct, and implementing eight separable concerns | **applied, in three.** `DeliveryRing` is the queue and the borrow rule, `Sender` the outbound half, `Receiver<TResponse>` the inbound half with the phase machine and the response it produces; what is left in the call is its identity - 370 lines and ten fields. The cut is by ownership rather than by code region: whoever owns the slots releases them, so the drain went with the reader. Two couplings surfaced in the move - `ReadOp` was cancelling the call where it means to cancel its own consumer, and `Settled` was two things under one name, the latch the receiver completes and the task the call waits on. The state machine stays explicit by decision: it is the mapping to the model's `reader_state` and `consumer_phase`, and it goes when something else carries that mapping |
| Z-001 | `is_reserved` covers gRPC's reserved names but not HTTP/2's connection-specific fields, so a host can put `connection`, `transfer-encoding`, `upgrade`, `keep-alive`, `proxy-connection`, `host` or `content-length` into call metadata and they reach the wire | **applied** - refused at `validate_key`, where the caller learns the key is impossible. RFC 9113 section 8.2.2 makes such a message malformed and the peer resets the stream, which names no header. Before: `append_ascii("connection", "close")` answered `Ok(())` |
| Z-003 | `from_headers` filters inbound keys by the header alphabet but never by `is_reserved`, so reserved headers reach the host as response metadata - and metadata read out of the channel is then refused when handed back in | **applied** - one predicate, both directions. Before: `from_headers` yielded `["content-type", "grpc-accept-encoding", "connection", "x-request-id"]` where it now yields the last alone, and the round-trip test failed with the asymmetry in its own words: "`content-type` came from a response: `content-type` is reserved; the channel sets it, not the caller". The file already stated this invariant for the alphabet - "every key in a `Metadata` is one gRPC names a header whichever end it came from" - and applied it in one direction only. **The cost**: `grpc-status-details-bin`, which carries a `google.rpc.Status`, is dropped with the rest of `grpc-`. Nothing in this repository reads it, and a host that needs rich status needs a deliberate channel for it rather than an accident through a type whose own invariant forbids sending it |
| N-027 | the boolean reader was widened to nine spellings, case-insensitively and after trimming, so `AllowUnsafeConnection=" YES "` now disables certificate verification where the previous reader raised an error | **refused, by decision.** The variable's *existence* is what the option means; it carries a value only because testing for existence is not portable across the APIs that read it. So an affirmative spelling is an affirmative, and one option reading a stricter dialect than the shared vocabulary would be a trap of its own. The empty value stays false, so a variable that exists carrying nothing is still the safe answer |
| H-044 | the terminal flag is published with a Release store while `lend` and `settled` read it with SeqCst, and the comments justify the claim-versus-publish race by a total order the Release store does not join | **applied** - the store is SeqCst. The audit is right on the model: a SeqCst load is not obliged to see a non-SeqCst store that precedes it in that order, so the outcome where `lend` reads no terminal *and* `settled` reads no claim is admitted, and that is a buffer lent to a call declared settled - a ledger charge nobody returns and a runtime that never quiesces. No test: the only instrument that would drive it is `loom`, which needs the atomics behind a facade, and that is a change of its own |
| A2-006 | a failed teardown-thread spawn is silently discarded, so the runtime never leaves GRPC_STOPPED and the promised RESOURCES_RELEASED never arrives | **half applied.** The state is `AK_RUNTIME_FAILED_UNQUIESCED` now: QUIESCENT *is* that thread having finished, so a runtime that cannot get one will never reach it, and GRPC_STOPPED reads as "wait longer" for a step nobody takes. The other two halves are refused: RESOURCES_RELEASED emitted inline would say the resources are released while tokio's workers are still running, and design.md is explicit that "after a failure there is no promise that callbacks stop, that a terminal arrives, that cleanup completes"; and the runtime claim is not relinquished, because destroy is refused from that state forever (requirement 14.9, one runtime per process, is what a second one would break). The header note said this state was reserved for a status fault - stale since the shutdown task's own guard, so it names the three producers now |
| A2-007 | `ak_bytes_in::as_slice` returns `Option<&'a [u8]>` with a lifetime unconstrained by `&self`, so a caller may pick any lifetime for a host-owned pointer | **applied** - elided, so the slice borrows the `ak_bytes_in` and cannot outlive the downcall the pointer was handed to. Nothing else changes: the four call sites use their slices locally, which is why an unbounded lifetime cost nothing today and would have cost a refactor |
| R-015 | `From<ChannelError> for ak_status` maps `InvalidMethod`, `InvalidMetadata` and `Transport` all to `AK_STATUS_INVALID_ARG` through a catch-all, so a host cannot tell a malformed method path from an unreachable peer | **deferred to T4.0**, and not for effort: `ak_status` has no member for an unreachable peer - Ok, HandleStale, SlotBusy, InvalidArg, Internal, BudgetBusy, InvalidState, MessageTooLarge - so every faithful mapping needs a status the ABI does not have. T4.0's `ak_error` is what carries the sentence, and the arm can be written out then rather than twice |
| N-034 | `safe_endpoint` is `pub(crate)`, so the redaction this branch built cannot be reached by `packages/rust/armonik`, which puts the whole endpoint URI into a tracing span | **applied** - public, and called there. The span read back out of a subscriber before the fix: `endpoint="http://alice:s3cret@127.0.0.1:1/"`, in a DEBUG line. It takes a config built by hand, since the environment path refuses userinfo, and `ClientConfig::endpoint` is a public field. Nine call sites relied on that function and none measured it; it has its own tests now |
| N-036 | `ConfigError` and `ConnectionError` render only their outermost message, and the `chain` helper that makes them readable is `pub(crate)`, so a caller who logs `{e}` gets a sentence with no cause | **applied differently** - `snafu` is re-exported, with the pointer to `snafu::Report` on the re-export. A `String` formatter is not what a consumer needs made public: the causes are already reachable through `Error::source()`, and `Report` is the renderer snafu's own author supplies. Re-exporting also pins the version, which a consumer adding `snafu` themselves would have to match - and it is already a public dependency through the error types |
| B-003 | the userinfo guard tests `endpoint.contains('@')` on the raw string, so it also refuses an endpoint whose path or query legally holds `@`, where `http2.rs` scopes the same test to the authority | **refused** - the order is the point, and the code says so: the parse error carries the endpoint (`UriSnafu { uri }`), so parsing first is what would put a password in a message. `http2.rs` can scope to the authority because it is handed a `Uri` that parsed. What the raw test costs is refusing `@` in a path, and the path of a gRPC endpoint is the method's |
| R-008 | the .NET Framework engine probe picks its directory from `IntPtr.Size` alone, so a 64-bit ARM host is directed at the `x64` folder and loads an x64 DLL | **refused** - that layout is emitted for `.NETFramework` consumers only, and .NET Framework has no ARM64 flavour: on an ARM64 host it runs emulated as x86 or x64, where `IntPtr.Size` names the process's own architecture. `RuntimeInformation.ProcessArchitecture` answers `X64` for that same emulated process, which is the same folder. A .NET consumer never sees these folders at all - it resolves `runtimes/<rid>/native` - so there is no host that can load the x64 engine into an ARM64 process. The reasoning is in the `.targets` comment now, where the question comes up |
| R-064 | the per-call delivery ring, its mask arithmetic, the `Claim` enum and the arrival signal re-implement a bounded single-producer single-consumer queue that `System.Threading.Channels` provides on netstandard2.0 | **open, and the three objections to it were mine.** A bounded `Channel<Slot>` allocates nothing per element the ring does not, the element being a reference to a native buffer; a queue that removes the item at the take loses no release, since `ak_event_consumed` names the payload it frees and the header caps how many a call may owe rather than fixing an order; and level 2's `RingHead`/`RingTail` bind nothing, that level describing this binding and a refinement needing only a mapping, which may be fictional. What survives is a trade: the swap is mechanical now `DeliveryRing` is one 122-line type and it does not touch the phase machine or the read-versus-token arbiter - which the finding itself concedes have no library answer - and it costs a package reference on .NET Framework and the release rule, which is the invariant that actually matters here: `DeliveryRing.Release()` is the sole releaser of an accepted payload, and a queue that hands the item out cannot hold that - the discipline goes back to the caller, where `ak_event_consumed`'s three call sites are exactly what a reader has to keep straight |
| F-004 | `Metadata`/`MetadataValue` re-implement `tonic::metadata::MetadataMap`, down to two base64 engines that are byte-for-byte tonic's own | **open, and half of the duplication it names is unreachable.** Measured against tonic 0.14.6: `util::base64::STANDARD`/`STANDARD_NO_PAD` are `pub(crate)`, and so are `GRPC_RESERVED_HEADERS` and `into_sanitized_headers`, so what reuse buys is `MetadataMap` with `get_bin`/`append_bin` - not the constants the finding quotes. `BINARY_OUT` is `STANDARD_NO_PAD` exactly; `BINARY_IN` is `STANDARD` plus `with_decode_allow_trailing_bits(true)`, which the finding calls the one genuine improvement and which is load-bearing: C-core, Go and Java hand such a value over, and without it the header vanishes instead of arriving. Two more behaviours this branch built are not in `MetadataMap` either: a reserved set broader than tonic's five names - pseudo-headers, every `grpc-`, the six connection-specific fields of RFC 9113 8.2.2, `host`, `content-length` - and a **refusal at the key** where `into_sanitized_headers` removes in silence, which is Z-001's whole point. So `MetadataMap` as the storage is defensible and all three come back on top of it |
| F-002 | `stated_status`, `from_http_status`, `code_of` and `decode_message` re-implement `tonic::Status::from_header_map`, `infer_grpc_status`'s table, `Status::code_from_h2` and `percent_decode` | **open, and the function it recommends would reintroduce a peer-controlled panic.** `tonic::Status::from_header_map` runs `.expect("Invalid status header, expected base64 encoded value")` on `grpc-status-details-bin`, so a server that writes that header badly panics the reader task - the class of fault `guarded` was added to remove, and the one thing an in-process ABI cannot afford. Of the rest: `Code::from_bytes` and the three header-name constants are public and reusable; `infer_grpc_status`, the HTTP-to-gRPC table, is `pub(crate)` and the copy of it here is six match arms; and the finding already concedes `code_from_h2` is behind the `server` feature. So what is genuinely reusable is small, what is recommended is unsafe as it stands, and the useful move is upstream - phase 9's register |
| F-005 | `Inner::sender`/`Inner::dial`/`Session` hand-roll a one-connection HTTP/2 pool with dial coalescing and reconnect-on-closed, which `hyper_util::client::legacy::Client` already is | **open, and it is the largest of the three.** It holds as stated. Two things bound when: `TransportConfig` carries an endpoint and a connect timeout and nothing else today, so no builder surface blocks the swap - but T4.1 widens it with keepalive, nodelay and the HTTP/2 windows, so the swap either precedes that widening or absorbs it. The finding's own caveat stands, that `close()` must drop the client rather than take a sender out of a slot, and it is what the channel's disposal ordering rests on |
| F-003 | one C ABI is transcribed by hand five times - `abi.rs`, the header, `NativeMethods.cs`, `tests/layout.rs`, `AbiLayoutTests.cs` - instead of being generated from the Rust | **answered by decision: generated, with the artefact committed.** `cbindgen` for the header, `csbindgen` for the P/Invoke declarations, verified by the same regenerate-and-compare step `ChannelOptions.g.cs` has, and the prose contract moved into `cbindgen.toml`'s `header`. T3.6, before T4.0, because T4.0 changes seventeen declarations and two structs at once and doing that by hand is the fifth transcription. One thing the finding gets wrong: the two layout tests do not both become redundant. `Marshal.SizeOf` and `Marshal.OffsetOf` measure what the CLR does with the declarations, per target framework and per architecture, and a generator proves the declarations agree with the Rust rather than that the runtime lays them out as Rust does - the generated declarations being precisely what has to be verified rather than trusted |
| R-062, N-046 | the crate has no feature separating the engine from the TLS client stack, so `packages/rust/armonik` links seven engine dependencies to parse a configuration and open a tonic channel | **answered** - design.md's decision is no split and no feature gate: T7.1 puts the client on the engine and T4.1 gives the engine the TLS stack, so both halves converge and a gate would be scaffolding with a demolition date. Two findings for one mechanism, the second naming the seven dependencies the first names by module |
| A3-030 | the native callback handler swallows every exception with a bare `catch` and the binding has no diagnostics surface at all - no `ILogger`, no `EventSource`, no trace - so a `Publish` that throws strands a call with nothing recorded anywhere | **deferred to T10.1**, which carries the host half of the question now. The swallow itself is not the defect and stays: the callback runs on a tokio thread and an exception crossing back into Rust is undefined behaviour, so the catch is the boundary. What is missing is that it records nothing, and choosing between an `EventSource` and an optional `ILoggerFactory` is the same decision as choosing how the engine's own `tracing` events cross - one surface, decided once |
| not in the audit | `NativeChannel` implements `IDisposable` beside `IAsyncDisposable`, where design.md says a synchronous dispose would have to block on the network and on host callbacks and that the surface therefore offers none | **applied.** `await using` was documented and not enforced, so the path the document rules out was there to be taken; now `using var` on a channel does not compile - `error CS8418: 'NativeChannel': type used in a using statement must implement 'System.IDisposable'. Did you mean 'await using' rather than 'using'?` - which is the compiler saying it rather than a channel that never drains saying it later. Thirty-nine call sites in five test files, and sixteen test methods that had no reason to be asynchronous until their channel gave them one. `ShutdownAsyncCore` stays the disposal: gRPC's own `ShutdownAsync` means the caller is finished with the channel, which is what this one does |
| D-004, R-017 | the FFI writer discards `send_message`'s and `end_send`'s error and then signals WRITE_DONE unconditionally, so a message the transport refused is acquitted as though it had been taken | **refused, and the two fixes proposed for it contradict the header and each other.** The header settles the question in its own words: WRITE_DONE "says nothing about the network: the message may have been written, or abandoned because the call was cancelled", and it "arrives exactly once per accepted send, in send order, and always before the terminal". D-004 asks for no WRITE_DONE on a refusal, which breaks the first and hangs a host waiting on the buffer it lent; R-017 asks for it after the terminal, which breaks the third. Applying D-004 as a mutant: `the abandoned send is acquitted too: [WRITE_DONE, WRITE_DONE, WRITE_DONE, INITIAL_METADATA, STATUS] left: 3 right: 4`, on six runs of six. And the refusal both name, `MessageTooLong`, cannot arrive through this door: `LARGEST_LENDABLE` caps the ceiling itself at the four-byte prefix, `could_ever_fit` refuses above it, and `a_length_no_frame_can_carry_is_refused_and_charges_nothing` already proved it - which is what `fill`'s own comment says that refusal is there for. R-017's second half is vacuous: `end_send` answers `Ok(())` unconditionally, the half-close being the send half's drop. What the code owed is the reasoning, which is written where the discard is now, with a `debug_assert` that fails the day a second door can produce the arm |
| O-039 | the WRITE_DONE ordering the header promises - once per accepted send, in send order, always before the terminal - is never asserted; only the count is, and `Seen` holds the whole ordered sequence | **applied**, and it is what carries the refusal above. The client-stream test reads positions now: every acquittal before the terminal, and - since this server answers having read the stream whole - before the reply too. One new test drives the interleaving the two findings were about, a send accepted and then cancelled under, which the mutant above fails |
| E-004 | the pending acquittal lives in a single `writing_` field with no one-writer check, so a second overlapping `WriteAsync` overwrites it and orphans the first write's completion source | **applied** - the field is claimed with a `CompareExchange` against null and released on the value, so a write that gave up its claim cannot clear the claim of the write after it, and a second writer is refused with `InvalidOperationException`. Synchronous, like the closed-stream refusal three lines above it, which meant starting the write in `NativeRequestStream.WriteAsync` rather than inside the `async` method that wraps it - otherwise the refusal reaches a task the caller who overlapped two writes may not be awaiting. Removing the claim: `Expected: instance of <System.InvalidOperationException> But was: no exception thrown` - the second write is accepted, and the first then waits on a source nothing completes. What the audit does not say is how narrow the window is on a local link: written the obvious way, with the first write left to the network, the test never saw the defect - the acquittal lands before the next statement runs. The test holds the first message inside its own serializer instead, where the claim is certainly taken because it is taken before anything is serialized |
| A3-008, A3-009 | `AK_STATUS_SLOT_BUSY` reaches the catch-all branch and is raised as a fatal `Internal` that also ends the call, where the header defines it as retryable backpressure whose wake-up is the next WRITE_DONE | **the mapping is applied, the backpressure wait refused.** Two findings for one mechanism, and one of them names the wrong line: `LentBuffer.Take` throws first, so `HoldingABufferAsync`'s status check is never reached with this status. It has its own arm now, and a sentence that says what happened instead of "no buffer to serialize into". The wait is refused because nothing can reach it: SLOT_BUSY means another send of this call is unacquitted, `lend` answers INVALID_STATE while a buffer is still held, and `WriteAsync` both admits one writer - E-004's claim, above - and waits for the acquittal before returning, so the window is open at every lend it makes. `the_send_window_refuses_a_second_buffer_until_a_write_is_acquitted` shows what does produce it: lend, commit, lend again before the WRITE_DONE, which is a host pipelining deeper than this binding does. Building the wait would mean a signal and a retry loop that no test could drive - the test gap the audit itself reports next to this - so what is recorded instead is that meeting the status is this binding's own bookkeeping being wrong, which is what the message now says. It becomes real work the day design.md's "native depth allows MaxSendsInFlight; this binding exercises one" stops being true |
| A3-022, R-078 | `RuntimeDisposeState.DestroyFailed` is absorbing for the life of the process and is reached by a 30 s quiescence timeout, so one slow drain stops the process ever opening another channel | **applied, and by the engine's own argument.** The deadline is gone. `release_threads` already dropped the Rust-side one and says why - "this thread finishing is what `state` reports as QUIESCENT, and the header promises that state alone permits `ak_runtime_destroy` or unloading the library, so a deadline that expired with work still running would make the promise a lie exactly when it matters" - and the same sentence condemns a host-side timer that calls a slow shutdown a broken runtime, which under one-runtime-per-process is the process's verdict for good. `DestroyFailed` is now reached by the two failures that are failures: `AK_RUNTIME_FAILED_UNQUIESCED`, the engine saying it could not start the thread quiescence is, and a destroy it refuses. R-078's own fix is refused - it asks the engine to relinquish its claim, which is requirement 14.9 - and A3-022's answers both |
| A3-023 | `Lease` blocks a thread on `GetAwaiter().GetResult()` waiting for a teardown that itself needs pool threads to resume after each `Task.Delay`, which is a starvation deadlock under load | **applied, by deleting what blocked.** The runtime is an object its caller creates and disposes, so no creation waits on a destruction and there is no `Lease` at all. Two answers were weighed and both refused - an asynchronous door beside the synchronous one, which leaves the synchronous one a trap of the same shape; and putting the teardown on a thread of its own, which makes a wait safe that has no reason to exist. What the finding reports is the cost of deriving the runtime's lifetime from its channels, and the derivation is what went. **Half of it stood before that:** The poll is what is answered: the long part of the wait is now a latch that the runtime's own events set, so the shutdown of every channel and every callback proceeds without a pool thread resuming a timer - the engine stores `GRPC_STOPPED` before it emits SHUTDOWN_COMPLETE, which is what makes a wake-up enough and the state the thing believed. What is left polled is the thread join, in milliseconds, because no event can announce it: whatever emitted the announcement would be running on the thread whose end it reports. Silencing the latch: the run aborts with the host hung on a teardown that never returns. The other half is `Lease` blocking at all, which is a public-surface decision - an async door beside the synchronous one - and is not this commit's |
| R-044 | the shutdown budget is two unrelated hard-coded timeouts in two languages, five seconds for tokio's own shutdown and thirty for the host's quiescence poll, so the host's wait can succeed against a runtime that gave up | **applied, in two halves and neither of them a timeout.** The engine's five seconds went with the quiescence commit, `release_threads` dropping the tokio runtime rather than deadlining it; the host's thirty go here. So there is no budget on either side and the divergence the finding names cannot exist. Its own fix asked for the host's to be the only one, which would have left the absorbing state A3-022 reports |
| K-069 | `ArmTheNextTest()` is sequenced after an `Assert.That` that throws, so one leaked lease leaves the memory ceiling configured for every later test in the fixture | **applied by deletion.** There is no factory to reconfigure and no lease to assert on: each test takes a runtime in its setup and gives it back in a teardown that asserts nothing, so a test failing on its subject still leaves the process able to run the next. A fixture whose tests need the engine started differently overrides one method, and a test whose subject is what it was started with restarts it - which is what the memory-ceiling test does |
| K-071 | only `UnaryTests` configures the process-global runtime, so the other four lease-taking fixtures run on whatever worker-thread count it happened to leave | **applied by deletion**, and the finding's mechanism was exact: the count was process-global and set by whoever ran first. It is an argument of `Create` now, so a fixture states what its tests need - `UnaryTests` two workers, the rest the engine's own - and nothing a fixture does reaches another |
| A3-004 | `SettlingAsync` cancels `ending_` only after `holding_` reaches zero, so a send parked on `AK_STATUS_BUDGET_BUSY` is never released by the call's own terminal | **applied**, and the order is the whole fix: the cancel moves ahead of the wait it was blocking. A call whose terminal is in has no business waiting for room to serialize a message that can no longer go anywhere - and the room is other calls' to give up, on a schedule this one does not control. The finding's "indefinitely" needs one correction and one addition. The correction: the disposal path it implies was never exposed, because `CancelAndDrain` calls `EndCall()` before it drains, so a cancelled or disposed call already released its parked sender. The addition: what is exposed is the **natural terminal** - the server answered, a reduction consumed it, nobody cancelled anything - which is what the new test drives, with the ceiling held by a serializer on another channel blocked on purpose. Restoring the order: `the parked send hears that its call is over, rather than waiting for room nobody is giving back / Expected: True / But was: False` after 30 s, where unbounded it hangs the run instead of reporting |
| R-030 | on `AK_STATUS_BUDGET_BUSY` the binding serializes into a managed `byte[]` instead, so the runtime-wide ceiling bounds only which allocator pays | **already closed** by the send-ceiling commit, which is the fix this finding asks for in the words it asks for it: wait for room first and lend afterwards. `spilled_` and the `byte[]` it named are gone - the file holds no such field - and `HoldingABufferAsync` serializes one whole attempt per turn against `WaitForRoomAsync`. Recorded rather than skipped, because the audit and the fix crossed |
| A2-005 | QUIESCENT is the teardown thread having finished, but that thread finishes via `shutdown_timeout(5s)`, which detaches still-running threads on timeout | **already closed**, by the first of the two answers it offers: drop the timeout and join unconditionally. `release_threads` drops the tokio runtime with no deadline and says why - this thread finishing is what `state` reports as QUIESCENT, and the header promises that state alone permits `ak_runtime_destroy` or unloading the library. `shutdown_timeout` appears nowhere in the crate. The host side lost its own deadline later, for the same reason |
| K-061 | the two `FreePort()` calls can return the same port, because the first listener is stopped before the second is opened, which collapses the mock to a single listener | **applied**, and the consequence is louder than the finding says. The mock binds one listener for one port asked twice, on purpose - it is the shape five of CI's six mock jobs ask of it - and that listener speaks HTTP/1 and HTTP/2 both. This engine cannot use it: it speaks h2c with prior knowledge, and a cleartext endpoint that also admits HTTP/1 answers `the request did not reach the peer: http2 error`. Measured by asking for it deliberately, which is how the collapse was confirmed to be a failure rather than a quieter success. Both listeners are now held until both ports are read, so the two cannot be the same |
| K-060 | `FreePort` reserves a port and releases it before handing it over, and its own remark says so: another process may take it in between | **holds, and is not closed by K-061's fix** - holding both listeners removes the collision between the two, not the window between the release and the mock's own bind. Its fix is the right one and it is bigger than one line: the mock would have to bind ephemeral ports and announce what it got, and because this engine needs two listeners with different protocols - `Http2` for the calls, HTTP/1 for the readiness probe - it would have to say which is which. That is a change to `ArmoniK.Api.Mock`, which six CI jobs drive with fixed ports, for a race in one test fixture. Recorded for whoever next has reason to touch the mock, with the note that the announcement also retires the readiness probe |
| E-009 | `ShutdownAsyncCore` is mapped onto full disposal, which cancels every outstanding call and blocks until they settle - the opposite of what `ChannelBase` documents | **refused, and the divergence is documented where an implementor reads it.** `ChannelBase` tells an implementor it need not cancel and need not wait; it does not forbid either, and it makes finishing the calls the caller's own responsibility, saying outright that shutting down with calls in flight may change their outcome. So doing that work narrows the ways to be surprised. And it is forced rather than chosen: a channel that let go of its handle while its calls still held payloads and lent buffers would leave the runtime owed them, and a runtime owed anything never reaches QUIESCENT - a cheaper shutdown would hide the cost until the runtime refused to go. The one consumer of `ShutdownAsync` in this repository is `WorkerStreamWrapper.DisposeAsync`, which maps its disposal onto it |
| A1-003 | `of_response_head` takes any `grpc-status` in the response HEADERS as the call's final status without checking that the head ended the stream, so a 200 carrying `grpc-status` plus DATA is reported as that status with the body unread | **applied**, and the check is what arrives rather than what the frame said. Trailers-Only is one HEADERS frame carrying the status and ending the stream, and the tempting test - did the head end it - is not reliable: hyper sends an empty DATA frame to end a body it did not end on the head, so a legitimate Trailers-Only response can arrive with `is_end_stream` false. What decides it is whether a message arrives: the stated status is carried into the body loop and answers when the body ends with nothing in it, and message bytes behind it are the malformation, named as such. An HTTP error keeps the old answer, terminal on sight, because a non-200 carries no gRPC body that could contradict it. **And it found a second thing**: a Trailers-Only head was being delivered as initial metadata, where that one frame is the trailers - the reader saw a head no such response has, carrying the same entries the status already carried. Removing the guard: `assertion left == right failed / left: Ok / right: Internal` - the call reported as succeeded, its message unread |
| A2-083 | `Ledger` keeps `bytes` and `outstanding` as two independent atomics updated in sequence, so `empty()` and `usage()` can observe a pair no caller ever produced | **the mechanism as stated is false, and a real one was behind it.** Neither reader reads the pair: `empty()` loads `outstanding` alone and `usage()` loads `bytes` alone beside a ceiling that never changes, so there is no pair to tear and nothing to pack into one atomic. What is wrong is the **order of the two writes on the acquire side**. `hold_bytes` charged the bytes and counted the lend afterwards, so a thread preempted between them left the ledger holding bytes that nothing was counting - and `empty()` is what decides whether the shutdown owes RESOURCES_RELEASED and whether it waits for the host to give anything back. Answered wrongly there, the runtime reports QUIESCENT with a buffer still lent, which is the one thing that state is promised not to mean. The count is raised first now and dropped last on both sides, so it is conservative in the one direction that matters: it can name a lend not yet charged, never a charge nothing counts. Removing the refusal's undo, which that order needs: `a_charge_the_ceiling_refuses_leaves_nothing_counted` fails - and a host that met the ceiling once would leave the ledger never empty again |
| D-007 | `close_the_gate` takes a blocking `std::sync::RwLock` write lock from inside a tokio task, parking a worker until every host thread releases its read pass | **applied, and the exposure is smaller than the pattern.** It runs on the blocking pool now. Measured before changing it: a pass is held only across `ak_channel_create` and `ak_call_start`, and neither waits on the runtime - `GrpcChannel::new` builds a config and `start_call` registers an actor, both synchronous - so this is a parked worker for the length of a short downcall, once per runtime, and not the deadlock it could have been had a pass-holder needed that worker. The finding's second suggestion does not work as worded: the teardown thread is started at the end of the shutdown, and the gate has to shut before the tables are read. Its first, a `tokio::sync::RwLock`, would change what a downcall racing the close is told; `spawn_blocking` changes only which thread waits |
| B-012 | none of the error enums added by this branch carries `#[snafu(implicit)] location`, so after the backtrace removal they have no positional information at all - contradicting the justification that "every rendering of these errors uses `{location}`" | **refused, and the contradiction is a misreading.** That sentence is in a commit that removed a backtrace from three variants, and it is about those three; it does not claim every error in the crate renders a location. The substantive question is whether the newer types want one, and the two families are differently built. The older messages are generic - `Could not read environment variable [{location}]`, `Invalid TLS configuration [{location}]` - and the file and line is the only thing that says which check spoke. Every variant added here names its subject in the sentence: `` `max_sends_in_flight` of {value} is past {LARGEST_WINDOW} ``, `` `{key}` is reserved; the channel sets it, not the caller ``, `a message of {len} bytes does not fit the four-byte gRPC length prefix`. A location on those adds a coordinate the reader does not need - and these messages are bound for `ak_error`, so it would travel to a .NET caller as noise in an exception detail and as this library's source layout |
| B-013 | `TransportError::Connect` and `Http2Handshake` store the underlying error as a rendered `cause: String`, so `Error::source()` is `None` and the `io::ErrorKind` behind a failed dial is unreachable except by substring-matching | **refused on the cost, and it names the wrong constraint.** It says `Eq, PartialEq` is what forces the string. The forcing constraint is **`Clone`**, and it is structural: a coalesced dial broadcasts its outcome to every caller waiting on it - `broadcast::Sender<Result<SendRequest, ChannelError>>` - and `tokio::sync::broadcast` requires its payload to be `Clone`. A `Box<dyn Error + Send + Sync>` is not, and `Arc<dyn Error + Send + Sync>` is not itself an `Error`, so snafu cannot take it as a `source` either. The reachable fix is therefore larger than the finding's: `Clone` off four public enums, the broadcast's payload behind an `Arc`, `Eq`/`PartialEq` off three, and the assertions that compare them. What it buys is typed access for a consumer that would act on `io::ErrorKind` - and none does. The message loses nothing meanwhile: `chain(&error, ": ")` flattens the whole chain into it, so what is missing is the downcast and not the sentence. It becomes worth paying the day something decides on the kind, which is retry - and retry decides on gRPC status |
| Q-001, Q-017, Q-023 | three shipped documentation strings tell a .NET caller about the engine: `MaxDeliveryCredits` names tokio's `Semaphore::MAX_PERMITS`, the generated `Validate()` says it "refuses a value the schema excludes", and `TransportOptions`' remarks say the endpoint "crosses the ABI as its own argument" and that `{}` configures "this document" | **applied, all three, at their sources.** A `Semaphore::MAX_PERMITS` in a .NET caller's IntelliSense is a Rust identifier they cannot look up, and the reason survives without it - the engine imposes no bound that sizes anything, so this one is the binding's. The other two are generated: `Validate()`'s sentence comes from the options generator's emitter, and `TransportOptions`' remarks are the schema's description, which is itself the Rust doc comment `schemars` renders. So the fix is in `options.rs`, then `options.schema.json` regenerated, then `ChannelOptions.g.cs` regenerated - the build checks both and rewrites neither. `the_committed_schema_is_the_one_the_types_describe` and the generator's `--check` both pass on the result |
| I-008 | the gRPC `-bin` convention is re-declared in the .NET binding and re-derived per entry on decode, with a case-insensitive comparison where the Rust side uses an exact one | **applied to the comparison, and it has a sharper edge than the finding gives it.** It is not only that the two sides could disagree: `Grpc.Core`'s own `Metadata.Add` refuses a byte value under a key whose `-bin` suffix does not match **ordinally**, so a looser test here calls a key binary that neither the engine nor `Metadata` does, and then hands `Metadata` bytes it throws on. One word. Unreachable today because every key reaching the blob came from an `http::HeaderName`, which is lowercase - but the rule mirrored was written laxer than the rule. The other half, carrying the entry kind in the blob so no host re-derives it, is an ABI change and belongs to T4.0 |
| I-004 | the `AkStatus`-to-caller-failure translation is written ad hoc at three call sites with three different answers, and one prints the raw C enum name | **already answered, by two rows above.** Its two halves are each a finding already re-derived here: the raw name is R-022, where it stays because it is the one token in the sentence that says which refusal happened; and the three different answers are F-030, where they are three different questions rather than three copies - a channel going away under a call, a call that has ended, a size refused at the lend. A third finding for the same two mechanisms, and the same answers |
| C-029 | `NativeCall` is 1036 lines carrying the delivery ring, a five-phase reader arbiter, the writer and its acquittal, the settlement | **already closed** by A3-056's three-way split, which it duplicates: `DeliveryRing`, `Sender` and `Receiver<TResponse>` are types, and what is left in the call is 374 lines of its identity |
| A2-070 | the channel's (state, call-count) pair is packed into one `AtomicU64` with hand-rolled `parts`/`word`, four free transition functions and a generic `advance` - about a hundred lines of bit-twiddling | **applied.** The pair is a `Phase { state, calls }` behind a `Mutex` now. What the packing bought was one atomic read-modify-write over both halves, which is what every transition needs - a call may join only an open channel, a close finishes only once the last has left - and a lock buys the same thing while letting the pair be a pair rather than a layout. It costs a lock on a channel's open and close and on a call's start and end, never on a message. The four transitions stay as the table they were and keep their tests; what goes is the CAS loop, the shift, and the two tests that existed to check the layout. It also left `ak_channel_state::from_repr` with no caller - its only one was unpacking the word - so that goes too, with the `unwrap_or(CLOSED)` that answered for a state the library never wrote |
| A4-001 | `RefuseCycles` keys its edge graph on the node that holds a `$ref` and values it with the pointer that `$ref` names, so a step exists only when the referenced node is itself a bare `$ref`; a cycle through `properties` is never seen | **applied**, and the shape it misses is the ordinary one. An edge is recorded at `#/$defs/A/properties/B`, and the walk then looked up its target `#/$defs/B` as a key - which exists only where `B` is itself a bare `$ref`. That is the rare schema; a type that reaches itself through its own properties is what a recursive one is, and it went through unseen. The walk follows what a target *contains* now. The consequence of missing it is not a wrong answer: a cycle overflows the stack inside Corvus's own reduction, and .NET cannot catch that, so the build dies with nothing naming the schema. The existing test used the one shape the old walk caught; the new one uses the shape a schema gets. Restoring the old edge rule: `Expected: <System.NotSupportedException> ... But was: no exception thrown` |
| A4-002 | `Resolve` walks a `$ref` chain to its end but `BoundsOf` and the description lookup consult only the first node and the final target, so a bound stated on an intermediate hop is dropped, and `Unhandled` cannot catch it because those keywords are understood | **refused, and the measurement is the interesting part.** The reasoning holds against the JSON; it does not hold against what this generator is handed. The far end of a chain is not the schema as written - it is Corvus's *reduction* of it, and that reduction already carries every hop's keywords. Measured rather than assumed: a chain whose `maximum` is stated on the middle node alone and whose `minimum` is stated on the last renders both checks while `BoundsOf` reads only the two ends. **I had written the fold over every hop before measuring it, and it was a no-op** - the mutation is what said so, by passing when it should have failed. The chain fold is reverted; what is kept is a test that pins the fact the reading rests on, so the day Corvus stops folding, something says it |
| A3-041 | `Validate` rejects zero, negative, NaN and infinite `ConnectTimeoutSeconds` but no upper bound, so a large finite value reaches the engine's `Duration::from_secs_f64` and panics there | **applied to the bound, refused on the panic.** `from_secs_f64` appears nowhere in this workspace - the conversion is `Duration::try_from_secs_f64`, which refuses rather than panicking, and has since the blocker that made every `Seconds` fallible. What is real is that it refuses *without naming the option*: a caller reads that their configuration was refused and not which line of it, which is the failure R-045's strict binding exists to avoid. The ceiling is stated now, and on the **type** rather than the option, because it is `Seconds`' own - every one becomes a `Duration`, which holds `u64::MAX` seconds, so 2^64 is the first value none can be. It reaches the generated `Validate()` through the `$ref`, which is A4-002's folding seen in production. And the schema-versus-reader test knew nothing of `exclusiveMaximum`, so the agreement would have been unverified: removing the ceiling now fails it with `left: None / right: Some(1.8446744073709552e19)` |
| R-031 | `MaxDeliveryCredits` caps `DeliveryCredits` at 32768 while the schema and the generated `Validate()` admit 536870910, and `MaxSendsInFlight` - the mirror window - gets no host-side bound at all | **answered, in two halves that are two rows.** The first is C-007: a binding may narrow what the ABI admits, because the ring is allocated at the next power of two above the window and the binding's bound is two megabytes of slots per call in a 64-bit process; the *statement* that a host may narrow is in design.md beside the options since c32d3ef4. The second half is not a gap, because the two are not mirrors: `DeliveryCredits` sizes something on this side - the ring - and `MaxSendsInFlight` sizes nothing here, being the engine's own window, which `LARGEST_WINDOW` already bounds in the schema and in the reader. A host-side bound on it would narrow a number this host does not allocate against |
| R-059 | the drain a call must run to settle is queued on the .NET thread pool with `Task.Run`, so under thread-pool starvation the channel's disposal and the runtime's quiescence wait behind application work | **refused, and the measurement is that the drain is not special.** The statement is true and says nothing about the drain in particular: every link of that chain is a pool continuation, including the `await`s the caller wrote. `NativeChannel.DisposeAsync` awaits each call's settlement, `NativeRuntime.DisposeAsync` awaits that, and `QuiescentAsync` waits on a latch and a timer - all resumed by the pool. Checked for the shape that would make it a deadlock rather than a delay: nothing **blocks** a pool thread anywhere on it, `Lease` having been the one that did and being gone. So moving the drain off the pool would make one link independent of a scheduler the next link still depends on. Its own first suggestion buys less than it looks - `TaskCreationOptions.LongRunning` governs the initial scheduling, and every continuation after the first `await` returns to the pool. Taking the whole chain off the pool is one coherent change, and a different question |
| K-026 | `TheEngineBesideThisHostMatchesItsWordSize` maps pointer width to x64 or x86 only, so on the win-arm64 target `RustTargets.props` ships it fails with a message blaming the engine | **applied.** A pointer width names two of the three architectures requirement 8.1 promises: an arm64 host is eight bytes wide like an x64 one and wants a different engine. Read off `RuntimeInformation.ProcessArchitecture` now, with the PE machine value per architecture and an inconclusive result rather than a pass for one nobody recorded - a test that cannot tell should say so and not answer |
| K-068 | `CallsWaitForRoomUnderAMemoryCeiling` fires sixteen concurrent 100 KB calls at a 128 KB ceiling with no bound, so a regression in the credit accounting hangs the run rather than failing it | **applied, both halves.** The wait is bounded, because what a regression in the accounting costs is a wait and not a fault. And the test proves the ceiling was *met* rather than merely not exceeded: `ak_runtime_memory_usage` is sampled while the calls run, since what the ceiling promises is about the middle of the run and not its end - read afterwards, every call has given everything back and the reading is zero whether the ceiling held or not. The high-water mark has to reach one message and stay inside the ceiling |
| K-041 | `AReservedMetadataKeyIsRefusedBeforeTheCallStarts` pins `StatusCode.Internal` for a caller-side validation failure, cementing the `InvalidArg -> Internal` mapping instead of forbidding it | **applied, mapping and test together, as the finding asks.** `ak_call_start`'s three refusals are three questions and now have three answers: a channel going away under a call is `Unavailable`, `InvalidArg` is `InvalidArgument`, and anything else is `Internal`. The reason is the one gRPC's own table gives - a caller's error maps to INVALID_ARGUMENT - and the sharper reason is what `Internal` means in this binding: the fault is its own. Pinning `Internal` made a caller's mistake indistinguishable from a bug here. What the caller handed over is a method that is not a path, or metadata the engine reserves, and neither left the process |
| E-010 | a header `Grpc.Core` accepts in `CallOptions.Headers` but the engine reserves is encoded unfiltered and fails the whole call with a status naming no key | **applied**, and the duplication it costs is deliberate and dated. The engine refuses these and is right to - a host is not to be trusted with an invariant of the wire - but it answers with one status for a whole document, so a caller read that their call could not be started and never which entry was wrong. The rule is written twice now, and the second copy's only job is to have the key still in hand when it speaks. It goes when the ABI can carry a sentence: `ak_error` is what would let the engine name the key itself, which is T4.0. The test asks for the key in the message rather than for the status alone |
| L-020 | win-arm64 is a claimed runtime identifier the .NET Framework side cannot serve, because the targets file copies only x64 and x86 and the loader picks between them by pointer width | **refused, by the measurement R-008 already carries.** That layout is emitted for `.NETFramework` consumers, and .NET Framework has no ARM64 flavour: on an ARM64 host it runs emulated as x86 or x64, and `IntPtr.Size` then names the process's own architecture, which is the folder it needs. A .NET consumer never sees those folders - it resolves `runtimes/<rid>/native`, which is where win-arm64 is served. So the identifier is claimed and kept by the half that can keep it, and the half that cannot is one no ARM64 process reaches. The pointer-width read that K-026 fixes is the *test's*, which compares against a PE file and does need the real architecture |
| R-014 | `config::parse` returns `Option`, discarding which option was refused and why, although `serde_path_to_error` is already a dependency | **deferred to T4.0**, which is where its own fix points: "carry the string out through the error channel". Naming the refused option inside the engine buys nothing while `ak_channel_create` can answer only `AK_STATUS_INVALID_ARG` - the sentence would be built and dropped at the boundary. T4.0 adds `ak_error` and this is one of the things that then has somewhere to go, alongside the five connector failures that motivated it |
| R-061 | a magic u64 tag read from host-supplied memory is the only thing between a double `ak_event_consumed` or `ak_return_call_buffer` and heap corruption, and the tag is read from the allocation it is meant to validate | **answered by decision: the guard is not built.** The finding is right about the tag, and that half is worth keeping: it is read from the thing being validated, so a second give-back reads eight bytes of a freed allocation before any check runs, and no in-allocation scheme fixes that. What it asks for is a hardening beyond the contract rather than a defect against it - the header assigns exactly-once to the host outright, calling a second give-back undefined behaviour because "there is nothing left to refuse with" - and the only guard that works is out of band, on the lend and return path of every message. The decision refuses the shape as well as the spending: **not `cfg(debug_assertions)`**, which is the build this repository's own tests run against, so the cost would land on the work rather than on the question, and which no host could switch on when it wanted it. If it is ever built it is a feature of its own, off by default and named, for whoever is bringing a host up |
| E-012, E-011, H-041 | the engine carries no keepalive, per-call timeout or rate limit; it speaks plain `http://` only, so no TLS-secured deployment is reachable; and any `CallOptions` carrying a deadline is refused with `Unimplemented` | **deferred, each to the task that builds it, and none of the three is an omission.** T4.1 gives the engine `connect.rs::https_connector` and brings the `tls`, `tcp_keepalive` and `http2` units with it - which is the same task for both the scheme and the keepalives, because they are branches of one connector. T6.2 is the deadline, and it waits on T4.0 because it adds a field to a struct whose size is checked. The rate limit is the one no task builds: T7.1 settles it, built or refused by the client. What makes them deferrals rather than gaps is that each is a phase of the plan with its prerequisites stated, and the refusals are deliberate: a deadline silently dropped would be a call that outlives what its caller asked for, and `Unimplemented` is how a binding says it cannot honour an option rather than ignoring it. The vocabulary test carries the same three as `Awaited`, against the phase that brings each |

## The abstraction leaks and the complexity

Twenty-one majors the inventory files as an abstraction leak or as complexity were open on
2026-09-29. Two are pre-existing and outside the 287 above: `GrpcChannelFactory`'s
160-line method, in the managed transport the binding sits beside, and `from_config_args`, which the Rust
configuration stack owns. Of the other nineteen, R-049 joins the I-012 row above, thirteen are
in ten rows below - three answered, eight deferred to the task that carries each, two applied -
and five are lots of their own that join as they close: the four on a call's state in the FFI
crate, and the .NET read loop.

| Finding | What it said | Status |
|---|---|---|
| I-002 | the engine's option types spell their JSON in .NET's PascalCase so the generator can take the names as they are | **answered by the user's decision**: Rust keeps its own names, serde spells the document PascalCase, and the C# reads that spelling as it is. What the document looks like is not a leak as long as each language gets names it would choose |
| R-016 | the engine's status code is an alias of tonic's, so the integers the ABI publishes are a third-party crate's discriminants | **answered**: tonic is the engine's by decision, the crate has no consumer outside ArmoniK, and every number the ABI publishes is checked against the gRPC status document by `every_grpc_status_the_abi_publishes_is_the_number_the_specification_gives_it` |
| R-012 | the three handle tables are process-wide statics, so shutdown and close walk every call of the process | **applied, by the user's choice of a list per channel.** The tables stay process-wide: one runtime per process makes every call the runtime's, and destroy empties them before the runtime goes, which 0e8efdf9 pins for channels. What changed is the close: a channel lists its calls under the lock a start and an end already take, so a release cancels its own calls rather than walking the process's, and a shutdown's close of each channel no longer walks every call once per channel. `releasing_a_channel_cancels_its_calls_and_none_of_another_channels` pins it |
| Z-014 | `ConnectTimeoutSeconds` is a `double?` of seconds on the public .NET surface, where every caller holds a `TimeSpan` | **deferred to T4.1**, the first task to add a duration beside it, as a point to settle: code wants a `TimeSpan`, and the configuration binder reads one as `d.hh:mm:ss`, where "5" is five days |
| N-042 | `TransportError` keeps its causes as rendered strings, so `source()` answers nothing | **answered by the B-013 row above**, which refused the same change on its cost: the forcing constraint is `Clone`, which the coalesced dial's broadcast needs, and nothing decides on the kind of an error today |
| N-014, B-014 | the Rust client re-exports the whole transport crate, and its configuration converts into nothing the engine reads | **deferred to T7.1**, which adapts that client to the engine and settles what becomes of the options the engine has not got by then: each is built or refused by the client, and none is read and ignored |
| I-005 | the P/Invoke declarations encode the package's deployment layout: the x64 and x86 folder names the `.targets` spells again, the `.dll` suffix and a kernel32 `LoadLibrary` | **deferred to T6.6 for the folder names**, whose deliverable is one table of runtime identifiers that every consumer reads, the loader and the `.targets` among them. **The suffix and the load stay**: .NET Framework runs on Windows alone and has no runtime-identifier probing, so the binding loads the engine from its folder itself before the first call, and T6.6's table is what names that folder |
| R-048 | the generator knows five scalar shapes, and phases 4 to 6 need more | **deferred to T4.1**: each shape comes with the first option that needs it |
| M-127, M-072, M-108 | design.md is too long for what it describes, and says itself that it mixes four registers | **deferred to T3.6**: the ABI's reference leaving for the Rust takes the declarations the Rust carries out, and the split by register follows |
| G-003 | the options emitter is 225 chained `Append` calls, 62 of them indentation, with a function that removes separators it wrote | **applied** in a1478877: one raw literal per construct over an `IndentedTextWriter`, byte-identical output |
