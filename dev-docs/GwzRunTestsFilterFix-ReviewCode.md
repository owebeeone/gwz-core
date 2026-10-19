# run_tests.py --test-filter fix (87c4d751) — CODE-AXIS REVIEW
**Review object:** gwz-core commit `87c4d75191377ae844e46a35006f998d2f205c84` ("run_tests.py --test-filter: narrow each group to its own tests …"), lane `/Volumes/projects/limbo/gwz-dev-filterfix/gwz-core`.  **Baseline:** parent `ca73cae15d4aa4a84021d893308014a22c6e71ec`.  **Date:** 2026-10-19  **Axis:** Code; single axis; read-only. Filed verbatim by the lane owner.
**Verdict: NO-GO**: 0 P0, 0 P1, 1 P2, 1 P3. P2-1 has a bounded fix. I pre-commit to GO on a revision that resolves P2-1. P3-1 is recommended in the same revision but does not block.

## 0. Evidence base
- Tuple at start: `git rev-parse HEAD` returned `87c4d751…`, and `HEAD~1` returned `ca73cae1…`. The only untracked file was `dev-docs/GwzRemoteTransportBugReport.md`, which is out of scope and was ignored.
- `git show 87c4d751` (full diff, 5 files, +186/−12), read in full.
- Files read at HEAD:
  - `scripts/run_tests.py`, all of it.
  - `scripts/windows_lane_check.py`: lines 1–184 and 185–484, which cover `HOSTED_SUITE`, `hosted_like_script`, `parse_hosted_like`, `run_entry`, `Receipt.passed` and option checks.
  - The parent's `run()` (`git show ca73cae1:scripts/run_tests.py`).
  - `scripts/checks/check_windows_receipts.py` (header and `gate`).
  - `scripts/checks/run_boundary_job.py` (header and `step_commands`).
  - `.github/workflows/checked-artifact-boundary.yml` lines 95–125.
  - `gwz-core/AGENTS.md` lines 15–45.
- Precedent searched: the repo's own anti-vacuity rules.
  - `.github/workflows/transport-candidate.yml:380–381`: "A filter that matches nothing passes, so the count is checked."
  - `.github/workflows/linux-identity-probe.yml:183`: "libtest exits 0 when a filter matches NOTHING".
