# GwzTransportSshHelperClockAmendment — SAFETY-AXIS REVIEW

**Review object:** Complete corrected DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md:1–268`, at core `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea`, including changed range `eb06fac24a3e1eb8f4db32de261a26627e1fd01b..d4c1f90f7b8c66c2dbbdf08451edc3db420273ea`. Remediation round 1; document closure only.

**Baseline:** root `f2db0e3b8e94f5aab7ecc0fd222b66a84637aeb7`; core `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea`; transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`; evidence `662d89828b478a2acce8c0308834db7d17c872f7`. Documents and source were read through `git show` at exact committed revisions. The tuple matched at both start and end. Unfinished implementation bytes remained excluded.

**Date:** 2026-10-03

**Axis:** Safety—attack transition arbitration, expiry, prepared-token lifecycle, provenance, cleanup ownership and late-secret prevention. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current report. Filed verbatim by the lane owner.

**Verdict: NO-GO** — both original Safety findings close, but one new P2 finding blocks. No P0, P1 or P3 findings. No NEW ARCHITECTURAL root cause is identified in this round. I pre-commit to GO on a revision resolving P2-3 as specified.

---

## Prior-finding closure table

“Verified” below means re-traced against the corrected contract at the new tuple, not executed implementation testing.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Safety P2-1 | One publication/expiry/cancellation arbitration boundary for every consumer | Publication at 99 immediately installs LocalAdmission under the authority lock. Control at 101 sees that phase even without PoolHost acknowledgement. Publication at 100 settles Network expiry first. Lines 58–66 and 106–116 prohibit an independent obsolete failure latch. | Closed at document level |
| Safety P2-2 | Capture and restore live stall remainder; preserve Disabled and Inactive | Discovery’s live stall deadline is captured at local departure and rebased before the resumed Control check. Local time is excluded; genuine network progress alone resets the interval. Lines 120–146 reject pre-entry equality and remove authoritative stale Control copies. | Closed at document level |
| Consistency P2-1 | Same arbitration correction | Independently re-traced the original delayed-PoolHost/Control-first sequence as above. | Original counterexample closed at document level |
| Consistency P2-2 | Core timing witness plus neutral connection/phase terminal bridge | For an admitted LocalInteraction with a captured 1,250 ms budget, pool-first, Control-first and helper-first expiry resolve the same immutable terminal phase and matching core witness. Lines 180–199 explicitly replace detail-dropping scalar conversion; lines 201–206 retain the context through capture and cleanup. The same rule covers admitted Allocation, while zero allocation has its own immediate typed refusal. | Original counterexample closed at document level |
| Consistency P2-3 | Explicit stall pause/resume | Independently re-traced the original live-stall/local-delay/resume sequence as above. | Original counterexample closed at document level |

## Changed-range analysis

The committed core range changes four documentation files: the corrected amendment, combined remediation plan and two verbatim initial reports. It changes no source or controlling credential contract.

The amendment replaces the complete 119-line draft with a 268-line mechanism:

- Lines 17–54 define one mutex, full connection identity, checked phase generations, a supplied endpoint clock, installation and neutral terminal observations.
- Lines 58–116 replace PoolHost-only chronological draining with publication as the linearization point, bounded prepared/pending storage, acknowledgement settlement, terminal arbitration and explicit lock order.
- Lines 120–147 move aggregate and stall timing into the authority and define remainder capture, resume and genuine progress.
- Lines 151–208 add the core timing witness, neutral `SetupEnded` bridge, reporting identity and lifetime.
- Lines 212–241 preserve authentication policy and state precise supersession and affected owners.
- Lines 245–268 add deterministic concurrency, timing, provenance and cleanup regression obligations without claiming execution.

These changes implement the three accepted dispositions in the RemPlan. They introduce no unrelated authentication choice, wire field, public application schema or protected dependency change.

