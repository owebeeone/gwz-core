# GwzTransportSshHelperClockAmendment — SAFETY-AXIS REVIEW

**Review object:** Complete corrected DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md:1–307`, at core `fde5878ac11b9e02892127f438cb50954551b9a3`, including changed range `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea..fde5878ac11b9e02892127f438cb50954551b9a3`. Remediation round 2; document closure only.

**Baseline:** root `0bb560c5f34ba5bde5ece053dc58e33ef10fd8e0`; core `fde5878ac11b9e02892127f438cb50954551b9a3`; transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`; evidence `662d89828b478a2acce8c0308834db7d17c872f7`. Inspection used committed `git show` and `git diff` reads. The tuple matched at both start and end. Unfinished implementation bytes remained excluded.

**Date:** 2026-10-03

**Axis:** Safety—attack transition arbitration, prepared-token refusal, expiry provenance, stall states, bounded ownership and cleanup. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current report. Filed verbatim by the lane owner.

**Verdict: GO** — all prior Safety P2 findings close at the document level; no open P0, P1, P2 or P3 findings. No NEW ARCHITECTURAL root cause was found. This verdict accepts the corrected mechanism text only.

---

## Prior-finding closure table

“Verified” means the original counterexample was re-traced against this exact corrected document. No implementation test was executed.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Safety P2-1 | Common publication/expiry/cancellation arbitration | Publication at 99 immediately installs LocalAdmission against Network expiry 100. Control observing at 101 sees the local phase before PoolHost acknowledgement. Publication at 100 settles expiry first. Lines 60–68 and 113–123 preserve the common lock and prohibit obsolete independent failure publication. | Closed — document |
| Safety P2-2 | Capture and restore stall remainder | A live discovery stall deadline is captured on departure and rebased before resumed checks. Local elapsed time is excluded; helper completion cannot reset stall. Lines 127–154 retain Disabled/Inactive/Live distinctions and remove authoritative stale Control copies. | Closed — document |
| Safety P2-3 | Complete prepared-token refusal while authority remains Alive | The 1-ms token survives observation as an identity-bound ExpiredPrepared record. Publication validates it and atomically commits PreparationDeadline, producing the original Allocation budget without work, renewal or a nonexistent-terminal wait. Active expiry/cancellation and invalid tokens have separate branches. Lines 77–82 and 184–207 defeat the original counterexample. | Closed — document |
| Consistency P2-1 | Common arbitration | Independently re-traced the original delayed-PoolHost/Control-first sequence; the publication and terminal ordering remain unchanged by round 2. | Original counterexample remains closed |
| Consistency P2-2 | Core witness and neutral terminal bridge | Admitted pool-first, Control-first and helper-first local expiry retain the exact connection/phase witness. Lines 212–240 preserve this path and add validated preparation refusal without letting rejected witnesses relabel Network or cancellation. | Original counterexample remains closed |
| Consistency P2-3 | Explicit stall pause/resume | Independently re-traced local work exceeding the original stall deadline; retained time is restored before the resumed check, and genuine progress alone resets it. | Original counterexample remains closed |
| Consistency P3-1 | Resume waiting described by live deadlines | With aggregate Disabled and configured stall Inactive, delayed resume acknowledgement supplies no manufactured stall deadline. Lines 94–97 preserve lifetime/cancellation-bound waiting; the subsequent real network wait starts the stall interval. | Closed — document |

## Changed-range analysis

The core range changes four documentation files: the amendment, RemPlan-2 and the two verbatim round-1 reports. It changes no source or controlling credential contract.

The complete amendment was reviewed, including these round-2 changes:

- **Lines 1–9:** Status and precedence now identify round 2 and retain the implementation prohibition pending root-relayed GO.
- **Lines 50–54:** The neutral terminal vocabulary adds `PreparationDeadline`.
- **Lines 77–82:** Expiration removes the work reservation but retains its exact issued identity, kind and deadline in the same bounded slot. Another preparation cannot reuse the slot while its token remains owned.
- **Lines 94–97:** Resume waiting is bounded by live deadlines, explicitly covering aggregate Disabled plus stall Inactive without starting a timer for acknowledgement.
- **Lines 181–207:** Publication refusal now distinguishes an existing active Terminal, validated preparation expiry while Alive, and invalid tokens.
- **Lines 220–227:** Core error capture explicitly includes the validated preparation outcome.
- **Lines 290–298:** Regression obligations include the original 1-ms counterexample, later enabled Network deadlines, Interaction preparation while Admission remains live, winning active expiry/cancellation, invalid tokens and delayed Inactive-stall resume.

All changes fall within RemPlan-2’s dispositions. The expired-preparation correction completes the existing token/refusal protocol; it introduces no new owner or synchronization architecture. The Inactive-stall change corrects a finite-bound claim without changing the state model.

**Architectural classification:** No **NEW ARCHITECTURAL root cause** was identified. The recorded architectural total remains two from the initial review. The third-cause stop is not triggered.

## 0. Evidence base

Read during this continuation:

- Complete canonical `PromptSafety-2.md`.
- Complete corrected amendment, lines 1–307, at the new core SHA.
- Complete committed amendment diff and changed-range summary.
- Complete committed `GwzTransportSshHelperClock-RemPlan-2.md`.
- Complete committed round-1 Consistency and Safety reports.
- Root `CurrentProgramCheckpoint.md:1–40`.
- Targeted committed diffs confirming that standing/process instructions, the five controlling credential/design documents and previously traced core clock/setup source owners did not change.

Retained evidence from the original review and round-1 continuation remains applicable:

- Workspace/member instructions, canonical review-loop skill and process authority.
- Initial RemPlan and both initial reports.
- TR1.6 bounds, admissions, secrets, retry rules and OQ6(a).
- Accepted helper timing and configuration-view amendments.
- Relevant GWZDesign/GWZRequirements ownership and transport requirements.
- Committed source traces through `ssh_pool`, `ssh_setup`, `agent_job`, `ssh_local`, `ssh_password`, `ssh_worker` and the transport pool clock, asynchronous, allocation and error owners.

Root/core/transport/evidence revision checks matched the required tuple twice. No files were written; no tests, builds, experiments, probes, helpers or network requests ran. No peer current-round prompt or report was inspected. Checkpoint references to implementation results were not used as closure evidence.

## 2. Invariant analysis

### Prepared-refusal counterexample

The original Safety P2-3 sequence now has a complete path:

1. Network aggregate and stall timing are Disabled.
2. Core captures a positive 1-ms Allocation allowance, prepares the transition and associates its witness.
3. Publication is delayed past the prepared deadline.
4. `observe` retains the exact issued token information as ExpiredPrepared in the same bounded slot. Network remains unpaused.
5. `publish_local` validates that issued token while the authority is still Alive.
6. Under the same lock, `PreparationExpired` consumes the slot and commits a new immutable Terminal with `PreparationDeadline`.
7. Core matches only this validated prepared identity and reports Timeout, Effect None, Allocation and the original 1-ms budget. It starts no admission work or child, creates no missing-Git latch and does not renew the allowance.

The outcome no longer requires a prior Terminal record. It creates the required record atomically at refusal.

The later-live-Network variant follows the same branch. Interaction preparation expiring before a still-live Admission phase likewise reports the original Interaction witness and releases already-owned admissions through the existing cleanup owner.

The controls remain distinct:

- If active expiry or cancellation has won, `ActiveTerminal` returns its immutable record and excludes rejected-preparation metadata.
- A stale or foreign token cannot validate arbitrary helper provenance or mutate a foreign connection. Core terminates only its own setup, reports the returned first cause and performs bounded disposal.
- Token drop and terminal invalidation retain bounded ownership; no subsequent preparation can be confused with an owned expired token’s generation.

### Preserved arbitration and clock guarantees

The round-2 additions do not weaken the earlier closures:

- Time is sampled inside one authority mutex. Active due deadlines settle before requested changes.
- Local publication changes the authoritative phase immediately; acknowledgement gates work.
- Exact-boundary publication and acknowledgement cannot revive expiry.
- First terminal cause is immutable. Later results cannot overwrite cancellation or timeout.
- Aggregate and live stall remainders are captured per departure and rebased before resumed checks. Intervening Network consumption is deducted on later departures.
- Helper work cannot manufacture network progress. Only a genuine network wait starts an Inactive stall, and genuine progress resets a live interval.
- Delayed resume acknowledgement consumes any live resumed network allowance and grants no extension.

### Ownership, provenance and scope

The complete mechanism retains these safeguards:

- One prepared/pending slot, one waiter registration and one driver registration bound transition storage.
- The declared Runtime → SetupClock → Control order remains unchanged; witness locks, callbacks, waits and external work are excluded from nested arbitration locks.
- Admitted local expiry retains full connection/phase identity through `SetupEnded`; core owns helper classification and exact captured `helper_budget_ms`.
- Generic Network timeout carries no helper detail. PreparationDeadline uses validated issued identity rather than inference from an arbitrary witness.
- Final pool connected admission checks and commits Completed through the authority; an advisory earlier observation cannot admit an expired result.
- Logical reporting and acknowledgements do not acknowledge physical disposal. Context, clock, permits and sensitive buffers survive unfinished setup/child/file-worker cleanup.
- Admission and interaction budgets remain independent and unchanged. Preparation/fill share the captured interaction deadline; zero allocation refuses without launch or latch.
- Installed shared paths have precise supersession. Non-shared Control and uninstalled pool users retain their contracts.
- Generic transport receives no secrets, configuration, helper policy or FailureDetail. Authentication choices, public application schemas and wire fields remain unchanged.

No further concrete defect emerged from the changed-range or full-document attack.

## 3. Risks and next action

This GO supplies no executable concurrency proof, implementation acceptance, platform qualification or release authority. Implementation must execute the declared arbitration, prepared-refusal, provenance, Inactive-stall and retained-cleanup regressions. Existing global-job debt and native/OS copy limitations remain explicitly unclosed.

The next action is for the lane owner to merge the independent document verdicts and relay authority to implement this exact mechanism only if all required reviewers pass. Final credential implementation acceptance remains separate.
