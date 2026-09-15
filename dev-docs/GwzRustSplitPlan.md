# GWZ Rust split plan

Date: 2026-09-16. Status: **draft**. Scope: gwz-core and gwz-cli product code.

Two kinds of work sit behind the word "split":

- **File splits**, which `rust-split` performs mechanically.
- **Procedure splits**, which it does not: it moves top-level items, never the
  statements inside one function.

This plan keeps them apart, and changes no behaviour in either case.

## 1. Measured facts

Measured on 2026-09-16 at gwz-core `901a430a` and gwz-cli `0c54380`, over
product code only: no `tests/` directories, no `tests.rs` or `*_tests.rs`, no
generated files.

Files against the policy in §2:

| Tree | Product files | Over 1,000 | Over 500 |
| --- | --- | --- | --- |
| `gwz-core/src` | 484 | 13 | 96 |
| `gwz-core/crates` | 41 | 4 | 14 |
| `gwz-cli/src` | 96 | 0 | 1 |

The largest, which are the cohesion reviews Phase 2 owns:

| File | Lines |
| --- | --- |
| `crates/local-disposal/src/lib.rs` | 2277 |
| `src/artifact/mod.rs` | 1501 |
| `src/workspace_ops/handle_materialize.rs` | 1311 |
| `src/workspace_ops/handle_create_repo.rs` | 1294 |
| `src/checked_artifact/capability/pre_catalog/provider/managed_mutation.rs` | 1267 |
| `src/git/gitbackend/fake_repository.rs` | 1230 |
| `src/git/gitbackend/contract.rs` | 1169 |
| `crates/work-detector/src/lib.rs` | 1188 |
| `src/checked_artifact/namespace/host.rs` | 1083 |
| `src/checked_artifact/entry.rs` | 1078 |
| `crates/refcopy/src/native.rs` | 1043 |
| `src/workspace_ops/pull_head_member_preflight.rs` | 1033 |
| `crates/workspace-install/src/lib.rs` | 1007 |
| `src/filesystem/native.rs` | 1003 |

Longest procedures, which no file split addresses:

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

Tooling, at `~/limbo/rust-split`:

- The installed CLI on PATH reports **0.1.3**; the checkout is at **0.2.0**.
- The commits in between fix path rebasing in moved bodies, import and include
  anchors, and opaque macro input — exactly the cases gwz-core exercises.
- The repository also ships a skill at `skills/rust-split/SKILL.md`, which is
  not installed in this workspace's Claude configuration.

## 2. The rule

Adopt the tool's own policy (`~/limbo/rust-split/docs/SplitPolicy.md`) rather
than inventing one:

- around 1,000 lines, review the file's cohesion; this is a soft trigger, not an
  automatic split;
- when a split is warranted, create responsibility owners below 500 lines;
- treat 500 as a ceiling, not a packing target;
- split earlier when a small file becomes a dumping ground;
- choose boundaries by responsibility, not by line count.

For procedures the policy says nothing, and neither does this plan. The trigger
is qualitative: a function whose phases are already named in its own comments
and locals, but which still reads as one procedure. `handle_push_with_events_in`
is the worked example.

## 3. Goals and non-goals

**Goals:** make each long procedure's phases visible; bring the files that fail
a cohesion review under the ceiling; record what was deliberately left large.

**Non-goals:**

- **No new crates.**
- **No behaviour change.** No output, JSON, exit-code or contract change.
- **No public API change.** Entry points keep their names and signatures.
- **No formatting pass.** A split must stay reviewable as a relocation.
- **No repository-wide campaign.** Ordinary feature work does not become a
  split refactor.

## 4. Method

### 4a. File splits, with `rust-split`

Follow the bundled skill. In short:

1. `rust-split --help`, and confirm the installed version (§5, step 0.1).
2. `rust-split explode <file> --out <fresh-dir>`.
3. Verify the chunks in manifest order reproduce the original **byte for byte**.
4. Read `manifest.toml` for item boundaries. Adjacency is evidence, not
   architecture; adjust grouping there rather than by hand.
5. `rust-split split <file> --max-loc 500 --out <fresh-dir>`, adding `--module`
   for anything that is not a crate root.
6. Review the generated layout: imports, re-exports, widened visibility,
   extracted inline test modules, file-module subdirectories, conditional
   scopes.
