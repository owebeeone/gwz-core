# D31 launch-guard review, State axis: gwz-sspi 0.1.2 and the NativeHost wiring

- **Object:** lane `/Volumes/projects/limbo/gwz-dev-sspi012`
  - gwz-sspi `bd684977` over `75c1b032`
  - gwz-core `02d73b40` over `4f4b7e01`
  - gwz-cli `8340ac9f` over `279bf85f`
  - gwz-py `7185f22e` over `8e8446ba`
- **Date:** 2026-10-18
- **Axis:** State (concurrency, inheritance windows, guard and lock lifetimes, deadlock, cancellation, fail-closed direction, test validity)
- **Verdict:** **NO-GO.** P0: 0, P1: 0, **P2: 2**, P3: 6.

The core mechanism is correct. Both P2s are about the boundary of the claim and the binding of the evidence, not the guard's ordering. Neither needs a gwz-sspi code change.

## §0 Evidence base

- **Tuple:** checked at start and at end. Heads and parents match the object, and no tracked files are dirty in any of the four repos. The only untracked files are the pre-existing `gwz-core/dev-docs/GwzRemoteTransportBugReport.md` and `gwz-core-evidence/.../2026-09-22-alpha-setup-timeout/`.
- **Code read:**
  - gwz-sspi: `windows/launch.rs` (`start`, `InheritWindow`), `monitor.rs`, `dispatch.rs`, `driver.rs`, `context.rs`, `futures.rs` (`Start`, `Shutdown`), `kernel.rs`, `api.rs`, `values.rs` (`LaunchGuard`).
  - gwz-core: `git/spawn_gate.rs`, `https_worker/native/sspi.rs` (`NativeHost`), `transport_host/{mod,request,local_command,cancellable}.rs`, `gitbackend/{backend,transport_support,repository,refs}.rs`, `https_auth/runner.rs`, `transport_scope.rs`.
  - gwz-cli: `globalargs/dispatch.rs`.
  - gwz-py: `native/src/{client_host,shims}.rs`.
- **Tests read:**
  - gwz-sspi: `launch_guard_tests.rs`, `windows/inheritance_tests.rs`, `tests/native/windows_worker.rs`, `tests/supervisor_values.rs`.
  - gwz-core: `process_tree/windows/tests.rs` (the probe), `tests/sspi_window.rs`, `spawn_gate_chain_tests.rs`.
- **Evidence:** `2026-10-17-sspi012/`.
  - `manifest.json` re-hashed: 92 files, 0 missing, 0 mismatched, 0 unlisted.
  - The staging-digest recipe was reverse-engineered and reproduced: `sha256(git diff HEAD ; for each untracked file outside dev-docs: "<sha256>  <path>")`.
- **Experiment:** I ran one, on a scratch copy of gwz-sspi `bd684977` (not the lane), with `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewd31-state-sspi`, now deleted. It was a fake-platform supervisor whose launch guard blocks past the start deadline (results under Attack 3). The test build's frozen clock (`supervisor/clock.rs`, `#[cfg(test)]`) had to be advanced by hand; that is an artifact of the experiment, not a finding.
- **Not done, per the rules:** `run_r4bg_aggregate_gates.py`, dabeest, any modification.

## Findings

### P2-1: A gwz-py Client's non-network operations are outside the Client's gate, and C9 says the opposite

- **Root cause:** gwz-py's ordinary calls run on `shims::Backend::own()`, which is `Git2Backend::new()` (`gwz-py/native/src/shims.rs:58`, used at `client_host.rs:109`). That backend has `spawn_gate: Default::default()` (`gwz-core/src/git/gitbackend/backend.rs:62`), a fresh gate per call that nothing else holds. These calls include `commit`, and `tag` when it reaches no remote (`transport_scope::in_scope`). Only network operations get the Client's `NativeHost` gate, through `with_host_context`.
- **Violated invariant:**
  - The D31 requirement: no concurrent std spawn without a handle list can inherit the worker's child ends.
  - The new C9 sentence (`GwzTransportWindowsHelperOwnerContract-DRAFT.md`, C9 row) is false for these operations: "A gwz-py `Client` shares its supervisor, and now its gate, across its operations, so two operations of one `Client` do exclude each other."
