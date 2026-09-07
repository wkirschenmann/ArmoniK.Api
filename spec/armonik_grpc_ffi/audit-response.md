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

**Status: the choice goes to the user**, with T6.7's benchmarks as what should settle it. Severity
as re-derived here is major, not blocker.
