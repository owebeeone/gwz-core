# Review: `GwzMergeDesign.md` (F5)

Reviewed: 2026-07-15. Subject: `gwz-core/dev-docs/GwzMergeDesign.md`
(status: proposed — first-class `gwz merge` with coordinated
continue/abort/preserve lifecycle).

Review method: every checkable claim was verified against current source
(core handlers, git backend, protocol pins, workspace artifacts, CLI docs).
Findings are ordered by weight: **F1–F5 should be resolved before M1 lands**
(F1–F3 before the design is approved), F6–F8 before their respective phases.
Nits, answers to the design's open questions, and test-matrix additions
follow.

## Verdict

The design is sound and the hard parts are right. The honest atomicity
stance (§18) — atomic preflight, explicit mixed state, evidence before
mutation, no illusion of distributed transactions — is exactly the correct
frame for coordinated Git. The all-or-nothing abort preflight (§14.1–14.2)
closes the genuinely nasty failure mode (aborting the conflicted member and
*then* discovering another member can't roll back), M7's lock-as-baseline is
the right call, and the protocol-ahead-of-CLI enum strategy (`MergeMode`
carries `no_ff` while the CLI defers it) avoids pre-1.0 churn. The decision
table and test matrix are unusually complete for a proposal.

The fact-check found no wrong load-bearing claims. It found two undefined
lifecycle paths (F1, F2), one architectural gap (F3), one understated
behavior flip (F4), and one unimplementable verification as specified (F5).

## Fact-check: claims verified as correct

- **§4 current-behavior summary is accurate.** `merge_branch`
  (`handle_branch.rs`) preflights all members before mutation
  (`merge_preflight`), leaves conflicts in native Git state with paths
  reported, does not roll back earlier members, and refreshes the lock from
  clean results. ✓ — with one understatement, see F4.
- **§17 existing primitives exist.** `reset_hard` (`gitbackend.rs:67`),
  `commit_merge_resolution` (`gitbackend.rs:260`), and libgit2
  `merge_analysis` already in use internally (`gitbackend.rs:665,707,787`) —
  formalizing the latter as a named trait method is, as the design says,
  narrow work. ✓
- **Enum append is clean.** `ActionKind` occupies through 24
  (`CloneRepoMember=22`, `DetachRepoMember=23`, `AttachRepoMember=24`, pinned
  in `tests/protocol.rs`); `merge` appends at 25 with no renumbering. ✓
- **`.gwz/merge/` records stay out of the root surface.** `.gwz` is a fixed
  root-exclude prefix (`diff/plan.rs:47`, `ROOT_EXCLUDE_FIXED`), so runtime
  merge records are invisible to root status/diff exactly as §8 intends. ✓
- **Mutator-lock precedent matches.** Branch mutations acquire the
  workspace-wide mutator lock shared with stash, and skip it for
  list/dry-run (`handle_branch.rs:21`). §7's lock acquisition follows the
  established pattern — mirror the dry-run/status skip (see nits). ✓
- **Preservation has real machinery to reuse.** Coordinated stash bundles
  exist (`.gwz/stash/bundles/`, identity by object id first, `gwz:<id>:`
  message prefix second) — §14.3's "idempotent by recorded object id" matches
  the existing stash identity model. ✓ (Make the reuse explicit — see nits.)
- **`resume` naming is justified.** `continue` is a reserved word in several
  codegen targets (Python, Java, Kotlin, JS); the wire-vs-CLI split is the
  right move. ✓
- **`+<snapshot>` source sigil is consistent** with the diff operand
  convention (`+` is rev-only syntax, never a path). Reserving the same
  server-side interpretation for `MergeRequest.source_ref` keeps M4's
  snapshot sources a compatible addition. ✓

## Findings

### F1 — `--continue` is undefined for `failed` / `unattempted` members (§13)

§9's unexpected-error path stops execution and "leaves the coordinated merge
open", so the record can hold `failed` and `unattempted` members. §13's
continue requirements only address conflicted and already-clean members — as
written, the only exit from a failed-mid-batch merge is abort, and the wire
op is nonetheless named `resume`. Decide explicitly:

- **Recommended:** `--continue` re-preflights and *resumes execution* of
  `unattempted` members (and optionally retries `failed` ones) after the
  same start-preflight checks, then proceeds to resolution commits and
  finalization. This is what "resume" promises and what the durable record
  exists to enable.
- Alternative: a typed rejection directing the user to `--abort`. Cheaper,
  but it makes the record's `unattempted` bookkeeping purely forensic.

Either way the test matrix needs a continue-after-host-failure row; it
currently only exercises that failure shape at start.

### F2 — Finalization's root commit must be path-scoped and fully specified (§10.5, §13)

No existing operation commits the workspace root; markers ride the *user's*
`gwz commit`. Finalization introduces an internal root commit — justified
for evidence durability, but the design must state:

- the commit includes **only gwz-owned paths** (lock, marker, boundary
  metadata under `gwz.conf/`), via a scoped commit primitive, even when the
  root index already carries unrelated user-staged content (a common state —
  doc edits sit staged in real workspaces);
- the commit message convention;
- the semantics of `MergeRequest.commit_marker` (§16): does `false` skip the
  marker, the root commit, or both? Today's `--commit-marker/--no-commit-marker`
  pairing on `gwz commit` suggests marker-only, but finalization's evidence
  guarantee (M13) reads as unconditional — reconcile.

Without the path-scoping rule, finalize-during-continue silently sweeps
unrelated root changes into a machine-authored commit.

### F3 — The open-merge gate needs one enforcement point and a per-command table (§11)

"Unrelated GWZ mutation commands reject with an open-merge error" spans a
dozen handlers (pull, materialize, stash, branch, repo lifecycle, tag,
commit, capture, `init --update`, …). Implemented per-handler, the gate will
drift exactly the way per-handler selection logic would have. Recommend: a
single gate in the operation runtime after workspace resolution, driven by
an allowlist, plus a table in this design giving the verdict for every
existing command. Two tensions belong in that table:

- **`gwz add` is allowed** "so conflict resolutions can be staged" — but
  staging into a *merged* (non-conflicted) member creates precisely the
  index drift §13 rejects at continue. Either scope add to conflicted
  members while a merge is open, or document the trap and make the continue
  rejection message name the staged-drift member.
- **`gwz stash` is blocked** as a mutation, yet §14.2's recovery choice 1
  ("manually preserve the work") then forces raw `git stash` — the tool's
  own escape hatch routes around the tool. Either allow stash scoped to
  drifted non-participant state, or make choice 1 point at
  `--abort --preserve` as the sanctioned path.

### F4 — M7 flips live behavior and the phasing understates it (§4, §10, §19)

Today `merge_branch` advances the lock for clean members **even when other
members conflicted** — `observed_states` is applied unconditionally and the
boundary is re-synced (`handle_branch.rs:345-363`). So the current partial
outcome *does* move the composition. §4's summary ("clean member results
refresh the workspace lock") is technically accurate but hides that this
happens on conflicted outcomes too — which is the strongest argument *for*
M7 and deserves stating. Phasing consequence: M0 ("preserve current
behavior") ships partial lock advance; M1 flips to lock-at-baseline. Flag
the flip in M1 as an explicit behavior change (release note + JSON
consumers), or pull the lock freeze forward into M0 and accept the small
delta from current behavior immediately.

### F5 — The merge record lacks the baseline §14.4 verifies (§8)

Abort's close step "verifies that the lock still represents the recorded
baseline", but the proposed record shape carries no workspace-level fidelity
field — only per-member before commits. Add to the record at creation: the
lock content digest (and/or root HEAD) and the record/tool schema versions.
Without it, the §14.4 verification and the §11 "revalidate live Git state"
promise have nothing recorded to validate against at workspace scope.

### F6 — Name the conflict-prediction primitive now, even though it's deferred (§7, §17)

The founding narrative of this feature is "check merges would succeed before
committing". libgit2 supports in-memory tree merges (`merge_commits` /
`merge_trees`) with no worktree mutation, and `MergeRepoSummary` already
carries predicted kind and conflict paths. Add
`merge_simulate(repo, target, source) -> Clean | Conflicts(paths)` to §17's
primitive list as explicitly optional/deferred (M4), so the dry-run upgrade
lands later with zero protocol or record change. §7's honesty about the
limitation is right; reserving the seam makes it temporary.

### F7 — §5 "Initial surface" contradicts the phasing (§5, §15, §19)

`--continue`, `--abort`, `--status` are listed as initial surface but arrive
in M1/M2 — and §15's conflict epilogue prints "run: gwz merge --continue",
which M0 must not do while the verb doesn't exist. Label §5 as the *target*
surface, and give M0 an interim conflict epilogue (today's per-member
resolve-in-place guidance) that switches at M2.

### F8 — Define merge's response to `--partial` and `--force` (§5)

§5 says global selection and output options continue to apply, but
`--partial` contradicts merge's atomic-preflight stance until M4's explicit
skip policy exists, and `--force` must not become an accidental force-abort
(§14.2 explicitly refuses one). Both should be typed rejections for merge
ops initially, stated in the design and covered in the matrix — otherwise
"global options apply" implies behavior the design elsewhere forbids.

## Nits

- **Execution order (§9):** name it — manifest order, matching the
  root-first-then-manifest convention used elsewhere; "deterministic
  selection order" leaves it implementation-defined.
- **§13 wording:** "commit each resolved native merge with the recorded
  source and target identity" — say *parents* (recorded target HEAD +
  `MERGE_HEAD`) and the message convention; "identity" reads as
  author/committer, which is a different (also worth stating) concern.
