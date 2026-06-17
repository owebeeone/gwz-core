# GWZ Audit Resolution Plan

Status: draft plan (consolidation of two independent audits)
Date: 2026-06-17
Sources:
- `dev-docs/GwzAudit.md` — the audit prompt (mutation-order safety)
- `dev-docs/GwzAudit-Response48.md` — independent review (Opus 4.8)
- `dev-docs/GwzAudit-Response55.md` — independent review (agents: Pascal / Nietzsche / Nash / Socrates)

Starting state (both reviews, unchanged):
- `gwz-core` @ `40a1be1` (main), working tree dirty: `src/git/mod.rs` (uncommitted fast-forward fix)
- `gwz-cli` @ `f47a442` (main), clean

This plan merges, de-duplicates, and reconciles the two reviews into an ordered,
test-first remediation. Where the reviews diverged on severity it says so and
takes a position. It is a plan, not an implementation — do not change code until
the policy decisions in §2 are made.

## 1. Root Cause And Guiding Principle

Both reviews independently reached the same conclusion: the confirmed
fast-forward bug was one instance of a class, not an isolated defect.

> Durable state — Git refs, remote-tracking refs, worktrees, index, lock files,
> manifests, snapshots, tags, and the claims in responses/events — can be
> advanced or recorded **before** the worktree/index/member state has been
> proven to match it.

The corrective discipline (both reviews agree) is to make every selected-member
operation **phased**:

- **A. Validate** — non-mutating checks for *every* selected member. No member
  is mutated until the whole selection passes.
- **B. Mutate** — perform the Git/filesystem mutation per member.
- **C. Observe** — re-read `HEAD` and `status` from each mutated member.
- **D. Record** — build lock files, manifests, and responses from the
  **observed** state (C), never from planned/intended state.

`handle_pull_head` is the closest existing model (it observes before writing the
lock) but is itself incomplete (it fetches during Validate, and does not reject a
dirty post-mutation state). This skeleton should be **extracted into a shared
helper** that all selected-member handlers use — which is also the natural seam
for splitting the 4175-LOC `workspace_ops/mod.rs` god file (see §6).

## 2. Policy Decisions To Make First (blocking)

Several fixes are under-determined until these are decided. Recommended answers
are given; override as needed, but answer them before §4 implementation.

| # | Question | Recommended answer |
| --- | --- | --- |
| Q1 | Does the atomic guarantee cover **remote-tracking refs** advanced by `fetch`? | **No** (they are derived/re-fetchable). But run a single **fetch phase for all members before Validate**, so fetch is not interleaved with per-member aborts and Validate sees accurate remote state. |
| Q2 | Is `push` **atomic by default**? | True cross-remote atomicity is impossible. Default = **preflight all members** (remote exists, refspec resolves, optional dry-run) **then push**; real partial only under an explicit partial policy, reported as `Partial` with per-member identity. |
| Q3 | Do `snapshot`/`tag` capture the **live worktree** or the **lock**? | **Live observed** state (matches REQ-084 "current state"). Reject dirty/unmaterialized members unless an explicit flag records them with a dirty marker. Never claim `lock_match: Matches` without verifying. |
| Q4 | What do `fetch-only` / `ff-only` / `merge` / `rebase` / `reset` mean, and must core honor them? | Core **must** honor them. `fetch-only` = update remote-tracking refs only, **no** branch/worktree mutation. `ff-only` = today's default. `merge`/`rebase`/`reset` = implement, or **reject loudly** — never silently downgrade to ff-only. |
| Q5 | Does `--force materialize` allow a **dirty end state**? | No. Force permits **overwriting** a dirty/occupied start, but the member must end **clean at the target commit**, verified in phase C. |
| Q6 | What **recovery metadata** exists when partial mutation is allowed? | A `.gwz/recovery/<operation_id>.yml` listing each member's terminal state (mutated/observed), plus `AggregateStatus::Partial` and per-member status in the response. |

## 3. Consolidated Findings Register

