# State Review: gwz-core `20d0f7cc` (D33, four-worker gwz-https runtime)

- **Object:** gwz-core `20d0f7cc97a68947b91fadb8a0f616b8687af904`, one commit over `4f4b7e01`, lane `/Volumes/projects/limbo/gwz-dev-https611`. 3 files, +108/−3.
- **Date:** 2026-10-18
- **Axis:** State (concurrency, races, ordering, shutdown and drain, cancellation, resource lifetimes, fail-closed direction)
- **Verdict: GO.** P0 0, P1 0, P2 0, P3 3.

## §0 Evidence base

**Read fully:**
- the diff, and `runtime_tests.rs`
- `transport_host/https_endpoint.rs`, `https_endpoint/poll.rs`, `wake_tests.rs`, and `retry.rs` around the preparing spawn (lines 560–657)
- `git/endpoint/https_pool.rs`, `https_connection.rs` (all of `HttpResource`, including `poll_slot`, `poll_connected`, `poll_dispose` and the Drops), `https_worker.rs` and `https_wake.rs`
- `shutdown_watch.rs`, `https_tls.rs`, `https_operation.rs`, `https_handshake.rs` and `https_backend/native_stack.rs`
- `https_worker/credentials.rs`
- `https_auth/owner.rs`, plus `runner.rs` (the launch and `hold_spawn_gate`) and `git/spawn_gate.rs`
- `ssh_pool.rs` (`PoolHost::step`, `actions`, `release`, Drop)

**Read in part:**
- `prepare.rs` (route admission, challenge carry, redirect and cleanup loop)
- `serve.rs` (request build, `renew_after_hold`, `serve`)
- `throttle.rs` (Gate and `pause_until`)
- `governor.rs` (book lock, `sync`, `reconcile`, Observer)
- `shared_reservation.rs` (`ReservedConnector`), the `agent_job.rs` lock and wake points, `https_local.rs`, `native/sspi.rs`
- the session and `mod.rs` HTTPS construction
- `close_tests.rs`, and the D33 evidence Addendum

**Checked:**
- The committed diff and test file are byte-identical to the evidence's `gwz-core-d33.diff` and `runtime_tests.rs.txt`.
- The `receipt.json` sha256 matches `a4734e4f…` (base `4f4b7e01`).
- `MODULE.bazel` and `BUILD.bazel` pin no tokio features, so Bazel cannot miss `rt-multi-thread`.

**Commands** (target `/Volumes/projects/limbo/build-targets/reviewd33-state`, the only one; deleted at the end; `run_r4bg_aggregate_gates.py` not run):
- `RUSTFLAGS="--cfg gwz_transport_candidate" GWZ_TEST_GIT=real cargo test --lib --no-run`: built in 5 m 19 s.
- `transport_host::https_endpoint::`: 47 passed.
- `runtime_tests` 15 times at `--test-threads 1`: 15 of 15 passed, deterministic.
- All tests matching `https`, three times at load 41–52: 460 passed, 2 failed each time. The two failures are `production_*_real_validator` / `real_sspi_request_validator`, which panic with `expect("external target required")` because they need `CARGO_TARGET_DIR` set at run time. They are a harness artifact: rerun with it set, all 59 `production_` tests passed. So all 462 HTTPS tests are green.
- `transport_host::` at `--test-threads 32`: 264 passed, 1 failed. The failure is `close_tests::command_exit_does_not_wait_for_a_close_in_flight`. It passes alone 5 of 5 (0.07 s). It builds `TransportRuntime::new(..)` with `https = None`, so no HTTPS endpoint exists in that test. It is not caused by D33; this matches the evidence's attribution to load.

**Tuple:**

| | Start | End |
|---|---|---|
| gwz-core | `20d0f7cc` (clean apart from the untracked `dev-docs/GwzRemoteTransportBugReport.md`) | unchanged |
| gwz-transport | `a27817b` | unchanged |
| gwz-sspi | `75c1b03` | unchanged |
| git2-rs | `d13951f` | unchanged |
| gwz-core-evidence | `471b66d4` | `c88dcba8` |
| lane root | — | `725f7236` |

The evidence member and the lane root moved while I worked, by other activity; I wrote nothing. The object is unaffected. One accidental side effect: a stray `/tmp/x1` from a diff command, which I deleted.