- Target inventory, to judge batching and listing parsing:
  - `Cargo.toml` has no `harness = false` targets.
  - The only fenced doc blocks in `src/` are ```` ```text ```` blocks, so there are no runnable doctests.
  - `tests/*.rs` holds 5 ordinary integration targets.
- Test run (the allowed hermetic command): `python3.13 -m unittest scripts.checks.test_run_tests_test_filter scripts.test_windows_lane_check` reported `Ran 51 tests … OK`. No cargo was run, and `run_tests.py` itself was not executed.
- Process authority located: `/Volumes/projects/limbo/gwz-dev/dev-docs/AgentProcessRules.md` and `/Volumes/projects/limbo/gwz-dev/dev-docs/GwzProcessOptimization.md`. Neither has a vacuity-specific rule; the severity contract in the brief governs.

## Problem, requirement provenance, and scope
- **Observed problem.** `windows_lane_check.py --hosted-like-filter NAME` put `NAME` before `--` in the `run_tests.py … --lib` command. Cargo forwards a bare `TESTNAME` to libtest as an extra filter, and libtest ORs filters. Each fake-filesystem group (`GWZ_TEST_FS=fake`) therefore also ran the matching real-filesystem tests, which failed with "open ambient artifact root: entity not found". That produced wrong filtered receipts: fixc-20261014-d and -f, 18 false failures.
- **Provenance.** The operator's 2026-10-19 closeout authorization ("merge … the isolated filter-tooling fix … using its required checks").
- **Does the remedy address the problem?** Yes.
  - `run()` now lists each group with that group's own arguments, intersects the listing with `NAME` in Python, and runs the intersection with `--exact` under the group's own `GWZ_TEST_GIT`/`GWZ_TEST_FS` environment (`scripts/run_tests.py:83–104`).
  - `windows_lane_check.py:258` passes `--test-filter NAME` instead of the bare name.
  - The fake/real separation that the original bug broke now holds by construction: names come only from the group's own filtered listing, and the run's environment is the group's.
- **Scope.**
  - No product code is touched (`src/`, `Cargo.*`, `tests/` are unchanged).
  - The source checks in `main()` (`run_tests.py:292–301`) still run unconditionally, filter or not.
  - With no `--test-filter`, `run()` builds the same command and environment, prints the same two lines and returns the same code as the parent's `run()`. I compared the parent at `ca73cae1` with HEAD lines 83–85 and 119–124: the default path is unchanged, so CI's unfiltered calls of `run_tests.py` (`platform-matrix.yml`, `transport-candidate.yml`, `windows-matrix.yml`, `release.yml`, `release.py`) behave exactly as before.
  - The workflow change only adds the new test module to the boundary job's unittest line. All edits are inside the authorized editable set: gwz-core `scripts/` and that one workflow.

## 1. Findings

### P2-1: A filter that matches nothing in any group gives exit 0 and a PASS hosted-like receipt with zero tests run
- **Root cause.** `run()` turns "no listed test of this group matches" into a successful group result (`return 0`), and nothing in `main()` checks that at least one group matched. Before this commit the fake groups always ran their whole prefix group under a bare filter, so a filtered run always executed tests. Now every group can select zero.
- **Location.**
  - `scripts/run_tests.py:97–99`: `if not names: print(...); return 0`.
  - `scripts/run_tests.py:318`: `raise SystemExit(filesystem_result or fake_result or real_result)`, with no check that anything matched.
  - Consumed by `scripts/windows_lane_check.py:363`: `entry['passed'] = result.returncode == 0 or …`. There is no tests-ran check, so `HOSTED_LIKE_RESULT passed=0 failed=0` still passes.
- **Violated invariant.** A filtered run must not pass vacuously. The repo already records this rule ("A filter that matches nothing passes, so the count is checked", `transport-candidate.yml:380–381`; `linux-identity-probe.yml:183`), and the brief's severity contract restates it.
- **Reproduction.**
  - Run `windows_lane_check.py --label x-20261019-a --receipt-dir D --hosted-like --hosted-like-filter checked_artifact::recovery_protocol`. This is a plausible slip: the real path is `checked_artifact::tests::recovery_protocol::…`, and `FILTER` accepts the string.
  - All four groups print "no test of this group matches", and `run_tests.py` exits 0.
  - The host log has no `test result:` lines, so `passed=0 failed=0`; the filtered run also skips the integration step.
  - `entry['passed']` is True and the receipt prints `(PASS)`.
  - Hermetically, the group-level half is already shown by the passing `test_a_group_with_no_matching_test_is_not_run` (it asserts `code == 0` with no runs). Applying the same `Cargo(listing=…)` fake to `runner.main([... '--test-filter', 'no_such_test'])` gives `SystemExit(0)`, because each group returns 0 at line 99 and line 318 ORs zeros.
- **Impact.** A lane that cites a filtered receipt — the use this fix exists to repair — can carry a `passed: true` receipt that executed no test. The receipt gate checks labels for form only (`check_windows_receipts.py` header), so only a reviewer who notices `passed: 0` would catch it.
- **Required correction.**
  - In `run_tests.py`, when `--test-filter` is given, count the matched names across all groups that ran. If the total is zero, print a clear message (for example "--test-filter NAME matches no test in any group") and exit nonzero.
  - Per-group zero matches stay legitimate.
  - Optional defence in depth: `run_entry` should also refuse a filtered hosted-like entry whose `tests.passed + tests.failed == 0`.
- **Regression test.**
  - In `scripts/checks/test_run_tests_test_filter.py`: `runner.main(['--no-fail-fast', '--lib', '--test-filter', 'no_such_test'])`, with the `Cargo` fake and the three check patches, raises `SystemExit` with a nonzero code and makes no non-`--list` cargo call.
  - A companion assertion that the existing match case still exits 0.
  - If the defence-in-depth guard is added, a `test_windows_lane_check.py` case: a filtered hosted-like run whose fake host prints `HOSTED_LIKE_RESULT passed=0 failed=0` and exits 0 gives a receipt with `passed` false.

### P3-1: The batch-aggregation test cannot detect a last-batch-wins regression
- **Root cause.** `test_a_failed_batch_fails_the_group` makes every batch return 101, so it cannot tell `result = result or code` from `result = code`.
- **Location.** `scripts/checks/test_run_tests_test_filter.py`, `test_a_failed_batch_fails_the_group` (the `Cargo(..., code=101)` fake returns one code for every run).
- **Violated invariant.** Any failing batch must make the group fail; the commit message and `run()` intend this. The test should pin that, not only "all fail → fail".
- **Reproduction.** Change `run_tests.py:103` to `result = code` and run the allowed unittest command. All tests in the module still pass, even though a failing first batch followed by a passing last batch would now return 0.
- **Impact.**
  - `windows_lane_check.py:363` treats `returncode == 0` as a pass whatever `tests.failed` says.
  - So such a regression would let a filtered hosted-like receipt pass with failed tests, and no test would catch it.
  - The current code is correct; the gap is coverage.
- **Required correction.** Make the fake's run code sequence-driven, for example a list of codes popped per run, and assert that a `[101, 0]` sequence across two batches returns 101.
- **Regression test.** The corrected test above: two or more batches with codes `[101, 0]` → `runner.run(...) == 101`, and `[0, 101]` → 101.

### Attacks that found no defect
- **Mismatch.**
  - The names are taken from `--list --format terse` lines ending `": test"`. Benches are excluded, and `text=True` normalizes Windows CRLF line endings.
  - libtest's `--exact` compares the full name string, so `::` paths match.
  - gwz-core has no runnable doctests, which are the only source of names with spaces or parentheses, and no `harness = false` target that could misread `--list`. Test functions cannot be generic.
  - Dropping `--skip` from the exact run is safe, because skipped names are already excluded from the listing.
  - In the non-`--lib` local real group, an exact name sent to every target matches only its own target.
- **Batching.**
  - Names are capped at 24,000 characters per batch, plus small cargo arguments: `--manifest-path=…`/`--target-dir=…` in hosted-like, about 150 characters.
  - Python's `subprocess` on Windows calls CreateProcess directly with no cmd.exe, so the bound is 32,767. Cargo starts the test binary the same way, and the binary path is short.
  - No doctest (`rustdoc --test-args`) amplification applies.
- **Exit codes across groups.** `main()` aggregation and the fail-fast `SystemExit`s are unchanged. A listing failure returns its own code (`run_tests.py:89–91`), which is tested.
- **Listing pollution.** The listing's stdout is captured, so it adds no `test result:` lines to the host log's counts. Compilation output still goes to stderr and into the log.
- **windows_lane_check modes.**
  - The filter reaches only `hosted_like_script`; `--hosted-like-filter` without `--hosted-like` is refused (`check_hosted_options`).
  - `--tests` only changes `shape_script` (`cargo check --profile test`) and carries no filter.
  - A filtered run is still recorded as filtered (`tests.filter`), and the integration step is still left out. A filtered run cannot be recorded as a full one any more than before.
- **Tests go red on the original bug.**
  - `test_windows_lane_check` asserts `--lib --test-filter background_close ` and the absence of `--lib background_close `.
  - Against the parent's `run_tests.py`, the new module fails: `run()` rejects `test_filter`, and in `main`, `--test-filter` would land in the cargo arguments.
  - Both modules are hermetic: `subprocess.run` is patched and there is no cargo build.
- **Workflow.** The module is named in the "Check complete entry and call graph" step of the Structural boundary job. That is the job and step whose lines `run_boundary_job.py` reads (`STEPS`), so it runs in CI and in the local boundary runner.
- **Code rules.** `EXACT_COMMAND_BUDGET` is an immutable constant, which the AGENTS.md "no globals" exception for immutable data allows. There is no mutable module state, and the brace and cfg rules do not apply to Python.

## 2. Out-of-scope observations
- **(Limitation, not blocking.)** `run_tests.py` still forwards any unknown positional token to cargo, so a direct caller who passes a bare test name gets the ORed-filter behaviour back. The new docstring (`run_tests.py:29–32`) warns about this. Refusing such tokens would need care, because cargo options take values (`--features X`). Suggested follow-up: decide whether to refuse a bare positional token.
- **(Pre-existing, limitation.)**
  - `Receipt.as_json`'s top-level `passed` carries no "filtered" marker; the filter appears only in `shapes[].tests.filter`, and the lane gate checks receipt labels for form only.
  - Also, `run_entry` passes a hosted-like entry whenever the exit code is 0, whatever `tests.failed` says.
  - Neither is introduced here. Together with P3-1 they mean receipt correctness rests entirely on `run_tests.py`'s exit code.
  - Suggested follow-up: a top-level `filtered` field, and a consistency check between the exit code and `tests.failed`.
- The prior receipts fixc-20261014-d and -f are invalid evidence for the reason the commit message gives; they need re-running, not reinterpretation. Noted for the operator; no action is in scope here.

## 3. Tuple check at end
- At the end of the review, `git rev-parse HEAD` returned `87c4d75191377ae844e46a35006f998d2f205c84`, and `HEAD~1` returned `ca73cae15d4aa4a84021d893308014a22c6e71ec`.
- `git status --short` showed only `?? dev-docs/GwzRemoteTransportBugReport.md`, which is out of scope.
- The tuple is unchanged and no file was modified by this review.
