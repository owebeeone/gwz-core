# GwzRemoteTransportGhChallengeReuseAmendment — SAFETY-AXIS REVIEW

**Review object:** Draft `gwz-core/dev-docs/GwzRemoteTransportGhChallengeReuseAmendment.md` at core `51e28002e3a9f78f5a8daa873e02b083e2b2341f`, file SHA-256 `0103055f7ad5a6dd01d7ae9cb662cf61d624f89c954db983020a8a4b892e5fa3`.

**Baseline:** root `b567fe46ea0112872772f9e532dad7c498cc69b4`; core `51e28002e3a9f78f5a8daa873e02b083e2b2341f`; transport `14f0d09cbca7ab54c06d36e8354799981584a7f9`; CLI `1e52b274ed242d81349b6d18c107680152ecfecf`; Python `d484e367d1df47eff5caf3421864e69ec85744f5`. All committed sources were read with `git show HEAD:`.

**Date:** 2026-09-23

**Axis:** Safety—authentication and credential containment, degraded response handling, cancellation, cleanup, deadlines, connection disposition and replay boundaries. Independent, adversarial and read-only. The parallel Consistency report was not consulted.

**Verdict: GO** — no P0–P3 findings.

---

## 0. Evidence base

Read the amendment in full; `GwzRemoteTransportHttpsDesign.md` §§4, 6, 7 and 10; the transport design’s pool ownership, lifecycle and timeout contracts; the Python design’s persistent-runtime and cancellation rules; and the private evidence run README plus `gwz-py-native-https-trace2.log`. The evidence establishes the current physical-socket churn but does not claim that this draft correction is implemented.

The amendment hash matched the specified SHA-256 at both boundaries. All five repository HEADs matched the exact tuple at start and end. Concurrent uncommitted root/core changes appeared during the review; they were not read as authority and did not change any reviewed commit or the amendment blob. No builds, tests, writes or peer reports were used.

## 2. Invariant analysis

The replay boundary remains closed. Only an anonymous discovery GET receiving `401` or `404` may take the already-authorized, once-only Gh transition. It retains the operation, request, route and remaining cumulative budgets. POST, network failure, other status, redirect and preemptive-auth paths gain no retry.

Credential containment holds across physical reuse. The challenged request sends no Authorization; Gh credentials are looked up afresh and attached only to the next logical request for the same canonical HTTPS destination. The connection stores neither a token nor an authenticated-account fact. A later operation begins anonymously again, so prior use of the TLS socket cannot skip its challenge or helper lookup. Existing restrictions continue to exclude credentials from errors, evidence and public observations.

Stale-body and framing attacks fail closed. Reuse requires complete response-body drain within both the fixed 64 KiB cap and existing cleanup allowance. Truncation, malformed framing, cap exhaustion, cancellation or deadline expiry discards the lease. `Connection: close`, peer closure, sender error or lack of Hyper readiness also discards it. Sender readiness is treated only as eligibility; a subsequent peer-close race becomes a typed failure and cannot trigger a network retry.

Cleanup cannot refill an operation’s budget or create an unbounded wait. The authorization transition uses the remaining cumulative allowances, while close cleanup remains independently bounded even when network timing is disabled. Failed drain, cancellation and Gh failure retain their own terminal outcome and connection disposition. The first anonymous receipt remains private; only the final attempt becomes the public operation result.

The exception does not widen mixed-version exposure. It changes endpoint-local physical disposition without adding a Taut field, wire version, observation meaning, public API, credential source or placement behavior.

## 3. Risks and next action

This verdict accepts the draft shape only. Implementation, real-account GitHub proof, release qualification and physical-wire work remain deferred.

The next gate must provide the specified causal authenticated-fixture regression and adverse cases for oversized or truncated bodies, server close, sender error, cancellation, cleanup expiry and Gh failure. Those tests must prove both socket disposition and fresh per-request credential lookup; this review does not certify their future implementation.