Severity is the reconciled view. "Source" shows which review(s) raised it; "WS"
maps to the workstream in §4. The confirmed P0 is fixed but **uncommitted**.

| ID | Sev | Finding | Source | File:line | WS |
| --- | --- | --- | --- | --- | --- |
| F0 | P0✓fixed | FF moved ref before checkout (incident) — fixed, not committed | both | git/mod.rs:219-229 (dirty) | WS0 |
| F1 | P1 | `materialize`/`pull_snapshot` write lock+response from **planned** target, never re-observe worktree | both | workspace_ops:594-616 | WS4 |
| F2 | P1 | `materialize`/`init` **partial mutation**: members mutate, first `outcome?` aborts before lock write; stale lock / orphan clones; no recovery | both | workspace_ops:562-616, 316-406 | WS3,WS5 |
| F3 | P1 | `snapshot`/`tag` capture **stale lock**, not live worktree (no backend) | both | workspace_ops:421-437, 465-479 | WS4 |
| F4 | P1 | `.gitignore` not resynced by `materialize`/`pull_snapshot`/`clone_workspace` → member paths show untracked | 48 | workspace_ops:504,631,681 | WS8 |
| F5 | P1 | `status` reports `MemberStatus::Ok` + `aggregate::Ok` for a **dirty/diverged** member | both | status:312, 630-643 | WS4 |
| F6 | P1 | `pull --head` does **not reject dirty post-fast-forward** state before writing the success lock | 55 | workspace_ops:787 | WS3 |
| F7 | P1 | `push` has **no preflight-before-mutation**: validates+pushes in one pass → partial remote advance under default atomic | 55 (48 conceded) | workspace_ops:855-911, 1899, 2010 | WS3 |
| F8 | P1 | `--sync fetch-only` (and merge/rebase/reset) **accepted but ignored** by core; pull still fast-forwards | 55 (verified) | cli:570,1983; workspace_ops:775-787 | WS6 |
| F9 | P1 | CLI `--json`/`--jsonl` **error path** prints plain stderr, not structured output (machine consumers lose detail) | 55 (verified) | cli:194-209, 1019 | WS7 |
| F10 | P2 | `pull --head` **fetches during preflight** → clean member's remote-tracking refs advance even if another member aborts | 55 (48: P2) | workspace_ops:1710 | WS3,WS6 |
| F11 | P2 | `lock_match` ignores branch/detached/upstream; returns `Matches` for a dirty-but-matching member | 48 | status:556 | WS4 |
| F12 | P2 | `write_atomic` rename-atomic but **not crash-durable** (no fsync) and fixed `{name}.tmp` (cross-process race) | both | artifact:335-343, 381-387 | WS8 |
| F13 | P2 | `snapshot` lacks the **duplicate-ID guard** that `tag` has | 55 (verified) | workspace_ops:414-446 vs 458 | WS8 |
| F14 | P2 | manifest→lock→gitignore writes **not semantically atomic** (first-write-ahead on later failure) | both | workspace_ops:134,141,227,403-406 | WS8 |
| F15 | P2 | Generic runtime `aggregate_status` has **no `Partial`**; top-level errors lose `member_id`/`member_path` | 55 | operation:768,834 | WS5 |
| F16 | P2 | Human `status --no-files` can **hide dirty** state (only branch text) | 55 | cli:1063,1238; status:326 | WS7 |
| F17 | P2 | `status` models renames but **rename detection not enabled/tested** | 55 | git/mod.rs:253 | WS9 |

## 4. Remediation Workstreams (ordered)

Each workstream is test-first (write the red test, then the fix) per
`AGENTS.md`, and sized to stay reviewable (~≤500 LOC net per step).

### WS0 — Land the baseline (do now)
The confirmed P0 fix is only in the working tree. Commit `git/mod.rs` (the
`checkout_tree`-before-ref reorder + the nested-new-file regression test) so the
fix is durable and the rest of the plan builds on a known-good base.

