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
| Q1 | Is `fetch` inside the atomic guarantee? | **Treat fetch as mutation.** Plan with non-mutating `ls-remote` (libgit2 `Remote::connect`+`list`); fetch only *after* the whole selection passes Validate; if a remote-tracking-ref advance persists after failure, report it as an explicit member outcome. (Removes F10; honors "failed = nothing changed". Does not force the CLI.) |
| Q2 | Is `push` atomic by default? | Cross-remote atomicity is impossible. Default = **preflight all** (remote exists, refspec resolves, optional dry-run) **then push**; real partial only under explicit policy, reported `Partial` with per-member identity. |
| Q3 | Do `snapshot`/`tag` capture the live worktree or the lock? | **Live observed** state (REQ-084 "current state"). Reject dirty/unmaterialized unless an explicit flag records a dirty marker. Never claim `lock_match: Matches` unverified. |
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

## 3. Findings Register

The confirmed P0 (F0) is fixed and committed. F4/F10 are **superseded** by §2
decisions and no longer drive standalone work. "WS" maps to §4.

| ID | Sev | Finding | File:line | WS |
| --- | --- | --- | --- | --- |
| F0 | P0 ✓committed | FF moved ref before checkout (incident) — fixed `79d23c7` | git/mod.rs | WS0 (done) |
| F1 | P1 | `materialize`/`pull_snapshot` write lock+response from **planned** target, never re-observe worktree | workspace_ops:594-616 | WS4 |
| F2 | P1 | `materialize`/`init` **partial mutation** — members mutate, first `outcome?` aborts before lock write; stale lock / orphan clones; no recovery | workspace_ops:562-616, 316-406 | WS3,WS5 |
| F3 | P1 | `snapshot`/`tag` capture **stale lock**, not live worktree (no backend) | workspace_ops:421-437, 465-479 | WS4 |
| F4 | ~~P1~~ → AD2 | `.gitignore` not resynced on materialize/pull/clone | workspace_ops:504,631,681 | **superseded by AD2** |
| F5 | P1 | `status` reports `Ok`/`aggregate::Ok` for a **dirty/diverged** member | status:312, 630-643 | WS4 |
| F6 | P1 | `pull --head` does **not reject dirty post-FF** state before writing the lock | workspace_ops:787 | WS3 |
| F7 | P1 | `push` has **no preflight-before-mutation** → partial remote advance under default atomic | workspace_ops:855-911, 1899, 2010 | WS3 |
| F8 | P1 | `--sync fetch-only` (and merge/rebase/reset) **accepted but ignored** by core | cli:570,1983; workspace_ops:775-787 | WS6 |
| F9 | P1 | CLI `--json`/`--jsonl` **error path** prints plain stderr, not structured output | cli:194-209, 1019 | WS7 |
| F10 | ~~P2~~ → Q1 | `pull --head` fetches during preflight, advancing remote-tracking refs | workspace_ops:1710 | **superseded by Q1** |
| F11 | P2 | `lock_match` ignores branch/detached/upstream; `Matches` for a dirty member | status:556 | WS4 |
| F12 | P2 | `write_atomic` not crash-durable (no fsync) + fixed `{name}.tmp` race | artifact:335-343, 381-387 | WS8 |
| F13 | P2 | `snapshot` lacks the duplicate-ID guard `tag` has | workspace_ops:414-446 vs 458 | WS8 |
| F14 | P2 | manifest→lock→gitignore writes **not semantically atomic** | workspace_ops:134,141,227,403-406 | WS8 |
| F15 | P2 | Generic runtime `aggregate_status` has **no `Partial`**; top-level errors drop `member_id`/`member_path` | operation:768,834 | WS5 |
| F16 | P2 | Human `status --no-files` can **hide dirty** state | cli:1063,1238; status:326 | WS7 |
| F17 | P2 | `status` models renames but rename detection not enabled/tested | git/mod.rs:253 | WS9 |
| **F18** | **P1** | **operation code mutates the Git index via `git2`** — `stage_workspace_git_metadata` opens `git2::Repository`, `add_all`, `index.write()`, violating `GWZGitBackendDecision`'s "operation code MUST NOT expose git2" | workspace_ops:2229-2240 | **WS-backend** |

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
