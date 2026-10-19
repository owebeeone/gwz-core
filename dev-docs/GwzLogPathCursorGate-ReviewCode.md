# gwz log path-cursor spawn gate (gwz-core 650dd4e0, gwz-py 02d79ca, gwz-cli cea7bd4) — CODE-AXIS REVIEW

**Review object:** the three code commits, gwz-core `650dd4e0ba3f707d47973030c2862df4098ff867`, gwz-py `02d79cad937f2d7752362861022cd07e4d1c09a5` and gwz-cli `cea7bd4cabae8b1a840c810a04794e62acb45600`, plus evidence run `campaigns/transport-qualification/runs/2026-10-19-loggate/` at gwz-core-evidence `7f44d19d84bfd44b688ddf6918be56295843590d`.
**Baseline:** gwz-core `9e8edb80e730e86e44894d79d18cef551f70e6e5`, gwz-py `5b6b66579867d96d21d5d5d003aaa69f50960b4e`, gwz-cli `5193db962e672c54cd20ab2ef8448d904129a559`.
**Date:** 2026-10-19.
**Axis:** Code. The review is independent, adversarial and read-only. The other axis runs in parallel. This report is filed verbatim.

**Verdict: GO** — P0 0, P1 0, P2 0, P3 2.

## 0. Evidence base

**Tuple at start.** All four HEADs and all three code parents match the full shas above. The evidence parent is `c442cf8d`. All four working trees are clean.

**Source read (gwz-core):**
- `git show HEAD`, in full, for `commit_log/{mod,request,handler,path_characterization,coalesce_tests,tests}.rs`, the new `tests/path_cursor_gate.rs`, `push_event.rs` and `scripts/checks/cfg_boundaries_allowlist.json`.
- `src/git/spawn_gate.rs`, `gitbackend/backend.rs`, `gitbackend/refs.rs`, `gitbackend/repository.rs` and `endpoint/https_worker/native/sspi.rs` (`NativeHost::{new,backend,gates}`).
- `endpoint/https_auth/runner.rs` (`hold_spawn_gate`) and `gitbackend/transport_support.rs` (`ensure_server_timeout`, `configure_server_timeout_ms`).
- `diff/classify.rs` and `diff/tagged.rs`.

**Source read (gwz-py and gwz-cli):**
- gwz-py: `native/src/{shims.rs, client_host.rs, route/native.rs, dispatch/mod.rs, dispatch/log.rs, dispatch/read.rs}` and `src/gwz/client.py`.
- gwz-cli: `src/{log_exec.rs, lib.rs, globalargs/dispatch.rs}`.

**Searches:**
- Every `Command::new`, `.output()`, `.spawn()`, `Git2Backend::new` and `SpawnGate::{new,default,with_exclusion}` under `commit_log/`.
- The same spawn search under everything `request.rs` imports: `crate::diff`, `crate::workspace_ops` and `crate::artifact`.
- Every caller of `handle_log`, `open_request_histories` and `open_default_head_histories`, across gwz-core, gwz-cli, gwz-py, gwz-git, gwz-transport, taut, taut-shape-rs, trial and scratch.

**Evidence checks:**
- `python3.13 -B tools/verify_archive.py` prints "Verified 5553 migration records; no build/cache directories in archive".
- All 25 sha256 entries in the run's `manifest.json` match the files.
- The `+`/`-` lines of `patches/gwz-{core,py,cli}.diff` equal `git diff HEAD^ HEAD` in each repo. The only difference is the gwz-core new test file, which is archived separately and is byte-identical.
- I read `raw/core/{red,mutation,green-focused}.log`, `raw/py/{red,green-native,run-tests-candidate}.log` and `raw/windows/{receipt.json,windows-lane-check.log}`.
- I diffed `patches/py-dispatch-log.rs.red-state` against HEAD.

**Independent run.** `GWZ_TEST_GIT=real CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/loggate-rev-code cargo +1.95.0 test --lib -- commit_log`, run in the lane's gwz-core, gave 110 passed and 0 failed. `a_pathspec_log_spawns_its_path_cursor_under_the_backends_gate` was ok. I then deleted the target dir.

