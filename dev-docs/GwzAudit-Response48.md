# GWZ Mutation-Safety Audit — Response (Opus 4.8)

Audit of: `gwz-core`, `gwz-cli` (per `dev-docs/GwzAudit.md`)
Date: 2026-06-17
Method: 5 parallel read-only audit agents (git backend / pull+materialize
orchestration / create+add+snapshot+tag+push / artifact writes / status /
CLI), with every P0/P1 re-verified against the source by the lead before
inclusion. No code, tests, docs, or git state were modified. Two findings were
reproduced empirically in throwaway `/tmp` crates.

## Starting State (recorded, not altered)

```
gwz-core  ## main...origin/main   HEAD 40a1be1
  M src/git/mod.rs          <- UNCOMMITTED in-progress fast-forward fix (preserved, not touched)
  ?? dev-docs/GwzAudit.md
gwz-cli   ## main...origin/main   HEAD f47a442 ("Pre split")
  (clean)
```

`gwz-core/src/git/mod.rs` is intentionally dirty: it holds the fix for the
confirmed incident. **That fix is not yet committed.** All findings below were
audited against this working-tree state.

## Executive Summary

**The confirmed P0 is fixed (but uncommitted), and I found no *new* P0 of the
exact incident class.** The triggering bug — `fast_forward` moving the branch
ref before checking out the target tree, so the stale index reported new remote
files as staged deletions — is correctly fixed in the dirty `git/mod.rs`:
`checkout_tree(safe)` now runs **before** the ref moves, and `checkout_commit`
already used that safe ordering. `fetch`/`push` never touch the worktree. I
could not construct another path that makes clean remote state appear as local
changes without an explicit `--force` opt-in.

**What remains is the same *smell* one layer up.** The root cause of the
incident was *"durable state asserted without verifying the worktree reached
it"* — and `git`-returns-ok ≠ tree-is-correct was proven by the incident
itself. `handle_pull_head` was hardened to **mutate → re-observe HEAD/status →
write the lock from the observed state**. Several sibling operations were *not*,
and still trust planned state or the lock instead of re-reading the worktree.
None has an active silent-corruption path as severe as the original (they error
visibly, or the signal survives in an adjacent field), so they are rated **P1**,
but they are the direct residue of the same root cause and matter before the
planned refactor.

Severity tally: **P0: 0 remaining (1 fixed, uncommitted) · P1: 5 · P2: 4.**
`gwz-cli` is correctly thin (no direct git, no artifact I/O, safe default
policies, correct exit codes, complete JSON) — one latent P2 only.

The unifying fix is cheap: **adopt `handle_pull_head`'s "re-observe before you
record" discipline everywhere durable state is written**, and **derive headline
status from observed dirtiness/divergence, not just from Failed/Rejected.**

---

## Findings (ordered by severity)

