# GwzRemoteTransportGhChallengeReuseAmendment — SAFETY-AXIS RE-VERDICT

**Review object:** Corrected draft `gwz-core/dev-docs/GwzRemoteTransportGhChallengeReuseAmendment.md` and its merged remediation plan at core `e416faa1b23b578b5aa50eadea64a11c6561fdf9`; amendment SHA-256 `a9aba45fa0d19ce38ba0f9bef1fd37280a4612014555c0b2a75ad2f564fcafe9`.

**Baseline:** root `78775e818b8a6398aad0859aa46a6662440ab95a`; core `e416faa1b23b578b5aa50eadea64a11c6561fdf9`; transport `14f0d09cbca7ab54c06d36e8354799981584a7f9`; CLI `1e52b274ed242d81349b6d18c107680152ecfecf`; Python `d484e367d1df47eff5caf3421864e69ec85744f5`.

**Date:** 2026-09-23

**Axis:** Safety—authentication scope, credential containment, degraded response handling, cancellation, deadlines, replay boundaries and physical disposition. Independent, adversarial and read-only. The current Consistency re-verdict was not consulted.

**Verdict: GO** — the prior Safety GO remains valid; no P0–P3 finding or new architectural root was found.

---

## Prior-finding closure table

| Prior Safety result | Status | Evidence |
|---|---|---|
| Round 1 contained no Safety findings | **No closure required; GO retained** | The complete amendment delta was attacked against the original authentication, replay, cleanup, deadline and credential-containment invariants. The correction narrows and clarifies the exception without weakening them. |

## Changed-range analysis

Lines 44–55 now supersede the precise HTTPS Design §6 and §7 clauses that otherwise require every terminal non-success response to discard its connection. The exception is limited to a fully drained, clean anonymous discovery `401` or `404` entering the existing immediate once-only Gh transition. It grants neither Git success nor a broader error-body rule.

The disposition table at lines 64–70 closes the degraded-path ambiguity. A failed reuse check always discards the challenged lease. Section 4 alone determines whether the authenticated GET may still begin with remaining cumulative budgets; terminal cancellation, deadline or protocol failure preserves its own result and permits no new request. A disallowed transition, Gh failure, POST failure, other status or unvalidated redirect still discards.

Lines 74–91 correct the evidentiary boundary. The future fixture must record every request before authorization branching, assign identity when the physical connection is accepted, distinguish anonymous and Gh policy, and count helper invocations without credential contents. It must prove that each qualifying challenge and immediate Gh GET use the same connection even when another idle connection exists, while required-discard cases either use a different connection or issue no retry.

**New-root classification:** none. These changes clarify authority, disposition and causal proof within the original bounded exception.

## 0. Evidence base

Read the revised amendment and remediation plan, the complete amendment diff from core `51e28002` to `e416faa1`, the prior Safety report, and the retained HTTPS authentication, pooling, failure and timeout contracts used in round 1. No build, test, implementation inspection, file write or current peer report was used.

All five repository HEADs matched the specified tuple at start and end. The amendment hash matched at both boundaries. The listed unrelated root/core working changes remained excluded; all review evidence came from committed blobs.

## 2. Invariant analysis

Authentication scope remains closed: only an anonymous discovery GET receiving `401` or `404` may enter the once-only Gh transition, for the same canonical destination, request, operation and route.

No credential is sent preemptively. Each eligible operation starts anonymously, and every Gh request performs a fresh helper lookup. Authorization belongs only to that logical request; the pooled TLS connection stores no token or authenticated-account fact.

The amendment creates no network or body replay. A complete bounded drain qualifies physical reuse but is not Git success. Truncation, invalid framing, cap exhaustion, cancellation, deadline expiry, sender failure, peer closure or non-keep-alive disposition cannot return that lease to idle. POST and network failures remain non-retriable.

Budgets remain cumulative. Cleanup cannot replenish network, interaction or operation allowances, and a terminal cleanup failure cannot be replaced by the later authenticated result. A clean peer close after a complete challenge may continue only through the already-authorized §4 transition on a new lease and remaining budget.

No mixed-version surface changes: there is no Taut field, wire version, public API, credential source, placement or release-order amendment.

## 3. Risks and next action

This is a documentation re-verdict, not implementation acceptance. Real-account proof, product activation, release qualification and physical-wire work remain deferred.

Implementation must next demonstrate the causal authenticated-fixture red/green result and every adverse disposition case named in the amendment. Until those implementation gates pass, no claim of corrected socket reuse is established.
