# GwzTransportSshHelperClockAmendment.md — CONSISTENCY-AXIS REVIEW

**Review object:** Complete corrected DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md:1–268` at core `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea`, including changed range `eb06fac24..d4c1f90f`. Remediation round 1; document closure only, dated 2026-10-03.

**Baseline:**

| Repository | Exact commit |
|---|---|
| root | `f2db0e3b8e94f5aab7ecc0fd222b66a84637aeb7` |
| gwz-core | `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea` |
| gwz-transport | `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Documents and source were inspected through committed `git show` and `git diff` reads. The complete tuple matched at both start and end. Working-tree implementation bytes were excluded.

**Date:** 2026-10-03

**Axis:** Internal coherence, agreement with controlling contracts, precise supersession, and satisfiability of the complete changed mechanism and regression obligations. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current-round work. Filed verbatim by the lane owner.

**Verdict: GO** — all three prior Consistency P2 findings close at the document level; no open P0, P1 or P2 findings. One new nonblocking P3 wording defect remains. No new architectural root cause was found.

---

## Prior-finding closure table

“Verified” below means the original counterexample was re-traced against the corrected committed document. It does not mean an implementation test ran.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Consistency P2-1 | Common linearization for transitions and every expiry consumer | Publication before Network expiry immediately changes the authoritative phase under one lock. Control’s later observation sees the local deadline even before PoolHost acknowledgement. Publication at expiry first commits Terminal. Cancellation and acknowledgement use the same arbitration. Lines 58–116 defeat the original delayed-PoolHost/Control-first sequence. | Closed — document |
| Consistency P2-2 | Core timing witness plus neutral identity-bearing terminal bridge | Pool-first expiry now carries full connection/phase identity through `SetupEnded`; core capture receives the retained SetupContext and resolves the matching captured allowance. Control-first and helper-first outcomes use the same terminal record. Zero admission and generic network timeout are explicitly distinguished. Lines 149–208 defeat the original detail-dropping path. | Closed — document |
| Consistency P2-3 | Explicit aggregate/stall pause and resume | A live stall deadline becomes a retained remainder on local entry and is rebased before the resumed Control check. Helper completion cannot reset it; old `wait_started` copies are removed or unused on the shared path. Lines 118–147 defeat the immediate post-helper stall failure. | Closed — document |
| Safety P2-1 | Common arbitration replacing PoolHost-only ordering | Independently re-traced the prior report’s Network100/publication99/Control101 sequence. Lines 60–66 make publication authoritative immediately, while lines 76–104 prevent expired/cancelled acknowledgement from launching work. | Closed — document |
| Safety P2-2 | Defined restoration of the retained stall state | Independently re-traced the prior report’s local interval exceeding the original live stall deadline. Lines 123–146 exclude local elapsed time, restore the remaining duration and preserve genuine network-progress resets. | Closed — document |

## Changed-range analysis

The core range contains four documentation changes: the complete amendment rewrite, the merged remediation plan, and the two verbatim initial reports. It contains no source implementation change. The controlling helper design, timing amendment, configuration-view amendment, GWZDesign/GWZRequirements and traced source owners are unchanged in this range.

The amendment expands from 119 to 268 lines. I reviewed the entire corrected mechanism:

- **Lines 17–54:** A single mutex, full ConnectionId, checked PhaseId, one timestamp source, bounded receipt state, neutral terminal vocabulary and installed/uninstalled compatibility replace the former shared-deadline sketch.
- **Lines 58–116:** Immediate publication, separate preparation and acknowledgement, first-terminal arbitration, bounded registrations, driver-loss settlement, lock order and atomic final admission replace PoolHost-only chronological processing.
- **Lines 120–147:** Aggregate and stall states now distinguish Disabled, Inactive and Live; each departure captures the then-current remainder. This also resolves the initial report’s residual concern about reusing the first departure’s remainder.
- **Lines 151–208:** A bounded core SetupContext, pre-publication witness association, neutral `SetupEnded` bridge and retained reporting/cleanup owners replace the formerly unspecified helper-provenance path.
- **Lines 212–241:** Authentication boundaries remain confined to the settled route. Supersession now names both clock authority and the pool/SSH worker error-conversion seams.
- **Lines 245–268:** Regression obligations cover the actual independent Control path, exact boundaries, prepared-token association races, all expiry winners, stall states, post-result races and retained cleanup. They remain explicitly unexecuted obligations.

These changes fall within the accepted dispositions. The new context/bridge and authority synchronization address the two existing architectural causes; they are not unrelated scope expansion. I found no third **NEW ARCHITECTURAL root cause**. The P3 below is a bounded inconsistency in the acknowledgement-bound wording.

## 0. Evidence base

Read or re-read during this continuation:

- Canonical round-1 Consistency prompt.
- Complete corrected amendment, lines 1–268, and its committed diff.
- Complete committed remediation plan and both initial reports. No current-round peer report or prompt was inspected.
- Root CurrentProgramCheckpoint’s opening corrected-draft and remediation sections.
- Accepted timing amendment, particularly lines 38–97: helper timeout provenance, exact captured milliseconds, zero/sub-millisecond admission and independent interaction allowance.
- Accepted configuration-view amendment, particularly lines 166–221: one preparation/fill deadline, sequential children, retained worker/child permits and cleanup.
- Committed source:
  - `agent_job.rs:119–168`: existing aggregate/stall calculations and progress resets.
  - `ssh_worker.rs:200–263`: existing pool-error capture and absent detail.
  - Transport `pool/allocation.rs:136–185`: current connection admission and scalar failure conversion.

