# GWZ Audit Resolution Plan

Status: consolidated single plan — Review-55 R1 **and** R2 folded into the body
(no addendum). One decision table, one finding register, one workstream sequence.
Date: 2026-06-17
Sources:
- `dev-docs/GwzAudit.md` — the audit prompt (mutation-order safety)
- `dev-docs/GwzAudit-Response48.md` — independent audit (Opus 4.8)
- `dev-docs/GwzAudit-Response55.md` — independent audit (GPT-5.5 agents)
- `dev-docs/GwzAuditResolutionPlan-Review55.md` — review of this plan (R1)
- `dev-docs/GwzAuditResolutionPlan-Review55-2.md` — second review (R2)
- Disposition history: §7.

Starting state: the audit ran against `gwz-core` @ `40a1be1` / `gwz-cli` @
`f47a442`. **The FF fix (F0) is now committed** — `79d23c7` (fix + regression
test), then `ae5e9b2` (audit docs). So WS0 is done; the plan builds on `ae5e9b2`.

This is a plan, not an implementation. **Do not change code until the §2
decisions are made** (they are blocking, on paper, and reshape the work).

## 1. Root Cause And Guiding Principle

The confirmed fast-forward bug was one instance of a class, not an isolated defect:

> Durable state — Git refs, remote-tracking refs, worktrees, index, lock files,
> manifests, snapshots, tags, and the claims in responses/events — can be advanced
> or recorded **before** the worktree/index/member state is proven to match it.

The corrective discipline is to make every selected-member operation **phased**:

- **A. Validate** — non-mutating checks for *every* selected member; no member is
  mutated until the whole selection passes.
- **B. Mutate** — the Git/filesystem mutation per member, through a **semantic
  backend primitive** (never raw ref/index/worktree steps — see AD1).
- **C. Observe** — re-read `HEAD`/`status` from each mutated member.
- **D. Record** — build locks, manifests, and responses from the **observed**
  state (C), never from planned/intended state.

`handle_pull_head` is the closest existing model (it observes before writing the
lock) but is incomplete: it *fetches during Validate* and does not *reject a dirty
post-mutation state*. Both are fixed below (Q1, F6). The phased skeleton is
extracted into a shared helper that all handlers use — the natural seam for
splitting `workspace_ops/mod.rs` (§6).

## 2. Decisions To Make First (blocking)

Recommended answers given; override as needed, but answer them before §4. AD1/AD2
are architecture decisions; Q1–Q6 are policy.

**Ratified 2026-06-17:** AD1, AD2, Q6 (the ✅ rows + detail below) — this clears the
architecture gate, so WS-api / WS-contract / WS-backend may start. Q1–Q5 stand at
*recommended* and are non-blocking for the backend API shape; confirm them at their
workstreams or on request.

