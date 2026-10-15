# State review: gwz-sspi 0.1.1, worker pipes and NUL handle made non-inheritable (D24, D19 State P3-1)

- **Object:** gwz-sspi `fa4ac930cfa78c42bd2524bee27dedcdc9fe3753`, one commit over `d303888`, in the lane `/Volumes/projects/limbo/gwz-dev-sspi011/gwz-sspi`. The secondary object is the lane's uncommitted pin changes in gwz-core, gwz-cli, gwz-py and the root, plus the WH2 contract C9 line.
- **Date:** 2026-10-16
- **Axis:** State (state machines, races, handle lifetimes, fail-closed direction, error paths, secret exposure)
- **Verdict: GO.** P0: 0, P1: 0, P2: 0, P3: 3. The code fix is correct and complete within D24's scope. I recommend fixing P3-1 before tagging, because `docs/**` ships in the crate archive and cannot be changed after publish.

## §0 Evidence base

**Read in full:**
- The gwz-sspi diff `d303888..fa4ac93`: CHANGELOG, Cargo.toml and lock, Architecture.md, `launch.rs`, `inheritance_tests.rs`, `native_tests.rs`.
- The whole of `launch.rs`, `attributes.rs`, `handles.rs`, `mod.rs` and `inherited.rs` at `fa4ac93`.
- `values.rs` (`WorkerExecutable::new`), `identity.rs` (origin and verify), `api.rs` (`Supervisor::new` and `start`), `monitor.rs` (`launch`).
- `.github/workflows/{ci,release}.yml`, `scripts/release_checks.py`.
- The lane diffs in gwz-cli (`Cargo.toml`, `Cargo.lock`, `release.py`, `test_release.py`, `HostPackaging.md`), gwz-py (`Cargo.toml`, `Cargo.lock`, `RELEASE.md`, `release.py`, `test_release_pins.py`, `test_worker_packaging.py`), gwz-core (`Cargo.lock`, contract C9) and the root `Cargo.lock`.
- gwz-core's `.github/gwz-sspi.commit` and the gwz-sspi checkout steps in the gwz-cli and gwz-py workflows.
- D19 ReviewState P3-1 and P3-2, decisions D24 and D25.
- The run `2026-10-16-sspi011`: README, `tools/remote-all.sh`, `hostrun.sh`, `lane_check_local_sspi.py`, the red/green/mutation log, `lane-check-run.log`, `receipt.json`, the package list, the gearu rehearsal log, and the Mac gate status files.

