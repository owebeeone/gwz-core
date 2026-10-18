# Re-verdict: Windows flake sweep, remediation round 1

**Object:** gwz-core `7f7f9149`, one commit over `263701cf` (10 files, +316/−38), in lane `/Volumes/projects/limbo/gwz-dev-winflakes`.
**Evidence:** gwz-core-evidence `e127a84d`, run `2026-10-16-winflakes` (README §9, `raw/round1/`, `raw/dabeest/winflakes-20261016-{k,l}/`).
**Date:** 2026-10-16
**Verdict:** **GO.** All six round-0 findings are closed. Open: P0 0, P1 0, P2 0, P3 1 (new, a broken evidence pointer). It is not blocking.

All four mutants fail and the unmutated rows pass on my own Mac build. Receipt `-k` binds to the commit. Every number in the CNG classification table matches the raw logs.

## Closure table

| Finding | Status | How it was verified |
|---|---|---|
| **F1** (P2): cancel row passed without a cancel | **Closed** | The row now asserts `failure.code == Cancelled`, and its helper runs `sleep 3600`. My mutant with `request.cancel()` deleted **fails** at `cancellation.rs:138` after 30.53 s; before the fix it passed in 10.65 s. The unmutated row passes, and no helper process is left behind. |
| **F2** (P2): CNG test retry absorbed any failure without classifying it | **Closed** | `observed.rs` records each libssh2 handshake code and retries only on an exact `[-8, -8]`. My blind-retry mutant (`again = result.is_err() && …`) **fails** the scripted row (`left: 5, right: 1`, `kex_retry_tests.rs:449`). Unmutated, all kex rows and `carriage_return_data_matches_native_trust` pass. The classification numbers are supported by the raw logs (see below). |
| **F3** (P3): windows outlived their fixtures | **Closed** | 9,000 ms mutant: **fails** with "…not the 9 s default: 9.0038 s". Mutant removing `start_kill` from `ReapBatch::drop`: **fails** (`left: 1, right: 0`, 30.39 s). `Hangs`, `RunsWithDescendantOnOutput` and `Delay::Forever` now run 3600 s on both platforms. The implementer's extra fix, `the_end_of_a_local_wait_wakes_the_sleeping_supervisor` going from 30 s to 10 s, is correct: the supervisor parks until `host.next_deadline()` (`https_pool.rs:117-135`), and the open's allocation is 20 s. Unmutated, every row passes, the whole set in 1.49 s, so the 120 s server stall adds no run time. |
| **F4** (P3): the record | **Closed** | README §1 and §4 corrected in place, and the decision request withdrawn (lines 35-37, 101). The `7f7f9149` message lists the four expiring bounds that changed and the fixture origin of `PATH=System32`. |
| **F5** (P3): receipt did not bind to the commit | **Closed** | I recomputed the `runner/tree_digest.sh` digest from the commit: the 9 modified files' diff plus the then-untracked `observed.rs`. It gives `891b15a7…c418bc`, the value recorded three times in `raw/round1/digest-*.txt`. I also hash-compared the 10 changed files on dabeest (`E:/gwz-tests/winflakes-20261016-{k,l}/ws/gwz-core`): identical on both hosts. Receipt `-k` failures are exactly the 7 named host gaps, and the receipt passed. |
| **F6** (P3): generic environment names on the ignored probe rows | **Closed** | The probes read `GWZ_TEST_PROCESS_TREE_PROBE_OUT` and `…_HANDLE` and return when either is missing; nothing panics or writes any more. |

## F2 classification: checked against the raw logs

`raw/dabeest/winflakes-20261016-l/e4{a,b,c}/`, using `tries.log` and the KEX-TRY lines in the kept outputs:

| Loop | Runs (all `rc=0`) | clean | recovered (first `-8`, cured by D14) | double `-8` | other |
|---|---|---|---|---|---|
| e4a | 1,688 | 26,805 | 203 | 0 | 0 |
| e4b | 2,072 | 32,915 | 237 | **2** | 0 |
| e4c | 1,056 | 16,763 | 133 | 0 | 0 |
| **Total** | 4,816 | 76,483 | 573 | 2 | 0 |