The prepared-token stage is part of the arbitration/provenance correction. Its expired-reservation refusal is incomplete: the corrected text explicitly permits that refusal while the active phase remains live, but requires projection of a terminal record. P2-3 below addresses that branch.

**Architectural classification:** P2-3 is a bounded missing outcome branch within the chosen mutex/token protocol. It needs no new owner, synchronization architecture or public shape. It is **not a NEW ARCHITECTURAL root cause**. The object’s recorded total remains two architectural causes from the initial round; this report does not trigger the third-cause stop.

## 0. Evidence base

New inspection in this round:

- Complete canonical `PromptSafety-1.md`.
- Complete corrected amendment at the new core SHA, with line numbering.
- Complete committed `GwzTransportSshHelperClock-RemPlan.md`.
- Complete committed **initial** `GwzTransportSshHelperClock-ReviewConsistency.md` and `GwzTransportSshHelperClock-ReviewSafety.md`.
- Complete committed amendment diff and core changed-range summary.
- Root `CurrentProgramCheckpoint.md:1–55`.
- Diff checks confirming no change to root standing/process instructions, the five controlling credential/design documents, or the previously traced core clock/setup owners.
- Re-read timing amendment lines 45–98.
- Re-read committed `agent_job.rs:119–167,260–280,436–459`, `ssh_worker.rs:186–267,509–542`, transport `pool/allocation.rs:137–205` and `pool/mod.rs:233–280`.

Retained evidence from the original review remains applicable because the relevant committed source and controlling documents are unchanged:

- Workspace and member instructions; canonical review-loop skill; process authority.
- TR1.6 bounds, admissions, secrets, retry rules and OQ6(a).
- Accepted timing and configuration-view amendments.
- Relevant GWZDesign/GWZRequirements ownership and transport requirements.
- Committed `ssh_pool`, `ssh_setup`, `agent_job`, `ssh_local`, `ssh_password`, `ssh_worker` and transport pool clock/asynchronous owner traces.

Commands were restricted to revision inspection, committed reads, committed diffs and text filtering. No files were written; no tests, builds, experiments, helper invocations or network requests ran. No current-round peer prompt or report was inspected. Required regression rows remain implementation obligations, not executed proof.

## 1. Findings

### [P2-3] Expired prepared-token refusal requires a terminal record that need not exist

**Classification:** Non-architectural missing refusal branch introduced within the remediation’s prepared-token lifecycle.

**Location:** Corrected amendment lines 68–76 and 171–178, especially the combination of:

- Expired reservations being discarded without changing the active Network clock.
- Refused publication projecting the prior immutable terminal record and ignoring the unadmitted witness.

**Violated invariant:** Every reachable transition refusal must have a defined, bounded outcome. Reporting cannot require a Terminal record while the authoritative state is Alive, and cannot silently replenish an exhausted captured allowance.

**State sequence:**

1. Network aggregate and stall timing are disabled, an expressly supported configuration.
2. Core captures a positive retained allocation of 1 ms and prepares LocalAdmission with its corresponding finite deadline. It associates the Allocation witness with the prepared token.
3. Before publication, the setup thread is delayed beyond that local deadline.
4. Another consumer calls `observe`. Under lines 75–76 it discards the expired reservation without changing the active Network clock. The authority remains Alive/Network; no timeout or cancellation has committed Terminal.
5. The setup thread resumes and calls `publish_local` with its now-invalid token. Publication must refuse.
6. Lines 174–176 require that refusal to project the prior immutable terminal record. There is no such record.

The same mismatch can occur with a sufficiently distant enabled Network deadline, or when a proposed Interaction reservation expires before a still-live Admission deadline. It is not restricted to a malformed caller or an already-terminal connection.

**Impact:** The contract supplies no legal reporting path for a normal scheduling delay between preparation and publication. An implementer must invent a generic error, fabricate a terminal cause, wait for a terminal record that disabled timing will never produce, or retry with a fresh allowance. Those choices respectively risk incorrect timeout provenance, false classification, a stuck setup, or extension of the accepted bound.

