# GWZ Rust split plan

Date: 2026-09-16. Status: **draft**. Scope: gwz-core and gwz-cli product code.

The types are already split; several procedures are not. This plan breaks the
longest procedures into named phases, in place. It changes no behaviour.

## 1. Measured facts

Measured on 2026-09-16 at gwz-core `62f8afa0` and gwz-cli `0c54380`, over
product code only (no test modules, no generated files).

Longest functions:

| Function | Lines |
| --- | --- |
| `workspace_ops/push_member.rs::handle_push_with_events_in` | 377 |
| `gwz-cli globalargs/dispatch.rs::execute_invocation` | 313 |
| `workspace_ops/handle_tag.rs::handle_tag_in` | 300 |
| `workspace_ops/handle_commit.rs::handle_commit` | 252 |
| `workspace_ops/handle_create_repo.rs::handle_add_existing_repo_in` | 247 |
| `workspace_ops/handle_init_from_sources.rs::handle_init_from_sources` | 239 |
| `workspace_ops/pull_head_member_preflight.rs::pull_head_member_preflight` | 236 |
| `workspace_ops/pull_head_member_preflight.rs::handle_pull_head_with_events_in` | 201 |
| `workspace_ops/publication.rs::root_dependencies` | 108 |

Largest product files:

| File | Lines |
| --- | --- |
| `crates/local-disposal/src/lib.rs` | 2277 |
| `src/artifact/mod.rs` | 1501 |
| `src/workspace_ops/handle_materialize.rs` | 1311 |
| `src/workspace_ops/handle_create_repo.rs` | 1294 |
| `crates/work-detector/src/lib.rs` | 1188 |
| `src/workspace_ops/push_member.rs` | 832 |
| `src/workspace_ops/publication.rs` | 729 |

Neither repo's `CLAUDE.md` nor `AGENTS.md` states a size budget.

## 2. Goals and non-goals

**Goals:**

- Split long procedures into named phase functions, so each phase is readable
  and testable on its own.
- Record a size target, so "too long" stops being a matter of taste.

**Non-goals:**

- **No new crates.** Everything stays in its current crate.
- **No behaviour change.** No output, JSON, exit-code or contract change. Each
  step is a refactor the existing suites already cover.
- **No public API change.** Entry points keep their names, signatures and
  visibility.
- **Not a formatting pass.** Touch only the functions a step names.

## 3. The target

Aspirational, not a gate:

- a product function fits in about 120 lines;
- a product file fits in about 800 lines;
- a function that stays longer carries a comment saying why.

Record the target in `gwz-core/CLAUDE.md` once Phase 1 proves it holds (step
1.3).

## 4. Method

Each step follows the same shape:

1. Name the phases the procedure already has, from its own comments and locals.
2. Extract each phase as a private function taking what it needs and returning
   what the next phase consumes. Prefer an existing type over a new tuple.
3. Leave the types where they are. This plan moves procedures, not data.
4. Keep the file unless it passes the size target afterwards; then make it a
   folder module, one file per phase group.
5. Evidence: the gwz-core suite, `cargo fmt --check`, both clippy
   configurations, and `cargo test -p gwz` at the root. Add a test only where
   an extraction exposes a seam worth pinning; say so in the report.

## 5. Phases

### Phase 1: the push procedure (the live one)

**Step 1.1: split `handle_push_with_events_in`.** Extract its phases as named
functions: select and validate, dry run and preflight rows, capture plans,
concurrent reads, already-on-origin, transfers, root proof. Budget about 400
LOC moved. Commit: gwz-core.

**Step 1.2: follow-through in the same file.** If `push_member` (107) and
`push_root` (91) still read as two procedures afterwards, split their transfer
and reporting halves. Skip if 1.1 already made them clear. Budget about 150
LOC. Commit: gwz-core.

**Step 1.3: record the target.** Add §3 to `gwz-core/CLAUDE.md`, with Phase 1
as the worked example. Budget about 30 lines. Commit: gwz-core.

Depends on: the push plan's own queue being drained first (§6). Steps run in
order.

### Phase 2: the push path's neighbours

**Step 2.1: `handle_tag_in` (300).** It shares the read, preflight and proof
shape with push. Extract the same phases, and name them as in step 1.1 where
they match. Budget about 300 LOC. Commit: gwz-core.

**Step 2.2: `root_dependencies` (108).** Extract only if it grows past the
target while Phase 2 runs; it is cohesive today. Budget about 120 LOC.
Commit: gwz-core.

Depends on 1.1, for the phase names.

### Phase 3: the other workspace_ops handlers

Each step is independent and can run in its own lane.

**Step 3.1: `handle_commit` (252).** Budget about 250 LOC.
**Step 3.2: `handle_add_existing_repo_in` (247).** Budget about 250 LOC.
**Step 3.3: `handle_init_from_sources` (239).** Budget about 240 LOC.
**Step 3.4: `pull_head_member_preflight` (236) and
`handle_pull_head_with_events_in` (201).** One step, one file. Budget about
440 LOC.

Commit: gwz-core, one per step.

### Phase 4: the CLI and the oversized files

**Step 4.1: `execute_invocation` (313).** gwz-cli's dispatch procedure. Budget
about 300 LOC. Commit: gwz-cli.

**Step 4.2: file-level splits.** Make folder modules of `artifact/mod.rs`
(1501), `handle_materialize.rs` (1311) and `handle_create_repo.rs` (1294), one
file per step. Budget about 400 LOC moved each. Commit: gwz-core.

**Step 4.3: the two large crates.** `crates/local-disposal/src/lib.rs` (2277)
and `crates/work-detector/src/lib.rs` (1188). These are libraries with their
own contracts; split by contract area, not by line count. Budget about 500 LOC
each. Commit: gwz-core.

Phase 4 depends on nothing in Phases 1 to 3.

## 6. Sequencing

- **Phase 1 waits** until the push plan's current queue drains. Its files were
  rewritten by steps 3.3 to 3.5 and by both bug fixes; splitting them mid-queue
  invites conflicts.
- **Phases 2, 3 and 4 are independent** of each other. Each step touches one
  file, so several can run in parallel lanes.
- **Never split a file** while another lane is editing it. Check the live lanes
  first (`gwz local list`).

## 7. Risks

- **Merge conflicts with in-flight work.** Mitigated by §6.
- **A refactor that changes behaviour.** The suites are the evidence; a step
  that cannot be proven by existing tests names the test it adds.
- **`gwz-alpha` drift.** A rebuild after any step changes the binary hash that
  the workspace root's `dev-docs/GwzUrlSchemePushAcceptanceRunbook.md` pins in
  P1. Refresh that table with the rebuild, or run the acceptance first.
- **Review load.** Each step is one function; keep them separately reviewable
  rather than batching a phase into one commit.
