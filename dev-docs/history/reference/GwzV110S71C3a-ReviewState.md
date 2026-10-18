# State-axis review: S7.1 (1.1.0) step C3a, the Cli placement and the 1.2.0 projection moved to `gwz_session_candidate`

- **Object:** lane `/Volumes/projects/limbo/gwz-dev-c3x`.
  - gwz-core `a5702b38` over `6f0699c9`.
  - gwz-py `cc2903d` over `c5bb918`.
  - gwz-cli `3d57acd` over `8d909ec`.
  - Evidence: gwz-core-evidence `34708231`, run `2026-10-16-c3a`.
- **Date:** 2026-10-16
- **Axis:** State. This covers request reach, refusal paths, fail-closed direction, runtime state, cleanup, stuck states, the tests that moved, and the evidence.
- **Verdict: GO.** No P0, P1 or P2 findings. Two P3 findings.

## §0 Evidence base

**Read fully:**
- `GwzV110S71ProtocolProposal-DRAFT.md`: all sections, including §1, §1.4, §5 C3a, §6 D1–D9 and the dispositions.
- `GwzV110RemainingPlan.md`, including Phase C.
- `src/transport_host/cli_placement.rs` (new).
- The full diffs of:
  - `transport_host/mod.rs`, `request.rs`, `session.rs`, `session/driver/opening.rs`;
  - `gitbackend.rs`, `transport_binding.rs`, `transport_support/identity.rs`;
  - `lib.rs`, `protocol/mod.rs`, `protocol/transport_capabilities.rs`;
  - `tests.rs`, `retry_tests.rs`, `qualification_tests.rs`, `https_tests.rs`, `cli_placement_tests.rs`;
  - `tests/protocol.rs`, `prepare.py`, `test_prepare.py`, `test_windows_qualification_boundary.py`;
  - Windows parity `4.4.json` and `5.1.json`;
  - gwz-py `dispatch/mod.rs`, `dispatch/merge.rs`, the inventory and `test_candidate_switches.py`;
  - gwz-cli (10 lint reason strings and nothing else).
- The evidence README, `receipt.json`, `windows-lane-check.log`, `win-run.sh` and `tests-moved-to-session.txt`.
- Of the runtime code, read fully:
  - `TransportRuntime` from `build_native` to `shutdown`;
  - `RequestContext::validate`, `validate_meta`, `max_retries`, `ClientRequest`;
  - `Session::cancel`, `seal` and `finish` (`session/requests.rs`), and `inbound_port` (`session/port.rs`);
  - the check paths in `pump.rs`, `close.rs` and `requests.rs`.

**Sampled:**
- `cli_driver_tests.rs` (harness, the host-scope test, the stall and think-time tests), `command_tests.rs`, `fetch_preflight_tests.rs`, `fault_tests.rs:499-590`.
- `ssh_pump_clock_tests.rs`, `transport_candidate_tests/drivers.rs` (test list), `handle_fetch.rs:40-110`, `g08.rs:790-850`.
- gwz-py `client.py:370-400`, the generated ordinary `RequestMeta` decoder, and `cbor.rs` (`try_get`).
- The CI workflow `transport-candidate.yml`.
- Logs: red1, red2, red3, green3, `green-tests-first`, the final checks, the three `run_tests` gates and the baseline.

**Not read:** the remaining unchanged session internals (`passes.rs`, `capacity.rs`), gwz-transport, and `docs/TransportPlacement.md` (D6, Code axis). I did not re-run the implementer's gates.

**Commands run:**
- Tuple check at start and end with `git log` and `status`.
- `git diff`, `git grep`, `git log -S`, and `stat` of the changed files' mtimes.
- A parse of the gate logs. The session-only set minus the transport-only set equals `tests-moved-to-session.txt`. Three extra names came out of interleaved output; I confirmed by name that they ran in both gates. The lib counts are 3794 transport-only, 3822 session and 3822 baseline.
- A comparison of the c21 and c3a Windows receipts.
- A probe build:
  - `prepare.py` into `build-targets/reviewc3a-state-prep`, then `cp -RL` to `reviewc3a-state-tree`.
  - Three probe tests appended to the copy's `src/transport_host/tests.rs`.
  - `RUSTFLAGS='--cfg gwz_transport_candidate' CARGO_TARGET_DIR=build-targets/reviewc3a-state-target cargo +1.95.0 test --offline --lib transport_host::tests::review_`. I dropped `--locked` because the copy's lock file had to update. It touched only the copy.
  - Result: **3 passed, 0 failed, 3794 filtered.**
  - All `reviewc3a-state-*` directories are deleted. At most one tree and one target were alive at a time.

