# TR2.8 — CODE-AXIS RE-REVIEW, ROUND 1

**Review object:** Bounded remediation diff in gwz-core `e3e5f12672efdcba66663a4b9a7fb8663cc94513..c51b1b5ff75a772f42847ce22fa4f35a61cf4497`, against the original Code findings and `dev-docs/GwzTransportSshKeyTypes-RemPlan.md`.

**Baseline:** Workspace `/Volumes/projects/limbo/gwz-dev-tr2-8`; root `2f65e3c898fdd9c9eb78b7557339613aab1c8ba7`; gwz-core `c51b1b5ff75a772f42847ce22fa4f35a61cf4497`; gwz-transport `6910ba669ccc654e11e7d7cc4a6a1f76b0db51b4`; git2-rs `d13951f7e0bfb6e0efcee1207ac5b140adefa455`. The exact tuple matched at both start and end.

**Date:** 2026-10-02.

**Axis:** Code — original counterexample closure, changed call paths and ownership, and regressions introduced by remediation. Independent, adversarial, read-only. No current peer re-verdict was read or used. Filed verbatim by the lane owner.

**Verdict: GO** — both original P2 findings and the original P3 finding are closed. No new findings or new architectural root cause were identified. The original pre-commit-to-GO conditions are satisfied.

---

## 0. Evidence base

The original authority and invariant analysis remain applicable. For this bounded re-review, I read:

- `GwzTransportSshKeyTypes-RemPlan.md` and the changed implementation-status document.
- The complete production remediation in `agent_keys.rs` and `ssh_key_container.rs`.
- Changed fixture ownership in `ssh_tests/key_fixture.rs`, including startup-failure tests.
- Changed regression tests in `agent_keys.rs`, `agent_auth.rs`, `agent_fixture.rs`, `rsa_sha1.rs`, `key_container.rs`, and `key_files.rs`; the new one-shot malformed-response mechanism in `key_agent.py`.
- External `remediation-red.log`: lines 36–57 show both original blocking counterexamples failing on the prior implementation.
- External `remediation-ssh.log`: line 193 records **153 passed, 0 failed, 3 ignored**. Lines 58, 72, 82, 86–87, and 191 record successful execution of the relevant closure tests.

Source inspection used `git diff`, numbered reads, and targeted searches. No builds, tests, mutations, or file writes were performed by this reviewer. External logs are supplied execution evidence.

## 1. Prior-finding closure

| Original finding | Disposition | Closure evidence |
| --- | --- | --- |
| P2-1 — RSA downgrade bypassed signature/key validation | **Closed** | `agent_keys.rs:102–127` now validates method agreement, the key, signature length, and trailing bytes before returning `Downgraded`. Plain/certificate malformed-response cases are covered in `ssh_tests/agent_keys.rs:245–279`. Production tests assert one sign request, no fallback query, and TCP disposal. |
| P2-2 — CR boundaries hid native-readable key blocks | **Closed** | `ssh_key_container.rs:57–68` recognizes both CR and LF, consuming CRLF together. `key_files.rs:119–140` rejects actual two-key input and an encrypted first key at snapshot admission, and authenticates valid single-key CR/LF/CRLF files. |
| P3-1 — Fixture construction leaked spawned children | **Closed** | `key_fixture.rs:42–48` introduces kill-and-wait ownership; each successful spawn is immediately wrapped before further fallible work. Startup failures after either spawn and failed key loading verify termination and reaping at lines 173–207. |

### P2-1 closure detail

At [agent_keys.rs:110](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/agent_keys.rs:110), the revised path calls `self.key(blob)` before admitting any downgrade. RSA replies must satisfy the existing modulus-length check at line 118 and the no-trailing-data check at line 121. Only then can line 127 return `Signed::Downgraded`.

The original oversized response therefore returns `InvalidData` rather than reaching `LIBSSH2_ERROR_ALGO_UNSUPPORTED`. [rsa_sha1.rs:55](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_tests/rsa_sha1.rs:55) verifies both short and oversized responses through production authentication: exactly `list`, `sign:0:4`, and only the SHA-2 query. [agent_auth.rs:211](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_tests/agent_auth.rs:211) additionally verifies connection disposal.

The fixture damages only the first response, so these tests cannot pass merely because every later response would also fail. The retained valid fallback row at `rsa_sha1.rs:81–90` still demonstrates the authorized second offer.

### P2-2 closure detail

The scanner now exposes the CR-separated block previously hidden inside skipped text. Existing duplicate-key and encryption checks therefore see it.

[Key-files regression:119](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_tests/key_files.rs:119) uses two independently generated keys, then encrypts the first key for the second rejection case. Both constructions fail `Registry::start` completion with `InvalidInput`, before an admitted snapshot can dispatch native authentication. The same test authenticates valid CR, LF, and CRLF single-key files through the existing snapshot/native path.

### P3-1 closure detail

[OwnedChild:42](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_tests/key_fixture.rs:42) owns cleanup throughout construction. The guards are installed before readiness checks, key loading, proxy observation, and ready-response parsing. Successful construction transfers those guards into fields ordered to dispose of the proxy before its upstream.

The startup test checks both `ESRCH` from process existence checks and `ECHILD` from `waitpid`, covering live-process leakage and unreaped children.

## 2. Changed-range analysis

The production changes repair validation order and parser boundaries within existing owners. They add no public API, dependency, protocol, allocator boundary, or production lifecycle.

The stricter downgrade path preserves ordinary signature extraction, security-key flags/counter framing, and the existing once-per-key fallback bound. It also verifies that the supplied algorithm agrees with the method before accepting a reply.

The CR/LF change retains the bounded cancellation-aware scan. Cursor advancement consumes a delimiter or remains at EOF; CRLF consumption does not suppress a nonempty block. Existing single-key, encryption, and surrounding-block rules continue to apply.

Test changes use the production authentication and snapshot paths. Fixture fault injection now resets after its first use, allowing a later valid response to expose an erroneous retry. Child cleanup ownership is established immediately, including during startup failure.

**New architectural root causes:** none. These are bounded corrections to the two original production root causes and the fixture ownership defect.

## 3. Risks and next action

This GO closes the Code review of the revised implementation. It does not assert completion of the final full candidate legs or clippy, whose results were not established here. Hardware-key manual execution and deferred platform qualification remain outside this implementation acceptance.

Next action: record this Code GO against the unchanged revised tuple and complete the remaining validation before recording overall acceptance.
