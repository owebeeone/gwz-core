# Credential helper implementation — remediation round 2

2026-10-03. Status: **NO-GO**, one new non-architectural State P2-1 at core
`64ec039089b6e217625d7784b106d917452963cb`. Round1 original Code/State
counterexamples are closed; fresh Code and Surface report GO. Fresh State
reports NO-GO solely for final refusal after cleanup ownership retirement.
The exact round1 tuple and complete reports accompany this plan.
This is round2 for the same implementation object, not a new design object.
Zero reviewer-classified new architectural causes have been found.

## One correction and closure

| Finding | Disposition | Required closure regression |
| --- | --- | --- |
| Round1 State P2-1 | Establish one explicit final success admission while HelperJob still owns group, child and permits. Every refusal at that boundary terminates the group before retirement. No check after successful cleanup may manufacture an ownerless refusal. Preserve deadline equality, cancellation, parsing checks, wiping and retained cleanup. | Deterministic boundary seam with completed leader and live same-group descendant whose inherited pipes are closed and independent heartbeat continues. Force deadline equality and cancellation; reject answer/Authorization, stop group and independent writes within bound, retain admissions until cleanup completes, never falsely acknowledge cleanup. Keep normal successful completion coverage. |

One drafter, one bounded patch, no Git operations. Read filed State report in
full; it supplies the exact source locations and credible sequence. Record
original RED before production correction, then GREEN. Refresh focused runner,
helper ownership and affected credential union, source guards and necessary
Clippy; no changes to existing timeouts/assertions to obtain green. Root owns
filing, settlement and exact per-core-commit gate. Build/test logs stay external,
normal Rust1.95 debug/incremental profiles and retained caches. Broader existing
warnings and earlier full suites remain honestly labeled. No protocol, public
API, platform assumption, new policy or unrelated refactoring is authorized.
If the accepted mechanism cannot satisfy the correction, report before expanding.

Re-verdict continues the same fresh State reviewer who raised this finding;
Code can inspect only the changed runner proof if needed. Original closed
findings and Surface corrections stay closed unless affected by this patch.
Any material shared-interface/architecture change requires fresh proof under
review-loop section5.5. Implementer cannot self-close the finding.

On closure GO, file the exact accepted tuple and original six plus new one
settled-review defects, then perform already-authorized GWZ member merge and
fresh combined MAIN CLI/core/Python validation. Preserve excluded drafts and
retained Python stash. Windows, platform/performance/selected-source acceptance,
push/tag/publication/activation and alpha installation remain outside this work.