## Problem, requirement provenance, and scope

**Problem.** The problem is confirmed at baseline. `next_unfiltered`'s `MessagesState::PathWalk` arm called `command…stderr(Stdio::null()).output()`, with no gate. In the gwz-py candidate build, a Client's `log` thread could therefore create a child during its own HTTPS helper's or SSPI worker's creation window.

**Closure.** The package closes it:
- The arm is now `gate.spawn(&mut command).and_then(|child| child.wait_with_output())`, with stdin and stdout piped and stderr null (`commit_log/mod.rs`).
- The gate's path is `handle_log(backend)`, then `open_request_histories(.., &backend.spawn_gate)`, then `open_history(.., gate)`, then `RepositoryState::Ready { gate }`, then `MessagesState::PathWalk { gate }`.
- In gwz-py, `log::call` receives the dispatch's `shims::Backend`. `run_log` calls `shims::backend`, which calls `Backend::get`, which calls `own_backend(route)`. In the candidate build that is `NativeHost::backend()`, which sets `spawn_gate = self.gate.clone()`. That is the gate that `launch_guard` gives the supervisor and that `NativeHost::caller()` gives the transport's helper slots.
- This is the only production spawn on the log path. The fresh `Git2Backend::new()` calls are covered in §2.

**Provenance.** Every element traces to the 2026-10-19 bounded proposal, items 1–7, which the operator's closeout authorization covers. No dependency or infrastructure change was made. Nothing out of scope was touched: no D52 source check, no required-constructor change, no gwz-cli spawns other than the log call, and no Python `subprocess` call.

**Implementer's deviations:**
- *Gate on `RepositoryState::Ready`, not on `RepositoryHistory`.* Sound. Only a walkable history can reach `from_repository`, and a `Degraded` history never spawns. No `RepositoryHistory` without a gate can walk.
- *Probe per prefix, counting the child's lifetime.* Test-only, inside the `cfg_if!` `cfg(test)` arm. Counting from spawn to the end of the wait measures the actual child lifetime. The F4 jobs row (`max_path_readers == 1`, with `jobs = 1`) still measures overlapping children, which is the property it claims.
- *gwz-py `run_log` seam.* Production-sound. It is a generic function with a three-line body, its production caller passes the real `handle_log`, and it contains no test code.
- *Two allowlist entries removed.* Correct:
  - `let _reader_probe` and `use std::sync::{Mutex, OnceLock}` no longer exist as bare attributes.
  - The removed `#[cfg(test)]` went out together with its `use`. Nothing re-associated onto the next line: `use crate::artifact::ArtifactSourceKind;` follows a blank line and was never under it.

**Behaviour beyond the gate.**
- *Stdin.* Before, it was `Stdio::Null`. std's `output()` uses null stdin when stdin is unset, because `needs_stdin = false`. Now it is a pipe, which `Child::wait_with_output` drops before it reads. Stdin was never written. `rev-list` without `--stdin` never reads it, and EOF on an empty pipe is the same as null for this child.
- *Deadlock.* None: stdin is closed first, and only stdout is read because stderr is null.
- *Error mapping.* Unchanged: a spawn error or a wait error still becomes "could not run local path history cursor: {error}", which is `HistoryUnreadable`.
- *Gate hold.* `SpawnGate::spawn` drops the hold right after `command.spawn()` returns. The wait runs outside the hold, as at the `refs.rs` and `repository.rs` sites.
- *Other behaviour.* No other behavioural change was found that bears on correctness. See §2 for the timeout-freeze ordering.

## 1. Findings

### P3-1 — the gwz-py row proves `run_log`, not "a log call", runs on its Client's gate

