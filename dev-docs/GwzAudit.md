# GWZ Mutation Safety Audit Prompt

Status: draft

Use this prompt to start focused audit agents for the mutation-order safety
review of the local GWZ repositories:

- `/Users/owebeeone/limbo/glial-dev/gwz-core`
- `/Users/owebeeone/limbo/glial-dev/gwz-cli`

Do not audit unrelated repositories unless explicitly asked.

## Agent Prompt

You are auditing GWZ for mutation-order, atomicity, and recovery-safety bugs.
This is a diagnosis task, not an implementation task.

The triggering incident was a `gwz pull` fast-forward that advanced a member
branch to a commit containing new `dev-docs/*` files while the index/worktree
did not materialize those files. Git then reported the new files as staged
deletions. The remote did not delete the files. The suspected root class is:

```text
durable metadata/ref says "new state" before worktree/index/artifacts actually
reach that state
```

The immediate fast-forward bug was in `gwz-core/src/git/mod.rs`: a branch ref
was moved before the target tree was checked out. Treat that as one confirmed
example of the risk class, not proof that all other paths are safe.

## Hard Rules

- Do not change code, tests, docs, Git state, or workspace files during the
  audit unless the user explicitly asks for fixes.
- Do not run destructive Git commands.
- Do not clean, restore, reset, checkout, stash, or otherwise normalize evidence
  states.
- Do not run `gwz pull`, `gwz materialize`, or other mutating GWZ commands in
  user workspaces as part of diagnosis.
- You may run read-only commands such as `git status`, `git log`, `git diff`,
  `git show`, `rg`, `sed`, `cargo test --no-run`, and targeted tests only if
  they do not mutate the working tree.
- If you need a reproducer, build it in a temporary directory under `/tmp` or
  another disposable path.
- Preserve user changes. If a repo is dirty, identify the dirty files and work
  around them.

## Audit Goal

Find paths where GWZ can leave Git refs, workspace artifacts, lock files,
manifest files, snapshots, tags, operation state, or member worktrees in a state
that falsely claims an operation succeeded, partially succeeded without clear
metadata, or makes user work look like user-authored changes.

Prioritize bugs that can:

- make clean remote changes appear as local deletions or modifications
- move refs before checkout/worktree updates are proven
- write `gwz.conf/gwz.lock.yml` before all selected members are verified
- update manifest/lock metadata before filesystem or Git mutations succeed
- mutate some members after another selected member should have blocked the
  operation
- leave partial mutation without explicit per-member outcome and recovery data
- silently accept dirty post-operation status as normal

## Repositories And Boundaries

`gwz-core` owns workspace semantics. Audit core first.

`gwz-cli` should be thin: argument parsing, request construction, output
rendering. Audit it only for cases where CLI parsing or output hides partial
failure, maps policies incorrectly, or directly mutates Git/GWZ artifacts.

Repo-specific rules:

- `gwz-core/AGENTS.md`: work TDD-first for fixes, keep core independent of CLI,
  protocol payloads are taut-defined.
- `gwz-cli/AGENTS.md`: do not call Git directly from CLI, do not read/write GWZ
  artifacts directly from CLI.

## Required First Step

Record the starting state in your notes:

```sh
git -C /Users/owebeeone/limbo/glial-dev/gwz-core status --short --branch
git -C /Users/owebeeone/limbo/glial-dev/gwz-cli status --short --branch
git -C /Users/owebeeone/limbo/glial-dev/gwz-core log --oneline --decorate --max-count=5
git -C /Users/owebeeone/limbo/glial-dev/gwz-cli log --oneline --decorate --max-count=5
```

Do not alter the state after recording it.

## Primary Search Targets

Start with these read-only searches:

```sh
rg -n "set_target|set_head|set_head_detached|checkout_head|checkout_tree|reset|merge|fast_forward" \
  /Users/owebeeone/limbo/glial-dev/gwz-core/src \
  /Users/owebeeone/limbo/glial-dev/gwz-cli/src

rg -n "write_lock|write_manifest|write_snapshot|write_tag|artifact::write|atomic_write" \
  /Users/owebeeone/limbo/glial-dev/gwz-core/src \
  /Users/owebeeone/limbo/glial-dev/gwz-cli/src

rg -n "clone_repo|fetch\\(|push\\(|checkout_commit|materialize|pull_head|pull_snapshot|handle_push|handle_clone" \
  /Users/owebeeone/limbo/glial-dev/gwz-core/src \
  /Users/owebeeone/limbo/glial-dev/gwz-cli/src

rg -n "dirty|is_dirty|lock_match|preflight|partial|mutation|MemberLock|requires_mutation" \
  /Users/owebeeone/limbo/glial-dev/gwz-core/src \
  /Users/owebeeone/limbo/glial-dev/gwz-cli/src
```

Then inspect:

- `gwz-core/src/git/mod.rs`
- `gwz-core/src/workspace_ops/mod.rs`
- `gwz-core/src/artifact/mod.rs`
- `gwz-core/src/status/mod.rs`
- `gwz-core/src/operation/mod.rs`
- `gwz-cli/src/main.rs`
- relevant tests under both repos