| # | Decision | Recommended answer |
| --- | --- | --- |
| AD1 ✅ | Mutating-Git strategy (revisits `GWZGitBackendDecision`) | **Ratified 2026-06-17.** libgit2 stays *behind a strict boundary*, each mutating primitive **contract-proven porcelain-grade** (detail below). Not "porcelain-only CLI" by fiat. |
| AD2 ✅ | Root/member boundary model | **Ratified 2026-06-17.** `gwz.yml` stays authoritative for membership; `.git/info/exclude` is the interim boundary. **Gitlink buys no consistency over the yml-recorded SHA** (recorded ≠ live; pinned oid goes stale/unreachable; pointer only moves on an explicit root commit — relocates churn, doesn't remove it) — it is a cleaner *boundary marker*, not a sync mechanism → gitlink stays a **deferred spike**, not the destination; sync yml→git representation on demand. Not "resync `.gitignore`". |
| AD3 ✅ | Workspace model — enforce vs capture/restore | **Ratified 2026-06-18** (capture built on it). gwz is a **developer-driven capture/restore tool, NOT an enforcer**: developers own their member repos and run arbitrary git directly; "out of sync" is the normal resting state, not an error. Two explicit, human-invoked directions — **capture** (worktree→record: `status`/`snapshot`/`tag`/`capture` observe live state) and **restore** (record→worktree: `materialize`/`pull`); gwz never silently forces the lock onto a tree. Recasts Q3. **All sub-questions resolved:** (a) capture-dirty, (b) carry-lock-for-unmaterialized, (c) materialize restores onto the saved branch / never silently resets one (`5674092`+`2d3c68a`), (d) `gwz capture` verb (`14985fd`+`ffeee75`). Detail below. |
| Q1 | Is `fetch` inside the atomic guarantee? | **Treat fetch as mutation.** Plan with non-mutating `ls-remote` (libgit2 `Remote::connect`+`list`); fetch only *after* the whole selection passes Validate; if a remote-tracking-ref advance persists after failure, report it as an explicit member outcome. (Removes F10; honors "failed = nothing changed". Does not force the CLI.) |
| Q2 | Is `push` atomic by default? | Cross-remote atomicity is impossible. Default = **preflight all** (remote exists, refspec resolves, optional dry-run) **then push**; real partial only under explicit policy, reported `Partial` with per-member identity. |
| Q3 | Do `snapshot`/`tag` capture the live worktree or the lock? | **Live observed** state (REQ-084 "current state") — done (F3 `60d034f`). **Recast by AD3:** *capture* dirty (record `commit=HEAD` + `dirty`, never reject) and *carry the lock state* for unmaterialized members (don't fail the snapshot). The earlier "reject dirty/unmaterialized" is withdrawn for capture ops. Never claim `lock_match: Matches` unverified. |
| Q4 | Must core honor `fetch-only`/`ff-only`/`merge`/`rebase`/`reset`? | **Yes.** `fetch-only` = no branch/worktree mutation; `ff-only` = today's default; `merge`/`rebase`/`reset` = implement or **reject loudly** — never silently downgrade. |
| Q5 | Does `--force materialize` allow a dirty end state? | No. Force permits *overwriting* a dirty/occupied start, but the member must end **clean at the target commit**, verified in phase C. |
| Q6 ✅ | Recovery metadata — **a schema question, not a path** | **Ratified 2026-06-17 — reject-partial for v0.** No recovery metadata: roll back this op where rollback is possible (local mutations — fresh clones/checkouts/worktrees), report explicit `Partial` with per-member identity where it isn't (`push` — can't un-push). Defers (b)–(e) — journal/record/event-log shape, `.gwz/` vs versioned, repair command, JSON/JSONL surface — until a real workflow needs *resume* over *redo*. `GWZDesign` defers persistent op logs, so **no ad-hoc `.gwz/recovery/<op>.yml`**. |

### AD1 — the backend primitive contract (makes "contract-proven" enforceable)

`GWZGitBackendDecision` is "accepted for v0: git2 behind a mandatory backend
boundary." The incident reopens it. The decision is not "abandon libgit2" — it is
to make the boundary *enforceable* and each mutating primitive *provable*:

- **No raw composition in operation code.** Operation code calls only *semantic*
  primitives; it never sequences ref/index/worktree steps and never names `git2`.
  Forbidden outside `src/git/*`: `git2::Repository`, `checkout_tree`,
  `set_target`, `set_head`, `set_head_detached`, raw index writes.
- **Each mutating primitive documents** its preconditions, the mutation it
  performs, the **final observed state**, and its failure state.
- **The primitive self-verifies:** it re-reads `HEAD`/index/worktree and confirms
  the intended final state *before returning success* (the phased "Observe" pushed
  down into the backend — exactly what the FF bug lacked).
- **Failure-injection tests are required**, not only happy-path comparisons with
  porcelain. Contract tests can prove specific success/failure behaviors; they
  can't prove crash safety or all interleavings — say so.
- **Per-primitive CLI fallback:** if a libgit2 primitive cannot meet this contract
  without hand-sequencing raw ref/index/worktree steps, *that primitive* falls
  back to the `git` CLI. (Keeps libgit2's `transfer_progress` callbacks and no-
  subprocess parallelism where it earns them; uses the CLI only where it must.)

### AD2 — root/member boundary (bare-gitlink is a spike)

| Option | Verdict |
| --- | --- |
| Committed `.gitignore` block | Current; leaky — dirties a versioned file, not a real boundary, duplicates membership. Not the long-term model. |
| `.git/info/exclude` | **Interim** — local, non-versioned, cheap, not thrown away by the eventual model. |
| **Bare gitlink** (mode 160000, no `.gitmodules`) | A **spike, not a recommendation.** Verified upside: root `git status`/`git add .` treat the member as one unit, and `git submodule` can't orchestrate it ("no mapping in `.gitmodules`") — so submodule-workflow sharpness is absent by construction. Verified cost: it duplicates the *volatile* commit oid, so a root `git commit` can pin a **stale** pointer — a fresh instance of the audit's own class. Large enough that gitlink is **not implied as the destination**. |
| External checkout root | Avoids nested repos; changes UX/path assumptions. |

The bare-gitlink spike must answer, **before** any implementation, whether it is a
*committed* projection, an *index-only* projection, or *rejected for v0* — and
prove behavior across normal Git workflows:
- Root `git status` with clean member / dirty member / untracked inside member /
  member `HEAD` ≠ `gwz.lock.yml`.
- Root `git add .` / `reset` / `clean -fd` / `checkout` / `switch` / `merge`; clone
  + checkout of root history containing gitlinks.
- Index-only vs committed gitlink tree entries; branch switch where the gitlink oid
  changes but the member worktree has local changes.
- `gwz status` when root gitlink, lock, and live member `HEAD` disagree.
- `ignoreSubmodules` + user global git config; ordinary tools seeing gitlinks
  without `.gitmodules`.

Do **not** implement gitlink as "same pattern as `.gitignore`" until that
projection-mode question is decided.

**Ratified 2026-06-17:** `gwz.yml` is authoritative for membership; the git
representation is a *projection synced on demand*, not the master. Gitlink's only
real gain over `.gitignore` is treating a member as one boundary unit in root
`git status`/`add` — it adds **no** consistency the yml SHA lacks (recorded ≠ live;
the pinned oid can go stale/unreachable; the pointer only moves on an explicit root
commit, which merely *relocates* the churn). So gitlink stays a **deferred spike**;
`.git/info/exclude` is the interim, `gwz.yml` the master.

### AD3 — workspace model: developer-driven capture/restore (proposed)

gwz is **not an enforcer**. Developers own their member repos and run arbitrary git
(branch, commit, rebase, checkout) directly; a member drifting from the lock is the
*normal resting state*, not an error. gwz exposes two explicit, human-invoked
directions and never silently reconciles one onto the other:

- **Capture** (worktree → record): `status` reports drift; `snapshot`/`tag` freeze the
  current *observed* multi-repo state as a named, restorable point. (F3 `60d034f` made
  these observe live committed state — commit/branch/detached/dirty.)
- **Restore** (record → worktree): `materialize`/`pull` rebuild worktrees from a saved
  point — on explicit request only.

Sub-questions to ratify:
- **(a) Dirty/uncommitted — capture, never reject.** Record `commit=HEAD` + `dirty`;
  note that uncommitted changes are not captured (git can't address a non-commit), so
  restore won't reproduce them. Withdraws Q3's "reject dirty." *(snapshot already does
  not reject dirty — this just ratifies it + adds the note.)*
- **(b) Unmaterialized in snapshot — carry the lock, don't fail.** Capture present
  members; for an absent member, carry its last lock state so the snapshot stays
  complete/restorable. **Reverses the unmaterialized rejection F3 added**
  (`observed_member_map`) — the one concrete snapshot code change AD3 implies.
- **(c) `materialize` restore UX — done (`5674092`+`2d3c68a`).** Restores onto the
  saved branch when safe (creatable, or already at the target); **never silently resets
  a branch** — when the branch has diverged it detaches at the target instead, preserving
  the developer's commits. New self-verifying `checkout_branch` primitive (refuses a
  diverged reset, porcelain-contract-tested) + materialize's catch-`DivergedMember`→detach
  fallback.
- **(d) A first-class capture op?** `snapshot`/`tag` write *named* artifacts and leave
  the lock untouched; only `materialize`/`pull` (which mutate the worktree) move the
  lock. Decide whether to add a pure `gwz capture`/`adopt` (worktree → lock, no
  mutation) so "I diverged, record it now" is one command, not snapshot-then-restore.

AD3 is **upstream of** F5 (status dirty surfacing) and the Q3/Q4/Q5 worktree policies —
ratify it before those. No code yet; the only near-term change it forces is (b).

### `gwz capture` — implemented (AD3 sub-question d)

**Status: shipped** — gwz-core `14985fd` (protocol via `tautc` + `handle_capture` +
AD3(b) `observed_member_map`), gwz-cli `ffeee75` (the `capture` verb). The open
choices below resolved to: name = **`capture`**; **first-class op** (new
`CaptureRequest`/`CaptureResponse`/`ActionKind::Capture` in `gwz.taut.py`, regenerated
— byte-parity corpus green), not a snapshot flag.

**Intent.** The everyday *capture* verb: observe the current worktrees and write the
**lock** from them, mutating nothing. "I diverged on purpose — rebased / switched
branches / committed — record that as the workspace's current state." Resolves the
`status` "out of sync" without a restore.

**Semantics.**
- Read the manifest (membership; `gwz.yml` authoritative per AD2).
- For each selected member, observe `head` + `status` → `resolved_member` — the same
  observe path F3 gave `snapshot`.
- Write the **lock** from the observed states (merge per selected member; unselected
  members keep their lock entry). **No worktree mutation** — read-only on the trees.
- Per AD3(a): record dirty members (`commit=HEAD` + `dirty`), never reject; **warn**
  that uncommitted changes aren't captured and won't restore.
- Per AD3(b): for an unmaterialized selected member, carry its existing lock entry,
  don't fail.

**It is `snapshot` retargeted from a named artifact to the lock** — same observe
machinery, only the write target differs. The verb set then reads cleanly:

| verb | direction | target |
| --- | --- | --- |
| `status` | read | — (reports worktree-vs-lock drift) |
| **`capture`** (new) | **capture** | the **lock** (current pointer) |
| `snapshot` / `tag` | capture | a **named** restorable point |
| `materialize` / `pull` | restore | worktree, from lock/snapshot/tag |

**Open choices:**
1. **Name** — `capture` vs `adopt` / `record` / `relock`. `snapshot` already means a
   *named* capture, so a distinct verb for the *lock* capture avoids overloading
   "capture"; `adopt` ("adopt current reality as the record") reads well too.
2. **New op vs snapshot-mode** — either a first-class `CaptureRequest` /
   `ActionKind::Capture` (clean intent, +`taut`/protocol surface) or a
   `snapshot --to-lock` flag (reuses `SnapshotRequest`, less surface). Lean: a
   first-class verb — moving the current pointer vs saving a named point is a genuinely
   different intent, and the surface mirrors snapshot.
3. **Safe + idempotent** — two captures in a row = same lock; pure capture can't orphan
   anything (no worktree change), so it's the safe counterpart to restore (the orphan
   risk lives entirely on the restore side — AD3(c)).

