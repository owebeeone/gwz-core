# GWZ 1.1.0 plan — amendment: gwz-py enters the transport through the CLI's entry

Date: 2026-09-26. Status: **superseded for §3.1, §3.2, §3.4, §3.7, §3.8, §3.9, §4, §6 and §3.5's sentence on gwz-py's notes by [`GwzTransportReleasePlan.md`](GwzTransportReleasePlan.md) as of 2026-09-27. Historical evidence and already-completed gates remain valid only where the new document says they do**. Its other clauses stay in force as that plan's §4 adopts them. The earlier status, kept for the record: **accepted at SHA-256 `cb4ae1664a595231bde73cdb83809fa4b2316f574ca6365fb878255df4be1585` after [Consistency-2](GwzV110PlanAmendment-ReviewConsistency-2.md) and [Safety-2](GwzV110PlanAmendment-ReviewSafety-2.md) reported GO; this accepts the plan text only**. The status sentence was added after that GO. So were four corrections the reviewers cleared without a further round, which [Verdict-2](GwzV110PlanAmendment-Verdict-2.md) records. Third draft, after the [first](GwzV110PlanAmendment-Verdict.md) and [second](GwzV110PlanAmendment-Verdict-1.md) reviews.
- Amended 2026-10-01 by [`GwzTransportReleasePlanAmendment-2.md`](GwzTransportReleasePlanAmendment-2.md). This document remains authoritative only as amended for its §3.4 (Phase 6) and its §3.5 and §3.6 sentences on gwz-py, in 1.1.0's run.

This amendment controls [GwzV110Plan.md](GwzV110Plan.md) and the standing of gwz-py's [transport design](../../gwz-py/dev-docs/GwzPyTransportDesign.md) as that plan's Phase 1 design. It backs out gwz-py's long-lived transport session. Instead, gwz-py reaches the transport through the entry gwz-cli uses, one runtime per operation. Its public API does not change. It changes no clock, no platform, no crate identity step and no release order.

## 1. Documents controlled

- `gwz-core/dev-docs/GwzV110Plan.md`, accepted at plan SHA-256 `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62`. The file last changed at gwz-core `b141e26d`, which added the closing note, and now hashes to `583dd3089df9bc1e972e174a386ed80e9e69597f2b0a94a89df833742d03659c`. Line numbers below are that file's.
- `gwz-py/dev-docs/GwzPyTransportDesign.md`, as last changed at gwz-py `5f938a0`: its 2026-09-23 acceptance as S1.1 and S1.2, the bounded amendment to the plan's Phase 1 and Phase 6 that its status line accepts, and its NO-GO paragraph.

Only the clauses in §3 change. The rest of the plan stays authoritative as written.

## 2. Problem evidence

1. **Phase 1's design, accepted on 2026-09-23.** S1.1 named `gwz-py/dev-docs/GwzPyTransportDesign.md`. It was accepted as S1.1 and S1.2 on 2026-09-23: Consistency, Safety and Surface reported GO at gwz-py `259f73cc`, paired with core `479926c1`.
   - Its status line also accepts a bounded amendment to Phase 1 and Phase 6. That amendment names gwz-py's extension as the binding, adds no fifth crate, and publishes `gwz-transport` at Phase 8 step 2, core at step 5 and gwz-py at step 7. It was never applied to the plan's text.
   - Its design, one long-lived `TransportSession` per Python `Client`, was implemented in candidate builds (`gwz-py/native/src/transport_session.rs`).
2. **The NO-GO it left open.** The same day, an [operator-directed finding](../../dev-docs/history/GwzPyTransportConcurrencyNoGo.md) put Phase 6 completion and Phase 7 activation at NO-GO. That session refuses a second network operation while one runs ("another network operation is active"), so overlapping commands on one `Client` fail. The design train meant to close the NO-GO was retired on 2026-09-24, which left the NO-GO with no closing condition. The plan's closing note still records it as open (line 446).
3. **The contract that followed.** The [clean-slate proposals](../../dev-docs/GwzClientCoreTransportProposals.md) and the [core session contract](../../dev-docs/GwzCoreSessionDesign.md) came next.
   - The contract's §14 supersedes the long-lived session for the Python `Client`. It restates S6.3's test as "two overlapping Python operations complete independently on one Client, each on its own runtime".
   - Its §5.2 has an operation enter the transport through a variant of gwz-cli's `with_local_transport` that takes the caller's cancellation token.