- **Interleaving:**
  1. Python thread T1 calls `client.fetch()`. With the GIL released (`py.detach`), its HTTPS native route launches an SSPI worker. gwz-sspi's launch thread takes the Client gate and opens `InheritWindow`.
  2. Python thread T2 calls `client.commit()` on the same Client, for a member with a hook.
  3. The `native_commit` route rule picks `commit_cli`, which calls `backend.spawn_gate.output(git commit)` on its own fresh gate. It does not wait.
  4. std's `CreateProcessW` (bInheritHandles=TRUE, no handle list) runs inside T1's window.
- **Impact:** `git commit` and its hooks inherit the worker's stdin read end and stdout write end. The C9 analysis applies to gwz-sspi's pipes:
  - The supervisor's reader gets no end-of-file until the stray process tree exits, so cleanup stalls and the record stays charged.
  - The stray can read the supervisor-to-worker stream (the request and the challenges).
  - It can write frames into the supervisor's reader.
  - There is no credential read-out, because the stray holds a write end of the token pipe, not a read end. Hence P2, not P0.
  - The same exposure already existed for WH2 windows, but the contract now states it as closed.
- **Correction:**
  - Within the authorized scope: correct the C9 scope sentence and the OQ-H "open after the gate" list. They should say that a Client's network operations exclude each other, and that its non-network operations (CLI-routed `git commit`, local `git tag` / `git tag -d`) and other Clients' spawns do not.
  - Structural fix, an operator decision because it is a new gwz-core public seam (`SpawnGate` is crate-private): let `NativeHost` hand out a `Git2Backend` that carries its gate, for gwz-py's `Backend::own`.
- **Regression test:** a gwz-py native unit row. Hold the Client host's gate through a test accessor, then assert that `Backend::own().get().spawn_gate.try_hold()` is `None`. It is red today, and green once the seam lands. For the doc-only correction, use a source check that pins the C9 scope sentence.

### P2-2: Receipt `sspi012-20261018-j` is not bound to gwz-core `02d73b40`

- **Root cause:** the digest recipe mixes code and documents, and gwz-core's dev-docs (C9 and the plan) were edited after staging. The C9 text cites receipt j by name, so it must have been written after j. The commit is at 08:49, while j was staged at 06:13 and the README is from 06:40.
- **Location:** `raw/digests/staging-j.digests` (gwz-core `a9545145…`) and the README claim "Staging digests … were unchanged after the run".
- **Violated invariant:** the evidence that counted must be on the final code.
- **What I verified:**
  - Recomputed from the commits with the reproduced recipe, gwz-sspi `f0dddb1e…`, gwz-cli `7b98bdab…` and gwz-py `74514df4…` match staging-j exactly.
  - gwz-core gives `b3b6118a…`.
  - Variants that exclude dev-docs, `Cargo.lock`, either edited doc, or that add the untracked bug report do not reproduce `a9545145…` either.
  - Supporting but not conclusive: panic lines 309 and 420 in `mutation-final-and-restored.log` (06:28–06:38) match the committed `sspi_window.rs`, and the restored run's output strings match the committed format strings.
- **Interleaving:** any edit to gwz-core code between 06:40 and 08:49 would go unnoticed. The only Windows evidence for the gwz-core row and the host wiring would then not describe the reviewed object.
- **Impact:** the D31 Windows claim for gwz-core is unproven at `02d73b40`. This is an evidence-validity defect, not a code defect.
- **Correction:** re-run on a fresh host directory staged from `02d73b40`. The `sspi_window` row plus the `spawn_gate_chain` rows are the minimum; a full `--tests --hosted-like` lane check is better. From now on, record a code-only digest (the diff with dev-docs excluded, plus per-file hashes of untracked files) alongside the full digest.
- **Regression test:** the new receipt's code-only digest equals the one recomputed from `git diff 4f4b7e01 02d73b40 -- . ':!dev-docs'` plus added files.

### P3-1: The originating-thread recheck is no longer "immediately before creation"

- **Location:** `gwz-sspi/src/supervisor/windows/launch.rs:181–186`.
- **What happens:** `origin.verify()` now runs before `LaunchGuard::hold`, which can block without bound (Attack 3). The comment "rechecks … immediately before creation. These synchronous metadata calls never poll" is no longer true of the whole step.
- **Requirement provenance:** D31 fixes this order, so this is raised, not treated as a defect.
- **Correction:** keep D31's order, and add a second `origin.verify()` after `hold()` returns and before `InheritWindow::open`. It is cheap and synchronous.
- **Test:** an `Origin` that flips to refusing while the guard blocks. Assert that no window opens and that the guard is released.