## Findings

### P3-1: `runtime_tests` passes vacuously when `RUNTIME_WORKERS ≤ 1`
- **Root cause:** the probe count is the constant under test: `Gate::new(RUNTIME_WORKERS)` and `(0..RUNTIME_WORKERS)` in `src/transport_host/https_endpoint/runtime_tests.rs:61-69`.
- **Violated invariant:** the test claims "one task that blocks its thread does not hold the others". With one member there are no others. `pass()` returns `true` at once, on `current_thread` and on `multi_thread(1)` alike.
- **Interleaving:** someone later sets the constant to 1, or to `available_parallelism()` on a 1-vCPU CI runner. The test stays green while handshakes serialise again, which is exactly the D33 defect.
- **Impact:** only the regression guard is lost. Production behaviour is fine today.
- **Correction:** decouple the probe from the constant. Use a fixed `const PROBES: usize = 2` (or 4), plus `assert!(RUNTIME_WORKERS >= PROBES)` or a `const _: () = assert!(..)`.
- **Regression test:** the same test with the fixed probe count. Checking it means temporarily setting `RUNTIME_WORKERS = 1`; it must then be red.

### P3-2: the regression test pins the runtime, not where the handshake runs
- **Root cause:** D33 is fixed by two facts together: the runtime has 4 workers, and the TLS handshake runs on its own task, spawned at `src/git/endpoint/https_connection.rs:454` (`self.connecting = Some(tokio::spawn(..))`), outside the `Mutex<Host>` (`https_pool.rs:31`). The test covers only the first.
- **Violated invariant:** members' synchronous handshake work must not serialise.
- **Interleaving:** a refactor drives `connect()` inline from `HttpResource::poll_connected`, i.e. under the host lock in `PoolHost::step`. Every handshake then serialises on the host mutex across all 4 workers, and the test stays green. The evidence says this openly: "a test of the property … not of TLS".
- **Impact:** a coverage gap; nothing is wrong now.
- **Correction:** a connector seam test. Use a `Connector` whose connect future blocks its poll on a shared gate (like `Gate`), run through the real `HttpsPool`, and require N concurrent setups to be inside at once.
- **Regression test:** that test, red if the connect is polled under the host lock.

### P3-3: the Observer's ordering invariant names a property that no longer holds
- **Root cause:** `src/git/endpoint/ssh_pool.rs:52-54` reads "Told, by the one thread that drives a pool host, of every connection's state change in the order it happened (F10)". For HTTPS this was literally true before D33: the supervisor and `lease()`'s steps all ran on `gwz-https`. Now any of 4 workers steps the host.
- **Violated invariant (text only):** F10 ordering still holds, but only because every observer call is made under `Mutex<Host>`. The comment does not say so.
- **Interleaving:** a future change calls the observer, or reads host-derived state, outside the lock and trusts "one thread". Two workers would then deliver `Seen::*` events out of order.
- **Impact:** a maintenance and diagnosability hazard, bounded.
- **Correction:** reword it to "under the host's lock, by whichever thread steps it". Optionally add a debug assertion that the observer is called with the lock held.
- **Regression test:** none needed beyond the existing `events_tests`.

## Attacks that failed

- **Lock order with truly concurrent holders.** I traced every edge:
  - host → book (Observer) → pool inner (`control.*`)
  - host → pool inner (driver)
  - host → `ReservedConnector` inner → Authority
  - host → agent_job `Supervisor`
  - Operations.state → routes → pool inner (`Credential::drop` → `retire_https_scope`)

  Nothing takes the host lock while holding book, routes, Operations, pool inner or the Supervisor. `HttpLease::finish` and `ChallengeLease::drop` take the host lock with no other std lock held. `agent_job` and `PoolWake` wake after releasing their locks. No cycle exists.
