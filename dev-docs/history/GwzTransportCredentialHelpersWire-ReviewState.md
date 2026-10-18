# TR2.22 wire detail/retry-count checkpoint — STATE-AXIS REVIEW

**Review object:** TR2.22 wire checkpoint: core `2e64e88a28c332ed422cc390adc76738dc701bb1..26922a1cfd823a4894e09be9aeb05f7f73d21414`, transport `35475977530171ab77ee2fbb1e8128f938acb5ae..9f9f0dc4dd82e6329d6ce53e102a231e214ff673`, including the implementation record and prepared review prompts. Settled for review; acceptance pending. The controlling implementation record remains DRAFT/in progress.

**Baseline:** Root `e489c4f9a1d152dc66dd50a9e3d3af583e31cd62`; gwz-core `26922a1cfd823a4894e09be9aeb05f7f73d21414`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Conclusions use settled sources read with `git show <SHA>:<path>` and the prescribed diffs.

**Date:** 2026-10-03

**Axis:** State machines, interruption legality, retained-result ownership, races, bounded admission and fail-closed behavior. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — zero P0, P1, P2 or P3 findings. This verdict covers the wire checkpoint only.

---

## 0. Evidence base

The review followed workspace `AGENTS_GWZ.md`, both member instruction files, and the dispatched canonical State prompt at `/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/wire-review/State.md`.

Authority inspected:

- Root `AgentProcessRules.md`: settled review, severity, evidence and canonical reviewer requirements; `GwzProcessOptimization.md`, including §8.
- Root `CurrentProgramCheckpoint.md:1–26`, identifying this settled checkpoint and its exclusions.
- Root `GwzTransportHandoff.md` §§6.1 and 6.4.
- Core `GwzTransportCredentialHelpersDesign.md` revision 4: operator-added endpoint retry count, M6/M8 vocabulary, and OQ7(1).
- Core `GwzTransportReleasePlanAmendment-2.md` revision 6: TR2.22 and §3.20’s retained retry semantics.
- Core `GwzTransportCredentialHelpersImplementation.md:1–209`, settlement manifest and prepared review prompts.

Implementation inspected:

- Core `setup_retry.rs:1–160` and `setup_retry/machine.rs:1–263`: classification, final projection, generations, flights, closure and abandonment.
- SSH placement admission, completion and fact retention: `placement_endpoint/admission.rs:165–215,280–325` and `completion.rs:160–235`.
- HTTPS retry admission and completion: `https_endpoint/retry.rs:1–315`; endpoint cancellation and outbound handoff paths.
- Driver consumers in `transport_host/request.rs`, `session/driver/opening.rs` and `session/requests.rs`.
- Transport schema and generated Failure/detail declarations; `scripts/regen.py`’s exact boxing projection.
- Transport `codec.rs`, `codec/failure_detail.rs`, `codec/validate.rs`, `codec/preflight.rs`, generated admission visitors and `budget.rs`.
- Transport stream terminal/Closed construction and incoming mux admission.
- Detail tests, retry-machine tests, changed SSH/HTTPS retry assertions, and configured-helper characterization fixtures.

The prescribed start and end checks both returned the exact tuple above. Both status checks returned only:

```text
gwz-core: ?? dev-docs/GwzRemoteTransportBugReport.md
gwz-transport: clean
```

The protected draft was not read. No peer current-round report was read. No files were written, and no builds or tests were executed during this review.

Recorded evidence was distinguished from independently executed review work. The implementation record reports Rust 1.95 transport suites, seven detail tests, generation checks, focused candidate retry/setup selections and the ordinary core runner passing. It explicitly records two inherited all-target Clippy diagnostics and the two intentionally red configured-helper tests.

The allowed archive-consumer receipt was inspected. It identifies transport revision `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`, archive SHA-256 `6477e9d2a17de2e1a88cf62cb78077f120f061cbe0a867b1a063f0a05d2d7248`, 32 integration tests passing and `exit_code=0`. Root’s checkpoint records Rust 1.95 and the clean archive provenance.

