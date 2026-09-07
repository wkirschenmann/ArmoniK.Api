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