- **`poll_dispose` try_lock, the 5 ms loop.** Connection `Arc` clones are made only in `adopt` under the host lock, and `poll_dispose` runs under that same lock. So `strong_count == 1` cannot be invalidated between the check and the second `try_lock`. A held connection re-polls every 5 ms on its own `Sleep` with the PoolWake waker. The holder progresses independently, so there is no livelock.
- **The new window in `finish(Reusable)`.** `reusable = true` is now visible to `poll_idle_lost` before the lease is released. That path is designed for it: `idle_closed` gives `WrongState` and the phase becomes `Lost`, after which `release_lease` discards. The window `reusable && !released` is new, but its outcome is already designed and correct.
- **Lost wakeups.** `PoolWake` and `CloseWake` use `Notify::notify_one` (the permit is kept). `operation_within` enables `freed` before it checks. `Watch` re-registers on every step. `stopped` is Release and Acquire.
- **Relaxed atomics.** These are `steps` (test-only), `attempts` (an id RMW), `stall_ms` (a value, already cross-thread before D33), the informational counter, and the progress counters (single-location, monotonic). None publishes other data.
- **The shutdown read order** (`Endpoint::shutdown`). The slot release is a semaphore `fetch_add(Release)`, read back by `available_permits` (Acquire), and the transfer is under the `pending` mutex. So "no false zero" holds across threads. `Prepared` drops its lease before `_slot`, and physical count stays above zero until disposal.
- **Runtime drop and deadlock.** The `Runtime` is dropped outside async context, only after the shutdown loop reports 0. Connecting tasks are joined by `poll_dispose`, so the runtime is not dropped mid-handshake on the normal path. No worker waits on the `gwz-https` thread. The Drops on workers (`HttpResource`, `Connection` abort, `HelperJob` → `retain_pending`, `ReapBatch`, `ChallengeLease`, `Credential`, `PokeOnDrop`) are not thread-affine. SSPI work runs on gwz-sspi's own supervisor threads, and TLS objects are only used serially.
- **Blocking calls.**
  - `SharedTls::connector()` runs only in the setup `Job` thread or `spawn_blocking`.
  - DNS runs in the `Job`.
  - Helper spawning is synchronous but short.
  - The synchronous chain check still runs in a poll. Four of them can occupy all workers, and the `block_on` thread does not drive the timer in multi-thread mode. Each stall is bounded by one poll and strictly better than the old single thread.
  - Late timers only delay failures. The publication check reads a fresh clock under the mux lock, so it fails closed.
- **D19/S3.1 clocks.**
  - The connect deadline is a `select!` inside the connect task, as before.
  - Stall progress is a 2 ms tick on a coherent counter.
  - Both host-step paths read `now` under the host lock.
  - Governor time arithmetic is all saturating, and non-monotonic `now` from the placement thread already existed before D33.
- **Spawn gate (D25).** `try_hold` is mutex-exclusive, and the hold goes from `try_hold` to `drop(hold)` with no await in between, so two workers cannot share a window. No runtime path calls the blocking `hold()`. gwz-sspi launches on its own threads; that residual window is D31's, not D33's.
- **Test determinism and vacuity.** The tests that inject `preparing` tasks loop until ready. The tests with their own runtimes are untouched. 462 of 462 HTTPS tests are green. `runtime_tests` passed 15 of 15. Its red output (`[false,false,false,true]`) matches the Gate's logic.
- **Repository rules.** Control flow is braced, modules are registered through `cfg_if` only, there are no globals or thread-locals, and nothing is added under `tests/`.

## Follow-ups (not blocking)

1. **Revocation and AIA retrieval in Schannel or Security.framework (unverified).** If enabled, an unreachable CRL/OCSP endpoint could hold a worker for each handshake for seconds. With four or more such handshakes, the supervisor, timers and helper lookups stall. Measure whether native-tls does online retrieval. It is not a D33 regression.
2. **Thread names.** `thread_name("gwz-https-worker")` also names the runtime's blocking-pool threads (TLS prebuild, file reads), so profiles merge them with the workers. Use distinct names, or note it in the doc comment.
3. **Harness.** The two `real_validator` tests need `CARGO_TARGET_DIR` exported at run time, not just at build time. The test runner presumably sets it; running the binary directly fails.
4. **SSH close test.** `command_exit_does_not_wait_for_a_close_in_flight` (1.5 s bound) fails under heavy parallel load. It involves no HTTPS endpoint, so it is a load-sensitivity issue for its owner.

## Next action

This axis gives GO. The three P3s can be closed in a small follow-up: decouple the probe count, add a connector-seam test, and reword the comment. None of them needs a re-review on this axis.
