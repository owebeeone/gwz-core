# Review: `GwzMergeDesign.md` — round 2 (F5)

Reviewed: 2026-07-15. Subject: `gwz-core/dev-docs/GwzMergeDesign.md`, the
revision marked "addresses `GwzMergeDesign-ReviewF5.md`".

Review method: disposition check of round-1 findings F1–F8 and the six open
questions, then a fresh pass over the new material (§8 baseline/retention,
§10 finalization, §11 gate table, §13 resume semantics, §16 message shapes,
§17 new primitives, the expanded matrix). Numbering continues from round 1:
new findings are **F9–F12**. None blocks M0; F9–F11 should land in the
document before M1/M2 implementation, F12 before M1.

## Verdict

The revision resolves all eight round-1 findings substantively — not by
annotation but by design. Several resolutions exceed what was asked: the
record now *retains unknown fields across read-modify-write* (protecting
newer-binary recovery data from older binaries), `commit_marker` was removed
from `MergeRequest` entirely rather than given semantics (mandatory evidence
is the cleaner contract), and the M0 lock-advance retention is now justified
("freezing the lock without a durable close/recovery lifecycle would leave
no safe way to advance it later") rather than merely staged.

Credit where due: the revision also fixed a defect round 1 missed — the
original evidence list required the marker to record "the root composition
commit", which is a self-reference (the marker rides inside that commit).
§10 now locates the composition commit *as the commit containing the marker*.
Correct, and it was not on my list.

This design is ready to implement once F9–F12 are folded in.

## Disposition of round-1 findings

| # | Finding | Disposition |
| --- | --- | --- |
| F1 | Continue undefined for failed/unattempted | **Resolved.** §13 defines resume as both "finish resolved" and "resume unattempted"; failed members retry only from an unchanged before state; ambiguous partial mutation is typed non-retryable; M8 updated; matrix rows added. |
| F2 | Root commit scope unspecified | **Resolved.** `commit_gwz_paths_checked` (§17) with tree-diff verification, expected-head CAS, candidate/publish split with idempotent crash recovery (§10), message + trailer convention, `commit_marker` field removed. |
| F3 | Gate needed one enforcement point | **Resolved.** Runtime allowlist before dispatch (M15), full per-command table (§11), `add` narrowed to conflicted participants, §14.2 choice 1 now routes through `--preserve`, matrix section added. |
| F4 | M7 flip understated | **Resolved.** §4 states today's partial lock advance on conflicted batches explicitly; M1 release-notes the freeze; M0 retention justified. |
| F5 | Record lacked the verified baseline | **Resolved.** `baseline` block (lock/manifest sha256 over exact persisted bytes, root_head), schema + writer versions, digest checks in continue/abort preflight and abort close. |
| F6 | Conflict-prediction seam unnamed | **Resolved.** `merge_simulate` in §17, deferred M4, M19 decision, matrix row. |
| F7 | §5 contradicted phasing | **Resolved.** §5 reframed as target surface; M0 interim epilogue specified verbatim in §15 (and it is honest about the lock behavior). |
| F8 | `--partial`/`--force` undefined | **Resolved.** Typed rejections incl. the deprecated alias (§5, M17, M0 bullet, matrix). |

All six open questions were adopted as §22 with the round-1 recommendations,
including the `--gc` verb and the never-age-out rule for records owning
preservation evidence.

## New findings

### F9 — `MergeRequest` needs a per-op field validation matrix (§16)

The request shape is optional-heavy and §16 never states which fields each
`MergeOp` accepts. Specify:

- `start`: requires `source_ref`; rejects `merge_id` and `preserve`;
  accepts `mode` (and `message` from M4).
- `resume`: rejects `source_ref`, `mode`, `preserve`; `merge_id` optional —
  when supplied it **must equal the open record's id** (guards scripts
  against racing a different merge).
- `abort`: as `resume`, plus `preserve` allowed.
- `status`: read-only; `merge_id` optional — the §12 "id-qualified status
  form" for archived operations should be reserved here now, even if it
  lands later.

Without this, the Rust and Python drivers will each invent validation and
the alias will invent a third. One table closes it; add matrix rows for the
rejects.

### F10 — Operation-level drift has no structured representation in `--status` (§12)

