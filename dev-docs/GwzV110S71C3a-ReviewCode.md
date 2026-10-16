# S7.1 (1.1.0) step C3a: Code-axis review

- **Object:** lane `/Volumes/projects/limbo/gwz-dev-c3x`, gwz-core `a5702b38` over `6f0699c9`, gwz-py `cc2903d` over `c5bb918`, gwz-cli `3d57acd` over `8d909ec`, evidence `gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-16-c3a/` (Windows receipt `c3x-20261016-a`).
- **Date:** 2026-10-16
- **Axis:** Code (architecture, interfaces, build shapes, wire, scope and requirement provenance)
- **Verdict:** **GO.** P0 0, P1 0, P2 0, P3 2.

The moves match §1's classification. No 1.1.0 production behaviour moved behind the session switch. The transport-only build compiles the ordinary protocol, and the new protocol test pins that. Both findings are about coverage and inert code; neither is a behaviour defect.

## §0 Evidence base

**Read in full:**
- `GwzV110S71ProtocolProposal-DRAFT.md`, including the dispositions.
- `GwzV110RemainingPlan.md` Phase C, and Amendment 2 §3.11 (S7.1, S7.3, S7.5).
- The whole gwz-core diff for these files: `lib.rs`, `protocol/mod.rs`, `protocol/convert.rs`, `transport_capabilities.rs`, `transport_host/{mod,cli_placement,request,session}.rs`, `session/driver/opening.rs`, `gitbackend.rs`, `transport_binding.rs`, `transport_support/identity.rs`, `transport_observations.rs`, `tests/protocol.rs`, every changed test module, the inventory, parity rows 4.4 and 5.1, `prepare.py`, `test_prepare.py`, `README.md`, `guide_test.rs`, and `docs/TransportPlacement.md`.
- The gwz-py and gwz-cli diffs in full.
- The evidence README, `receipt.json`, `tests-moved-to-session.txt` and the red3 log.

**Sampled:** `transport-candidate.yml`, `cli_driver_tests.rs` (the harness and three tests), `fault_tests.rs`, `wake_tests.rs`, `pump.rs`, `placement_endpoint/admission.rs`, the lint-reason hunks, and `check_candidate_switches.py`'s docstring.

**Not read:** the other `cli_driver_tests` bodies beyond their names (they are verbatim moves, confirmed by `--stat -M`), the gwz-cli `g02`/`g12`/`g15` bodies (reason-string changes only), and the gwz-core-evidence logs other than those named above.

**Commands and results** (all with external target dirs `build-targets/reviewc3a-code-{t,s,o}`, at most two alive at once, all deleted; logs kept in `build-targets/reviewc3a-code-logs/`):

| Command | Result |
|---|---|
| `cargo +1.95.0 check --tests --locked`, ordinary | exit 0 |
| same, `--cfg gwz_transport_candidate` | exit 0 |
| same, both switches | exit 0 |
| Warning sets, transport-only vs both switches | identical; all pre-existing, none in a C3a file |
| `--test protocol`, transport-only | 45 passed, including `the_build_without_the_session_candidate_has_no_session_projection` |
| `--test protocol`, both switches | 44 passed |
| `--test protocol`, ordinary, the two 1.1.0 tests | 2 passed |
| `--lib`, transport-only: `transport_host::tests`, `wake_tests`, `retry_tests`, `qualification_tests`, `fault_tests::oversized…` | 24 passed |
| `--lib`, both switches: `cli_placement_tests`, `cli_driver_tests::host_scope*`, `transport_host::tests` | 19 passed |
| `check_candidate_switches.py` (gwz-core) | 72 sites match |
| pytest: gwz-core `test_check_candidate_switches`, `test_prepare`, `test_windows_qualification_boundary` | 35 passed |
| pytest: gwz-py `test_candidate_switches` | 6 passed |
| pytest: gwz-cli `test_candidate_switches` | 2 passed |
| `rustfmt --check` on every changed `.rs` | clean, except `guide_test.rs`, whose diff is entirely in `guide_example.rs` (pre-existing; the note is accurate) |
| `#[path]` count | 57 in `src`, 8 in `tests`, the same at base and head; the guide's `#[path]` only moved into a `cfg_if` arm |
| Added lines checked for bare `#[cfg]`, `cfg_attr`, `thread_local`, `static mut`, `OnceLock`/`LazyLock` | none in all three repos |

