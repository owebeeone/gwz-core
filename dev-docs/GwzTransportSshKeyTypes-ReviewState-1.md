# TR2.8 — STATE-AXIS RE-REVIEW, ROUND 1

**Review object:** Bounded remediation diff in gwz-core, `e3e5f12672efdcba66663a4b9a7fb8663cc94513..c51b1b5ff75a772f42847ce22fa4f35a61cf4497`, under `dev-docs/GwzTransportSshKeyTypes-RemPlan.md`. Implementation checkpoint pending re-review, dated 2026-10-02.

**Baseline:**

| Repository | Exact revision |
|---|---|
| Workspace root | `2f65e3c898fdd9c9eb78b7557339613aab1c8ba7` |
| gwz-core | `c51b1b5ff75a772f42847ce22fa4f35a61cf4497` |
| gwz-transport | `6910ba669ccc654e11e7d7cc4a6a1f76b0db51b4` |
| git2-rs | `d13951f7e0bfb6e0efcee1207ac5b140adefa455` |

Sources were inspected using `git diff`, `nl`, `sed` and `rg`. The tuple matched at both start and end. Final `git diff HEAD` checks showed no differences in inspected remediation sources and records.

**Date:** 2026-10-02

**Axis:** Authentication transitions, interruption, resource ownership and fail-closed behavior. Independent, adversarial, read-only. Nothing here relies on the peer’s current re-verdict. Filed verbatim by the lane owner.

**Verdict: GO** — both original State findings are closed. No new P0–P3 finding or architectural root cause was identified in the remediation ranges. This accepts the bounded correction; it does not claim completion of pending aggregate or release qualification.

---

## 0. Evidence base

The original review’s authority and baseline analysis remain applicable. This round inspected:

- `GwzTransportSshKeyTypes-RemPlan.md:1–17`, including the combined remediation dispositions.
- The complete remediation diff, excluding peer report contents.
- `agent_keys.rs:93–183`, particularly validation before fallback at 102–127.
- `ssh_key_container.rs:47–76`, including CR, LF and CRLF boundaries.
- `ssh_tests/agent_keys.rs:245–280`: malformed plain/certified RSA downgrade cases.
- Changed native callback tests in `ssh_tests/agent_auth.rs:210–236` and one-shot response corruption in `ssh_tests/agent_fixture.rs`.
- `ssh_tests/rsa_sha1.rs:54–91`: malformed replies produce no fallback query/signature, while valid fallback remains supported.
- `ssh_tests/key_fixture.rs:27–208`: immediate child ownership and startup-failure termination/reaping.
- `ssh_tests/key_container.rs`’s added carriage-return cases and `ssh_tests/key_files.rs:118–140`: actual key snapshots reject ambiguous/encrypted inputs, while valid CR/LF/CRLF files authenticate.
- `tests/transport_backend/key_agent.py`’s one-shot malformed reply.
- Supplied external logs `remediation-red.log` and `remediation-ssh.log`.

No builds, tests, file writes or mutations were performed by this reviewer.

`remediation-red.log:36–57` records both targeted regressions failing before correction, including the original one-byte RSA signature counterexample. `remediation-ssh.log:193` records **153 passed, 0 failed, 3 ignored**, with the relevant regression rows individually passing.

Pending/running final candidate and clippy gates were not treated as completed evidence.

## 1. Prior-finding closure

| Original State finding | Disposition | Closure evidence |
|---|---|---|
| **P2-1 — RSA downgrade bypasses signature-shape validation** | **Closed** | `agent_keys.rs:102–127` now verifies RSA family and requested algorithm, validates the key, parses the signature, checks its modulus-sized length and rejects trailing bytes before returning `Downgraded`. Unit coverage at `ssh_tests/agent_keys.rs:245–280` includes plain/certified keys, empty/short/oversized/truncated/trailing-data replies and malformed keys. Production coverage at `rsa_sha1.rs:54–61` requires one sign request and only the original SHA-2 query. Callback coverage at `agent_auth.rs:210–236` requires `InvalidData` and TCP closure. These rows pass in `remediation-ssh.log:58`, `:86` and `:191`. |
| **P3-1 — Fixture children have no cleanup owner during construction** | **Closed** | `key_fixture.rs:42–48` provides kill-and-wait ownership. Both spawns are immediately wrapped at `:78–86` and `:123–133`, before subsequent fallible startup work. The completed fixture preserves proxy-before-upstream drop order at `:51–54`. Startup regressions at `:172–208` cover the original missing-key failure and injected failures after each spawn, asserting both termination and completed reaping. The row passes at `remediation-ssh.log:82`. |

No new findings.

## 2. Changed-range invariant analysis

**Fallback admission is now fail-closed.** The original counterexample cannot reach `Signed::Downgraded`: its one-byte signature fails `raw.len() == modulus` at `agent_keys.rs:118`. Key validation and trailing-byte checks also precede the transition. Valid fallback remains exercised at `rsa_sha1.rs:80–90` and passes in `remediation-ssh.log:184`. The existing callback invocation bound and native allocation ownership are unchanged.

**Production regressions distinguish refusal from forbidden retry.** The malformed fixture response is one-shot, so a mistakenly admitted second request would receive a usable upstream response. The tests therefore expose the original transition defect instead of making every possible attempt fail. Server-side request assertions independently verify that no fallback query was sent (`rsa_sha1.rs:59–60`).

**Child ownership covers construction failures.** Each spawned process receives its cleanup owner before the observation hook or later startup operation runs. Unwinding after either spawn invokes kill and wait. The regression checks both absence of the process and absence of an unreaped child (`key_fixture.rs:194–205`).

**The additional container correction preserves bounded scanning.** `ssh_key_container.rs:57–68` recognizes CR and LF and consumes CRLF together. Cursor movement remains bounded and retains the existing cancellation-aware scanner. Ambiguous two-key input and an encrypted first key now fail snapshot admission before native authentication (`key_files.rs:127–133`). Valid single-key CR, LF and CRLF files authenticate (`:135–139`); the row passes at `remediation-ssh.log:87`.

**No new architectural root cause.** These corrections remain within signature validation, container line recognition and test child ownership. They introduce no production durable state, public API, protocol change, allocator boundary or new authentication owner. Modified control-flow bodies remain braced; no new conditional declaration boundary was introduced.

## 3. Risks and next action

Hardware-key execution, Linux/Windows release qualification and Windows bridge/allocator work remain explicitly deferred. This re-review does not expand their acceptance.

The next action is to record this State GO against the settled tuple and complete the remaining aggregate validation before advancing the lane.
