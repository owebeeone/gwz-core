# GWZ lane disposal clean-up plan

Status: **active** (written 2026-09-17). Owner: Gianni. Implements
`GwzLaneCleanFixes.md` requirements **R0 to R19**. It does **not** implement
that document's §3.6 (**R20 to R22**, the owner token, `--wait` and the
`gwz.local-family/v2` index schema): those are a separate, parallel package
whose edits land in `family-model`'s member row, `family-store` and gwz-cli.
This plan deliberately keeps out of the member row and out of the index
schema so the two packages can be merged in either order (§7).

Authorities, unchanged: `GWZDesign.md` for the workspace model, §4/§5 of the
local-clone design for create and dispose, `GWZRequirements.md` for the
baseline, and `GwzLaneCleanFixes.md` for what is required here. The lane
issues register (gwz-dev `dev-docs/GwzLaneIssues.md`, L1) is the observed
problem: **112 hazard entries, 60 `dirty` and 52 `unpreserved-history`,
identical in every lane**, because they describe what the copy inherited and
not what the lane did.

## 1. Objective

**R0.** `gwz local dispose <name>` on a lane whose work is already
integrated succeeds in one command: no waiver, no operator comparison.

**R0.1.** A lane holding anything the surviving family does not hold still
refuses.

The follow-on: once R0 holds in an *installed* gwz, an integrated lane
disposes in one command, which is exactly the precondition that the Claude
Code integration plan's **S3.5** ("adopt the disposal fixes", gwz-cli
`dev-docs/GwzClaudeIntegrationPlan.md`) waits on, and which its D3 lane-per-
subagent rule and its S3.4/S4.1/S4.2 revisions follow from. Nothing else in
this plan waits on gwz-cli.

## 2. The three facts the plan is built on

1. **Both hazards are the copy's, not the lane's.** A verbatim lane inherits
   every ignored entry (`dirty`) and every stash entry and reflog-only
   commit (`unpreserved-history`), because
   `gwz_history_check::is_eligible_witness_root` rightly excludes a
   *witness's own* reflog and stash roots from certifying durable history.
   That rule is correct in general and wrong for **deleting a copy**, where
   the family's own entry is untouched by the deletion (R4).
2. **Nothing records what was copied.** `install` writes the pointer
   (`.gwz/family-root`) and the allocation marker
   (`.gwz/local-clone-allocation`) into the destination and nothing else;
   dispose therefore has no baseline and has to treat every inherited entry
   as the lane's own work (R1, R2).
3. **The waiver vocabulary is two words wide.** `HazardWaiver` is
   `open-merge | dirty | unpreserved-history`, and
   `HazardKind::force_name` maps every work hazard — a `__pycache__`
   directory and a unique untracked source file alike — onto `dirty`. So
   waiving caches waives unique work (R11), and the refusal is the only
   report there is (R12).

## 3. Where the copy record lives (decision D1)

The record R1 requires is **not** a member-row field and **not** an index
field. It is its own file, `.gwz/local-clone-copy.yml`, written into the
**lane's** own `.gwz/` beside its pointer and marker, with its own frozen
schema string `gwz.local-clone-copy/v1`, `deny_unknown_fields`, and an
unknown schema refusing as *unknown evidence* (never silently clean),
exactly as the pointer and marker formats do.

- **Why not the row.** R20 (the parallel package) adds an optional `owner`
  field to the row and bumps the index schema to `gwz.local-family/v2`. A
  per-repository inventory of object ids and ignored-path fingerprints does
  not belong in an index that every family command rereads under the lock,
  and two packages editing one `deny_unknown_fields` row is a needless
  merge conflict.
- **Why the lane and not the root.** The record describes one lane; it is
  read only while disposing that lane; it is removed with the lane, so the
  root grows no orphan sidecars that family merge, disband and `--keep`
  would each have to clean up. The record is *evidence*, not authority: R2
  requires the entry to be **both** unchanged since the record **and** still
  present in the family, so a record inside the tree being deleted cannot
  certify a loss on its own.
- **Rejected alternative,** recorded for review: `<root>/.gwz/local-clone-
  copy/<name>.yml`. It resists tampering by the lane, at the cost of
  sidecar lifecycle in four more verbs. Revisit only if a threat model
  appears; the corroboration rule above makes tampering useless today.

