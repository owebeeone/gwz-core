# GWZ Merge Design

Status: **proposed** (2026-07-16). Owner: Gianni.
Revision: addresses `GwzMergeDesign-ReviewF5.md` and
`GwzMergeDesign-ReviewF5-2.md` (2026-07-16), plus explicit workspace-root merge
participation and the `BranchOp.merge` wire disposition identified by
`GwzMergePlan-ReviewF5.md`.

`GWZDesign.md` remains authoritative for the overall workspace model and
`GWZRequirements.md` remains the baseline for required behavior. This document
is the design checkpoint for promoting merge to a first-class GWZ operation and
for managing a coordinated merge across repositories through success,
conflict, continuation, or abort.

## 1. Problem

GWZ already implements selection-wide merge behavior as:

```text
gwz branch --merge <source>
```

That surface is difficult to discover and does not match GWZ's goal of making
the primary Git verbs available as primary workspace verbs. Merge is not merely
branch administration; it is a stateful integration operation with its own
conflict, continuation, and abort lifecycle.

The existing implementation also exposes a specifically multi-repository
problem. One merge attempt can leave selected members in different states:

- a member may already contain the source and require no change;
- another may fast-forward or create a clean merge commit;
- another may enter an unresolved native Git merge;
- an unexpected host or Git error may stop execution after earlier members
  changed.

GWZ must retain enough evidence to understand that mixed state, safely finish
it, or safely roll back the changes made by the coordinated attempt. It must
not pretend that independent Git repositories form a transactional database.

## 2. Goals

- Add `gwz merge` as a first-class CLI and protocol operation.
- Preserve ordinary Git merge semantics within every selected repository.
- Resolve the source independently in each selected repository.
- Preflight every selected repository before mutation.
- Report conflicts as expected structured outcomes, not generic errors.
- Record a durable local coordinated-merge state before the first mutation.
- Support workspace-level `--continue` and `--abort` operations.
- Make an abort selection-wide: either every required rollback is eligible
  before rollback begins, or no rollback begins.
- Detect changes made after a partially successful merge and refuse to destroy
  them silently.
- Provide an explicit preservation path for post-merge work that would
  otherwise block rollback.
- Refresh and persist the workspace composition only after the coordinated
  merge completes successfully.
- Exclude the workspace root by default, but support it as an explicitly
  selected merge participant with composition-aware recovery and finalization.
- Preserve machine-readable parity across Rust, Python, JSON, and JSONL
  drivers.

## 3. Non-goals

- No claim of distributed transactional atomicity across Git repositories.
- No automatic push as part of merge.
- No silent skipping of members that lack the source ref.
- No automatic conflict resolution.
- No automatic reapplication of work preserved during abort.
- No `--squash`, arbitrary merge strategies, or `--no-commit` in the initial
  first-class merge release.
- No `--adopt` of work created after a recorded merge result. Drift must be
  resolved outside the open lifecycle unless a later design adds adoption as
  distinct evidence.
- No implicit switch to an explicitly named target branch in the first
  release. Merge targets each selected repository's current attached branch.
- No implicit participation by the workspace root. Root merge is opt-in
  because it can change the manifest, lock, and composition ledger while the
  operation is in flight.

## 4. Existing behavior and migration

The current `BranchOp.merge` implementation already provides a useful base:

- default selection is active workspace members;
- all selected members are structurally preflighted before mutation;
- selected members must be materialized, clean, attached, non-unborn, free of
  merge/rebase state, and able to resolve the source;
- ordinary Git merge analysis produces up-to-date, fast-forward, clean
  three-way merge, or conflicted outcomes;
- conflicts remain in native Git merge state with conflict paths reported;
- clean member results refresh the workspace lock, including when another
  selected member conflicts.

That last behavior means the interim start-only implementation advances the
recorded composition for its clean members. The durable lifecycle deliberately
changes this: once M1 lands, an open merge keeps the lock at the complete
pre-merge baseline until finalization. M0 retains the legacy partial lock
advance because freezing the lock without a durable close/recovery lifecycle
would leave no safe way to advance it later. M0 and M1 are internal delivery
checkpoints, so first-release user and machine-output documentation describes
only the durable baseline-lock behavior delivered with the complete member
lifecycle.

That M0 rule also applies when an unexpected backend or host failure halts a
batch: every earlier outcome that was verified clean is written to the lock,
the failed participant is reported honestly, and later participants remain
unattempted. M1 deliberately replaces this behavior with its durable baseline
lock and recovery lifecycle.

The first-class design supersedes `BranchOp.merge` as the canonical protocol
surface. `gwz branch --merge <source>` remains temporarily as a deprecated CLI
compatibility alias which constructs the new `MergeRequest`. It must produce a
merge action and merge response, not a branch action disguised by the CLI.

The existing `BranchOp.merge` wire value remains reserved at its current enum
number for append-only compatibility, but M0 removes its core behavior. After
M0, a protocol-level `BranchRequest { op: merge }` returns the appended typed
`deprecated_operation` error with replacement method `merge`; it is never
lowered internally to a second merge path. Rust and Python CLI compatibility
syntax intercepts `branch --merge` before constructing a `BranchRequest` and
submits `MergeRequest { op: start }` instead. Direct protocol clients must move
to the `merge` method.

Because GWZ is pre-1.0, the protocol should be corrected now rather than making
`merge` a permanent alias over semantically misleading branch messages.

## 5. CLI surface

Target lifecycle surface:

```text
gwz merge <source>
gwz merge <source> --dry-run
gwz merge --continue
gwz merge --abort
gwz merge --status
```

Preserving post-merge work before an abort is explicit:

```text
gwz merge --abort --preserve
```

Later compatible additions:

```text
gwz merge <source> --ff-only
gwz merge <source> --no-ff
gwz merge <source> -m <message>
gwz merge +<snapshot>
gwz merge <source> --into <branch>
gwz merge --gc [<merge-id>]
```

The delivery phases in section 19 introduce this surface incrementally. M0 has
only start and dry-run; it must not advertise continue or coordinated abort in
its conflict epilogue. Status arrives with the durable operation record in M1,
and continue and abort arrive in M2.

Global GWZ selection and output options continue to apply. `--dry-run` applies
only to merge start. Until an explicit partial/skip policy is designed,
`--partial` is a typed rejection for every merge operation. `--force` is also
a typed rejection and must never lower to a force-abort or bypass a drift
check. The deprecated `branch --merge` alias applies the same validation.

With no explicit selection, merge targets all active members and excludes the
workspace root. An explicit `--target @root` selects the root repository,
either alone or alongside explicitly selected members under the normal
selection rules. Human and machine plans must make root participation visible.

Root participation depends on the durable continue/abort lifecycle and scoped
finalization described below, so it is enabled in M2. Earlier delivery phases
return a typed `root_merge_not_yet_supported` result for explicit `@root`;
they do not describe root as permanently invalid.

Only one coordinated merge may be open in a workspace at a time.

## 6. Source and target semantics

For:

```text
gwz merge feature/refactor
```

each selected repository independently resolves `feature/refactor` to a Git
commit and merges that commit into its currently attached branch. The source
name does not identify one shared object id across repositories.

The source must resolve in every selected repository. A missing source rejects
the whole operation before mutation. A user who intends to merge only the
repositories where a branch exists must select those repositories explicitly.
A future explicit skip policy may be added, but skipping must never be the
default.

