# Credential helper remediation round 1 — CODE-AXIS REVIEW

**Review object:** Bounded original-reporter closure of Code **P2-1 and P2-2**, on the revised settled tuple. Controlling correction: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan.md` at core `64ec039089b6e217625d7784b106d917452963cb`. Source acceptance remains pending.

**Baseline:** Original reviewed core `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185` and transport `41a16b2713b302c3675f081584d392afeae26ad5`. Corrected sources were read using committed `git show HEAD:` and targeted diffs; necessary surrounding call sites were inspected on the verified tree.

| Member | Revised reviewed HEAD |
|---|---|
| root | `1de9e7fcbd17e5159ebb71ef3b683c2656a1cf8a` |
| gwz-core | `64ec039089b6e217625d7784b106d917452963cb` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

**Date:** 2026-10-03

**Axis:** Code — independently re-trace the original publication-race and challenge-projection counterexamples, with necessary changed surroundings and source-specific evidence. Independent, adversarial, read-only. Current peers remain blind; nothing here relies on their reports. Filed verbatim by the lane owner.

**Verdict: GO** — original Code **P2-1 and P2-2 close**. No new blocker was found within this bounded closure inspection. This verdict closes these original counterexamples only; it does **not** accept the complete credential package, the shared API’s full proof surface, integration or release.

---

## 0. Evidence base

All six required HEADs matched at the beginning and end. End statuses showed no tracked modifications. Excluded untracked drafts and evidence remained outside inspection.

Read the complete canonical closure prompt, the newest credential checkpoint at `dev-docs/CurrentProgramCheckpoint.md:1–42`, the committed combined RemPlan, and the committed RemediationRecord. Prior Code findings supplied the original counterexamples. Current-round peer reports were not read.

Targeted source inspection covered:

- Core `ssh_setup_context.rs:1–172`, including the pending-publication wait and zero-allocation producer; new `ssh_setup_context/publication.rs:1–59`; immediate-observer regression at `ssh_setup_context/tests.rs:45–67`; and `ssh_setup_context/tests/publication.rs:1–136`.
- Necessary SSH consumers in `ssh_worker/open_request.rs`, especially its failure capture and corrected completion lock scope; `ssh_password_helpers.rs:88–143`; `transport_host/session/driver.rs:42–68`; and the typed budget recognizer in `transport_host/helper_failure.rs`.
- Transport `setup_clock.rs`’s existing mutex/settlement path, deferred `ClockUpdate::deliver`, and new `terminate_if_alive`; `tests/setup_clock/atomic_admission.rs`.
- Core `https_worker/challenges.rs:1–78`, the failure producer at `https_worker/prepare.rs:279–285`, typed model classification at `transport_host/request/https_failure.rs:22–46`, libgit2 classification at `https_remote.rs:215–230`, and private suppression at `workspace_ops/handle_materialize/apply.rs:69–89`.
- The complete real private-materialization regression in `transport_host/https_negotiate_projection_tests.rs`.

The final receipt’s SHA-256 was independently verified:

`ceed1a3740aa76ab3609c1758392bea9d3c9abe6d04200e462147addd2ed523d`

All **39 owned-file hashes** matched the corrected tree. Hashes also matched for the inspected final affected log, final transport log, original core RED-v2 log and atomic-admission RED log.

The retained final affected log records **455 passed, 0 failed, 4 ignored** and explicitly records successful execution of all six core regressions relevant here:

- Immediate observer retains zero-allocation detail.
- Independent pool observer retains exact detail, including publisher unwind.
- Equal prior resource terminal and competing publishers preserve the first detail.
- Abandoned publication and prior terminals do not install unadmitted detail.
- Negotiate survives bounded projection and codec round-trip.
- Real private materialization fails loudly without helper invocation.

The final transport log records the atomic-admission regression as passed. Original RED-v2 evidence records the immediate observer receiving `None` instead of `Some(0)` and Negotiate disappearing from the bounded projection. Atomic RED evidence records an equal existing terminal incorrectly returned as admitted.

No builds, tests, probes, writes or git mutations were performed by this reviewer. Closure combines independent source re-tracing with inspected retained execution evidence.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Code P2-1 | Associate sanitized detail with the exact admitted terminal before observers can cache fallback; preserve earlier winners and release pending association on unwind. | Re-traced the original zero-allocation pool-first interleaving. Publication reserves the association before clock admission; early projection waits; guard destruction installs detail only for the newly admitted record before notifying/delivering wakes. Immediate-callback and independent-pool regressions pass in the final affected log. Earlier cancel/network/equal-resource and unwind cases are covered. | **Closed** |
| Code P2-2 | Preserve Negotiate within four bounded tokens, without a schema or host-only workaround. | Re-traced the original five separate challenge fields. Negotiate replaces the fourth noncritical token when capacity is full, survives the carried failure, selects `GitCommandFailed`, and avoids private suppression. Mixed-case/duplicate/reordering and codec regression plus real private-materialization regression pass. Basic eligibility still examines later fields. | **Closed** |

## Changed-range analysis

For P2-1, core adds a cohesive publication owner and a condition variable guarding the logical association. `Publication::new()` reserves publication under the core state lock and releases that lock before clock access. The clock’s atomic result distinguishes **newly admitted** from **existing** terminals, including identical scalar causes. Guard destruction installs the supplied detail only for `Ok(admitted)`, clears the pending state, releases the core lock, then notifies readers and delivers clock wakes.

`failure()` waits while that logical association is pending, so postponing notifications is not the sole protection. An already-running pool observer cannot cache a scalar fallback during the publication interval. `OpenRequest::complete()` also now clones its setup context into a local before invoking failure capture, releasing its setup-slot lock before the possible wait.

The additive shared `terminate_if_alive` method is a **material interface change**, explicitly approved by root and assigned separate fresh Code/State review. I inspected the admission/settlement behavior needed to close the original race; this bounded report does not substitute for that broader interface review.

For P2-2, the parser retains its existing bounded projection and case-insensitive Basic eligibility. When four diagnostic tokens are already present, a newly encountered Negotiate token replaces the last retained token. Subsequent noncritical tokens cannot displace it. No new wire field or local-only classification flag is introduced.

The nearby HTTPS RPC facts change belongs to another original disposition; it was not re-reviewed here. No concrete new root cause was found in the changed surroundings necessary for these two closures. No new architectural finding is asserted.

## 2. Invariant analysis

**P2-1’s original race no longer succeeds.** For zero admission, the producer still constructs `Timeout`, `Allocation` and `helper_budget_ms: Some(0)` before any permit acquisition or child start. The new publication reservation precedes neutral terminal exposure. An observer that reaches `failure(record)` during publication waits with the core mutex released. After publication resolves, both the earliest and later projections return the same detailed first failure. The typed SSH consumer therefore recognizes the exact zero budget and selects code75/M10.

An existing terminal cannot acquire the candidate detail merely because its scalar cause matches. `terminate_if_alive` settles time and checks the terminal under the existing authority mutex; core installs detail only for the admitted result. Precommit abandonment clears the pending association without installing detail. Postcommit unwind runs the same guard destruction that associates admitted detail and releases waiters. Notifications and callback delivery occur after the relevant locks are released.

**P2-2’s original challenge sequence no longer succeeds.** For `Bearer`, `Digest`, `Foo`, `Bar`, then `Negotiate`, the retained names now include Negotiate within four entries. No Basic is offered, so no helper starts. The sanitized names enter the existing failure detail. Both typed model classification and the libgit2 bridge see Negotiate; the former selects `GitCommandFailed`, and the latter avoids the suppressible authentication result. The private-member suppression branch requires `RemoteRejected`, so it no longer quietly skips this member.

The parser still bounds each token to 32 characters and excludes realms/parameters. Basic discovery remains independent of diagnostic capacity: a later Basic token sets eligibility even when the retained names are full. The inspected codec round-trip and real materialization regression substantiate those source paths.

## 3. Risks and next action

This is limited closure, not whole-package re-review. The new shared atomic API requires the separately mandated fresh Code/State verdicts. Other original findings and Surface corrections are outside this report’s closure authority.

Retained affected-suite evidence is bounded. Earlier full suites remain pre-correction; the disclosed default-parallel physical SSH failure and its subsequent isolated/bounded-concurrency disposition are not relabeled as full or release proof. Deferred platform, packaging and release outcomes remain unchanged.

The next action is to combine this original Code closure with the required independent fresh-interface and remaining-axis verdicts on the same tuple. **Code P2-1 and P2-2 are closed; aggregate source acceptance remains pending.**