### P3-2: A launch released after its record ended still runs `CreateProcessW` under the gate

- **Location:** `monitor.rs:62–70` checks `record.stop` only before `platform.launch`. `launch.rs:188` does not recheck it after `hold()`.
- **Effect:** a record that timed out, was cancelled or was closed while blocked still creates a suspended worker, which is then killed through the job. The experiment trace shows `launch-return` after release, with no `resume`.
- **Impact:** wasted work and gate time; it is contained.
- **Correction:** recheck a stop predicate after `hold()`, for example by passing `&record.stop` into `create` and `start`.
- **Test:** a Windows row with a guard blocked past the deadline. Assert the Job's `ActiveProcesses` stays 0 after release.

### P3-3: A launch blocked on the gate is reported as a generic SSPI Timeout, and C9 describes it wrongly

- **Effect:** WH2 reports `GateTimeout` with busy-host wording. A gwz-sspi start blocked on a whole-call credential-helper hold reports `Timeout` (or `CapacityUnavailable` once all 8 records are charged).
- **Wrong wording:** C9 says the deadline "is acted on when the launch returns". In fact the experiment shows the caller gets `Timeout` at its deadline (302 ms against a 300 ms deadline); what persists is the charged record, until the hold ends.
- **Correction:** fix the C9 and `LaunchGuard` doc wording. Optionally, word an SSPI `Timeout` that happened while the gate was held the way WH2 does.
- **Test:** the experiment, landed as a gwz-sspi orchestration row.

### P3-4: The window does not fail closed if resetting inheritance fails

- **Location:** `launch.rs:131–135`. `InheritWindow::drop` ignores a failed `SetHandleInformation(…, 0)`; then `drop(held)` releases the gate while an end may still be inheritable, until `drop(ends)` in `create_in_job`.
- **Size:** microseconds, and only if `SetHandleInformation` fails on a valid handle.
- **Correction:** keep the guard until the ends are closed, by moving the hold into `create_in_job` to cover `start` plus `drop(ends)`. Or, on a reset failure, close the ends before releasing.

### P3-5: The gated phase of `sspi_window.rs` does not assert that its windows opened

- **Location:** `sspi_window.rs` `gated()` and `the_row_in_its_own_process`. It prints `windows.len()` but does not assert `== GATED_LAUNCHES`. `probes >= SPAWNERS` does not show that spawns ran during the launches.
- **Mitigation:** the control asserts a leak in the same process, and the logs show 100 windows and about 305 probes. So this is a vacuity guard, not a false pass.
- **Correction:** assert the window count, and assert that some spawn spans fall between the first and last window.

### P3-6: Sharing the gate per Client couples a Client's independent operations

- **Effect:** before D31, each gwz-py operation had its own gate. Now one operation's whole-call `helper_credential` hold (waiting for the user) blocks other operations of the same Client:
  - their `SpawnGate::output` / `hold()` calls, which are blocking with no clock or cancellation;
  - their SSPI launches;
  - their WH2 lookups, until `GateTimeout`.
- **Bounded by:** gwz-py's cleanup bound on cancel and close.
- **Status:** reachability in the transport build is a hypothesis; the credentials callback is installed on every libgit2 remote op. This is for the operator alongside the existing D25 helper-hold item.

## Attacks that failed

1. **Ordering.**
   - `held = guard.map(hold)` comes after `origin.verify()?` and before `InheritWindow::open(ends)?`. The window is dropped before `held`, explicitly and also in reverse declaration order on unwind.
   - Paths covered:
     - an `open` that fails part-way resets the opened ends inside `open`, then `held` drops;
     - a `CreateProcessW` failure takes the same drops;
     - a panic in or after the callback is caught by `spawn_launch`'s `catch_unwind`, and the locals unwind in order;
     - a refusal before the window takes no guard.
   - `InheritWindow::open` is the only way an end becomes inheritable (`inherited.rs` is the worker side, which only clears the flag).
   - The Windows rows observe OS state from inside the guard (inherit flags and Job `ActiveProcesses`): taken with flags off and 0 processes, released with flags off and 1 process (0 on the failure path). The four ordering mutations are red on dabeest.