**Cost.** Small: reuse `observed_member_map` + `write_lock`; +one request type and CLI
verb. The `taut` protocol addition is the only non-trivial bit.

## 3. Findings Register

**Resolved 2026-06-17:** F0 (incident + full AD1 primitive contract `f16f258`), F18
(`57c68f5`), F7 (`8ed50cb`), F2 (`baf912a`), F1 (`b5744d2`) — ✓ below. AD1/AD2/Q6
ratified (`a024907`); `ls_remote` added as the Q1 plan-before-fetch foundation
(`d9b1d68`); all four src god files split (WS-split-cli/core done). F4/F10 are
**superseded** by §2 decisions and no longer drive standalone work. "WS" maps to §4.

| ID | Sev | Finding | File:line | WS |
| --- | --- | --- | --- | --- |
| F0 | P0 ✓ | FF incident fixed `79d23c7`; primitive now self-verifies + porcelain-contract-tested (`fast_forward`/`checkout_commit`) `f16f258` | git/gitbackend.rs | WS0 ✓ |
| F1 | P1 ✓ | `materialize` re-observes head/status post-mutation, records observed state + computed `lock_match` `b5744d2`; `clone_workspace`/`pull_snapshot` inherit via delegation | handle_materialize.rs | WS4 ✓ |
| F2 | P1 ✓ | `materialize`/`init` roll back this op's **fresh clones** on mid-batch failure, no stale lock `baf912a` (Q6 reject-partial; existing-member re-checkout rollback deferred) | handle_materialize.rs, handle_init_from_sources.rs | WS3 ✓ |
| F3 | P1 ✓ | `snapshot`/`tag` now take a `GitBackend` and capture each member's **live observed** head/status (`observed_member_map`); unmaterialized rejected, dirty recorded honestly `60d034f` (+cli `2f7ac72`; Q3 reject-dirty deferred) | handle_materialize.rs | WS4 ✓ |
| F4 | ~~P1~~ → AD2 | `.gitignore` not resynced on materialize/pull/clone | workspace_ops:504,631,681 | **superseded by AD2** |
| F5 | P1 ✓ | `status` aggregate now surfaces dirty — new `AggregateStatus::Dirty` (taut) returned when any member's `git_status.dirty`; exit 0, dirty is normal per AD3 `7e4d1df`+`50948ac`. (Lock-divergence already carried by `lock_match: Differs`; branch-vs-upstream divergence deferred.) | status_member.rs | WS4 ✓ |
| F6 | P1 ✓ | Closed by existing mechanisms — preflight rejects dirty members before any FF (`DirtyMember`, tested by g06) and `fast_forward` self-verify (`f16f258`) makes a dirty-post-FF state unreachable for FF'd members. Local-only members are Noop (not pulled); recording their dirty state is a Q3 question, not F6. No code change. | pull_head_member_preflight.rs | WS3 ✓ |
| F7 | P1 ✓ | `push` **preflights all members** (remote/refspec/materialization) before pushing any; rejects the batch if any invalid, no remote advanced `8ed50cb` (Q2) | push_member.rs | WS3 ✓ |
| F8 | P1 ✓ | **all sync styles implemented** (S1–S6 `97319ee`..`98318da`): planner now routes `policy.sync` — fetch-only / ff-only / merge / rebase / reset — instead of always FF. Decision (Gianni): gwz is a faithful multi-repo fan-out of git, NOT an enforcer; refusing merge/rebase/reset is the hand-holding AD3 rejects. Backend primitives `merge_upstream`/`rebase_onto`/`reset_hard` (porcelain-parity, clean+conflict, self-verifying); conflicts surface as `MemberStatus::Conflicted`/`AggregateStatus::Conflicted` (exit 1), worktree left `--continue`-able; reset gated on `policy.destructive`; fetch-only allows dirty. | gitbackend.rs, pull_head_member_preflight.rs | WS6 ✓ |
| F9 | P1 ✓ | top-level CLI errors render envelope-consistent JSON under `--json`/`--jsonl` (`CliError` carries the gwz-core code); human/porcelain unchanged on stderr `e4a43ce` (gwz-cli) | gwz-cli clirequest/globalargs | WS7 ✓ |
| F10 | ~~P2~~ → Q1 | `pull --head` fetches during preflight, advancing remote-tracking refs | workspace_ops:1710 | **superseded by Q1** |
| F11 | P2 ✓ | `lock_match` now means provably-equal-to-lock: clean worktree at the locked commit, branch, AND attachment; any divergence (dirty included) → `Differs` `0467aa7` | member_not_materialized.rs | WS4 ✓ |
| F12 | P2 ✓ | `write_atomic` fsyncs the temp before rename (+ best-effort dir fsync) and uses a unique per-process temp name (no fixed-`.tmp` race); cleans up on error `e1413a6` | artifact/mod.rs | WS8 ✓ |
| F13 | P2 ✓ | `snapshot` rejects a duplicate id up front (the guard `tag` had) → `InvalidRequest` `2050cb4` | handle_materialize.rs | WS8 ✓ |
| F14 | P2 ✓ | manifest+lock written through one seam `write_manifest_and_lock` — stage both durably, publish lock-last (crash ⇒ at worst a stale, rebuildable lock); all 4 call sites converted `e4389eb`. True cross-file atomicity (journal) deferred — recoverable window only. | artifact/mod.rs | WS8 ✓ |
| F15 | P2 ✓ | runtime `aggregate_status` gains `Partial` (applied+failed mix); `OperationError` carries member identity, propagated + filled from the execution `5dc8fd5` | aggregate_status.rs | WS5 ✓ |
| F16 | P2 ✓ | human `status --no-files` surfaces a per-member/root dirty **count** summary (counts are first-class, independent of the file list) `05497b1` | append_branch_summary.rs (cli) | WS7 ✓ |
| F17 | P2 ✓ | rename detection enabled in `StatusOptions`; fixed the modeling directionality (path=new, original_path=old) `66bb124` | gitbackend.rs | WS9 ✓ |
| F18 | P1 ✓ | `stage_workspace_git_metadata` moved behind `GitBackend::stage_paths` (self-verifying, porcelain-contract-tested); production `workspace_ops` no longer names `git2` `57c68f5` | git/gitbackend.rs | WS-backend ✓ |

