## Prior-finding closure table

| Prior finding | Original counterexample | Corrected location | Status |
|---|---|---|---|
| P2-1 — blanket terminal-failure discard contradicted challenge-socket reuse | The amendment permitted reuse after a clean anonymous `401`/`404`, while controlling HTTPS Design §7 still required every terminal non-success response to discard its connection. | Amendment lines 44–69 explicitly supersede the relevant §6 and §7 sentences only for the immediate once-only §4 Gh transition. The exhaustive disposition table preserves discard for failed reuse checks, disallowed transitions, and every other terminal non-success. | **Closed** |
| P3-1 — fixture could not prove challenge-socket identity | The prior evidence recorded connection identity only after Authorization and checked aggregate connection counts, so another idle socket could satisfy the observation. | Amendment lines 76–90 require recording every request before authorization, assigning identity at TCP/TLS accept, and asserting that each qualifying anonymous challenge and its immediate Gh GET share the same physical ID, including with another idle connection available. The native Python path and adverse discard cases receive equivalent assertions. | **Closed** |

## Changed-range analysis

The correction is bounded to the amendment and its remediation record. It adds the missing §7 supersession, an exhaustive response/transition disposition table, and causal socket-identity requirements.

The table remains consistent with the controlling §4 transition: failure of a physical reuse check discards that lease, while §4 may still permit the Gh GET on another connection when cumulative budgets and cancellation state allow it. Terminal cancellation, deadline, or protocol failure permits no new request. The amendment continues to prohibit token caching, preemptive authentication, POST replay, redirect relaxation, and network-error retry.

The revised fixture is satisfiable and distinguishes reuse from pool coincidence. No new architectural root or unlisted controlling-document impact was found.

# GwzRemoteTransportGhChallengeReuseAmendment — CONSISTENCY-AXIS REVIEW 1

- **Review object:** `gwz-core` `e416faa1b23b578b5aa50eadea64a11c6561fdf9`, `dev-docs/GwzRemoteTransportGhChallengeReuseAmendment.md`
- **Baseline:** `gwz-core` `51e28002e3a9f78f5a8daa873e02b083e2b2341f`
- **Root tuple:** `78775e818b8a6398aad0859aa46a6662440ab95a`
- **Transport:** `14f0d09cbca7ab54c06d36e8354799981584a7f9`
- **CLI:** `1e52b274ed242d81349b6d18c107680152ecfecf`
- **Python:** `d484e367d1df47eff5caf3421864e69ec85744f5`
- **Document SHA-256:** `a9aba45fa0d19ce38ba0f9bef1fd37280a4612014555c0b2a75ad2f564fcafe9`
- **Date:** 2026-09-23
- **Axis:** Consistency
- **Verdict:** **GO**

## 0. Evidence base

I reviewed the committed revised amendment, its committed remediation plan, and the amendment diff from the baseline core commit. I retraced both original counterexamples against the corrected text and checked the changed clauses against the controlling HTTPS transition, pooling, budget, and persistent-session requirements.

The five repository HEADs and document hash matched the required tuple at both review boundaries. I used committed blobs only. I did not inspect the current Safety report, modify files, run builds, or execute tests.

## 1. Findings

No open findings.

Severity count: P0 0, P1 0, P2 0, P3 0.

## 2. Invariant analysis

The corrected amendment now defines one narrow exception to the existing physical-discard rule: a fully drained, bounded, correctly framed anonymous discovery `401`/`404` may release its lease for the immediate, policy-permitted, once-only Gh GET when sender readiness, keep-alive, destination, budget, and cancellation checks all pass.

Logical isolation remains intact. The challenge is not Git success; its typed failure remains private; the Gh request is a new logical stream; credentials are looked up afresh and appear only on that request. Failed qualification discards the lease. Other failed responses retain the controlling §7 disposition.

Budget and cancellation behavior is implementable as written because the amendment distinguishes physical reuse eligibility from §4 authority to start the next logical request. It does not reset cumulative budgets or convert terminal cancellation, deadline, or protocol failure into retry authority.

The revised regression proves the required causal relationship by observing both sides of the transition at physical accept identity, rather than inferring it from aggregate connection counts. The adverse cases also prove that mandatory discard cannot silently reuse the rejected socket.

## 3. Risks and next action

The remaining work is the explicitly deferred implementation and causal evidence: implement the bounded drain/readiness handoff, run the fixture and native Python path, and retain the real-account Gh proof for its later gate. This review does not certify product code, release readiness, or a physical wire protocol.

The corrected draft is internally consistent and provides sufficient disposition and evidence shape for implementation. **GO.**
