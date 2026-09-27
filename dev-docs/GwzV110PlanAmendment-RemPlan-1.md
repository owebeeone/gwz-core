# GWZ 1.1.0 plan amendment — remediation plan 1

Date: 2026-09-26. Status: **applied as the amendment's third draft; focused re-verdict pending**.

Input: [Verdict-1](GwzV110PlanAmendment-Verdict-1.md) on the second draft (SHA-256 `1606f967…`), merging [Consistency-1](GwzV110PlanAmendment-ReviewConsistency-1.md) and [Safety-1](GwzV110PlanAmendment-ReviewSafety-1.md). Both reported NO-GO on one shared root, B1, and both pre-committed to GO on its specified correction. Every ID carries its axis name.

| Finding | Disposition in the third draft | Closure test |
| --- | --- | --- |
| B1 (Consistency P2-3, Safety P2-2) | §3.5 amends S7.1: the switch is also removed from gwz-py's three sites, from S6.1's variant and S6.2's arms, from both crates' `check-cfg` declarations, and from `prepare.py`. After S7.1, `rg gwz_transport_candidate` over gwz-core and gwz-py finds nothing, and the native branch stays for unsupported paths. §3.5 amends S7.3: on each platform, one CLI and two overlapping Python network operations assert the transport route before S7.5, as the pre-publish proof. §3.6's step 7 names S7.3 as that proof. §3.4 and §4 are updated to match. | Left in place, gwz-py's gate fails S7.3's assertion before any tag. Removed, the `rg` finds nothing and S7.3 passes. |
| Consistency P3-8, Safety P3-9 | S1.1 no longer lists environment stability as surviving. A new "What changes in documented behaviour" bullet states capture at each operation's start, the process-wide `configure_transport_runtime`, and the process-wide helper caps. S1.2 now includes Surface for that change. S6.3 adds a capture row, and S7.2's notes state the capture point. | S1.1, §6, S7.2 and the S6.3 row name the same capture point. |
| Consistency P3-6 | §3.5's S7.2 text names S6.3's and S7.3's route assertions as the evidence for the Python rows, in place of an S5.6 row, since S5.6's matrix has no Python rows. | One evidence source for each class of ledger row. |
| Consistency P3-7 | §3.9 adds the sketch edge `S4.5 ── S6.3` and the prose "S6.3 also waits on S4.5". S6.3 says its dabeest rows wait on S4.5. The re-run rule now names `with_local_transport` for S6.1 landing second, and S6.3's dabeest rows as the guard for S6.2. | Every platform row of S6.3 has its enabling step as a sketch predecessor. |
| Safety P3-8 | S6.3 adds rows for wrong, foreign or completed cancel IDs, the ninth operation waiting under the 8-bound, and close and interpreter exit with an operation running. S1.1 states whether a waiting operation holds a native thread, and what close, exit and cancel do with several operations running. | The rows exist in S6.3. |
| Safety P3-10 | S6.1 gains the library-safety rule: never finish from `Drop` while unwinding; catch the operation's panic first; guard finish and shutdown; report failure with cleanup unconfirmed. It adds the fault-injected double-panic unit test. | That test in S6.1. |
| Residuals | §6 says the authority for S6.1 and S6.2 is the amended plan and S1.1's revision, and that later changes to the contract's §5.2 do not amend S6.1. S6.3 is labelled gwz-py-level. | — |

Re-verdict: the same two reviewers, focused on B1 and their own P3 findings, on the third draft's hash. Reports go to `-ReviewConsistency-2.md` and `-ReviewSafety-2.md`, merged into `-Verdict-2.md`.
