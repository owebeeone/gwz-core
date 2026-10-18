I reviewed the four commits on the code axis and the verdict is **NO-GO**: two P2s and four P3s are open. The API shape itself is right, and the gate does reach the guard. The blockers are where the guard sits relative to the identity recheck, and a gwz-core commit that can't be pushed in its current form.

# Code review: gwz-sspi 0.1.2 launch guard and the D31 host wiring

- **Object:** lane `/Volumes/projects/limbo/gwz-dev-sspi012`, root `9befc11d`
  - gwz-sspi `bd684977` over `75c1b032` (v0.1.1)
  - gwz-core `02d73b40` over `4f4b7e01`
  - gwz-cli `8340ac9f` over `279bf85f`
  - gwz-py `7185f22e` over `8e8446ba`
  - evidence `gwz-core-evidence` `64731bdc`, run `2026-10-17-sspi012`, receipt `sspi012-20261018-j`
- **Date:** 2026-10-18
- **Axis:** CODE (interfaces, public API and semver, call graphs, build shapes, requirement provenance and scope)
- **Verdict: NO-GO.** P0: 0, P1: 0, P2: 2, P3: 4.

## §0 Evidence base

- **Tuple.** Checked at start and end with `rev-parse` and `status` in all six repos. It was identical both times. The only untracked files are the pre-existing ones: gwz-core `dev-docs/GwzRemoteTransportBugReport.md`, the evidence member's alpha run, and five root dev-docs. Nothing in the lane was modified.
- **Read in full:**
  - all four diffs;
  - `SpawnGate` (`git/spawn_gate.rs`);
  - `TransportRuntime::build_native`, `request()`, `runner.rs` `run_finished` and `hold_spawn_gate`;
  - `transport_support.rs` credential paths;
  - gwz-sspi's `monitor::launch`, `context::spawn_launch`, `futures::Start`, `identity::Captured::verify`;
  - gwz-py's `client_host.rs`, `route/transport.rs`, `route/native.rs`, `shims.rs`;
  - gwz-cli's `dispatch.rs`;
  - D24, D25 and D31; the C9 and OQ-H text;
  - `.github/gwz-sspi.commit`, `checkout-siblings.sh`, `transport-candidate.yml`, and gwz-cli's and gwz-py's sspi checkout steps;
  - gwz-sspi's `release_checks.py`, `gearu.toml`, `RELEASE.md`; the evidence README and manifest.
- **Built or ran.** External `CARGO_TARGET_DIR`s, all deleted afterwards.
  - gwz-sspi 0.1.2 (from a scratch `git archive`):
    - clippy `-D warnings`, all targets, host and `x86_64-pc-windows-msvc`: clean;
    - `cargo test --all-features --locked`: 84 lib tests and all integration tests pass;
    - `cargo package --list`: the `docs/` folder and the CHANGELOG ship in the crate.
  - Auto-trait probe on 0.1.1 and 0.1.2 (Send, Sync, UnwindSafe, RefUnwindSafe and Unpin, for `Supervisor`, `Conversation`, `CallerCapture`, `Options`, `ShutdownReport`): identical.
  - gwz-core `check --lib --tests --locked` in the ordinary, transport and session shapes: no errors, and no warnings in changed files.
  - gwz-core transport-shape tests filtered on `spawn_gate`: 15 pass.
  - gwz-cli and gwz-py `check --all-targets --locked` in the ordinary, transport and session shapes: no errors.
  - gwz-cli `scripts.test_release`: 26 OK. gwz-py `test_release_pins.py`: 22 passed.
  - `check_cfg_boundaries.py` and `check_process_globals.py`: "nothing new".
  - CI simulation and a mutation test (P2-2 and P3-2).
