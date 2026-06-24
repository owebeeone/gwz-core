# GWZ Branch Management Plan

Status: **proposed** (2026-06-24). Owner: Gianni.

`GWZDesign.md` stays authoritative for the overall workspace model. This plan is
the active design checkpoint for workspace-wide branch management: creating,
switching, listing, deleting, and merging Git branches across the repositories in
a GWZ workspace.

## 1. Goal & shape

Make GWZ branch operations a faithful fan-out of ordinary Git branch workflows:

- **`gwz branch ...`** manages Git branches across selected member repos, with a
  path to include the workspace root repo explicitly.
- **`gwz materialize --switch <branch>`** switches selected members to an existing
  local branch and rewrites the lock from the observed post-switch state.
- **`gwz snapshot <id> --branch`** records the HEAD of the current branch, or a
  named branch, as a normal GWZ snapshot.
- **`gwz branch --merge <source>`** merges a branch into the current target branch
  across selected members, reporting conflicts instead of hiding them.

The split is intentional:

- `branch` is Git-native ref management.
- `materialize --switch` is the checkout/switch equivalent, because it changes
  the materialized workspace state and therefore belongs with other materialize
  targets.
- `snapshot --branch` produces a normal snapshot artifact. It does not create a
  new kind of branch artifact.

## 2. Repo set and root handling

For member operations, "all repos" means every active, materialized member repo
selected by the normal GWZ selection rules.

The workspace root is also a Git repo, but switching it is special: changing the
root branch can replace `gwz.conf/gwz.yml` and `gwz.conf/gwz.lock.yml` while the
operation is in flight. The safe default is:

- member repos are included by default;
- the root repo is included only when the CLI asks for it, e.g. `--root` or
  `--all-repos`;
- root switching is implemented after the member path is solid, with an explicit
  reload rule.

Recommended root switch rule:

1. Preflight the current workspace root and selected member repos.
2. Switch the root repo first.
3. Reload the manifest and lock from the new root branch.
4. Resolve the selected members against the reloaded workspace.
5. Switch/materialize those members.

This avoids using an old manifest to mutate a new workspace branch. If this is
too much for v0, root participation should be accepted for list/create/delete and
deferred for switch/merge.

## 3. Decisions to confirm

| ID | Decision | Recommendation |
| --- | --- | --- |
| B1 | Primary switch surface | Add `gwz materialize --switch <branch>` as the branch checkout surface. Protocol target kind is `branch`; `--switch` is CLI wording. |
| B2 | Branch create behavior | `gwz branch --create <name>` creates the branch in each selected repo at current HEAD by default and does not switch. Add `--switch` for create-and-switch. |
| B3 | Snapshot branch source | `gwz snapshot <id> --branch` captures the current attached branch HEAD. `gwz snapshot <id> --branch <name>` captures `refs/heads/<name>` without switching. |
| B4 | Mixed current branches for `snapshot --branch` | Reject by default when selected members are on different branch names. Allow later via `--allow-mixed-branches` if needed. |
| B5 | Missing branch on a selected member | Reject the whole batch in preflight for switch, snapshot, delete, and merge. Create can report existing-at-same-commit as noop. |
| B6 | Dirty worktrees before switch or merge | Require clean selected repos by default. Add an explicit policy escape hatch later if drivers need Git's permissive checkout behavior. |
| B7 | Merge conflicts | Conflicts are an expected result, not a hard error. Leave conflicted repos in Git's normal in-progress merge state and report per-member `conflicted`. |
| B8 | Root repo participation | Keep root opt-in until root switch reload semantics are implemented and tested. |

## 4. CLI surface

Recommended user-facing shape:

```text
gwz branch
gwz branch --list
gwz branch --create <name> [--from <ref>] [--switch]
gwz branch --delete <name> [--force]
gwz branch --merge <source> [--into <target>]

gwz materialize --switch <branch>
gwz snapshot <id> --branch
gwz snapshot <id> --branch <branch>
```

Details:

- `gwz branch` lists the current branch label per selected member, plus an
  aggregate view similar to `gwz status --branches`.
- `gwz branch --list` lists local branches present in each selected repo,
  aggregated by branch name.
- `gwz branch --create <name>` creates `refs/heads/<name>` in each selected repo.
  If `--from` is omitted, the start point is current `HEAD` in each repo. If
  `--from <ref>` is supplied, each repo resolves that ref independently.
- `gwz branch --create <name> --switch` creates the branch, then switches to it
  in every selected repo. The operation preflights all repos before mutating.
- `gwz materialize --switch <branch>` switches to an existing local branch in
  each selected repo. It does not create missing branches and does not fetch.
- `gwz snapshot <id> --branch` records the selected members' current branch HEADs
  as snapshot `<id>`. It requires attached HEADs and, by default, one shared branch
  name across the selection.
- `gwz snapshot <id> --branch <branch>` records the named branch head in each
  selected repo without checking it out.
- `gwz branch --merge <source>` merges `<source>` into the currently checked-out
  branch in each selected repo.