## 4. Remediation Workstreams

Test-first per `AGENTS.md`; ~≤500 LOC net per step.

- **WS0 — baseline.** *Done* (`79d23c7`).
- **WS-decide — the §2 decisions** (AD1/AD2/Q1–Q6, paper). Blocking for everything
  below.
- **WS-api — define the semantic backend API**, including the **root
  metadata/boundary staging** primitive (the one F18 needs and AD2 boundary tests
  exercise). This is where AD1's contract shape is written down.
- **WS-contract — real-Git backend contract suite** (RED first). Disposable real
  repos prove each mutating primitive matches porcelain — what would have caught
  the incident: add/delete/rename/nested-file FF; dirty-target rejection;
  interrupted-checkout; `fetch-only` leaves `HEAD`/worktree unchanged; primitive
  self-verifies final state; root/member boundary under real `git status`/`add .`/
  `clean -fd`/`checkout`/`pull`. Plus failure-injection, not only happy-path.
- **WS-backend — implement F18** (and any AD1 primitives) behind `GitBackend`;
  move root staging out of operation code; make WS-contract green. F18 is the first
  concrete application of AD1, not a separate prerequisite.
- **WS-split-cli — split `gwz-cli/src/main.rs`** (architecture-independent; may
  proceed now). Acceptance gate in §5.
