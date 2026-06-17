# GWZ Audit Response 55

Status: consolidated read-only audit response
Date: 2026-06-17

Scope:

- `/Users/owebeeone/limbo/glial-dev/gwz-core`
- `/Users/owebeeone/limbo/glial-dev/gwz-cli`

Agents:

- Pascal: Git backend/ref/worktree safety
- Nietzsche: workspace operations and artifact ordering
- Nash: CLI/status/reporting safety
- Socrates: test-gap and reproducer matrix

Starting state recorded by the agents and rechecked before writing this file:

- `gwz-core`: `40a1be1` on `main`, with `M src/git/mod.rs` and `?? dev-docs/GwzAudit.md`
- `gwz-cli`: `f47a442` on `main`, clean

No agent modified code, tests, Git state, or workspace state. This file is the
only requested consolidation artifact.

## Executive Conclusion

The confirmed fast-forward bug is not isolated. The same failure class appears
in several other flows:

```text
durable refs, locks, artifacts, or responses can claim a new state before the
worktree/index/member state has been proven to match it
```

The highest-risk areas are:

1. `pull --head` fetches and can mutate remote-tracking refs during preflight,
   before all selected members are known to be safe.
2. `pull --head` writes lock state after `fast_forward` without rejecting dirty
   or unexpected post-operation status.
3. `materialize`/`pull_snapshot` rewrite lock state from the planned target
   rather than the observed final member state.
4. `push` can partially advance remote refs under the default atomic policy.
5. `snapshot` and `tag` persist stale lock state without live member
   verification.
6. CLI JSON/JSONL failure paths bypass structured rendering, so automation can
   lose per-member failure detail.
7. CLI accepts `--sync fetch-only`, but `pull --head` still fast-forwards.

The immediate design correction should be policy-first and invariant-first:

- Split every selected-member operation into a non-mutating validation phase and
  a mutation phase.
- Treat Git refs, remote-tracking refs, worktrees, index state, lock files,
  manifests, snapshots, tags, and event/response claims as durable state.
- Do not mutate any selected member under the default atomic policy until every
  selected member has passed validation.
- After a mutation, re-read `HEAD` and `status`; build lock files and responses
  from observed final state, not intended state.
- If partial success is allowed, make it explicit in policy, in the response, and
  in enough recovery metadata for users and drivers to understand what changed.

## Consolidated Findings

### P1: `pull --head` Mutates During Preflight

Files:

- `gwz-core/src/workspace_ops/mod.rs:1601`
- `gwz-core/src/workspace_ops/mod.rs:1710`

Operation: `pull --head`

Risk: a clean selected member can have `refs/remotes/<remote>/<branch>` advanced
even though another selected member later rejects the operation as dirty or
diverged. The command can fail without changing the branch or lock, but Git refs
inside another member have still moved.

Evidence: `pull_head_preflight` runs selected members through `par_map_per_host`.
Inside each member's preflight, status is checked and then `backend.fetch` is
called before all selected members are known to be safe.

Required decision: decide whether `fetch` is allowed as a preflight mutation. If
GWZ's default atomic guarantee covers refs, `fetch` must be moved out of
preflight or represented as a deliberate partial mutation.

Recommended tests:

- Two selected members: one clean member with a remote update, one dirty member.
  Run default `pull --head`; assert the clean member's remote-tracking ref did
  not advance.
- Repeat with explicit partial/fetch policy once that policy is specified.

### P1: `pull --head` Accepts Dirty Post-Fast-Forward State

File: `gwz-core/src/workspace_ops/mod.rs:787`

Operation: `pull --head`

Risk: a backend checkout/ref bug can leave staged deletions or dirty files while
GWZ returns success and writes `gwz.lock.yml` for the new commit.

Evidence: after `backend.fast_forward`, the handler reads `head/status`, inserts
the resolved member into the lock, and writes the lock. It does not reject
`status.is_dirty`.

Required invariant: a successful default pull must leave every mutated member at
the observed target commit with clean status, or it must fail without writing a
success lock.

Recommended tests:

- Fake backend where `fast_forward` returns `Ok` but `status()` returns dirty;
  assert `handle_pull_head` errors and leaves the lock unchanged.
- Real backend add/delete/rename/nested-file fast-forward cases that assert
  final `status().is_dirty == false`.

### P1: `materialize` Writes Planned Lock State Instead Of Observed State

Files:

