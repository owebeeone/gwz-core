# Credential helpers, typed consumers and shared SetupClock — CODE-AXIS REVIEW

**Review object:** Final credential implementation in `/Volumes/projects/limbo/gwz-dev-tr2-22`, at the exact settled tuple below. Controlling document: `gwz-core/dev-docs/GwzTransportCredentialHelpersDesign.md`, revision 4, at core `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185`; its acceptance covers design text, not this implementation.

**Baseline:** Reviewed committed sources using `git show HEAD:`, with implementation diffs from the specified per-repository bases:

| Repository | Diff base | Reviewed HEAD |
|---|---|---|
| root | Process and settlement authority | `dc2b4af36e6ad6eca4a781611fcf76f6d9e76226` |
| gwz-core | `2e64e88a28c332ed422cc390adc76738dc701bb1` | `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185` |
| gwz-transport | `35475977530171ab77ee2fbb1e8128f938acb5ae` | `41a16b2713b302c3675f081584d392afeae26ad5` |
| gwz-py | `b2369f1d0bf72f7fbbd5949d92c4c75a6fbc24b5` | `947ed579abec292a23db3db7394923e70ee4363e` |
| gwz-cli | `90fdb108f2a91ead456e07053da108b721300cd3` | `1543a3bec00cda913a02c266e89d841eeeafb55b` |
| gwz-core-evidence | Retained evidence member | `662d89828b478a2acce8c0308834db7d17c872f7` |

Auxiliary accepted MAIN Py sources were inspected at `a0773afa0cb3ae09be742dadd216dfd13527e692`.

**Date:** 2026-10-03

**Axis:** Code — architecture, interfaces, call graphs, ownership, compatibility, error paths and implementation adherence. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on their reports. Filed verbatim by the lane owner.

**Verdict: NO-GO** — two P2 findings block source acceptance. No P0, P1 or P3 findings. I pre-commit to GO on a revision that resolves **P2-1 and P2-2** as specified, with the stated regression evidence and no material expansion of scope.

---

## 0. Evidence base

The exact six-member tuple was verified at both the beginning and end of review. All HEADs matched. End statuses showed no tracked modifications. Excluded untracked drafts, reports, proposals and old evidence directories were not reviewed.

Inspection followed root `AGENTS_GWZ.md`, applicable `AGENTS.md`, `AgentProcessRules.md`, `GwzProcessOptimization.md`, and `dev-docs/CurrentProgramCheckpoint.md`. Controlling documents inspected included the helper design’s decisions, lookup, ownership, retry, wire, test and parity sections; the timing, configuration-view and SSH-helper-clock amendments; relevant `GWZDesign.md` and `GWZRequirements.md` sections; retry-plan §§4–5; transport release-plan/amendment authority; and the library-boundary policy. The ImplementationRecord was treated as evidence and limitations, not as overriding authority.

Source inspection covered:

- Configuration discovery and controlled input: `https_auth/{view,view/framing,file_worker,executable}.rs`; lookup, runner, owner and secret handling; environment and native-scope integration.
- HTTPS destination and policy, route-owned credential reuse, `https_worker/challenges.rs:1–43`, preparation through the authentication-failure producer, serving, and helper-failure producers.
- SSH password/helper selection, `ssh_setup_context.rs:1–230`, especially failure projection at lines 50–109, terminal publication at 129–142, and zero-allocation handling at 149–159; setup and agent Control ownership.
- The separate setup-job and endpoint-worker threads in `agent_job.rs` and `ssh_worker/endpoint.rs`; pool observation, worker checkout completion and `ssh_worker/open_request.rs` failure capture.
- Typed consumers in `transport_host/session/driver.rs:42–100`, `transport_host/request/https_failure.rs`, `gitbackend/transport_binding.rs`, transport observations, clone/fetch/push error consumers, and private materialization suppression at `workspace_ops/handle_materialize/apply.rs:69–84`.
- Local Runtime→Session→SSH Open budget carriage and the default/supplied-path distinctions.
- Transport SetupClock, its state/transitions and deferred wake delivery; pool installation, generation and admission paths; authored taut fields, generated protocol and regeneration checks; bounded detail/destination validation and incoming/local stream failure admission.
- Generated-only Py changes, historical drift-pin handling, auxiliary MAIN Py help/config composition, and the CLI diff, which contains only `docs/Troubleshooting.md`.

Read-only hash inspection checked all **160 indexed owned files** against the final ownership manifest: zero mismatches.

Existing evidence was inspected without executing it. The final focused consumer log records **2 passed, 0 failed**; the final affected suite records **442 passed, 0 failed, 4 ignored**. The ordinary **2103/0/1 ignored** and full candidate **2652/0/7 ignored** suites precede the final consumer patch and were not treated as final whole-suite proof. The retained Clippy log contains **50 warnings** and is not strict-core green. Source-guard receipts report no new conditional-scope, candidate-boundary or global-state violations.

