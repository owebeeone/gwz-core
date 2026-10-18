# Review: `GwzMergePlan.md` (F5)

Reviewed: 2026-07-15. Subject: `gwz-core/dev-docs/GwzMergePlan.md` (proposed —
implementation plan for `GwzMergeDesign.md` and its two review dispositions).

Review method: fidelity check against the revised design and both design
reviews (F1–F12 dispositions), structural check against the house planning
rules (milestone phases, parallel-friendly steps, foundational-first), and
fact-check of paths, tooling, and sequencing against the current tree.
Findings are ordered by weight: **F1 blocks approval as written**; F2–F5
should be resolved before I0 starts. Nits follow.

## Verdict

As an execution plan, this is the strongest planning document in the repo:
I0's interface-freeze-before-parallelism is exactly the right shape for a
lead-plus-agents build, the one-writer-per-file wave rule and ownership
tables are enforceable, the change-control triggers (§17) correctly route
design questions away from feature lanes, the fault-point enumeration in the
M2b gate is the kind of list that actually gets written as tests, and "no
wave ends with 'all agents finished' — it ends with one integrated tree that
passes its gate" is the correct definition throughout.

The design-review dispositions are almost fully absorbed: the F9 validation
matrix (I0.2 + `validate.rs`), F10 operation-level drift including
`record_unreadable` (M1-A/B), F11 gate rows including remote tag forms and
plan-only init (M1-C), F12 durable `finalizing` with crash-point coverage
(M2b), and every round-2 nit (unborn root, default-20 retention, GC phased
into M3, restore-or-abort guidance, classified retry point, advisory
dry-run) all landed.

But the plan contains one large piece of work the design explicitly forbids
(F1), and that must be resolved at the design level before I0 freezes
interfaces around it.

## Fact-check: claims verified as correct

- **Module layout precedent.** `workspace_ops/merge/` as a module directory
  matches the existing `src/diff/` decomposition; current branch merge lives
  in flat `handle_branch.rs` re-exported from `workspace_ops/mod.rs:1,26`. ✓
- **Python generated-protocol path exists**:
  `gwz-py/src/gwz/protocol/generated/`. ✓
- **Regen order matches the tooling**: `gwz-core/protocol/regen.py` (Rust +
  corpus), then `gwz-py/scripts/regen_protocol.py` (drift check against the
  same schema), both exist and are the current release-gate commands. ✓
- **Verification gate commands are real** (`cargo test -p gwz-core`,
  `-p gwz`, `regen.py --check`, `run_tests.py`, clippy form matches
  `release.py`'s gate). One gap in what they prove — see F3. ✓
- **Driver split matches reality**: the Python CLI is already structured as
  `cli_*.py` slices over one native bridge; a merge slice fits the existing
  pattern. ✓
- **"Migrate, don't expand `handle_branch.rs`"** is consistent with the
  design's supersession stance and the existing code shape. ✓

## Findings

### F1 — M2c implements what the design forbids (blocking)

The design is unambiguous: *"No merge of the workspace root repository"*
(§3), *"Selecting `@root` for merge is an error rather than a silently
ignored target"* (§5), decision **M2**: *"root is rejected as a merge
target."* The plan schedules the opposite as a full milestone — M2c
"explicit workspace-root participation," plus supporting threads woven
through earlier waves: the Objective bullet ("explicit opt-in workspace-root
participation"), I0.1 requirements ("explicit `@root` selection", "root-last
execution"), M0-B ("return the phased typed result for explicit root before
M2" — a *coming-soon* result where the design specifies a permanent error),
and `recovery.rs`'s "discovery before normal manifest parsing" (only
necessary because a root merge can leave `gwz.conf` itself conflicted).

The plan's own rules say what must happen: §3 "Requirements and design are
updated before implementing behavior outside the current accepted contract"
and §17's change-control triggers (this trips at least "allowing an
operation transition not present in the design" and "reparsing conflicted
root metadata to find recovery state" — which §17 itself lists as a
*stop* trigger, yet M2c-B requires it).

Two coherent resolutions; pick one before I0:

1. **Revise the design first.** Root participation is real design work, not
   a plan detail: merging the root means merging the composition ledger —
   `gwz.lock.yml`/`gwz.yml` conflicts become the *common* case between any
   two branches that advanced composition; the baseline-digest model, marker
   placement ("on top of the root merge result"), frozen-participant rules,
   and no-valid-manifest recovery all need design sections, new decisions
   (M2 reversed, new M-rows), and a review pass. The plan already contains
   good design *content* for this (M2c-B's reconciliation rules are
   thoughtful) — that content belongs in `GwzMergeDesign.md`, not only here.