Selected repositories may currently be attached to different target branch
names. This is valid because the Git-equivalent target is the current branch of
each repository. Human and machine plans must display the per-target mapping
before or during execution:

```text
app   feature/refactor -> main
lib   feature/refactor -> main
docs  feature/refactor -> release
@root feature/refactor -> main  (explicit only)
```

The selected participant set and its source/target plan are frozen from the
pre-merge manifest and lock. A later root merge may change workspace metadata,
but it does not add, remove, retarget, or reorder participants in the already
running operation.

`--into <branch>` is deferred because switching target branches while an
operation is in flight adds a second coordinated mutation and additional
rollback requirements.

## 7. Start preflight

`gwz merge <source>` acquires the workspace-wide mutator lock and builds a
complete plan before mutating any repository. Dry-run performs the same
read-only planning without acquiring the mutator lock. `gwz merge --status`
also holds no mutator lock; continue, abort, preserve, and garbage collection
do acquire it.

Every selected repository must satisfy all of the following:

- a member is active and present in the lock, or the target is the explicitly
  selected `@root` repository;
- the checkout exists and is an ordinary Git repository;
- the worktree and index are clean;
- HEAD is attached to a local branch;
- HEAD is not unborn;
- no merge, rebase, cherry-pick, or other incompatible integration state is in
  progress;
- the source resolves to a commit;
- the target branch ref still points to the observed HEAD;
- Git identity is available if the planned merge may create a commit;
- the repository can be locked for the operation.

Every backend failure discovered while preflighting a participant retains its
typed backend error code and carries that participant's id and path in both the
human diagnostic and structured error fields. Request-supplied author and
committer identities are validated for libgit2 representability before
planning or mutation begins.

The plan records, per participant:

- target id (`@root` or member id) and path;
- target branch;
- before commit;
- source text and resolved source commit;
- predicted integration kind when available;
- whether a commit identity will be required.

`--dry-run` returns this plan and performs no mutation. Git merge analysis can
reliably classify up-to-date, fast-forward, and true-merge cases, but a dry run
must not promise that a true merge is conflict-free unless the backend performs
an equivalent non-mutating tree merge. The response states that limitation.
The optional `merge_simulate` seam in section 17 allows that limitation to be
removed later without changing the response or operation-record shape. Because
dry-run deliberately does not hold the workspace mutator lock, its plan is
advisory and may be stale by the time a later real merge begins; the real merge
always repeats preflight under the lock.

`--ff-only` can be enforced entirely during preflight: if any selected repository
requires a true merge, the whole operation is rejected before mutation.

When `@root` is selected, preflight additionally records the exact manifest and
lock digests and verifies their bytes are available from the recorded root
before-commit tree. It validates that the operation can be recovered from
`.gwz/merge/` without reparsing possibly conflicted root metadata, and reserves
the root as the final execution participant. This is necessary because the root
merge can legitimately change `gwz.conf/`.

## 8. Durable coordinated-merge record

After successful preflight and before the first participant mutation, core
writes a local operation record:

```text
.gwz/merge/<merge-id>.yaml
```

The record is runtime recovery state and is not committed to the workspace
root. Final completed composition evidence belongs in the root's versioned GWZ
metadata.

Proposed record shape:

```yaml
schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: 0.1.0
workspace_id: ws_default
merge_id: merge_...
operation_id: op_...
state: executing
source_ref: feature/refactor
created_at: 2026-07-16T00:00:00Z
baseline:
  lock_sha256: 9f86d081...
  manifest_sha256: 60303ae2...
  root_head: ddd111
selected_targets:
- mem_app
- mem_lib
- mem_docs
- '@root'
participants:
  mem_app:
    path: app
    target_kind: member
    target_branch: main
    before_commit: aaa111
    source_commit: aaa999
    commit_message: "Merge 'feature/refactor' into 'main'"
    state: planned
  mem_lib:
    path: lib
    target_kind: member
    target_branch: main
    before_commit: bbb111
    source_commit: bbb999
    commit_message: "Merge 'feature/refactor' into 'main'"
    state: planned
  mem_docs:
    path: docs
    target_kind: member
    target_branch: release
    before_commit: ccc111
    source_commit: ccc999
    commit_message: "Merge 'feature/refactor' into 'release'"
    state: planned
  '@root':
    path: .
    target_kind: root
    target_branch: main
    before_commit: ddd111
    source_commit: ddd999
    commit_message: "Merge 'feature/refactor' into 'main'"
    state: planned
```

Participant lifecycle values must distinguish at least:

```text
planned
up_to_date
fast_forwarded
merged
conflicted
failed
unattempted
continued
aborted
rolled_back
```

Operation lifecycle values must distinguish at least:

```text
executing
awaiting_resolution
halted
finalizing
preserving
rolling_back
completed
aborted
recovery_required
```

The operation states have the following meanings:

- `executing`: start or resume is applying the frozen participant plan;
- `awaiting_resolution`: at least one expected content conflict needs user
  resolution and no unexpected failure is the primary blocker;
- `halted`: an unexpected host/backend failure stopped later participants, but
  all recorded state remains classified as retryable or abortable;
- `finalizing`: all participant merges succeeded and core is creating or
  publishing the candidate lock, boundary, and root evidence;
- `preserving`: explicit preserve-abort is creating and verifying recovery
  artifacts before rollback;
- `rolling_back`: coordinated abort has passed whole-operation preflight and is
  unwinding recorded mutations;
- `completed`: final composition and evidence were published and verified;
- `aborted`: all required merge mutations were restored to the baseline;
- `recovery_required`: an ambiguous or invariant-breaking state currently
  prevents safe automatic action and requires the reported manual correction.
  It is not terminal: after correction, a fresh whole-operation preflight may
  transition to `executing` or `rolling_back` when the live state is exact.

Every participant stores the exact merge message frozen before mutation so a
restart-safe continue cannot reconstruct different commit bytes. For changed
participants the record stores the resulting commit when known. For a
conflicted participant it stores the expected `MERGE_HEAD`, conflict paths,
and the unchanged target HEAD. For a failed participant it stores the typed
error code, message, and optional detail. When root participates, its record
also distinguishes the Git merge result commit from the later root
composition-evidence commit.

The record is written atomically before execution and atomically updated before
and after each participant Git action. Before mutation, an additive
`pending_action` freezes the action kind and exact branch/before/source/message
inputs. The participant outcome clears it atomically. After interruption the
shared classifier recognizes only not-started, exact expected conflict, exact
completed result, or ambiguous state. Exact results may be adopted durably;
ambiguous results remain recovery-required. This applies to start, retry, and
resolution commits rather than special-casing one action path.

Status projects that reconciliation as an optional structured pending-action
summary on the participant row. The summary contains the durable action kind,
the exact reconciliation class, and guidance. Ambiguity additionally produces
member-scoped `pending_action_ambiguous` drift and blocks both automatic
continue and abort until the next exact classification.

Atomic record writes use a temporary file in the destination directory,
flush it, and rename it over the destination. Readers tolerate unknown fields,
and an updater retains unknown fields across read-modify-write so an older
binary does not erase recovery data introduced by a newer one.

