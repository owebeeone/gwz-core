# GWZ Gitlink Boundary Plan

> **SUPERSEDED 2026-06-23.** The gitlink boundary specified below was implemented
> (WS1–WS6) and then **reverted** at the owner's direction in favor of hiding members
> via the root repo's local `.git/info/exclude` (members + `/gwz.conf/.tmp/`,
> regenerated from the lock on every run, never committed). Members are no longer
> tracked as `160000` gitlinks, and gwz writes no `.gitignore`; `gwz.yml` /
> `gwz.lock.yml` is the authoritative record — which lands back on the original AD2
> disposition. The **`gwz commit` verb (§6) shipped and remains current**, minus the
> gitlink-sync step: the root commit now records the lock, not gitlinks. See
> `GWZDesign.md` → "Root/Member Boundary" for the live boundary. The remainder of this
> document is retained as the historical design record of the gitlink approach.

---

Status: ~~ratified 2026-06-22~~ — boundary mechanism reverted (see banner). The
`gwz commit` verb (§6) remains the live spec. Owner decision: Gianni.

Original framing (historical):
- Overrode `history/GwzAuditResolutionPlan.md` §2 **AD2** — which parked gitlink as a
  *deferred spike* and named `.git/info/exclude` the interim. (The revert restores that
  interim as the final boundary.)
- Closed Finding **F4** (`.gitignore` not resynced) and removed the dynamic `.gitignore`
  member-block writer; the exclude block now plays the resynced-on-every-run role.

## 1. Decision And Rationale

