# gwz log path-cursor spawn gate (gwz-core 650dd4e0, gwz-py 02d79ca, gwz-cli cea7bd4) — STATE-AXIS REVIEW
**Review object / Baseline / Date 2026-10-19 / Axis**
- **Object:** gwz-core `650dd4e0`, gwz-py `02d79ca`, gwz-cli `cea7bd4`, and the evidence run gwz-core-evidence `7f44d19d` `campaigns/transport-qualification/runs/2026-10-19-loggate/`, with receipt `loggate-20261019-a`.
- **Baseline:** gwz-core `9e8edb80`, gwz-py `5b6b665`, gwz-cli `5193db9`.
- **Axis:** State (concurrency, lifecycle, failure direction). The review is independent, adversarial and read-only. The other axis runs in parallel. This report is filed verbatim.

**Verdict: GO** — P0 0, P1 0, P2 0, P3 3

## 0. Evidence base
- **Tuple at start.** All four HEADs and parents match the brief by full sha:
  - gwz-core `650dd4e0ba3f707d47973030c2862df4098ff867` ← `9e8edb80e730e86e44894d79d18cef551f70e6e5`
  - gwz-py `02d79cad937f2d7752362861022cd07e4d1c09a5` ← `5b6b66579867d96d21d5d5d003aaa69f50960b4e`
  - gwz-cli `cea7bd4cabae8b1a840c810a04794e62acb45600` ← `5193db962e672c54cd20ab2ef8448d904129a559`
  - gwz-core-evidence `7f44d19d84bfd44b688ddf6918be56295843590d` ← `c442cf8d…`
  - All four working trees are clean.
- **Code read.** I read all three diffs in full, plus:
  - `src/git/spawn_gate.rs`
  - `src/git/gitbackend/{backend.rs,transport_support.rs,spawn_gate_tests.rs}`
  - `src/git/endpoint/https_worker/native/sspi.rs` (`NativeHost`)
  - `src/git/endpoint/https_auth/runner.rs` (`hold_spawn_gate`) and the `GATE_POLL` constant in `https_auth.rs`
  - `src/operation/commit_log/{merge.rs,handler.rs,request.rs,mod.rs}`
  - `src/operation/resolve_jobs.rs`
  - gwz-py `native/src/{shims.rs,client_host.rs,dispatch/mod.rs,dispatch/log.rs}`
  - gwz-cli `src/log_exec.rs` and `src/globalargs/dispatch.rs`
- **Caller inventory.** `handle_log` has exactly two external callers: gwz-py `dispatch/log.rs:46` and gwz-cli `log_exec.rs:117`. `open_request_histories` and `open_default_head_histories` have no production callers besides `handle_log`. No other production `Command` spawn is reachable from the log path:
  - `diff/output.rs` and `workspace_ops/historical_identity.rs` spawn only in test helpers;
  - `endpoint/helper_script.rs` is `cfg(test)`.
- **Archive check.** `python3.13 -B tools/verify_archive.py` reported "Verified 5553 migration records; no build/cache directories in archive". I recomputed the run's `manifest.json`: 25/25 sha256 match, and no file is unlisted.
- **Patches against commits.** I compared the archived patches with the committed diffs, and they are byte-identical:
  - `patches/gwz-core.diff` equals `git diff 9e8edb80 650dd4e0`, minus the new file;
  - `patches/gwz-core-new-file-path_cursor_gate.rs` equals the committed `tests/path_cursor_gate.rs`;
  - `gwz-py.diff` equals `5b6b665..02d79ca`;
  - `gwz-cli.diff` equals `5193db9..cea7bd4`.
- **Test logs.**
  - Red and mutation logs (`raw/core/red.log`, `raw/core/mutation.log`): the new row fails at `path_cursor_gate.rs:43` with "a pathspec log finished while another hold…". The mutant finished in 0.09 s.
  - gwz-py red is `(false,false,false)`; green is 39/39.
- **Optional run.** `GWZ_TEST_GIT=real CARGO_TARGET_DIR=…/loggate-rev-state cargo +1.95.0 test --lib -- commit_log`: 110 passed, 0 failed. The new row passes. The target dir was deleted.

**Answers to the review areas, where no finding is raised**