The baseline digests are computed over the exact persisted manifest and lock
bytes. The lock digest is the authoritative abort baseline. The manifest
digest detects membership changes while the operation is open. `root_head` is
for diagnostics and checked root-ref updates; when root participates it is
also the root's recorded before commit. After the root is attempted, recovery
uses its recorded participant state rather than incorrectly requiring the live
manifest and lock to retain their baseline digests. Every record also carries
both the record schema version and writer/tool version.

Closed records move to `.gwz/merge/done/`; the versioned root marker remains
the canonical history. GWZ retains the latest 20 ordinary closed records by
default for local crash forensics; configuration is deferred. A record that
owns preservation refs or stash bundles is not aged out and its evidence does
not expire silently. A later
`gwz merge --gc [<merge-id>]` explicitly removes eligible archived records and
their verified `refs/gwz/merge/...` evidence together. Supplying a merge id is
the explicit request to remove that merge's private refs and archive;
unqualified GC only enforces ordinary record retention. Coordinated stash
bundles remain governed by explicit `gwz stash drop`. Status reports retained
preservation evidence until it is cleaned up.

## 9. Merge execution

Selected members execute in manifest order after selection filtering. An
explicitly selected root executes after every member. The root remains last
even when an earlier member reports a content conflict, so the initial command
can report the complete participant conflict set. An unexpected host failure
still stops later participants, including root. Parallel merge may be added
later after recovery semantics are proven.

Per participant, ordinary Git behavior applies:

- source already integrated: record `up_to_date`;
- target strictly behind source: fast-forward and record `fast_forwarded`;
- divergent but cleanly mergeable: create a merge commit and record `merged`;
- content conflict: leave the repository in native Git merge state and record
  `conflicted` plus conflict paths.

A content conflict is an expected participant outcome. Execution continues to
the remaining independent participants so one invocation reports the complete
conflict set.

An unexpected Git, filesystem, identity, or host error is different. Core stops
starting new participant mutations, records the error and all unattempted
participants, and leaves the coordinated merge open for inspection or abort.

Atomic-by-default continues to mean atomic preflight, not transactional
execution. Once execution starts, content conflicts and host failures can
produce mixed participant state. The operation record is how GWZ manages that
fact explicitly.

## 10. Successful finalization

If every participant is `up_to_date`, `fast_forwarded`, or `merged`, the
coordinated merge can finalize immediately:

1. Re-observe every selected participant.
2. Verify each observed branch and HEAD against its recorded result.
3. Atomically set the operation state to `finalizing` before creating or
   publishing any finalization artifact.
4. If root participated, reload and validate the manifest and lock from the
   recorded root merge result. Use them as candidate future workspace metadata,
   but do not change the frozen participant set for this operation. Reconcile
   selected member entries with their verified merge results; reject ambiguous
   workspace identity, member identity, path, or source changes.
5. Build the resulting lock, marker, and boundary update as candidate data
   without replacing the baseline lock.
6. If at least one participant changed, create durable root metadata recording
   the coordinated merge composition and use the scoped primitive to commit only
   the exact candidate GWZ-owned paths in the workspace root. When root
   participated, this composition-evidence commit is created on top of its
   recorded merge result; it is not treated as the root's merge result.
7. Publish the resulting workspace lock and local boundary state, verify them,
   and record the root composition-evidence result in the merge operation.
8. Mark the local merge operation complete and archive its open
   runtime record.

If the merged root metadata is invalid or cannot be reconciled safely,
finalization fails closed and the coordinated operation remains open and
abortable. GWZ never switches to a newly merged manifest midway through member
execution and never silently expands the operation to members introduced by
that manifest.

If scoped root commit or candidate publication fails, the operation remains
open in `finalizing` and does not report completion. Candidate hashes, completed
finalization steps, and any root commit already created are recorded so status
can show the publication stage and a repeated continue can verify and finish
publication idempotently rather than create a second evidence commit.

Finalization extends the commit-marker schema with an additive optional merge
section and a minor schema revision rather than inventing a second evidence
kind. Merge evidence is mandatory for a changed coordinated merge; merge does
not expose `--no-commit-marker`, and `MergeRequest` therefore has no
`commit_marker` field. The durable evidence must associate:

- merge id and operation id;
- source ref;
- selected targets, including explicit `@root` participation;
- per-participant target branch;
- per-participant before, source, and resulting commits;
- the root merge-result commit when root participated;
- root composition commit, located as the commit containing the marker rather
  than encoded as an impossible self-reference inside the marker.

The root commit is created by a scoped commit primitive. Its tree may differ
from its parent only at the exact candidate lock, marker, and versioned boundary
paths under `gwz.conf/`. It must not consume, reset, unstage, or otherwise alter
unrelated root index or worktree content. This remains true when the user
already has unrelated staged files, including a different `gwz.conf/` artifact
outside the candidate path set. The primitive updates the root branch using an
expected-current ref check so a concurrent root commit fails closed.

The default root message is:

```text
gwz merge: <source-ref>

GWZ-Merge-ID: <merge-id>
GWZ-Operation-ID: <operation-id>
```

For a true member merge, start records the exact member commit message before
mutation. Unless `-m` overrides it in a later phase, the default is
`Merge '<source-ref>' into '<target-branch>'` with the same two GWZ trailers.
Immediate clean merges and resolution commits use that recorded message.
Author and committer identity come from the member repository at commit time;
the recorded parent commits, not the person identity, are the merge invariant.

M0 intentionally retains the legacy member message
`Merge <source-ref> into <target-branch>` without quotes or GWZ trailers. The
quoted message and `GWZ-Merge-ID`/`GWZ-Operation-ID` trailers begin in M1,
where the durable operation record can freeze the merge id and exact message
before mutation.

An all-up-to-date operation, including an explicitly selected up-to-date root,
is a successful no-op and does not require a new root commit or marker.

While a merge is open because of conflict or failure, the recorded baseline
lock remains the last accepted complete workspace composition. When root is not
a participant, its persisted lock remains at that baseline. When root is a
participant, its Git merge may change the live `gwz.conf/` files; those bytes
are treated as unaccepted candidate metadata until finalization and do not
redefine the frozen operation. Recovery reads the baseline bytes from the
recorded root before-commit tree and verifies them against the record digests.
The accepted lock advances only when the coordinated merge completes.

## 11. Open merge restrictions

The workspace-wide mutator lock is held only during each command invocation; it
cannot remain held while a user resolves conflicts. Therefore every subsequent
operation must detect the open merge record and revalidate live Git state.

This is enforced once, in the operation runtime after workspace resolution and
before handler dispatch. It is an allowlist keyed by command and subcommand,
not a check copied into each handler. The gate attaches the open merge id and
allowed recovery commands to its typed rejection. A handler may perform a
narrower check after the centralized gate, as `add` does below.

When root participates, its merge may leave the manifest or lock conflicted or
temporarily invalid. Recovery discovery therefore checks `.gwz/merge/` before
normal manifest parsing and resolves participant repositories from the frozen
operation record. `merge --status`, `merge --continue`, `merge --abort`, and
conflict-resolution `add` must remain reachable without accepting the partially
merged root metadata as the current workspace definition.