**Replace the committed `.gitignore` member-hiding block with gitlink index
entries that gwz refreshes from the lock on every change.** Each member directory
becomes a `160000` (commit) entry in the **root repo's index**, pointing at the
member commit gwz has *recorded* (the lock's `commit`). The git representation is
an **index-only projection of `gwz.lock.yml`, synced on demand** — never committed
by gwz implicitly.

This closes the open question AD2 left explicit ("the bare-gitlink spike must
answer whether it is a *committed* projection, an *index-only* projection, or
*rejected*"): **index-only, synced on demand.**

### Why this is not the "volatile pinned-oid" trap AD2 warned about

AD2's cost case was aimed at the **committed** gitlink — an oid baked into shared
root history that goes stale/unreachable and churns the root log, re-introducing
the audit's "durable state recorded before it's proven" class. The index-only
projection avoids all three:

- **recorded ≠ live** shrinks to the same drift window the lock already has, and
  per AD3 that drift is the *normal resting state*, already surfaced by
  `gwz status`. Gitlink adds no new staleness — it just makes root `git status`
  show a moved member as one line instead of `.gitignore`'s total silence.
- **stale/unreachable oid** and **root-log churn** only bite the committed mode.
  Index-only, neither applies — gwz rewrites the index entry, never commits it, and
  the index is a *rebuildable* projection of the lock (the master).
- The one path that *does* pin oids into history is committing the root — now an
  explicit, gwz-managed verb (`gwz commit`, §6) that syncs gitlinks from the lock
  first, plus the developer's own raw `git commit`. gwz never commits the root
  implicitly. See §7 R1.

`gwz.yml` stays authoritative for membership; `gwz.lock.yml` is the source of the
oid; the index is the projection. This is exactly AD2's ratified shape ("git
representation is a projection synced on demand, not the master") — only the
projection *mechanism* changes from `.gitignore` to gitlink.

## 2. Decisions To Confirm (ratify before §5)

| # | Decision | Recommendation |
| --- | --- | --- |
| G1 | Projection mode | **Index-only, synced on demand.** gwz refreshes the gitlink in the index from the lock after every lock write and never commits it *implicitly*. The only root commit is the explicit `gwz commit` verb (§6). |
| G2 | Oid source | **The lock** (`ResolvedMemberArtifact.commit`), not a fresh live `head()`. The index mirrors gwz's state of record; "live" only enters when an op observes and writes the lock (capture/materialize/pull/commit). |
| G3 | `.gitignore` fate | **Only ever ensure the `/gwz.conf/.tmp/` line is present** (append-if-missing, idempotent). gwz stops *writing* the dynamic member block, but it does **not** strip or edit anything — old `.gitignore` entries (incl. a legacy managed block) stay exactly as-is. |
| G4 | Where the tmp ignore lives | **Stays in `.gitignore`** as a single static line — not `.git/info/exclude`. "Move away from `.gitignore`" applied to *member hiding* (now gitlinks); a shared, static tmp-ignore is a legitimate keep and survives clones. Retained regardless of whether `.tmp/` is still produced post-F12 (cheap safety ignore). |
| G5 | New primitive name | `sync_gitlinks` (reconciles the root index's `160000` entries to a desired set). Alt: `stage_gitlinks`. Bikeshed, non-blocking. |
| G6 | Boundary-sync atomicity | **Best-effort, after the lock.** The index is a rebuildable projection; a crash between lock-write and sync leaves a stale index that the next op repairs. NOT folded into the consistency-critical `write_manifest_and_lock` seam. |

## 3. The Backend Primitive (AD1 contract)

New `GitBackend` method, defined alongside the existing self-verifying primitives in
`src/git/gitbackend.rs:8` and implemented for `Git2Backend` — the **only** place
allowed to name `git2` (AD1; today there is zero production `git2::Index` use
outside `src/git/*` — confirmed). It mirrors the `stage_paths` template at
`src/git/gitbackend.rs:668` (mutate → write → re-open fresh → self-verify → count).

```rust
/// Reconcile the ROOT index's gitlink (`160000`) entries to exactly `desired`
/// (member_path → commit oid). Adds/updates an entry per desired member, removes
/// any existing `160000` entry whose path is not in `desired`, and touches no
/// other index entry. Index-only — never commits. Self-verifies every desired
/// entry persisted with mode 160000 and the requested oid, and that no stale
/// gitlink remains, before returning. Parity with `git update-index --cacheinfo
/// 160000,<oid>,<path>` is proven by contract test.
fn sync_gitlinks(&self, root: &Path, desired: &[(&str, &str)])
    -> ModelResult<GitGitlinkResult>;   // { written: usize, removed: usize }
```

Implementation shape:
- Open the root repo (`open_repo(root)`), take `repo.index()`.
- Build the desired-path set. For each `(path, oid)`: parse `oid` → `git2::Oid`;
  construct an `IndexEntry { mode: 0o160000, id: oid, path: path.into(), file_size: 0,
  ctime/mtime/dev/ino/uid/gid/flags*: 0, .. }`; `index.add(&entry)`.
- Remove reconciliation: iterate `index.iter()`, collect entries with
  `mode == 0o160000` whose path ∉ desired set, `index.remove_path(p)` each. **Guard:**
  only `160000` entries are ever removed — real tracked files are untouched.
- `index.write()`.
- **AD1 self-verify:** re-open the repo so the index is read fresh; for each desired
  `(path, oid)` assert `get_path(path, 0)` is `Some` with `mode == 0o160000` and
  `id == oid`; assert no `160000` entry outside the desired set remains; else
  `ErrorCode::GitCommandFailed`.

Notes:
- Operates on the **root** repo only; never opens a member repo.
- The member must be a real nested repo on disk for git to treat the `160000`
  entry as a gitlink boundary (it already is — members live at
  `root/<member.path>/.git`). The **caller** filters the desired set to members
  that are materialized with a recorded commit (§4.1); the primitive writes what
  it's given and proves the index state.

## 4. Integration

### 4.1 Source the desired set from the lock

After every lock write, derive `desired` from `LockArtifact.members`
(`src/artifact/mod.rs:135`): for each `ResolvedMemberArtifact` with
`materialized == Some(true)` **and** `commit.is_some()`, emit
`(member.path, commit)`. Skip unmaterialized / carry-lock / unborn members (AD3(b)):
no on-disk repo or no oid means no gitlink — and the reconcile step drops any entry
they previously had.

### 4.2 New `workspace_ops` seam

Replace the `.gitignore`-based `sync_workspace_git_metadata`
(`src/workspace_ops/handle_create_repo.rs:477`) with a lock-driven boundary sync:

```text
fn sync_workspace_boundary(root, &LockArtifact) -> ModelResult<()>:
    ensure_gitignore_tmp(root)                  // G3/G4: append `/gwz.conf/.tmp/` if missing; touch nothing else
    let desired = desired_gitlinks(lock)        // §4.1
    Git2Backend::new().sync_gitlinks(root, &desired)?
    stage_workspace_git_metadata(root)          // stages gwz.conf + the static .gitignore
```

`stage_workspace_git_metadata` (`src/workspace_ops/stage_workspace_git_metadata.rs:9`)
stays as-is — it stages only explicit pathspecs (`gwz.conf`, `.gitignore`), never
`.`, so it will not pull in member contents.

### 4.3 Call it after **every** lock write (fixes the F4-class gap)

The current `.gitignore` sync runs on only three paths; the lock-only handlers
never resync the boundary. Wire `sync_workspace_boundary` in after each lock write:

| Handler | Lock write today | Action |
| --- | --- | --- |
| `handle_create_workspace` | `write_manifest_and_lock` @ `handle_create_repo.rs:51` area | swap existing `sync_workspace_git_metadata` → `sync_workspace_boundary` |
| `handle_create_repo` (add) | `:131` | swap |
| `handle_add_existing_repo` | `:215` | swap |
| `handle_init_from_sources` | `handle_init_from_sources.rs:203` | swap |
| `handle_materialize` | `write_lock` @ `handle_materialize.rs:322` | **add** call (was missing) |
| `handle_pull_head_with_events` | `write_lock` @ `pull_head_member_preflight.rs:101` | **add** call (was missing) |
| `handle_capture` | `write_lock` @ `handle_materialize.rs:135` | **add** call (was missing) |
| `handle_snapshot` / `handle_tag` | no lock write | **no change** — named artifacts don't move the pointer |

Per G6 the call sits *after* the lock write and is best-effort with respect to
crash safety (lock is master; index rebuilds on next op).

### 4.4 `.gitignore`: only ensure the tmp line (no stripping)

- Add one idempotent `ensure_gitignore_tmp(root)`: if `.gitignore` has no entry for
  `/gwz.conf/.tmp/`, append it; otherwise no-op. It **never** removes or rewrites
  existing lines. A fresh workspace ends up with just that line; an existing one keeps
  whatever it has, plus (if absent) the tmp line.
- Stop calling the dynamic member-block writers and delete them once dead:
  `update_workspace_gitignore`, `managed_gitignore_block`, and the
  `replace_managed_gitignore_block` surgical-replace logic / `GITIGNORE_GWZ_BEGIN/END`
  constants (`src/workspace_ops/replace_managed_gitignore_block.rs`).
- A legacy managed block in an existing workspace's `.gitignore` is left untouched —
  its member lines become redundant once those paths are tracked gitlinks (the index
  entry wins; gitignore is moot for a tracked path), but gwz does not clean them up.

## 5. Workstreams (TDD-first per AGENTS.md; RED before GREEN)

- **WS1 — RED contract tests.** Write the `sync_gitlinks` contract test **RED** in
  `src/git/tests/` (model on `stage_paths_matches_porcelain_git_add`,
  `src/git/tests/g01.rs:25`; reuse `TempDir` `g06.rs:10`, `run_git`, `ls_files_stage`).
  Assert byte-parity with `git update-index --cacheinfo 160000,<oid>,<path>` and the
  removal/guard semantics.
- **WS2 — the primitive.** Implement `sync_gitlinks` + `GitGitlinkResult` behind
  `GitBackend`; self-verify per §3; turn WS1 green. No `workspace_ops` change yet.
- **WS3 — the seam.** Add `sync_workspace_boundary` + `desired_gitlinks` +
  `ensure_gitignore_tmp`. Swap the four existing call sites; **add** the three
  missing ones (§4.3).
- **WS4 — ensure-tmp-line, retire the member-block writer.** Add `ensure_gitignore_tmp`
  and delete the dead dynamic gitignore producers (§4.4); no stripping. Update the
  existing assertion in `src/workspace_ops/tests/g00.rs:75` (currently checks
  `.gitignore` contains `/remote/` and is staged `A`) to assert instead that, for a
  freshly created workspace, `remote` is a staged `160000` entry at the recorded oid,
  `.gitignore` contains the `/gwz.conf/.tmp/` line (and no member block is written), and
  `untracked == 0`.
- **WS5 — behavior matrix** (§5.1) as integration tests; docs note in `GWZDesign.md`.

### 5.1 Behavior / parity matrix (from AD2's required boundary checklist)

Prove against real git, primitive-vs-porcelain where applicable:

| Scenario | Expected |
| --- | --- |
| Root `git status`, member clean at recorded oid | member shows clean (one unit), no recursion into member files |
| Member `HEAD` advanced past recorded oid | root shows member as `modified` (new commits) |
| Member worktree dirty at recorded oid | root shows member modified content (`-dirty`), parity with porcelain |
| Untracked file inside member | not surfaced at root (opaque boundary) |
| `git submodule status` at root | refuses/ignores — no `.gitmodules` mapping (submodule workflow absent by construction) |
| Member removed from manifest | its `160000` entry removed; no stale gitlink |
| `gwz status` when index oid, lock, and live `HEAD` disagree | reports drift per AD3; index is not treated as truth |
| Migration: existing `.gitignore`-block workspace | first mutating op writes gitlinks and ensures the tmp line; the legacy block is left as-is (now-redundant member lines), `untracked == 0` |
| `sync_gitlinks` re-run with same desired set | idempotent (`written` updates in place, `removed == 0`) |

## 6. The `gwz commit` Verb

**Intent.** A faithful multi-repo fan-out of `git commit` (AD3: gwz is a fan-out of
git, not an enforcer). `gwz commit -m "msg"` commits every selected repo that has
staged changes; `gwz commit -a -m "msg"` first stages tracked modifications (git's
`-a`) then commits. The **root workspace repo is included**, so a `gwz commit`
snapshots the workspace composition (manifest + lock + gitlinks) into root history
alongside the members — a git-native workspace history.

It is the first verb that *creates* member history, and it composes with the rest of
the model in a load-bearing order:

1. **Validate** (all selected repos + root, non-mutating). A repo is commit-able when:
   materialized; no unresolved merge/rebase conflicts; committer identity resolvable
   (`user.name`/`user.email`); and it has something to commit — staged entries
   (default) or staged-or-modified tracked entries (`-a`). A repo with nothing to
   commit is **skipped, not failed** (no empty commits). If any selected repo is hard-
   blocked (conflicts, missing identity), reject the whole batch before committing
   anything (Q2/Q6 preflight-all).
2. **Commit members** — run the commit primitive per commit-able member with the
   shared `-m` message (`-a` stages first).
3. **Observe → re-lock** — re-read each committed member's HEAD and update the lock.
   This *is* capture/`resolved_member` (`handle_create_repo.rs:371`) — reuse it;
   member commits advance HEADs, the lock must record the new oids.
4. **Sync gitlinks + commit root (last)** — the member commits in steps 2–3 have changed
   gwz's internal state (the lock now records new member HEADs; the gitlinks move to
   match). Refresh the root index gitlinks from the new lock (§3 / §4.2), stage
   `gwz.conf` (manifest + lock), then commit the root **last** with the same message —
   so every gwz-internal update caused by the member commits lands in that one root
   commit, and the committed gitlink oids match the just-recorded member HEADs (never
   stale).

Members commit **before** the root so the root snapshot pins post-commit member oids.

**Partial failure.** Cross-repo commit isn't atomic (same as `push`). Preflight-all
rejects a bad batch up front; a failure *during* the commit phase reports `Partial`
with per-member identity (Q6 reject-partial — gwz won't synthesize a `git reset`
rollback; un-committing is the deferred recovery-metadata question).

**Backend primitive — CLI-backed (AD1 fallback).** Commit is the canonical case for
AD1's per-primitive CLI fallback: libgit2's commit does **not** run
`pre-commit`/`commit-msg` hooks, honor `commit.gpgsign`, or apply `commit.template` /
user config. A fan-out commit that silently skipped a team's pre-commit linting or
signing would violate "porcelain-grade." **Implement the commit primitive via the
`git` CLI** (`git commit -m <msg>`, `-a` as requested), self-verifying afterward
(re-read HEAD: a new commit whose first parent is the prior HEAD; worktree/index
reflect it). Keeps hooks, signing, and config faithful.

**Protocol + CLI.** New `CommitRequest`/`CommitResponse`/`ActionKind::Commit` in
`gwz.taut.py` (regenerate — byte-parity corpus), mirroring how `capture` was added.
CLI verb `gwz commit [-a] -m <msg> [selection]`, honoring the standard member
selection (default: all members + root).

**Open choices (commit):**
- C1 — **Root inclusion default.** Include the root by default (per request); add
  `--no-root` later if a members-only commit is wanted.
- C2 — **One message for all.** A single `-m` fans out to every repo; per-repo
  messages are out of scope for v0.
- C3 — **`-a` matches git** — stages modified/deleted *tracked* files only, never
  untracked; no `gwz add` is implied here.
- C4 — **Root commit absorbs gwz-internal updates (ratified).** Whatever the member
  commits change in gwz's own state — the updated lock (new HEADs) and the refreshed
  gitlinks — is committed in the root commit, which is always done **last** (step 4).
- C5 — **Empty batch** — nothing commit-able anywhere → exit success with a
  "nothing to commit" aggregate, not an error.

**WS6 (commit).** After WS1–WS5 (step 4 needs the gitlink sync): RED contract test for
the CLI-backed `commit` primitive (hooks honored, self-verify) → primitive →
`handle_commit` (validate → commit members → re-lock → sync + root commit) → `taut` +
CLI verb. Reuses capture for step 3 and the §3 primitive for step 4.

## 7. Risks & Non-Goals

- **R1 — committing the root pins gitlink oids into history.** The stale-pointer
  concern only re-enters when the root is *committed*. Two paths: (a) the developer
  runs raw `git commit` at the root — out of gwz's control, document in
  `GWZDesign.md`; (b) the explicit `gwz commit` verb (§6) — which gwz makes *safe* by
  syncing gitlinks from the lock and committing members **before** the root, so the
  pinned oids match the just-recorded member HEADs and are never stale. gwz never
  commits the root *implicitly* (e.g. as a side effect of a boundary sync).
- **R2 — untracked-embedded-repo window.** A member dir is an untracked embedded repo
  only within a single gwz op — between materialize/create and the boundary sync that
  writes its gitlink — so it isn't user-visible. Pre-existing `.gitignore` member lines
  are left in place (redundant once the path is a tracked gitlink); gwz never edits them.
- **R3 — `gwz commit` partials can't be auto-undone.** Per Q6 there's no rollback of a
  committed member; the failure is reported with identity and the developer resolves
  it. Acceptable for v0 (same posture as `push`).
- **Non-goal — *implicit* committed gitlinks / `.gitmodules` / submodule workflow.**
  gwz never commits the root as a side effect (G1); committed gitlink history happens
  only via the explicit `gwz commit` verb (§6). No `.gitmodules`, no submodule
  workflow, no recovery metadata / journal (unchanged from Q6).

## 8. Sequencing

WS1 (RED) → WS2 (primitive GREEN) → WS3 (seam) → WS4 (static `.gitignore`, drop
wrapper) → WS5 (matrix + design note) → **WS6 (`gwz commit`, §6)** — last, since its
root-commit step depends on the gitlink sync. Each step ≤ ~500 LOC net, suite green +
clippy clean at every commit (AGENTS.md).
