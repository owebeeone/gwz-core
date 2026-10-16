# Adversarial review: gwz-core `263701cf`: Windows candidate suite flake fixes (test-only)

**Object:** gwz-core `263701cf`, one commit over `1c16373f`, in lane `/Volumes/projects/limbo/gwz-dev-winflakes`. It changes 83 files (+443/−288).
**Date:** 2026-10-16
**Verdict:** **NO-GO.** P0: 0, P1: 0, **P2: 2**, P3: 4.

Most of the widening holds up. The NO-GO comes from two P2s:
- One row no longer proves its named claim, and I reproduced that by mutation.
- The CNG test retry hides a failure that has already survived the product's own D14 retry.

---

## §0 Evidence base

**Read in full:**
- The whole diff (`git diff -U4 1c16373f 263701cf`, 3,139 lines).
- The commit message.
- The evidence `README.md`.
- The receipt `-j` JSON.

**Read in the post-change tree** (the rows behind every wait I flagged, plus the product code each one depends on):
- `ssh_network.rs` (`establish_inner`, `handshake`, `clean`) and `ssh_network/kex_retry.rs`.
- `https_auth/owner.rs` (`reap_pending`, `ReapBatch::drop`) and `runner/environment.rs`.
- `agent_pipe.rs` error mapping.
- `https_worker.rs` endpoint constructors.
- `gwz-transport/src/pool/mod.rs` defaults.

**Sampled:**
- dabeest run logs, with a grep for the `carriage_return` failure.
- `dial_retry_tests.rs`.
- The other `--ignored` probe conventions in `src`.
- `gh run view --log-failed` for CI runs 38746214496 and 38776718017. Both failed in `ssh_fixture::dial_retry_tests` rows, which an earlier commit already handles. They did not fail in a row this commit touches.

**Not read:** `waits-changed.md`, `audit-raw.txt`, `audit-extra.txt`, `widened.json`, the `runner/*` scripts, and the Mac gate logs.

**Commands and results:**
1. **Receipt binding (read-only on dabeest).** I hashed the receipt's staged tree `E:/gwz-tests/winflakes-20261016-j/ws/gwz-core` with `git hash-object --stdin-paths` and compared it with `git ls-tree -r 263701cf`. Of 2,092 tracked files, 2,091 are identical. The exception is `process_tree/windows/tests.rs`, and its only difference is one comment line (365: "(OpenSSL's)" against "`0xC0000135` on dabeest"). The staged tree also has two extra `__pycache__/*.pyc` files. One side effect: I wrote a file list to a temp file in dabeest `/tmp` and removed it in the same command.
2. **Mutation runs on the Mac.** I built `git archive 263701cf` in the scratchpad, with symlinks to the lane's siblings and `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewflakes-1`, using `RUSTFLAGS=--cfg gwz_transport_candidate` and `cargo +1.95.0 test --locked --lib --no-run`. The build exited 0. I then ran two mutants (results under F1 and F3). Afterwards I deleted the target dir and the scratch copy; no `reviewflakes-*` dir is left.

**Tuple, start and end (unchanged):**

| Repository | Commit | Working tree |
|---|---|---|
| lane root | `5a2666fd` | 5 untracked dev-docs |
| gwz-core | `263701cf` | 1 untracked `dev-docs/GwzRemoteTransportBugReport.md` (not mine) |
| gwz-core-evidence | `c5c12423` | 1 untracked |
| gwz-transport | `a27817b9` | clean (end) |
| git2-rs | `d13951f7` | clean (end) |
| gwz-sspi | `d3038882` | clean (end) |

I recorded only the first three at the start; I checked the last three at the end only.

---

## Findings

### F1 (P2): `cancelling_receive_pack_challenge_helper_has_no_publication_effect` no longer proves the cancel does anything

- **Location:** `src/transport_host/https_policy_tests/cancellation.rs:131`. The post-cancel wait went from `timeout(2 s, pending)` to 30 s.
- **Root cause:** the observation window now outlasts the fixture's own natural end. The helper is `touch "$BLOCKED"; exec /bin/sleep 10`, and the default interaction bound is 120 s (`gwz-transport/src/pool/mod.rs:57`). Without a working cancel, the helper exits by itself after about 10 s with no credentials. The open then fails `Err` anyway, and `post_count` stays 0.
- **Violated invariant:** a row named for a cancel must fail when the cancel has no effect.
- **Reproduction:** I deleted the `request.cancel();` line. The row **passes in 10.65 s** on `263701cf`. With the old 2 s window it would panic at `timeout(..).unwrap()`.
- **Impact:** a regression where cancel no longer reaches a running credential helper during a receive-pack challenge passes CI. The sibling row (`cancellation_wakes_pending_https_open_and_retires_its_owner`, 250 ms bound) covers a different path, with no helper and `auth: None`.
- **Correction (either is enough):**
  - Assert that the receipt's `failure.code == Cancelled`, as the sibling does.
  - Or keep the helper's lifetime well above the window, for example `sleep 3600`.