| Existing command | While merge is open | Reason or constraint |
| --- | --- | --- |
| `add` (stage paths) | conditional | Only conflicted participants are selectable; every path is staged in its selected conflicted repository. Root is selectable only when `@root` is a recorded conflicted participant. Selecting a cleanly merged or unaffected target rejects the whole add. |
| `branch --list` | allow | Read-only. |
| `branch --create/--delete/--merge` | block | Mutates participant refs; the deprecated merge alias may not start a second merge. |
| `capture` | block | Would replace the baseline lock with partial live state. |
| `clone` | not gated | Creates a different workspace; it does not resolve or mutate the open workspace. |
| `commit` | block | Coordinated resolution must use `merge --continue`. |
| `diff` | allow | Read-only. |
| `forall` | block | An arbitrary command cannot be proven read-only. |
| `init` for a new workspace | not gated | No existing workspace operation is resolved. |
| `init <url>` at an existing workspace | allow while plan-only | The current handler returns an accepted plan and performs no mutation. If that behavior becomes mutating, this row changes to block. |
| `init --update` | block | Mutates GWZ-owned root files. |
| `ls` | allow | Read-only. |
| `materialize` | block | Mutates member refs and worktrees. |
| `pull` | block | Mutates member refs/worktrees and may create foreign merge state. |
| `push` | block | Publishing a partial coordinated result is outside the open lifecycle. |
| `repo add/clone/create/detach/attach/sync` | block | Changes membership, materialization, or recorded repository metadata. |
| `snapshot` | block | Records a partial live composition. |
| `stash list` | allow | Read-only, including merge-preservation bundles. |
| `stash push/apply/pop/drop` | block | Changes participant worktrees or preservation evidence outside merge recovery. |
| `status` | allow | Read-only. |
| `tag --list` / remote list | allow | Read-only. |
| `tag --create` / local delete | block | Mutates participant refs. |
| `tag --push/--fetch` / remote delete | block | Publishes or imports refs while the coordinated result is partial. |
| `merge --status` | allow | Read-only recovery inspection. |
| `merge --continue/--abort/--abort --preserve/--gc` | allow | Purpose-built lifecycle operations; each performs its own whole-operation preflight. GC rejects for the currently open merge. |
| `merge <source>` | block | Only one coordinated merge may be open. |

Raw Git use cannot be prevented, but any resulting drift is detected before
continuation or rollback. `gwz add` is intentionally narrower than raw
`git add`: it stages conflict resolutions without offering an easy path to
create index drift in a participant already recorded as merged.

`gwz commit` must not silently finish an open coordinated merge. Users finish it
with `gwz merge --continue`, allowing core to validate and finalize the whole
operation.

`gwz pull --sync merge` remains a separate pull mechanism. Any native merge
state it creates outside a coordinated merge is foreign in-progress state and
causes merge start preflight to reject. Conversely, all pull modes are blocked
while a coordinated merge is open; pull never joins, continues, or aborts the
coordinated operation.

## 12. `gwz merge --status`

Status reads the open record and compares it with every live participant. With
no open operation, initial status returns a successful `idle` response with no
merge id, participants, or drift; it does not fabricate a completed operation.
Archived-record enumeration, id-qualified archived status, and retained
preservation evidence arrive with the preservation/GC increment in M3. Status
does not mutate Git or GWZ artifacts.

Status also reports operation-level drift independently from participant drift.
This allows a caller to see why the next continue or abort would reject even
when every participant row is otherwise clean. Operation drift reasons include
at least:

```text
baseline_lock_changed
baseline_manifest_changed
root_candidate_metadata_invalid
root_candidate_state_changed
record_unreadable
```

`record_unreadable` is emitted as a typed recovery/status result when an open
record path exists but its required schema cannot be decoded; participant
inspection may be unavailable in that case. It is not downgraded to "no open
merge."

It reports, per participant:

- recorded merge outcome;
- expected target branch and HEAD;
- live branch and HEAD;
- native integration state;
- unresolved conflict paths;
- index/worktree cleanliness;
- whether the participant is eligible for continue;
- whether the participant is eligible for abort;
- drift reason when expectations no longer match;
- explicit recovery guidance for blocking drift.

Drift reasons must be structured. They include at least:

```text
branch_changed
head_advanced
head_rewound
target_ref_changed
worktree_modified
index_modified
merge_state_missing
merge_head_changed
new_integration_state
repository_missing
```

It also reports the operation-level lifecycle state and participant counts by
state, the current `finalizing` publication step when applicable, and any
operation-level drift. Preservation refs and coordinated stash bundles retained
from completed or failed preserve attempts remain visible in status until
explicit cleanup.

## 13. `gwz merge --continue`

Continue acquires the workspace mutator lock, loads the single open merge, and
preflights the entire operation again before performing any retry or committing
any resolution. It means both "finish resolved conflicts" and "resume work not
attempted because an unexpected failure stopped the batch."

A failed participant is at a classified retry point only when its target branch
still points at the recorded before commit, its index and worktree are clean,
and no merge or other integration state exists. Any other failed-participant
state is ambiguous and cannot be retried automatically.

Requirements:

- when root has not been attempted, the persisted manifest and lock match the
  recorded baseline digests;
- after root has been attempted, its live Git and metadata state matches its
  recorded participant outcome rather than the baseline digests;
- every recorded conflicted repository still has the expected target branch,
  before HEAD, and `MERGE_HEAD`;
- every conflicted index has no unresolved entries;
- every previously clean result still has the recorded resulting HEAD;
- every `failed` participant is back at a classified retry point with no
  unrecorded mutation from the failed attempt;
- every `unattempted` participant still passes the complete start preflight
  against its recorded before/source/target plan;
- no participant has unrelated index or worktree changes;
- no new integration state exists;
- no participant repository or target ref is missing.

If any participant fails continue preflight, no retry or resolution commit is
started. The response identifies the exact blocking target and drift. A failed
participant with ambiguous partial mutation is not retryable; status directs
the user to coordinated abort or manual recovery. Drift in an `unattempted`
participant explicitly instructs the user to restore that repository to its
recorded before commit and clean state, or abort the coordinated merge.

After successful preflight:

1. Walk all actionable member participants in original manifest execution
   order, followed by root when it was explicitly selected.
2. For a resolved `conflicted` participant, verify that its two merge parents
   are the recorded target-before commit and expected `MERGE_HEAD`, then commit
   it with the exact message recorded at start.
3. For `unattempted`, run the recorded merge plan. For `failed`, retry the same
   plan only when the classified retry point is the unchanged before state.
4. Atomically update the participant result in the operation record after each
   action. A new content conflict is recorded and execution continues; another
   unexpected host failure stops later actions again.
5. Re-observe every selected participant. If any conflict, failure, or
   unattempted participant remains, keep the operation open and return its new
   state.
6. Otherwise finalize the lock and root merge evidence as described above and
   close the local operation record.

The exact recorded parents are the merge invariant. Author and committer
identity use attribution from the request that creates the resolution commit
when present, otherwise they are resolved from the member repository. The same
rule applies to an immediate true merge and a true-merge retry.

Post-merge changes in a participant that had already merged are not
automatically adopted into the coordinated merge. This includes root changes
made after its recorded merge result. The user must preserve or remove that
drift and restore the recorded resulting state before continuing. A future
explicit `--adopt` policy may be designed separately; it must never be
implicit.

## 14. Coordinated abort

`gwz merge --abort` means abort the still-open coordinated attempt, not merely
run `git merge --abort` in the repositories currently showing conflicts.

