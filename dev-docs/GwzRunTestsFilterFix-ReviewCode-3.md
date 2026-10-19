# run_tests.py --test-filter fix, round 3 (a7e677d1, filed at 2b601b4b) — CODE-AXIS RE-VERDICT

**Review object:** gwz-core code commit `a7e677d1627a33e1d31b2250aead3ba72d7cbdf2` ("run_tests.py --test-filter: ignored tests do not count as selected; a failed listing is named as the cause").
- Lane: `/Volumes/projects/limbo/gwz-dev-filterfix/gwz-core`.
- Lane HEAD: `2b601b4be09247038da19ca15e63e9ca9750e48a`, which only files the round-2 report.
- **Baseline:** `ea6258566e129f7f1b0f6df849adcb993bb0303a`, the round-2 object.
- The package under judgement is `ca73cae1..2b601b4b`.

**Date:** 2026-10-19. **Axis:** Code; single axis; read-only. **Round:** 3, the final round under the two-round cap. Filed verbatim by the lane owner.

**Verdict: GO.** 0 P0, 0 P1, 0 P2, 0 P3 open.
- Round-2 P2-2 and P3-2 are closed. Round-1 P2-1 and P3-1 are still closed.
- No new finding was found.
- No new architectural root cause was found; the lane is not stopped.

## 0. Evidence base

**Tuple at start**
- `git rev-parse HEAD HEAD~1 HEAD~2` returned `2b601b4b…`, `a7e677d1…` and `ea625856…`.
- `git diff a7e677d1 2b601b4b --stat` shows only `dev-docs/GwzRunTestsFilterFix-ReviewCode-2.md` (+111). The filing commit changes no code.
- The only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`. It is the operator's file and out of scope.

**Diff read in full:** `git show a7e677d1 -- scripts/`
- `scripts/run_tests.py`:
  - `FilterTally.listing_failures`;
  - a second listing per group, `-- <group args> --ignored --list --format terse`;
  - `names = [name for name in every if test_filter in name and name not in ignored]`;
  - a two-branch zero-tally message in `main`.
- `scripts/checks/test_run_tests_test_filter.py`:
  - `Cargo(ignored=…)`;
  - three new rows;
  - the existing rows now expect 8 listings (4 groups × 2).

**Allowed test command:** `python3.13 -m unittest scripts.checks.test_run_tests_test_filter scripts.test_windows_lane_check` ran 56 tests, OK.

**Mutants.** Each mutant was applied to a scratch copy of the `a7e677d1` files under `$SCRATCH/r3/{a,b,c,d}/scripts/…`, outside the repository. `subprocess.run` is patched.

| Mutant | Change | Rows that went red |
|---|---|---|
| a | Drop the ignored subtraction: `names = [name for name in every if test_filter in name]` | `…matches_only_ignored_tests_fails_the_run` and `…only_the_matched_tests_that_are_not_ignored_run` (FAILED, failures=2) |
| b | Never take the listing-failure message branch (`if tally.listing_failures:` → `if False:`) | `…listings_failed_says_so_and_does_not_blame_the_filter` (failures=1) |
| c | Stop counting listing failures (`tally.listing_failures += 1` → `pass`) | the same row (failures=1) |
| d | Make the ignored set empty (`ignored = set()`) | the same two rows as mutant a (failures=2) |

**libtest premise.** The lane owner's probe log, `/Volumes/projects/limbo/build-targets/filterfix-logs/libtest-probe.log`, shows:
- `--list --format terse` names both `t::runs` and `t::stress`;
- `--ignored --list --format terse` names only `t::stress`;
- `--exact t::stress` reports `0 passed; 0 failed; 1 ignored` and `exit=0`.

This confirms the round-2 premise that I had taken from libtest's semantics without running it.

**Inventory:** `rg '#\[ignore' tests` finds no ignored test in any integration target. All 28 ignored tests are in the library.

**Not run:** cargo, and `run_tests.py` as a program.

## Problem, requirement provenance, and scope

The authorization is unchanged: the operator's 2026-10-19 closeout of the isolated filter-tooling fix.

`a7e677d1` changes only:
- `scripts/run_tests.py`;
- its test module.

It touches no product code, workflow or other gate.

Default path (no `--test-filter`): `run()` still returns from `run_command` before any listing. In `main`, the new branch is guarded by `options.test_filter is not None`. Default runs are therefore unchanged.

Still recorded as a follow-up and outside this package: the round-1 optional guard in `windows_lane_check.run_entry`. That deferral is acceptable.

## Prior-finding closure table

| ID | Round | Finding | Remediation | Re-check | Status |
|---|---|---|---|---|---|
| P2-1 | 1 | A filter that matches no test in any group exits 0, so the receipt says PASS. | `FilterTally`, and a nonzero exit when it is zero (`ea625856`). | The row still passes. Mutant m2 (round 2) went red. The behaviour is preserved in `a7e677d1`, and the exit is still `result or 1`. | Closed |
| P3-1 | 1 | The batch-aggregation test could not tell when the last batch wins. | A sequence of run codes, with a failure placed in each of three batches (`ea625856`). | Unchanged in `a7e677d1`. The allowed command passes. | Closed |
| P2-2 | 2 | A filter that matches only `#[ignore]` tests passes having run nothing. | An `--ignored --list` per group; its names are removed from the selection and the tally (`a7e677d1`). | The `port_stress` counterexample now selects nothing and the run exits nonzero with "matches no test that runs in any group". The mixed listing runs only the non-ignored name. Mutants a and d go red. The premise is confirmed by the probe log. | Closed |
| P3-2 | 2 | The no-match message blames the filter when the listings failed. | `listing_failures` is counted, and the message names the failure while keeping the group's code (`a7e677d1`). | The row asserts exit 101, the absence of "matches no test" and the presence of "listing failed". Mutants b and c go red. | Closed |

