# run_tests.py --test-filter fix, round 2 (ea625856) — CODE-AXIS RE-VERDICT
**Review object:** gwz-core commit `ea6258566e129f7f1b0f6df849adcb993bb0303a` ("run_tests.py --test-filter: a filter that selects no test in any group fails the run; any failed batch fails its group"), the remediation of round-1 object `87c4d751`, in lane `/Volumes/projects/limbo/gwz-dev-filterfix/gwz-core`. **Baseline:** parent `87c4d75191377ae844e46a35006f998d2f205c84`. The package under judgement is `ca73cae1..ea625856`. **Date:** 2026-10-19. **Axis:** Code; single axis; read-only. Filed verbatim by the lane owner.

**Verdict: NO-GO.** Counts: 0 P0, 0 P1, 1 P2, 1 P3.
- Round-1 P2-1 and P3-1 are closed.
- A new P2-2 remains. It is the same vacuous-pass defect as P2-1 through a path round 1 missed: a filter that matches only `#[ignore]` tests.
- P2-2 has a bounded fix. I pre-commit to GO on a revision that resolves P2-2. P3-2 is non-blocking.

## 0. Evidence base
- **Tuple at start.**
  - `git rev-parse HEAD` returned `ea625856…`; `HEAD~1` returned `87c4d751…`.
  - The only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`. It belongs to the operator and was ignored.
- **Diff.** `git show ea625856`: `scripts/run_tests.py` +20/−4, `scripts/checks/test_run_tests_test_filter.py` +32/−2, and the filed round-1 report `dev-docs/GwzRunTestsFilterFix-ReviewCode.md`.
- **Allowed test command.** `python3.13 -m unittest scripts.checks.test_run_tests_test_filter scripts.test_windows_lane_check` ran 53 tests: OK.
- **Mutant runs.** These ran on scratch copies of the `ea625856` files outside the repo, under `$SCRATCH/r2/m{1,2,3}/scripts/…`. The test module loads `run_tests.py` relative to its own path, and `subprocess.run` is patched.
  - **m1 (P3-1 mutant).** `result = result or code` was changed to `result = code`. `test_any_failed_batch_fails_the_group_whichever_batch_it_is` FAILED; 8 tests, failures=1.
  - **m2 (P2-1 mutant).** The tally check `if options.test_filter is not None and tally.matched == 0:` was changed to `if False:`. `test_a_filter_that_matches_no_test_in_any_group_fails_the_run` FAILED; 8 tests, failures=1.
  - **m3 (unmodified, plus a probe).**
    - The module passes: 8 tests, OK.
    - A probe used the module's own `Cargo` fake with a listing of three `git::endpoint::ssh_fixture::port_stress::{a,b,c}: test` lines and called `main(['--no-fail-fast', '--lib', '--test-filter', 'ssh_fixture::port_stress'])`.
    - The result was `EXIT 0`, with an exact-name run issued per group.
- **Source read.**
  - `src/git/endpoint/ssh_fixture/port_stress.rs`: all three `#[test]`s are `#[ignore = "a stress run for the port race, by name"]`. The module doc tells the reader to run it by name.
  - The module is compiled under `cfg(test)` on every platform (`src/git/endpoint/mod.rs:123–129`, `src/git/endpoint/ssh_fixture.rs:819`).
  - `src/git/endpoint/ssh_tests/idle_loss/stress.rs:23` is another such test.
  - `rg '#\[ignore'` counts 28 ignored tests in `src` and `tests`.
- **Trigger paths.** `windows_lane_check.triggered(...)` over every path the package changes returns `[]`. The `Windows-receipt: ci-only` line is therefore harmless, and the lane gate does not need a receipt for `87c4d751`.
- **Lane-owner logs.** `/Volumes/projects/limbo/build-targets/filterfix-logs/{red,green,mutation-last-batch-wins,boundary}.log` exist. I did not depend on them; m1 and m2 reproduce their claims independently.
- **Not run.** No cargo was run, and `run_tests.py` was not executed as a program.

