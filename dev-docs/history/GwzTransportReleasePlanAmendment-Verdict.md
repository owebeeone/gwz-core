# GWZ transport release plan amendment — first review verdict

Date: 2026-09-27. Status: **NO-GO at SHA-256 `9ef88e44a7307a6d6b56c10719b26d3bdcf65a5b85942a575e23233be008fb9e`: [Consistency](GwzTransportReleasePlanAmendment-ReviewConsistency.md) and [Safety](GwzTransportReleasePlanAmendment-ReviewSafety.md) both reported NO-GO. Both reviewers committed in advance to GO on a revision that resolves their blocking findings as specified.** This verdict accepts nothing.

The object was the uncommitted draft `gwz-core/dev-docs/GwzTransportReleasePlanAmendment.md`, identified by its SHA-256, against root `3480a761`, gwz-core `4bd92285`, gwz-cli `ebbea902`, gwz-py `0b535dc5` and gwz-transport `a7a36aec`. The plan it amends was read at SHA-256 `48cccf58…`.
- Two fresh reviewers ran in parallel. Neither read the other's report.
- Both verified the object, the plan and the five HEADs at the start and the end.
- Both ran only inspection commands, over the workspace and the Cargo registry's native-tls, libssh2 and OpenSSL sources.

| Axis | Verdict | P0 | P1 | P2 | P3 |
| --- | --- | --- | --- | --- | --- |
| Consistency | NO-GO | 0 | 0 | 2 | 9 |
| Safety | NO-GO | 0 | 0 | 3 | 4 |

## Blocking findings

| ID | Axis | Finding |
| --- | --- | --- |
| C-P2-1 | Consistency | OD12's "if yes" edits say only that two clauses "drop their OD12 clause". Applied literally, they leave §1, §2 and §9 with no positive statement that the SSH remote form ships, or they call it unsupported and out of scope. |
| C-P2-2 | Consistency | Plan line 561 stays authoritative and excludes "a separate-process wire other than the local server socket", which the stdio mode is. |
| S-P2-1 | Safety | The must-match floor for native routes through a server is anchored to the contract's §5.8 disclosure. That disclosure omits OpenSSL's `SSL_CERT_FILE` and `SSL_CERT_DIR`, which libgit2's native HTTPS stream on Linux reads from the server's environment. A routed operation would then silently verify against the server's trust roots. |
| S-P2-2 | Safety | The SSH remote form's launch rule stops option injection but not OpenSSH's expansion of the host and user into `ProxyCommand` and similar settings, which run through the user's shell. Clients before OpenSSH 9.6 do not check those for shell characters (CVE-2023-51385). |
| S-P2-3 | Safety | The remote form does not say how `ssh` is found. On Windows, gwz-py's CLI would search the caller's directory, which is inside a workspace, before `PATH`. |

Both reviewers classified every finding as a bounded text correction. No finding is architectural. This was the first round; the two-round cap has one round left.

## Blind convergence

Both axes, reviewing blind, landed on the same clauses:
1. **The must-match floor for native routes.**
   - Safety P2-1: it omits the TLS-root variables.
   - Consistency, as a residual: it "under-describes the native HTTPS route", and TR1.3 must enumerate the reads.
2. **`GWZ_SERVER` and the SSH remote form.**
   - Consistency P3-4: the address parser admits the form in `GWZ_SERVER` while question 1 leaves that open.
   - Safety P3-3: the amendment's own evidence about unread sources says to refuse it there.
3. **The `auto` key and the native-route refusal.**
   - Consistency, as a residual: Phase 7's row is satisfiable only at an explicit address.
   - Safety P2-1's third correction: the risk text must not rely on the `auto` key.
4. **Agent confirmation under the stall clock.** Both accept recording it rather than changing it.
   - Consistency, as a residual: changing it would amend more than the retry plan.
   - Safety, as a residual: the notes should name `--ssh-timeout 0` and the repeated prompts.

## Next action

[GwzTransportReleasePlanAmendment-RemPlan.md](GwzTransportReleasePlanAmendment-RemPlan.md) maps each finding to one disposition and one closure test. The whole mapping is applied as one revision. The same two reviewers then give a focused re-verdict on the new SHA-256, with the plan and the diff.