- **WS-split-core — split `workspace_ops/mod.rs`** along operation seams, *after*
  AD1/AD2 settle its helper modules.
- **WS3 — the phased refactor** (Validate→Mutate→Observe→Record helper) onto the
  now-split, contract-backed handlers:
  - `pull --head`: `ls-remote` plan (Q1, removes F10); Validate all; after FF
    **reject dirty observed state** (F6).
  - `push`: Validate phase (remote/refspec) before any push; partial only under
    explicit policy (F7, Q2).
  - `materialize`/`init`: complete Validate for all before any clone/checkout; on
    mid-batch failure, roll back this op's fresh clones or emit explicit partial
    per Q6 (F2).
- **WS4 — record observed state** (F1,F3,F5,F11): locks/responses from re-read
  `head`/`status`; thread a backend into `snapshot`/`tag` (Q3); derive
  `MemberStatus`/`aggregate_status` from dirty/diverged; extend `lock_match` to
  branch/detached/upstream.
- **WS5 — partial/recovery** (F2,F7,F15): `AggregateStatus::Partial` + per-member
  error identity; recovery metadata **only per the Q6 schema decision** (or reject
  partial until designed).
- **WS6 — policy enforcement** (F8): core honors the sync modes (Q4); `--force`
  per Q5.
- **WS7 — CLI safety** (F9,F16): structured `--json`/`--jsonl` error records;
  `status --no-files` still shows dirty counts + aggregate.