- `gwz-core/src/workspace_ops/mod.rs:590`
- `gwz-core/src/workspace_ops/mod.rs:607`
- `gwz-core/src/git/mod.rs:236`

Operations: `materialize --lock`, `pull --snapshot`, `clone_workspace`

Risk: checkout can succeed but leave the member dirty or at an unexpected
commit, while the response and rewritten lock claim the planned snapshot/tag
state.

Evidence: `handle_materialize` calls `backend.checkout_commit`, returns a member
response from `plan.state`, then copies selected `target_lock` states into the
lock. It does not re-read `HEAD` and `status` after checkout. The backend
`checkout_commit` result only reports the requested commit.

Required invariant: materialize-style operations must write lock state from
observed final member state. If the final state is dirty or not the target
commit, the operation should fail or explicitly record non-matching state.

Recommended tests:

- Fake backend where `checkout_commit` returns `Ok` but `head/status` show old
  commit or dirty state; assert materialize rejects and does not rewrite the
  lock.
- Two selected members where the first checkout succeeds and the second fails
  after preflight; pin whether no member mutates or whether partial state is
  explicitly reported and recoverable.

### P1: `materialize` Can Partially Mutate Selected Members

File: `gwz-core/src/workspace_ops/mod.rs:1441`

Operations: `materialize`, `pull_snapshot`

Risk: member A can be cloned or checked out before member B fails clone or
checkout, leaving worktrees changed with stale lock and no persistent partial
recovery metadata.

Evidence: preflight does not fully prove non-repo target availability or target
commit reachability for all selected members before mutation. Mutations then run
through `par_map_per_host`.

Required invariant: under default atomic policy, selected-member validation must
complete before any selected member is cloned or checked out.

Recommended tests:

- Two selected members: first clean and movable, second has a non-empty non-repo
  path or missing target commit. Assert no selected member mutates.
- If partial materialize is intentionally supported later, assert response and
  recovery metadata identify exactly what changed.

### P1: `push` Can Partially Advance Remote Refs Under Default Policy

Files:

- `gwz-core/src/workspace_ops/mod.rs:869`
- `gwz-core/src/workspace_ops/mod.rs:1899`
- `gwz-core/src/workspace_ops/mod.rs:2010`

Operation: `push`

Risk: one selected member can push successfully while another selected member is
rejected or fails, even though `OperationPolicy::default()` is atomic.

Evidence: `handle_push_with_events` runs selected members in parallel. Each
member validates and calls `backend.push` in the same path. Aggregate status can
be `Partial`, but only after a remote ref may already have advanced. The `partial`
policy is not used to gate this behavior.

Required invariant: default push must preflight all selected members before
calling `backend.push` for any member. Partial remote mutation should require an
explicit partial policy.

Recommended tests:

- Two selected members: one pushable, one missing a push remote. Under default
  policy, assert the pushable member's remote ref is unchanged.
- Repeat with explicit partial policy and assert the successful remote ref
  advances while the failed member is reported with member identity.

### P1: `snapshot` And `tag` Capture Stale Lock State

Files:

- `gwz-core/src/workspace_ops/mod.rs:421`
- `gwz-core/src/workspace_ops/mod.rs:465`

Operations: `snapshot`, `tag`

Risk: dirty or advanced members can be snapshotted or tagged as if the lock still
matches the live workspace. This creates durable named states that may never have
represented reality.

Evidence: both handlers read `gwz.lock.yml` and write artifacts directly. Their
responses are derived from locked member state and use `lock_match: Matches`
without live member verification.

Required invariant: a snapshot/tag should either be a snapshot/tag of the live
observed workspace state or should explicitly state that it is a tag of the
existing lock file without claiming live match.

Recommended tests:

- Dirty a member after lock creation, then run snapshot/tag; assert rejection or
  explicit dirty/live-state capture.
- Advance a member after lock creation without updating the lock; assert
  snapshot/tag does not claim the stale lock matches live state.

### P1: CLI JSON/JSONL Errors Bypass Structured Rendering

Files:

- `gwz-cli/src/main.rs:196`
- `gwz-cli/src/main.rs:1019`

Operations: any `gwz --json` or `gwz --jsonl` command that errors before a
successful response envelope

Risk: machine consumers can receive empty stdout or a partial JSONL stream
followed by plain stderr. Per-member error detail may be lost.

Evidence: `main` only calls `render_response` on `Ok`. `Err` prints
`gwz: ...` to stderr. `execute_invocation` collapses `ModelError` into
`CliError` text.