**Tuple at start and at end:** gwz-core `a5702b38`, gwz-py `cc2903d4`, gwz-cli `3d57acd7`, gwz-core-evidence `34708231`. Unchanged. The only new untracked file at the end, `gwz-core/dev-docs/GwzV110S71C3a-ReviewState.md`, is not mine.

## Findings

### P3-1: 1.1.0 invariants that were pinned only through the Cli harness moved to the session-only shape, so the 1.1.0 build and Windows lose them

- **Root cause:** tests were classified by the harness they use, not by the behaviour they pin. Proposal U1 classifies by what an item serves, and C3a red test 4 says the Local-behaviour pins in `driver_tests.rs` must not move.
- **Location** (now under `cfg(all(test, gwz_session_candidate))`):
  - `src/transport_host/cli_driver_tests.rs:67` `host_scope_rejects_metadata_or_operation_changes_before_transport`. This is the only test of `RequestContext::validate`'s mismatch refusal (`request.rs:95-98`), which every workspace network handler calls through `validate_transport_scope`. `git grep` finds no other test of it.
  - `cli_driver_tests.rs:564` `cli_authentication_failure_retains_attempt_facts_without_claiming_an_open_stream`. This is the only pin of the C2 receipt-absence rule: a transport-route setup refused before any open leaves `endpoint_id`/`connection_id`/`stream_id`/`reused` as `None` while `credential_offered` is set. The C2 State review's P3-2 remediation (`6f0699c9`) added it; no other test asserts `(None, None, None, None)` outside `tests/protocol.rs`'s codec test.
  - `cli_placement_tests.rs` `unregistered_bind_cannot_create_endpoint_authority` and `registration_dropped_before_bind_cannot_be_resurrected_by_late_message`. These pin endpoint-`Session` authority, which the in-process Local endpoint shares: `open_request` admits the client before the driver's Bind (`mod.rs:392-411`), so a late Bind after a dropped registration is a 1.1.0 race. They use `CliEndpoint::new` only as a constructor, and `Session::endpoint` is still compiled in test builds.
- **Violated invariant:** 1.1.0 behaviour pins stay in the transport-only shape, which becomes the ordinary build at C3b and is the only candidate shape any Windows job builds.
- **Reproduction:** delete the `self.meta != meta || self.operation != operation` check in `RequestContext::validate`, or make `TransportAttempt::failed` write receipts. Every transport-only gate passes: `run_tests.py` in the transport shape, `candidate-windows`, `windows_lane_check.py --hosted-like`. Only the session leg on Linux and macOS fails.
- **Impact:** coverage of a host-scope safety guard, of a wire-field contract named for S7.5's Surface, and of endpoint authority. On Windows it is gone completely; these tests ran there before C3a. Production behaviour is unchanged.
- **Correction:**
  - Rewrite the scope test on a Local runtime: `TransportRuntime::new(config())`, a request with no placement, then mismatched meta and operation.
  - Add a Local-route refused-setup row (empty `authorized_keys` over `with_local_transport` or `TransportRuntime::new`) asserting the four receipts are `None`.
  - Port the two endpoint-authority tests to `Session::endpoint(config())` with `ClientRequest::new`.
  - Keep the Cli versions in the session shape.
- **Regression tests:** the three ports themselves, run in the transport-only and Windows hosted-like shapes. The Windows receipt should name them.

### P3-2: The Cli identity-check driver machinery stays ungated and is dead in the 1.1.0 transport build

