# GwzTransportSshHelperClockAmendment.md — CONSISTENCY-AXIS REVIEW

**Review object:** Complete corrected DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md:1–307` at core `fde5878ac11b9e02892127f438cb50954551b9a3`, including changed range `d4c1f90f..fde5878a`. Remediation round 2; document closure only.

**Baseline:**

| Repository | Exact commit |
|---|---|
| root | `0bb560c5f34ba5bde5ece053dc58e33ef10fd8e0` |
| gwz-core | `fde5878ac11b9e02892127f438cb50954551b9a3` |
| gwz-transport | `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Documents and source continuity were inspected through committed `git show` and `git diff` reads. The complete tuple matched at both start and end. Unfinished implementation bytes remained excluded.

**Date:** 2026-10-03

**Axis:** Internal coherence, controlling-contract agreement, precise supersession, and satisfiability of the changed mechanism and regression obligations. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current-round work. Filed verbatim by the lane owner.

**Verdict: GO** — all prior findings close at the document level; no open P0, P1, P2 or P3 findings. No new architectural root cause was found.

---

## Prior-finding closure table

“Verified” means the original counterexample was re-traced against the corrected committed document. No implementation test was executed.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Consistency initial P2-1 | Common transition/expiry arbitration | Timely publication immediately installs the local phase under the authority lock. Control observing after the displaced Network deadline cannot latch it. Publication at equality settles expiry first. Lines 60–68 and 113–123 retain the round-1 correction. | Closed — document |
| Consistency initial P2-2 | Core witness and neutral terminal identity bridge | Pool-first, Control-first and helper-first admitted local expiry resolve the exact connection/phase witness through `SetupEnded` and core capture. Lines 212–240 retain the reporting and lifetime correction. | Closed — document |
| Consistency initial P2-3 | Live stall remainder pause/resume | Local departure captures the live remainder; Network publication rebases it before Control checks. Helper work cannot reset stall, and stale Control copies are nonauthoritative. Lines 127–154 retain the correction. | Closed — document |
| Safety initial P2-1 | Common arbitration | Independently re-traced Network100/publication99/Control101 and publication100 controls. The immediate-publication and exact-expiry rules still defeat the original race. | Closed — document |
| Safety initial P2-2 | Explicit stall restoration | Independently re-traced local work extending beyond the original live stall deadline. Resume preserves the remainder rather than charging local elapsed time or inventing progress. | Closed — document |
| Consistency round-1 P3-1 | Resume waiting depends on live deadlines, preserving Inactive stall | With aggregate Disabled and configured stall Inactive, delayed resume acknowledgement creates no deadline. Lines 92–97 explicitly make waiting cancellation/driver-lifetime bounded; lines 296–298 require the first real network wait to start stall timing. | Closed — document |
| Safety round-1 P2-3 | Defined prepared-token refusal while the active authority is Alive | With disabled network clocks and a captured 1-ms preparation, `observe` retains validated expired identity in the same slot. Publication consumes it and commits `PreparationDeadline`, allowing bounded reporting of the original allowance. Active-terminal and invalid-token alternatives have separate outcomes. Lines 70–82 and 181–207 defeat the nonexistent-terminal counterexample. | Closed — document |

## Changed-range analysis

The core range changes four documentation files: the amendment, RemPlan-2 and both round-1 reports. It changes no implementation source. Targeted diffs confirm that the controlling helper documents, GWZDesign/GWZRequirements and previously traced core clock/setup/reporting owners remain unchanged. Transport and evidence revisions are unchanged.

The amendment grows from 268 to 307 lines. The complete changed range comprises:

- Status text recording round-1 outcomes and the second correction, while retaining the implementation prohibition.
- Addition of neutral `PreparationDeadline` to the terminal vocabulary.
- Replacement of destructive expired-reservation discard with a bounded `ExpiredPrepared` state retaining the issued connection/generation/kind/deadline.
- Clarification that resume acknowledgement waits depend on **live** deadlines, including the Disabled aggregate/Inactive stall combination.
- Three explicit publication-refusal outcomes: `ActiveTerminal`, validated `PreparationExpired`, and `InvalidToken`.
- Extension of core terminal projection to the validated preparation-expiry record.
- Specific regression obligations for expired preparation, active-expiry/cancellation controls, foreign/stale tokens and inactive-stall resume.

These changes implement the two dispositions in RemPlan-2. They preserve the existing mutex, token, witness and reporting owners. The new neutral cause distinguishes failure to start the prepared phase from expiry of the still-live active phase; it adds no helper policy to transport and no public wire or application field.

**Architectural classification:** No **NEW ARCHITECTURAL root cause** was found. The correction completes an outcome branch within the previously selected synchronization/provenance mechanism. The recorded architectural count remains two; the third-cause stop is not triggered.

## 0. Evidence base

Read or re-read in this round:

- Complete canonical `PromptConsistency-2.md`.
- Complete corrected amendment, lines 1–307, and the full committed amendment diff.
- Complete committed `GwzTransportSshHelperClock-RemPlan-2.md`.
- Complete committed round-1 Safety report, including its prepared-refusal counterexample and classification.
- The committed round-1 Consistency inactive-stall finding and closure obligation.
- Root CurrentProgramCheckpoint’s opening round-2 settlement and correction sections.
- Accepted timing amendment lines 45–97: valid detail shape, captured allowance, exact milliseconds, zero/sub-millisecond allocation and independent interaction allowance.
- Accepted configuration-view amendment lines 166–174: one preparation/fill deadline after admission.
- Committed range summaries and targeted diffs confirming no change to the five controlling credential/design documents or previously traced `agent_job`, `ssh_pool`, `ssh_setup` and `ssh_worker` owners.

The original and round-1 instruction, process, controlling-document and committed-source traces remain applicable because their relevant bytes are unchanged. Those traces established the independent Control supervisor, pool expiry/admission paths, detail-dropping baseline bridge and physical cleanup owners.

Revision checks matched the exact root/core/transport/evidence tuple at start and end. Commands were inspection only. No files were written; no builds, tests, experiments, helper invocations or network requests ran. No current-round peer prompt or report was inspected. Required executable regressions remain obligations, not claimed proof.

## 2. Invariant analysis

The two new closure sequences now have defined outcomes.

For the original prepared-refusal counterexample:

1. Network aggregate and stall are Disabled.
2. Core captures 1 ms, receives a prepared token and associates its witness.
3. Publication is delayed beyond the preparation deadline.
4. Another consumer observes expiry. The work reservation expires, but the same bounded slot retains the exact issued token identity as `ExpiredPrepared`; active Network remains Alive.
5. Publication validates that identity and commits `PreparationDeadline`.
6. Core projects Timeout/Effect None/Allocation with the original 1-ms allowance. No admission or child starts, no latch or fresh allowance appears, and reporting requires no previously nonexistent Terminal record.

A later enabled Network deadline produces the same preparation result while active Network remains live. Interaction preparation can likewise expire while the old Admission remains live. Conversely, active expiry or cancellation winning arbitration takes `ActiveTerminal`, and the rejected witness cannot relabel that outcome. A stale, foreign or forged token cannot supply helper detail; core terminates only its own connection and preserves any earlier authoritative terminal cause.

For the inactive-stall counterexample, Disabled aggregate plus Inactive stall has no live deadline. Delayed Network acknowledgement is explicitly lifetime/cancellation bounded and cannot start stall timing. The subsequent real network wait establishes the ordinary configured stall interval. This agrees with the preserved Inactive state.

The full-document re-trace also retained these properties:

- **One authority:** Pool expiry, Control checks/quantum and final connection admission consult the same locked state.
- **Chronological decisions:** Time is sampled inside the authority lock; active expiry is settled before requested changes. Equality cannot revive a phase.
- **Acknowledgement:** Publication changes phase visibility; acknowledgement gates work. Expired/cancelled acknowledgement cannot launch work or extend resumed network allowance.
- **Bounded state:** Prepared, ExpiredPrepared and pending acknowledgement use the same single slot. An owned expired token prevents another preparation from being issued. Registrations remain bounded.
- **Lock discipline:** Runtime → SetupClock → Control ordering and separate witness locking remain unchanged. Waiting, notifications and effectful work occur outside locks.
- **Network clocks:** Local time consumes neither aggregate nor live stall remainder. Repeated departures capture intervening Network consumption; genuine network progress alone resets stall.
- **Provenance:** Admitted local expiry and validated preparation expiry have distinct neutral causes and exact core-owned witnesses. Generic network timeout, cancellation and invalid tokens cannot acquire inferred helper detail.
- **Allowance preservation:** Allocation is captured before both admission waits. Interaction is captured after both permits and covers preparation and fill without renewal. Preparation refusal preserves the original captured allowance.
- **Terminal and physical lifetime:** First terminal causes remain immutable. Logical failure does not acknowledge disposal. Reporting and retained child/worker cleanup preserve their context, clock and permit ownership.
- **Compatibility and scope:** Installed-path supersession remains precise; uninstalled pool users and non-shared Control users retain their contracts. Authentication policy, numeric bounds, protected dependencies and public schemas remain unchanged.

No additional independent defect was established by these attacks.

## 3. Risks and next action

This GO closes the document mechanism, including both second-round dispositions. Actual token validation, terminal projection, lock ordering, wakeups and retained cleanup still require the stated executable regressions and independent source acceptance. Existing global-job debt and native/OS copy limitations remain explicitly unclosed.

The next action is for the lane owner to merge the independent round-2 verdicts and relay the resulting document decision. Implementation authority depends on the required axes passing; final credential implementation, platform and release acceptance remain separate.