Required invariant: `--json` and `--jsonl` must produce structured terminal
errors even when core returns an error instead of a response envelope.

Recommended tests:

- Dirty/diverged member with `gwz --jsonl pull --head`; assert every stdout line
  is JSON and the final record contains aggregate failure, error code,
  `member_id`, and `member_path` when known.
- `gwz --json status` outside a workspace; assert structured JSON error rather
  than plain stderr-only output.

### P1: `--sync fetch-only` Is Accepted But Pull Still Fast-Forwards

Files:

- `gwz-cli/src/main.rs:570`
- `gwz-cli/src/main.rs:1983`
- `gwz-core/src/workspace_ops/mod.rs:775`
- `gwz-core/src/workspace_ops/mod.rs:787`

Operation: `gwz --sync fetch-only pull --head`

Risk: a user-selected non-worktree-mutating policy can still advance member
branches and worktrees.

Evidence: CLI maps `SyncArg::FetchOnly` to core policy. `pull --head` still
plans and executes `backend.fast_forward` for fast-forward plans; no policy check
prevents the mutation.

Required invariant: `fetch-only` must not advance local branch refs or worktrees.
It may update remote-tracking refs only if the atomic/fetch policy explicitly
allows that.

Recommended test:

- Member starts at commit A, remote has commit B. Run
  `gwz --sync fetch-only pull --head`; assert member `HEAD` remains at A and the
  response clearly reports fetch-only behavior.

## P2 Findings

### P2: Generic Operation Aggregation Cannot Express Partial Success

Files:

- `gwz-core/src/operation/mod.rs:768`
- `gwz-core/src/operation/mod.rs:834`

Operation: `OperationRuntime` / generic `ResponseBuilder::result`

Risk: mixed OK plus failed/rejected member execution is reported as `Failed` or
`Rejected`, not `Partial`. Top-level errors serialize with `member_id: null` and
`member_path: null`.

Evidence: `aggregate_status` has no `Partial` branch. `OperationError` carries
only code/message, and `operation_error_to_protocol` always sets member fields
to `None`.

Recommended test:

- Build an `ExecutionReport` with one `Ok` member and one `Failed` member.
  Expected aggregate should be `Partial`, and member-scoped errors should retain
  identity either in `members[].error` or by not being duplicated as anonymous
  top-level errors.

### P2: Human `status --no-files` Can Hide Dirty State

Files:

- `gwz-cli/src/main.rs:1906`
- `gwz-core/src/status/mod.rs:326`
- `gwz-cli/src/main.rs:1063`
- `gwz-cli/src/main.rs:1238`

Operation: `gwz status --no-files`

Risk: dirty root/member states, including staged deletions, can render only
branch text with no dirty summary.

Evidence: `--no-files` disables file-change output. Core still knows clean/dirty
counts, but human rendering primarily prints branch summaries, file sections,
unmaterialized notices, and non-OK issues. Dirty members can remain
`MemberStatus::Ok`, so they may not appear under issues.

Recommended tests:

- Dirty root and dirty member with `status --no-files`; require explicit dirty
  summary/counts.
- Staged deletion and staged addition cases.

### P2: Manifest/Lock/Gitignore Updates Are Not Semantically Atomic

File: `gwz-core/src/workspace_ops/mod.rs:134`

Operations: `create_workspace`, `create_repo`, `add_existing_repo`,
`init_from_sources`

Risk: failures after the first write can leave `gwz.yml` ahead of
`gwz.lock.yml`, or both artifacts updated while `.gitignore` or index staging
failed.

Evidence: write order is manifest, lock, then workspace Git metadata sync in
several handlers.

Recommended tests:

- Force `stage_workspace_git_metadata` failure, for example with an index lock in
  a disposable temp repo. Assert rollback or explicit recovery state, not silent
  manifest/lock advancement.

### P2: Artifact Writes Are File-Atomic But Race-Prone

File: `gwz-core/src/artifact/mod.rs:335`

Operations: manifest, lock, snapshot, and tag writes

Risk: fixed `{file}.tmp` names can collide across concurrent writers, `rename`
can overwrite, and snapshots do not have the same duplicate-ID guard that tags
have.

Evidence: `write_atomic` always writes `temp_path(path)` and renames it to the
target. `handle_snapshot` writes without checking whether the snapshot ID already
exists.

Recommended tests:

- Duplicate snapshot ID should fail.
- Concurrent same-tag/same-snapshot writes should produce one success and one
typed duplicate/conflict error.