[P1] Materialize writes the lock and member response from PLANNED state, never re-observing the worktree
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/workspace_ops/mod.rs:594-616
Operation: materialize --snapshot / --tag, pull --snapshot (rewrite_lock path)
Risk: The lock — the durable artifact that `status`, `snapshot`, `tag`, and
`materialize --lock` all trust as the observed member state — is written by
copying `target_lock.members` (the *planned* state loaded from the snapshot/tag
artifact), never by reading each member's HEAD/status back after checkout. The
member response is built the same way (`materialized_response(&manifest,
&plan.member_id, &plan.state)` echoes the plan and stamps `lock_match::Matches`
unconditionally). This is invariant #7 ("verify final member state before
writing gwz.lock.yml") flatly unmet. The incident proved a git call can return
Ok while the tree is wrong; `checkout_commit` is correctly ordered so the active
risk is lower than the original FF bug, but materialize cannot represent a
post-checkout dirty worktree (it records `dirty: false` by construction) and
cannot catch drift — exactly the trust-the-mutation pattern that caused the
incident, now feeding durable truth that snapshot/status propagate.
Evidence:
```
591  backend.checkout_commit(&root.join(&plan.state.path), commit)?;   // mutate
594  Ok(materialized_response(&manifest, &plan.member_id, &plan.state)) // echoes PLAN
...
610  if let Some(state) = target_lock.members.get(member_id) {
611      lock.members.insert(member_id.clone(), state.clone());        // PLANNED, not observed
613  }
615  artifact::write_lock(&root, &lock)?;
```
Contrast the correct pattern in `handle_pull_head_with_events` (794-802):
`backend.head` + `backend.status` are re-read after `fast_forward` and the lock
is built from `resolved_member(member, &head, &status)`.
Reproducer: fake backend whose `checkout_commit` returns Ok but lands a
different commit (or leaves the worktree dirty); run `handle_materialize` with a
one-member snapshot fixture; assert `read_lock(root).members[m].commit ==
backend.head(member_root).commit` and that a dirty post-checkout worktree
surfaces `dirty: true`. Currently the lock equals the planned commit and
`dirty` is absent regardless of the repo.
Expected invariant: `gwz.lock.yml` and the member response must describe the
OBSERVED final state read back from the repo, not the planned target.
Suggested fix: after the executor succeeds, re-read each selected member
(`is_repository` → `head` → `status` → `resolved_member`) and build both the
lock entries and the response from that — mirror pull --head exactly.
Suggested test: `materialize_lock_records_observed_head_not_planned_target`.