## 2. Invariant analysis

**The projection does not change retry authority.** The semantic addition to `Final` consumes its existing failure and copies its existing attempt/maximum into optional detail. It does not increment an attempt, move a generation, schedule a wake, start a connection or change classification. The retry state machine’s transitions and budgets remain unchanged.

The attempted counterexamples were:

- First retriable exhaustion with `max_retries=0`: final attempt 1 of 1 receives a count because its setup failure classifies as retriable.
- First authentication, trust or interaction refusal: attempt 1 remains without an inferred retry suffix.
- Retriable failure followed by a non-retriable setup failure on attempt 2: the final projection attaches 2 of the endpoint’s maximum, rather than substituting exhaustion.
- Allocation timeout, cancellation or another `Return` outcome: the endpoint publishes that member’s failure without converting it into a counted setup final.
- A stale first-wave retriable failure after key closure: the machine returns the retained closing `Final`; its diagnostic identifies the attempt that closed the key.
- A stale non-retriable setup failure: the existing machine uses that flight’s own attempt. Projection preserves that distinction.

These attacks did not expose a changed transition or invented progress.

**Queued members receive the closing attempt without inheriting another member’s facts.** Both endpoint admission paths project `Decision::Finish(last)`, clearing the closing setup’s facts before publication. SSH `fail_open` and HTTPS `fail` then merge the receiving member’s own retained facts. Completion paths similarly replace final facts with the completing member’s facts before merging. The count remains the endpoint key’s final count; authentication observations remain member-owned.

**The drivers cannot reconstruct missing evidence.** Both production consumers now use `reported_attempt`. Neither substitutes its registration budget or failure cause when detail is absent. Conversion rejects negative, zero, reversed and out-of-range internal fixture counts. Removing the production budget accessor leaves its inspection form under an explicit test boundary.

**Malformed diagnostics refuse before delivery.** Envelope validation checks BindRejected, OpenFailed, Failed, IdentityCheckFailed and Closed’s nested failure. The same validator serves local admission and encoded decoding. Stream terminal and Closed construction, and mux incoming handoff, invoke codec admission before accepting their outcomes.

The closed validator rejects:

- Helper cause together with scheme diagnostics.
- A pipe kind without PipeFailure, PipeFailure without a recognized kind, and arbitrary pipe text.
- More than four schemes, empty tokens, tokens longer than 32 bytes, non-ASCII characters and non-token delimiters.
- Nonpositive counts, `attempt > attempts`, and maxima above `u32::MAX`.

Unknown helper enum values fail generated decoding. Missing/null detail remains absent, while `Some([])` remains a distinct valid no-scheme diagnostic.

**Allocation limits continue to cover the new shape.** Encoded preflight still precedes generic CBOR and typed decoding. The added maps, strings, list entries and count scalars participate in the generated typed admission visitor. Its conservative node charge covers the boxed detail; the boxing projection changes neither tags nor the visitor. Scheme bounds further restrict accepted diagnostic strings.

**No new durable recovery grammar is introduced.** This checkpoint adds runtime diagnostic values and their wire projection. It adds no production filesystem write, journal phase, lock or external mutation. Interruption can prevent a diagnostic from reaching its consumer, but the added field does not authorize replay, heal a key or establish successful authentication. Retained-result and cancellation ownership remain with the existing machines.

## 3. Risks and next action

This was a static read-only review; it did not execute fresh crash, race or malformed-message probes. The passing test results are recorded producer/coordinator evidence, with the archive receipt inspected independently.

The configured-helper runner, challenge/route ownership, message catalog, SSH password parity, native Windows work and platform/performance qualification remain deferred. The inherited all-target Clippy failures remain recorded debt. This GO does not qualify those boundaries or authorize release.

The next action is for root to combine the independently formed required verdicts and, if both pass, record acceptance of this exact wire checkpoint before proceeding to the secret-runner checkpoint.
