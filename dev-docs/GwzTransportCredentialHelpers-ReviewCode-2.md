# Credential implementation remediation round 2 — CODE-AXIS REVIEW

**Review object:** Bounded final helper-admission correction in `/Volumes/projects/limbo/gwz-dev-tr2-22`, frozen on 2026-10-03. Controlling DRAFT: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan-2.md` at core `ec43f585f7c768afa0fe71e23a7e503cfda9150c`. Source acceptance remains pending.

**Baseline:**

| Repository | Reviewed HEAD |
| --- | --- |
| root | `b35887e579cbf8d42e42ae848038f408f51ddecb` |
| gwz-core | `ec43f585f7c768afa0fe71e23a7e503cfda9150c` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

The changed range is core `64ec039089b6e217625d7784b106d917452963cb..ec43f585f7c768afa0fe71e23a7e503cfda9150c`. Committed bytes were read with `git show HEAD:`, range diffs and numbered source inspection.

**Date:** 2026-10-03

**Axis:** Code — interface contracts, call graphs, ownership, compatibility and error paths. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on their current reports. Filed verbatim by the lane owner.

**Verdict: GO** — zero open P0, P1, P2 or P3 findings on this axis. The changed runner preserves the preceding Code GO while correcting final refusal after cleanup-capability retirement. No new architectural root cause was identified.

---

## 0. Evidence base

I read the complete canonical `PromptCode-2.txt`, newest credential checkpoint, controlling RemPlan-2 and my preceding Code report. Standing instructions and unchanged adopted design/process authority remain those inspected in round 1. No current-round peer report was read.

Fresh inspection covered:

- Complete production diff and committed `https_auth/runner.rs`, especially clock sampling at lines 20–32 and finalization at lines 152–200.
- Complete added regressions in existing `runner/tests.rs:145–294`.
- Unchanged `HelperJob` ownership, completion, termination and Drop paths in `https_auth/owner.rs:179–263`.
- RemediationRecord’s round-2 evidence section.
- Source-diff inventory, confirming no protocol, dependency or other production-source change.
- Final source receipt, recorded RED/GREEN logs and unchanged boundary-guard receipt.

The receipt SHA-256 matched:

`3d8d7ef0b898d5139a9529cf4a60410070423288c69741362b6cc5b7883084a1`

All three owned-path hashes matched committed bytes. All eight listed log hashes matched; the RED source receipt and working boundary-guard hashes also matched.

Recorded evidence contains:

- Original-behavior RED: **0 passed, 3 failed**, including continuing independent descendant writes after deadline/cancellation refusal.
- Corrected focused suite: **17 passed, 0 failed**, including all three new boundary regressions.
- Corrected affected union: **458 passed, 0 failed, 4 existing ignores**.
- Core Clippy: exit 0 with **49 retained warnings**.
- Conditional-scope, candidate and process-global guards: exit 0.
- Working boundary guard: 24 visible entries, nine classified modules, passing.

The receipt’s predecessor HEAD identifies when evidence was captured; committed source hashes bind its three owned paths to the settled revision. Historical whole suites were not promoted to final integration proof. The focused command’s nonmatching ownership selector was disclosed; its actual 17 runner/helper rows and the broader affected union were inspected.

All six HEADs matched at both review boundaries. No tracked modifications appeared; excluded untracked drafts/evidence remained unchanged. No builds, tests, probes, writes or Git mutations were performed.

## 2. Invariant analysis

**Refusal precedes retirement.** `run_finished` still checks completed output before finishing/parsing. It then invokes the private `finish_job` with the unchanged successful-result admission check. That callback executes before `complete_if_exited` or `terminate`, while HelperJob retains its child, process-group identity and permit reference.

If final admission refuses, `terminate` signals the retained process group before waiting/reaping and retiring ownership. This includes an already-exited leader with a live descendant that closed inherited pipes. There is no post-retirement check capable of manufacturing an ownerless refusal.

**Accepted result delivery.** Once final admission succeeds, the finalizer retires completed ownership and returns that admitted value. Later cancellation does not retroactively convert this result into refusal. This is an explicit admission boundary within the accepted correction; it adds no timeout allowance or retry.

**Error and cancellation ownership.** Existing error results remain errors and enter termination. A cleanup miss continues to return CleanupPending, with remaining child/permit ownership governed by the unchanged HelperJob Drop path. Dropping the future while termination awaits still drops the caller-owned Job through that path. Output and parsed-secret values retain their existing wiping owners. The finalizer does not clone credentials or introduce an unowned result path.

**Clock sampling.** Production `check()` supplies `Instant::now` to the private seam. Sampling remains after setup and cancellation checks, at the existing comparison position. Equality still refuses. The seam introduces no stored clock, public API, shared arbitration change or caller-selected production timestamp.

**Regression credibility.** The boundary fixture waits for a successful leader and independently writing descendant, then forces exact deadline equality or cancellation while inspecting charged admissions and the Job permit reference. Its refusal cases assert that no answer is admitted and that independent writes cease. The fixture’s group guard runs after these assertions, so GREEN heartbeat cessation is not produced by that guard. The success case verifies admission survives subsequent cancellation without a new refusal. RED records the original contrary behavior.

The correction leaves parser rules, preparation-error mapping, configuration scope, shared SetupClock publication, selected-key/helper policy, typed carriers and wire compatibility unchanged.

## 3. Risks and next action

Unchanged round-1 proofs are reused. This review does not establish Windows, provider/trust, performance, packaging, selected-source, release, supplied-carrier/iroh or session outcomes. Existing warnings and ignored campaigns remain unwaived. Descendant heartbeat cessation is not a claim of native grandchild reaping or OS-copy zeroization.

The next action is lane-owner acceptance after required closure GO, followed by the authorized GWZ merge and fresh combined MAIN CLI/core/Python validation.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
| --- | --- | --- | --- |
| Original Code P2-1 / State P2-1 | Exact detailed terminal association | Publication/atomic-clock sources unchanged; round-1 independent closure proof retained | Closed |
| Original Code P2-2 | Negotiate survives bounded projection | Projection and typed/private-clone consumers unchanged | Closed |
| Original State P2-2 | Username representation sized before secret copy | Constructor/conversion/header sources unchanged | Closed |
| Original State P2-3 | Reject late successful helper results | Completed-output and before/after parsing checks remain; final check now runs before retirement | Closed; affected proof refreshed |
| Original State P2-4 | Private helper enablement and isolated reuse | Propagation, pool partition and authentication-selection sources unchanged | Closed |
| Original State P2-5 | Retain Closed Failure and separate facts | Transport retention and RPC derivation sources unchanged | Closed |
| Prior Surface P3-1 / P3-2 | Correct persistent recipes and timing help | Python/CLI sources unchanged; prior surface evidence retained | Unaffected |
| Round-1 State P2-1, as specified by RemPlan-2 | Final refusal retains cleanup capability | Original post-retirement sequence retraced: finalizer admits/refuses before retirement; refusal terminates retained group; boundary regressions record RED then GREEN | Closed on Code analysis |

## Changed-range analysis

Only `runner.rs` changes production behavior: it factors private clock sampling and replaces post-retirement admission with one finalizer whose admission callback precedes retirement. The existing runner test file gains three boundary regressions; the remediation record gains corresponding evidence. Other range changes file prior reports and the controlling plan.

There is no new source-loading edge, library boundary, public/shared interface, protocol field, dependency, timeout policy or helper-selection policy. Existing braced control and enclosing conditional scopes remain intact in the changed code.

The correction is within RemPlan-2’s disposition. **NEW ARCHITECTURAL root causes: zero.** No new correctness defect was established by the changed-range attacks.