- **Root cause.** `a_log_call_runs_on_its_clients_gate` calls `run_log` directly with a `Backend::own(host)` it builds itself. The production change it is meant to guard is not exercised: `dispatch::call` passing `backend` to `log::call`, and `call_log` forwarding it.
- **Location.** gwz-py `native/src/dispatch/log.rs`, `mod tests`, about lines 78–107. `dispatch/mod.rs:125`.
- **Invariant.** The backend that handles a `log` request through `dispatch::call` must be the dispatching Client's `NativeHost` route.
- **Reproduction.** Make one of these mutants:
  - In `call_log`, ignore the `backend` parameter and pass `&shims::Backend::own(gwz_core::transport_host::NativeHost::new(Err(..)))` to `run_log`.
  - In `dispatch/mod.rs`, hand `log::call` a fresh `Backend::own`.

  The row stays green. The type signature makes such a mutant unlikely, but nothing tests against it.
- **Impact.** It is a coverage gap only. The production wiring is correct today: I checked `client_host.rs:119` (`Backend::own(scope.own)`), `dispatch/mod.rs:125` and `log.rs`. The row's name and doc comment claim more than it checks.
- **Correction.** Do one of the following:
  - Re-scope the row's name and doc to `run_log`.
  - Drive the row through `log::call` (or `dispatch::call` with a Client's `NativeRoute`) using a pathspec request and an observation seam.
- **Regression test.** A `dispatch::log::call` row with a `Backend::own(host)` that asserts `host.gates(..)` on the backend `handle_log` receives. One option: `call_log` takes the handler through the same seam that `call` uses, so the row can observe it.

### P3-2 — the archived gwz-py candidate gate run predates the committed gwz-py tree

- **Root cause.** The README reports two things about the final tree: "before my inventory line the switch-site test failed with three NEW sites, now two", and `run_tests.py --candidate` gave 1060 passed and 2 failed. The archived log is the run *before* the inventory line. Its lines 739–742 list three NEW sites, including `native/src/dispatch/log.rs mod tests`, which is the line the commit adds. No candidate pytest log exists for the committed gwz-py `02d79ca`.
- **Location.** gwz-core-evidence `…/2026-10-19-loggate/raw/py/run-tests-candidate.log` and `README.md`, in the Gates table and Open findings.
- **Invariant.** Each gate result reported for the committed tree is backed by an archived log of that tree.
- **Reproduction.** Run `grep -n 'NEW ' raw/py/run-tests-candidate.log`. It returns three lines, and one of them is the committed inventory line.
- **Impact.** The claim is diagnosable but not evidenced. I checked by hand that the two remaining NEW sites predate the change: `client_host.rs type NativeRoute` and `shims.rs mod tests` both exist at `5b6b665` and are absent from its inventory. The claim is therefore plausible, and it does not affect the Rust gates.
- **Correction.** Archive a `run_tests.py --candidate` log of the committed gwz-py tree, or correct the README to say that the archived run predates the inventory line.
- **Regression test.** None. This is an evidence-hygiene item.

**Review areas checked with no defect found:**
- *Completeness.* There is one production spawn under `commit_log/`. Everything that `request.rs`, `handler.rs`, `merge.rs` and `aggregate.rs` call runs no `Command`; all other hits are test-only. The per-member threads in `merge.rs` each take their history's `SpawnGate` clone (`Arc`, `Send + Sync`).
- *Interface.*
  - `handle_log` changed from generic to a concrete `&Git2Backend`. This is required, because the gate is a `pub(crate)` field and `GitBackend` exposes no gate accessor.
  - The only callers are gwz-cli `log_exec.rs`, gwz-py `dispatch/log.rs` and the core tests. No other caller exists in the workspace.
  - The break of gwz-core's public API is consistent with the 1.1.0 line, which already removes many `push_event` public items since `v1.0.17`.
  - `open_default_head_histories` is `pub` inside the private `commit_log` module and only tests call it, so a `pub(crate)` `SpawnGate` parameter raises no interface lint.
- *gwz-cli.* The log branch at `lib.rs:268` runs before `execute_invocation_selected`, and that is the only place a `NativeHost` is constructed (`globalargs/dispatch.rs:61`). `set_server_timeout_ms` has already run by then. A private gate is correct there.
- *Code rules.* All new conditional code is in `cfg_if!` blocks. The remaining bare `#[cfg(test)]` attributes in `commit_log/mod.rs` and `handler.rs` were not changed and are allowlisted. The probe `static` exists only in the `cfg(test)` arm; the non-test arm is a zero-sized stub. No thread-locals or globals were added to production. No production code is under `tests/`.
- *Core row.*
  - It proves that the gate is the backend's own: a fresh `SpawnGate::new()` is non-excluding on macOS and a different gate on Windows, so either one finishes and goes red.
  - It proves blocking, not only delay. The log must not finish while the hold exists, and it must go on once the hold is dropped.
  - The red state and the `Command::output()` mutant both fail at line 43.
  - The probe firing after the "not finished" assertion does not weaken this. The first assertion alone refutes every ungated implementation. The probe adds the stricter "no child ran" claim, and it catches an ungated spawn on a host slow enough to exceed 500 ms.
  - A pure fixed delay of 500 ms or more before the spawn would also pass. No plausible mutant has that shape, and I judge that acceptable.

## 2. Out-of-scope observations

- **Fresh `Git2Backend::new()` on the gwz-py log route (limitation, not exposure).** `open_request_histories` calls three diff helpers that each build a backend on a private gate:
  - `candidate_repos` (`diff/classify.rs:221`)
  - `default_rev_resolver` (`classify.rs:204`)
  - `missing_exact_local_tags` (`diff/tagged.rs:63`)

  They spawn nothing: `is_repository`, `read_ref` and `tag_list` are libgit2-only (`gitbackend/repository.rs:7`, `refs.rs:230`, `refs.rs:286`). So no spawn is ungated today. If any of those backend methods later gains a CLI route, it would spawn outside the Client's gate. Suggested improvement, outside this proposal: thread the request's backend into these helpers, which `diff` also shares. It does not block this deliverable.
- **Timeout-freeze ordering (negligible).** `shims::backend` now builds the Client's backend before the handler runs. `Git2Backend::new()` calls `ensure_server_timeout`, which freezes the process-wide timeout. Before, a gwz-py `log` froze it only once `candidate_repos` or `missing_exact_local_tags` ran, which happens on almost every log request. Only a log that fails before those calls changes. `configure_transport_timeout` is documented as a startup call (`test_timeout_configuration_is_a_typed_startup_call`), so I see no practical impact.
- **Gate contention on Windows (hypothesis).**
  - *Observation.* `DEFAULT_JOBS` is 100, and a pathspec log spawns one `rev-list` per commit per member. In a candidate-build Windows Client, those spawns now compete with the HTTPS helper's launch, which polls `try_hold` every 1 ms (`https_auth/runner.rs:192`). They also compete with the SSPI launch guard's blocking hold.
  - *Assessment.* Every release opens a window in which `held == false` and the poller can win. As I understand std, it already serializes `CreateProcessW` on Windows. So I expect added latency, not starvation leading to `GateTimeout`, but I have not measured it.
  - *Suggested probe.* On dabeest, run a large many-member pathspec log against a concurrent HTTPS fetch that needs a credential helper on the same Client, and record the helper's gate wait. Any fix (gate fairness) belongs to `spawn_gate.rs` and is outside this package.

## 3. Tuple check at end

| Repo | HEAD | Parent | Tree |
|---|---|---|---|
| gwz-core | `650dd4e0ba3f707d47973030c2862df4098ff867` | `9e8edb80e730e86e44894d79d18cef551f70e6e5` | clean |
| gwz-py | `02d79cad937f2d7752362861022cd07e4d1c09a5` | `5b6b66579867d96d21d5d5d003aaa69f50960b4e` | clean |
| gwz-cli | `cea7bd4cabae8b1a840c810a04794e62acb45600` | `5193db962e672c54cd20ab2ef8448d904129a559` | clean |
| gwz-core-evidence | `7f44d19d84bfd44b688ddf6918be56295843590d` | `c442cf8d3e5bce5312ea232e4f89a455213744f8` | clean |

All match the shas at start. `/Volumes/projects/limbo/build-targets/loggate-rev-code` has been deleted.
