# Credential implementation remediation round 1

2026-10-03. Working correction to the six distinct blockers and two Surface
follow-ups in `GwzTransportCredentialHelpers-RemPlan.md`. This is a drafter
implementation receipt, **not a reviewer verdict or acceptance**. Root owns
settlement, fresh Code/State review of the material admission API change, Surface
continuation and integration. Previous complete ordinary/candidate suites are
pre-correction evidence only.

## Correction and original counterexamples

| Finding | Corrected owner and deterministic regression |
| --- | --- |
| Code P2-1 / State P2-1 | Core `ssh_setup_context/publication.rs` owns the pending logical association and its unwind/drop release. The transport-neutral `SetupClock::terminate_if_alive` atomically distinguishes newly admitted from already committed terminals under the existing authority mutex. Core associates detail only for `Ok(admitted)`; even an equal scalar earlier resource cause cannot acquire candidate detail. Readers wait on the core association only, with no authority/runtime/owner lock held. Notifications and callbacks run after releasing locks; no cleanup is awaited. Tests cover independent pool observers, zero allocation/code75, immediate callbacks, pre/post-commit unwind, competing publishers, earlier network/cancel/equal resource winners and exact-deadline admission. |
| Code P2-2 | Challenge parsing retains Negotiate within four bounded tokens even when fifth/later; Basic eligibility still examines every scheme. Tests cover mixed case/duplicates/reordering, codec round-trip and actual private HTTPS clone. Clone stays loud `git_command_failed`, with no helper launch or realm leakage. |
| State P2-2 | Username capacity includes its SSH NUL before copying recognized sensitive bytes. SSH conversion cannot grow or release a populated allocation; repeated conversion keeps pointer/capacity, and HTTPS Basic excludes the terminator. Tests observe capacity/pointer and repeated SSH/HTTPS results without inspecting freed memory. |
| State P2-3 | Runner checks the unchanged interaction deadline and cancellation after successful child completion, before/after parsing and before answer admission. Parsing remains inside supervised job/process-group/permit ownership. Refusal drops/wipes the answer and terminates/reaps the group. Tests cover equality, final parse cancellation/deadline crossing and unpolled real completed children with timer/cancel simultaneously ready. |
| State P2-4 | The existing backend helper policy travels through private local SSH opening/handoff. Disabled operations reject helper work before password lookup, and use an opaque separate pool identity while preserving original key/agent selection. Enabled/disabled orders and reuse are tested with real password-only SSH; selected-key clone remains successful and each scope reuses only its own connection. Disabled private carriage fails closed on a supplied carrier that cannot carry it; no public wire change. |
| State P2-5 | After validation/admission, `Closed.failure` moves into first retained ownership; `Closed.facts` remains separately available. Real close handshake tests preserve detail/retry, error/facts and pointer ownership; invalid/late duplicate terminals cannot replace it. The private RPC adapter forwards typed separate facts only into its derived callback/error clone when Failure has none. Raw retained Failure is immutable; existing Failed facts take precedence. |

The new admission method preserves the existing `terminate` API and all clock,
phase, expiry, cancellation and wake rules. Its single atomic admitted/existing
result is necessary: observing Alive then comparing scalar cause after terminate
cannot prove exact admission. Root explicitly authorized this bounded correction;
no renewal, new phase, policy, wire, schema or transport secret owner was added.

Core call-site audit: `NativeResource` obtains its result after Control/result
locks release; pool observers call `failure` after pool `take` returns; OpenRequest
now clones `setup_slot` into a local before callback/failure projection. Rejection
already cloned first. `terminate_failure` drops the publication guard before any
error projection. No production caller waits on its own pending publication.

Original RED: core v1/v2 captured the first five defects, v2 **5 failed / 11
incidental old passes**. Closed v5 captured **1 failed / 1 pass** after correcting
fixture construction errors (earlier v1–v4 are retained and are not closure
proof). Atomic admission RED captured **0 passed / 1 failed**, with an equal
resource terminal incorrectly returned as admitted. RPC v5 captured its separate
facts consumer gap (**131 passed / 1 failed**); the broad `publication` filter
incidentally ran unrelated crash matrices and is not reused. These logs are
indexed and hashed in the external exact receipt. Original RED commands for early
runs are retrospectively reconstructed, not claimed as captured run envelopes.

## Surface and composition

Python follow-up changes only README and shared `TRANSPORT_HELP`: paired
`git config --file "$HOME/.gitconfig"` set/unset commands name GWZ's conventional
file and explain that `GIT_CONFIG_GLOBAL` makes ordinary `--global` edit another
file. Seven parser help artifacts include the same guidance. Root composed
accepted MAIN configuration/help and generated code75 through GWZ into Python
`34398b6ace772a5d85191273086a17363a404a4d` before final checks; old four-file WIP
is preserved in root's coordinated stash and must not be popped. No handwritten
Python driver/settings/runtime or generated-schema edit belongs to this patch.