- **WS8 — artifact hardening** (F12,F13,F14, and the AD2 boundary mechanism that
  replaces F4): snapshot duplicate-ID guard; semantic manifest+lock atomicity;
  `write_atomic` fsync + unique temp; advisory workspace lock.
- **WS9 — rename decision** (F17): enable + test rename detection, or document it
  off.

## 5. Test Plan

### Tree-shape / behavior matrix (acceptance checklist)

| Scenario | Now | Required |
| --- | --- | --- |
| Existing-file modification | baseline | keep |
| Added top-level / nested file (FF/pull) | partial | backend + workspace + CLI |
| Deleted / renamed tracked file (incl. nested) | missing | add, or document rename-off |
| Checkout/FF returns Ok but dirty | missing | fake-backend reject + lock unchanged |
| Primitive self-verifies final HEAD/index/worktree | missing | per AD1, contract test |
| Dirty-target rejection (untracked/nested/delete/rename) | partial | extend |
| Post-op clean status + lock `dirty:false` | partial | assert across ops |
| Partial multi-member failure (pull/materialize/push) | inconsistent | two-member + recovery/partial per Q6 |
| `--sync fetch-only` no local mutation | missing | add |
| Structured JSON/JSONL error records | missing | add |
| Duplicate snapshot ID rejected | missing | add |
| **F18:** no production module outside `src/git/*` names `git2`; root staging is a backend op | missing | add (lint + contract) |
| **AD2 boundary** under real `git status`/`add .`/`clean -fd`/`checkout` | missing | contract, for the chosen interim + final mode |

