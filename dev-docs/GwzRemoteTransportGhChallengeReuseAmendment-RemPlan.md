# Gh challenge reuse amendment — round 1 remediation

Status: draft remediation for the Consistency NO-GO at core
`51e28002e3a9f78f5a8daa873e02b083e2b2341f`; Safety reported GO on that
same revision. This is one document-only correction. Product HTTPS behavior
remains unchanged until the amended document receives the retained re-verdict.

| Finding | Disposition | Correction | Closure |
|---|---|---|---|
| Consistency P2-1 | Accept | Name both HTTPS Design §6's “no terminal failure” condition and §7's blanket non-success discard/error-body sentences as superseded for exactly a clean anonymous discovery 401/404 entering the existing once-only Gh transition. Add an exhaustive disposition table; retain all other discard and replay rules. | Retained Consistency reviewer checks every cited clause and table row on the amended committed document. |
| Consistency P3-1 | Accept | Label the archived trace as evidence of authenticated socket churn only. Require the causal fixture to log every request before the authorization branch, a physical connection ID, policy/status/order, and fresh gh invocation count. Assert each qualifying anonymous challenge and immediate Gh GET share that ID, including when other idle connections exist. | Retained Consistency reviewer verifies the revised regression specification; implementation gate later runs the specified red/green fixture and adverse cases. |
| Safety | GO, no findings | Preserve the unchanged no-token-cache, no-preemptive-auth, no-POST-replay, bounded drain, budget, cancellation, and failure-disposal constraints while clarifying authority and evidence. | Retained Safety reviewer rechecks the bounded document correction on the new tuple. |

No test run or implementation is claimed by this document correction. The
re-verdict object is the amended document and this disposition plan at a new
committed core SHA; round-1 reports remain verbatim.
