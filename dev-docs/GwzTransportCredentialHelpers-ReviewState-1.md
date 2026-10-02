# Credential helpers, typed consumers and shared SetupClock — STATE-AXIS REVIEW

**Review object:** Credential implementation remediation round 1 in `/Volumes/projects/limbo/gwz-dev-tr2-22`, frozen at the tuple below. Source acceptance remains pending. Controlling document: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan.md` at core `64ec039089b6e217625d7784b106d917452963cb`.

**Baseline:**

| Repository | Reviewed HEAD |
|---|---|
| root | `1de9e7fcbd17e5159ebb71ef3b683c2656a1cf8a` |
| gwz-core | `64ec039089b6e217625d7784b106d917452963cb` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Committed sources were inspected with `git show HEAD:` and range diffs. Numbered working-source reads were bound to committed bytes by the supplied source-hash receipt. All six heads matched at the beginning and end.

**Date:** 2026-10-03

**Axis:** State machines, terminal arbitration, races, ownership, physical cleanup and fail-closed behavior. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — one new P2 finding blocks acceptance. All six original blocking counterexamples are closed on this tuple. No P0, P1 or P3 finding is asserted. I pre-commit to GO on a revision resolving this report’s **P2-1** as specified, with its closure regression and no material expansion of scope.

---

## 0. Evidence base

I read the complete canonical `PromptState-1.txt`, root workspace instructions, root/core `AGENTS.md`, the newest credential checkpoint sections, relevant process review and material-change clauses, and the library-boundary policy.

Authority inspected included:

- `GWZDesign.md` and `GWZRequirements.md`: adopted helper context, configuration view and shared SSH clock requirements.
- `GwzTransportCredentialHelpersDesign.md`: revision 4’s selected operator rulings, process-group lifecycle, completion, buffers and outcomes, particularly §§3.2–3.4 and §4.
- Accepted timing, configuration-view and SSH-helper-clock amendments: exact budget capture, final result checks, phase arbitration, witness ownership, cleanup and conditional configuration scope.
- Retry-plan §4 and relevant clock/lifecycle contracts.
- The original Code, State and Surface reports and the combined RemPlan.
- `GwzTransportCredentialHelpersRemediationRecord.md`, treated as evidence and claimed dispositions rather than superior authority.

The fresh source examination covered the entire changed production range and its relevant callers:

| Area | Principal portions inspected |
|---|---|
| Atomic terminal admission | Transport `pool/setup_clock.rs:1–303`, `state.rs`, `transitions.rs`; new `atomic_admission.rs:1–31`; surrounding pool observation, service, installation and final connected admission |
| Detailed publication | Core `ssh_setup_context.rs:1–232`, `publication.rs:1–59`, context regressions and `tests/publication.rs:1–136`; Control shared bookkeeping; NativeResource result projection; OpenRequest completion |
| Helper result and cleanup | `https_auth/runner.rs:1–219`, `lookup.rs:1–193`, `owner.rs`, `runner/tests.rs:1–143`, relevant runner and ownership tests |
| Credential representation | `https_auth/secret.rs:1–116`, username regression and whole-capacity overwrite implementation |
| SSH enablement and reuse | Backend transport binding, private request/opening, session opening, URL handoff, worker runner, local authentication selection, selected-key registry and enablement regressions |
| Failure carriers | Transport `stream/incoming.rs:1–169`, stream machine retention/close contracts, `tests/closed_failure.rs:1–121`; core BlockingStream and HTTPS RPC adapter/tests |
| Challenge classification | Challenge parser and tests, actual private-materialization regression, typed HTTPS classification and error propagation |
| Unchanged ownership contracts | Helper admission, route answers and pool-scope retirement, retained allocation, configuration preparation/file-worker ownership, native FIFO correction and independent descendant-write cleanup assertion |

Unchanged original-package analysis was reused where the remediation did not alter its proof surface. I independently re-traced every original blocking counterexample and attacked the new atomic API and publication owner.

The final source receipt was read and hashed:

`/Volumes/projects/limbo/gwz-tr222-remediation-final-source-receipt-20261003.json`

SHA-256:

`ceed1a3740aa76ab3609c1758392bea9d3c9abe6d04200e462147addd2ed523d`

Read-only hash inspection compared all **39 owned entries** with committed `git show HEAD:` bytes: 31 core, five transport, one CLI and two Python; **zero mismatches**. The receipt’s precommit member-head fields were not substituted for the reviewed tuple.

All eleven final gate logs matched their receipt hashes. Recorded results inspected included:

- Core affected union: **455 passed, zero failed, four ignored**.
- Transport, including doctest: **185 passed, zero failed, two ignored**; SetupClock suite includes the new atomic regression.
- Strict transport library Clippy: exit zero.
- Core Clippy: exit zero, **49 warnings**; the modified private opening arity remains explicitly unwaived.
- CLI help/retry tests: **three passed**.
- Python parser: **28 passed**; protocol drift passed.
- Conditional-boundary, candidate-inventory and process-global guards: passed.

The default-concurrency **454/1/4** result remains preserved. Its ordinary SSH address-fallback row passed in isolation, and the bounded-concurrency affected union passed before and after final cleanup. Earlier full ordinary/candidate suites remain pre-correction evidence.

For the prior Surface counterexamples, I inspected the revised source text and relevant passages in all six CLI network help artifacts and all seven Python help artifacts. All thirteen artifacts matched their manifests. This was textual closure, not executable product qualification.

No builds, probes, tests, writes or Git mutations were performed. No current-round peer report or excluded draft was read. End status showed only the specified excluded untracked drafts and old evidence directory; intended tracked source was unchanged.

## 1. Findings

### [P2-1] Final helper refusal can occur after process-group cleanup ownership has been discarded

**Location:** Core `src/git/endpoint/https_auth/runner.rs:151–158`, especially the last `admit_child_output` at line 158. Ownership destruction: `https_auth/owner.rs:201–214`; empty-job drop: lines 234–242.

**Violated invariant:** Helper-design §3.3 and §4 require a timeout or cancellation to kill the spawned group, with retained ownership where cleanup is incomplete. The final result-admission decision must occur while the job still owns the cleanup capability needed for its refusal.

The corrected runner performs successful-output and parsing checks while the job owns the child and group. It then does:

```rust
if result.is_ok() {
    job.complete_if_exited();
} else {
    job.terminate().await?;
}
self.admit_child_output(result)
```

`complete_if_exited()` calls `complete()` when the leader has exited. `complete()` clears `process_group`, takes/drops the child and releases the job’s permit reference. The following admission check can independently convert the still-successful result into `Timeout` or `Cancelled`. That new refusal does not revisit the termination branch.

**Credible state/interleaving sequence:**

1. A supervised Git/helper leader starts a descendant in its process group. The descendant closes inherited stdout/stderr and continues writing a heartbeat file through its own descriptor.
2. The leader emits a valid credential answer and exits successfully. Stdout reaches EOF; the completed work branch is ready.
3. The checks at lines 151–152, including final parsing, succeed before the unchanged deadline and before cancellation.
4. At lines 153–154, `complete_if_exited()` observes the already-reaped leader and clears the child/group ownership.
5. Before line 158’s fresh check, the thread is preempted until the deadline, or another thread cancels the operation.
6. Line 158 returns `Timeout` or `Cancelled`, wiping/refusing the answer.
7. `HelperJob::drop` sees no child and returns immediately. Neither that drop nor the final check signals the former group. The descendant continues its writes, and there is no retained cleanup entry accounting for it.

Thread preemption or concurrent cancellation is sufficient; no malformed answer, escaping process group or unverified SSH host is required. Closing the descendant’s inherited pipes makes the leader/EOF completion independent of its continued activity.

**Impact:** The operation correctly refuses the credential, but reports timeout/cancellation without the mandated group kill. Spawned work can remain alive after the lookup and its admissions retire. This is a physical-cleanup correctness defect, not demonstrated corruption or credential exposure.

**Required correction:** Keep the last decision capable of changing success into refusal inside the job’s cleanup ownership. If that decision refuses, terminate the group before releasing that capability. Establish one explicit final admission point so later success cleanup cannot be followed by a new refusal that lacks an owner. Preserve the unchanged deadline, parsing checks, cancellation behavior, wiping and retained-cleanup rules; merely adding another post-cleanup check repeats the problem.

**Closure regression:** Add a deterministic seam at the success-cleanup/final-admission boundary. Use a completed leader with a live same-group descendant that has closed inherited output pipes and writes a heartbeat. Force both deadline equality and cancellation at that boundary. Assert no admitted answer or Authorization, group termination, bounded cessation of independent heartbeat writes, and no premature admission release or false cleanup acknowledgement. Keep the normal-success case proving the accepted completion behavior.

The current new regressions exercise already-ready timeout/cancellation and parsing crossings before cleanup. The existing `timeout_kills_descendants_even_after_git_exited_with_stdout_open` regression exercises the error branch while ownership still exists. Neither covers this post-completion refusal path.

**Classification:** **NEW NON-ARCHITECTURAL root cause.** The remediation adds a refusal check after disposing its required cleanup capability. The accepted owner and policy suffice to correct it; no new shared protocol, clock, policy or platform assumption is needed. No new architectural cause is asserted.

## 2. Invariant analysis

**Exact publication association held under the attacked interleavings.** `Publication::new` reserves the logical association before terminal commitment. Readers wait on the context Condvar without holding the authority lock. `terminate_if_alive` settles expiry and distinguishes `Ok(admitted)` from `Err(existing)` under the existing mutex, including identical scalar causes. Drop installs detail only for admitted results, clears the reservation, unlocks, notifies readers and then delivers deferred wakes. Precommit abandonment and postcommit unwind resolve the reservation. Synchronous reentrant wake observation therefore occurs after association, and competing publishers cannot attach rejected detail to an earlier terminal.

Relevant production readers release their owning locks before projection. In particular, OpenRequest now clones `setup_slot` into a local before `EndpointOpenFailure::capture`. NativeResource projection follows Job result polling rather than holding Control bookkeeping through the Condvar wait. No production self-wait or authority/context lock cycle was established.

**The shared clock’s arbitration remained fail-closed.** `change` samples the supplied endpoint clock under the authority mutex, clamps backwards samples and settles active expiry before mutation. Exact equality defeats new admission. Prepared expiry retains the issued identity without pausing the active phase; expired publication can settle its own typed refusal while earlier active expiry/cancellation remains authoritative. Foreign/stale token and receipt checks do not grant allowance. Registration remains bounded to one driver and one receipt waiter.

**Network time was not renewed by helpers.** Local departure captures aggregate and live-stall remainders once. Admission-to-Interaction preserves them; Network publication rebases the retained remainder before acknowledgement. Disabled and Inactive remain distinct. Local progress cannot reset network stall. Final pool connected admission still commits Completed through the authority and rejects late native success.

**The original late-answer admission defect is corrected.** Successful child output is checked before parsing; parsing checks before and after; another check follows parsing. Thus selecting completed work when the timer/cancel is already ready cannot admit its answer. The new finding concerns cleanup after a later refusal, not acceptance of the original late answer.

**Username conversion preserves allocation ownership.** Parsing reserves the terminator slot before copying credential bytes. First and repeated SSH conversion fit that allocation. HTTPS Basic construction excludes the terminator. Whole-capacity overwrite still covers unused capacity at destruction.

**Helper-disabled SSH operations are isolated.** The existing backend policy reaches private opening/handoff and gates helper selection. Disabled operations use a separate opaque pool identity while `Opening.identity` preserves authentication selection. Registry tokens use the selected-key namespace; disabled pool identities do not substitute for registry lookup. Selected-key authentication is rebound to the requested pool identity after authentication. The real enablement regressions cover both operation orders and selected-key reuse.

**Incoming terminal ownership is preserved.** Admitted `Closed.failure` now moves into retained ownership after validation. Close facts remain separate. Terminal receive guards ignore later messages, preserving the first raw Failure. The RPC adapter enriches only its derived callback/error value when Failure-owned facts are absent; existing Failure facts retain precedence.

**Classifier-critical challenge information survives carriage.** Negotiate displaces a noncritical fourth diagnostic token when necessary. Basic eligibility still scans beyond diagnostic capacity. The carried projection remains bounded, sanitized and sufficient for loud private-clone classification.

**Configuration/crash grammar remains bounded.** Preparation creates no named sensitive copy or durable recovery marker. Controlled-empty stdin parsing suppresses inherited native prereads; initial discovery’s native-file limitation remains explicit. File workers open nonblocking, verify the opened handle’s regular-file type, and retain permits/buffers through unfinished ownership. Output, scratch, configuration and credential buffers retain whole-capacity wiping. The independent descendant-write assertion is meaningful evidence for its existing error branch, but does not close P2-1.

## 3. Risks and next action

The next action is a bounded correction to **P2-1**, with the deterministic ownership-boundary regression, refreshed affected gates and a newly frozen tuple for closure. This tuple must not be accepted for credential integration while that finding remains open.

Residual limits remain those recorded in the package: pre-correction full suites, retained core warnings and ignored campaigns, native Git/OS copies outside Rust’s zeroization claim, and the native initial-discovery read limitation. Windows/provider/trust, performance, selected-source, packaging, release and supplied-carrier outcomes remain deferred. This report grants no release or platform acceptance.

## Prior-finding closure table

“Verified” below means the original counterexample was independently re-traced on the corrected tuple. Recorded test passes support the source trace; they are not executions performed by this reviewer.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Code P2-1 | Associate exact winning detailed terminal before fallback projection | Immediate wake and independent pool observer cannot pass `publishing` before admitted detail is associated. Equal earlier scalar resource terminal returns `Err(existing)` and acquires no candidate detail. Unwind releases the reservation. | Closed |
| State P2-1 | Same publication race | Zero-allocation terminal retains exact zero budget/code75 across earliest and later captures; prior network/cancel wins unchanged. Context and publication regressions cover the original observation window. | Closed |
| Code P2-2 | Preserve Negotiate beyond four tokens | Original fifth-token field order now retains Negotiate. Mixed case, duplicates and reordering remain bounded; Basic beyond capacity remains eligible. Codec and real private-materialization regressions preserve loud `GitCommandFailed` without helper/realm leakage. | Closed |
| State P2-2 | Prevent populated username growth | The original tightly sized conversion now has a reserved NUL slot before secret copy. Pointer/capacity remain stable on first/repeated conversion; HTTPS Basic stays equivalent. | Closed |
| State P2-3 | Check successful output and final parsing against unchanged deadline/cancel | With both work and refusal ready, completed-output check refuses success. Equality and parsing-crossing checks refuse the parsed answer before caching. A different later cleanup-order defect is this report’s P2-1. | Original counterexample closed; new P2-1 open |
| State P2-4 | Carry per-operation disabled policy and isolate pool reuse | Helper-enabled endpoint plus disabled backend cannot enter helper lookup or reuse enabled helper authentication. Both operation orders and selected-key scope reuse are traced and covered by physical regressions. | Closed |
| State P2-5 | Retain admitted Closed Failure and separate facts | Real close handshake moves exact Failure ownership, retains separate close facts/error, rejects invalid detail and ignores later terminals. RPC callback derives typed facts without mutating raw retained Failure. | Closed |
| Surface P3-1 | Use effective conventional-file set/unset recipes | Revised README and all seven help artifacts use paired explicit `.gitconfig` commands and explain the alternate `GIT_CONFIG_GLOBAL` case. Both setting/removal now address the documented resolver source. | Textual counterexample closed |
| Surface P3-2 | Qualify elapsed examples and identify helper phases | All six revised CLI network help artifacts identify network-only examples, separate retained admission/full interaction, SSH network pause and code75 recovery. A helper wait beyond the network example is now explained. | Textual counterexample closed |

## Changed-range analysis

The reviewed delta is core `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185..64ec039089b6e217625d7784b106d917452963cb`, transport `41a16b2713b302c3675f081584d392afeae26ad5..1aab733783e06b25cb5d2321d71ec0b34417a29c`, CLI `1543a3bec00cda913a02c266e89d841eeeafb55b..f925e1165c2b2d368a00277594450b010a95867a`, and Python’s composed base `34398b6ace772a5d85191273086a17363a404a4d..a0d4350f31069362b3cfbeae668666ab47567264`.

Production changes fit the claimed dispositions:

- Transport adds the neutral atomic admitted/existing result and preserves the old `terminate` method. The same clock mutex, settlement and deferred-notification mechanism remain authoritative.
- Core adds one cohesive logical publication owner and bounded Condvar state; it introduces no secret into transport.
- Helper final parsing moves inside supervised Job ownership and receives the required result checks. Its additional post-cleanup refusal check introduces **NEW NON-ARCHITECTURAL P2-1**.
- Username sizing and Basic conversion repair the sensitive-growth path.
- Negotiate prioritization repairs the bounded classifier projection without schema expansion.
- Existing helper policy travels through private local SSH carriage, with pool partition and original authentication selection kept distinct.
- Closed Failure retention and derived RPC facts repair the supported terminal consumer path.
- CLI and Python changes are help/README corrections; no new runtime selector policy is introduced.

The public atomic API addition warrants the mandated fresh review. Its name and result expose precisely the admitted/existing distinction needed by the consumer, and no missing lifecycle pair, default or command-placement defect was established. The changed opening arity warning remains disclosed debt; it is not silently counted as strict Clippy acceptance.

There are **zero new architectural root causes** in this review and **one new non-architectural root cause**. All six heads remained at the specified tuple through the final check.