**Decision D2: no new crate.** The record's model, codec and comparison live
in gwz-core's own `src/local_clone/copy_record.rs`; `gwz-local-disposal` and
`gwz-work-detector` gain only plain data types they already own the shape of
(they hold no core or protocol types, and that stays true).

**Decision D3: the record is taken from the destination**, after the copy
and before the pointer. The copy is verbatim, so at that instant the
destination *is* what was copied; taking the fingerprints from the
destination means dispose later compares like with like and never races the
source.

## 4. Phases

Each phase is a shippable increment. Each step is one goal with an
aspirational **< 500 LOC** budget (tests included). Steps inside a phase are
written so different agents can take them independently; the dependency
sketch in §6 is the real ordering.

### Phase 1: dispose recognises the copy (milestone: an integrated verbatim lane of a workspace that has stashes and reflogs disposes with no `unpreserved-history` waiver, and unchanged copied data is not `dirty`)

Clears the 64 of 112 entries that recognising the copy accounts for
(`GwzLaneCleanFixes.md` §4). Foundational: Phases 2 to 4 all report through
the categories Step 1.7 introduces.

- **S1.1: the copy record — model, codec, constant** *(gwz-core
  `src/local_clone/copy_record.rs`, new; ~300 lines with tests)*. **R1
  (format half).** `CopyRecord { schema, family_id, allocation_id,
  source_path, repositories }`; `RepositoryCopy { key, protected_roots,
  ignored }`; `CopiedRoot { source_kind, reference, index, oid }`;
  `CopiedEntry { path (bytes), kind, size, mtime_nanos, inode }`. Frozen
  schema string, `deny_unknown_fields`, byte round-trip, an unknown schema
  and an unknown field each refusing with a named detail. Pure: opens
  nothing. No dependency on any other step.

- **S1.2: write it at clone time** *(gwz-core
  `src/local_clone/adapters/install.rs` + `crates/workspace-install/src/
  {ports,install,report,test_support}.rs`; ~400 lines)*. **R1 (write
  half).** One new port, `InstallPorts::record_copy(&ConfigurationPlan-like
  RecordRequest) -> Result<CopyReceipt, InstallPortError>`, one new
  `InstallStep::RecordCopy` and one new `InstallEffect::CopyRecorded`,
  called **after `install_destination_git` and before `install_pointer`**;
  the port call order in `ports.rs`'s doc comment is part of the contract
  and is updated with it. Core's adapter inventories the destination's
  repositories with `gwz-repo-inspect` (`inventory_history` for the roots,
  `observe_work` for the ignored and untracked entries, `fs::metadata` for
  the fingerprints) and writes S1.1's file. Depends on S1.1.

- **S1.3: a named witness policy for deleting a copy** *(gwz-core
  `crates/history-check/src/lib.rs` + its tests; ~250 lines)*. **R4.**
  `check_history` gains an explicit `WitnessPolicy`: `Durable` (today's
  behaviour, the default everywhere else, unchanged) and `IdenticalCopy`,
  under which a witness's **own reflog and stash roots are eligible**,
  because deleting a copy leaves the witness's entry untouched. The
  `is_eligible_witness_root` free function keeps its present meaning and
  gains the policy-aware sibling; the connectivity walk is untouched.
  Refusal still stands when **no** witness holds the object at all (R0.1).
  Independent of S1.1/S1.2.

- **S1.4: the copy record narrows the history question** *(gwz-core
  `src/local_clone/adapters/disposal.rs`, `crates/local-disposal/src/
  {ports,inspect}.rs`; ~350 lines)*. **R2 (history half), R4 (wiring).**
  `TargetEvidence` gains `copy: Option<CopyWitness>` — a plain data struct
  owned by `gwz-local-disposal`, carrying the record's protected roots per
  repository. Core's `check_history` adapter runs the paired family witness
  under `WitnessPolicy::IdenticalCopy` when the target is a verbatim local
  clone; a root the record listed and the witness still holds is
  `unchanged-copy`, not `unpreserved-history`. Depends on S1.1, S1.3.

