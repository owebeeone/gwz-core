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
- Windows compile gate (`GwzTransportWindowsParityPlan.md`, step 0.4). A lane that changes
  `src/git/endpoint/`, `src/transport_host/`, `Cargo.toml` or `tests/transport_backend/prepare.py`
  runs `scripts/windows_lane_check.py` before it merges: the lane head is compiled on the Windows
  host (`cargo check` of the ordinary, transport and qualification shapes, as `candidate-windows`
  does) and the receipt is archived under a new immutable label. `--if-triggered BASE` skips the run
  when none of those paths changed; `--tests` also checks the library in test mode, for a lane that
  edits test code; `--cache-from LABEL` makes a warm run. A pushed lane is covered by the
  `candidate-windows` job instead. The gate is a compile gate only; Windows behaviour is judged by
  that job's test run and by the dabeest rows of the step that needs them. Every `cfg(unix)`-style
  gate in the transport also needs an entry in `scripts/checks/windows_parity_inventory.json`
  (`check_windows_parity.py`, which `run_tests.py` and the lane gate run).