- **Root cause:** `Session::check` was gated (`opening.rs:346`, `any(test, all(transport, session, unix|windows))`), but the state it feeds was not. Two related items were also left ungated.
- **Location:**
  - The state `Session::check` feeds:
    - `session.rs:131` `struct Check`
    - `session.rs:152` `State.checks`
    - `pump.rs:137-148`, the `IdentityChecked`/`IdentityCheckFailed` completion arm
    - `pump.rs:427`
    - `requests.rs:71`
    - `close.rs:39`, `:52`
  - Related, same class:
    - `mod.rs:394-411`: the `local_endpoint == None` branch of `open_request`, unreachable without the session arm.
    - `session.rs:52-55`/`:488-499`: `CHECK_MS` and `Session::driver(…, None)`, compiled into transport-only test builds only so that `wake_tests`, `fault_tests` and `tests.rs::a_driver_with_no_endpoint_in_process_refuses_a_url_password` (a Cli-shape driver with no in-process endpoint) keep compiling. That test pins a branch that is unreachable in 1.1.0 production.
- **Violated invariant:** proposal U1 says an item that "exists only to serve" the Cli placement is class (b) and moves to `gwz_session_candidate`. The repository rule says found dead code is removed now unless it is meant for 1.2.0+. This code is meant for 1.2.0, so it should be gated, not left ungated.
- **State sequence:** in the transport-only non-test build no code path inserts into `State.checks`. `check` is not compiled, and `opening.rs:381` is the only constructor of `Check`. So every read of `checks` iterates an empty map, and the pump arm can never fire. rustc does not warn, because the fields are read.
- **Impact:** after C3b this ships as dead code in the 1.1.0 ordinary build. It is invisible to the inventory, so neither the Surface review nor S7.1 (1.2.0)'s inventory check can tell live code from inert code. Reviewers of the 1.1.0 driver pump must reason about a path that cannot run. There is no correctness effect.
- **Correction:** pick one.
  - (a) Put the map behind a slot type with a stub arm, as `CliSlot` does: `checks: CheckSlot` with `insert`/`take`/`for_request`. Gate the pump arm through a `CheckSlot::complete` method. Change `fault_tests::oversized_open…`'s post-rejection probe from `check` to a second `open`. Add the new rows to the inventory.
  - (b) Have the operator record this as a named exception in the S7.1 record and in C3b's plan text, so it is not parked silently.
- **Regression test:** for (a), the inventory rows plus `check_candidate_switches.py`. Also a transport-only compile in which `Session::check` is absent from non-test builds, which already holds today.

## Invariant analysis (attacks that failed)

- **Classification against §1.**
  - All 8 class (b) cfg sites moved: rows 3, 4, 12, 13, 15, 17, 19 and 23.
  - The §1.4 Cli items are all under `all(transport, session, unix|windows)`: `install_cli`, `capabilities`, `remove_cli`, `CliEndpoint`, `require_cli_ssh`, the `is_cli` arm of `open_request`, the Cli handling in `close`/`Drop`, the `TransportPlacement` import, the `request.rs` `is_cli`/`transport_message`/`endpoint_path_base`/`path_base` rules, and the five `is_cli_context` uses in `identity.rs`.
  - The (a) rows are untouched.
  - The extra items the implementer moved are all Cli-only: `Session::check`, `CHECK_MS`, `driver`, `endpoint`, `endpoint_with_https`, `ssh_and_https_policies`, and the `RuntimeState` fields. The Local route uses `driver_with_ssh_budgets` and `endpoint_with_https_native` (`mod.rs:308-317`).
  - Keeping `driver`/`endpoint`/`endpoint_with_https` under `any(test, …)` is acceptable: they are thin wrappers. `check` is covered under P3-2.
- **No 1.1.0 behaviour moved.**
  - The `for_backend`, `check_selected` and `configured_path` refactors are equivalent to the old non-Cli arms in the ordinary and transport builds: `from_options` with `ssh_home`, then `validate_files`/`validate_file`/`resolve_path`.
  - D8 holds: `RequestContext::is_cli()` returns `false` without the session switch (`request.rs:80-85`), so `cli_hint` at `request.rs:151/201/331` and `transport_binding.rs:112/227` stays `false`, as before for gwz-cli.
  - In the session shape the `CliSlot` rewrite (`take`, `close`, `cleanup`, `install_cli` budgets, `capabilities().https`) is equivalent to the old `Option<Arc<Session>>` with its `io_timeout_ms`/`connect_timeout_ms`/`https` fields.
  - `validate_meta`'s split refusal keeps the same message and order.
