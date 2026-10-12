# AGENTS

Follow the root `AGENTS.md` rules.

- Work TDD-first: failing test, implementation, green tests, then refactor.
- Keep `gwz-core` independent of `gwz-cli`.
- Do not add behavior beyond what `GWZDesign.md` (authoritative) and `GWZRequirements.md` (baseline) specify; update them before expanding scope.
- Protocol payloads are taut-defined; do not create a shadow protocol.
- Build each new library as a small crate with a narrow API that builds and tests alone, under
  `crates/` (or `candidate-crates/` for crates only the candidate build compiles). Follow the
  [library boundaries](../dev-docs/GwzLocalCloneLibraryBoundaries.md) policy (LBT-001 to LBT-012)
  and the rules of the [core session crate map](../dev-docs/GwzCoreSessionCrateMap.md) §1: GWZ
  protocol encoding stays in core, secrets stay in core, and no crate depends on gwz-core.
- No global mutable state, and a thread-local counts as global state. Pass an explicit context;
  unique numbers come from a context's `IdSource`. The only exceptions are immutable data, caches
  of immutable data, and state a named dependency imposes.
- Windows compile gate (`GwzTransportWindowsParityPlan.md`, step 0.4). A lane that changes a
  trigger path runs `scripts/windows_lane_check.py` before it merges: the lane head is compiled on
  the Windows host (`cargo check` of the ordinary and transport shapes, as
  `candidate-windows` does) and the receipt is archived under a new immutable label. The trigger
  paths are every scope root of `scripts/checks/windows_parity/meta.json` (`roots`; the script
  derives them, so this list cannot drift from it), `Cargo.toml`, `Cargo.lock`,
  `tests/transport_backend/prepare.py` and `.github/*.commit`. `--if-triggered BASE` skips the run
  when none of those paths changed; `--tests` also checks the library in test mode, for a lane that
  edits test code; `--cache-from LABEL` makes a warm run. A pushed lane is covered by the
  `candidate-windows` job instead. The gate is a compile gate by default; Windows behaviour is judged
  by that job's test run and by the dabeest rows of the step that needs them. `--hosted-like` also
  RUNS the candidate library suites and the job's integration targets under the hosted job's
  conditions (its `run_tests.py` command and `--test` list, the pinned portable `sshd.exe`,
  `GWZ_TEST_SH`, a TEMP that is an 8.3 short path), takes about ten minutes warm and belongs under
  the host booking; its receipt entry names the failed tests, and the only failures it passes are
  the embedded-Python rows that need a gwz-py checkout and the four `protocol` rows that need
  `taut-proto` in the host's interpreter (named as gaps).
  A lane that edits tests or fixtures that run on Windows uses it, because `cargo check` cannot
  see a short TEMP, a slow process start or a shell that is not Git's. The lane gate
  (`scripts/checks/check_lane_commits.sh`) fails a lane that changes a trigger path unless a commit
  message at or after the change carries `Windows-receipt: <label>` (the label the run used), or the
  changing commit itself carries `Windows-receipt: ci-only <reason>` for a change that cannot alter
  how gwz-core compiles on Windows (the gate prints the reason for the reviewer; a waiver covers its
  own commit only). Every `cfg(unix)`-style gate, Unix-only OS
  call and runtime platform split (`cfg!(windows)`) in the transport also needs a row in the
  Windows-parity inventory, `scripts/checks/windows_parity/<step>.json` for the row's first owner
  step (`check_windows_parity.py`, which `run_tests.py` and the lane gate run; its docstring says
  how to add, move and remove rows). A row that becomes `platform` needs `reason` and
  `approved_platform` naming the recorded decision.