The candidate CLI's six network help surfaces qualify 44/128-second examples as
network-clock examples, explain separate retained helper admission and full
interaction (up to 120 seconds), network pause during SSH helper phases and
code75 recovery. The ordinary CLI branch is unchanged. Runtime timeout/default
policy and settings are unchanged. Actual candidate binary help, not prose-only
mocks, is archived externally for Surface review.

## Reproducible gates and exact receipt

Final source/log hashes, toolchain/interpreter paths, member HEADs, gate cwd/argv,
whitelisted environment and final Surface artifact hashes are recorded in:
`/Volumes/projects/limbo/gwz-tr222-remediation-final-source-receipt-20261003.json`.
The external runner `/Volumes/projects/limbo/gwz-tr222-remediation-gates-20261003.py`
records argv/env/exit/log SHA for each final Cargo/source gate; it is not a product
or CI dependency. Candidate preparation is anchored to this lane. Test/build
profiles retain debug/incremental and ordinary optimization; no compiler/source
mutation probes or protected dependency edits were used.

Core cwd is `/Volumes/projects/limbo/gwz-dev-tr2-22`; exact candidate manifest is
`/Volumes/projects/limbo/gwz-tr222-candidate-core-20261003/Cargo.toml`.
Affected gate uses `cargo +1.95.0 test --offline --manifest-path <that manifest>
--lib -- transport_host ssh https transport_observations --test-threads=4`, with
`GWZ_TEST_GIT=real`, `GWZ_TEST_FS=real`,
`RUSTFLAGS=--cfg gwz_transport_candidate` and
`CARGO_TARGET_DIR=/Volumes/projects/limbo/gwz-dev-tr2-22/target/candidate-transport`.
Core Clippy uses the same manifest/Rust flags/target, `clippy --offline --lib`.
The JSON receipt expands every argv fully; angle-bracket shorthand here is prose.

Transport cwd is lane `gwz-transport`, `cargo +1.95.0 test --offline` including
doctest, then `cargo +1.95.0 clippy --offline --lib -- -D warnings`, with
GWZ_TEST_GIT/FS, RUSTFLAGS, CARGO_TARGET_DIR and PYTHONPATH unset. Its ordinary
retained member target is `gwz-transport/target`.

Final checks include cfg boundaries (disabled platforms too), candidate inventory
and process-global guard, all unchanged inventories/allowlists. Python uses this
lane's `.venv/bin/python`, explicit own-lane `PYTHONPATH`, parser suite and normal
protocol drift guard. Root-provided boundary-checker unit receipt is separate
pre-remediation evidence: 7/0,91.032 seconds, checker unchanged since `4039f821`.

One concurrent affected run was **454 passed / 1 failed / 4 ignored**. Its sole
failure was the unchanged ordinary `Job::start` address-fallback OpenSSH fixture
(3-second aggregate/1-second stall), outside the helper/shared-clock path. The
exact isolated row and bounded-concurrency final union determine its disposition;
the failed log remains preserved. No fixture timeout or assertion was weakened.
The initially unused original opening wrappers are used through the preserved
enabled path; the driver uses one existing private opening owner, and its ordinary fixtures pass the existing default enablement. The new Result notification return is explicitly consumed;
new warnings are not folded into inherited Clippy debt. Final core Clippy exits 0 with 49 retained warnings (previously 50; one callback type-complexity warning removed). The existing opening arity warning now describes its private helper boolean (9/7 instead of 8/7), remains unwaived, and is explicitly part of the revised source review.

Final gates: core affected **455/0/4 ignored**, core Clippy exit0/49 warnings; transport full including doctest **185/0/2 ignored**, strict lib Clippy exit0/no warnings; candidate CLI build and existing help/retry tests **3/0**; composed Python parser **28/0**, protocol drift exit0; cfg/candidate/global guards exit0. Seven Python and six actual candidate CLI help artifacts validate. Thirteen new core and three new transport regressions are enumerated separately from incidental old tests in the exact receipt. No full predecessor suite is relabeled as final.

## Cohesion and limitations

`rust-split explode` byte reconstruction receipts cover complete declaration
moves for SSH opening and the atomic admission regression, including attributes
and enclosing scope. New cohesive leaves remain below 500 lines. The pre-existing oversized fault-test owner receives only default-enabled argument additions at existing private-open fixture calls; no unrelated test owner migration is claimed. Existing
oversized handler owners remain the previously documented post-integration
refactor deferral; this patch does not move them or claim workspace migration.

No Windows, selected-source release, platform/performance or trust approval is
claimed. Initial Git discovery's accepted native-file read limitation and physical
cleanup budgets remain. Inherited warnings/ignored long randomized campaigns
remain labeled and unwaived. Source correction does not close reviewers' findings;
root must settle exact bytes and obtain required fresh Code/State and Surface GO.
