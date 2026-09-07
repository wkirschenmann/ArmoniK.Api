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

## The blockers, at a glance

| | count | |
|---|---|---|
| applied | 15 | the four version tags, the lock file, a path Linux cannot read, the archive's schema, two numbers a host may send, the prologue that answers the headers with its two test gaps, an internal type that stopped escaping, and a send ceiling that is waited on |
| answered by a decision | 5 | the platform set, the ABI's error channel, its size check, and two the absence of a publication channel dissolves |
| re-derived and downgraded | 1 | the HTTP/2 window: the evidence holds, the number does not |
| open | 11 | |

Of the eleven open, three are the engine's failure paths - a panic in a spawned task, a debt read
before a lend has counted, a five-second timeout reported as quiescence - one is a public option
that may have no effect, and the rest belong to tasks already named.

**What the re-derivation is finding, over sixteen blockers so far: the audit's evidence lines hold
and its conclusions need redoing.** Three of its claims were wrong on the number or the consequence
while right about the mechanism, and one - the headers deadlock - was under-described rather than
over: it missed that the formal model already prescribed the fix and that an invariant was vacuous
against the code. So each finding is reproduced before it is believed, and what the reproduction
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