- **S1.5: the copy record narrows the work question** *(gwz-core
  `src/local_clone/adapters/disposal.rs` + `crates/work-detector/src/
  {hazard,builder}.rs`; ~400 lines)*. **R2 (work half), R8.** A fresh
  ignored or untracked entry whose kind, size, mtime and inode match the
  record's is `unchanged-copy`; anything else stays reported exactly as
  today. `HazardKind` gains the provenance, not a new force name yet —
  narrowing the waiver vocabulary is Phase 3, so this step changes *what
  refuses*, never *how it is spelled on the wire*. Depends on S1.1.

- **S1.6: no record — dispose makes the comparison itself** *(gwz-core
  `src/local_clone/adapters/disposal.rs`; ~400 lines)*. **R3.** A lane made
  by an older gwz or copied outside gwz has no record. Dispose then derives
  the same witness live, per repository, against the paired family
  repository: the protected roots no surviving family repository holds, and
  the ignored and untracked entries that differ from the family's. Finding
  nothing unique needs **no waiver**. Depends on S1.4, S1.5 (it feeds the
  same classification); it can be drafted against their types in parallel.

- **S1.7: hazards by category, and the exact waiver command** *(gwz-core
  `src/local_clone/dispose.rs` and `crates/local-disposal/src/
  {hazard,report}.rs`; ~350 lines)*. **R9, R10.** A refusal reports by
  category — `regenerable`, `unchanged-copy`, `changed-copy`, `unique` —
  each with its count and its paths or object ids, and prints the **exact**
  command that waives **exactly** the categories it found, not a generic
  `--force <hazard,...>` hint. In Phase 1 the `regenerable` category is
  present and empty; Phase 2 fills it. Depends on S1.4, S1.5.

- **S1.8: Phase 1 traceability** *(gwz-core
  `crates/local-disposal/src/tests/`, `src/local_clone/tests/`; ~400
  lines)*. **R17 for R1 to R4, R9, R10; R19's history half.** A fixture
  family whose root and members carry native stashes and reflog-only
  commits, cloned as a verbatim lane and merged back: dispose needs no
  `unpreserved-history` waiver. A second fixture whose lane holds a commit
  no family repository holds: dispose still refuses, naming it `unique`.
  Depends on S1.2, S1.4, S1.6, S1.7.

### Phase 2: regenerable data is not user work (milestone: a lane's caches, `__pycache__`, egg-infos, build symlinks and compiled extensions never refuse a disposal)

Clears the other 48 of the 112 entries.

- **S2.1: the regenerable recogniser** *(gwz-core `crates/repo-inspect/src/
  work.rs` or a sibling module; ~400 lines)*. **R5, R7.** Pure recognition,
  by marker and shape and never by name alone: a directory holding a valid
  `CACHEDIR.TAG` (the literal first-line signature, checked, not the file's
  mere presence); `__pycache__/`; `*.egg-info/`; a symlink pointing outside
  the workspace whose name matches a build tool's convenience-link shape
  (`bazel-*`, `razel-*`); `*.so`, `*.pyd`, `*.dylib` inside a source tree.
  **R7:** recognition never consults the copy record — a cache the lane
  rebuilt is still a cache. Independent of Phase 1.

- **S2.2: tools that write no `CACHEDIR.TAG`** *(same module; ~250 lines)*.
  **R6.** `gwz-cli/target`, built by an older cargo, has no tag: recognise
  it by the markers cargo does write (`.rustc_info.json`, `CACHEDIR.TAG`
  when present, the `debug/`/`release/` shape) and record each tool's
  marker set in one table with a test per entry. A directory named `target`
  with none of them is **not** regenerable. Depends on S2.1.

- **S2.3: regenerable in the classification and the report** *(gwz-core
  `crates/work-detector/`, `crates/local-disposal/`, `src/local_clone/
  adapters/disposal.rs`; ~300 lines)*. **R5 (wiring), R8.** Regenerable
  entries enter S1.7's `regenerable` category; anything neither regenerable
  nor unchanged since the copy is still reported, under `changed-copy` or
  `unique`. Depends on S1.7, S2.1.

- **S2.4: Phase 2 traceability** *(tests; ~300 lines)*. **R17 for R5 to
  R8; R19's ignored-data half.** One test per recogniser, one proving a
  rebuilt cache is still recognised (R7), one proving an untagged
  `target/` is recognised by its markers and a same-named non-build
  directory is not (R6), and one proving a lane holding ignored data the
  family lacks still refuses (R19). Depends on S2.3.