- **Totals match the README.** The total is 77,058, which is 4,816 × 16 plus the 2 retries. The README's 575 first-try `-8` is 573 + 2.
- **Kept outputs agree.** The counts of `recovered` and `double` lines in the kept outputs equal the `tries.log` counts.
- **Each double was absorbed correctly.** Both doubles are followed by a clean try in the same run.
- **No refused retry.** A refused D14 retry would show as a single `[-8]`, which is class "other". None occurred, which supports the claim that no other failure class appeared.
- **The loop ran the final code.** It used host `l`'s build, and host `l`'s 10 changed files hash identical to `7f7f9149`.

The residual double-`-8` rate is about 1 in 38,500 establishes. It now goes to the operator as a D14 follow-up instead of being absorbed silently.

## The new code: `observed.rs`

- **Test-only.** It is declared inside the `cfg_if!` `if #[cfg(test)]` arm of `ssh_network.rs:566-570`. The only change to `ssh_network.rs` is that `mod` line, so the production `establish` path is untouched.
- **Same path as production.** `establish_observed` calls the same `establish_inner(key, &key.host, …, read_regular, resolve, …)` as `establish_written`. Its `Network` is the same as `Network::production` except that `Native` is wrapped in a pass-through `Recording`, with `KexRetry::PLATFORM` and `credential_offered = || false`. `Recording` returns the inner result unchanged and skips `-37` (would-block).
- **Retry condition is narrow.** It retries only when the result is `Err`, the record is exactly `[Some(-8), Some(-8)]`, and fewer than 5 tries have been made.
- **Repository rules.** No bare `#[cfg]` or `cfg_attr`; only a nonconditional `derive`. No statics, thread-locals or globals: state is a per-call `Arc<Mutex<_>>`. All control flow is braced. It sits under `src`, not `tests/`.

## Evidence base

- **Read in full:** the `7f7f9149` diff, its commit message, README §9, `runner/exp4.sh`, `runner/tree_digest.sh`, and the receipts `-k` and `-l`.
- **Read in the post-change tree:** `ssh_network.rs` (`establish_written` and the test arm), `https_pool.rs` (`supervise`), and `job_budget_wait_tests.rs` (the wake row).
- **Not read:** the `raw/round1/mac` gate logs and the implementer's mutant logs. I reproduced all four mutants myself instead.
- **Mac builds.** Two builds, using `git archive 7f7f9149` in the scratchpad, `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewflakes-2` and `--cfg gwz_transport_candidate`.
  - My first unmutated "rebuild" was a no-op: `git archive` mtimes are older than the existing build artifacts, so cargo reused the mutant binary. I discarded those results, touched the sources, rebuilt, and confirmed the binary has no `MUTANT` code before the green run.
  - The target dir and scratch copy are deleted, and no `reviewflakes-*` dir is left.
- **Orphaned helpers.** The no-kill mutant twice left an orphaned `/bin/sleep 3600` on the Mac (its cwd was the test's tempdir). I identified each one by that cwd and ended it by PID; none is left. Only the mutant does this.
- **dabeest access.** Read-only `git hash-object --stdin-paths` over ssh.
- **Tuple, start and end (unchanged):**

| Repository | Commit | Working tree |
|---|---|---|
| lane root | `d3e7ed14` | 5 untracked |
| gwz-core | `7f7f9149` | 2 untracked, including the filed review |
| gwz-core-evidence | `e127a84d` | 1 untracked |
| gwz-transport | `a27817b9` | clean |
| git2-rs | `d13951f7` | clean |
| gwz-sspi | `d3038882` | clean |

## Open finding

**N1 (P3, new): broken evidence pointer.**
- **Location:** README §9 F5 (line 263) cites `raw/receipts/winflakes-20261016-k/receipt.json`, and the coordinator's brief repeats that path. The path does not exist; the receipt is at `raw/dabeest/winflakes-20261016-k/receipt.json`.
- **Impact:** someone following the README cannot find the receipt for `-k`.
- **Correction:** fix the path in the README, or copy the receipt into `raw/receipts/` as was done for `a` to `j`.

## Follow-ups (not blocking)

- **D14 residual for the operator:** a double `-8` in about 1 of 38,500 establishes on transport-route Windows. The question is whether to add a second retry or accept the rate.
- **Hour-long fixtures orphan on failure:** if a real kill regression hits, the `Hangs` and `sleep 3600` helpers live on for up to an hour on the CI runner.

## Next action

Fix N1 in the evidence README; that needs no re-review. Then gwz-core `7f7f9149` (on `263701cf`) can be merged and pushed under the lane's normal process.
