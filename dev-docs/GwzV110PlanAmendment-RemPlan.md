# GWZ 1.1.0 plan amendment — remediation plan

Date: 2026-09-26. Status: **applied as the amendment's second draft; focused re-verdict pending**.

Input: [the verdict](GwzV110PlanAmendment-Verdict.md) on the first draft (SHA-256 `774bb164…`), merging [Consistency](GwzV110PlanAmendment-ReviewConsistency.md) and [Safety](GwzV110PlanAmendment-ReviewSafety.md). Both axes reported NO-GO, with blocking roots A1–A3. Every finding ID carries its axis name.

## 1. Operator decision

The first draft made the whole of 1.1.0 wait for the core session program. The verdict asked the operator whether it should. The options offered were:
- (a) Python ships on the full session host in 1.1.0;
- (b) a CLI-only 1.1.0.

The operator chose a third option on 2026-09-26: **1.1.0 backs out gwz-py's long-lived transport session, so gwz-py reaches the transport through the entry gwz-cli uses, one runtime per operation, keeping gwz-py's public API.** The rest of the contract moves to its own plan, [GwzCoreSessionPlan.md](../../dev-docs/GwzCoreSessionPlan.md), for a later release.

The second draft applies that decision, and every finding below, as one patch.

## 2. Dispositions

| Finding | Disposition in the second draft | Closure test |
| --- | --- | --- |
| A1 (Consistency P2-1, Safety P3-5) | §1 controls `GwzPyTransportDesign.md`: its 2026-09-23 acceptance, its unapplied bounded amendment, and its NO-GO. §2 items 1–2 state that history. S1.1 revises that same document to the per-operation model, so the plan keeps one Phase 1 object. §3.8 names the NO-GO's closing condition. §5 adds the design's status-only edit on GO. | One Phase 1 object and one NO-GO closing condition, readable from the plan and the gwz-py design alone. |
| A2 (Consistency P2-2) | Dissolved by the decision. Phase 6 does not change gwz-cli: gwz-cli's dispatch stays where it is, and S6.1 adds a variant beside `with_local_transport` without changing it. The dispatch move is in §3.7's out-of-scope list. §5's claim is corrected: gwz-core gains one additive crate function. | `rg "execute_invocation"` finds no Phase 6 text that moves it. |
| A3 (Safety P2-1) | §3.4: every new transport arm is written for Unix and Windows, matching S4.5, and whichever lands second re-runs S4.5's dabeest fixtures. S6.3 runs on all three platforms and asserts the transport route through the result's transport observations. §3.5 adds S7.2's Python ledger rows and maps the exit row to S6.3 too. §3.6 adds route assertions to Phase 8's post-release check. | A Windows build with the Python arm removed fails S6.3 instead of passing natively. |
| Consistency P3-1 | §3.1 supersedes line 50's "one pool" and says "local placement for Python". | No in-release cell promises cross-operation pooling. |
| Consistency P3-2, Safety P3-1 | S1.2 closes on a filed verdict naming the revised design's gwz-py commit with both axes GO. A later revision needs its own GO. Remediation follows GwzProcessOptimization §4. There is no floating reference to the contract's revision. | The closing revision is identifiable from the verdict alone. |
| Consistency P3-3 | The contract's §14 test is quoted verbatim. | Identical string in both documents. |
| Consistency P3-4 | Each Phase 6 step names its repository and files. The contract's §15 rows are no longer adopted wholesale; they belong to the session plan. | Every S6.3 test's owner matches where it lives. |
| Consistency P3-5, Safety P3-2 | Dissolved. Each Phase 6 step is within 500 lines, not counting removed code, so no separate implementation plan gates Phase 6. The session plan has its own review. | The sketch is unchanged and complete. |
| Safety P3-3 | §3.1 extends the redaction rule to Phase 6. §3.4 says path-pinned runs are development evidence, and the release proof is Phase 8's post-release check. | Filed S6.3 evidence passes the Phase 4, 5 and 8 secret scan. |
| Safety P3-4 | S1.1 bounds each `Client` to 8 running network operations. S6.3 records construction cost and connection counts for 1, 2 and 8. S7.2's notes state the per-operation model and the bound. | S6.3's recorded counts, and S7.2's notes carrying the statement. |
| Safety P3-6 | S6.3 tests the accepted default clocks without `configure_transport_runtime`, and runs S3.3's stall regression through gwz-py's path. | Both tests in S6.3. |
| Safety residuals | Line 75 gains "Before S7.1". Python's placement is stated as local. | — |

The residual about the size of the S6.1 milestone no longer applies. The contract's example host binary is not in 1.1.0.

## 3. Re-verdict

The same two reviewers continue with this plan, the second draft and its hash:
- Consistency re-checks A1, A2 and its five P3 findings.
- Safety re-checks A3 and its six P3 findings.

Both check that the operator's decision is stated consistently. The decision's outcome is deferred; its shape is in scope. Reports go to `GwzV110PlanAmendment-ReviewConsistency-1.md` and `-ReviewSafety-1.md`, merged into `-Verdict-1.md`.