### WS1 — Decide the policy matrix (§2)
Blocking for WS3, WS4, WS6. Write the decisions into this doc (or a
`GwzPolicy.md`) so the phased refactor has a spec.

### WS2 — Red tests for the risk class (before fixes)
Land these as failing tests that pin the invariants, then make them pass in WS3–WS5:
- Fake backend: `fast_forward`/`checkout_commit` return `Ok` but `status()` is
  dirty → `pull`/`materialize` must error and leave the lock unchanged (F1,F6).
- Two-member partial: member 1 succeeds, member 2 fails after preflight →
  pin behavior for `pull`/`materialize`/`push` (no mutation, or explicit partial
  + recovery) (F2,F7).
- `--sync fetch-only`: member at A, remote at B → `HEAD` stays A (F8).
- Structured JSON/JSONL error record on a failing `--jsonl pull` (F9).

### WS3 — The phased-operation refactor (core of the plan)
Extract the **Validate → Mutate → Observe → Record** skeleton into a shared
helper and convert each selected-member handler onto it:
- **pull --head**: move `fetch` to a pre-Validate fetch phase (F10); Validate all
  before mutating; after FF, **reject dirty observed state** before recording (F6).
- **push**: add a Validate phase (remote exists, refspec resolves) for all
  members before any `backend.push`; partial only under explicit policy (F7).
- **materialize / init**: complete Validate (target availability + commit
  reachability for *all* members) before any clone/checkout; on mid-batch
  failure, roll back this op's fresh clones or emit explicit partial + recovery (F2).
By now `workspace_ops` is already split by operation seam (§6); this lands the
shared phase helper and the per-handler fixes in those now-small modules.

### WS4 — Record observed state everywhere (F1,F3,F5,F11)
- `materialize`/`pull_snapshot`/`clone_workspace`: build lock entries and member
  responses from re-read `head`/`status` (the pull pattern), not `target_lock`.
- `snapshot`/`tag`: thread a backend, verify live state per Q3.
- `status`: derive `MemberStatus`/`aggregate_status` from observed dirty/diverged;
  extend `lock_match` to branch/detached/upstream and stop reporting `Matches` for
  a dirty member.

### WS5 — Partial-success & recovery modeling (F2,F7,F15)
- Add `AggregateStatus::Partial` to the generic runtime aggregation; preserve
  per-member error identity (no anonymous top-level errors).
- Define and write the `.gwz/recovery/<op>.yml` metadata (Q6) on partial mutation.

### WS6 — Policy enforcement (F8,F10)
- Core honors the sync modes (Q4): `fetch-only` performs no branch/worktree
  mutation; unimplemented modes reject loudly.
- `--force` materialize per Q5 (overwrite-but-end-clean).

### WS7 — CLI machine-output & human safety (F9,F16)
- `--json`/`--jsonl`: emit a structured terminal error record (aggregate failure
  + error code + `member_id`/`member_path` when known) on the error path.
- `status --no-files`: still print dirty counts/summary; surface the aggregate.

### WS8 — Artifact hardening (F4,F12,F13,F14)
- `snapshot` duplicate-ID guard (parity with `tag`).
- Resync `.gitignore` at the end of `materialize`/`pull_snapshot`/`clone_workspace`.
- Semantic manifest+lock atomicity: write lock-first or roll back the manifest on
  lock-write failure.
- `write_atomic`: `sync_all` + parent-dir fsync; unique temp name
  (`{name}.{pid}.{nonce}.tmp`); consider an advisory workspace lock.

### WS9 — Tree-shape test matrix & rename decision (F17, §5)
Fill the matrix below across backend + workspace + CLI; decide whether rename
detection is enabled (then test it) or explicitly unsupported (document it).

## 5. Test Matrix (acceptance checklist)

Merged from both reviews. Each row should be covered at the backend layer and,
where it changes user-visible state, at the workspace + CLI layer.