## Problem, requirement provenance, and scope
- **Unchanged from round 1.** The commit answers the round-1 findings. It changes only `scripts/run_tests.py` and its test module, plus the filed review document.
- **Product code and gates.** No product code, workflow or other gate is touched.
- **Default path.** Without `--test-filter`, the default path is still unchanged:
  - `run()` returns from `run_command` before it reads `tally`.
  - `main()` creates a `FilterTally`, but it reaches the new branch only when `options.test_filter is not None`, so it ends with `raise SystemExit(result)` and `result` is the same expression as before.
- **Deferral.** The defence in depth suggested in round 1, in `windows_lane_check.run_entry`, is recorded as a lane-owner follow-up and is not in this package. That is acceptable, because round 1 marked it optional. P2-2's required correction below does not depend on it.

## Prior-finding closure table

| ID | Round-1 finding | Remediation in `ea625856` | Re-check on `ea625856` | Status |
|---|---|---|---|---|
| P2-1 | No group matches the filter, so the run exits 0 and the receipt says PASS | `FilterTally` counts the selected names across groups. If the filter selects none, `main` prints "matches no test in any group" and exits with `result or 1` (`run_tests.py:329–333`). | The counterexample (`no_such_test`) now exits nonzero; the regression row passes. Mutant m2 turns that row red. | **Closed** for the case named. The selected-but-not-executed case is P2-2 below. |
| P3-1 | The batch-aggregation test could not detect last-batch-wins | `Cargo(codes=...)` makes the run codes a sequence. The new row checks `[101,0,0]`, `[0,101,0]` and `[0,0,101]` and asserts three batches each time. | Mutant m1 (`result = code`) turns the row red. | **Closed** |

## 1. Findings

### P2-2: A filter that matches only `#[ignore]` tests passes, having run no test
- **One root cause.** `FilterTally` counts names *selected from the listing*, and libtest's `--list` (run without `--ignored`) lists `#[ignore]` tests as ordinary `name: test` lines. The `--exact` run then skips those tests: libtest reports them as ignored and exits 0. The tally therefore counts tests the run never executes.
- **Location.**
  - `scripts/run_tests.py:95–108`: names come from every `": test"` line, and `tally.matched += len(names)` follows.
  - `run_tests.py:329–333`: the zero check reads that count.
  - Consumed unchanged by `scripts/windows_lane_check.py:363` (`entry['passed'] = result.returncode == 0 or …`).
- **Violated invariant.** A filtered run must not pass vacuously. This is the invariant behind P2-1, the repo precedent "A filter that matches nothing passes, so the count is checked" (`.github/workflows/transport-candidate.yml:380–381`), and the brief's contract that a receipt reporting a vacuous run as a pass is P2.
- **Reproduction.**
  1. Run `windows_lane_check.py --label x-20261019-a --receipt-dir D --hosted-like --hosted-like-filter ssh_fixture::port_stress`. This is the module's own documented "run by name" path, and it passes `FILTER`.
  2. The `--lib` real group's listing contains the three port-stress rows, so `tally.matched` is 3.
  3. The exact run prints `test result: ok. 0 passed; 0 failed; 3 ignored` and exits 0.
  4. `run_tests.py` exits 0. The host script reports `HOSTED_LIKE_RESULT passed=0 failed=0`, and the receipt says `passed: true`.
  - **Evidence for steps 2–4.**
    - The `run_tests.py` half is shown hermetically by the m3 probe (`EXIT 0`).
    - The libtest half rests on how libtest's `--list` and `#[ignore]` behave: tests marked ignored are listed, and an `--exact` run of them only reports them ignored and exits 0. I did not run it, because cargo is not allowed.
    - To confirm the libtest half on any host: `python scripts/run_tests.py --lib --test-filter ssh_fixture::port_stress` exits 0, and its log shows `3 ignored`.
  - `idle_loss::stress` behaves the same way.