- **Not done.**
  - `run_r4bg_aggregate_gates.py` (as instructed).
  - Any Windows execution, or a Windows type-check of gwz-core, gwz-cli or gwz-py.
  - Recomputing the evidence staging digests (the recipe isn't recorded in `tools/`). Binding the receipt to the committed trees is left to the State reviewer. For what it's worth, the gwz-sspi digest's tracked diffstat (22 files, +477/−34) matches `bd684977` minus the then-untracked `launch_guard_tests.rs`.

## Findings

### P2-1: the launch guard sits between the identity recheck and `CreateProcessW`, so the recheck can be arbitrarily stale

- **Root cause.** In `gwz-sspi/src/supervisor/windows/launch.rs`, `start()` runs `origin.verify()?` (line 185), then `let held = guard.map(LaunchGuard::hold)` (188), then `InheritWindow::open` (189). The host's guard is `gate.hold()`, a blocking wait on a condition variable with no clock. Under D25 the gate is held for the whole of libgit2's credential helper call, which can last as long as a user takes to answer a prompt. Nothing checks the caller again after the wait. `monitor::launch` resumes the worker on whatever `CreateProcessW` returns.
- **Violated invariant.**
  - The SSPI design says "launch rechecks identity and thread liveness" (`dev-docs/GwzSspiDesign.md:242`).
  - gwz-sspi's `docs/Architecture.md:59-60` says the recheck is "immediately before CreateProcess". That doc ships in the crate and wasn't changed; the new paragraph at :81 now contradicts it.
  - The recheck exists because a launch can be delayed after `start` (FIFO capacity queueing). 0.1.2 adds a new, unbounded delay *after* it.
- **Reproduction (Windows, credible by construction).** Build a `Supervisor::with_launch_guard` whose callback blocks on a channel. Call `start` on thread T. While the launch thread is blocked in the guard, make T call `ImpersonateSelf`, or exit. Then release the channel.
  - 0.1.2 creates and resumes the worker and returns a `Conversation`.
  - With the check after the guard, the same sequence refuses with `IdentityMismatch`, as 0.1.1 did when the change happened before its recheck.
- **Impact.** For a guarded host (every gwz-cli and gwz-py native route), a worker can authenticate as the process logon on behalf of an original thread that has died or started impersonating. That is exactly what the recheck is there to refuse. The window is as long as any gate hold, and a credential prompt can hold it for minutes. If 0.1.2 is published like this, the behaviour and the contradictory docs are frozen into an immutable release.
- **Correction.** Keep the current pre-guard `verify` (so a refused launch still takes no guard), and call `origin.verify()?` again after `held` is taken and before `InheritWindow::open`; a failure drops the hold. Update `Architecture.md:59-60` and the CHANGELOG sentence to say "rechecked again after the guard is held". The alternative of taking the guard before the only `verify` also works, but it changes the tested "a refused launch takes no guard" property.
- **Regression test.** In `windows/inheritance_tests.rs`: a fake `Origin` whose `verify` succeeds once and fails after that, with a guard that records its calls. Expect `launch::start` to return `IdentityMismatch`, with the guard taken and released and Job `ActiveProcesses == 0`. Mutation: removing the second `verify` must turn the row red.

### P2-2: gwz-core `02d73b40` moves the lock to gwz-sspi 0.1.2 but not `.github/gwz-sspi.commit`, so it can't be pushed as committed

- **Root cause.** `02d73b40` changes `Cargo.lock` to `gwz-sspi 0.1.2` and code that needs 0.1.2's `with_launch_guard` and `LaunchGuard`. `.github/gwz-sspi.commit` still names `75c1b032` (0.1.1). Leaving it alone is right *now*: the bump rule (`.github/gwz-sspi.commit:4-6`) requires the pin to name a commit that is already on gwz-sspi's main, and CI enforces that ("The pinned commits are on their repositories' main" step). But the same rule requires the pin to move *in the same commit* as the change that needs it, so this commit as built can never satisfy the rule.
- **Violated invariant.** The `.github/gwz-sspi.commit` bump rule, and the per-commit "no red intermediate commit" policy (`check_lane_commits.sh` header). This is the same hazard the 0.1.1 State review recorded (`GwzSspi011PipeFix-ReviewState.md:140`); the 0.1.1 cycle avoided it by moving the lock and the pin together in `bb34619d`.
- **Reproduction (run).** I unpacked gwz-core `02d73b40` next to gwz-sspi at its own pin, plus git2-rs and gwz-transport, and ran `cargo metadata --locked --offline`. It failed: "cannot update the lock file … because --locked was passed", exit 101. It also follows that:
  - gwz-cli's Windows CI (`ci.yml:107-117`, which checks out gwz-sspi at gwz-core's pin) fails against `=0.1.2`;
  - the candidate build can't compile `NativeHost`.
  - The evidence wrapper `lane_check_local_sspi.py` was written precisely to get around this. Its docstring says "0.1.0", but the pinned commit is 0.1.1.