| Scenario | Now | Required |
| --- | --- | --- |
| Existing-file modification | baseline | keep |
| Added top-level file (FF/pull) | missing | add |
| Added nested file | backend-only (dirty WT) | extend to workspace + CLI |
| Deleted tracked file | missing | add backend/workspace/CLI |
| Renamed tracked file (incl. nested) | missing | add, or document rename-detection-off |
| Checkout/FF returns Ok but dirty | missing | fake-backend reject + lock unchanged |
| Dirty-target rejection (untracked/nested/delete/rename forms) | partial | extend |
| Post-op clean status + lock `dirty:false` | partial (backend) | assert across ops |
| Partial multi-member failure (pull/materialize/push) | partial/inconsistent | two-member tests + recovery |
| `--sync fetch-only` no local mutation | missing | add |
| Structured JSON/JSONL error records | missing | add |
| Duplicate snapshot ID rejected | missing | add (parity with tag) |

## 6. Sequencing & The God-File Split

- **WS0 first** (commit the FF fix) — the whole plan rests on it.
- **Degodify before fixing (decided).** Split `workspace_ops/mod.rs` (4175 LOC)
  and `gwz-cli/src/main.rs` (2746 LOC) as a **pure, byte-identical move** on
  today's green code, *before* the behavioral audit fixes. Three reasons: (1) the
  move is verifiable byte-identical-green now — the ideal condition for the split;
  you forfeit that proof the moment behavior changes in the same pass; (2) the
  seams are stable — the audit changes handler *internals* and adds a shared phase
  helper, it does not move handlers between files, so an operation-seam split is
  not re-cut (even if the handlers later collapse onto a generic driver, the
  per-op files just thin out); (3) the delicate safety fixes are far safer to make
  and review in <500-LOC modules than buried in a 4175-LOC file.
- **Split by operation seam**, not line-cuts: pull / materialize / push /
  create-add / snapshot-tag / a `common` for shared helpers (`resolved_member`,
  gitignore sync, selection). `main.rs` splits independently (CLI vs core audit
  are decoupled).
- **Then** WS1 (policy) + WS2 (red tests) → WS3–WS9 behavioral fixes, each landing
  in its now-small module. WS4–WS6 depend on WS1+WS3; WS7–WS9 are largely
  independent.

Order: **WS0 → degodify split (workspace_ops + main.rs, gate green) → WS1 + WS2 →
WS3–WS9.** (Mixing a pure move with a behavioral change forfeits the
byte-identical proof that makes the split safe — so split, then fix.)

## 7. Reconciliation Notes (how the two reviews combined)

- **Agreed (high confidence):** materialize writes planned-not-observed (F1);
  materialize/init partial mutation (F2); snapshot/tag stale (F3); status can read
  Ok over a dirty member (F5); write_atomic durability/race (F12); manifest+lock
  non-atomic (F14).
- **Unique to Response48:** init orphan-clones block retry (F2 detail); `.gitignore`
  not resynced on materialize (F4); `lock_match` ignores branch/detached (F11).
- **Unique to Response55:** fetch-during-preflight (F10); dirty-post-FF not
  rejected (F6); push no-preflight under atomic (F7); `--sync fetch-only` ignored
  (F8); CLI JSON/JSONL error path (F9); snapshot duplicate-ID (F13); generic
  aggregation lacks `Partial` (F15); `status --no-files` hides dirty (F16); rename
  detection (F17).
- **Revised after cross-checking Response55:** Response48 had rated `push` and the
  CLI JSON output as clean. Verified against the code: `push` has no
  preflight-before-mutation pass (F7) and the CLI error path is unstructured (F9).
  Both are upheld as real and incorporated. Response48's narrower P0→P1 framing of
  the materialize/lock issues is retained (no remaining silent-corruption P0 once
  the FF fix lands), but the policy/preflight findings raise the *aggregate* risk:
  several default-atomic operations can mutate before the selection is proven safe.
- **Net:** no remaining P0 (after WS0), but a coherent cluster of P1 preflight/
  observability defects that the WS3 phased refactor resolves at the root.