2. **Cut M2c** and strip the root threads: Objective bullet, I0.1's two root
   requirements, M0-B's phased result (becomes the design's permanent typed
   error), the M2 complete gate's root matrix, and simplify `recovery.rs`'s
   before-manifest-parsing requirement (member-only merges never invalidate
   root metadata).

Either is defensible. What is not defensible is freezing I0 interfaces
(protocol enums, record schema, requirements ids) around a feature the
authoritative design rejects — that bakes the contradiction into the wire.

### F2 — The wire-level fate of `BranchOp::Merge` is unspecified

The design makes `branch --merge` a *CLI* alias that constructs
`MergeRequest`, and the plan's M0 gate "remove[s] the old branch merge
behavior after its tests are transferred." Neither says what core does with
an incoming protocol-level `BranchRequest{op: merge}` after M0 — and other
clients (Python scripts, JSON drivers) can send one today. Enum values are
append-only, so the value stays; the behavior must be chosen: keep handling
it (a second implementation path, which §3 forbids) or return a typed
`deprecated`/`use-merge-method` error. Recommend the typed error, stated in
the design's §4, implemented in I0.5's validation layer, with a protocol
test row. Silence here yields the one thing the plan explicitly bans — two
merge paths.

### F3 — The gwz-py gate validates a stale native module

§15's gate runs `.venv/bin/python run_tests.py`, but `run_tests.py` does not
rebuild the native bridge — it runs pytest against whatever `.so` was last
installed. This exact gap produced a false-green during the v0.9.2 release
(240 tests passed against a June build while the crate didn't compile).
Merge work changes the bridge in every wave (new dispatch, new messages).
Add `maturin develop` (or an equivalent freshness check) to the wave gate —
or better, fix `run_tests.py` to rebuild first and let the plan inherit it.
Without this, every "Python parity" checkmark in the plan is advisory.

### F4 — I0.2 extends the protocol beyond the design's §16; sync the design

Two additions are right but undocumented in the design: `gc` joins
`MergeOp` (design §16 has only start/resume/abort/status; §8/§11 define GC
semantics but no wire op), and `OperationStateChanged` joins `EventKind`
(the design's event story is one doc-freshness bullet). Both need design
§16 text — and `gc` needs a row in the F9 validation matrix (accepts
optional `merge_id`; rejects `source_ref`, `mode`, `preserve`; rejects the
currently open merge id per §11's table). Cheap now; a schema archaeology
question later.

### F5 — Steps have no size budget, and two lanes look oversized

The house planning rule budgets steps to an aspirational <500 LOC. The plan
never mentions size, and at least two tasks plausibly blow through it:
M0-B (`validate.rs` + `plan.rs` + `start.rs` + fake-backend tests in one
task) and M2b-A (the full finalization state machine plus marker extension).
Both have natural split points the plan already names — `validate.rs` is
separable from plan/start (it is also the I0 exit-gate test subject, so it
arguably belongs to I0's lead work), and the additive marker conversion is
separable from the publication state machine. Annotate expected sizes per
task and pre-split these two; everything else looks within budget.

## Nits

- **mkdocs nav**: M0-C owns `docs/commands/merge.md` but the nav entry lives
  in `gwz-cli/mkdocs.yml` — name it in the ownership list so the page isn't
  orphaned (the generated-reference check won't catch a missing nav row).
- **M1-C has two owners** ("lead for central runtime/gate; driver agent for
  surfaces") — legitimate, but it's the only task where the one-writer rule
  depends on an intra-task file split; make the ownership table's rows for
  this task explicit when the wave starts.
- **I0.1 requirement ids**: the plan says ids are assigned once and
  referenced by tests — also state where the id registry lives
  (`GWZRequirements.md` itself, presumably) so agents cite consistently.
- **§15 protocol-staleness note**: the vendored-taut
  `generated_protocol_is_current` test regenerates from the *schema file*,
  so I0.2 schema edits keep it green only when vendored taut and the pinned
  PyPI wheel agree (they do at 0.8.1). Worth one line so a future taut bump
  mid-plan doesn't surprise a wave gate.
- **M0 gate wording**: "remove the old branch merge behavior after its tests
  are transferred" — say explicitly that `BranchOp::Merge`'s *wire value* is
  retained (append-only) even as the handler path is removed; only F2's
  typed answer remains.
- **Mermaid map**: G2B → M2R edge encodes the root-after-finalization
  sequencing; if F1 resolves by cutting M2c, remember the map, DoD table
  row M2c, and §18 all change together.

## Closing

Resolve F1 at the design level, pin F2's wire answer, patch the F3 gate, and
this plan is ready to run — the I0-first structure, ownership discipline,
and gate definitions are better than they needed to be, and the test
architecture (fast fake-backend lifecycle tests separated from real-repo
backend tests separated from filesystem service scenarios) is the right
three-layer split for this codebase. The recommended first run (§19,
stopping at I0) is the correct opening move regardless of how F1 lands.