Consider three members:

| Member | Start outcome | Rollback requirement |
| --- | --- | --- |
| `app` | source already integrated | none; leave it untouched |
| `lib` | clean merge commit | restore its target branch to the recorded before commit |
| `docs` | content conflict | abort its native merge back to the recorded before commit |

If GWZ aborted only `docs`, the workspace would retain the clean merge in `lib`
even though the user asked to abort the coordinated operation. The merge record
therefore treats the clean merge as part of the still-open attempt and includes
it in rollback.

### 14.1 Abort preflight

Abort first computes the complete rollback plan without mutating anything:

- before root is attempted, the persisted manifest and lock must match the
  recorded baseline digests; after root is attempted, its recorded Git state
  is preflighted like every other participant and its baseline metadata is
  restored by rolling back root;
- `up_to_date`: no merge mutation occurred; no rollback action is needed;
- `fast_forwarded`: target branch must still point to the recorded resulting
  commit and its worktree/index must be eligible to reset;
- `merged`: target branch must still point to the recorded merge commit and its
  worktree/index must be eligible to reset;
- `conflicted`: HEAD, target branch, and `MERGE_HEAD` must still match the
  recorded merge, and native abort must be available;
- `failed` or `unattempted`: verify that no merge mutation was recorded;
- already `aborted` or `rolled_back`: treat as an idempotent no-op after
  verifying the recorded target ref is at the before commit; later unrelated
  worktree/index content is reported but does not block rollback that will not
  mutate this participant;
- a recorded `conflicted` participant already at its exact before ref with a
  clean index/worktree and no native integration state: treat as an
  already-restored no-op, including before the operation entered
  `rolling_back`.

If finalization created the recorded root composition-evidence commit but did
not close the operation, abort includes that commit in the rollback plan. Root
must still point at the recorded evidence commit. Abort removes it with a
checked ref update before unwinding the root merge, if root participated, and
then the member merges. If root did not participate, it restores only the
recorded pre-evidence root HEAD. Later root drift blocks abort under the same
rules as drift in any other successfully mutated repository.

Every required rollback must pass preflight before the first rollback occurs.
If any affected participant cannot roll back safely, default abort changes
nothing.

An `up_to_date` participant does not need to be reset because merge did not
mutate it. Post-start work in that participant may be reported as drift but
does not by itself block rollback of other participants, provided GWZ does not
need to mutate it. Its work is left untouched.

### 14.2 Drift in a successfully merged member

Suppose `lib` merged cleanly, and while `docs` remained conflicted the user then
edited or committed additional work in `lib`.

The recorded state might be:

```text
before:    bbb111
merge end: bbb222
live HEAD: bbb333
worktree:  dirty
```

Resetting `lib` to `bbb111` would destroy work not created by the merge
operation. Default `gwz merge --abort` must therefore reject the entire abort
before aborting `docs` or resetting any other repository. It reports that
`lib` has post-merge drift and gives its recorded and live state.

This all-or-nothing abort preflight prevents a particularly bad outcome where
GWZ successfully aborts the conflicted repository and only then discovers that
another member cannot be rolled back.

The user then has three safe choices:

1. Use `gwz merge --abort --preserve`, the sanctioned preservation path, where
   the drift is eligible for automatic preservation.
2. Where preserve reports an unsupported state, follow its explicit manual Git
   recovery instructions, restore `lib` to the recorded merge result, and rerun
   `gwz merge --abort`.
3. Resolve the remaining conflicts, relocate or remove the unrelated drift,
   and complete the merge with `gwz merge --continue`.

GWZ does not offer a default force-abort that silently discards this work.

### 14.3 `--abort --preserve`

Preserve mode is explicit because preservation itself creates Git state. Its
purpose is to save post-merge work in successfully merged participants before
rolling the coordinated operation back. An explicitly selected root, or a root
carrying a recorded but not yet finalized evidence commit, is eligible under
the same rules.

For each eligible drifted merged participant, GWZ records:

- a private backup ref at the current HEAD, for example
  `refs/gwz/merge/<merge-id>/<target-key>/head`, where root uses the stable
  target key `root`;
- a coordinated stash-bundle entry for staged, unstaged, and untracked work
  when the index has no unresolved entries;
- the backup ref target and stash object id in the merge operation record.

The stash is not a parallel hidden mechanism. It uses the existing coordinated
stash-bundle store, is tagged with the merge id, is visible through
`gwz stash list`, and retains the existing object-id-first identity and restore
rules. Preserve includes untracked files by default, equivalent to
`git stash push -u`; ignored files are excluded and there is no preserve
equivalent of `git stash -a`.

Private `refs/gwz/merge/...` refs do not match Git's default push refspecs and
must never be added to a GWZ-generated push refspec. They therefore remain
local preservation evidence unless the user explicitly pushes one with raw
Git.

Preservation has two phases:

1. Preflight that every required backup ref and stash can be created.
2. Create and verify all preservation artifacts before beginning any rollback.

If preservation fails after some artifacts are created, no rollback begins.
The operation remains open and the successfully created artifacts are reported
as recoverable evidence. Rerunning preserve is idempotent by recorded object id.

After all preservation artifacts exist, coordinated rollback proceeds:

- abort native conflicted merges;
- restore fast-forwarded and cleanly merged target refs and worktrees to their
  recorded before commits;
- verify every affected participant is back at its before state;
- close the merge operation as aborted.

Preserved work is not automatically reapplied. It was created against the
post-merge tree and may conflict when applied to the pre-merge tree. GWZ reports
the backup refs and stash bundle so the user can inspect, cherry-pick, or apply
them deliberately.

Initial preserve support is intentionally conservative:

- the participant must still be on the recorded target branch;
- native merge state must not exist in a participant recorded as cleanly merged;
- uncommitted preservation requires an index without unresolved entries;
- branch switches, missing repositories, changed merge parents, or ambiguous
  integration state require manual recovery;
- preserving partial conflict-resolution work from an unmerged index is
  deferred because a normal stash cannot safely represent it.

### 14.4 Abort execution and failures

After successful abort preflight, rollback runs in reverse mutation order.
Reverse order makes the record and diagnostics mirror the unwind of the
original operation. Any recorded root evidence commit is removed first. Because
root executes last, an affected root merge then rolls back before member
rollback proceeds, restoring the baseline manifest and lock.

Each successful participant rollback is written atomically to the operation
record. Unexpected host failure can still interrupt rollback despite preflight.
In that case the record remains open, participants already restored are marked
`rolled_back`, and a later `--abort` resumes the remaining idempotent plan.

The accepted pre-merge workspace lock was never advanced while the operation
was open, so successful abort does not need to synthesize a new composition.
Core verifies the exact lock and manifest bytes against the recorded baseline
after all rollbacks, refreshes only GWZ-owned local workspace boundary state if
necessary, and closes the operation. This verification also proves that a
participating root was restored. A mismatch blocks close and leaves the
recoverable record open rather than claiming a complete abort.

## 15. Conflict resolution example

The following is the M2 lifecycle output, after continue and coordinated abort
exist:

```text
$ gwz merge feature/refactor
app   feature/refactor -> main  up-to-date
lib   feature/refactor -> main  merged       bbb222
docs  feature/refactor -> release  conflicted   guide.md

Workspace merge merge_01... remains open.
Resolve conflicts, then run: gwz merge --continue
Abort all merge changes with: gwz merge --abort
Inspect state with:           gwz merge --status
```