- **Regression check:** rerun the delete-the-cancel mutant. It must fail.

### F2 (P2): the `carriage_return_data_matches_native_trust` retry hides a failure the product's D14 retry did not cure, and the failure was never classified

- **Location:** `src/git/endpoint/ssh_tests/network.rs:414-424`, which retries `establish` up to 4 more times when `matched`.
- **Root cause:** the test's `establish` is the production path. It goes `establish_written` → `establish_inner` → `Network::production`, and `KexRetry::PLATFORM == Once` on Windows (`ssh_network.rs:56`, `kex_retry.rs:28-38`). So every libssh2 `-8` is already retried once inside the call. Any failure the row sees is therefore one of:
  - two consecutive `-8` results;
  - a non-`-8` handshake error, which `handshake` maps to `Ended::Other` and does not retry (`ssh_network.rs:364-365`);
  - a refused retry, because `has_room_since` found no budget.

  `clean()` strips the error down to `io::ErrorKind` (`ssh_network.rs:550-560`), which is why the log only shows `result=Some(Kind(Other))` (`winflakes-20261016-i/logs/run-q1.log:5218`). The README's "about one in a hundred" is the rate *before* the retry (D14, D22). One observed failure in roughly 560 post-D14 connections (trees c to i, 16 per run) is unlikely if retries were independent (p ≈ 0.02). That is weak but real evidence of a residual class.
- **Violated invariant:** a test-side retry may absorb only a failure class the operator has accepted, and only after it is identified.
- **Impact:** a residual Windows establish failure on the transport route, which would fail a user's clone or fetch, becomes invisible. Up to 10 handshakes per expected connection now hide it, and nobody knows whether D14's single retry is enough.
- **Correction:**
  - Wrap `Native` in a recording `Steps` through `establish_inner`, so each handshake's libssh2 code is logged.
  - Retry only on an observed double `-8`, and print the codes otherwise.
  - Put the residual-rate observation to the operator as a D14 follow-up.
- **Regression check:** a scripted `Steps` that returns a non-`-8` error must fail the row on the first try.

### F3 (P3): several widened windows now meet or pass the fixture's own natural end, so the configured bound or the kill is no longer distinguished

Each instance is bounded; the named claim mostly survives.

- **`https_worker_tests/failure.rs:155`, `stalled_post_response_times_out_after_endwrite_without_replay`:** the window went from 5 s to 30 s. The configured I/O bound is 1 s, the default is 9 s (`https_worker.rs:475`), and the server stalls 15 s. I mutated the row to use 9,000 ms, modelling "the configured bound is ignored after endwrite". It **passes in 9.13 s**; the old 5 s window would have failed. Headers-phase fidelity is still pinned by `https_budget_tests::active_io_custom_timeout_expires_delayed_headers`; the post-endwrite phase is not pinned anywhere now.
- **`https_auth/test_support.rs:184`, `aborting_reap_preserves_child_and_permit_ownership`:** the second `reap_pending` went from 5 s to 30 s. `reap_pending` only waits. The kill comes from `ReapBatch::drop` (`owner.rs:177-190`), and `Behavior::Hangs` is `ping -n 30` (about 29 s) on Windows. A regression that drops that `start_kill` would now pass on Windows when the helper exits by itself. I did not reproduce this one; it is reasoned from the code.
- **Correction:** keep each window under the fixture's natural lifetime, or lengthen the fixtures (`Hangs` to 3600 s, the server stall to more than 30 s). For the HTTPS row, also assert `elapsed < 9 s`.

### F4 (P3): the commit message and README state two things that are not accurate

- **Commit message: "every ... expiring deadline ... is as it was."** Four expiring bounds changed:
  - `runner/process_tests.rs:84`: 1.5 s → 10 s.
  - `runner_tests.rs:276`: 4 s → 10 s.
  - Projection interaction bound: 500 ms → 2,000 ms.
  - Native exchange I/O: 60/40 ms → 600/400 ms.

  The README discloses all four; the permanent record contradicts it.
- **"The owner's PATH=System32" (commit message; README §1 and §4, which also propose a "product consequence worth a decision").** `PATH=System32` comes from the test fixture's `command()` (`process_tree/windows/tests.rs:36-47`). The product hands helpers the captured environment snapshot (`runner/environment.rs:17-60`). No product behaviour is involved, so the proposed operator decision rests on a false premise.
- **Correction:** add a README correction and archive a note next to the commit. The decision request should be withdrawn.

### F5 (P3): receipt `-j` does not bind to `263701cf` by itself

- **Location:** `raw/receipts/winflakes-20261016-j/receipt.json`. It records `commit: 1c16373f`, `source: worktree`, and no tree digest. It started at 16:53:56; the commit is dated 18:06:32 +1100.
- **Impact:** the `Windows-receipt:` trailer can only be checked while the dabeest staging dir survives. I checked it: identical except for the one comment line and two `.pyc` files.
- **Correction:**
  - Record this verification, or a tree digest, in the README.
  - Have receipts for worktree runs record a `git write-tree`-style digest.