**Tuple:**
- At start: gwz-core `a5702b38`, gwz-py `cc2903d`, gwz-cli `3d57acd`, evidence `34708231`.
- At end: identical.
- Untracked files: only the pre-existing ones (gwz-core dev-docs, the evidence alpha-setup-timeout directory). Nothing was modified.

## Findings

### P3-1. Tests moved session-only by harness rather than by the invariant they pin, leaving three shared-path invariants without a pin in any 1.1.0 shape

- **Root cause.** C3a moved whole tests to `cfg(all(test, gwz_session_candidate))` whenever they were built through `CliHarness` or `CliEndpoint`. Several of these pin behaviour of code the 1.1.0 Local route shares, and no Local row was left behind:
  1. **Endpoint bootstrap refusals.** `unregistered_bind_cannot_create_endpoint_authority` and `registration_dropped_before_bind_cannot_be_resurrected_by_late_message` (now in `src/transport_host/cli_placement_tests.rs:60` and `:107`) pin `Session::inbound_port` (`session/port.rs:9-15`) and the seal and registration path (`session/requests.rs`). The in-process Local endpoint uses that code for every request. `CliEndpoint::new` is only `Session::endpoint` plus `ClientRequest::new`, and both already exist in transport-only test builds. `Session::endpoint` is gated `any(test, …)` and is already called that way at `fault_tests.rs:501`.
  2. **The §10.1 stall clock, end to end.** `cli_a_server_stall_after_done_reaches_the_client_as_a_timeout` and `cli_client_think_time_after_the_advertisement_never_times_out` (`cli_driver_tests.rs:210`, `:237`; added in `22147b68` with the pump fix) are the only rows that carry the endpoint's Timeout through LocalLink, the driver and `BlockingStream` to the client. The harness is a driver plus an endpoint over a `LocalLink`, which is the Local route's own shape. Only the pump-level rows remain (`ssh_pump_clock_tests.rs:153`, `:201`).
  3. **The fetch refuses whole before any tracking ref moves.** `fetch_checks_later_repo_identity_before_any_tracking_ref_moves` is the only pin of `handle_fetch`'s `validate_transport_remotes` running before the fanout (`handle_fetch.rs:~100`). That ordering does not depend on the route.
- **Violated invariant.** A fail-closed behaviour the 1.1.0 shapes still have keeps a regression pin in a shape that the 1.1.0 gates and Windows run. This is mandate §4.
- **Sequence.** A change that drops the `registrations.contains_key` check in `inbound_port`, or that stops a pump Timeout reaching `BlockingStream`, or that moves `validate_transport_remotes` after the fanout, passes:
  - the ordinary gate;
  - the transport-only gate;
  - `candidate-windows` and the dabeest hosted-like suite.

  Only the Linux session CI leg would catch it.
- **Impact.**
  - Coverage only; behaviour is unchanged.
  - On Windows nothing runs the session shape. The c21 receipt had 3949 passed and 7 failed; c3a has 3926 passed and 4 failed. That is 27 fewer Windows rows, of which 24 had been passing. Those 24 include all five rows above.