Normal resolution:

```text
$ edit docs/guide.md
$ gwz add docs/guide.md
$ gwz merge --continue
lib   verified   bbb222
docs  continued  ccc333
workspace merge complete
```

Safe abort with post-merge drift:

```text
$ edit lib/src/new_work.rs
$ gwz merge --abort
rejected: lib changed after its merge result
no repositories were rolled back

$ gwz merge --abort --preserve
lib   preserved  refs/gwz/merge/merge_01.../mem_lib/head
lib   stashed    <stash-object-id>
docs  aborted
lib   rolled back to bbb111
workspace merge aborted
```

M0 must instead give honest interim guidance and must not print commands that
do not yet exist:

```text
docs  feature/refactor -> release  conflicted   guide.md

Resolve or abort this member with ordinary Git commands in docs/.
Other members may already have changed; coordinated continue and rollback are
not yet available. The workspace lock reflects clean member outcomes.
```

## 16. Protocol

Add a first-class service method:

```text
method("merge", role="in",
       params=Params(request=Ref.MergeRequest),
       out=Ref.MergeResponse)
```

Proposed messages:

```text
MergeOp = start | resume | abort | status | gc
MergeMode = normal | ff_only | no_ff

MergeRequest {
  meta,
  op,
  source_ref?,
  merge_id?,
  mode?,
  message?,
  preserve?
}

MergeResponse {
  response,
  merge_id?,
  state,
  open,
  participant_counts: MergeParticipantCounts,
  repos: List<MergeRepoSummary>,
  operation_drift: List<MergeOperationDrift>,
  preservation: List<MergePreservation>?
}
```

The protocol uses `resume` for the user-facing `--continue` operation to avoid
language-keyword code generation problems. `state` is an operation-level enum;
`open` is a convenience value derived from it rather than the only lifecycle
signal. `participant_counts` gives totals by participant lifecycle state so
JSON clients do not need to reconstruct the operation summary from repository
rows.

Core owns request validation so every driver and the deprecated branch alias
shares one contract:

| `MergeOp` | `source_ref` | `merge_id` | `mode` | `message` | `preserve` |
| --- | --- | --- | --- | --- | --- |
| `start` | required | reject | accepted when its delivery phase supports it | accepted from M4 | reject |
| `resume` | reject | optional; must equal the single open merge id | reject | reject | reject |
| `abort` | reject | optional; must equal the single open merge id | reject | reject | optional |
| `status` | reject | optional; selects the open or one archived record | reject | reject | reject |
| `gc` | reject | optional; selects one archived record, otherwise enforces default retention | reject | reject | reject |

Without `merge_id`, resume and abort address the single open operation. A
supplied mismatch rejects before any mutation, preventing a script from acting
on a newer merge than the one it observed. Status without an id reports the
open operation plus retained summary information; its id-qualified archived
form is reserved in the protocol even if the initial M1 CLI exposes only open
status. GC rejects an id naming the open operation. Unknown or disallowed field
combinations are typed protocol-validation errors, not silently ignored input.

Each `MergeRepoSummary` reports at least:

- target id (`@root` or member id) and path;
- source ref and resolved source commit;
- target branch;
- before commit;
- resulting or live commit;
- merge lifecycle state;
- conflict paths;
- continue eligibility;
- abort eligibility;
- structured drift;
- backup ref and stash object id when preservation occurred.

Append `merge` to the operation-action enum. Do not renumber existing protocol
enum values. JSON and JSONL responses for `gwz merge` must report action
`merge`, including when invoked through the deprecated `branch --merge` alias.
Both drivers serialize the complete current `MergeResponse` shape from M0:
all participant counts (including reserved `continued`, `aborted`, and
`rolled_back` counts), `operation_drift`, `preservation`, and
`publication_step`. Reserved values are empty or null until their delivery
phase, but their keys are present so M1 can populate them without another
output-shape change. Rust and Python validate this contract against one shared
canonical fixture.
Append `deprecated_operation` to `GwzErrorCode`; direct protocol
`BranchRequest { op: merge }` returns that code and names the `merge` method in
its diagnostic, as defined in section 4.

Merge uses the existing event sequence conventions: each command invocation
emits `OperationStarted`/`OperationFinished`; every actionable target, including
`@root`, emits `MemberStarted` and `MemberFinished` with its structured outcome;
and durable record/evidence writes emit `ArtifactWritten`. Append a generic
`OperationStateChanged` `EventKind` without renumbering existing values so each
transition such as `awaiting_resolution`, `finalizing`, or `rolling_back` is
machine-visible. Event order follows durable order: a transition or participant
outcome is emitted only after the corresponding atomic record update succeeds.
For a participant action, `MemberStarted` precedes the durable action-intent
write, `ArtifactWritten` follows the verified intent, the Git action runs, a
second `ArtifactWritten` follows the verified outcome write, and only then does
`MemberFinished` carry that outcome. `OperationFinished` is emitted on both
success and failure.

## 17. Backend requirements

Existing backend primitives cover much of start execution:

- `head`
- `status`
- `read_ref`
- `merge_upstream`
- `commit_merge_resolution`
- `reset_hard`

Add or formalize narrowly scoped primitives for:

```text
merge_analysis(repo, target_branch, source) -> MergeAnalysis
merge_simulate(repo, target_commit, source_commit) -> Clean | Conflicts(paths)
merge_state(repo) -> NativeMergeState?
abort_merge(repo, expected_before, expected_merge_head)
set_branch_target_checked(repo, branch, expected_current, target)
create_backup_ref(repo, name, target)
stash_for_merge_preservation(repo, merge_id, include_untracked)
commit_gwz_paths_checked(root, expected_head?, candidate_files, message)
```

`merge_simulate` is an explicitly optional/deferred M4 primitive. It uses an
in-memory tree merge (`merge_commits`/`merge_trees` in a libgit2 backend), never
touches the index or worktree, and upgrades dry-run to predict clean versus
conflicted true merges and conflict paths. Reserving it now keeps that upgrade
within the existing plan, response, and recovery-record contracts.

`commit_gwz_paths_checked` constructs a root commit from an isolated/scoped
index and candidate bytes, verifies that only the supplied GWZ-owned paths
differ from the parent, and updates the attached root ref only if it still
equals `expected_head`. `expected_head = none` is supported for an unborn-ref
check and creation of the root's first commit; its candidate tree contains only
the explicitly supplied GWZ-owned paths. An explicitly selected root merge
still requires a born root during start preflight because it needs a target
commit, but a member-only coordinated merge may finalize into an otherwise
unborn root. The primitive preserves unrelated entries in the user's real index
and worktree and returns the new root commit plus candidate hashes for
idempotent publication/recovery.

Every mutating primitive self-verifies its postcondition. Checked ref updates
must use the expected current object id so a concurrent or manual ref movement
cannot be overwritten.

The backend must distinguish untracked files, staged changes, ordinary dirty
tracked files, unresolved index entries, and native merge metadata. A single
`is_dirty` boolean is not enough for safe continue, preservation, and rollback.
It must also expose complete repository operation state independently from
expected merge detail: clean, merge, cherry-pick, revert, rebase, apply/mailbox,
bisect, and any other state exposed by the backend. Status and checked recovery
actions consume the same observation so preflight cannot approve a state that
execution rejects after another participant has changed.

