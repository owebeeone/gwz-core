# TR2.22 wire detail/retry-count checkpoint — CODE-AXIS REVIEW

**Review object:** TR2.22 wire checkpoint in `/Volumes/projects/limbo/gwz-dev-tr2-22`, including the DRAFT `gwz-core/dev-docs/GwzTransportCredentialHelpersImplementation.md`, settlement manifest and canonical prompt. Settled for review; implementation acceptance and release qualification are not assumed.

**Baseline:** root `e489c4f9a1d152dc66dd50a9e3d3af583e31cd62`; gwz-core `26922a1cfd823a4894e09be9aeb05f7f73d21414`, reviewed against `2e64e88a28c332ed422cc390adc76738dc701bb1`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`, reviewed against `35475977530171ab77ee2fbb1e8128f938acb5ae`. Conclusions use committed sources read with `git show` and exact-range diffs.

**Date:** 2026-10-03

**Axis:** Code — architecture, interface contracts, ownership, call graphs, compatibility and allocation/error paths. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — zero P0, P1 or P2 findings; one nonblocking P3 robustness finding.

---

## 0. Evidence base

Start and end verification returned the exact three SHAs above. Transport status was clean at both checks. Core status contained only the permitted untracked `dev-docs/GwzRemoteTransportBugReport.md`; its contents were not read.

Authority inspected:

- Workspace `AGENTS_GWZ.md` and both member `AGENTS.md` files.
- Root `AgentProcessRules.md`, including the baseline, review and severity rules; `GwzProcessOptimization.md` §8.
- Root `CurrentProgramCheckpoint.md`’s settled TR2.22 record and `GwzTransportHandoff.md` §§6.1/6.4.
- Core credential-helper design revision 4, including §11’s M6/M8 vocabulary and OQ7(1); release amendment 2 revision 6, including §3.20.
- Core implementation record in full, settlement manifest and canonical Code prompt. The dispatched prompt’s exact tuple controls over the committed, explicitly undispatched placeholder prompt.

Source inspected:

- Transport authored schema lines 19–61 and the complete generated IR delta; generated `protocol.rs`’s new enum/messages and Failure encoding/decoding.
- Transport `scripts/regen.py:63–122`, regenerator tests, `admission.rs:94–159`, `budget.rs:21–104`, `codec.rs:33–95`, `codec/preflight.rs:14–125`, `codec/validate.rs:104–143`, and `codec/failure_detail.rs:1–86`.
- All seven new detail tests; retained-reader implementation `tests/fixtures/retained_protocol_v1.rs:516–533` and compatibility test `tests/placement_v2.rs:282–348`.
- Core `setup_retry.rs:24–143`, `setup_retry/machine.rs:1–243`, both SSH final-result boundaries and both HTTPS final-result boundaries.
- Both count-consuming adapters and their error displays: SSH driver opening, driver `failure_io`/`SshOpenFailure`, and HTTPS request `HttpsOpenFailure`.
- Remaining source/test diffs, including mechanical Failure literals, characterization fixtures and the checker inventory addition.
- Transport terminal failure path `src/stream/machine.rs:262–280` and its public asynchronous wrapper.

No builds, tests, generation, edits or Git mutations were performed by this reviewer. Executed results were read from the implementation record. The allowed archive receipt confirms source revision `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`, archive SHA-256 `6477e9d2a17de2e1a88cf62cb78077f120f061cbe0a867b1a063f0a05d2d7248`, 32 passing consumer tests, and exit code 0. Rust 1.95 selection is confirmed by the coordinator’s dispatch and settlement record.

## 1. Findings

### [P3-1] Terminal publication clones rejected detail before local admission

**Location:** gwz-transport `src/stream/machine.rs:267–270`, together with the newly extended derived `Clone` implementation of `Failure`/`FailureDetail` in `src/protocol.rs`.

**Violated invariant:** A bounded local publication path should validate caller-owned diagnostic metadata before making additional copies of it. Boxing reduces the common stack layout but does not bound a deep clone.

**Reproduction:** Construct an endpoint-side live `StreamMachine`. Preconstruct a Failure whose detail contains `schemes: Some(vec!["x".repeat(large_size)])`, then call `fail_terminal(failure)`. Line 268 clones the Failure, Box, vector and oversized string. Only afterward does `admit_limited` reject the token for exceeding 32 bytes.

**Impact:** Invalid local diagnostic metadata incurs an allocation proportional to its rejected contents before returning `Error::Protocol`. The clone ordering predates this checkpoint, but the new detail extends that ordering to another caller-owned, dynamically sized payload. This is a local typed-caller robustness defect; encoded ingress still checks its generic allocation bounds before decoding. No credential exposure or externally reachable wire bypass is demonstrated.

**Required correction:** Move the owned Failure into the temporary Envelope, admit that Envelope, then borrow the admitted Failure to obtain its code/effect and copy any retained facts. Avoid cloning detail before admission.

**Closure/regression test:** With fixture construction excluded from measurement, call `fail_terminal` using an oversized diagnostic string. Require `Error::Protocol`, no terminal publication or stream failure transition, and no allocation proportional to the rejected string. Also verify that a valid detail survives terminal publication unchanged.

## 2. Invariant analysis

**Additive wire compatibility holds.** Failure’s existing tags 1–4 and enum values remain unchanged. Detail occupies tag 5. The new decoder accepts missing or null detail; the retained decoder reads its recognized Failure tags without rejecting additive fields. Older writer bytes therefore remain readable. The retained compatibility test exercises null detail; populated-detail compatibility is additionally supported by the inspected retained decoder’s lookup behavior, rather than a newly executed test.

**The admitted detail vocabulary is closed and bounded.** Helper causes have exactly seven fixed wire values. Unknown enum values fail typed decoding. PipeFailure requires a whitelisted pipe kind; another cause or no cause cannot carry one. Helper cause and schemes cannot coexist. Scheme entries must be nonempty ASCII HTTP tokens, at most four entries and 32 bytes each. Retry counts require `1 <= attempt <= attempts <= u32::MAX`. Empty scheme lists remain distinguishable from absent scheme diagnostics.

**All Failure carriers reach validation.** The envelope validator covers BindRejected, OpenFailed, Failed, IdentityCheckFailed and nested Closed.failure. Local admission and encoded decode both invoke it. Existing body-selection, effect/facts and version restrictions remain intact.

**Generic allocation accounting holds for the codec paths.** Encoded preflight precedes both the generic CBOR tree and typed decoding. Generated admission walks the added nested maps, optional values, strings and lists. The conservative per-node charge covers the bounded boxed detail. The generator’s exact two-replacement projection fails closed on representation drift and does not alter schema tags or the admission walk. P3-1 concerns an earlier copy in the separate terminal-publication API.

**Retry counts originate at the endpoint.** `Final::wire_failure` projects the machine’s attempt and maximum. Both immediate-final and retained-key-final boundaries use that projection for SSH and HTTPS. Their existing facts replacement/merge behavior remains intact. Retriable first-attempt exhaustion reports 1 of 1; later terminal setup failures report their known attempt. First authentication/trust refusals retain the documented display behavior.

**Driver inference is removed.** Both adapters use `reported_attempt`; neither substitutes a local retry budget for an absent count. Checked conversions reject malformed internal counts. Retry classification, state transitions, generation handling and successful-setup resets are unchanged by the diff.

**Scope and compile shape are consistent.** Mechanical `detail: None` additions preserve existing constructions. The clean archive-consumer receipt supports the same-build API update. Public GWZ request/response schemas are unchanged. The two configured-gh tests remain explicitly red characterization of the next checkpoint; this GO makes no helper-runner completion claim.

## 3. Risks and next action

The recorded all-target transport Clippy failures remain inherited, identified debt. This review did not independently execute the recorded gates or qualify deferred platforms. Helper producers, message rendering, route ownership and SSH password parity require their separate checkpoints.

The coordinator should file this Code GO with P3-1 tracked and combine it with the independently required State verdict before accepting the wire checkpoint.