[P1] Materialize partial failure leaves member worktrees advanced with a stale lock and no recovery metadata
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/workspace_ops/mod.rs:562-616
Operation: materialize --lock / --snapshot / --tag (and clone-workspace, which delegates here)
Risk: `par_map_per_host` runs every member's clone+checkout closure to
completion with no early cancellation; only afterward does the collector hit the
first `outcome?` and bail — **before** the lock write. So if member B fails,
member A's worktree is already advanced (`checkout_commit` ran), but the lock is
never written (stays at A's old commit) and the response is a bare `Err` with no
per-member outcome and no recovery record. The on-disk worktree and the durable
lock now disagree, invisibly. Materialize hard-codes `AggregateStatus::Ok` and
has no `Partial` path (contrast `handle_push`, which models per-member
Failed/Rejected + `Partial`).
Evidence:
```
562  let outcomes = par_map_per_host(plans, ... |plan| { ... checkout_commit ... }); // ALL members mutate
601  for outcome in outcomes { responses.push(outcome?); }  // first Err returns here
607  if rewrite_lock { ... write_lock ... }                 // skipped on partial failure
619  response_envelope(context, crate::AggregateStatus::Ok, responses)
```
Expected invariant: "Partial mutation, if allowed, must be explicit in policy,
response, and recovery metadata." Here the disk is advanced while the lock and
response report nothing.
Suggested fix: collect per-member outcomes (as push does); on partial failure,
write lock entries for the members that actually succeeded (from observed
state, per the previous finding) and return `Partial` with per-member status —
or document materialize as all-or-nothing and persist a recovery marker naming
the mutated members. Do not leave disk ahead of a silent stale lock.
Suggested test: `materialize_partial_failure_records_moved_members`.

[P1] init-from-sources partial clone failure strands orphan member repos that block retry
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/workspace_ops/mod.rs:316-406
Operation: gwz init (clone N sources in parallel)
Risk: Same par_map shape: if one source clone fails, the others are already
cloned to disk, but `outcome?` (398) returns before `write_manifest`/`write_lock`
(403-405). The succeeded clones are real repos that no manifest/lock describes,
and a retry trips `ensure_member_target_available` ("member path is not empty")
on the orphan, blocking recovery without manual cleanup. Rated below the
materialize cases (P1, not P0) because no durable artifact *misrepresents* the
orphans — nothing claims they are materialized — and fresh clones into
empty targets do not corrupt user work; the harm is blocked retry + partial
mutation without recovery metadata. (Empirically reproduced: one good + one bad
URL → `repos/good` is a real repo, no `gwz.yml`/`gwz.lock.yml`.)
Evidence:
```
397  for outcome in outcomes { let (...) = outcome?; ... }  // bails before any write
403  artifact::write_manifest(...); 405 artifact::write_lock(...);
```
Expected invariant: selection-wide default must not leave some members mutated
when another fails, unless partial state is explicit + recoverable.
Suggested fix: on first clone failure, remove the member dirs this op created
(preflight guaranteed them empty, so deletion is safe) and return cleanly; or
write a partial manifest/lock for the succeeded members behind an explicit
partial policy with per-member Failed responses.
Suggested test: `init_partial_clone_failure_rolls_back_or_reports_partial`.

[P1] snapshot / tag capture the LOCK, not the live worktree — a worktree advanced past the lock is silently frozen at the stale commit
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/workspace_ops/mod.rs:423-437 (snapshot), 465-479 (tag)
Operation: gwz snapshot / gwz tag
Risk: Both handlers take no `GitBackend` and never read each member's HEAD; they
copy member records straight out of `gwz.lock.yml` (`read_lock` →
`selected_member_map`) into the snapshot/tag artifact. If a member's worktree
has advanced (commits made but not re-locked), gone dirty, or been checked out
elsewhere, the artifact records the *stale locked commit*. A later `materialize
--snapshot` then restores the stale commit, silently discarding the member's
real work — and the artifact is a permanent, durable misrepresentation. This is
contingent on intended semantics: `dev-docs/GWZRequirements.md` REQ-084 and
`docs/Reference.md` describe snapshot as "the current selected member states,"
which makes the current lock-relative behavior a defect; if "snapshot == freeze
the lock" is in fact intended, that contradicts the docs and should be made
explicit. (Empirically reproduced: lock commit A, commit B in the worktree,
snapshot records A.)
Evidence:
```
423  let lock = artifact::read_lock(&root)?;
425  let members = selected_member_map(&lock, &selected)?;   // pure lock projection, no head()
426  artifact::write_snapshot(&root, &SnapshotArtifact { members, ... });
```
Expected invariant: an artifact that claims "current state" must reflect each
member's observed HEAD/dirty at capture time.
Suggested fix: thread a `GitBackend` into snapshot/tag; re-read each selected
materialized member (as pull does) and record observed state; decide policy for
dirty/unmaterialized members (reject pre-write, or record an explicit marker) —
do not emit a stale lock commit as if it were current. If lock-relative is
intended, document it and reject when the worktree differs from the lock.
Suggested test: snapshot after committing B without re-locking asserts the
artifact records B (or the op errors on worktree-vs-lock drift).

[P1] Materialize / pull-snapshot / clone-workspace never resync `.gitignore`, so newly-materialized member paths can surface as untracked
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/workspace_ops/mod.rs:504,631,681 (no sync) vs 59/141/227/406 (sync)
Operation: materialize / pull --snapshot / clone-workspace
Risk: `sync_workspace_git_metadata` (writes the managed `.gitignore` block that
ignores each member path inside the workspace root repo) is called only by
create-workspace, create-repo, add-existing, and init. It is NOT called by
`handle_materialize`, `handle_clone_workspace`, or `handle_pull_snapshot`. When
a snapshot/tag materializes a member at a path not present in the workspace
root's committed `.gitignore` (e.g. a member added in a newer manifest than the
checked-out root has staged), that member's tree lands inside the workspace repo
with no ignore entry → `git status` on the root shows the whole member subtree
as untracked. An operation that should leave a clean root makes it dirty.
Evidence: `rg sync_workspace_git_metadata` → call-sites 59, 141, 227, 406 only;
absent from materialize (504-621), clone-workspace (631-679), pull-snapshot
(681-718).
Expected invariant: after any op that materializes members into the workspace
tree, the managed `.gitignore` must cover every materialized member path before
status is observed.
Suggested fix: call `sync_workspace_git_metadata` at the end of
`handle_materialize` (after members are on disk and the lock is written), and on
the clone-workspace/pull-snapshot paths.
Suggested test: `materialize_snapshot_updates_gitignore_for_new_member_paths` —
assert the managed block covers `/repos/new/` and the root is clean post-op.