## 18. Atomicity and recovery guarantees

GWZ guarantees:

- all selected participants structurally preflight before merge mutation;
- the merge recovery record exists before mutation;
- expected conflicts are fully reported across the selection;
- continue preflights every participant before resolution commits;
- abort preflights every required rollback before rollback mutation;
- drift is never silently overwritten;
- preservation artifacts are verified before rollback begins;
- rollback progress is durable and resumable after interruption;
- an explicitly selected root executes last and cannot redefine the frozen
  participant plan;
- successful completion writes one coherent workspace composition.

GWZ does not guarantee:

- that a host cannot fail after preflight;
- that raw Git commands cannot change repositories while a merge is open;
- that rollback can proceed after arbitrary manual changes;
- that preserved post-merge work applies cleanly to the pre-merge state;
- that independent remote repositories observe an atomic push.

These limits must be stated in help and machine responses. The design favors
explicit partial state with recoverable evidence over an unsafe illusion of
distributed atomicity.

## 19. Delivery phases

These are internal implementation checkpoints, not independent release
promises. M0, M1, and the continue/abort implementation checkpoint remain
unreleased until the complete member lifecycle, including finalization, passes
the first public release gate defined in `GwzMergePlan.md`.

### Phase M0: first-class start surface

- Add `MergeRequest` and `MergeResponse`.
- Add top-level `gwz merge <source>`.
- Migrate existing branch-merge behavior behind the new handler.
- Keep `gwz branch --merge` as a deprecated alias.
- Retain the `BranchOp.merge` wire value but make direct protocol use return
  `deprecated_operation` naming the `merge` method.
- Add action-correct human, JSON, and JSONL output.
- Preserve current preflight, conflict, and partial lock-advance behavior.
- Reject unrelated histories for both first-class merge and the existing
  `pull --sync merge` path, matching Git porcelain; do not implicitly enable
  `--allow-unrelated-histories`.
- On conflict, print only per-member ordinary Git recovery guidance; do not
  advertise status, continue, or coordinated abort before they exist.
- Reject `--partial` and `--force` with merge-specific typed errors.

### Phase M1: durable open operation

- Write the merge operation record before mutation.
- Update it after every participant result.
- Add the complete operation-state enum, structured operation-level drift, and
  state-transition events; M2 uses `finalizing` for publication recovery.
- Keep the workspace lock at its last complete baseline while merge is open.
- Replace the interim M0 lock behavior before the first public merge release
  and update human/JSON consumer documentation together.
- Add `gwz merge --status`.
- Add the centralized open-operation gate and complete command allowlist.
- Archive closed records with the retention rules in section 8.

### Phase M2: continue and safe abort

- Add `gwz merge --continue`.
- Enable explicit `@root` participation with frozen planning, root-last
  execution, recovery-before-manifest-parse, and metadata reconciliation.
- Resume eligible failed and unattempted members after whole-operation
  preflight.
- Add selection-wide abort preflight.
- Roll back cleanly merged and fast-forwarded members as well as conflicted
  members.
- Add idempotent interrupted-abort recovery.
- Finalize successful merges with root composition evidence.

### Phase M3: drift preservation

- Add private backup refs.
- Integrate coordinated preservation stashes.
- Add `gwz merge --abort --preserve`.
- Add `gwz merge --gc [<merge-id>]` for archived records and preservation refs.
- Report manual-recovery cases without destructive fallback.

### Phase M4: controlled expansion

- Add optional in-memory `merge_simulate` and conflict-predicting dry-run.
- Add `--ff-only`, then `--no-ff`.
- Add custom message support.
- Add exact per-member snapshot sources.
- Consider `--into` only after switch-plus-merge rollback is designed.
- Consider explicit partial/skip policy only with complete machine reporting.

## 20. Test matrix

The implementation is not complete until tests cover at least:

### Parsing and protocol

- top-level start, continue, abort, status, dry-run, preserve, and GC forms;
- mutual exclusion of source, continue, abort, status, and GC command forms;
- start rejects `merge_id` and `preserve`; resume rejects `source_ref`, `mode`,
  `message`, and `preserve`; preserve outside abort rejects;
- resume and abort with a wrong merge id reject before mutation;
- status accepts an optional open or archived merge id, and GC rejects the open
  merge id;
- `--partial` and `--force` produce typed merge-policy rejections for both the
  first-class command and deprecated branch alias;
- protocol corpus regeneration and Rust/Python byte parity;
- merge action in response metadata and event streams;
- deprecated branch alias maps to a merge request and response;
- protocol-level `BranchRequest { op: merge }` retains its wire value but
  returns `deprecated_operation` naming the `merge` replacement method;

### Start preflight

- dirty, detached, unborn, missing, unmaterialized, missing-source, and
  in-progress-integration members reject the whole operation;
- a failure in the last selected member leaves every earlier member unchanged;
- source refs resolve independently per selected repository;
- default selection excludes root and an explicit root request returns the
  phased typed result until M2;
- dry-run leaves Git, lock, marker, and runtime state unchanged;
- `--ff-only` rejects selection-wide before mutation.

### Start execution

- all-up-to-date no-op;
- all-fast-forward;
- clean true merges;
- mixed up-to-date, fast-forward, and true merge;
- one conflict followed by later clean and conflicting members;
- unexpected failure after an earlier mutation;
- operation record exists before the first backend mutation;
- crash between record creation and first mutation makes abort a clean,
  idempotent no-op;
- operation record survives process restart;
- operation record tolerates unknown fields and retains them through an
  update round-trip;
- optional simulated dry-run reports clean/conflict prediction without index
  or worktree mutation.
- dry-run is explicitly advisory and real start repeats preflight under the
  mutator lock.

### Explicit root participation

- `--target @root` alone merges only the root;
- explicit root plus members freezes the participant set from the pre-merge
  manifest and lock;
- root source resolution and ordinary dirty/detached/unborn/integration-state
  preflight match member safety rules;
- root executes after all selected members, including after expected member
  conflicts, but remains unattempted after an earlier unexpected host failure;
- a root conflict in manifest or lock files remains recoverable even when
  normal workspace metadata parsing fails;
- `gwz add` accepts root paths only while root is a recorded conflicted
  participant;
- a merged root manifest is reloaded for finalization but never changes the
  frozen participant set;
- invalid or ambiguous merged root metadata fails finalization closed and
  leaves the operation abortable;
- selected member results are reconciled into the post-root candidate lock;
- the root composition-evidence commit is created on top of the recorded root
  merge-result commit;
- root post-merge drift blocks ordinary abort and is eligible for explicit
  preserve under the same rules as member drift;
- an incomplete finalization's recorded root evidence commit is removed before
  the root merge and member results are unwound;
- reverse rollback restores root first and verifies the exact baseline
  manifest and lock before closing;
- an up-to-date selected root is left untouched on abort and an all-up-to-date
  root-only merge creates no evidence commit.

### Continue

- all conflicts resolved and staged;
- one unresolved member rejects before any resolution commit;
- changed `MERGE_HEAD`, branch, or HEAD rejects;
- drift in an already merged member rejects;
- eligible failed and unattempted members resume after an unexpected
  mid-batch host failure;