- `gwz branch --merge <source> --into <target>` switches to `<target>` first,
  then merges `<source>`. This is convenient but should be Phase 3, after plain
  switch and plain merge are reliable.

Remote branch management is intentionally later. The first branch tool should be
local-only. Fetch/push behavior already belongs to `pull`, `push`, and the tag
remote plan; branch tracking can build on the same remote primitives later.

## 5. Materialize branch target

Add a new materialization target:

```text
MaterializeTargetKind.branch
MaterializeTarget { kind: branch, name: "<branch>" }
```

CLI mapping:

```text
gwz materialize --switch feature/x
```

Semantics:

1. Resolve the workspace, manifest, lock, and selection.
2. For every selected materialized member, resolve `refs/heads/<branch>`.
3. Build an in-memory target state:
   - `commit` = the branch ref target;
   - `branch` = `<branch>`;
   - `detached` = `false`;
   - path/source fields copied from the manifest and current lock.
4. Run the same preflight style used by existing materialize:
   - branch exists in every selected member;
   - selected repos are clean by default;
   - missing or unmaterialized selected members reject the batch.
5. Switch each repo to the branch.
6. Re-observe each repo and rewrite the lock from observed state.
7. Refresh the workspace boundary from the rewritten lock.

This keeps `--switch` aligned with materialize's core contract: move the
materialized workspace to a named target and then record the observed result.

Important non-goals:

- `--switch` does not create branches. Use `gwz branch --create`.
- `--switch` does not fetch remote branch refs. Use `gwz pull` or a later remote
  branch command.
- `--switch` does not detach. Exact commits remain `gwz materialize --commit`.

## 6. Snapshot branch source

Today `gwz snapshot <id>` records the observed selected member states. Add a
source option so a snapshot can be sourced from branch heads instead:

```text
gwz snapshot <id> --branch
gwz snapshot <id> --branch <branch>
```

Protocol recommendation:

```text
SnapshotSourceKind = observed_head | current_branch_head | named_branch_head

SnapshotSource {
  kind: SnapshotSourceKind,
  branch: STR?
}

SnapshotRequest {
  meta,
  snapshot_id,
  source: SnapshotSource?
}
```

Default `source = None` preserves existing behavior.

`current_branch_head` behavior:

- require each selected materialized member to have an attached branch;
- require a shared branch label across the selection by default;
- resolve each member's `refs/heads/<current-branch>`;
- write a normal snapshot artifact with those commits and branch labels.

`named_branch_head` behavior:

- resolve `refs/heads/<branch>` in each selected member;
- do not switch worktrees;
- write a normal snapshot artifact with `branch = <branch>` and `commit` equal
  to the branch ref target.

A branch-sourced snapshot is a point-in-time composition. Restoring it later with
`gwz materialize --snapshot <id>` restores the captured commits; it does not keep
following the branch as the branch moves.

## 7. Branch command protocol

Add a Git-native branch request/response pair. Keep it separate from
`MaterializeRequest` because create/delete/list/merge are not all materialization
targets.

```text
BranchOp = list | create | delete | merge

BranchRequest {
  meta,
  op: BranchOp,
  name: STR?,          # create/delete branch name
  source: STR?,        # merge source or create start ref
  target: STR?,        # merge target branch, optional
  force: BOOL?,
  switch_after_create: BOOL?,
  include_root: BOOL?
}

BranchResponse {
  response: ResponseEnvelope,
  branches: List<BranchRepoSummary>?
}
```

Per-repo branch summary should report:

- repo id (`root` or member id);
- repo path;
- current branch label;
- local branch names for list;
- action result (`created`, `exists`, `switched`, `deleted`, `merged`, `noop`,
  `conflicted`, `skipped`);
- resulting head commit when known;
- conflict paths for merge conflicts.

The response should reuse the standard envelope and `MemberResponse` where a
member is the target. Root results need a small root-specific result structure or
an explicit synthetic repo id; do not force the root through member selection.

## 8. Backend primitives

Existing primitives already cover part of the plan:

- `head`
- `status`
- `read_ref`
- `checkout_branch`
- `merge_upstream`
- `rebase_onto`
- `reset_hard`

Add narrowly-scoped branch primitives behind `GitBackend`:

```rust
fn branch_list(&self, path: &Path) -> ModelResult<Vec<GitBranch>>;
fn branch_create(&self, path: &Path, name: &str, start_ref: &str) -> ModelResult<GitBranchResult>;
fn branch_delete(&self, path: &Path, name: &str, force: bool) -> ModelResult<()>;
fn switch_branch(&self, path: &Path, name: &str) -> ModelResult<GitUpdateResult>;
```

Contracts:

- `branch_create` self-verifies that `refs/heads/<name>` exists at the resolved
  start commit. Existing branch at the same commit is `noop`; existing branch at
  another commit is rejected unless a future destructive policy explicitly allows
  moving it.
- `switch_branch` checks out an existing branch without moving it and
  self-verifies HEAD is attached to that branch. It should use safe checkout
  behavior and report dirty/conflicting worktrees clearly.