[P1] Status reports `MemberStatus::Ok` and `aggregate_status::Ok` for a dirty or diverged member
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/status/mod.rs:312, 630-643
Operation: gwz status (structured fields)
Risk: Any openable, materialized member is hard-coded `status: MemberStatus::Ok`
(line 312); dirtiness/divergence live only in the nested `git_status.dirty` and
`lock_match`. `aggregate_status` (630-643) inspects only Failed/Rejected and is
otherwise Ok. So the incident residual state (staged deletions) — and any
uncommitted work — yields member `Ok` and workspace `Ok` at the headline. A
programmatic consumer keying on `aggregate_status`/`MemberStatus` (the whole
point of a declarative workspace tool) reads success over a dirty member.
Mitigant (why P1, not P0): the truth survives in `git_status.dirty`,
`lock_match: Differs`, and the combined-mode `WorkspaceGitStatus.clean` flag
(which correctly ANDs `!is_dirty`), and the CLI human `status` output does render
per-member change sections — so a human running `gwz status` sees the dirty
files. The gap is the structured headline.
Evidence:
```
312  status: crate::MemberStatus::Ok,                       // unconditional
316  git_status: Some(protocol_git_status(member, &head, &status)),  // dirtiness only here
630  fn aggregate_status(...) { ... Failed ... Rejected ... else Ok }  // dirty never consulted
```
Expected invariant: status must surface a dirty/diverged member at the headline
level; a corrupted member must not collapse into ordinary Ok/Ok.
Suggested fix: derive `MemberStatus` (and feed `aggregate_status`) from observed
dirtiness/divergence — a distinct non-Ok signal when `status.is_dirty` or
`lock_match == Differs`.
Suggested test: `status_dirty_member_is_not_aggregate_ok`.

[P2] `lock_match` ignores branch / detached / upstream, and returns `Matches` for a dirty-but-matching member
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/status/mod.rs:556
Operation: lock_match
Risk: The comparison is only `commit` + `dirty`. The lock records `branch`,
`detached`, and `upstream`, but a member at the locked commit while DETACHED or
on a different branch reports `Matches`. Separately, when the lock recorded
`dirty: Some(true)` and the worktree is still dirty, `(true == true)` →
`Matches` — "matches a dirty lock" reads as "in sync"; and `dirty: None`
collapses to clean.
Evidence: `if locked.commit == head.commit && locked.dirty.unwrap_or(false) ==
status.is_dirty { Matches } else { Differs }` — branch/detached/upstream unused.
Expected invariant: a lock-vs-observed match must account for every recorded
dimension (or document what it intentionally ignores), and must not report a
clean "Matches" for a currently-dirty member.
Suggested fix: require branch/detached agreement; treat any observed
`is_dirty` as not a clean `Matches`.
Suggested test: `lock_match_differs_when_detached_at_locked_commit` +
`lock_match_dirty_member_is_not_matches`.

[P2] `write_atomic` is rename-atomic but not crash-durable, and uses a fixed temp name
File: /Users/owebeeone/limbo/glial-dev/gwz-core/src/artifact/mod.rs:335-343, 381-387
Operation: write_manifest / write_lock / write_snapshot / write_tag
Risk: (a) No `sync_all` on the temp file before rename and no parent-dir fsync
after — atomic for concurrent readers, but on crash/power-loss right after
`rename` the artifact may be truncated or revert. (b) The temp path is a fixed
`<name>.tmp` in the same dir; safe within one op (the lock is written once on the
single-threaded tail; members write distinct paths) but two concurrent gwz
processes writing the same lock/manifest would race the same temp file. There is
no workspace lock serializing mutating commands.
Evidence: `fs::write(&tmp); fs::rename(&tmp, path)` with no flush; `temp_path` →
`format!("{file_name}.tmp")`. `rg fsync|sync_all` in artifact/mod.rs → none.
Expected invariant: an artifact that returns Ok is durable across a crash;
concurrent writers must not corrupt it.
Suggested fix: `File` + `write_all` + `sync_all` on the temp, rename, then fsync
the parent dir; make the temp name unique (`{name}.{pid}.{nonce}.tmp`); consider
an advisory workspace lock for mutating ops.
Suggested test: `write_atomic_fsyncs_before_rename`;
`concurrent_write_atomic_same_path_no_corruption`.