## Mutation-Order Checklist

For each mutating operation, answer these questions:

1. What state does the operation mutate?
2. What is preflighted before mutation starts?
3. Does any durable ref/artifact/lock move before the corresponding worktree,
   index, or member state is proven updated?
4. If an intermediate step fails, what state is left behind?
5. Is partial success represented explicitly in the response?
6. Is partial success represented in persistent metadata, if needed for
   recovery?
7. Does the operation verify the final member state before writing
   `gwz.lock.yml`?
8. Can new files, deleted files, renames, nested directories, executable-bit
   changes, symlinks, or subdirectories expose a gap that an existing-file test
   would miss?
9. Does the operation accidentally make user-visible Git status dirty after an
   operation that should be clean?
10. Are all selected members protected from mutation when one selected member
    fails preflight?

## Operation-Specific Audit Areas

### Git Backend

Audit `GitBackend` methods and their tests:

- `clone_repo`
- `fetch`
- `fast_forward`
- `checkout_commit`
- `status`
- `push`

Look for ref-before-worktree bugs, checkout failure behavior, non-empty target
handling, dirty target handling, post-operation status verification, and missing
tests for added/deleted/renamed nested files.

Required test-gap classification:

- existing file modification
- added file
- added nested file
- deleted file
- renamed file
- checkout failure after preflight
- dirty target rejection
- post-operation clean status

### Workspace Operations

Audit `workspace_ops` handlers:

- `handle_create_workspace`
- `handle_add_existing_repo`
- `handle_create_repo`
- `handle_init_from_sources`
- `handle_clone_workspace`
- `handle_materialize`
- `handle_pull_head`
- `handle_pull_snapshot`
- `handle_snapshot`
- `handle_tag`
- `handle_push`

Focus on whether manifest/lock writes happen only after the relevant Git and
filesystem mutation has succeeded, and whether responses/locks reflect reality
after the operation, not intended state before verification.

### Artifact Writes

Audit atomic write semantics and call sites:

- manifest writes
- lock writes
- snapshot writes
- tag writes
- `.gitignore` updates

Confirm writes are atomic as files, but also review whether the higher-level
operation is semantically atomic across multiple files and member repos.

### Multi-Member Operations

Audit all selection-wide operations for:

- preflight-before-any-member-mutation
- per-member mutation ordering
- rollback or recovery behavior when member N fails after member 1..N-1 changed
- lock/artifact writes after partial member changes
- event streams and final responses accurately reporting partial outcomes

### CLI Layer

Audit `gwz-cli` for:

- direct Git calls, which should not exist
- direct artifact reads/writes, which should not exist
- flags that map to unsafe policies by default
- human output that hides dirty/partial/failed states
- JSON/JSONL output that omits per-member failure details

## Findings Format

Report findings first, ordered by severity. Use this format:

```text
[P0/P1/P2] Short title
File: /absolute/path/to/file.rs:line
Operation: e.g. pull --head / materialize --lock / git fast_forward
Risk: concrete failure mode
Evidence: exact code path and current behavior
Reproducer: minimal test or command, preferably in a temp dir
Expected invariant: the safety rule that should hold
Suggested fix: high-level fix only unless asked to implement
Suggested test: focused regression test
```

Severity guide:

- `P0`: can silently corrupt or misrepresent user work, move refs incorrectly,
  or make clean remote state appear as user changes.
- `P1`: partial mutation or stale metadata that can mislead users or block
  recovery but has visible errors.
- `P2`: missing hardening, missing test coverage, confusing output, or policy
  ambiguity.

After findings, include:

- Open questions.
- Test gaps.
- Areas reviewed with no issue found.
- Commands run.
- Repos and starting commit ids.

## Expected Invariants

Use these invariants while auditing:

- A ref must not be advanced before the target tree/index/worktree update has
  succeeded, unless failure recovery is explicit and tested.
- `gwz.lock.yml` must describe observed final member state, not planned state.
- Manifest and lock writes must happen after all required preflight for that
  operation.
- Selection-wide default behavior must not mutate any selected member if another
  selected member fails preflight.
- If partial mutation is allowed, it must be explicit in policy, response, and
  recovery metadata.
- Operations that claim success should leave member status clean unless dirty
  output is an explicit, expected result.
- CLI output must not collapse failed/partial state into ordinary success.

## Known Confirmed Incident To Use As Baseline

Confirmed class:

```text
fast-forward commit added new files -> branch ref advanced -> worktree/index did
not contain new files -> Git reported the added files as deletions
```

Use this incident as a baseline for similar bugs. Do not stop at the current
fix. Look for the same ordering smell in every operation that combines durable
metadata mutation with Git/filesystem mutation.

## Do Not Implement Without Approval

If you find issues, stop at the audit report unless the user asks for fixes.
When fixes are requested, apply them TDD-first with narrow regression tests and
preserve unrelated dirty state.