1. **Hold scope.** `SpawnGate::spawn` holds the gate only across `command.spawn()`. That covers std's stdio pipe creation and `CreateProcessW`. `wait_with_output` runs after the hold is dropped.
   - **Lock order.** A merge member thread takes its `ReadBudget` permit, then the gate. No gate holder takes a permit or any log state. The holders are the gate's own spawn, the whole-held libgit2 credential-helper call (`transport_support.rs:319`), gwz-sspi's `LaunchGuard` and the auth runner's `try_hold`. So there is no cycle.
   - **Re-entrancy.** The gate is not re-entrant. No holder calls back into a log, and no log path holds the gate while asking for it again.
   - **Credential prompt.** A held credential-helper prompt stalls every log reader for the prompt's length. That is the D25 per-Client coupling. It is bounded by the holder and is not a deadlock.
   - **GIL.** gwz-py releases the GIL (`py.detach`) for the whole call.
2. **Child I/O.**
   - **Stdin** is now piped, where it used to be null. `wait_with_output` drops the child's stdin before waiting, so `git` sees EOF. `rev-list` without `--stdin` never reads it.
   - **Stdout.** `--max-count=1` makes stdout at most one line, and only one stream is piped (stderr is null), so `wait_with_output` cannot deadlock on a full pipe.
   - **Exit status, empty-output and invalid-oid handling** are unchanged from the baseline.
   - **Windows inheritance.** The piped stdin's child end is created inside `spawn`, so on Windows it is inheritable only under the gate.
3. **Failure direction.**
   - The gate has no closed state. `hold()` is infallible, and poison is absorbed by `PoisonError::into_inner`.
   - A spawn error becomes the same `HistoryUnreadable` degradation as before.
   - There is no path to an ungated spawn. `MessagesState::PathWalk` is built only from `RepositoryState::Ready`, which carries a gate by construction.
   - Log is a non-network call. It is not registered with `Operations`, before or after this change, so `close()`, `cancel_operation` and atexit neither wait for it nor cancel it. The change does not alter shutdown ordering.
   - The gate is an `Arc` clone, so a log in flight across `close()` keeps it alive.
4. **The test probe.**
   - It is confined to the `cfg(test)` arm of one `cfg_if!`. The non-test arm is a zero-size no-op.
   - It is keyed by prefix. The live prefixes are `mem_gate_` and `mem_f4_jobs_`. They are disjoint, and no other fixture uses them.
   - `f4_jobs_one_bounds_real_path_reader_lifetimes` still asserts `== 1` under jobs=1. That stays valid with the narrower child-lifetime window.
   - Proof strength is covered in P3-3.
5. **Evidence.** See P3-2.
6. **gwz-py lifecycle.**
   - `Backend::own(self.native.clone())` → `NativeHost::backend()` uses the same `SpawnGate` that `launch_guard` gives the Client's supervisor, and that `caller()` gives the transport. A Client's log and its own fetch are therefore on one gate.
   - The ordinary build (no candidate) gets `Git2Backend::new()`, as before.

## Problem, requirement provenance, and scope
- **Problem.** The baseline `next_unfiltered` (`commit_log/mod.rs`) ran `git rev-list` through a bare `Command::output()`. It was the only std spawn on a gwz-py Client's paths that did not take the Client's gate. That makes it a D25 site: D25 says "gwz-core's std spawn sites on transport paths go through it". It is also within D47's intent: a Client's non-network spawns serialize against its SSPI worker window. The exposure is real only on Windows in transport-candidate gwz-py builds.
- **Necessity.** The fix is the minimal plumbing: the backend goes into `handle_log`, the gate goes into `Ready` and `PathWalk`, and the spawn is gated. The CLI passes `Git2Backend::new()`, a private gate. That is correct because the CLI's log process creates no `NativeHost`: `log_exec::run_log` bypasses `execute_invocation_selected`'s transport branch.
- **Scope.** The change matches the controlling proposal and the operator's "bounded corrective package":
  - D52's source check, the 1.2.0 required-gate migration, gwz-cli's own spawns and Python `subprocess` are untouched.
  - The two allowlist removals are forced by the code change: the cfg-boundary check fails STALE otherwise.
  - The `candidate_switch_inventory.txt` line is forced by the new test module.
  - Nothing outside the authorized boundary was found.

## 1. Findings

