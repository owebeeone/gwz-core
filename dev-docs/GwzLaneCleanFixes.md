# GWZ lane disposal clean-up requirements

Status: draft requirements, 2026-09-16. R20 to R22 and Phase 1 S1.1 to S1.4 released in gwz 1.0.14; Phase 1 complete (S1.5 to S1.8) in gwz 1.0.16; Phase 2 (R5 to R8, R0 reached: a merged verbatim lane disposes with no waiver) released in gwz 1.0.17 on 2026-09-18, together with D9 (a forced dispose names only the waivers it used). Requirements only: no design, and no
decision is taken here. Sources: the lane issues register (gwz-dev
`dev-docs/GwzLaneIssues.md`, L1) and a recount of every hazard entry made on
2026-09-16. Observed with gwz 1.0.12.

## 1. What disposal costs today

`gwz local dispose <name>` reports 112 hazard entries for a lane of this
workspace and refuses: 60 `dirty` and 52 `unpreserved-history`. Every lane from
round 2 on reported the same counts, because the counts describe what the copy
inherited, not what the lane did. Removing an integrated lane therefore takes
three steps: the operator's own comparison against the family, a refused
dispose, and a second dispose with `--force dirty,unpreserved-history`.

How the entries are counted, from current gwz-core source:

- An ignored directory counts once, not per file
  (`crates/repo-inspect/src/work.rs`).
- A repository holding native stash entries adds one `dirty` hazard, not one per
  entry (`crates/work-detector/src/lib.rs`, `HazardKind::NativeStash`).
- `refs/stash` is not a temporary operation ref, so the newest stash entry
  counts as preserved and older entries do not
  (`crates/history-check/src/lib.rs`, `is_temporary_operation_ref`).

## 2. Goal

- **R0.** Disposing a lane whose work is already integrated MUST take one
  command, with no waiver and no operator comparison: `gwz local dispose <name>`
  succeeds.
- **R0.1.** A lane that holds anything the surviving family does not hold MUST
  still refuse.

Conventions follow `GWZRequirements.md`: `MUST`, `SHOULD`, `MAY`.

## 3. Requirements

### 3.1 Recognize what the clone copied

- **R1.** `gwz local clone` MUST record what it copied, per repository: the ids
  of the stash and reflog entries, and the ignored and untracked paths with a
  fingerprint cheap enough for a workspace of this size (for example size, mtime
  and inode).
- **R2.** `gwz local dispose` MUST NOT report as `dirty` or
  `unpreserved-history` anything that is unchanged since that record and still
  present in the family.
- **R3.** Where no such record exists, because the lane came from an older gwz
  or was copied outside gwz, dispose MUST make the comparison itself: per
  repository, the commits from `rev-list --all`, the reflog and the stash that no
  surviving family repository holds, and the ignored and untracked entries that
  differ from the family's. Finding nothing unique MUST NOT require a waiver.
- **R4.** Deleting a copy that leaves the family's own entries untouched is not a
  loss. When the surviving witness holds the identical object,
  `unpreserved-history` MUST NOT refuse merely because the witness holds it in
  its own reflog or stash. Refusal MUST stand when the lane holds the only copy.

### 3.2 Regenerable data

- **R5.** Dispose MUST classify as regenerable, not as user work: a directory
  holding a valid `CACHEDIR.TAG`; `__pycache__/`; `*.egg-info/`; a build tool's
  convenience symlinks pointing outside the workspace (`bazel-*`, `razel-*`); and
  compiled extension modules inside a source tree (`*.so`, `*.pyd`, `*.dylib`).
- **R6.** A build directory whose tool wrote no `CACHEDIR.TAG` MUST still be
  recognizable by the markers that tool does write; `gwz-cli/target`, built by an
  older cargo, is the case in this workspace. Recognition MUST NOT rest on a
  directory's name alone.
- **R7.** Recognition MUST NOT depend on the data being unchanged since the
  clone: a cache the lane rebuilt is still a cache.
- **R8.** Ignored data that is neither regenerable nor unchanged since the copy
  MUST still be reported.

### 3.3 Reporting and waivers

- **R9.** Dispose MUST report hazards by category, with counts and with the paths
  or object ids, separating at least: regenerable, unchanged copy, changed copy,
  and unique to the lane.
- **R10.** A refusal MUST print the exact command that waives exactly the
  categories it found.
- **R11.** A waiver name MUST be no broader than its category. Waiving caches
  MUST NOT also waive unique work, as today's `dirty` does.
- **R12.** Dispose MUST offer a check-only mode that reports and deletes nothing.
  Today the refusal is the only report, and it costs a whole extra run.
- **R13.** Disposal of an integrated lane SHOULD take about as long as today's
  forced deletion (23-34 s for this workspace), and MUST NOT take longer than
  the operator's manual comparison plus the two runs it replaces.

### 3.4 Avoiding the hazards at clone time

- **R14.** `gwz local clone --clean` MUST either work or be refused at parse with
  a message saying so. Today it parses and is refused after the command starts.
- **R15.** A `--clean` lane MUST have nothing to waive: no copied ignored data,
  no copied stash entries, no reflog-only objects. Its disposal MUST need no
  waiver.
- **R16.** The clone mode MUST be recorded with the lane, so dispose applies the
  expectations of the mode that made it.

