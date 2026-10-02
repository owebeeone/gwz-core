# GwzTransportSshHelperClockAmendment.md — CONSISTENCY-AXIS REVIEW

**Review object:** DRAFT `gwz-core/dev-docs/GwzTransportSshHelperClockAmendment.md`, introduced in core `646e3e1c..eb06fac24a3e1eb8f4db32de261a26627e1fd01b`, dated 2026-10-03. Document review only.

**Baseline:**

| Repository | Exact commit |
|---|---|
| root | `5d89a096dd0688e5b82b501a6ce997dcfae060f1` |
| gwz-core | `eb06fac24a3e1eb8f4db32de261a26627e1fd01b` |
| gwz-transport | `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Documents and source were read with `git show` at these commits. The complete tuple was verified at the start and end and remained unchanged. Uncommitted implementation bytes were excluded.

**Date:** 2026-10-03

**Axis:** Internal coherence, agreement with controlling contracts, precise supersession, and satisfiability of the proposed regression obligations. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — three P2 findings block. I pre-commit to GO on a revision that resolves P2-1, P2-2 and P2-3 as specified.

---

## 0. Evidence base

Read:

- Workspace `AGENTS_GWZ.md`, root `AGENTS.md`, and core/transport instruction files.
- Canonical `/Users/owebeeone/.claude/skills/review-loop/SKILL.md`.
- Root process authority: `AgentProcessRules.md`, particularly L1-08, L1-09 and L1-18; `GwzProcessOptimization.md`, particularly §§4 and 8.
- Root `CurrentProgramCheckpoint.md`, opening SSH-clock draft and configuration-mechanism status sections.
- Complete controlling draft, lines 1–119, and its committed introduction diff.
- TR1.6 `GwzTransportCredentialHelpersDesign.md`: acceptance and operator answers, §§1–7, bounds/cleanup at lines 78–83, and outcomes at lines 98–119.
- `GwzTransportCredentialHelperTimingAmendment.md`: exact timing/provenance rules, particularly lines 38–97, supersession and regression sections.
- Complete `GwzTransportCredentialHelperConfigurationViewAmendment.md`, particularly the shared preparation deadline at lines 166–209 and source-owner/supersession sections at lines 223–262.
- `GWZDesign.md` and `GWZRequirements.md`: credential-context/configuration-view authority additions and relevant transport, ownership, timeout and credential requirements.
- Committed core owner graph:
  - `ssh_setup.rs`: lines 72–200 and 355–415.
  - `ssh_pool.rs`: lines 22–65 and 80–250.
  - `agent_job.rs`: lines 72–238 and 256–455.
  - `ssh_worker.rs`: failure capture at lines 186–267, completion at lines 288–309, and pool dispatch/reporting at lines 625–739.
  - `ssh_local.rs`: setup assembly.
  - Relevant searches of `ssh_connection.rs` and `ssh_remote.rs`.
- Committed transport owner graph:
  - `pool/clock.rs`: complete file.
  - `pool/mod.rs`: configuration, request, errors and actions.
  - `pool/machine.rs`: pending/opening clock representation and request deadlines.
  - `pool/allocation.rs`: scheduling and connection-completion admission.
  - `pool/lifecycle.rs`: failure/cancellation ownership and connect-clock creation.
  - Relevant `pool/asynchronous.rs` checkout, driver and interaction seams.

Commands were limited to reading, searching, committed diffs and revision inspection. No files were written; no builds, tests, experiments, helpers or network requests ran. The sequences below are contract/source traces, not executed test evidence.

## 1. Findings

### [P2-1] PoolHost ordering does not establish chronological ordering for the other expiry consumers

**Classification:** **NEW ARCHITECTURAL root cause** — the shared authority lacks a defined linearization rule between pending transitions and independently executing expiry readers.

**Location:** Draft lines 28–45, especially the instruction that PoolHost drains pending transitions before advancing expiry.

**Violated invariant:** A timely local transition must prevent stale network expiry at every consumer. Sharing a handle does not establish this ordering when Control still independently records irreversible failure.

**State sequence:**

1. Network expires at endpoint time 30,000.
2. At 29,999, the setup owner requests LocalAdmission and retains the request pending pool acknowledgement.
3. Before PoolHost drains that request, Control’s independent supervisor runs at 30,001.
4. An implementation that publishes the acknowledged phase only when PoolHost processes the request still exposes Network to Control. Control records aggregate timeout and cancellation.
5. PoolHost subsequently processes the timely request in timestamp order. Its ordering cannot undo Control’s already recorded failure without violating the draft’s cancellation/terminal rules.

The independent execution is real in the committed owner graph: `agent_job::Entry::reap` invokes `Control::update` at lines 260–280, independently of `ssh_pool::PoolHost::step_reported`. Control checks also occur on the setup thread and during result polling. The draft orders PoolHost’s work but supplies no corresponding rule for these readers.

**Impact:** A valid pre-deadline pause can still fail at the original network deadline. The central stale-tick guarantee is therefore not established by the proposed mechanism.

**Required correction:** Specify one ordering rule covering transition submission, all three expiry readers, cancellation and terminal publication. For example, every expiry observation must account for already-submitted transitions through its observation timestamp under the authority’s synchronization boundary. Define when submission becomes visible, who acknowledges it, and whether any consumer may commit expiry while an earlier transition remains pending. State the lock order and prohibit holding a consumer/authority lock while waiting for pool acknowledgement.

**Closure/regression test:** Submit a pause just before Network expiry, delay PoolHost, and run Control’s supervisor/check first just after the original deadline. It must not commit stale aggregate expiry. Run the reverse ordering with a genuinely late submission; it must remain expired. Add cancellation between submission and acknowledgement and prove no launch or resurrection.

### [P2-2] The local-expiry reporting path has no owner that preserves helper timing provenance

**Classification:** **NEW ARCHITECTURAL root cause** — phase expiry arbitration and the required helper outcome are connected only by an asserted regression, without a defined outcome/provenance bridge.

**Location:** Draft lines 32–36, 47–52, 76–79 and 105–110.

**Violated invariant:** A helper Allocation or Interaction timeout must retain its exact captured allowance and remain distinguishable from a generic timeout, regardless of which expiry consumer wins.

The accepted timing amendment is explicit:

- Lines 40–43 identify helper timeout through `helper_budget_ms`.
- Lines 54–55 require absent detail to preserve generic rendering and prohibit relabeling by inference.
- Lines 57–65 require the applied interaction allowance and the initial retained admission allowance, rather than the remainder observed at expiry.

**State sequence:**

1. LocalInteraction is acknowledged with a captured 1,250 ms allowance.
2. At exact expiry, PoolDriver observes the phase deadline before the helper/result owner reports its typed outcome.
3. The generic pool fails the checkout and cancels connecting work.
4. The committed pool error representation has only scalar `InteractionTimeout`/`AllocationTimeout`, or `ConnectFailed` containing code/effect/setup cause (`pool/mod.rs:247–266`). `PoolMachine::connected` also drops `Failure.detail` when constructing `ConnectFailed` (`allocation.rs:172–181`).
5. The SSH worker reports the checkout error through `EndpointOpenFailure::capture`, which constructs `Failure { detail: None, … }` (`ssh_worker.rs:201–263`).

The draft changes the clock consulted by pool expiry and requires captured detail to survive, but names neither an outcome carrier nor the SSH worker’s capture seam in its affected owners. An eventual helper outcome cannot repair a checkout already completed through the generic expiry path.

**Impact:** The pool-first interleaving loses M4/M10 provenance. It produces generic rendering or forces forbidden inference from a generic timeout, and the required captured-detail regression has no specified passing path.

**Required correction:** Define the private connection-bound record that owns the captured helper phase/allowance, its lifetime through terminal reporting, and how core retrieves it when pool expiry wins. Preserve generic transport’s policy independence: it may carry neutral connection/phase identity, while core owns helper classification and `FailureDetail`. Explicitly include the SSH worker/error-capture seam in the impact and supersession boundary. Apply the same rule when Control or the helper outcome wins, including zero-allocation refusal.

**Closure/regression test:** Force pool-first, Control-first and helper-first expiry separately for both local phases. Assert identical helper code, setup cause and exact budget detail, including 1,250 ms and zero allocation. A generic non-helper timeout must retain absent helper detail.

### [P2-3] Suppressing stall expiry during local work leaves its resumed state undefined

**Classification:** Bounded contract omission; no new architectural root cause.

**Location:** Draft lines 28–30, 56–58, 71–73 and 95–102.

**Violated invariant:** Network clocks must exclude helper admission and interaction time. The draft expressly retains Control’s stall-progress ownership, but only says that a local phase neither starts nor expires a network stall timer. It does not define what happens to an existing timer on entry and resume.

**State sequence:**

1. Authentication-method discovery has a live network wait timestamp and a 5-second stall allowance.
2. The setup enters LocalAdmission/LocalInteraction while that timestamp remains recorded.
3. Helper work succeeds after 10 seconds, within its independent allowance.
4. Network resumes. The required fresh cancellation/expiry check invokes Control.
5. If the retained timestamp is used unchanged, the resumed stall timer is immediately due, although those 10 seconds were local work.

This follows the committed Control representation: `wait_started` is retained state; `update` computes `wait_started + stall` at lines 123–145, and `stall_remaining` subtracts wall-clock elapsed time at lines 214–220. Merely skipping that calculation while local does not exclude local elapsed time after resume.

**Impact:** A successful helper can be rejected immediately as a network stall. Aggregate timing may be corrected while the retained stall state recreates the original failure through another clock.

**Required correction:** Specify the stall state at each acknowledged boundary. Choose and state whether a live stall remainder is paused/resumed or the relevant network wait is ended and a new wait begins on resume. Define treatment of progress reports received while local, preserve exact-expiry behavior, and prevent helper work from manufacturing network progress.

**Closure/regression test:** Enter local work with a live stall timer, spend longer than its original wall-clock deadline locally, then resume. Local time must not trigger stall expiry. Subsequent network inactivity must expire at the stated retained/reset allowance; a stall already expired before local entry must not be revived.

## 2. Invariant analysis

Several attacks did not identify independent defects:

- **Single aggregate authority:** The draft correctly identifies Control, NativeResource and generic pool timing as consumers that must stop retaining separate original network deadlines. Its proposed handle remains transport-neutral.
- **Configuration preparation:** Draft lines 69–70 agree with the accepted configuration-view amendment’s single deadline covering discovery, parsing, file work, verification and fill. No stage receives a fresh preparation allowance.
- **Admission versus interaction:** The draft preserves distinct endpoint/host admission and excludes slot waits from the newly captured interaction allowance. Zero/exhausted allocation cannot start a helper or create a missing-Git latch.
- **Disabled network timing:** `None` remains distinct from an exhausted positive allowance, while local phases remain bounded.
- **Cancellation and physical cleanup:** The draft requires acknowledgement before child launch, rejects late results, retains both permits through unfinished child/worker ownership, and keeps terminal authority alive through physical cleanup. These requirements agree with the accepted configuration-view cleanup contract.
- **Policy and compatibility scope:** Existing SSH policy choices remain settled. The proposal adds no secret-bearing transport field, public application schema, protected dependency change or process-global clock.
- **Existing users:** Non-shared Control users and old pool interaction APIs are expressly preserved. The helper’s fresh interaction allowance is confined to the proposed configured-password mechanism.
- **Status:** DRAFT status and the implementation prohibition before root-relayed GO are explicit. Required TDD rows are an inventory, not claimed execution evidence.

These strengths do not close the ordering, provenance or stall-resumption gaps above.

## 3. Risks and next action

The repeated-local-phase wording should also be made precise during correction: “captured exactly once” must mean once per departure from Network, with intervening Network consumption deducted, rather than reusing an earlier remainder. I have not raised a separate finding because the draft explicitly prohibits manufacturing network allowance.

Platform qualification, performance, unfinished implementation acceptance and final credential release outcomes remain deferred as instructed.

The next action is one bounded draft revision defining the shared chronological arbitration, helper-expiry reporting bridge and stall boundary rules, followed by Consistency re-review of P2-1 through P2-3. No implementation acceptance follows from this report.