- **Impact.** The vacuous filtered receipt P2-1 was meant to remove can still be produced for 28 existing tests. Several are the "run by name" stress and host-specific rows that a lane is most likely to name in a filter.
- **Required correction.** Count, and run, only the matched tests that will actually execute.
  - Subtract the ignored set: list the group a second time with `--ignored --list --format terse`, which lists only ignored tests, and remove those names before batching and tallying.
  - An `--include-ignored` pass-through is not wanted: the hosted job does not run ignored tests either.
  - The no-match message should then say that no non-ignored test matches.
- **Regression test.** In `test_run_tests_test_filter.py`:
  - Extend `Cargo` so that a `--list` command containing `--ignored` answers with an ignored subset.
  - Assert that `main([... '--test-filter', 'port_stress'])` exits nonzero, with no exact-name run, when every match is ignored.
  - Assert that a mixed listing runs only the non-ignored names.

### P3-2: The no-match message blames the filter when the listings failed
- **One root cause.** The zero-tally branch does not check whether any group's listing failed. A failed listing contributes nothing to the tally, so a build failure also reaches the "matches no test in any group" message.
- **Location.** `scripts/run_tests.py:89–91` (a failed listing returns before tallying) together with `run_tests.py:330–332`.
- **Violated invariant.** A diagnostic must name the actual cause.
- **Reproduction.**
  1. Run with `--no-fail-fast` (as `HOSTED_SUITE` does) and a lane head whose test build fails.
  2. Each group prints `listing the group failed (101)`.
  3. `main` then prints `--test-filter 'NAME' matches no test in any group` and exits 101.
- **Impact.**
  - The exit code is correct (`result or 1` keeps 101).
  - The last line of the log points at the filter rather than the build, which costs an operator a detour.
  - It does not block.
- **Required correction.** Print the no-match message only when every group's listing succeeded. Otherwise say that the filter selected no test because N listings failed.
- **Regression test.**
  - Use `Cargo(list_code=101)` with `main([... '--no-fail-fast', '--test-filter', 'x'])`.
  - Assert the exit code is 101.
  - Assert the "matches no test" text is absent, or is qualified by the listing failure, using `redirect_stdout`.

### Remediation attacks with no defect found
- **Per-group zero still legitimate.** A filter that matches in only some groups makes `tally.matched > 0`, and the exit is the groups' own result.
- **Early fail-fast exits.** When a group fails without `--no-fail-fast`, `main` raises before the tally check, and the code is nonzero anyway.
- **Explicit state, not a global.** `FilterTally` is mutable state passed explicitly into `run()`. It is not module or global state, so the AGENTS.md "no globals" rule is met.
- **Test fake.** `codes.pop(0)` is test-only, and each row supplies exactly the codes for the batches it expects. The rows also assert three batches, so a change in batch size would show as a test failure instead of an `IndexError` hiding behind a wrong count.
- **Hermetic tests.** Both modules remain hermetic, and the boundary workflow still names the module. The module set is unchanged since round 1.

## 2. Out-of-scope observations
- The round-1 observations still stand and are unchanged:
  - A bare positional test name passed to `run_tests.py` still reaches cargo.
  - The receipt has no top-level `filtered` field.
  - `run_entry` passes a hosted-like entry whenever the exit code is 0, whatever `tests.failed` says.
- The deferred `run_entry` guard would refuse a filtered entry with `tests.passed + tests.failed == 0`. It would also catch P2-2 at the receipt level. It does not replace the `run_tests.py` correction, because `run_tests.py` is also run directly.
- The round-1 report did not catch P2-2: its "Mismatch" attack checked how names are spelled and missed that `--list` includes ignored tests. I am recording this so that a later auditor can weigh both rounds.

## 3. Tuple check at end
- At the end of the review, `git rev-parse HEAD` returned `ea6258566e129f7f1b0f6df849adcb993bb0303a`, and `HEAD~1` returned `87c4d75191377ae844e46a35006f998d2f205c84`.
- `git status --short` shows only `?? dev-docs/GwzRemoteTransportBugReport.md`, which is out of scope.
- The tuple is unchanged. No file in the repository was modified; the mutant and probe copies live only in the session scratchpad.