### 3.5 Traceability

- **R17.** Every requirement MUST be traceable to a test before it is accepted.
- **R18.** One test MUST build a workspace that has stashes, reflog-only
  commits, tagged and untagged caches, `__pycache__`, ignored user data and a
  merged lane, and prove that disposal needs no waiver.
- **R19.** One test MUST prove that a lane holding a unique commit, or unique
  ignored data, still refuses.

### 3.6 Lanes made by tools (added 2026-09-17)

Source: gwz-cli `dev-docs/GwzClaudeIntegrationPlan.md`, whose hooks create
and dispose lanes unattended, sometimes twice in parallel for one request.
Two facts drive these: the family lock is `try_lock` only, refusing `Busy`
without waiting (`FamilyStore::try_lock`, `flock(LOCK_EX | LOCK_NB)`), and
the family row records nothing about who asked for the lane.

- **R20.** `gwz local clone` MUST accept an owner token (`--owner <token>`,
  an opaque string up to 128 bytes of `[A-Za-z0-9._:-]`) and record it on
  the member row in the same index write that reserves the row. The token
  MUST be reported by `gwz local list` (human and `--json`), MUST never
  change after creation, and MUST NOT be interpreted by gwz: it is the
  caller's identity for the caller's own reuse decisions. A row with no
  token (a hand-made lane, or one made before the workspace's first owned
  lane) reports none. Compatibility: the row format is
  `deny_unknown_fields` and the index schema is matched exactly, so the
  field cannot be added silently. The index schema becomes
  `gwz.local-family/v2` with `owner` optional; a v2-aware gwz reads v1
  unchanged and writes v2 on its first write; an older gwz refuses a v2
  index as a whole, and its existing `wrong_schema` refusal MUST name the
  minimum gwz version that reads it. Every gwz binary used on one
  workspace must therefore be at or above the R20 release once any gwz at
  or above it has written the family index, and a dispose, a `--keep`, or
  a family merge is such a write, not only a create.
- **R21.** Every family command MUST accept `--wait <secs>`. With it, a
  `Busy` lock is retried until the deadline (polling `try_lock` at a short
  fixed interval; no blocking acquisition, so the wait is portable and
  cancellable) and only then reported as `Busy`. Without it, behaviour is
  unchanged: `Busy` is immediate. A wait that succeeds MUST reread the index
  before acting, so a create that waited behind another create of the same
  name reports the name as held, not a stale view.
- **R22.** One test MUST run two creates of the same name concurrently, one
  with `--wait`, and prove that exactly one lane exists afterwards, that the
  waiting create reports the name as held with the first create's owner
  token visible, and that the index was written once per create. A second
  test MUST read a v2 index with a v1-only decoder and prove the refusal
  names the minimum version.

Scope note: R20 and R21 are not gwz-core-only. The clap arguments for
`local clone` and the other family commands live in gwz-cli
(`src/clirequest/local.rs`), `docs/CLI.md` is generated and byte-compared
by a gwz-cli test and re-checked by its release gate, the long help is in
`src/local_long.rs` and `docs/commands/local.md`, the `local list` sample
is in `docs/LocalClones.md`, and the `local_family_members` fields are a
documented contract in `docs/MachineOutput.md`. Those edits land in
gwz-cli, in the same lane as the gwz-core change, under gwz-cli's review
loop.

## 4. What each cause needs

Counts are per lane, and were identical in every lane from round 2 on.

| Cause | Entries | Cleared by |
| --- | --- | --- |
| Commits only a reflog still reaches | 45 | R2, R3, R4 |
| `__pycache__/` directories | 25 | R5 |
| Stash entries (7 history roots, 3 `dirty`) | 10 | R2, R3, R4 |
| `CACHEDIR.TAG` caches: `target/`, `.venv/`, `.regen-venv/`, `.pytest_cache/`, `.ruff_cache/` | 13 | R5 |
| Bazel and razel output symlinks | 7 | R5 |
| Transient entries the recount could not identify | 6 | R2, R3 |
| Agent and editor settings, and `scratch/` | 3 | R2, R3 |
| gwz-py build output: `_gwz_core.abi3.so`, `src/gwz_py.egg-info/` | 2 | R5 |
| `gwz-cli/target`, untagged | 1 | R6 |
| Commits made in a lane and never integrated | 3, in 2 of 18 lanes | Nothing: must still refuse (R0.1) |

Regenerable classification (R5, R6) clears 48 of the 112 entries. Recognizing
the copy (R2, R3, R4) clears the other 64.

## 5. Non-goals

- No change to what counts as durable history outside the disposal of a copy.
  R4 applies only where the family still holds the identical object.
- The operator loss waiver stays for genuine loss. These requirements only
  remove waivers for what is not a loss.
- `--bare` lanes.
- The register's other lane issues: an untracked file blocking merges (L2), and a
  copied virtualenv pointing at the source workspace (L3).

## 6. Open questions

- Whether agent and editor state a lane changed (`.claude/`, `.cursor/`) counts
  as user work or as tool state.
- Whether `--clean` lanes are acceptable in practice, given that the verbatim
  copy exists to keep build caches warm.
- Whether dispose MAY write a durable ref in the family to preserve a unique
  root, instead of refusing.