### `main.rs` split acceptance gate

The CLI split must be a **behavior-preserving move only** — it must not absorb
audit fixes or output changes:
- no behavior changes; CLI unit + integration tests green; output golden tests
  unchanged; no policy-parsing changes; **no drive-by JSON/JSONL error-path
  changes** (that is WS7, gated by Q1/AD-nothing — keep it separate).

## 6. Sequencing

1. **WS0** — commit the FF fix. *(done, `79d23c7`)*
2. **WS-split-cli** — split `main.rs` now (architecture-independent), under the §5
   acceptance gate. Keeps the degodify moving without waiting on the decisions.
3. **WS-decide** — AD1 / AD2 / Q6 **ratified 2026-06-17** (architecture gate cleared).
   Q1–Q5 remain at *recommended* (policy; confirm at their workstreams — non-blocking
   for WS-api / WS-contract / WS-backend).
4. **WS-api** — define the semantic backend API incl. root metadata/boundary
   staging.
5. **WS-contract** — write the real-Git contract tests **RED** against that API.
6. **WS-backend (F18)** — implement the primitives behind `GitBackend`; make the
   contract suite **green**.
7. **WS-split-core** — split `workspace_ops` along the now architecture-stable
   seams.
8. **WS3 → WS9** — phased ops + behavioral fixes in the now-small modules.

Why this order: contract tests are written RED against the *intended* backend API
(step 5), then F18 implements that API and turns them green (step 6) — so
"contract tests before F18" is deliberate, not a contradiction. The
`workspace_ops` split waits on AD1/AD2 because those reshape its helper modules;
`main.rs` does not, so it goes first. (Earlier drafts said "split everything
first"; that held for the *behavioral* fixes but wrongly assumed the
backend/boundary architecture was settled — it is now a §2 decision.)

## 7. Review History & Dispositions