- **Impact.** Pushing the tuple as it stands turns every gwz-core `--locked` job red, plus gwz-cli's Windows job, and the ci-only receipts in gwz-cli's and gwz-py's commit trailers rely on that CI. It's recoverable, not destructive. The reviewed gwz-core SHA can't be the pushed one, and the object doesn't record a push order.
- **Correction (record it with the verdict).**
  1. Push gwz-sspi `bd684977` to main (tag `v0.1.2` and publish as D31 authorizes; this can follow, since main builds by path).
  2. Amend gwz-core `02d73b40` so that the same commit moves `.github/gwz-sspi.commit` to `bd684977`. A separate follow-up commit would leave a permanently red intermediate commit.
  3. Run the stock `windows_lane_check.py` (without the wrapper) on the amended commit, to bind a receipt to the pushed tree.
  4. Push gwz-core, then gwz-cli and gwz-py (both read gwz-core's pin or main), then re-record the root lock.
- **Regression test.** The stock lane check on the amended commit, plus CI's existing pinned-on-main and `--locked` steps. Optionally, a cheap source check: the gwz-sspi version in `Cargo.lock` must equal `[package].version` of the pinned commit's manifest. That would have caught this at commit time.

### P3-1: C9 says operations of one gwz-py `Client` exclude each other, but its non-network and off-switch operations use gates of their own

- **Root cause.** gwz-py builds a non-network request's backend with `shims::Backend::own()` (`client_host.rs:109`), and an off-switch request's with `Git2Backend::new()` (`route/transport.rs:185`). Both carry a fresh `SpawnGate::default()`. A `commit` (`repository.rs:301` `git commit`, the CLI route) or a local `git tag` from one Python thread can therefore overlap a worker's creation window from a concurrent fetch on the same `Client`. Under the off switch, git2's credential helper can too.
- **Violated invariant.** The new C9 text says "two operations of one `Client` do exclude each other", and OQ-H is marked "Discharged". This is the same kind of overclaim the 0.1.1 review's P3-3 rated P3.
- **Impact.** The residual exposure in gwz-py is unrecorded. It is not a regression: before D31 the window was ungated.
- **Correction.** Narrow the C9 sentence to network operations on the transport route. Record the non-network and off-switch backends as an operator item, next to `gwz log`. Closing it would need a gwz-core seam (for example a backend built on `NativeHost`'s gate), which is beyond D31's "shares it with HelperSlots" and needs separate authorization.
- **Regression test.** A docs assertion on that sentence. If the seam is authorized: a gwz-py row checking that `Backend::own()` holds the `Client`'s gate.

### P3-2: no test covers the production constructor's guard wiring

- **Root cause.** Every gating test builds the host with `NativeHost::on_gate` and a test builder:
  - `spawn_gate_chain_tests`;
  - the Windows `sspi_window` `gated()` phase;
  - `the_guard_and_the_gate_exclude_each_other`.

  `NativeHost::new` (`sspi.rs:30`), the only constructor gwz-cli and gwz-py call, is covered only by `real_sspi.rs`, which tests authentication, not the gate.
- **Reproduction (run).** On a scratch copy I replaced `with_launch_guard(worker, …, guard)` in `new` with `{ let _ = guard; Supervisor::new(worker, …) }`. The transport-shape tests filtered on `spawn_gate` and `native` all passed: 280 passed, 0 failed. By inspection, no Windows row exercises `new` either.
- **Impact.** A refactor that drops the guard in production would pass every gate.
- **Correction and regression test.** Run the Windows `gated()` phase, or a variant of it, through `NativeHost::new` (the `leaked == 0` probe detector needs no guard wrapper). Or make `new` delegate to a crate-private function that the chain test calls with the production builder.

### P3-3: gwz-py's non-candidate build still builds an unused, unguarded `Supervisor`

- **Location.** `client_host.rs:44-52`, the `else` branch. Its own doc comment says "nothing uses the supervisor".
- **Is it dead or required?** Dead. It is never read: the ordinary shape has no `NativeCaller`, and gwz-core's ordinary build doesn't even depend on gwz-sspi. It's harmless for inheritance (no worker is ever launched, so there is no window), but on Windows it costs two gwz-sspi driver threads and a token probe per `Client`. This predates the change (`e0c5af1`), but these lines were rewritten here.
- **Correction.** Either make the branch `type NativeRoute = ();`, or write down why it is kept. That is the lane owner's call under "remove dead code now" against release-scoped decisions; the branch disappears at S7.1 either way.
- **Regression test.** The ordinary-shape build stays warning-free.

### P3-4: D31's literal mechanism ("an additive `Options` hook") was replaced, and the decision row doesn't say so

- **Is the replacement justified?** Yes, by demonstrated necessity:
  - `Options` is `Copy`, not `#[non_exhaustive]`, and has a public field;
  - struct literals (`Options { max_workers }`, in `tests/supervisor_values.rs`) would break with any new field;
  - an `Arc` callback would remove `Copy`.

  C9, OQ-H and the CHANGELOG explain it.
- **Gap.** The operator's ledger row for D31 still reads "Options hook".
- **Correction.** Annotate D31 "as built: `Supervisor::with_launch_guard`, because…" for the operator to acknowledge.

## Attacks that failed

- **Semver.** The only public additions are `LaunchGuard` (Clone, Debug, Send, Sync; `new<F: Fn() -> G + Send + Sync, G: Send>`; `hold`) and `with_launch_guard`. `Platform` is private. The auto traits of existing types are unchanged (probe run). Additive API in a z bump is correct for 0.y.z. `Supervisor::new` is exactly 0.1.1's: `from_platform(…, None)`, and `guard.map` with `None` is a no-op.
- **Guard lifetime.** It is dropped after `drop(window)` and before `drop(attributes)`. A failed `InheritWindow::open` or a failed `CreateProcessW` drops it with the stack frame. A panic in the callback is caught by `spawn_launch`'s `catch_unwind` before any window opens.
- **Deadlock.** No gate holder waits on gwz-sspi. The runner's hold covers a synchronous spawn only, `helper_credential` doesn't touch SSPI, and the launch thread never holds the gate itself.
- **Gate identity.** In `with_https_native`, the caller's gate flows into `HelperSlots::with_gate`, then `state.spawn_gate`, then `RequestContext`, then the backend and derived backends. The only other production `HelperSlots::new()` is the native-less entry, which has no supervisor. The chain mutations go red.
- **Callers of the removed `NativeCaller::capture`.** None remain. `transport_host` is candidate-only and has never been released. gwz-core's `.github/gwz-py.commit` pins source that is only read, not built.
- **gwz-py per-`Client` gate.** It is within D31 by necessity: the guard is fixed when the per-`Client` supervisor is built, so the gate must be per `Client` too.
- **Pins and release scripts.** Updated consistently: `=0.1.2` in both `Cargo.toml`s and both locks, accepted shapes and tests, and the `test_worker_packaging` glob. No stale `0.1.1` remains in tracked files.
- **`release_checks.py`.** Unchanged and green at 0.1.2. Pre-bumping the version follows the 0.1.1 precedent (`fa4ac93`).
- **Repository rules.**
  - Every new file is under 1000 LOC (largest: 974).
  - No new globals.
  - Tests stay test-only.
  - Braced control flow holds.
  - New `cfg` is inside `cfg_if` or windows-only modules. The one bare `#[cfg(not(windows))]` sits on a braced test `fn` (`tests/supervisor_values.rs:77`), which `check_cfg_boundaries.py`'s reading of the rule allows.

## Follow-ups (not blocking)

- `Supervision.md`'s `Supervisor` row doesn't list `with_launch_guard`. It ships in the crate, so fix it before the tag.
- Consider `#[must_use]` on `LaunchGuard::hold`.
- When gwz-core takes a registry pin for gwz-sspi (Phase 8 or S7.1), it must require at least 0.1.2.
- Record the per-`Client` coupling cost in C9. A credential prompt in one operation now blocks other operations' backend spawns (an uncancellable `hold()`) and their worker launches, as well as their helper launches (which still time out with `GateTimeout`).
- Add a stale `=0.1.1` exact pin to gwz-py's refused-shape test.
- An infallible, unbounded `LaunchGuard` is frozen at publish. A deadline-aware guard would need a new constructor later.

## Next action

The lane owner fixes P2-1 in gwz-sspi (a second `verify` after the guard, the docs, the regression row) before any tag, and records and runs P2-2's push order with the gwz-core amend. The P3s can be fixed or recorded in the same remediation round. Then run a round-2 re-review limited to the two P2s.