- **Correction.** Add transport-only Local rows to `src/transport_host/tests.rs` (or `driver_tests.rs`), built on `Session::endpoint` and `ClientRequest::new` or on the Local `TransportRuntime`:
  - the two bootstrap rows;
  - an end-to-end stall-after-done row and a think-time row over `TransportRuntime::new` (the fixture's 3 s);
  - a Local `handle_fetch` row with a missing identity on a later member that asserts both tracking refs did not move. `transport_candidate_tests/drivers.rs` has the fixture pattern for this.
- **Regression test.** I demonstrated the first two. In the transport-only build, `review_unregistered_bind_cannot_create_endpoint_authority_transport_only` and `review_registration_dropped_before_bind_cannot_be_resurrected_transport_only` both passed. They are verbatim ports with `Session::endpoint(config())` and `ClientRequest::new` in place of `CliEndpoint`. So the coverage can be kept without the Cli placement.

### P3-2. Windows parity row 5.1 still records the S4.3/S7.2 Windows capability rule as "paired" at session-only code, and its only Windows pin is now session-only

- **Root cause.** C3a moved `capabilities()` and its `cfg!(windows)` split (`file_identity` and `exact_agent_identity` false on Windows) into `cli_placement.rs`, which only the session shape compiles. It re-pointed row 5.1 to that file, keeping state `paired` and the appendix "which Windows keeps false until a named fixture GO (GwzV110Plan.md S4.3, S7.2)" (`scripts/checks/windows_parity/5.1.json`). It also moved the only Windows assertions of it (`assert!(!caps.file_identity)` and `assert!(!caps.exact_agent_identity)`, `qualification_tests.rs:~110`) into a `cfg(gwz_session_candidate)` arm. No Windows job builds that arm.
- **Violated invariant.** A parity row names code and a test that exist in the shapes it governs. S7.2 says "Selected-identity Windows SSH is not advertised before that GO" (`GwzV110Plan.md:322-323`).
- **Sequence.**
  1. On Windows in the 1.1.0 shape, gwz-py's `_require_transport_capability` (`client.py:378-394`) calls `transport_capabilities::handle`, which answers `file_identity: true` on every platform (`protocol/transport_capabilities.rs:13-16`).
  2. The parity check still reports the Windows-false rule as paired.
  3. No 1.1.0-shape test on any platform asserts the Windows value.
- **Impact.**
  - The parity ledger misdescribes what the 1.1.0 build does on Windows. The S7.2/S7.5 reviewers relying on it would take the rule as implemented.
  - The static handler's Windows answer predates C3a; see follow-up F1. C3a removed the last Windows-built expression and pin of the rule, and kept the row "paired".
- **Correction.** Do one of these, then add the regression test below:
  - Mark row 5.1 as session-only and add a row, or an S7.2 open item, for the 1.1.0 capability answer on Windows.
  - Or bring the `!cfg!(windows)` rule into `transport_capabilities::handle` if the operator confirms S7.2 governs it. That is out of C3a's scope.
- **Regression test.** A transport-shape test, run on Windows, that asserts the 1.1.0 capability answer's `file_identity` and `exact_agent_identity` on Windows, whichever value is ruled.

## Invariant analysis (attacks that failed)

**Request reach in the transport-only build:**
- **No request can reach a Cli arm.** Each of these is gated `all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows))`:
  - `is_cli`, `route`, `refuse_budget_for_cli`;
  - `check_identity` in `RequestContext` and in the binding `Runtime`;
  - `is_cli_context`, `from_options_cli`, `validate_endpoint_files`, `opaque_path`;
  - the Cli scheme refusal in `resolve_remote`, and the placement guard in `with_transport`.
- **Stubs that silently accepted were removed, not kept.** The ordinary `Runtime::check_identity` returned `Ok(())` and `is_cli_context` returned `false`. Both are gone, so any un-gated caller would fail to compile. That is the fail-closed direction.
- **`RequestContext::is_cli()` returning `false`** feeds only `cli_hint` wording (`request.rs:151`, `:201`, `:331`; `transport_binding.rs:112`, `:227`). That keeps D8.
- **`validate_meta` and `check_identity`.**
  - In the transport-only shape the protocol has no `transport_message`, `placement` or `endpoint_path_base` field. `tests/protocol.rs::the_build_without_the_session_candidate_has_no_session_projection` pins the tag sets: TransportOptions 1–3, RequestMeta 1–9, ResponseMeta 1–8.
  - The `max_retries` range rules stay unconditional.
  - `path_base()` returns `None`. Before C3a, `validate_meta` refused any non-Cli base, so a Local Open's `path_base` was always `None` then too.
- **Unknown candidate tags on the wire.** My probe `review_session_tags_on_the_wire_are_ignored_by_the_transport_only_build` passed. A RequestMeta carrying TransportOptions tag 4 = Cli, tag 5 = a path and RequestMeta tag 10 = a map decodes equal to the meta without them and runs on the Local route.
  - This is ignore, not refuse. It is the protocol's additive contract (proposal §3, D2 `MISSING_OK`, `candidate_compat.rs`) and the ordinary build's behaviour at every earlier release.
  - No 1.1.0 API can write these tags: gwz-cli's Rust types and gwz-py's generated `api.py` have no such fields.
  - The ignored route is Local, which is what 1.1.0 does anyway. No authority, credential or path context is gained.
  - I record this as a fact for S7.5 Surface, not as a defect.
- **Mixed builds.**
  - RUSTFLAGS apply to the whole cargo graph, so gwz-py's `cfg(gwz_session_candidate)` `ResponseMeta` arms and gwz-core's protocol selection (also `gwz_session_candidate` alone) cannot disagree inside one extension. A disagreement would be E0560 or E0063 at compile time (shown in `red4-gwzpy-compile-transport-old-arms.log`).
  - The Python layer encodes the ordinary schema in both shapes, and MISSING_OK covers the candidate decoder. There is no cross-process GWZ wire in 1.1.0 (§3).

**Runtime state:**
- `io_timeout_ms`, `connect_timeout_ms` and `https` are removed from `RuntimeState` in every build, so any remaining reader would fail to compile. All builds compile, so nothing on the transport-only path read them.
- The Local driver is still built from `local.io_timeout_ms` and `pool.connect_timeout_ms` directly in `build_native`. That is unchanged, so there is no default-timeout drift.
- `max_retries` still flows: `request::max_retries` reaches `endpoint.set_max_retries` in `open_request` and `session.set_max_retries` in `RequestContext::new`. The new assertion in `retry_tests` checks the driver's record.
- `shutdown` keeps its order (endpoint, then local, then Cli) and its sums.
- `Drop for RuntimeState` closes the same sessions.
- The session-build `CliSlot::take` keeps the budgets for a later `install_cli`, which a closed runtime refuses.
- The empty slot (`CliSlot;` with no-op `close`, `take` and `cleanup` returning 0) is correct: nothing in that build can populate it.

**The Cli identity-check bookkeeping (`State.checks`, `Check`, reply handling):**
- The only inserter is `Session::check` (`opening.rs:349`), compiled only for `any(test, session)`. In a production transport-only binary `checks` is always empty.
- The pump's `checks.remove` falls through to the stream lookup, as it did for Local before C3a.
- No timer or pending reply can be held. `cancel` and `close_state` still drain the map in builds where it can fill.
- In transport-only test builds, `fault_tests.rs:571` still drives `check` over its own mux.

**Moved tests:**
- The 28 match the gate logs exactly.
- The Local counterparts that do exist:
  - `tests.rs:228-234` (scope mismatch, so `host_scope_rejects…` is covered);
  - `drivers.rs::candidate_command_drivers_share_pool…` and `candidate_service_refusal_skips_only_private_members…` (for `command_tests`);
  - the two remaining Local rows in `driver_tests.rs`.
- The gaps are P3-1.
- The `qualification_tests`, `https_tests` and `qualification` capability rewrites assert the same schemes and policies against the endpoint's bound offer. Only the Windows `file_identity` and `exact_agent_identity` assertions were lost (P3-2).

**Evidence:**
- **The red/green claims hold.**
  - red1 lists the NEW and STALE rows; green1 shows 72 matching sites.
  - red2 has 17 errors, all at production sites (`mod.rs`, `request.rs`, `gitbackend.rs`).
  - red3 is E0063 at `tests/protocol.rs:180`; green3 passes.
  - The gates exit 0. Lib counts: transport-only 3794, session 3822, baseline 3822.
- **The Windows receipt binds to `a5702b38` by timing, not by content.**
  - `receipt.json` has `commit` = `6f0699c9` (the parent) and `source` = "worktree".
  - The latest mtime of any changed file is 10:43:40. The lock was acquired and staging started at 11:16:23 and released at 11:29:53. `a5702b38` was committed at 11:32:47.
  - Nothing currently untracked lies outside dev-docs.
  - The README states it was an uncommitted tree on `6f0699c9`. The binding is credible but not machine-checkable (F3).

**Session switch alone:** `--cfg gwz_session_candidate` without the transport switch now selects `gwz_transport::cbor` and the candidate protocol, while gwz-transport is a dependency only under `cfg(gwz_transport_candidate)` (`Cargo.toml:125`). That shape would fail to compile. I reasoned this from the manifest and did not run it. No CI or script uses that shape, and C3b makes it the session shape with gwz-transport as an ordinary dependency. It fails at compile time, not at runtime, so it is not a finding.

## Follow-ups (outside C3a's boundary)

- **F1 (confirmed observation; S7.2 owns it).** `transport_capabilities::handle` answers `file_identity: true` on Windows. That answer is gwz-py's gate for identity options. S7.2 says selected-identity Windows SSH is not advertised before a named fixture GO. This predates C3a. Either confirm a GO exists or decide the 1.1.0 Windows value. It does not block C3a.
- **F2 (S7.5 Surface).** Record that the 1.1.0 build ignores, rather than refuses, the 1.2.0 tags on the wire (placement, path base, embedded message), with the probe above as evidence.
- **F3 (tooling).** `windows_lane_check.py` receipts for worktree runs should record a digest of the staged tree, so a receipt binds to the commit made from it by content, not by mtimes and timing.
- **F4 (evidence nit).** `logs/green-tests-first-transport-check.log` ends in `error: could not compile gwz-core (test "protocol")`, which is test 3's red state. Only the lib test target built. The README's "(test files edited first, builds)" should say "the lib test target builds; `tests/protocol.rs` is test 3's red".

## Risks and next action

- **Residual risk.** C3a changes no runtime behaviour on the 1.1.0 path. The risk is that the shared endpoint-bootstrap, stall-propagation and fetch-preflight invariants regress unseen in 1.1.0 shapes, Windows included, until a session shape runs there.
- **Next action.** Close P3-1 by adding the Local rows; the two bootstrap ports are already written and green in the probe. Close P3-2 with a row fix and an S7.2 item. Take F1 to the operator before S7.2.