No builds, tests, probes, writes or git mutations were performed. The reproductions below are source-established interleavings/fixtures proposed for deterministic closure tests, not executions claimed by this reviewer.

## 1. Findings

### [P2-1] SSH publishes the terminal before storing its detailed failure, allowing permanent loss of helper provenance

**Location:** Core `src/git/endpoint/ssh_setup_context.rs:129–142`, interacting with `failure()` at lines 50–109 and zero-allocation handling at 149–159. The receiving path is pool terminal observation→checkout failure→`ssh_worker/open_request.rs` capture. `SshOpenFailure::model_error()` in `src/transport_host/session/driver.rs:57–62` requires the retained typed detail to select code75.

**Violated invariant:** The SSH clock amendment requires pool-first, Control-first and helper-first consumers to retain the same first sanitized core `Failure`, including exact helper timing provenance. Zero retained allocation must produce M10/code75 with `helper_budget_ms = 0`, without entering a helper phase or starting work.

`terminate_failure()` first calls:

```rust
let record = self.clock.terminate(cause).deliver();
```

Only afterwards does it acquire the core state lock and store the supplied detailed `Failure`. The neutral terminal contains code, effect and setup cause, but no helper detail. `failure()` projects such a `ResourceFailure` into a scalar-only `Failure` and permanently caches it when `state.first` is empty.

**Credible interleaving:**

1. A live SSH setup reaches helper admission with zero allocation. `enter(Admission, 0)` constructs `Timeout`, `Allocation`, and `helper_budget_ms: Some(0)`.
2. `terminate_failure()` commits the neutral `ResourceFailure` terminal. Its `.deliver()` wakes the endpoint worker before the detailed failure is stored.
3. The separately running endpoint worker observes the terminal and completes the failed checkout. Its capture calls `context.failure(record)`.
4. That call sees neither an existing first failure nor a zero-allocation witness. It stores a failure with `detail: None`.
5. The producer resumes. Its `if state.first.is_none()` condition is false, so the original detail is discarded permanently.

This race can also occur after terminal publication but before wake delivery if the worker is already running. The separate owner threads make it a production concurrency path, not merely a hypothetical reentrant callback.

**Impact:** The first pool consumer can lose M10’s zero-budget provenance and code75. `SshOpenFailure` then renders a general allocation timeout and supplies no helper-specific `ModelError`; later consumers cannot recover the detail. Other detailed resource failures through the same bridge can likewise lose their bounded cause detail.

**Required correction:** Make the winning terminal’s sanitized core detail available before another observer can project and cache that terminal. Bind it to the exact winning authority record; preserve already-winning network expiry, cancellation and other terminals. Keep callbacks outside owner locks and respect the amendment’s lock-order constraints. Do not repair this by overwriting the first retained failure later or inferring helper origin from a generic allocation timeout.

**Closure/regression test:** Deterministically force observation between neutral terminal publication and the producer’s continuation—for example, a registered wake consumer that captures the terminal immediately during delivery. For `enter(Admission, 0)`, assert that the earliest capture and every subsequent capture retain `helper_budget_ms: Some(0)`, yield code75/M10, start no child and leave no permit held. Exercise a detailed fixed helper cause through the same bridge. Retain tests proving that an earlier network expiry or cancellation wins unchanged and that wake callbacks run outside the relevant locks.

**Classification:** **Non-architectural.** The accepted architecture already specifies one authority and a bounded core provenance bridge. The defect is publication ordering inside that bridge; correction does not require another clock, policy owner or wire field.

### [P2-2] The four-token challenge projection can discard Negotiate and silently change private-clone classification

**Location:** Core `src/git/endpoint/https_worker/challenges.rs:19–20`; unsupported-challenge failure construction in `https_worker/prepare.rs:279–285`; consumers in `src/transport_host/request/https_failure.rs:22–43` and `src/git/endpoint/https_remote.rs:207–220`; private-member suppression in `src/workspace_ops/handle_materialize/apply.rs:69–84`.

**Violated invariant:** Helper-design §11 and the accepted OQ7(1) ruling require a private clone challenged with **Negotiate without Basic** to fail loudly with `GitCommandFailed`, preserving the specified native parity. The bounded diagnostic projection must retain the information needed to select that behavior.

The parser scans all challenge fields for Basic but retains only the first four distinct scheme names. Both typed model classification and the libgit2 bridge decide the Negotiate exception exclusively from those retained names.

**Credible reproduction:** A discovery 401 has these five separate `WWW-Authenticate` fields, in order:

```text
Bearer realm="fixture"
Digest realm="fixture"
Foo
Bar
Negotiate
```