**Commands and results:**
- `cargo +1.95.0 clippy --all-targets --all-features --locked --target x86_64-pc-windows-gnu -- -D warnings` with `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewsspi011-state`: clean.
- The same command for `x86_64-pc-windows-msvc`: clean.
- The rehearsal clone's commit `78a76b6^{tree}` is `31710d60…`, which equals `fa4ac93^{tree}`. The Mac gearu rehearsal, `cargo package` and the dry run therefore ran on exactly the object's tree.
- The Windows log, section B, shows the five rows green. Mutation 1 (the window's drop does not clear) fails the window row and the failure row. Mutation 2 (the window never opens) fails the success row with `WorkerUnavailable`: `CreateProcessW` refuses a non-inheritable handle in the list. The opt-in native fixtures pass: native_worker 8/8, lib ignored 7/7.
- Receipt `sspi011-20261016-a` passed. Hosted-like had 7 failures, all named host gaps that need gwz-py and taut-proto, which the host lacks.

**Tuple (identical at start and end; nothing was modified):**

| Repo | Commit | Uncommitted entries |
|---|---|---|
| root | `e52ad04` | 8 (3 modified or staged, 5 untracked) |
| gwz-sspi | `fa4ac93` | 0 (clean) |
| gwz-core | `af7e404` | 3 |
| gwz-cli | `8d909ec` | 5 |
| gwz-py | `da098f5` | 6 |
| gwz-core-evidence | `db1fe67` | 1 (the untracked alpha run) |

## Findings

### P3-1: gwz-sspi's own docs claim the residual race is gone

- **Root cause.** The new text states the property "never inherited by a concurrent spawn" when the code only gives "inheritable only during the window".
- **Location:**
  - `gwz-sspi/docs/Architecture.md:71-74`: "…inheritable only inside the `CreateProcessW` call … so a concurrent spawn that inherits all inheritable handles never receives one."
  - The doc comment on `launch.rs:109-112` for `InheritWindow`: "at no other time, so a concurrent spawn that inherits every inheritable handle (std `Command`, the git CLI) cannot take them."
  - `CHANGELOG.md` 0.1.1, first bullet: it describes the hazard as a "Before" condition and never says that it persists for the duration of `CreateProcessW`.
- **Violated invariant.** Documentation must not claim more than the code gives. P3-1's own correction said "Full closure needs P3-2's gate", and it included correcting the contract's earlier overclaim ("does not take part"). This text brings back the same kind of overclaim, inside the crate.
- **State sequence:**
  1. Thread A is in `start()` between `InheritWindow::open` (`launch.rs:180`) and `drop(window)` (`:201`). This spans the whole `CreateProcessW` call, which takes milliseconds and longer under AV scanning.
  2. Thread B calls std `Command::spawn`, which passes `bInheritHandles=TRUE` with no handle list. Examples are libgit2's `Cred::credential_helper` and gwz-core's `Command::new("git")` sites.
  3. B's child inherits the worker's stdin end, stdout end and NUL. With the stdin read end it can read the token frames the supervisor writes. With the stdout write end it can inject frames, and the parent sees no EOF until that child exits.
- **Impact.** The code is right. The contract C9 line is honest: it says gwz-sspi "is a victim of a concurrent spawn exactly as the helper is". But the published 0.1.1 archive (`docs/**` and `CHANGELOG.md` are in `include`) would state a credential-boundary property that does not hold. A host that embeds gwz-sspi, or a future reviewer, would conclude that no spawn serialization is needed. A published crate's text cannot be corrected without another release.
- **Correction.** Reword Architecture.md and the doc comment to: "inheritable only from just before to just after `CreateProcessW`; during that window a concurrent spawn that names no handle list (std `Command`) still inherits them; the host must serialize such spawns with worker launch (gwz: D25's spawn gate)." Add one CHANGELOG sentence giving the residual.
- **Regression test.** None executable. Have a review check that the crate docs agree with C9.

### P3-2: the Windows execution evidence is not bound to `fa4ac93`, and the red tree is not archived

- **Location:** `2026-10-16-sspi011/README.md`, `tools/remote-all.sh:5`, `raw/windows/gwz-sspi-dabeest-red-green-mutations-natives.log`, `raw/windows/lane-check-run.log`.
- **Root cause.** The dabeest runs used uploaded working trees: the lane check at 08:32, the red/green/mutation run at 09:49. The commit was made at 09:52. No tree hash or tarball digest is recorded.
- **The red tree (`_red`) was not archived, and the log shows it was not the committed test set:**
  - It reports `gwz-sspi v0.1.0`.
  - Its fifth row is `a_creation_through_the_window_succeeds_and_leaves_the_ends_closed`; the committed row is `a_creation_through_the_window_succeeds`.
  - The red rows call `launch::start`, `Ends` and `InheritWindow`, which do not exist in `d303888`. So "the original behaviour" was really the new code with only `pipe()` and `nul()` reverted to inheritable creation.
- **Violated invariant.** Evidence must be reproducible and bound to the reviewed object.
- **Impact.** The Windows red/green claim for `fa4ac93` is very likely right (the green row names match the commit, and the Mac rehearsal tree equals the commit tree), but it cannot be shown from the archive. The README's description of red is inaccurate. The README also does not mention that gwz-cli fmt exited 1 (`raw/mac/a/cli-py-gates.status`; the diffs are in files the lane did not touch).
- **Correction:**
  - Record the tested tree hashes, or rerun section B and the two mutations on a checkout of `fa4ac93` (seconds of work).
  - Archive the red patch, and describe red as "the new launch with creation-time inheritance restored".
  - Note the gwz-cli fmt exit as a pre-existing failure.
  - The windows-2025 CI job after the push also binds the five rows, because they are not `#[ignore]`d and run under `cargo test --lib --locked`.
- **Regression test.** Not applicable; this is evidence hygiene.

### P3-3: C9 says D25's gate covers gwz-sspi's window, but 0.1.1 gives a host no seam to take that gate

- **Location:** gwz-core `GwzTransportWindowsHelperOwnerContract-DRAFT.md`, the C9 line (lane diff), which says gwz-sspi's "window is closed to a std spawn by the same spawn gate (D25)". On the gwz-sspi side:
  - `api.rs:47-60, 70-90`: `Supervisor::new(executable, options)` and `start`.
  - `values.rs:33`: `Options { pub max_workers }`, which is not `#[non_exhaustive]`.
  - `monitor.rs:61-68`: launch runs on gwz-sspi's internal driver thread.
- **Root cause.** The worker launch happens inside gwz-sspi at a time the host cannot see. No public API lets the host's gate be held around `InheritWindow`. `CHANGELOG.md` says 0.1.1 makes "no change to the public API".
- **Violated invariant.** The contract describes a closure mechanism that the pinned dependency cannot take part in.
- **Consequence.** D25 ("build it before S7.1", which names the "gwz-sspi worker launch") can cover gwz-sspi only in one of two ways:
  - **Coarse gating:** gwz-core holds the gate across the whole `Supervisor::start` future, including capacity and Hello waits of up to the deadline. Every gated std spawn then blocks for that long.
  - **A new gwz-sspi release:** add, for example, a constructor that accepts a gate. That is additive, so it could be 0.1.2. It also means a second move of the `=0.1.1` pins in gwz-core, gwz-cli and gwz-py.

  The D25 implementer may otherwise assume coverage that does not exist.
- **Correction.** In C9 and the D25 plan step, state which route covers gwz-sspi. If it is the API route, record that gwz-sspi needs a further release before S7.1. This is not a reason to hold 0.1.1, which is correct as a step.
- **Regression test.** The D25 step should run the contract's control spawn concurrently with a gwz-sspi launch, under the gate, and show that it inherits none of the three ends.

## Invariant analysis (attacks that failed)

**Handles are inheritable only inside the window.**
- `pipe()` passes null `SECURITY_ATTRIBUTES` to `CreatePipe`, and `nul()` passes null to `CreateFileW`, so both are non-inheritable from creation.
- The parent ends (`parent_input`, `parent_output`) are never passed to `set_inheritable`, so they are never inheritable. This also closes the old gap between creation and `noninherit()` that P3-1 step 3 described.
- The removed `noninherit` was replaced, not dropped; nothing else in `src/` sets `HANDLE_FLAG_INHERIT`.

**The window closes on every path.**
- `open` builds the guard before the first `set_inheritable`. On a partial failure, the `?` drops the guard and clears all three; clearing a flag that was never set is harmless.
- Every fallible step comes before `open`: `Attributes::new`, the path validation, `roots()`, `origin.verify()`.
- Only the FFI call `CreateProcessW` sits between `open` and the explicit `drop(window)`, so no panic is possible there. An unwind would run Drop anyway.
- `drop(window)` runs unconditionally before `created == 0` is checked.
- On every return from `start`, `ends` is still owned by `create_in_job` and is closed right after: by `drop(ends)` on success, or by the unwind of `?` on failure.
- A `SetHandleInformation(false)` failure in Drop is ignored. The handle is closed microseconds later, so the remaining exposure is negligible.

**The handle list matches the window exactly.**
- `Attributes::new(job, [ends.input.0, ends.output.0, ends.stderr.0])` names the same three handles the window flags, and `hStdInput`, `hStdOutput` and `hStdError` are the same three.
- The list is checked only at `CreateProcessW`, after `open`, so the order is correct. Mutation 2 shows empirically that leaving out the window makes creation fail.

**The worker side still validates.** The child's copies keep the inherit flag they had at creation, so `inherited::validate` (`inherit & HANDLE_FLAG_INHERIT != 0`) still passes. The native fixtures confirm this. The worker clears onward inheritance on its pipe ends, as before.

**Concurrent launches cannot leak to each other.** Two gwz-sspi launches, or a gwz-sspi launch and a WH2 helper launch, cannot take each other's ends, because each names a handle list. Only spawns that name no handle list are exposed (P3-1, P3-3).

**Lifetimes are enforced by the borrow checker.** `InheritWindow<'a>` borrows `&'a Ends`, so the guard cannot outlive the handles, and a handle's value cannot be reused while it is flagged.

**No secrets are exposed.** `Ends` and `InheritWindow` have no `Debug`. Errors are the fixed kinds `ContainmentFailed` and `WorkerUnavailable`. The command line carries only handle numbers.

**The tests observe the OS and run on CI.**
- `inheritable()` calls `GetHandleInformation`; it does not mirror the code.
- The five rows are not `#[ignore]`d, so CI's windows-2025 job runs them under `cargo test --lib --locked`, and the cross jobs compile them.
- The failure row really goes through the window. Mutation 1 fails it, which proves the window opened on that path. Its `WorkerUnavailable` can only come from line 204, because line 152 does not apply to that path.
- One residual gap: no committed row observes the ends after a *successful* creation; the earlier draft's name suggests it once did. Breaking that would need a deliberate `mem::forget(window)`, and the ends close immediately anyway. This is not a finding.

**Repository rules.** No new `#[cfg]`; the tests are included in the existing braced `#[cfg(test)] mod native { … }`. All control flow is braced. No globals or thread-locals.

**Release mechanics.**
- Version 0.1.1 in `Cargo.toml` and the lock.
- `CHANGELOG.md` is in `include` and in the package list.
- `release.yml` checks that the tag is `v{version}`, and `release_checks.py` passes in the rehearsal.
- The pin shapes in gwz-cli and gwz-py `release.py` accept only the two `=0.1.1` forms, so a stale `=0.1.0` is refused. The tests are updated, and the evidence shows them passing: 29 and 47.
- `.github/gwz-sspi.commit` still names `d303888`, as intended.

## Follow-ups (outside this object)

- gwz-cli `docs/HostPackaging.md` and gwz-py `RELEASE.md` still say SSPI is "currently publish=false and unavailable/unpublished". That has been stale since `73de8c8` and the `v0.1.0` tag; the lane edited the neighbouring sentences.
- D24 says "gwz-core … pin `=0.1.1`", but gwz-core's manifest has an unversioned path dependency (`gwz-core/Cargo.toml:127`); only its lock moved. Record that gwz-core's registry pin comes in Phase 8 (D27).
- gwz-cli `cargo fmt --check` fails on files the lane did not touch (`raw/mac/a/cli-fmt.log`).

## Risks and next action

**Push ordering (transient red CI).** Cargo refuses a path dependency whose version does not meet the requirement.
- gwz-core's lock now says 0.1.1 while its pin file names a 0.1.0 commit. If the lock is committed without moving `.github/gwz-sspi.commit` in the same commit, every `--locked` gwz-core job fails. gwz-cli CI fails the same way, because it checks out gwz-sspi at gwz-core's pin.
- gwz-py's workflows check out gwz-sspi `main` without a ref. Once gwz-sspi main is 0.1.1, gwz-py main (`=0.1.0`) fails until its pin commit lands.
- Order: push gwz-sspi main, then gwz-core with the lock and pin together, then gwz-cli and gwz-py. Read CI after each push.

**The residual window remains** until D25 lands and covers gwz-sspi's launch (see P3-3 for the seam).

**Next action:**
1. Fix P3-1's wording before tagging.
2. Bind the Windows rows to `fa4ac93` (P3-2), or rely on the post-push windows-2025 CI run.
3. Add the gwz-sspi coverage route to C9 and D25 (P3-3).
4. Push in the order above, then tag `v0.1.1` on the commit on gwz-sspi main through gearu.