## 1. Findings

None.

### Attacks on the new code that found no defect

**Cost of the second listing**
- The second listing runs after the first listing has already built the group's test binaries with the same arguments and environment. It costs one more cargo freshness check, plus one `--list` invocation of each target binary.
- The hosted-like suite has 4 groups, all `--lib`. That makes 4 extra test-binary list calls, each well under a second, set against a suite of about ten to twenty minutes.
- In local runs without `--lib`, the last real group lists each of the 5 integration targets as well. There are no runnable doctests.
- No rebuild is triggered: the flags, `RUSTFLAGS` and environment are identical between the two listings.

**Interaction with `--skip`**
- Both listings carry the same `test_args`: the group's prefix filters, or the real group's `--skip` pairs. libtest applies name filters and skips first, then keeps only the ignored tests for `--ignored`. So the ignored set is exactly the group's ignored tests that are not skipped.
- Names are taken only from the first listing, and removing a name that is also skipped is harmless. A stricter or looser ignored listing could never add a test to the run, because it is used only to subtract. Mutant d (an empty ignored set) is the dangerous direction, and the rows catch it.
- In the exact run, `--skip` is still dropped, as in round 1. That is safe, because skipped names never enter the selection.

**Listing order**
- The ignored names are used as a set, and the selection keeps the first listing's order, so libtest's output order cannot change what is selected.
- The test pins the order of the two listing calls by their arguments (`listing, ignored = cargo.lists()`).

**Failure of either listing**
- Either listing failing increments `listing_failures` once and returns that listing's nonzero code. The group can never report 0 after a listing failure.
- Without `--no-fail-fast`, `main` raises at the failing group, as before.

**Fake/real separation**
- Both listings use the group's `env` (`GWZ_TEST_GIT`, `GWZ_TEST_FS`).
- The exact run is unchanged from round 1.

**Tally semantics**
- `tally.matched` now counts names that are selected and not ignored, which is exactly what the `--exact` run executes. The round-2 hole is closed for every compiled-in test.

**No-globals rule:** `FilterTally` is still passed explicitly; no module state was added.

## 2. Out-of-scope observations

**Cross-target name collisions (limitation, not a defect)**
- In a local run without `--lib`, the real group's exact names go to every target. An exact name that is ignored in one target but not in another would be removed from both.
- Today this cannot happen: no integration target has an ignored test, and the hosted-like suite is `--lib` only, so it has a single target.
- If it ever did happen, the result is fail-closed when that name is the only match, and an omission otherwise.
- Suggested follow-up: if integration targets gain `#[ignore]` rows, key the subtraction per target.

**Message wording (cosmetic, no consequence):** the P3-2 message reads "{n} group listing failed" whatever n is.

**Round-1 observations still stand, unchanged:**
- a bare positional name still reaches cargo;
- the receipt has no top-level `filtered` field;
- `run_entry` passes a hosted-like entry on exit 0 whatever `tests.failed` says;
- the deferred defence-in-depth guard is still to do.

**Runtime-skipped tests (limitation):** a test that returns early when a host lacks something counts as passed, filtered or not. That is a test-design matter and is not introduced by this package.

## 3. Tuple check at end

At the end, `git rev-parse HEAD HEAD~1 HEAD~2` returned:
- `2b601b4be09247038da19ca15e63e9ca9750e48a`
- `a7e677d1627a33e1d31b2250aead3ba72d7cbdf2`
- `ea6258566e129f7f1b0f6df849adcb993bb0303a`

`git status --short` shows only `?? dev-docs/GwzRemoteTransportBugReport.md`, which is out of scope.

The tuple is unchanged. Nothing in the repository was modified; the mutants exist only as scratch copies in the session scratchpad.