- **No 1.2.0 leak into transport-only.**
  - `TransportCapabilitiesResponse` has two fields: the exhaustive literal, and the red log with E0063 before the change.
  - The tags of `TransportOptions` (1..=3), `RequestMeta` (1..=9) and `ResponseMeta` (1..=8) are pinned.
  - `pub use cli_placement::{…}` is gated.
  - The guide target is empty without the session switch.
  - The C2 fields are present in the ordinary schema: `ordinary_operation_policy_and_observation_carry_the_1_1_0_fields` passes in the ordinary build.
  - The corpus selection falls back to production vectors in transport-only `test` builds, which pins the whole wire.
- **Shapes.** All three compile `--tests`. A `--cfg gwz_session_candidate`-only build would not compile, because `gwz_transport` is a dependency only under `cfg(gwz_transport_candidate)`. No CI shape builds it, and C3b makes gwz-transport unconditional, so this is not a defect.
- **gwz-py and gwz-cli consistency.**
  - gwz-py's two `ResponseMeta` arms now select `gwz_session_candidate` alone. That matches gwz-core's module selection exactly, and it removes the earlier `any(unix, windows)` mismatch.
  - gwz-py's inventory and `test_candidate_switches.py` agree.
  - gwz-cli names no projection field (`git grep`), so its 19 class (a) rows and its unchanged inventory are right. Only reason strings changed.
- **Repository rules.**
  - Every new conditional is a `cfg_if!` arm. Nested arms inside `impl` blocks and fn bodies, and `use` statements inside session arms, are within the boundary.
  - The disabled arms were checked: the non-session `CliSlot` stub, the `is_cli`/`path_base`/`check_selected` else arms, and the Windows arm of `ssh_and_https_policies` (compiled only in the session shape).
  - Control flow is braced, there are no new globals, and no production code was added under `tests/`.
  - Files over 1000 lines: `tests/protocol.rs` (2107, +38) and `workspace_ops/tests/g08.rs` (1252, −6) were already oversize. No new file is over 700.
- **Lint reasons.** No `gwz_transport_candidate adds` string remains in gwz-core, gwz-cli or gwz-py. The two allows that served `OperationPolicy` and `TransportObservation` are removed, as finding 2 asked.

## Follow-ups (not findings)

1. `pub use session::{Attachment, TransportPort}` stays public in the transport build with no public constructor. `Attachment` names `gwz_transport::protocol::Envelope`. U3 ruled them class (a); S7.5's Surface review should note them as public 1.1.0 types that only 1.2.0 can produce.
2. In 1.1.0 the only capabilities answer is the static `transport_capabilities::handle` (`file_identity: true` on every platform). Parity row 5.1's "Windows keeps false until fixture GO" now applies only to the session-only runtime projection. This was the same before C3a (gwz-py always called the static handler). It is worth a check against S4.3/S7.2 for 1.1.0 on Windows. Unverified hypothesis.
3. The session shape has no Windows compile. `cli_placement.rs`'s Windows arms and parity rows 4.4 and 5.1 are never compiled on Windows. The evidence README records this; 1.2.0 needs a session leg in `windows_lane_check.py` and `candidate-windows`.
4. `docs/Embedding.md:108` still calls the placement API "candidate", which becomes ambiguous after C3b. This belongs to C5's docs.
5. The Windows receipt `c3x-20261016-a` is of an uncommitted worktree on `6f0699c9`, with no tree digest. Its correspondence to `a5702b38` is not mechanically shown (State-axis matter).
6. A stale `#[allow(unused_mut)]` on a non-`mut` binding at `transport_observations.rs:70` (pre-existing).

## Risks and next action

- C3b must rewrite about 29 session arms of the form `all(gwz_transport_candidate, gwz_session_candidate, …)` and the `any(test, …)` sites. The inventory lists every one under both switches, so this is mechanical.
- **Next action:** close P3-1 by porting the three tests into the transport shape and naming them in a fresh Windows receipt. For P3-2, either gate the check machinery with a slot type or get the operator's recorded exception before C3b.