- **Preservation stash (§14.3):** state that it *is* a coordinated stash
  bundle (visible in `gwz stash list`, tagged with the merge id) rather than
  a parallel mechanism — discoverability plus free reuse of existing restore
  verbs and identity rules. Worth a decision-table row.
- **Backup refs (§14.3):** one sentence noting `refs/gwz/*` does not match
  default push refspecs, so preservation refs will not leak on push — plus a
  matrix row confirming `gwz push` never pushes them.
- **Mutator lock (§7):** mirror branch's skip for `--dry-run` and hold
  nothing for `--status` (read-only per §12).
- **`gwz pull --sync merge` (§11):** one paragraph on the relationship —
  pull-created native merge states remain outside the coordinated lifecycle,
  are treated as foreign in-progress state by merge preflight (already
  covered), and pull is blocked while a coordinated merge is open. Without
  this, two merge machineries coexist unexplained.
- **§6 vs §15 example drift:** §6 establishes `docs → release`, §15 reuses
  the same trio with `docs → main`. Harmless, but the mixed-target display
  is the pedagogical point of §6 — keep it consistent.
- **`MergeResponse` (§16):** consider an operation-level `state` enum (and
  member counts) beside `open` — JSON consumers otherwise reconstruct
  operation state from per-repo rows.