4. **The operator's decision (2026-09-26).** 1.1.0 backs out the long-lived session, so gwz-py and gwz-cli reach the transport through one entry, one runtime per operation, and gwz-py keeps its public API.
   - The rest of the contract is not in 1.1.0: the session host, its message channel and wire proof, core-owned admission and limits, process globals moved into contexts, and gwz-cli's dispatch moved into core.
   - [GwzCoreSessionPlan.md](../../dev-docs/GwzCoreSessionPlan.md) schedules that rest for a later release.
5. **The stale plan text.** The plan still describes a binding, a choice of package boundary, "one pool" in §2 (line 50), and one pool serving two Python operations (S6.3).

## 3. Superseded clauses and their replacements

### 3.1 §2 scope

- **Line 50.** Superseded: "| SSH and gh-only HTTPS, both placements, one pool |". Replacement for its left cell: "SSH and gh-only HTTPS; both placements for the CLI and local placement for Python; one pool in each operation's runtime". Its right cell is unchanged.
- **Line 52.** Superseded: "| Python using the same `gwz-transport` pool as Rust | A second pool implemented in Python |". Replacement: "| Python network operations entering the transport through the CLI's entry, one runtime per operation | A second pool implemented in Python; a Python binding of `gwz-transport`; connection reuse across Python operations |".
- **Lines 62–64.** Superseded: "Phases 4, 5, and 8 redact". Replacement: "Phases 4, 5, 6, 7, and 8 redact". The rest of that paragraph is unchanged, including "Python-visible errors in S6.3 follow the same rule."

### 3.2 Phase 1 (lines 72–100)

The heading is unchanged. In the paragraph at lines 74–76, "Production core does not depend on `gwz-transport`" becomes "Before S7.1, production core does not depend on `gwz-transport`." S1.1, S1.2 and the sentence after them (lines 78–100) are superseded. Replacement:

> - **S1.1: revise the design** *(`gwz-py/dev-docs/GwzPyTransportDesign.md`; design only, 0 product lines)*.
>   - **What changes.** The revision replaces the long-lived `TransportSession` with the per-operation model. Each Python network operation enters the transport through gwz-core's `with_local_transport` entry, as a gwz-cli command does, in the variant that takes the operation's cancellation token (S6.1), and runs on its own runtime.
>   - **What the public API keeps.** The public Python API is unchanged, and the revision states what each transport-related part means under the per-operation model: `TransportCleanup`, `Client.close` and `close_report`, `Client.cancel_operation`, `transport_capabilities` and `configure_transport_runtime`.
>   - **What survives.** It keeps these guarantees from the contract's §14 list:
>     - isolation from invalid proxy or CA settings;
>     - the file-identity preflight;
>     - the `TransportCleanup` shape;
>     - `Client.meta(max_retries)`;
>     - gh-only HTTPS and the sanitisation of Python-visible errors;
>     - the release, registry-pin and credential-hygiene checks.
>   - **What changes in documented behaviour.** The endpoint environment is captured at each operation's start, as a gwz-cli command captures it, and is stable only within that operation. A change to the process environment takes effect at the next operation, so it is no longer stable for the life of a `Client`. `configure_transport_runtime` stays process-wide, so two `Client`s in one process share its setting. The process-wide helper caps remain the only bound across `Client`s.
>   - **What is added.** It bounds each `Client` to 8 running network operations; further ones wait, cancellable, and the revision states whether a waiting operation holds a native thread. It also states what `Client.close`, interpreter exit and `Client.cancel_operation` do with several operations running.
>   - **No binding.** There is no Python binding of `gwz-transport` and no separate package. gwz-py's extension ships the transport through gwz-core, as its 2026-09-23 package decision already found.
>
> - **S1.2: review the revision** *(review documents beside the design; 0 product lines)*.
>   - Dual peer-blind Consistency and Safety review of the revised design at an exact gwz-py commit, plus Surface, because the revision changes documented Python behaviour: the environment's capture point. Remediation follows GwzProcessOptimization §4.
>   - S1.2 closes on a filed verdict that names that commit, with both axes GO.
>   - A later revision of the design needs its own GO before implementation follows it.
>
> Nothing in this plan implements the revision before S1.2 closes.