- **Audit consolidation** (Response48 + Response55): agreed on F1/F2/F3/F5/F12/F14;
  Response48 unique — init orphan-clones (F2), F4, F11; Response55 unique — F6, F7,
  F8, F9, F10, F13, F15, F16, F17. On cross-check, Response48's "push clean" and
  "CLI JSON clean" were wrong (→ F7, F9). Net: no remaining P0 after WS0.
- **Review-55 R1** (review of this plan): accepted — no-git-internals as a blocking
  gate (AD1), `.gitignore`-is-a-design-question (AD2), gitlink-as-boundary spike,
  direct `git2` in `workspace_ops` (F18), fetch-is-mutation (Q1 recast), recovery
  needs a schema (Q6), real-Git contract tests (WS-contract). Pushed back on
  porcelain-only-by-fiat → AD1 "prove, don't presume."
- **Review-55 R2** (second review): accepted — the addendum left two competing
  plans, so §11 is now folded into the body (this rewrite); F18 added to the
  register + matrix + a workstream; AD1 contract sharpened (self-verify, failure-
  injection, forbidden-ops, per-primitive CLI fallback); gitlink kept firmly a
  spike with a workflow matrix + the committed/index-only/rejected question; Q6
  recast as a schema question; sequencing clarified (RED contract tests → F18 →
  green); `main.rs` split acceptance gate added; stale starting-state / F0 fixed
  (committed at `79d23c7`).
- **Decision ratification** (2026-06-17): AD1, AD2, Q6 ratified, clearing the §2
  architecture gate → WS-api / WS-contract / WS-backend (F18) unblocked. AD1 —
  libgit2 behind a strict boundary, each mutating primitive contract-proven, per-
  primitive CLI fallback. AD2 — `gwz.yml` authoritative; `.git/info/exclude` interim;
  gitlink a deferred spike that buys no consistency over the recorded SHA (boundary
  marker, not sync). Q6 — reject-partial for v0: roll back where possible, explicit
  `Partial` for `push`, no recovery metadata. Q1–Q5 still at *recommended*.
- **Implementation pass** (2026-06-17): all four `gwz-core` src god files split
  (`workspace_ops` `0e352a0`, `operation` `3659913`, `status` `dd07720`, `git`
  `86ef63d`); F18 `stage_paths` (`57c68f5`); F0 primitive AD1 contract — `fast_forward`
  + `checkout_commit` self-verify + porcelain contract tests (`f16f258`); `ls_remote`
  Q1 foundation (`d9b1d68`); F7 push preflight (`8ed50cb`); F2 fresh-clone rollback
  (`baf912a`); F1 materialize observed-state (`b5744d2`); F3 snapshot/tag observe live state
  (`60d034f` + cli `2f7ac72`). Full suite green (95 lib +
  16 integration), 0 warnings, clippy clean throughout. F6 closed by existing mechanisms (no code).
  **All P1s are now done.** F8 (the last) is implemented in full (S1–S6, `97319ee`..`98318da`): the
  decision flipped from "reject merge/rebase/reset" to **implement every sync style** — gwz is a faithful
  multi-repo fan-out of git, not an enforcer (refusing developer operations is the hand-holding AD3 rejects).
  Conflicts are an expected outcome (`Conflicted` status, `--continue`-able), not a failure. F5/F9 done.
  AD3 ratified + fully implemented (capture/restore, `gwz capture`, branch-restore).
  **All P2s (F11–F17) are now done too** (`0467aa7`,`e1413a6`,`2050cb4`,`e4389eb`,`5dc8fd5`,`05497b1`,`66bb124`;
  111 lib tests, 0 warnings, clippy clean). **Every audit finding F0–F18 is resolved.** What remains is
  optional follow-up, not findings: **Q1** (`ls_remote` is built but unwired — pull still fetches during
  preflight; wiring it = plan-before-fetch so a rejected batch leaves no ref advance), F14's true-atomicity
  journal (deferred; current window is recoverable), and the small deferred sub-items (Q3 reject-dirty,
  F2 existing-member re-checkout rollback, AD2 gitlink spike).
