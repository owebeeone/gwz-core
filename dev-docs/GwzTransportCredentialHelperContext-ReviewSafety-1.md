# TR2.22 helper context amendment, remediation round 1 — SAFETY-AXIS REVIEW

**Review object:** Committed `gwz-core/dev-docs/GwzTransportCredentialHelperTimingAmendment.md` and caller note `GwzTransportCredentialHelpersSurface.md` at core `ec1b95831582651953bb0bd3be3c4f88985aba6b`. DRAFT, not implementation acceptance; dated 2026-10-03.

**Baseline:** root `745c37398ebe3da9037dffff4020cebd49bc7c89`; gwz-core `ec1b95831582651953bb0bd3be3c4f88985aba6b`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Corrected documents were read through `git show <pinned SHA>:<path>` and exact revision diffs. Accepted producer-source evidence remains core `26922a1cfd823a4894e09be9aeb05f7f73d21414` / transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Working source edits were excluded.

**Date:** 2026-10-03

**Axis:** Safety — degraded paths, truthful failure provenance, credential disclosure, account isolation, and bounds. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current-round report. Filed verbatim by the lane owner.

**Verdict: GO** — zero open P0, P1, P2 or P3 findings. Original Safety P2-1 is closed by the corrected contract and specified regression. No introduced defect was found.

---

## 0. Evidence base

This closure review read:

- The canonical `Safety-1.md` review prompt, including the exact tuple, deferrals and read-only restrictions.
- Committed `GwzTransportCredentialHelperContext-RemPlan.md`, lines 1–52. Its combined prior-round dispositions are legitimate remediation inputs; no current-round peer report was read.
- The complete corrected `GwzTransportCredentialHelperTimingAmendment.md`, lines 1–298, particularly:
  - timing validation and capture, lines 32–91;
  - complete replacement M10 and zero-allocation assignment, lines 93–109;
  - scope-aware recovery wording, lines 111–136;
  - unchanged username confinement, lines 149–195;
  - precise supersessions, lines 197–233;
  - affected owners and regressions, lines 235–298.
- The complete corrected `GwzTransportCredentialHelpersSurface.md`, lines 1–166, particularly M10 at lines 74–97 and configuration/recovery guidance at lines 114–166.
- Exact diffs from original draft core `0adb093a99f4eb8e9b8818b767514a842533a193` to corrected core `ec1b95831582651953bb0bd3be3c4f88985aba6b` for both reviewed documents.
- The root checkpoint diff from `bb44a7214eac779095352224e8940df268fd3f96` to `745c37398ebe3da9037dffff4020cebd49bc7c89`, including its settled-remediation status and excluded working implementation.

The controlling process/design and accepted-source evidence inspected during the original review remains the basis: `AgentProcessRules.md` L1-08/09 and L1-17–20; `GwzProcessOptimization.md`; TR1.6 revision 4; release-plan amendment 2 revision 6; accepted core budget, preparation, destination, route, carried-retry and driver sources; transport authored schema, failure-detail validation, all Failure-carrier validation and pool configuration bounds.

The mandated root/core/transport `rev-parse HEAD` commands matched the pinned tuple at both start and end.

No files were written, and no tests, builds or history mutations were run during this review. Private evidence, secret values and working implementation were not inspected. Regression rows below are contract requirements, not executed evidence.

## 2. Invariant analysis

**Original Safety P2-1 is closed.** Replaying the original counterexample—zero retained allocation with both helper semaphores free—now produces a truthful message. Amendment lines 93–109 replace the complete M10, qualify busy helpers as a possibility, expressly prohibit inferring occupancy, and assign the exhausted case to M10 without waiting, spawning or latching. Lines 214–218 explicitly supersede the old complete wording and unchanged-remainder restriction.

The caller heading now reads “A helper could not start in time.” Its explanation expressly states that earlier waits can leave zero seconds while slots are free and that M10 does not establish saturation (Surface lines 74–91). Regression row 5 requires the exact zero/free-slot case, truthful `0 seconds`, no helper, no latch and no saturation assertion, while preserving positive-allowance saturation and one shared deadline (amendment lines 267–271). The original pre-commit condition is satisfied.

**The rounding correction closes the old/new assertion conflict without enlarging a budget.** Lines 81–91 specify that a sub-millisecond retained allocation remainder expires at captured zero. Positive captured milliseconds drive both the timer and detail, and seconds render exactly. Lines 219–221 precisely supersede T15(b)’s rounded-down-seconds assertion. Regression row 2 applies the same rule to the existing and new rows. No fresh allocation allowance is introduced.

**Recovery no longer treats ordinary Git success as proof that GWZ’s helper was repaired.** The revised M8 and shared explanation distinguish native Git’s repository/conditional configuration from GWZ’s unconditional chain. The caller guidance identifies system/global/XDG and captured `GIT_CONFIG_*` sources, follows unconditional includes, preserves helper ordering and empty resets, and directs the reader to repair the actual helper A even when native Git succeeds through B. It also supplies restoration of prior entries/order and avoids printing credential-bearing configuration values. The supersession trail covers the affected recovery claims. These corrections do not authorize a new command, authentication policy or credential-store mutation by GWZ.

**The remaining timing protections hold.** Helper provenance remains a single bounded optional integer with phase selected by existing setup cause. Wrong code/effect/cause and forbidden combinations are rejected on every Failure carrier. Absent detail remains generic. Endpoint configuration supplies the proven 86,400,000 ms allocation ceiling, and Open/retained charging only shortens it. Helper admission still shares one allocation deadline; interaction begins after admission and retains its positive bounded allowance.

**Credential confinement and account selection remain intact in the contract.** The separate HTTPS username field, encoded carriage, password refusal, decoded-control checks, reconstructed URL bound, endpoint revalidation and redirect checks are unchanged. Request serialization removes userinfo. Diagnostic text, facts and Failure detail cannot receive the username. Generated and local Debug representations require redaction through a reproducible projection. Private account selection must remain isolated through route and carried-retry ownership.

**The fixed parser cause remains closed and non-disclosing.** The new enum alternative names an already-required refusal using fixed wording. It carries neither helper output nor credential content and does not relax parser acceptance.

**The revision stays within the reviewed amendment.** It resolves causal diagnosis, recovery claims and rounding precedence through bounded text changes. Public GWZ schemas, error-code allocation, clone outcomes, retry transitions and accepted authentication decisions remain outside its changes.

## 3. Risks and next action

Implementation must still demonstrate the specified timer/detail equality, zero/free-slot behavior, all-carrier validation, Debug redaction, carried-endpoint account isolation and configuration-scope recovery regressions. Concurrent selectors for the same host/path should exercise both route ownership and carried-retry selection. These are pending implementation obligations, not defects in the corrected draft.

The next action is to record this Safety closure with the other required amendment verdicts against the settled tuple. This GO accepts the reviewed draft’s Safety contract only; it does not accept runner implementation, Windows authentication, performance qualification or release readiness.