The existing regression row for expiry between preparation and publication does not distinguish active-phase expiry from prepared-reservation expiry while the active phase remains live.

**Required correction:** Define publication refusal separately for:

- An already committed active-phase terminal outcome.
- An expired or invalid prepared reservation while the active phase remains Alive.

Specify the bounded core outcome and authority/context settlement for the second case. Preserve the original captured allowance, prohibit launch and allowance renewal, and define exactly which phase identity and timing provenance are reported. Continue projecting the existing immutable cause when active Network/Admission expiry or cancellation actually won. Do not label a live Network phase as a helper expiry merely because an unadmitted witness exists.

This can be fixed within the existing owner and token protocol.

**Closure/regression test:** With Network aggregate and stall disabled, prepare LocalAdmission with a captured 1 ms allocation, install its witness, delay publication beyond the reservation deadline, and call `observe` first. Verify bounded publication refusal and logical reporting, no child/admission work, no missing-Git latch, no reset allowance and no dependence on a nonexistent terminal record. Repeat with an enabled but later Network deadline. Add controls where active Network expiry or cancellation wins; those must preserve their first cause and carry no helper detail inferred from the rejected witness.

## 2. Invariant analysis

The full corrected mechanism was attacked beyond the original counterexamples:

- **Common arbitration:** Sampling inside the mutex closes the caller-sampled timestamp race. Valid publication changes the phase immediately; acknowledgement gates work rather than phase visibility. Every irreversible terminal or completion decision must use the authority.
- **Exact expiry:** Operations settle active deadlines at or before their sampled time before applying changes. Equality cannot pause an already-expired clock. Aggregate wins aggregate/stall ties.
- **Delayed acknowledgements:** Local acknowledgement settles the installed local deadline first. Network resume rebases immediately and charges acknowledgement delay to retained network time. Acknowledgement cannot extend either allowance.
- **Stall restoration:** Disabled, Inactive and Live are explicit. Local progress calls refuse without mutation. Resumed Live state uses its retained remainder, and another departure captures the then-current remainder.
- **Bounded ownership:** There is one prepared/pending slot, one waiter registration and one driver registration. Cancellation, driver loss and disposal can settle pending receipts. Wakeups, waits and work run outside the declared lock hierarchy.
- **Outcome provenance:** Admitted local expiry carries connection/phase identity to core, where the captured witness supplies exact helper detail. Network timeout retains absent helper detail. SetupContext survives both logical capture and physical cleanup.
- **Success admission:** NativeResource observation is advisory; pool connected admission checks and commits Completed through the authority. A result expiring between those steps cannot become a successful lease.
- **Cleanup and late results:** Logical failure does not acknowledge physical disposal. Child/file-worker ownership retains both admissions and sensitive buffers; later results cannot replace the first terminal outcome.
- **Scope:** One helper lookup per setup bounds witness storage and removes the original repeated-lookup ambiguity. Existing non-shared and uninstalled users keep their contracts. Transport receives no secrets, helper policy or FailureDetail.
- **Policy and schemas:** Ambient password-only gating, key/agent precedence, URL-password policy and host-key verification remain controlling. The new library record adds no public wire or application field.

These attacks did not reveal another independent defect. The remaining blocker is the live-authority prepared-refusal branch described above.

## 3. Risks and next action

This remains a document gate. The actual implementation must execute the specified interleavings, provenance checks, lock-order checks and retained-cleanup regressions before source acceptance. Existing global-job debt and native/OS copy limitations remain inherited and explicitly unclosed.

The next action is one bounded contract correction for P2-3, followed by Safety re-verification of its counterexample at a new exact tuple. The two original Safety findings remain closed unless that correction changes their proved ordering or stall rules. Shared-clock implementation remains gated on root-relayed review GO.