The original review’s instruction, process, controlling-document and source-owner reads remain part of this continuation’s evidence base. A targeted committed diff confirmed that the controlling documents and core owner files used for those traces did not change. Transport and evidence commits also remained unchanged.

Read-only revision checks verified the corrected root/core/transport/evidence tuple twice. No files were written. No builds, tests, experiments, helper invocations or network requests ran. Root checkpoint references to implementation test results were not used as closure evidence.

## 1. Findings

### [P3-1] Resume-wait bounds omit the enabled-but-inactive stall state

**Classification:** Bounded wording/contract consistency defect; **not a new architectural root cause**.

**Location:** Amendment lines 86–90 versus lines 120–135 and 140–144.

**Violated invariant:** Disabled, Inactive and Live clock states must retain their defined meanings without requiring a timer to be manufactured merely to satisfy acknowledgement waiting.

Lines 88–90 say a pending Network resume has a finite aggregate/stall deadline “when enabled,” and describe lifetime-bound waiting only “when both are disabled.” But enabled stall timing can be Inactive and therefore supply no deadline. Lines 122–123 explicitly define that state, and lines 133–135 require it to remain Inactive on resume.

**State sequence:**

1. Aggregate network timing is Disabled.
2. Stall timing is configured positively but remains Inactive because no network wait has begun.
3. A bounded local phase completes.
4. Network resume is published while PoolHost acknowledgement is delayed.
5. There is no live aggregate or stall deadline. Preserving Inactive contradicts the stated finite-deadline guarantee for an enabled timer; creating a stall deadline would contradict the preserved-state rule.

**Impact:** Implementers and test authors receive conflicting acknowledgement expectations for this supported state combination. One interpretation invents stall timing; the other exceeds the paragraph’s stated finite-bound promise.

**Required correction:** Describe the finite wait bound in terms of **live deadlines**, rather than configured enablement. Explicitly include Disabled aggregate plus Inactive stall in cancellation/driver-lifetime-bound resume waiting. Do not start a stall timer solely for acknowledgement.

**Closure/regression test:** With aggregate Disabled and configured stall Inactive, publish Network resume and delay acknowledgement beyond the configured stall interval. The phase must remain alive and Inactive until acknowledgement, cancellation or driver loss; the first subsequent real network wait must establish its normal stall deadline.

## 2. Invariant analysis

The full changed-mechanism attack established the following document-level properties:

- **Chronological arbitration:** Every authority operation samples time under the common lock and settles due expiry before applying a change. Timely publication immediately displaces Network; preparation alone provides no pause. Advisory snapshots cannot independently authorize irreversible outcomes.
- **Exact boundaries and first cause:** Equality expires before publication or acknowledgement. Cancellation arriving after due expiry preserves the timeout; cancellation committed earlier cannot be replaced by success. This is consistent with the existing Control ordering.
- **Prepared-witness race:** Core associates the witness before publication. Refused publication projects the prior terminal record and ignores the unadmitted witness. A pool-first observer therefore cannot encounter an admitted local phase awaiting later witness association.
- **Bounded storage and lock order:** One prepared/pending slot, one waiter registration and one driver registration prevent receipt-queue growth. Runtime → SetupClock → Control ordering and the prohibition on nested witness locks avoid the original consumer-order ambiguity. Wake delivery and effectful work occur outside locks.
- **Stall and aggregate preservation:** Local time consumes neither network clock. Resume acknowledgement consumes already-resumed network time and grants no extension. Repeated departures capture intervening network consumption. Actual network progress alone resets stall.
- **Typed reporting:** Neutral full connection/phase identity crosses the same-build pool seam; helper policy and FailureDetail stay in core. Pool-first, Control-first and helper-first expiry have a defined path to the same captured allowance. Generic network failures retain absent helper detail.
- **Allowance agreement:** Allocation provenance is captured before either admission wait. Interaction is captured after both permits, using the accepted minimum, and covers configuration preparation and fill without resetting. Zero/sub-millisecond admission refuses without launch or latch.
- **Post-result admission:** Final pool admission checks and commits Completed under the authority lock, closing expiry between NativeResource observation and pool acceptance.
- **Physical lifetime:** Logical reporting, transition acknowledgement and terminal publication do not acknowledge physical disposal. Clock/context owners survive retained setup, child and file-worker cleanup; both helper permits remain owned.
- **Compatibility and ownership:** Installed shared-clock paths have precise supersessions. Uninstalled pool users and non-shared Control users retain existing behavior. Generic transport gains no helper policy, configuration parser, credential material or public wire field.
- **Policy boundaries:** The accepted password-only route, explicit-key/publickey precedence, disabled-helper behavior, URL-password precedence and pool/account identity remain controlling.

These are satisfiable design obligations. Their actual implementation and concurrency behavior remain unproved at this document gate.

## 3. Risks and next action

The two initially identified architectural causes are resolved in the corrected text; no third cause invokes the cap. P3-1 can be corrected by narrowing the acknowledgement-bound sentence without changing the selected mechanism.

The next action is for the lane owner to merge the independent document verdicts and relay implementation authority only if the required axes pass. Implementation must then execute the specified interleavings, provenance projections and retained-cleanup regressions. This GO accepts the corrected design mechanism only; it supplies no final credential implementation, platform or release acceptance.