2. **Deadlock.**
   - No path holds a `SpawnHold` and then waits for a gwz-sspi start. Holders are:
     - `Runner` (polls `try_hold`, held for the spawn only);
     - `SpawnGate::spawn` / `output` (held for the spawn only);
     - `helper_credential` (waits on an external `git credential` process only);
     - the launch thread (`CreateProcessW` only).
   - The launch thread holds no gwz-sspi state lock at `platform.launch`, and the `SpawnGate` mutex is a leaf.
   - The callback is not re-entrant into gwz-sspi.
   - The HTTPS endpoint's tokio workers are not needed by the gate holder.
   - The wakeups are correct: the flag is cleared under the lock, then `notify_one`, and waiters re-test in a `while` loop.
3. **Cancellation and deadlines (experiment).**
   - With the guard blocked: `start` returned `Timeout` at 302–306 ms against a 300 ms deadline; a queued start returned `Timeout` at its deadline; `shutdown` returned at its deadline with 1 outstanding.
   - After release: the record was reaped, nothing was resumed, and nothing was outstanding.
   - `Supervisor::drop` only closes the context and joins nothing, so a blocked launch cannot stall drop or shutdown.
4. **Gate identity.**
   - In gwz-cli there is one invocation per process (`lib.rs:304`), so one `NativeHost` and one gate per process.
   - `NativeHost` makes the gate and the guard together, and `with_https_native` builds `HelperSlots::with_gate(caller.spawn_gate())`, which reaches the request context and every backend (chain rows).
   - The gwz-py non-candidate `Supervisor::new` never launches, because there is no caller there.
   - **Two-Client residual, stated precisely:** Client A's worker window is not serialized against Client B's std spawns that name no handle list. Those are: B's backend `git commit`, `git tag` and `git tag -d`; B's `helper_credential` spawn; any non-network operation of either Client (P2-1); `gwz log`'s `rev-list`; and the embedding process's own spawns. Launches that name a handle list (WH2, gwz-sspi) cannot take each other's ends.
5. **Test validity.**
   - The probe counts only inheritable pipe handles with default 4096-byte buffers. The detector is calibrated in-process (a CreatePipe pair counts as +2).
   - The control (`LaunchGuard::new(|| ())`, ungated spawners) is red within 1–2 launches.
   - The overlap spans are taken inside the holds, so they test gate identity.
   - `quick-6` shows the pipe oracle red in the gated phase under a no-op-guard mutant (a probe held 2 worker pipes).
   - The mutated-`sspi.rs` incident was confined to host directory `sspi012-20261017-b` (quick-6 to quick-13). Receipt j used its own directory `sspi012-20261018-j`. `mutation-final` re-uploaded the lane file for both its mutant run (red, through the thread-only check at line 309) and its restored runs (green, 100 windows, 0 overlaps, 0 leaks).
   - `lane-check-a` was killed after uploading gwz-sspi, before any build, and does not count.
6. **Repository rules.**
   - No globals or thread-locals were added; the guard captures an `Arc` gate clone.
   - New gwz-core and gwz-py conditionals use `cfg_if!`.
   - The one new `#[cfg(not(windows))]` sits on a braced `#[test] fn` in `gwz-sspi/tests/supervisor_values.rs`. That is a braced item, consistent with gwz-sspi's existing pattern, and gwz-sspi has no `cfg_if` dependency, so it is not a violation.
   - Rust enforces braced control flow.

## Follow-ups (not prerequisites)

- Operator: the gwz-py structural fix for P2-1 (a `NativeHost`-provided backend gate), and the P3-6 coupling, together with the existing D25 helper-hold item.
- gwz-sspi 0.1.3 candidates: P3-1, P3-2, P3-4, and landing the blocked-guard experiment as a row.
- Evidence recipe: record a code-only digest and per-file hashes of untracked files.

## Next action

1. Close P2-1 within scope by correcting C9 and the OQ-H list in gwz-core dev-docs, and file the structural fix for an operator decision.
2. Close P2-2 by re-receipting the committed tuple on a fresh host directory staged from `02d73b40` (at least the `sspi_window` and `spawn_gate_chain` rows), with code-only digests recorded.
3. Then re-review those two items only.
