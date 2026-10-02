# GwzTransportSshHelperClockAmendment — SAFETY-AXIS REVIEW

**Review object:** DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md`, lines 1–119, at core `eb06fac24a3e1eb8f4db32de261a26627e1fd01b`; introduced in `646e3e1c..eb06fac2`. Document review only, dated 2026-10-03.

**Baseline:** root `5d89a096dd0688e5b82b501a6ce997dcfae060f1`; core `eb06fac24a3e1eb8f4db32de261a26627e1fd01b`; transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`; evidence `662d89828b478a2acce8c0308834db7d17c872f7`. Documents and source were read with `git show <exact-SHA>:<path>`, excluding working-tree changes. The tuple matched at both start and end.

**Date:** 2026-10-03

**Axis:** Safety—attack the mechanism’s races, expiry semantics, stuck states, cleanup ownership and disclosure boundaries. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — two P2 findings block; no P0, P1 or P3 findings. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.

---

## 0. Evidence base

Read authority and review instructions:

- Workspace `AGENTS_GWZ.md`; committed root `AGENTS.md`; committed core and transport `AGENTS.md`.
- Safety prompt at `/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/ssh-helper-clock-review/PromptSafety.md`.
- Canonical `/Users/owebeeone/.claude/skills/review-loop/SKILL.md`.
- Root `dev-docs/CurrentProgramCheckpoint.md:1–51`.
- Root `dev-docs/AgentProcessRules.md`, especially L1-17 through L1-21 at lines 370–460.
- Root `dev-docs/GwzProcessOptimization.md`, especially §§3–4 at lines 78–120.

Read controlling contract sections at the pinned core SHA:

- `dev-docs/GwzTransportSshHelperClockAmendment.md:1–119`, plus its committed introduction diff.
- `dev-docs/GwzTransportCredentialHelpersDesign.md`: accepted decisions; §§3.1–3.5, §§5–7 and OQ6(a), particularly lines 62–96, 128–144 and 419–422.
- `dev-docs/GwzTransportCredentialHelperTimingAmendment.md:1–115`: exact captured millisecond budgets, allocation provenance and independent interaction allowance.
- `dev-docs/GwzTransportCredentialHelperConfigurationViewAmendment.md:1–290`: preparation deadline, sequential children, bounded worker, retained permits and buffers, and supersession.
- `dev-docs/GWZDesign.md:1–145`, particularly the context and transport ownership rules.
- `dev-docs/GWZRequirements.md:45–122`, particularly bounded endpoint capacity, cancellation and operation isolation.

Read committed source solely to trace proposed owners:

- Core `src/git/endpoint/ssh_setup.rs:1–230,362–421`: deadline conversion, setup job, retained deadline and result classification.
- Core `src/git/endpoint/ssh_pool.rs:1–270`: Connector/Opening seam, physical ownership and host tick order.
- Core `src/git/endpoint/agent_job.rs:1–535`: independently serviced Control failure state, stall timestamps, supervisor, result admission and retained cleanup.
- Core `src/git/endpoint/ssh_local.rs:1–133` and `ssh_password.rs:1–220`: setup route and network wait/progress calls.
- Core `src/git/endpoint/ssh_worker.rs:90–155,260–320,476–554,604–780,790–835`: outer Open deadline, pending cancellation and attachment.
- Targeted committed-source searches in `ssh_network.rs`, `ssh_connection.rs` and `ssh_worker.rs`.
- Transport `src/pool/clock.rs:1–193` and `src/pool/asynchronous.rs:1–338`: current expiry, interaction transitions, runtime mutex and driver acknowledgement.

Commands were inspection only: `pwd`, `git rev-parse`, `git status --porcelain=v1`, `git show`, the committed document diff, and `rg`/`nl`/`sed`/`cat` for inspection. Status showed in-flight changes and untracked artifacts covered by the prompt’s exclusions; their contents were not reviewed.

No files were written. No tests, builds, experiments, helper invocations or network operations ran. Closure tests below are proposed, not executed evidence. No current-round peer prompt or report was inspected.

## 1. Findings

### [P2-1] Pending transitions are ordered against pool expiry but not against Control’s irreversible failure latch

**Classification:** **NEW ARCHITECTURAL root cause.** Shared deadline storage does not by itself establish one arbitration order between asynchronously acknowledged transitions and every expiry consumer.

**Location:** Amendment lines 26–45, particularly Control’s retained failure ownership at lines 28–29 and the PoolHost-only chronological drain at lines 39–44.

**Violated invariant:** A timely transition must prevent every consumer from expiring the displaced network deadline. Whether it succeeds cannot depend on which supervisor reads the shared handle before PoolHost acknowledges it.

**State sequence permitted by the text:**

1. Network expires at endpoint time 100.
2. At time 99, the setup owner publishes `LocalAdmission(until=500)` and waits for the pool acknowledgement. No child starts.
3. PoolHost has not yet serviced the request. The acknowledged phase remains Network.
4. At time 101, the independently running setup supervisor calls Control’s expiry update. Control consults the shared authority, sees the still-acknowledged Network deadline of 100, and latches an aggregate failure.
5. PoolHost later drains the timely transition. Its timestamp ordering cannot undo Control’s failure; cancellation or Terminal now defeats the request.

This independent update path exists in committed `agent_job.rs:260–280`, and failure is retained by `Control::update` at lines 119–146. The draft changes the deadline source but specifies chronological processing only for PoolHost. It does not require Control’s expiry query to account for an earlier pending transition before latching failure.

**Impact:** A valid admission can fail as network aggregate expiry despite a timely pause. The shared-clock mechanism retains the scheduler-dependent rejection it is intended to eliminate. Depending on when the race occurs, the helper never launches or a valid result is discarded.