### F6 (P3): the ignored probe rows are gated on generic environment names

- **Location:** `process_tree/windows/tests.rs:270-311`, which reads `OUT` and `PROBE`.
- **Root cause:** the other probes in this crate use namespaced variables: `GWZ_TEST_AGENT_WAIT_SOCKET`, `GWZ_SESSION_HOST_ENVIRONMENT_PROBE`, `GWZ_TEST_NATIVE_CLONE_*`.
- **Reproduction:** on Windows, run `cargo test -- --include-ignored` in a shell where `OUT` names a file. `probe_breakaway` overwrites that file. If `OUT` is set and `PROBE` is not, `probe_handle_target` panics.
- **Impact:** low, since no CI job runs ignored rows.
- **Correction:** rename to `GWZ_TEST_PROCESS_TREE_PROBE_OUT` and `GWZ_TEST_PROCESS_TREE_PROBE_HANDLE`.

---

## Invariant analysis: attacks that failed

1. **Must-expire deadlines and "success path" job deadlines.** All the `Job::start_isolated(Some(..30 s))` rows are either success paths or end on another cause that the row asserts: `PermissionDenied`, `Other`, a panic, or a refusal. A hang would surface as a different `kind`.
   - `late_loader` and the `supervised` overrun rows widen the window over a cleanup bound of 10 to 1,000 ms. That bound's timing stays pinned by `cleanup_deadline_wakes_a_pending_disposal_waiter`, which is unchanged (wake within 80 ms).
   - `message_deadlines_tighten_pool_policy...` still asserts `calls[0] < 500`.
   - `route_wait_reduces...` keeps `elapsed < 1300 ms`.
   - `release_wake` keeps `PROMPTLY` = 500 ms.
   - `queued_expiry...` uses a fake clock and asserts `calls == 0`.
2. **Negative waits.** None was shortened; the only shortened wait is the probe's positive 60 s → 30 s. `assert_stopped` is unchanged. `assert_alive` is still a positive check, so its meaning is unchanged.
3. **The 1.5 s → 10 s lookup.** It still requires `Timeout`, a started descendant, and a stopped descendant afterwards.

   **`selected_key` with 25 logins.** The guarded regression is a slept 20 ms quantum at each of two exchanges, so at least 40 ms on every try; best-of-25 still fails it. Residual risk: a random-phase quantum was already only about 11% detectable with 5 tries and is now about 0%. I note it, but it is not a finding.
4. **The PowerShell replacement.**
   - `std::process::Command` with `creation_flags(CREATE_BREAKAWAY_FROM_JOB)` makes a real `CreateProcessW` call inside the job, and `err=5` proves the process is in a job and not allowed to break away.
   - The handle row keeps its inheriting control and now asserts that the probe actually ran (`path=`).
   - The ignored rows run only with `--ignored`; no CI job uses that flag.
   - The PATH override hides no product behaviour (see F4).
5. **The six fixes.**
   - `PipeAgent` `ERROR_NO_DATA` is server-side fixture state only. The product client already types `ERROR_NO_DATA` as a closed pipe (`agent_pipe.rs:374`).
   - The `0000` fix is correct: it reads until the length of the reference stdout, then compares exactly.
   - The allocation-timeout race is in the test's observation. (Pre-existing: its "second never set up" check reads a counter that is incremented asynchronously, with no settle.)
   - The 16 ms tick allowance is still nonzero and finite.
   - Interaction at 2,000 ms is still reported exactly.
   - The I/O pair at 600/400 ms still separates "reset per round", which would reach the round cap and give `Protocol`.
   - The CNG fix is F2.
6. **Repository rules.** No added `#[cfg]`, `cfg_attr`, static, `OnceLock` or `thread_local`. The new probe items sit in an existing `cfg_if` `cfg(test)` module (`process_tree/windows.rs:188`). Every touched file compiles only under test. Rust enforces braced bodies.
7. **Suite time.** Deterministic growth, on every platform: about +8.5 s (`process_tests`), +6 s (`runner_tests`), +1.5 s per Interaction operation, and about +0.7 s for the exchange row. Full-suite wall times in the table did not materially change (quiet runs: 4:53–5:41 before, 5:06–5:56 after).
8. **Evidence.** The `-j` tree matches the commit (§0.1). The only gap is that the receipt does not bind itself (F5).

## Follow-ups (not blocking, outside scope)

- `PipeAgent::wait_client_closed` checks a cumulative `left > 0`, so any second check in a row is vacuous.
- `an_allocation_timeout_is_returned_once_and_moves_nothing` needs a settle before its negative `starts == 1` read.
- `gwz-sspi/tests/native/completion.rs:348` has a 2 s readiness `until`, already listed in the README.

## Next action

The implementer should fix F1 and F2 (correction and regression check as given in each finding), then file F3 to F6 as corrections or follow-ups. A Windows receipt that binds to the corrected tree should follow. A re-review is needed only for F1, F2 and the new receipt.