### P3-1 — The gate is unfair to a polling launch. The log adds up to 100 continuously cycling gate clients, which could starve the Client's own HTTPS helper launch into a `GateTimeout`. This is a hypothesis, not reproduced.
- **Root cause.**
  - `SpawnGate::hold` is an unordered mutex+condvar. `SpawnHold::drop` sets `held=false` and calls `notify_one`, and any thread that arrives fresh can take the gate first.
  - The WH2 helper launch (`https_auth/runner.rs:192-209`) does not join the waiters. It polls `try_hold` every `GATE_POLL = 1 ms` (`https_auth.rs:32`), with nominal Windows sleep granularity coarser than that, until its lookup clock runs out. It then returns `AuthError::GateTimeout`, which maps to `ErrorCode::Timeout`.
  - Before this change, log readers never touched the gate. Now each pathspec member thread takes it once per log entry: `--max-count=1 --skip=N`, one spawn per entry. Up to `DEFAULT_JOBS = 100` such threads may run (`resolve_jobs.rs:2`; the `ReadBudget` limit).
- **Location.**
  - `gwz-core/src/git/spawn_gate.rs` (`hold`, `Drop for SpawnHold`)
  - `src/git/endpoint/https_auth/runner.rs:192-209`
  - `src/operation/commit_log/mod.rs` (`PathWalk` arm)
- **Invariant at risk.** A launch that is waiting for the gate should get it within its clock while other holds are each one spawn long. The comment on `GATE_POLL` assumes "A hold lasts one spawn, so a wait is short".
- **Reproduction.** Not run, because this review is read-only. The scenario: on Windows, one gwz-py Client runs a pathspec `log` over many members (jobs ≥ ~ (spawn + child)/spawn) while its own HTTPS fetch needs a credential helper launch. Once J·s/(s+c) ≥ 1, the gate is held or contended almost continuously. Here s is the time under the gate (`CreateProcessW` plus pipes) and c is the child's run time. The poller succeeds only if it polls inside the wake-latency gap after a release.
  - A host-neutral test can probe this: use `SpawnGate::with_exclusion(true)`, run 32–100 threads looping `hold → sleep(s) → drop → sleep(c)`, and run one `try_hold` poller at 1 ms with a 2 s deadline. Then measure its success rate.
- **Impact.** If confirmed, the scenario that motivates this fix becomes a liveness failure. The fetch fails with Timeout, where it used to succeed with the inheritance hazard. That is the safe direction (no handle leak), but it is a new failure. gwz-sspi's `LaunchGuard` uses a blocking `hold()`, so it is only delayed, not starved.
- **Correction.**
  - Measure first.
  - If the poller is starved, make the gate fair or give launches priority: a ticket/FIFO gate, or a "launch pending" flag that `hold()` defers to. Alternatively, have the async waiter register instead of polling.
  - Reducing the log's spawn rate is another option, e.g. one `rev-list` per member instead of one per entry. That rate is pre-existing.
- **Regression test.** Add the poller-under-contention row above to `src/git/spawn_gate/tests.rs`, asserting that the poller gets a hold within its clock while the gate is saturated by blocking holders.

### P3-2 — The receipt's binding to the committed tree cannot be recomputed, and the run archives no Windows per-test logs.
- **Root cause.**
  - `receipt.json` names only `"commit": "9e8edb80…"` with `"source": "worktree (…)"`. The worktree-to-commit binding rests on `digest.py`, which the README says is "not archived". Its definition ("sha256 of `git diff HEAD` plus untracked files outside `dev-docs/`") does not say how the parts are framed or ordered. I tried 12 plausible constructions from the committed diff plus the new file, and none reproduced `core-only 7c02e9b7…48d9`.
  - The Windows shape logs (`ordinary.log`, `transport.log`, `hosted-like.log`) remain on `E:/gwz-tests/loggate-20261019-a` and were not archived. Neighbouring runs did archive them: `2026-10-18-gapsb`, `-win92`, `-words2` and `2026-10-19-sspi012-l` all carry their per-shape logs under `raw/windows/`.
- **Location.** `gwz-core-evidence/…/2026-10-19-loggate/README.md` (§Digests) and `raw/windows/`.
- **Invariant.** A reviewer can tie a receipt to the exact committed tree and see the gating row's result on the platform where the gate excludes in production.
- **Reproduction.** Recompute `core-only` from `git diff 9e8edb80 650dd4e0` plus `tests/path_cursor_gate.rs`; there is no defined procedure that matches. Then search `raw/windows/` for `a_pathspec_log_spawns…`; there is no entry.
- **Impact.**
  - The binding is still sound by a second route. The archived patches are byte-identical to the commits (verified here), and the core-only digest is equal at receipt start and after all gates.
  - That the new row ran and passed on Windows can only be inferred: hosted-like reports `failed: 4`, and none of the four named failures is this row.
  - gwz-py's change has no Windows execution at all, because the host lacks gwz-py. Its coverage is by construction: the gwz-py gate-identity row plus gwz-core's forced-exclusion row. The code is platform-neutral Rust over types already used on Windows. Nothing is broken; the run is harder to audit.