### Phase 3: waivers that fit, and a report that costs one run (milestone: `--force` names exactly one category, and `--check` reports without deleting)

- **S3.1: narrow waiver names** *(gwz-core `crates/local-disposal/src/
  hazard.rs`, `crates/work-detector/src/hazard.rs`; ~350 lines)*. **R11.**
  `HazardWaiver` gains `regenerable`, `copied-data` and `lane-data`;
  `HazardKind::force_name` maps onto them one-to-one, so waiving caches no
  longer waives unique work. `dirty` stays parseable as the union of the
  three for one release, reported as deprecated in the refusal text;
  `open-merge` and `unpreserved-history` are unchanged. Depends on S2.3.

- **S3.2: check-only mode** *(gwz-core `crates/local-disposal/src/
  {policy,run}.rs`, `src/local_clone/dispose.rs`, and gwz-cli
  `src/clirequest/local.rs`, `src/local_long.rs`, `docs/commands/local.md`,
  `docs/CLI.md` regenerated; ~450 lines)*. **R12.** `DisposePolicy::Check`
  runs steps 1 and 3 and stops: it observes, classifies, reports every
  category and removes nothing, writes no row state, and exits 0 whatever
  it finds. **Touches the CLI surface**: regenerate `docs/CLI.md` with
  `python3 scripts/generate_cli_reference.py --write` from the gwz-cli
  directory, which its g00 test byte-compares. Depends on S1.7, S3.1.

- **S3.3: the time budget** *(gwz-core `crates/local-disposal/`,
  `src/local_clone/adapters/disposal.rs`, plus a timed acceptance; ~300
  lines)*. **R13.** Disposal of an integrated lane must stay in the band
  today's forced deletion occupies (23–34 s for this workspace) and must
  not exceed the manual comparison plus the two runs it replaces. The work
  the record adds is one `stat` per recorded entry and one extra witness
  walk; the step measures it on this workspace, records the numbers beside
  the plan, and caps the per-repository comparison so an unusually large
  lane degrades to `Unknown` rather than to an unbounded walk. Depends on
  S1.6, S2.3.

### Phase 4: lanes with nothing to waive (milestone: `--clean` works, its lanes dispose with no waiver, and the mode is honoured at dispose)

- **S4.1: `--clean` refuses at parse, or works** *(gwz-cli
  `src/clirequest/local.rs` and gwz-core `src/local_clone/request.rs`;
  ~200 lines)*. **R14.** Today `clone_local` refuses `CloneMode::Clean` as
  `unsupported` **after** the command starts. Until S4.2 lands, the refusal
  moves to request validation, before any observation, with a message that
  says the mode is not implemented. If S4.2 lands in the same release this
  step is folded into it. Touches the CLI surface: regenerate `docs/CLI.md`.
  Independent of Phases 1 to 3.

- **S4.2: `--clean` lanes** *(gwz-core `crates/workspace-install/src/`,
  `src/local_clone/adapters/install.rs`; ~450 lines)*. **R15.** A clean
  lane copies no ignored data, no stash entries and no reflog-only objects:
  it is constructed through the existing `construct_repositories` path at
  the frozen commit rather than by tree copy, so its disposal has nothing
  to waive. Its copy record (S1.2) records an empty inventory, which is
  itself the proof. Depends on S1.2, S4.1.

- **S4.3: the mode is applied at dispose** *(gwz-core
  `src/local_clone/adapters/disposal.rs`; ~150 lines)*. **R16.** The clone
  mode is **already** on the member row (`MemberRow::mode: CloneMode`), so
  no schema change is needed and none is made — see §7. Dispose reads it
  and applies the expectations of the mode that made the lane: a `Clean`
  lane reporting copied ignored data or copied stash entries is a
  contradiction and refuses as `PathMismatch`, not as ordinary dirt.
  Depends on S1.5, S4.2.