- `branch_delete` refuses to delete the current branch unless `force` and the
  backend can prove the behavior matches porcelain Git.
- Merge should continue using `merge_upstream` initially. A branch handler can
  call `switch_branch(target)` first when `--into` is provided, then
  `merge_upstream(path, target, source_ref)`.

## 9. Operation safety model

Preflight is mandatory for multi-repo branch mutation.

For create:

- all selected repos must be materialized;
- all start refs must resolve;
- no selected repo may already have the branch at a different commit unless a
  destructive policy is explicitly set;
- if any repo fails preflight, create no branches.

For switch:

- all selected repos must be materialized;
- the branch must exist in every selected repo;
- selected repos must be clean by default;
- if any repo fails preflight, switch no repos.

For delete:

- the branch must exist in every selected repo;
- the branch must not be checked out in any selected repo;
- deletion is all-or-nothing by default.

For merge:

- all selected repos must be materialized and clean;
- source branch/ref must resolve in every selected repo;
- target branch must be attached currently, unless `--into` supplies an explicit
  target;
- a clean merge advances the target branch and rewrites the lock from observed
  state;
- a conflicted merge leaves Git's normal in-progress state in that repo and
  reports `MemberStatus::conflicted`;
- conflicted members are not rolled back. The developer resolves with normal Git
  or future GWZ conflict helpers, then runs `gwz capture` or `gwz commit`.

Merge is the one operation where post-preflight conflicts can still create a
mixed outcome. That is normal Git behavior and should be represented honestly in
the response rather than hidden behind a failed batch abstraction.

## 10. Phased plan

### Phase 1 - Branch target and snapshot source

- **1.1 Protocol** - Add `MaterializeTargetKind.branch` and
  `SnapshotSource{kind, branch?}`. Regenerate generated Rust and corpus.
- **1.2 Backend switch primitive** - Add `switch_branch` and branch ref helpers
  with self-verifying tests.
- **1.3 `materialize --switch` handler** - Build branch target states from
  `refs/heads/<branch>`, preflight selected members, switch, observe, rewrite
  lock, sync workspace boundary.
- **1.4 `snapshot --branch` handler** - Add current-branch and named-branch
  snapshot sources. Tests for mixed branches, missing refs, detached HEAD, and
  no-worktree-switch behavior.
- **1.5 CLI** - Wire `gwz materialize --switch <branch>` and
  `gwz snapshot <id> --branch [branch]`.

### Phase 2 - Branch create/list/delete

- **2.1 Protocol** - Add `BranchRequest`, `BranchResponse`, `BranchOp`, and branch
  summary structs.
- **2.2 Backend branch primitives** - Add `branch_list`, `branch_create`, and
  `branch_delete`.
- **2.3 Handler** - Implement list/create/delete across selected materialized
  members. Keep root opt-in if implemented.
- **2.4 CLI** - Wire `gwz branch`, `gwz branch --list`,
  `gwz branch --create <name> [--from <ref>] [--switch]`, and
  `gwz branch --delete <name> [--force]`.

### Phase 3 - Merge branches

- **3.1 Handler** - Implement `gwz branch --merge <source>` into the current
  attached branch across selected members.
- **3.2 Optional target** - Add `--into <target>` by switching to target first,
  then merging.
- **3.3 Conflict reporting** - Return per-member conflict paths and aggregate
  `conflicted` status while leaving repos in Git's normal merge state.
- **3.4 Lock behavior** - Re-observe and rewrite the lock for cleanly merged
  members; leave conflicted members to be captured after resolution.

### Phase 4 - Root and remote branch polish

- **4.1 Root repo support** - Add `--root` / `--all-repos` and the root switch
  reload rule from section 2.
- **4.2 Remote tracking** - Add optional creation from remote branches, e.g.
  `gwz branch --create feature --track origin/feature`, reusing existing remote
  fetch/list primitives.
- **4.3 Docs** - Update `GWZDesign.md`, `README.md`, and CLI help. Full
  `cargo test`, clippy, and corpus checks green.

## 11. Anchors in existing code

- `src/workspace_ops/handle_materialize.rs` - existing materialize target
  handling, lock rewrite, observed-state discipline, and workspace boundary sync.
- `src/git/gitbackend.rs` - existing checkout, merge, rebase, reset, read-ref,
  head, and status primitives.
- `src/status/status_member.rs` and `src/status/branch_groups_and_differences.rs`
  - branch labels, grouping, and branch difference projection for status/list.
- `protocol/gwz.taut.py` - `MaterializeTargetKind`, `MaterializeTarget`,
  `SnapshotRequest`, and new branch request types.
- `GWZTagPlan.md` - similar split between GWZ workspace artifacts and Git-native
  ref management.

## 12. Out of scope for the first implementation

- Remote branch push/delete/fetch workflows.
- Worktree-per-branch management.
- Automatic branch creation during `materialize --switch`.
- Branch rename.
- Conflict resolution helpers beyond clear reporting.
- Destructive branch reset/move across the workspace without an explicit policy
  gate.