### 3.3 S2.2, cross-repo edges (lines 125–127)

Superseded: "Cross-repo edges (`gwz-git` to the API crate, and a separate binding package if S1.1 names one) are registry dependencies"

Replacement: "Cross-repo edges (`gwz-git` to the API crate) are registry dependencies"

### 3.4 Phase 6 (lines 276–301)

Superseded in full. Replacement:

> ### Phase 6 — Python integration (milestone: gwz-py's network operations run through the CLI's transport entry, one runtime per operation, and overlap on one `Client`)
>
> Phase 6 starts after S1.2 closes.
> - **Size.** Each step is within this plan's 500-line aspiration, not counting removed code, so no separate implementation plan gates the phase.
> - **gwz-cli.** This phase does not change gwz-cli.
> - **Windows.** Every new transport arm is a `cfg_if` arm for Unix and Windows under the candidate switch, matching S4.5. S7.1 later removes the switch from these arms too.
>   - If S6.1 lands after S4.5, it re-runs S4.5's dabeest fixtures through `with_local_transport` before it merges.
>   - S6.2's arms are guarded by S6.3's dabeest rows, which wait on S4.5.
>
> - **S6.1: the cancellable entry** *(gwz-core `transport_host`)*.
>   - **The variant.** A variant of `with_local_transport` registers the operation's request with a caller-supplied cancellation token. It refuses with `Cancelled` once the token is cancelled. Otherwise it builds, runs, finishes and shuts down as `with_local_transport` does.
>   - **Library safety.** Unlike the CLI's entry, the variant runs inside a host process, so it never finishes from `Drop` while unwinding. It catches a panic in the operation first, runs finish and shutdown under their own panic guard, and reports failure with cleanup unconfirmed. The `TransportSession` it replaces had the same guard, and the contract's §5.2 requires it.
>   - **Unchanged.** `with_local_transport` and gwz-cli's use of it stay as they are.
>   - **Tests.** Core unit tests cover:
>     - a cancel before the start;
>     - a cancel while running;
>     - the cleanup report each returns;
>     - a fault-injected panic in finish after a panic in the operation, with the process staying alive and the next operation succeeding.
>
> - **S6.2: gwz-py on the entry** *(gwz-py native extension, the bridge, and `gwz-py/RELEASE.md`)*.
>   - **What goes.** The native `TransportSession` and the bridge's use of it are removed.
>   - **The new path.** gwz-py's dispatch runs each transport-scope operation inside the S6.1 entry, and scopes the entry's backend through the extension's existing backend scope. It registers the operation's token so that `Client.cancel_operation` reaches it. Each `Client` runs at most 8 network operations at once.
>   - **The public API.** The public Python API is unchanged, and its per-operation meanings are those S1.1's revision states.
>   - **Release pins.** Amend `RELEASE.md` and the publish workflow so every native dependency pin is named. `gwz-core` on the release branch is `=1.1.0` from crates.io. `GwzCratesIoPlan.md` D7's git-tag-only core pin is not the 1.1.0 form.
>
> - **S6.3: focused tests** *(gwz-py-level tests that S1.1's revision names)*. They run on macOS ARM64, Linux x86-64 and dabeest, against disposable SSH and HTTPS fixtures. The dabeest rows wait on S4.5. Each network test asserts, through the result's transport observations, that the operation took the transport route.
>   - Two overlapping Python operations complete independently on one Client, each on its own runtime.
>   - A gh failure and an unsupported proxy still refuse, and credential material does not appear in the Python-visible errors.
>   - An operation run without `configure_transport_runtime` uses the accepted default clocks, and S3.3's stall regression passes through gwz-py's path.
>   - Cancelling a running network operation returns its cleanup report.
>   - A cancel naming a wrong, foreign or completed operation fails without cancelling any running operation, and completed-cancellation state stays bounded.
>   - With 8 operations running on a `Client`, a ninth waits without refusal, runs when a slot frees, and can be cancelled while it waits.
>   - `Client.close()` with a network operation running cancels or joins it, and its report counts it. Interpreter exit also cancels or joins it. Neither leaves a helper process behind.
>   - An environment change made after an operation starts does not affect that operation, and a change made before the next operation does.
>   - Runtime construction cost and connection counts are recorded for 1, 2 and 8 overlapping operations, for S7.2's notes.
>
> Retained S6.3 evidence follows §2's redaction rule. Runs on a path-pinned candidate tree are development evidence. S7.3's route assertion on the normal builds is the pre-publish proof, and Phase 8's post-release check repeats it on the published artifacts.

### 3.5 Phase 7

- **S7.1 (lines 308–315).** Added after "Keep each platform arm inside `cfg_if`.":
  > The switch is also removed from:
  > - gwz-py's sites (`native/src/lib.rs`, `native/src/dispatch/mod.rs`, `native/src/dispatch/merge.rs`);
  > - gwz-cli's sites (`src/globalargs/dispatch.rs`);
  > - S6.1's variant and S6.2's arms;
  > - all three crates' `check-cfg` declarations (`gwz-core/build.rs`, `gwz-cli/build.rs`, `gwz-py/Cargo.toml`);
  > - gwz-core's transport-candidate harness: `tests/transport_backend/prepare.py`, its `test_prepare.py`, whose protocol-boundary assertion is retired or inverted, and its `README.md`.
  >
  > After S7.1, `rg gwz_transport_candidate` over the source, tests, scripts and manifests of gwz-core, gwz-cli and gwz-py finds nothing; `dev-docs` is excluded. Only the switch goes: the native branch stays for the paths 1.1.0 does not support.
- **S7.2 (lines 317–323).** Added after "The notes state the gh-only HTTPS policy.":
  > The route ledger has a Python row for each platform. Its evidence is S6.3's and S7.3's route assertions, in place of an S5.6 row, since S5.6's matrix has no Python rows. The rows are entered here on S6.3's assertions and confirmed by S7.3 before S7.5. The notes state that gwz-py builds one runtime per network operation, captures the environment at each operation's start, reuses no connections across operations, and runs at most 8 operations at once per `Client`.
- **S7.3 (lines 325–329).** Added after its first sentence:
  > On each platform, the consumer build runs one CLI network operation and two overlapping Python network operations against the disposable fixtures. Each asserts through its transport observations that it took the transport route. This is the pre-publish proof that the normal builds ship the transport, and it runs before S7.5.
- **The exit-row table (line 352).** Superseded: "| Network-entry ledger, no silent native route | S7.2 |". Replacement: "| Network-entry ledger, no silent native route | S7.2, S6.3, S7.3 |".

### 3.6 Phase 8

- **Step 2 (lines 377–378).** Remove "If the bindings live in this repo, this step waits on Phase 6 and is their publisher."
- **Step 3 (lines 380–383).** Superseded in full. Replacement: "3. **No separate binding package.** There is none, so this step does not exist. The later steps keep their numbers."
- **Step 7 (lines 401–402).** Superseded: "after `gwz-core` 1.1.0 and the binding's publishing step (step 2 or step 3) are on their registries". Replacement: "after `gwz-core` 1.1.0 is on crates.io. S7.3's route assertion was the pre-publish proof for gwz-py; the post-release check repeats it on the published wheel".
- **The post-release check (line 409).** After "The route ledger's advertised commands work on those hosts.", add: "On each host, one CLI and one Python network operation assert through their transport observations that they took the transport route."

### 3.7 §5, out of scope (after line 442)

Added:

> - A Python binding of `gwz-transport`, and connection reuse across Python operations.
> - The rest of the core session contract: the session host, its message channel and wire proof, core-owned admission and limits, process globals moved into contexts, gwz-cli's dispatch moved into core, and gwz-cli onto the session host. [GwzCoreSessionPlan.md](../../dev-docs/GwzCoreSessionPlan.md) schedules them.

### 3.8 The closing note (line 446)

Superseded: "S6.3 and the Phase 6/7 NO-GO remain open."

Replacement: "The Phase 6/7 NO-GO of 2026-09-23 closes when S6.3's overlapping-operations test passes on all three platforms."

The note's first sentence, on the withdrawn S6.3 clarification, is unchanged.

### 3.9 The dependency sketch (lines 415–425 and 429)

The sketch gains one edge, `S4.5 ── S6.3`. In line 429, "S6 waits on S1.2." becomes "S6 waits on S1.2, and S6.3 also waits on S4.5." The chain `S1.1 ── S1.2 ── S6.1 ── S6.2 ── S6.3 ── S7.1` is unchanged.

### 3.10 Unchanged on purpose

- **Phases 3, 4 and 5**, except for the S4.5 re-run in §3.4.
- **S7.4 and S7.5.**
- **S4.5's "binding construction"**, which names core's transport construction, not a Python binding.
- **§1's outcome.** 1.1.0 still ships the transport in the normal gwz, gwz-core and gwz-py builds, with connection pooling within each operation.

## 4. Affected tests and evidence

- **S6.3's one-pool test** becomes the contract's §14 test, run on all three platforms with route assertions, together with the other S6.3 rows.
- **The long-lived session's tests leave with it**, as the contract's §14 lists them: physical-session reuse, the construction and admission barriers, the single-active-operation refusal, and the Python network lock.
- **gwz-cli's tests** are unaffected.
- **New release evidence:**
  - S7.3's pre-publish route assertions on the normal builds;
  - the Python rows of S7.2's route ledger;
  - Phase 8's post-release route assertions.

## 5. Review and application

- **Review.** Dual peer-blind Consistency and Safety review of this draft's text, identified by its SHA-256, by the reviewers of the earlier drafts. No Surface review of this amendment: no gwz command or option and no public Python API changes. gwz-core gains one additive crate function. The documented change to the environment's capture point is reviewed with S1.1's revision, whose S1.2 includes Surface.
- **On GO**, status-only edits under AgentProcessRules §7.2:
  - **`GwzV110Plan.md`'s status gains:** "Amended <date of GO> by `GwzV110PlanAmendment.md`. This document remains authoritative only as amended for §2's scope rows and evidence rule, Phase 1, S2.2's cross-repo edges, Phase 6, S7.1, S7.2, S7.3 and the exit-row table, Phase 8 steps 2, 3 and 7 and its post-release check, the dependency sketch and its prose, §5 and the closing note."
  - **`GwzPyTransportDesign.md`'s status gains** that this amendment supersedes its 2026-09-23 acceptance and bounded plan amendment for the long-lived session, that S1.1's revision replaces its design, and that its NO-GO closes under §3.8.
  - **The program checkpoint** records the acceptance.
- **Superseded text stays readable** in both documents.
- **No authorization.** This amendment authorizes no implementation, commit, tag, push or publish.

## 6. The core session contract

The contract stays the design for the session host, which [GwzCoreSessionPlan.md](../../dev-docs/GwzCoreSessionPlan.md) schedules after 1.1.0. 1.1.0 builds two pieces that the contract also describes:
- the token-taking part of §5.2's transport entry (S6.1), without its session-context environment and timeouts;
- §14's supersession of the long-lived session (S6.2).

The authority for both is this amended plan and S1.1's reviewed revision, not the contract, whose status grants no implementation authority. A later change to the contract's §5.2 does not amend S6.1. The session host's workers later call these same pieces, so 1.1.0's work is the session program's first step, not a detour.

## Changelog

- 2026-09-27: §3.1, §3.2, §3.4, §3.7–§3.9, §4, §6 and §3.5's sentence on gwz-py's notes superseded by [`GwzTransportReleasePlan.md`](GwzTransportReleasePlan.md), accepted at SHA-256 `4ec6ba33…`. Its other clauses stay in force as that plan's §4 adopts them.
- 2026-10-01: amended by [`GwzTransportReleasePlanAmendment-2.md`](GwzTransportReleasePlanAmendment-2.md), accepted at SHA-256 `c5850e52…` ([its verdict](GwzTransportReleasePlanAmendment-2-Verdict.md)). Its revision 3 applied the operator's decision OD14, restoring this document's §3.4 (Phase 6, S6.1–S6.3) for 1.1.0's run and applying its §3.5 and §3.6 sentences on gwz-py as written. On the operator's instruction, revision 3 was skim-reviewed only ([skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md), [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-1.md)), with no dual-review GO.