- **S4.4: the whole-workspace proof** *(gwz-core tests plus one operator
  acceptance run; ~400 lines)*. **R18, and R17's close-out.** One test
  builds a workspace holding stashes, reflog-only commits, a tagged cache,
  an untagged cache, `__pycache__`, ignored user data and a merged lane,
  and proves disposal needs **no waiver** — R0 end to end. The operator
  acceptance repeats it against this workspace with the installed gwz and
  records the counts (expected: 0 where 112 stood) and the timing beside
  this plan. Depends on every preceding phase.

## 5. Requirement-to-step map

| Req | Step | Req | Step |
| --- | --- | --- | --- |
| R0 | S4.4 (proved); S1.*, S2.* (delivered) | R10 | S1.7 |
| R0.1 | S1.3, S1.8, S2.4 | R11 | S3.1 |
| R1 | S1.1, S1.2 | R12 | S3.2 |
| R2 | S1.4, S1.5 | R13 | S3.3 |
| R3 | S1.6 | R14 | S4.1 |
| R4 | S1.3, S1.4 | R15 | S4.2 |
| R5 | S2.1, S2.3 | R16 | S4.3 |
| R6 | S2.2 | R17 | S1.8, S2.4, S4.4 |
| R7 | S2.1 | R18 | S4.4 |
| R8 | S1.5, S2.3 | R19 | S1.8 (history), S2.4 (data) |
| R9 | S1.7 | R20–R22 | **not this plan** (§3.6, parallel package) |

## 6. Dependency sketch

```text
S1.1 ─┬─> S1.2 ──────────────────────────┐
      ├─> S1.5 ─┬─> S1.7 ─┬─> S1.8 <─────┤
S1.3 ─┴─> S1.4 ─┘         │              │
                 S1.6 ────┘              │
S2.1 ─> S2.2 ─> S2.3 ─┬─> S2.4           │
                      ├─> S3.1 ─> S3.2   │
                      └─> S3.3 <─ S1.6   │
S4.1 ─> S4.2 <─ S1.2                     │
        S4.2 ─> S4.3 <─ S1.5             │
        S4.4 <─ (S1.8, S2.4, S3.2, S4.3) ┘

R0 in an installed gwz ─> gwz-cli GwzClaudeIntegrationPlan S3.5
```

Four starting points can be taken up at once by four agents: **S1.1**,
**S1.3**, **S2.1** and **S4.1**. Phase 2 is independent of Phase 1 up to
S2.3, and Phase 4's S4.1 is independent of everything.

## 7. What this plan does and does not touch, for the R20 merge

The R20–R22 package (owner token, `--wait`, `gwz.local-family/v2`) is being
implemented in parallel. To keep the two merges independent:

- **No member-row field is added, removed or reordered here.** R16 is
  satisfied by the row's existing `mode` field, read only.
- **No index schema change here.** The copy record is its own file with its
  own schema string (§3).
- **The create step order is extended, not reordered** (S1.2): one new step
  between `install_destination_git` and `install_pointer`. R20 writes the
  owner token in the same index write that reserves the row — `Reserve`,
  two steps earlier — so the two edits do not overlap in
  `crates/workspace-install/src/install.rs`, though both add an
  `InstallStep`/`InstallEffect` variant and the enum lists will conflict
  textually. Resolve by keeping both variants in call order.
- **Both packages touch gwz-cli's command surface** (S3.2 and S4.1 here;
  R20's `--owner` and R21's `--wait` there) and therefore both regenerate
  `docs/CLI.md`. Whichever lands second regenerates again.
- **Open question for the merge:** if R20's v2 index arrives first, S1.2's
  record write happens under a v2-aware store; nothing here reads or writes
  the index directly, so no change is expected — but the first lane to
  carry both should be disposed once by hand as a check.

## 8. Implementation status

Landed in gwz-core, one commit per step, each on a green
`python3.13 scripts/run_tests.py` and
`cargo clippy --all-targets -- -D warnings`:

| Step | State | What landed |
| --- | --- | --- |
| S1.1 | **done** | `src/local_clone/copy_record.rs`: the frozen `gwz.local-clone-copy/v1` format, its codec, the percent-escaped byte paths, the one-`stat` fingerprint, and read/write at `.gwz/local-clone-copy.yml`. |
| S1.2 | **done** | `InstallPorts::record_copy`, `InstallStep::RecordCopy`, `InstallEffect::CopyRecorded`, called between `install_destination_git` and `install_pointer`; core's adapter inventories the **destination**, strips GWZ's own structural paths with disposal's own function, and writes the record. |
| S1.3 | **done** | `gwz-history-check`: `WitnessPolicy`, `is_eligible_witness_root_under` and `check_history_under`. `check_history` and `is_eligible_witness_root` are unchanged and are the `Durable` policy. |
| S1.4 | **history half done** | Disposal checks a **verbatim** lane's history under `WitnessPolicy::IdenticalCopy`, so a lane that only copied the family's reflog and stash entries needs no `unpreserved-history` waiver. See the adjustment below. |
| S1.5 | **done** | The record narrows the **work** question: disposal reads the lane's record back, corroborates each ignored or untracked entry against the surviving family's paired repository, and hands `gwz-work-detector` a `CopyBaseline` per repository through `TargetEvidence::copy`. A copied entry that is unchanged and still in the family, and a native stash the copy brought and the family still holds, are reported and no longer refuse. |
| S1.6 | **done** | No record -- an older gwz's lane, or one copied outside gwz -- and dispose derives the same witness live: each ignored and untracked entry is compared with the family's own entry at that path, by link target, by bytes or (past 1 MiB) by size and mtime, recursively for a directory and within a bounded walk. Finding nothing unique needs no waiver. |
| S1.7 | **done** | `gwz-local-disposal` owns `HazardCategory`, `categorise` and `required_waivers`; a refusal reports `regenerable`, `unchanged copy`, `changed copy` and `unique to the lane`, each with its count and its paths or object ids and each named even when empty, then prints the one `gwz local dispose <name> --force <hazards>` that waives exactly what refused. |
| S1.8 | **done** | `src/local_clone/tests/dispose/phase1.rs`: a workspace shaped like the one the register measured -- root and member, each with a native stash, a reflog-only commit, ignored user data, a tagged cache, a `__pycache__`, a compiled extension and a `bazel-out` link -- cloned verbatim. Its lane's work is merged back and the lane then disposes in **one command** (R0); a lane holding a unique commit, or unique ignored data, still refuses under `unique to the lane` (R0.1, R19); and a third test states exactly what Phase 1 leaves. |
| Phases 2 to 4 | not started | |

**What Phase 1 leaves, measured (2026-09-18).** On the S1.8 fixture, a
merged verbatim lane that has been **built in** refuses over exactly one
thing: the cache directory it rebuilt, reported as

```text
regenerable 0; unchanged copy 11: ...; changed copy 1: `mem_app` ignored
user data (__pycache__/); unique to the lane 0; to delete anyway ...
`gwz local dispose A --force dirty`
```

The history, the native stashes, the untouched ignored user data and the
untouched caches are all cleared. The one remaining entry is cleared by
**S2.3**, which wires S2.1's `__pycache__/` recogniser (and S2.2's untagged
build directories) into the `regenerable` category this refusal already
prints and leaves empty. A lane that was never built in needs no waiver at
all today.

**Known limitation: R1's fingerprint is one `stat` (2026-09-18).** In the
same fixture the lane also rewrote `target/debug/build.bin`, two levels
below the recorded directory entry `target/`, and one `stat` of `target/`
does not see it, so that entry reads as an unchanged copy. R1 asks for
exactly that cheap fingerprint (size, mtime, inode) and the register's
caches are recorded as whole directories, so this is the common case and
not an accident -- but it does mean a change deep inside a recorded
directory can be cleared. The live comparison S1.6 makes for a lane with
**no** record does not have this gap: it compares bytes, recursively,
within a bounded walk. **S3.3** prices making the recorded path do the same
for directory entries; until it does, this is the one place where the
record is weaker than the comparison it replaces.

**S1.4 adjustment.** As planned, S1.4 was to let the copy record narrow the
history question through a `CopyWitness` on `TargetEvidence`. Building it
showed the record is not needed for **R4**: R4's test is "the surviving
witness holds the identical object", which the witness policy answers on its
own, at the same object id and with the same whole-subgraph proof, and a
root no witness holds at all is still unpreserved under either policy
(R0.1). The record's narrowing is therefore **R2's** business, not R4's, and
`CopyWitness` moves into S1.5 with the rest of R2. Nothing in the
requirement map changes: R4 is S1.3 plus this wiring, R2 is S1.5.

