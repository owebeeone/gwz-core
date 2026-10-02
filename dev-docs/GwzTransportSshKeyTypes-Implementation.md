# TR2.8 — SSH keys and signatures implementation

Date: 2026-10-02. Status: **implementation accepted at core
`c51b1b5ff75a772f42847ce22fa4f35a61cf4497` after
[Code](GwzTransportSshKeyTypes-ReviewCode-1.md) and
[State](GwzTransportSshKeyTypes-ReviewState-1.md) reported GO. This accepts
TR2.8's implementation only; release qualification remains below.**

## Scope and authority

[Transport release amendment 2](GwzTransportReleasePlanAmendment-2.md) §3.19
controls this step. The baseline is gwz-core
`bb67a8264a71a5141d3345a5d1367f4228aeb5db`; the inventory was committed first
at `485546f`. [The key inventory](GwzTransportSshKeyTypes.md) records source
citations, the platform distinctions and the test matrix.

The private signing bridge now admits Ed25519, RSA, the three NIST ECDSA
curves, DSA, both OpenSSH security-key types, and the supported types'
certificates (excluding DSA certificates). It validates signature framing
before handing bytes to libssh2. Security-key flags and counters are retained.
RSA flags request SHA-2, including certificates. SHA-1 is used in exactly the
three cases in amendment 2 §3.19; an agent's SHA-1 response to a SHA-2 request
allows one fallback for that key. Malformed responses terminate authentication;
an explicit agent refusal permits trying the next listed key.

Selected private-key files now accept the five container forms the inventory
identified as parity gaps. Reads remain bounded and cancellable, encrypted
containers are refused before native authentication, and native signing still
uses the captured key bytes rather than reopening the path.

No application protocol, transport stream protocol, dependency or public API
changes. The existing Unix allocator/agent bridge boundary remains: Windows
agent primitives and platform qualification belong to Phase 4. The new
software-authenticator fixture is Unix-scoped explicitly.

## Automated validation

The focused candidate SSH suite passed: 148 passed, 0 failed, 3 ignored
child-only tests (run by their parent tests). Disposable OpenSSH 10.3p1 and
the standard-library Python server exercise certificates, security keys,
agent refusal, selected-key formats, and the three SHA-1 cases. Full suite
results are recorded in the acceptance section once complete.

Both original full suites (candidate and ordinary) passed. The reviewers then
found two blocking root causes, recorded in the [remediation plan](GwzTransportSshKeyTypes-RemPlan.md).
Both regressions failed on the reviewed code (`remediation-red.log`). After
correction the focused SSH suite passed: 153 passed, 0 failed, 3 ignored
(`remediation-ssh.log`), including malformed downgrade responses, native CR/LF
parser agreement, snapshot rejection before authentication, and fixture startup
unwind/reaping. Full final candidate validation covers both candidate switches.

Build manifest: `/Volumes/projects/limbo/gwz-tr2-8-candidate-20261002`.
Candidate target: `/Volumes/projects/limbo/gwz-tr2-8-target-20261002`.
Local gate logs: `/Volumes/projects/limbo/gwz-handoff-2026-10-02/logs/tr2-8-final/`
(local handoff access required; these paths are not CI dependencies).

Production additions at takeover were about 397 lines, within the aspirational
500-line step budget; fixtures and tests are reported separately. No new
production owner or protocol delta was introduced.

## Remaining release qualification

The hardware security-key row requires the operator's explicit go under
amendment 2 §3.19. It has not run; the software authenticator covers both
security-key algorithms and their certificates automatically. Linux and Windows
release qualification remain in the plan's platform batch. Passing this lane's
review accepts its implementation, not those outstanding release rows.

## Acceptance record

Reviewed tuple: root `2f65e3c898fdd9c9eb78b7557339613aab1c8ba7`, core
`c51b1b5ff75a772f42847ce22fa4f35a61cf4497`, transport
`6910ba669ccc654e11e7d7cc4a6a1f76b0db51b4`, git2-rs
`d13951f7e0bfb6e0efcee1207ac5b140adefa455`. The closing commit changes only
review/acceptance records; its production sources match the reviewed core.

| Gate | Result and source |
| --- | --- |
| Original ordinary suite | Passed at `e3e5f126`; `RUST_TEST_THREADS=8 python3.13 scripts/run_tests.py --no-fail-fast`. Remediation changes only candidate endpoint code and its fixtures, excluded from ordinary builds. |
| Original full transport-only suite | Passed at `e3e5f126`; the same runner with the prepared manifest, external candidate target and `RUSTFLAGS='--cfg gwz_transport_candidate'`. |
| Corrected transport-only SSH suite | 153 passed, 0 failed, 3 child-only ignored rows at `c51b1b5f`; `cargo test --locked --manifest-path=<candidate>/Cargo.toml --lib git::endpoint::ssh_tests:: -- --test-threads=8`. |
| Corrected full transport-and-session suite | Passed at `c51b1b5f`; runner as above with `RUSTFLAGS='--cfg gwz_transport_candidate --cfg gwz_session_candidate'`, target `/Volumes/projects/limbo/gwz-tr2-8-target-both-20261002`; `final-both-suite.log`. |
| Format and source guards | `cargo fmt --all --check`, conditional boundaries, candidate inventories, checked-artifact boundary, process globals, filesystem and target-selection guards passed. |
| Per-commit boundary gate | Passed all three commits from `bb67a826` through `c51b1b5f`; `PYTHON=python3.13 bash scripts/checks/check_lane_commits.sh bb67a826 HEAD`. |
| Clippy | Candidate `cargo clippy --locked --manifest-path=<candidate>/Cargo.toml --all-targets` completed. **Strict `-- -D warnings` remains red on inherited warnings** (86 test diagnostics); none of the diagnostics are in files this lane changed. This is recorded debt, not a strict-gate pass or waiver. |

The candidate runner executes its own core/transport globals and crate-version
guards. `GWZ_TEST_PYTHON=/opt/homebrew/bin/python3.13` selected the disposable
Python fixtures for candidate runs. Logs listed above contain full commands and
counts; the multiple backend invocations are not a unique-test count.

Final production diff: 412 lines added and 152 removed in five files, below
the aspirational 500-line additions budget. Test fixtures are counted separately.
One dual review and one bounded remediation re-review closed two distinct P2
root causes and one P3 fixture defect, all found at settled review. No third
architectural root cause, or post-acceptance escape, was identified.

The lane is committed for integration. No merge, push, tag, publication,
hardware-key execution or release-platform qualification is implied by this
record. The strict-Clippy baseline remains an outstanding release check.