[P2] CLI status renderer never prints `aggregate_status`; failure surfacing depends entirely on per-member fields
File: /Users/owebeeone/limbo/glial-dev/gwz-cli/src/main.rs:1063 (vs 1036 for the mutating-op renderer)
Operation: render human output (status command)
Risk: `render_human_status_response` (taken whenever `workspace_git_status` is
set, i.e. the `status` command) never references `meta.aggregate_status`; a
non-Ok aggregate with no per-member signal would fall through to "nothing to
commit, working tree clean". Latent only today: `status` is the sole request
that sets `workspace_git_status`, and core's status aggregate is non-Ok only
when a member is Failed/Rejected (always surfaced by `append_status_issues`), so
a hidden non-Ok aggregate is structurally unreachable — but the renderer has no
guard if that ever changes (e.g. if finding P1-status adds a Dirty aggregate).
Expected invariant: human output must make a non-Ok aggregate visible regardless
of per-member detail.
Suggested fix: in `render_human_status_response`, surface the aggregate (or
suppress the "clean" fallback) when `meta.aggregate_status` is not Ok/Noop/Accepted.
Suggested test: feed a `Failed` aggregate + clean workspace + all-Ok members;
assert output contains "Failed" and is not the clean-tree line.

---

## Open Questions

1. **Snapshot semantics (gates finding P1-snapshot/tag).** Is a snapshot/tag a
   freeze of the *current worktree* (REQ-084 "current state") or a freeze of the
   *lock*? The code does the latter; the docs say the former. This is a policy
   decision, not just a bug, and it dictates the fix (re-observe vs reject-on-drift).
2. **Materialize atomicity policy.** Is materialize intended all-or-nothing, or
   partial-with-recovery? It currently is neither (silent partial). Pick one.
3. **Lock as source of truth.** Several ops trust the lock without re-deriving it
   from the worktree. Should there be a single "re-observe selected members" step
   that every durable write funnels through (the pull --head pattern, extracted)?
4. **Workspace-level locking.** Should concurrent mutating `gwz` invocations be
   serialized by an advisory lock (relates to P2-write_atomic)?
5. **Headline status semantics.** Should `MemberStatus`/`aggregate_status`
   incorporate dirty/diverged, or is "Ok = openable & materialized, see
   git_status for cleanliness" the intended contract? If the latter, document it.

## Test Gaps

- No test asserts a materialize/pull lock reflects *observed* HEAD vs a faked
  divergent checkout (P1-materialize-planned).
- No test for materialize/init partial failure (worktree-vs-lock divergence,
  orphan clones) (P1-materialize-partial, P1-init).
- No test that snapshot/tag captures a worktree advanced past the lock
  (P1-snapshot).
- No test that materialize updates `.gitignore` / leaves the root clean
  (P1-gitignore).
- No test that a dirty member changes `MemberStatus`/`aggregate_status`
  (P1-status); `lock_match` has no branch/detached/dirty-edge tests (P2).
- CLI: no `Partial`/`Failed`/`Rejected` coverage in the status renderer or
  exit-code table (only Ok/Accepted today) (P2-cli).

## Areas Reviewed, No Issue