- **Correction.**
  - Archive `digest.py`, or state the exact byte framing.
  - Pull the three shape logs from `E:/gwz-tests/loggate-20261019-a/logs/` into `raw/windows/` and add them to the manifest.
  - Add one README line stating that gwz-py's Windows coverage is by construction.
- **Regression test.** None; this is an evidence-process fix. `verify_archive` could optionally require `raw/windows/*hosted-like*.log` whenever a `receipt.json` is present.

### P3-3 — The gate row proves the wait by a 500 ms window. Its second assertion, the probe, is never shown discriminating.
- **Root cause.**
  - `a_pathspec_log_spawns_its_path_cursor_under_the_backends_gate` turns red on an ungated implementation only if the log reaches and completes its spawn within `BLOCKED = 500 ms`; the mutant took 0.09 s.
  - The `path_readers_seen == 0` assertion would catch an ungated child that started within the window but had not finished. No mutant exercises that: as the implementer notes, the "not finished" assertion always fires first.
  - Neither assertion shows that the log is parked in `SpawnGate::hold` specifically. A future gated step elsewhere in `handle_log` would also satisfy both assertions, even with an ungated cursor.
  - The probe's `keys().find(starts_with)` misattributes counts if two watched prefixes ever overlap; `watch_path_readers` does not refuse an overlap.
- **Location.** `gwz-core/src/operation/commit_log/tests/path_cursor_gate.rs:28-55`, and `mod.rs` (`path_reader_probe_enter`, `watch_path_readers`).
- **Invariant.** The row should fail on any implementation where the cursor's `git` child starts while another hold exists, regardless of how fast the machine is.
- **Reproduction.**
  - Run a mutant that sleeps 600 ms before an ungated `Command::output()` in `PathWalk`. The row passes, because the log is still unfinished and no child has been counted at 500 ms.
  - Alternatively, a mutant that takes the gate once in `open_request_histories` while leaving the cursor ungated also passes.
- **Impact.** The proof is valid for the shipped code and the archived mutant, but its strength depends on timing. Correct code cannot pass falsely. Broken code passes falsely only if it is slow or blocked elsewhere.
- **Correction.**
  - Add a `cfg(test)` waiter count to `SpawnGate`, or a hook on entering `hold()`. Have the row wait until exactly one waiter is parked before dropping the hold, and assert `path_readers_seen == 0` at that point. Wait for the event rather than sleeping.
  - Make `watch_path_readers` assert that the new prefix is neither a prefix of nor prefixed by any watched key.
- **Regression test.** Keep the two mutants above in the run's mutation log as must-be-red.

## 2. Out-of-scope observations
- gwz-py still runs `diff` and three `read` dispatches on `shims::no_backend`, a private gate (`dispatch/diff.rs:50`, `dispatch/read.rs:152,313,338`). I found no production std spawn reachable from those gwz-core handlers in the files inspected, so this review raises no exposure. A complete inventory belongs to D52's source check.
- The probe remains a process-global `static OnceLock<Mutex<BTreeMap>>`. It is test-only, carried over from the baseline's single-prefix global, and now widened to a map. That conflicts with the operator's no-globals preference. Passing an observer through the gate or the history would remove it. This is pre-existing and not a release matter.
- The path cursor runs one `git rev-list --skip=N --max-count=1` per entry. That is O(n²) child work per member and pre-existing. It is now also what drives the gate traffic in P3-1.
- gwz-py `run_tests.py --candidate` shows 2 pytest failures: inventory sites and `fn network(` discovery. The run records them as pre-existing at `5b6b665`. I did not re-verify that.

## 3. Tuple check at end
- gwz-core `650dd4e0ba3f707d47973030c2862df4098ff867` ← `9e8edb80e730e86e44894d79d18cef551f70e6e5`, clean.
- gwz-py `02d79cad937f2d7752362861022cd07e4d1c09a5` ← `5b6b66579867d96d21d5d5d003aaa69f50960b4e`, clean.
- gwz-cli `cea7bd4cabae8b1a840c810a04794e62acb45600` ← `5193db962e672c54cd20ab2ef8448d904129a559`, clean.
- gwz-core-evidence `7f44d19d84bfd44b688ddf6918be56295843590d` ← `c442cf8d3e5bce5312ea232e4f89a455213744f8`, clean.
- `/Volumes/projects/limbo/build-targets/loggate-rev-state` has been removed. Nothing in the lane was modified.
