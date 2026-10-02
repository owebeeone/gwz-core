# TR2.22 helper context amendment, remediation round 1 — CONSISTENCY-AXIS REVIEW

**Review object:** Corrected committed DRAFT `gwz-core/dev-docs/GwzTransportCredentialHelperTimingAmendment.md` and caller note `GwzTransportCredentialHelpersSurface.md`, at core `ec1b95831582651953bb0bd3be3c4f88985aba6b`, dated 2026-10-03. This is the original Consistency reviewer’s closure round, not implementation acceptance.

**Baseline:** Root `745c37398ebe3da9037dffff4020cebd49bc7c89`; gwz-core `ec1b95831582651953bb0bd3be3c4f88985aba6b`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Documents were read through `git show` at these commits and compared with the original reviewed core `0adb093a99f4eb8e9b8818b767514a842533a193`. Accepted producer-source baseline remains core `26922a1cfd823a4894e09be9aeb05f7f73d21414`.

**Date:** 2026-10-03

**Axis:** Consistency with controlling contracts, exact supersessions, producer semantics and satisfiability of the stated regressions. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — original P2-1 and P3-1 are closed. No new P0, P1, P2 or P3 findings. The prior conditional pre-commit to GO is fulfilled.

---

## 0. Evidence base

Read in full at the corrected core commit:

- Helper context amendment, lines 1–298.
- Caller Surface note, lines 1–166.
- Combined `GwzTransportCredentialHelperContext-RemPlan.md`, lines 1–52.

Read the exact original-to-corrected diff of the amendment and caller note. Read the settled root checkpoint, lines 1–49.

Rechecked the controlling TR1.6 revision 4 clauses at corrected core:

- Configuration scope and helper admission, lines 73–87.
- Outcome assignment, lines 98–118.
- Parser and contingency clauses, lines 208–220.
- Queue regression T15(b), line 275.
- M4, M8, M10 and affected caller-page requirements, lines 323–338.

Exact revision diffs showed no changes to `GwzTransportCredentialHelpersDesign.md`, `GwzTransportReleasePlanAmendment-2.md` or `GwzRemoteTransportHttpsDesign.md` between the original and corrected core commits. The controlling graph and accepted-source analysis from the original review therefore remain applicable.

Root revision diffs likewise showed no changes to `AGENTS_GWZ.md`, `AgentProcessRules.md` or `GwzProcessOptimization.md`. The corrected checkpoint records the document-only remediation and excludes working runner changes.

Re-read accepted `session/driver/opening.rs:139–151`, confirming its existing treatment of sub-millisecond allocation remainders as expired.

The prescribed root/core/transport `git rev-parse` checks matched the pinned tuple at both review start and review end. No tests, builds, writes, history mutations, secret inspection or private evidence inspection were performed. No current-round peer report was read; the combined prior-round remediation plan was the only cross-axis input.

## 2. Invariant analysis

### Original Consistency P2-1 — closed

The original counterexample remains possible: helper admission can receive zero retained allocation while both helper semaphores have capacity.

The corrected contract now handles that state without inventing slot contention:

- Amendment lines 93–109 replace the complete M10 message, including its formerly preserved remainder.
- The message describes busy helpers as a possible cause.
- Lines 104–109 explicitly cover zero allowance with all slots free, no wait, no spawn and no latch.
- Exact supersession item 4, lines 212–218, expressly replaces the old remainder and specifies the zero-allocation M10 assignment.
- Regression row 5, lines 267–271, requires the original zero/free-slots counterexample alongside positive saturation.
- The caller heading and explanation, lines 74–97, agree with this assignment and explain that earlier waits can leave zero seconds even when helper slots are free.

Thus `Timeout` + `Allocation` + `helper_budget_ms = 0` no longer produces the unsupported assertion that eight helpers were busy. The correction follows the permitted neutral-message remedy without another wire field.

### Original Consistency P3-1 — closed

The controlling T15(b) text still exists in the older accepted design, but its changed assertion now has an explicit amendment boundary:

- Amendment lines 81–91 distinguish capture in integer milliseconds from exact rendering in seconds.
- Exact supersession item 5, lines 219–221, names T15(b)’s rounded-down `D−W` assertion and replaces it.
- Regression row 2, lines 258–261, applies the same expectation to T15(b) and the new retained-budget row.
- A captured 1,250 ms drives a 1,250 ms timer and renders as `1.25`.
- A retained allocation below one millisecond captures zero and expires without starting a wait or helper, extending the deadline or setting a latch.

The timer, detail and rendering assertions are now mutually satisfiable. This also matches the accepted driver’s conservative treatment of sub-millisecond allocation remainders.

### Introduced-change attacks

The broader recovery correction did not introduce a consistency defect:

- The complete revised M8 text in the amendment and caller note agrees.
- M4’s fixed message remains unchanged; its common explanation qualifies ordinary Git recovery.
- The revised guidance distinguishes repository-local and conditional helper selection from GWZ’s accepted system/global/XDG/environment scope.
- It preserves unconditional includes, helper ordering and empty resets, and supplies paired undo.
- The A/B examples cover a working native helper masking a broken helper in GWZ’s unconditional chain.
- Supersession item 6 and regression row 10 identify the changed recovery claims and their counterexamples without changing lookup configuration policy.

The unchanged wire and ownership invariants from the original review still hold at document level: distinct Destination tag 6, bounded FailureDetail tag 5, fixed parser cause value 8, preserved existing tags, validation on every Failure carrier, no inferred helper provenance when detail is absent, unchanged public GWZ schemas, and required request/diagnostic username isolation.

The 86,400,000 ms allocation ceiling remains tied to the validated endpoint configuration rather than the driver default. The fixed helper interaction cap and caller shortening remain separate from network deadlines. The remediation introduces no allocation reset, retry transition or new free-form diagnostic.

## 3. Risks and next action

This GO closes the Consistency review of the corrected draft only. Producer behavior, generated Debug redaction, account isolation through local/carried routing, and the stated regressions remain implementation obligations. No Windows, release, performance or campaign qualification is established.

The previously noted imprecise attribution of the remote-username supersession to TR1.6 §9.1 remains below the finding bar; the accepted OQ1(b) decision supplies the controlling policy. This report does not infer permission to relax separate redirect-Location restrictions.

The next action is coordinator settlement of this re-verdict with the other required review closures, then implementation behind the accepted amendment boundary. The final tuple check matched the pinned tuple exactly.