### P2: Status Models Renames But Does Not Enable Or Test Rename Detection

File: `gwz-core/src/git/mod.rs:253`

Operation: `status`

Risk: renamed files can be reported as delete/add instead of `R` with
`original_path`, weakening status output and hiding rename-specific regressions.

Evidence: status options include untracked recursion, but no rename detection
setting. The mapper handles `INDEX_RENAMED` and `WT_RENAMED`, and tests cover
clean/untracked/modified/staged states but not renames.

Recommended tests:

- Staged rename and unstaged rename, including nested paths. Assert either a
  single rename entry with `original_path` or document that rename detection is
  intentionally unsupported.

## Test Gap Matrix

| Scenario | Core coverage | CLI coverage | Required additions |
| --- | --- | --- | --- |
| Existing file modification | Covered as baseline | Covered as baseline | Keep |
| Added top-level file | Missing | Missing | Add fast-forward/pull tests |
| Added nested file | Partial backend regression exists in dirty worktree | Missing | Extend to workspace and CLI |
| Deleted tracked file | Missing | Missing | Add backend/workspace/CLI tests |
| Renamed tracked file | Missing | Missing | Add backend/workspace/CLI tests |
| Checkout failure after preflight | Missing | Missing | Fake backend failure after clean preflight |
| Dirty target rejection | Partial | Partial | Add untracked/nested/delete/rename dirty forms |
| Post-operation clean status | Partial backend assertion | Missing | Assert clean status and lock `dirty: false` |
| Partial multi-member failures | Partial and inconsistent | Missing | Two-member partial push/pull/materialize tests |
| Fetch-only pull | Missing | Missing | Assert no local HEAD/worktree mutation |
| Structured JSON/JSONL errors | Missing | Missing | Assert structured terminal error records |

## Recommended Fix Order

1. Define the policy matrix before editing implementation:
   - Default atomic behavior for fetch, fast-forward, checkout, push, and
     artifact writes.
   - Explicit partial behavior and recovery metadata.
   - Exact meaning of `fetch-only`, `ff-only`, `force`, and destructive
     materialize.

2. Add failing tests around the confirmed risk class:
   - `pull --head` dirty member must not mutate clean member remote-tracking refs
     unless policy permits it.
   - `pull --head` must reject dirty post-fast-forward state and leave the lock
     unchanged.
   - `materialize` must verify observed post-checkout `HEAD` and `status` before
     rewriting the lock.
   - Default `push` must not partially advance remote refs.

3. Refactor selected-member operations into phases:
   - Phase A: non-mutating validation for every selected member.
   - Phase B: mutation only after the selected set passes Phase A.
   - Phase C: post-mutation observation and validation.
   - Phase D: artifact/lock/response writes from observed final state.

4. Make partial results explicit:
   - `AggregateStatus::Partial` for mixed outcomes.
   - Per-member error identity in JSON/JSONL.
   - Persistent recovery metadata if a command can leave selected members in
     different terminal states.

5. Fill the tree-shape test matrix:
   - Added file
   - Added nested file
   - Deleted file
   - Renamed file
   - Nested delete/rename
   - Symlink/executable-bit cases if the backend intends to preserve them.

## Open Policy Questions

1. Does GWZ's atomic guarantee cover remote-tracking refs updated by `fetch`?
   The audit treated them as durable mutation because they are Git refs and can
   affect later planning.

2. Should `--force materialize` allow a dirty final state, or does it only allow
   overwriting while still requiring an end-clean member?

3. Should snapshot/tag represent the live workspace state or the current lock
   file state? Either can be valid, but the command output must not claim a live
   match if only the lock was copied.

4. Should default push be strictly atomic across selected members? The current
   protocol has `Partial`, but the default policy says atomic.

5. What persistent recovery record should exist if partial mutation is allowed
   for pull/materialize/push?

## Areas With No New Issue Found

- `gwz-cli` appears thin for Git/artifact ownership. It constructs a
  `Git2Backend` and routes work into `gwz-core`; no direct `git2`,
  `Command::new("git")`, or artifact read/write path was identified in the CLI
  slice.
- `clone_repo` preflights existing targets with non-empty target rejection, and
  existing tests verify non-empty target preservation.
- The live `gwz-core/src/git/mod.rs` fast-forward patch checks out the target
  tree before moving the local branch ref, addressing the original
  ref-before-checkout bug in that backend method.

