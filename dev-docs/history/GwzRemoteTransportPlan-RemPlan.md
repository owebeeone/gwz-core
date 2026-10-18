# Remote transport plan — G46 review remediation

Date: 2026-09-19. Status: **complete; G46 re-review GO, all seven findings
reviewer-closed**. No implementation has started.

The [G46 report](GwzRemoteTransportPlanReview-G46.md) is preserved unchanged.
It is a combined draft-stage review, not a dual peer-blind gate. Its verdict
is NO-GO with four P2 and three P3 findings. All seven corrections are accepted;
none requires changing the accepted transport design. This record describes
the drafter's corrections; the [G46 re-review](GwzRemoteTransportPlanReview-G46-1.md)
supplies reviewer closure. No implementation tests are claimed.

## Review object

- Original plan SHA-256:
  `32d5d12a16287bed035cfb4fda0c2c3cbeb97823cb3337f8c7cc2aead1ec54df`.
- Reviewed [plan](GwzRemoteTransportPlan.md) SHA-256 before acceptance annotations:
  `55120dd1af7b77818eb71fda609818b6c1bb2539a08ec9f025ab01f7d4899b99`.
- Unmodified G46 report SHA-256:
  `f28947411ac4b2937b6df6383584b6b6a5175fa5769a7f3273b19762008633bb`.
- Controlling core baseline remains `1cea3a93bf980a80f14fa032e920befa7ccaa72b`.
  The plan and these review records are working-tree documents, not a newly
  committed interface checkpoint. The original report records its supporting tuple.

## Dispositions and closure checks

| Finding | Disposition and corrected location | Original-reviewer closure check |
|---|---|---|
| P2-1 — freeze versus later tuning | Accept. Phase 1 names schema/tags, generated consumer types, outer framing/routing, full message inventory and hard ingress caps as its freeze set. It records the design's starting numbers and requires finite aggregate budgets before freeze. A table assigns runtime/pool, adapter and GWZ surface gates separately. Phase 6 tunes only construction defaults within the contract/caps. | Trace a lower negotiated payload versus a changed hard cap/type: the former narrows; the latter requires amendment and re-review. Confirm later adapters cannot silently add missing SSH/HTTPS descriptors. |
| P2-2 — pool owner across backend clones | Accept. §2 assigns runtime-installed endpoints and their shared handles, forbids global and per-clone pools, and preserves operation-scoped identity/errors/observations. Phases 2–3 require clone/drop and successive `operation_services()` lifetime tests. | Trace two operation contexts through one endpoint: the second may reuse the first's idle connection and dropping the first clone does not shut the endpoint down. Distinguish backend cloning from stream-handle cloning. |
| P2-3 — incomplete SSH entry coverage | Accept correction (a). Phase 3 has a coverage ledger for bootstrap, clone/advertisement, materialize, fetch, tags, pull, push/verification and local/native compatibility paths. All SSH entries must use the adapter before SSH support is advertised. Phases 4–6 repeat/check coverage for carried SSH and both HTTPS placements. | Attempt to advertise SSH with pull or materialize still native: the gate remains closed. File/local-family stay local; HTTP/git retain their explicit compatibility disposition. Pending ledger rows are not implementation evidence. |
| P2-4 — operation and physical capacity conflated | Accept. Phase 2 separately assigns endpoint construction ceilings (initially eight per user/host and eight aggregate per host) and unchanged per-operation scheduler fan-out. All physical lifecycle states count; requests cannot raise endpoint ceilings. | Trace overlapping operations with different fan-out limits: combined physical count stays bounded and lowering one operation's limit does not resize the pool or evict the other's connections. |
| P3-1 — observation semantics | Accept. Phase 4 forbids copying whole observation rows, distinguishes offered/authenticated/reused, preserves unknown authentication and private-member suppression; Phase 6 rechecks them. | Reused connection does not report a fresh credential offer, unknown authentication remains nullable, and private-member data stays suppressed on result/error paths. |
| P3-2 — timeout ownership and login behavior | Accept. Phase 2 assigns all seven timer domains; Phases 3/5 own adapter integration. Phase 5 explicitly forbids implicit login and requires actionable missing-login errors and separately bounded/cancellable interaction waits. | Helper wait does not consume network timeout; idle expiry does not fail a busy stream; missing gh login cannot start an interactive login workflow. |
| P3-3 — placement Surface gate | Accept. Phase 4 binds Surface review to the option/capability freeze: name, local default, explicit refusal, help and binding lifecycle. Phase 6 validates that surface rather than deferring its review. | Explicit unavailable driver placement cannot become an omitted/local option; runtime install/drop is defined, and any future host command requires matching teardown before exposure. |

The review's residual test notes are also assigned: Phase 2 proves batching
without a Git flush callback; Phase 5 proves EndWrite/body completion and that
Flush or the batching timer cannot end an HTTP request body.

## Validation and next action

Checked each correction against design §§3, 4.2, 6–10 and the live backend
`operation_services()` clone boundary. Documentation validation checks local
links, Markdown fences and whitespace; no code, schema generation, network
experiment or implementation test is part of this revision.

The completed [G46 re-verdict](GwzRemoteTransportPlanReview-G46-1.md) is GO:
all four P2 and three P3 findings closed, no new P0–P3. Verified that its plan
hash matched the working-tree plan before applying acceptance/next-action
annotations; sections 1–5 remain byte-for-byte unchanged. Both reviewer reports
are preserved verbatim. Two combined draft-stage review rounds and one merged
remediation were completed; this is not a dual peer-blind gate. Phase 1 remains
unstarted pending an operator execution request.