No field offers Basic. The parser returns `basic = false` and names `Bearer`, `Digest`, `Foo`, `Bar`, discarding Negotiate. The endpoint therefore starts no helper and emits an ordinary unsupported-challenge `Authentication` failure.

`HttpsOpenFailure::model_error()` selects `RemoteRejected` because the retained names do not contain Negotiate and no credential was rejected. The libgit2 bridge likewise selects `Auth`. A new private member’s materialization then follows the quiet-skip path. Moving Negotiate earlier in the same field set changes the result to loud failure.

**Impact:** A valid challenge’s field order changes a private member from a failed member into a silently skipped member. The diagnostic also hides the scheme responsible for the required parity exception.

**Required correction:** Preserve classifier-critical Negotiate presence within the accepted bounded projection, for example by prioritizing it over a noncritical retained token when the four-token capacity is full. Preserve the four-token/32-character limits and sanitized wire shape. The correction must survive encoded/carried failures; a host-local flag alone would leave other consumers inconsistent.

**Closure/regression test:** Add parser and typed-consumer cases with Negotiate fifth and later, mixed case, duplicates and reordered fields, all without Basic. Assert loud `GitCommandFailed` private-clone behavior, no helper invocation and no realm leakage. Round-trip the resulting failure through the admitted carriers. Include Basic beyond the retained-token limit to confirm that accepted Basic challenges still select the helper path correctly.

**Classification:** **Non-architectural.** This is a bounded projection-selection defect. The existing detail shape can preserve the classifier-critical scheme without a new schema, policy owner or architectural decision.

## 2. Invariant analysis

Several attempted refutations did not produce findings:

- **Additive protocol compatibility:** The authored schema keeps existing SSH username semantics and adds optional HTTPS username/detail fields. Missing/null optional decoding and the generated field projections support retained readers and older writers. MalformedOutput remains fixed cause8. The inspected detail validators bound token count, token size, pipe vocabulary, attempt counts and helper budgets, and reject incompatible combinations.
- **Admission before retention:** Encoded preflight and envelope validation cover the relevant failure carriers. Incoming and locally generated stream failures are admitted before retaining their diagnostic data. The inspected changes avoid cloning unchecked detail into retained state.
- **Typed helper ownership:** The SSH and HTTPS helper-specific consumers require bounded typed provenance for code75; ordinary timeouts lacking that provenance remain ordinary. First `TransportAttempt` errors are retained rather than replaced by later libgit2 defaults. The inspected fallback recognizes the helper-owned HTTP error code/class and fixed message prefix, rather than arbitrary timeout text. P2-1 defeats the producer side of this otherwise appropriate contract.
- **Configuration-view contract:** The inspected implementation uses native scoped discovery and an unconditional controlled view, with controlled-empty stdin handling, byte/null/empty preservation, fixed HOME anchoring and no repository-local discovery. Environment filtering and direct spawning preserve the accepted mechanism. The discovery FIFO limitation remains recorded; it was not represented as solved.
- **Resource and secret ownership:** Helper permits, host slots, process-group cleanup, stdout bounds, discarded stderr and route-owned credentials have explicit owners. The inspected call graph introduces no credential approve/reject/store operation or helper retry fallback.
- **URL and route boundaries:** Destination validation rejects forbidden decoded controls before helper input. Encoded HTTPS username remains distinct from SSH username. Challenge-triggered lookup and route-owned reuse remain scoped to the effective destination; the selected redirect policy does not resend the previous location’s credential.
- **SSH clock and budgets:** The neutral clock remains transport-owned without helper secrets or policy. Core witnesses carry local timing meaning. The installed/uninstalled pool paths, generation checks and wake-delivery structure were inspected. Local validated allocation and interaction caps reach Session and SSH Open; narrower caps are not replaced by the defaults. P2-1 is specifically a defect in detailed terminal publication, not evidence that these budget-carriage paths widen caps.
- **Source boundaries and integration:** New conditional sections use explicit enclosing boundaries in the inspected changes. Existing unchanged debt was not reclassified as newly introduced. Py changes are generated-only and additive for code75; accepted MAIN help/config sources compose without a handwritten credential-runtime alteration. CLI changes are documentation-only.

These results support the bounded nature of the two corrections. They do not negate the observable failures in §1.

## 3. Risks and next action

The retained gates do not exercise the two identified adversarial cases. Whole ordinary/candidate suites predate the final consumer patch; final affected and focused gates provide narrower evidence. Windows, performance, selected-source, packaging and release outcomes remain explicitly deferred. This review makes no activation or release claim.

The next action is to correct **P2-1 and P2-2**, retain deterministic regression evidence, settle a new exact tuple and obtain the required source acceptance. The current tuple must not be accepted for credential integration while either P2 remains open.