7. Copy it in deliberately, let the compiler enumerate the `use` and visibility
   fallout, then run the suites.

**Guardrails:** attributes and `#[cfg]` gates travel with their declaration or
module boundary; unconditional imports stay outside conditional sections; a file
left over the ceiling is recorded with its reason, not silently accepted.

### 4b. Procedure splits, by hand

1. Name the phases the procedure already has.
2. Extract each as a private function, taking what it needs and returning what
   the next consumes. Prefer an existing type over a new tuple.
3. Leave the data types where they are.
4. Evidence: the existing suites. Add a test only where an extraction exposes a
   seam worth pinning, and say so.

## 5. Phases

### Phase 0: prerequisites (operator decisions)

**Step 0.1: install the current `rust-split`.** The 0.2.0 fixes cover path
rebasing and macro preservation. Splitting gwz-core with 0.1.3 risks exactly
the breakage those commits fixed.

**Step 0.2: install the bundled skill** into the Claude configuration, so the
workflow and guardrails are loaded when the work runs.

### Phase 1: the push procedure (hand; §4b)

**Step 1.1: split `handle_push_with_events_in` (377).** Its phases: select and
validate, dry run and preflight rows, capture plans, concurrent reads,
already-on-origin, transfers, root proof. Budget about 400 LOC moved.
Commit: gwz-core.

**Step 1.2: follow-through.** Split `push_member` (107) and `push_root` (91)
only if they still read as two procedures afterwards. Budget about 150 LOC.

Waits on the push plan's queue (§6). Steps run in order.

### Phase 2: cohesion reviews of the 17 largest files

One step per file, each producing a decision before any split: keep and record
why, or split with §4a. The candidates are the table in §1.

Order by how often the file is edited, not by size. `handle_materialize.rs`,
`handle_create_repo.rs` and `pull_head_member_preflight.rs` are in the
workspace-ops churn; `local-disposal` and `work-detector` back the lane-disposal
issues (root `dev-docs/GwzLaneIssues.md`, L1) and would be split as part of that
work rather than ahead of it.

Budget: the review is cheap; a split that follows is about 400 LOC moved.
Commit: gwz-core, one per file.

### Phase 3: the push path's neighbours

**Step 3.1: `handle_tag_in` (300), by hand,** reusing Phase 1's phase names
where they match. Budget about 300 LOC.

**Step 3.2: `push_member.rs` (832) and `publication.rs` (729).** File-level,
only if their cohesion review says so after Phase 1 shortens them.

Depends on 1.1.

### Phase 4: the other long procedures (hand)

Independent steps, one file each: `handle_commit` (252),
`handle_add_existing_repo_in` (247), `handle_init_from_sources` (239), the
pull-head pair (236 and 201), and gwz-cli's `execute_invocation` (313).

Commit: gwz-core, except the last, which is gwz-cli.

### Phase 5: the 500-ceiling backlog

96 files in `gwz-core/src`, 14 in `crates` and 1 in `gwz-cli/src` sit between
500 and 1,000 lines. This is **not** a campaign. Split one when it is already
being worked on and the ceiling is in the way, using §4a.

## 6. Sequencing

- **Phase 0 first.** Do not run a split with the older CLI.
- **Phase 1 waits** for the push plan's queue to drain. Those files were
  rewritten by steps 3.3 to 3.5 and by both bug fixes.
- **Never split a file another lane is editing.** Check `gwz local list` first,
  and defer when the merge cost would dominate the benefit.
- Phases 2 to 5 are independent of each other, one file per step, so several can
  run in parallel lanes.

## 7. Risks

- **Stale tool.** Step 0.1 exists because 0.1.3 predates the path and macro
  fixes.
- **A split that changes behaviour.** The suites are the evidence. A step that
  existing tests cannot prove names the test it adds.
- **A formatter run inside a split.** It destroys the pure-move diff. Formatting
  is always a separate pass.
- **Merge conflicts with in-flight work.** Mitigated by §6.
- **`gwz-alpha` drift.** A rebuild after any step changes the binary hash that
  the workspace root's `dev-docs/GwzUrlSchemePushAcceptanceRunbook.md` pins in
  P1. Refresh that table with the rebuild, or run the acceptance first.
