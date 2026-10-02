# TR2.8 — bounded remediation, round 1

Date: 2026-10-02. Reviewed core: `e3e5f12672efdcba66663a4b9a7fb8663cc94513`.
Both [Code](GwzTransportSshKeyTypes-ReviewCode.md) and
[State](GwzTransportSshKeyTypes-ReviewState.md) report NO-GO. They independently
converged on malformed RSA downgrade validation and fixture startup cleanup.

| Findings | Disposition | Closure test |
| --- | --- | --- |
| Code P2-1; State P2-1 | Validate the RSA key and signature size before admitting the single SHA-1 fallback. | Plain and certified RSA reject short, oversized, empty, truncated and trailing-data downgrade replies and malformed keys. Production authentication with a malformed first response must make one sign request, no fallback query and close its connection; retain the valid fallback test. |
| Code P2-2 | Recognize CR and LF as native line boundaries, including CRLF, so skipped text cannot hide native-readable keys. | Mixed-line-ending files containing two different keys, including an encrypted first key, fail container and snapshot admission before authentication. Single-key CR, LF and CRLF files remain covered. |
| Code P3-1; State P3-1 | Install kill-and-wait ownership immediately after spawning each fixture child. | Startup failures after the upstream and proxy spawns unwind through cleanup; child PIDs are terminated and reaped. |

One patch covers all findings. The existing private API, native allocation
boundary and transport protocol stay unchanged. The same reviewers re-check
their original counterexamples on the revised settled tuple. This is the first
remediation round; no finding is self-closed by the implementer.