- **Git backend post-fix (the epicenter):** `fast_forward` now `checkout_tree
  (safe)` → `set_target` → `set_head` (worktree before ref); `checkout_commit`
  already used `checkout_tree(safe)` → `set_head_detached`. `fetch`/`push` do not
  touch the worktree. The dirty fix adds a nested-new-file (`dev-docs/new.md`)
  regression test asserting content materializes and status is clean.
- **pull --head preflight + execution:** preflight rejects dirty/diverged/missing-
  remote members for the *whole* selection before any mutation (tested:
  `pull_head_dirty_member_blocks_all_selected_members_before_mutation`,
  `..._divergence_blocks_...`); the lock is written from re-observed HEAD/status.
  This is the reference-correct pattern the other handlers should adopt. (One
  residual: a mid-execution-loop git error after an earlier member fast-forwarded
  leaves that member advanced with the lock unwritten — narrow fetch→execute race,
  same family as P1-materialize-partial.)
- **materialize / pull preflight:** dirty existing members rejected by default
  (`materialize_lock_blocks_dirty_member_by_default`); snapshot/tag-not-found
  fails before mutation.
- **create-repo / add-existing-repo:** git repo created/verified and HEAD/status
  observed *before* the manifest describes it; lock built from `resolved_member`
  (observed). Correct order. (Note: manifest-then-lock is two non-transactional
  writes — if the lock write fails after the manifest write, the manifest
  advertises a member with no lock entry; low-likelihood, not reproduced, worth a
  rollback or lock-first ordering — borderline P2.)
- **push / push-with-events:** writes no local durable state; per-member failures
  are explicit (Failed/Rejected) and roll up to `Partial`.
- **status is strictly read-only:** only `is_repository`/`head`/`status` +
  `read_manifest`/`read_lock`; no checkout/reset/write outside `#[cfg(test)]`.
  Unmaterialized/failed/dirty are distinct buckets, not collapsed; combined-mode
  `clean` correctly reflects dirtiness.
- **gwz-cli is correctly thin:** no `git2`/`Command`/repo fs-mutation in
  production code (only `#[cfg(test)]` temp dirs); no direct `gwz.conf`/`.gwz`
  I/O; `--partial`/`--force`/`--sync` are opt-in only (defaults Atomic / Refuse /
  ff-only); exit codes map correctly (Accepted/Ok/Noop→0, Rejected→2,
  Partial/Failed→1) for all output modes; JSON/JSONL carry full per-member
  error/state/dirty/lock_match detail. Mutating-op human output leads with
  `status: {aggregate}` and lists per-member errors.
- **par_map_per_host:** host grouping / per-host + global caps / order-preserving
  / empty-input are correct and tested; not a mutation-ordering concern (its only
  audit relevance is that it runs all closures to completion → the partial-failure
  findings above).

## Commands Run (all read-only)

- `git -C <repo> status --short --branch` / `log --oneline --decorate -5` (both repos)
- `git diff -- src/git/mod.rs` (read the dirty FF fix; not modified)
- `git show HEAD:src/git/mod.rs` (committed baseline ordering)
- `rg` for: mutation/ref ops, artifact writers + call-sites, clone/fetch/push/
  materialize/pull, dirty/lock_match/preflight/partial, `sync_workspace_git_metadata`,
  direct-git/artifact-I/O in the CLI
- `Read` of git/mod.rs, workspace_ops/mod.rs, artifact/mod.rs, status/mod.rs,
  operation/mod.rs, gwz-cli/src/main.rs, and relevant `model`/`protocol` defs
- `cargo test --no-run` (compile-only; no execution side effects)
- Two reproducers built/run under `/tmp` (path-linked to the real crate), then
  removed: init partial-clone orphan; snapshot-records-stale-lock
- `gwz-core/src/git/mod.rs` was read but never touched; no `gwz pull`/`materialize`
  or other mutating GWZ command was run; no workspace files altered.

## Repos and Starting Commit IDs

- `gwz-core` @ `40a1be1` (main), working tree dirty: `src/git/mod.rs` (uncommitted FF fix)
- `gwz-cli` @ `f47a442` (main), clean