**Required correction:** Define a common linearization and arbitration contract for transition publication, timestamp assignment, cancellation, expiry and acknowledgement. Every consumer must resolve earlier published transitions before committing an expiry outcome. A pending timely transition must not leave Control free to latch the displaced deadline.

Specify bounded request ownership and acknowledgement waiting, and require waits and callbacks outside arbitration locks. Preserve these outcomes explicitly:

- A transition published at or after expiry cannot revive the phase.
- Cancellation wins before acknowledgement and launch.
- Delayed acknowledgement does not permit work after the resulting local deadline.
- Driver loss or disposal settles pending requests.

The implementation primitive may vary; the ordering rule must be part of the contract.

**Closure/regression test:** With a deterministic endpoint clock, publish LocalAdmission at 99 against a Network deadline of 100, pause PoolHost, and run the actual Control/supervisor expiry path at 101 before acknowledgement. The request must subsequently receive the correct acknowledgement without aggregate failure, and no child may start beforehand. Add controls for publication at 100, cancellation before acknowledgement, and acknowledgement after local expiry. Repeat the ordering attack for LocalInteraction and resume Network.

### [P2-2] The stall timer has no defined restoration rule after a local phase

**Classification:** Non-architectural contract omission in the retained Control stall owner.

**Location:** Amendment lines 28–32 and 71–73. Local phases suppress starting and expiring stall timers, but only the aggregate network remainder has a specified capture/resume rule.

**Violated invariant:** Helper admission and interaction must consume no network-stall allowance. A successful helper cannot cause an immediate stall rejection merely because its local work took longer than the stall interval.

**State sequence permitted by the text:**

1. SSH authentication-method discovery completes at time 5 with a 10-second stall interval.
2. As committed `ssh_password.rs:141–143` demonstrates, successful discovery calls `Control::complete_wait`. Committed `agent_job.rs:159–167` leaves `wait_started = Some(now)`.
3. LocalAdmission and LocalInteraction run from time 5 to time 25. The amended Control suppresses stall expiry during those phases, as required.
4. Network resumes with a positive aggregate remainder.
5. The fresh Control check before offering the password evaluates the retained stall timestamp. Its deadline remains time 15, so the check at time 25 latches Stall and rejects the valid result.

Suppressing checks during a phase does not exclude that phase’s elapsed time from the existing absolute stall calculation. The draft neither freezes and restores the stall remainder nor clears/rebases the old timestamp.

**Impact:** A valid sign-in lasting longer than the configured stall interval fails immediately on resume, even when it satisfies the full helper interaction allowance and leaves ample network time.

**Required correction:** State the stall transition policy explicitly. Preserve any live stall remainder across local phases, or define the justified fresh-wait behavior at resume; local elapsed time must never enter the resumed stall calculation. Include what happens when no stall wait was active, when stall timing is disabled, and when the transition timestamp is exactly the stall deadline. Apply the rule before the fresh post-helper check.

**Closure/regression test:** Use a Control with a completed discovery wait, a positive aggregate remainder and a short enabled stall interval. Spend longer than that interval in acknowledged local phases, finish within the helper bound, resume Network, and verify the fresh check and password offer succeed. Then stop network progress and verify expiry at the specified resumed stall boundary. Include disabled-stall and exact-boundary controls.

## 2. Invariant analysis

The following attacks did not establish additional findings:

- **Three aggregate expiry consumers:** The draft explicitly replaces Control, NativeResource’s original-deadline result check and pool connecting expiry with the shared authority. It prohibits independent arithmetic extensions of three deadline copies. P2-1 concerns their arbitration order, not an omitted aggregate consumer.
- **Outer Open deadline:** `ssh_worker.rs:509–542` constructs an envelope from allocation plus connect plus interaction allowances when network timing is enabled. For one lookup using retained allocation, retained network and one interaction allowance, this is not demonstrably shorter than the legitimate phase total. It is not reported as another blocker.
- **Independent helper allowance:** Lines 65–70 capture the interaction budget after both permits and include configuration preparation and fill under one deadline. Slot waits cannot shorten it; preparation cannot restart it. This agrees with the timing and configuration-view amendments.
- **Disabled network timing:** Lines 56–58 and 87–91 preserve disabled network timing while keeping local phases bounded. Disabled network does not imply an unbounded helper.
- **Exhausted admission:** Lines 60–68 preserve exhausted allocation, refuse launch and avoid the missing-Git latch. They do not replenish admission with a default.
- **Cancellation and late secrets:** Lines 44–45 and 71–85 require cancellation to defeat launch, a fresh pre-offer check, host-key verification and disposal of late results. They retain the clock through physical cleanup and forbid reuse for another setup.
- **Cleanup and capacity:** Lines 81–85 preserve both admissions and sensitive buffers while a child or file worker remains unfinished. Logical timeout is not a disposal acknowledgement.
- **Ownership and disclosure:** The transport handle excludes secrets, URLs, configuration and SSH policy. Configuration parsing remains under the accepted core mechanism. No new process-global owner, public wire field or GWZ request/response change is proposed.
- **Existing paths:** The amendment preserves non-shared Control users and old pool interaction APIs, and confines the changed helper route to the accepted ambient password-only case. Publickey offers, explicit keys, disabled helpers and URL-password policy are not reopened.

These are document guarantees, not implementation acceptance.

## 3. Risks and next action

The document supplies no executable proof. Final implementation review must verify bounded transition storage, wakeups, lock ordering, driver-loss settlement, exact phase boundaries and retained cleanup against the adopted contract. Existing global-job debt and native/OS secret-copy limitations remain inherited; this review does not certify or expand them.

The next action is one bounded revision of the DRAFT resolving P2-1 and P2-2, followed by independent re-review at a new exact tuple. Shared-clock implementation remains gated on root-relayed review GO.