**S1.5 adjustments (2026-09-18).** Five, each recorded where the built
shape is not the step text's:

1. **The provenance is on `Hazard`, not on `HazardKind`.** The step text
   says the kind gains it. `HazardKind::force_name` must stay a function of
   the kind *alone* -- that is exactly what "changes what refuses, never how
   it is spelled on the wire" means -- and a kind carrying provenance would
   have made every existing `HazardKind::Work(kind)` comparison
   provenance-sensitive for no gain. The field sits beside `kind` on
   `Hazard`, and `force_name` is untouched.
2. **`CopyWitness` carries the classified baseline, not the record's raw
   protected roots.** The S1.4 adjustment above already found that the
   roots' own question is answered better by `WitnessPolicy::IdenticalCopy`
   than by any record. What survives of the roots in S1.5 is the one work
   hazard they cause: the native stash, cleared only when the copy brought
   every stash entry **and** the family still holds every one of them.
3. **Corroboration is against the family root's paired repository**, the
   one S1.6's own text names, not against every surviving lane. A lane is
   not evidence that another lane's data is safe, and polling every lane
   would make the comparison cost grow with the number of lanes (R13).
4. **Only ignored and untracked entries are cleared.** The step text says
   "a fresh ignored or untracked entry"; tracked dirt -- staged, unstaged,
   conflict, rename, deletion -- stays reported whatever the record says,
   which is the conservative reading of R2.
5. **A hazard that does not refuse is still reported.** R9 asks the refusal
   to name the unchanged-copy category, so the classifier keeps the hazard
   and marks it, and `gwz-local-disposal` refuses only when some finding
   refuses. Nothing is hidden and no hazard is dropped.

**S1.6 adjustments (2026-09-18).** Two:

1. **R3's history half needed no new code and got none.** The step text has
   dispose derive "the protected roots no surviving family repository
   holds" live. `check_history` already does exactly that, for every lane,
   recorded or not: it walks the lane's whole protected inventory --
   `rev-list --all`'s commits through the refs that reach them, the reflog
   and the stash -- against each surviving family repository's own object
   store under `WitnessPolicy::IdenticalCopy`, and a root no witness holds
   is unpreserved (R0.1). The live witness therefore carries only the work
   half plus the native stash's work hazard.
2. **The comparison is bounded, and the bound refuses.** R13 forbids an
   unbounded walk, so one entry's comparison visits at most 4096
   filesystem entries and reads at most 1 MiB per file, falling back to
   size and modification time above that. Exceeding the bound makes the
   entry the lane's -- a refusal the operator can still waive -- never a
   silent pass and never unwaivable unknown evidence. S3.3 is where the
   numbers are measured and tuned.

**S1.7 adjustments (2026-09-18).** Two:

1. **The category vocabulary is not the waiver vocabulary**, and Phase 1
   keeps them apart deliberately. `HazardCategory` is the report's;
   `HazardWaiver` is still `open-merge | dirty | unpreserved-history` on
   the wire, so several categories share the `dirty` name until R11 (S3.1)
   narrows it. That is exactly why R10's printed command is computed rather
   than left to the operator.
2. **The per-finding `<waiver>` marker is gone from the refusal text.** A
   refusal used to read `` `mem_app` <dirty>: ... ``; it now leads with the
   categories and ends with the command. The waiver names are still in the
   message -- in the command that carries them -- and the tests assert them
   by parsing that command, which pins R10 rather than the prose.

## 9. Open questions carried from the requirements

- `GwzLaneCleanFixes.md` §6: whether agent and editor state a lane changed
  (`.claude/`, `.cursor/`) is user work or tool state. This plan classifies
  it as **changed copy** (S1.5) and so refuses until waived, which is the
  conservative answer; a decision to call it tool state is a one-line
  change to the recogniser table in S2.1.
- Whether `--clean` lanes are acceptable in practice, given the verbatim
  copy exists to keep build caches warm (S4.2 delivers the mode; the
  practice question stays the operator's).
- Whether dispose MAY write a durable ref in the family to preserve a
  unique root instead of refusing. **Out of scope here:** R0.1 says a lane
  holding the only copy refuses, and this plan keeps that.