- ambiguous mutation left by a failed member rejects retry before any action;
- successful continue refreshes lock and writes root evidence once;
- finalization with unrelated staged and dirty root content commits only
  GWZ-owned paths and leaves the unrelated index/worktree state intact;
- a concurrent root ref update makes the scoped root commit fail closed;
- crash after the scoped root commit but before lock publication reports
  `finalizing`, identifies the completed publication step, and resumes without
  creating a second evidence commit;
- drift in an unattempted participant names the target and instructs the user
  to restore its recorded before/clean state or abort;
- repeated continue after completion is a clear no-op or typed closed-operation
  result.

### Abort

- conflict-only abort;
- clean-merge plus conflict rolls both members back;
- fast-forward plus conflict restores the fast-forwarded ref;
- up-to-date plus conflict leaves the up-to-date member untouched;
- three-member up-to-date/merged/conflicted scenario restores the complete
  pre-merge composition;
- dirty worktree in the successfully merged member rejects the entire abort
  before any rollback;
- extra commit in the successfully merged member rejects the entire abort;
- branch switch in the successfully merged member rejects the entire abort;
- changed or missing native merge state rejects before rollback;
- unexpected manifest or lock drift before root is attempted, or a recorded
  root-state mismatch afterward, rejects before any participant rollback;
- interrupted rollback is resumable and already-restored members are
  idempotent;
- the recorded baseline remains authoritative throughout the open operation,
  including while a participating root contains candidate metadata.

### Preserve abort

- uncommitted post-merge work is saved by verified stash object id;
- committed post-merge work is saved by verified private backup ref;
- combined committed and uncommitted drift preserves both;
- preservation failure performs no rollback;
- created preservation artifacts remain reported and reusable after failure;
- preserved work is never automatically reapplied;
- unmerged-index preservation rejects with manual-recovery guidance;
- backup-ref and stash-name collisions are idempotent or fail closed.
- preserve includes untracked files, excludes ignored files, and records the
  result as an existing coordinated stash bundle;
- normal `gwz push` never includes `refs/gwz/*` in generated refspecs.

### Open-operation gate

- one allow/block assertion for every command and subcommand row in section 11;
- `add` accepts paths only in recorded conflicted members;
- `add` accepts a conflicted root participant and rejects a merged or
  unaffected root target;
- `stash list` remains available while stash mutations reject;
- tag push, fetch, and remote delete are blocked while local and remote listing
  remain available;
- plan-only `init <url>` at an existing workspace remains allowed and is
  reclassified as blocked if it ever becomes mutating;
- every rejection reports the open merge id and recovery commands;
- `pull --sync merge` is blocked and cannot join the coordinated lifecycle.

### Output and documentation

- human output shows source-to-target mapping and every participant outcome;
- conflicts show member-relative paths;
- status shows recorded versus live state and structured drift;
- manifest or lock edits while an operation is open appear as structured
  operation-level baseline drift even when participant rows are clean;
- an unreadable open record is reported as `record_unreadable`, not as no open
  merge;
- JSON/JSONL include merge id, before/source/result commits, eligibility, and
  preservation evidence;
- JSON/JSONL include operation state and participant counts;
- event streams include durable participant outcomes and operation-state
  transitions in record-update order;
- M0 conflict output does not advertise unavailable continue/abort commands;
- CLI reference, command documentation, protocol catalog, event catalog, and
  Python help remain generated/current.

## 21. Decisions captured by this proposal

| ID | Decision | Recommendation |
| --- | --- | --- |
| M1 | Primary surface | `gwz merge` is first-class; `branch --merge` is deprecated compatibility syntax. |
| M2 | Default targets | Active selected members; root is excluded by default but supported when explicitly selected. |
| M3 | Source resolution | Resolve the same source text independently in every selected repository; missing source rejects the batch. |
| M4 | Target branch | Merge into each selected repository's current attached branch and display mixed targets explicitly. |
| M5 | Conflict execution | Continue other independent participants and report the complete conflict set. |
| M6 | Open-state evidence | Write a local durable merge record before mutation and update it after each outcome. |
| M7 | Lock semantics | Keep the recorded baseline as the last accepted composition until merge finalization. |
| M8 | Continue | Preflight the complete open operation; resume eligible failed/unattempted work and do not absorb unrelated drift. |
| M9 | Abort scope | Roll back clean/fast-forward results as well as native conflicts from the open coordinated attempt. |
| M10 | Abort drift | Any affected participant that cannot safely roll back rejects the whole abort before mutation. |
| M11 | Post-merge work | Never discard silently; require manual preservation or explicit `--abort --preserve`. |
| M12 | Preservation | Reuse coordinated stash bundles plus verified private refs before rollback and never auto-reapply them. |
| M13 | Final evidence | Refresh the lock and path-scope the mandatory root composition-evidence commit only after complete success. |
| M14 | Initial strategy | Git's normal merge behavior; advanced strategies are deferred. |
| M15 | Open-operation gate | Enforce one runtime allowlist before handler dispatch, with a conflicted-participant-only exception for `add`. |
| M16 | Recovery baseline | Record exact lock/manifest digests, root HEAD, and schema/tool versions before mutation. |
| M17 | Global policies | Reject `--partial` and `--force`; neither weakens merge lifecycle safety. |
| M18 | Record retention | Archive the latest 20 ordinary records by default; retain preservation evidence until explicit GC/drop. |
| M19 | Dry-run evolution | Reserve optional in-memory merge simulation for later conflict prediction. |
| M20 | Root ordering | Freeze the pre-merge participant plan, execute an explicit root last, and never let merged root metadata redefine the in-flight operation. |
| M21 | Root finalization | Reload and reconcile merged root metadata, then create composition evidence on top of the root merge result. |
| M22 | Request validation | Core owns a per-operation accepted-field matrix, including exact open-id checks for resume and abort. |
| M23 | Operation drift | Status reports baseline and record failures independently from participant drift. |
| M24 | Finalization lifecycle | Enter durable `finalizing` before creating or publishing final artifacts and expose the publication step in status. |
| M25 | Lifecycle cleanup | M3 owns `merge --gc`; ordinary retention is a configurable-later default of 20 records. |
| M26 | Lifecycle events | Emit participant outcomes and durable operation-state transitions using append-only event kinds and record-update ordering. |
| M27 | Branch wire compatibility | Retain the `BranchOp.merge` enum value, remove its behavior, and return `deprecated_operation`; only driver syntax lowers to `MergeRequest`. |

## 22. Review questions resolved

The design adopts the following answers:

1. Archive ordinary completed records locally with a default last-20 retention
   limit; the committed root marker is canonical history. Records owning
   preservation evidence are retained until explicit cleanup.
2. Evolve the existing marker with an additive optional merge section and a
   minor schema revision rather than introducing a second marker kind.
3. Report but do not block on post-start work in an `up_to_date` member because
   GWZ has no rollback action for that member.
4. Preserve untracked files by default and exclude ignored files.
5. Retain private preservation refs until `gwz merge --gc [<merge-id>]` removes
   them with their archived recovery record. Never expire them silently.
6. Do not add `--adopt` without a separate demand-driven design. If introduced
   later, adopted commits must be distinct evidence and must pass an explicit
   whole-operation preflight.