- **Record hygiene (§8):** state the atomic write mechanism (temp + rename,
  as elsewhere) and that unknown record fields are tolerated on read
  (forward-compat for `gwz.merge-operation/v0`).

## Answers to the open questions (§22)

1. **Records:** archive on completion to `.gwz/merge/done/` with bounded
   count-based retention (e.g. last 20); the root marker is the canonical
   history. Delete-on-success loses crash forensics; unbounded retention is
   clutter.
2. **Marker schema:** additive optional merge section (before/source/result
   tuples) with a minor schema revision — a second marker kind fragments
   every evidence consumer, and additive-optional matches the taut evolution
   discipline already governing the protocol.
3. **`up_to_date` post-start work:** report-only, as recommended. Blocking
   abort on a member gwz has no rollback action for inverts M10's own logic.
   A strict whole-workspace flag can come later if demanded.
4. **Preserve untracked by default:** yes, as recommended — mirror
   `stash push -u` semantics and say so; exclude ignored files (no `-a`
   analogue) explicitly.
5. **Preservation-ref lifetime:** retain until explicit cleanup; surface
   leftovers in `gwz merge --status`; add `gwz merge --gc [<merge-id>]` (or
   fold into a future maintenance verb) deleting refs and archived records
   together. Refs are cheap; silent expiry is how evidence vanishes.
6. **`--adopt`:** keep it out until real demand exists. Drift resolution
   outside the open lifecycle preserves M8's invariant. If ever added, it
   must run the full continue preflight minus the drift check and record
   adopted commits as distinct evidence — never merged into the recorded
   results.

## Test-matrix additions

- continue after unexpected mid-batch failure (per the F1 decision);
- finalization with unrelated staged and dirty root content — the root
  commit contains only gwz-owned paths (F2);
- the open-merge gate: one row per existing mutation command (F3);
- `gwz add` into a merged member while open → continue rejects naming that
  member (F3);
- crash between record write and first mutation → `--abort` is a clean
  idempotent no-op (record-first invariant);
- `gwz push` never pushes `refs/gwz/*` (preservation refs);
- M0 conflict epilogue does not advertise `--continue` (F7);
- `--partial` and `--force` typed rejections for merge ops (F8);
- merge record round-trips with unknown fields preserved/tolerated
  (forward-compat).