Continue and abort preflights now check the baseline digests (§13, §14.1)
and abort close verifies them (§14.4) — but §12's drift enum is member-scoped
only. A user who edited the manifest while a merge was open will have every
member report clean while continue rejects. Add operation-level drift
reasons (at least `baseline_lock_changed`, `baseline_manifest_changed`,
`record_unreadable`) to the status report so `--status` explains the
blocking condition before the user discovers it via a failed continue. One
matrix row: manifest edited while open → status reports the baseline drift.

### F11 — The gate table misses the remote tag forms (§11)

`tag --list` (allow) and `tag --create/--delete` (block) are covered, but
`gwz tag` also has remote push/fetch/remote-delete forms spanning member
repositories. They publish or import refs while the coordinated result is
partial — block them, same rationale as `push`. While editing the table, one
more row for completeness: `gwz init <url>` at the root of an *existing*
workspace is plan-only today (returns `Accepted`, mutates nothing) — say
whether it stays allowed as read-only planning or is blocked for symmetry;
either is defensible, unstated is not.

### F12 — The operation lifecycle lacks a `finalizing` state (§8, §10)

States jump from `awaiting_resolution`/`executing` to `completed`, but §10
defines a multi-step finalization window (verify → scoped root commit →
publish lock/boundary → close) with explicit crash recovery ("candidate
hashes and any root commit already created are recorded"). A crash inside
that window leaves a record whose state cannot express where it stopped —
`--status` would show an open operation with no conflicts, no failures, and
no name for what it is doing. Add `finalizing` (entered before the scoped
commit, exited at close), or document which existing state covers the
publication window and how status renders it. Matrix already has the
crash-after-commit-before-publication row; it needs the state assertion.

## Nits

- **"Classified retry point" (§13)** is used before it is defined; the
  definition is implicit in step 3 ("the unchanged before state"). Define it
  once where first used — a failed member is retryable iff its branch still
  points at the recorded before commit, the worktree/index are clean, and no
  integration state exists.
- **Unattempted-member drift blocks the whole continue** (§13). Consistent
  with the no-absorption stance, but unlike a conflicted member there is no
  native Git state hinting at the fix. Have `--status` (or the rejection)
  state the recovery explicitly: restore the member to its recorded before
  commit, or abort.
- **Operation-state definitions (§8):** the enum is listed without meanings;
  `halted` vs `awaiting_resolution` vs `recovery_required` will drift between
  docs and implementation without one line each.
- **Retention constant (§8):** "latest 20" should be named as a default, not
  a constant of nature — even if configuration is deferred, say "default 20".
- **`--gc` phase (§5, §19):** listed as a later addition and referenced by
  §8/§22, but no M-phase owns it. M3 (it exists to clean up what M3
  creates) or M4 — pick one.
- **Unborn root (§17):** `commit_gwz_paths_checked` takes `expected_head`; a
  workspace root with no commits yet makes finalize the root's first commit.
  Rare but legal — state whether `expected_head = none` is supported or the
  operation requires a born root.
- **Events:** still no explicit event design; §20 mentions the event catalog
  only as a doc-freshness item. One sentence in §16 naming the emitted
  events (per-member outcome, operation state transitions) against the
  existing `EventKind` conventions would close it.
- **Dry-run without the mutator lock (§7)** can race a concurrent real merge;
  that matches branch's existing dry-run behavior and plans are advisory —
  fine, but worth one clause acknowledging the race so nobody "fixes" it into
  a lock-holding dry-run later.

## Matrix additions for this round

- per-op request validation rejections from F9 (start with `merge_id`,
  resume with `source_ref`, wrong-id resume/abort, preserve outside abort);
- manifest edited while open → `--status` reports operation-level baseline
  drift (F10);
- remote tag forms blocked while open (F11);
- crash inside finalization shows `finalizing` (or the chosen state) in
  status, and resumed continue completes publication without a second
  evidence commit (F12 — extends the existing row);
- unattempted-member drift rejection names the member and the restore-or-
  abort recovery (nit).

## Closing

Round 1 said the hard parts were right; the revision made the soft parts
match. The remaining findings are paper cuts in specification completeness —
request validation, status expressiveness, two gate rows, one lifecycle
state. Nothing touches the core invariants, which held up under a second
adversarial pass: evidence before mutation, baseline-verified recovery,
all-or-nothing abort, no silent drift destruction.
