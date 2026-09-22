# GwzRemoteTransportAlphaTimeoutPlan — remediation 1

Round 1 of the draft plan at SHA-256
`773639e1b545eeba776c3faeac1b0785aef50ee33faa0ba1e984b5ec214cac4a`.
Both axes NO-GO. This plan maps every filed finding to one disposition.
The correction is one revision of
`GwzRemoteTransportAlphaTimeoutPlan.md`, not a patch series.
The implementer does not close findings; the reviewer who raised each one
re-verdicts the original counterexample.

Blind convergence: Consistency P2-1 and Safety P0-1 / P1-1 are the same root.
A scripted `Control` test can pass while production setup never receives a
stall allowance or a wait boundary, so Phase 5 can accept a longer cumulative
budget. That is the highest-confidence defect in the round.

## Dispositions

| ID | Axis | Disposition | Closure test the revision must name |
|---|---|---|---|
| P2-1 | Consistency | Accept. S3.1 names `begin_wait` / `complete_wait`. Idle poll and sleep slices are not completions. New S3.3 wires that rule on the production DNS, TCP, handshake, and agent path and requires a production-graph regression. | Four production-shaped completions each inside the stall allowance, sum beyond it, aggregate ahead, succeed. One stage with no completion past the stall allowance, aggregate ahead, times out and is not reusable. |
| P2-2 | Consistency | Accept. S3.1 adds a deterministic clock seam used by stall and aggregate checks. S3.2 drives that seam and forbids wall-clock sleep. | Advance the fake clock past the stall allowance while the aggregate is still ahead; result is stall timeout; no `thread::sleep`. |
| P2-3 | Consistency | Accept. S4.1 depends on S2.2 and S3.1. Late-result rejection uses the uncoupled aggregate instant and the cancel path. | `io_ms=3000`, `connect_ms=10000`: cancel at 5s drops the result; success after the aggregate instant is dropped; success before the aggregate is accepted. |
| P3-1 | Consistency | Accept. S1.1 quotes the superseded §10 / §10.2 sentences and lists the unchanged §10.1 and §10.2 sentences. | A design-diff checklist: every superseded quote is replaced; every unchanged quote remains; §10.1 Open-shorten is not edited. |
| P0-1 | Safety | Accept, same root as Consistency P2-1. S3.3 is required before S5.2. S5.2 refuses to close on a default cold fetch that only shows `connect_ms=10000`. | Stall wired: one idle stage expires at the stall allowance with the aggregate ahead. Stall unwired: S5.2 fails even if live fetches pass. |
| P1-1 | Safety | Accept, same root as Consistency P2-1. The wait rule is: a wait completes when the socket is ready or the native call returns a finished result. A quantum slice with no readiness does not complete it. S3.3 applies that rule at the production poll sites. | Hung TCP expires as stall while the aggregate is ahead. Four short stages whose sum exceeds the stall allowance succeed. An agent EAGAIN sleep loop past the stall allowance, with no auth progress, is stall timeout. |
| P2-1 | Safety | Accept. Phase 2 states the HTTPS connect-budget change as intentional and bounded by the pool aggregate (10_000 ms). It does not preserve the copied 3-second ceiling, and it cannot grow past `connect_timeout_ms`. S2.2 locks that value and adds a stuck-connect expiry fixture. | Default HTTPS Open has `connect_ms=10000`. A stuck HTTPS connect expires at that aggregate and not later. |
| P2-2 | Safety | Accept. Stall expiry and aggregate expiry stay setup-stage timeouts and carry distinct reasons `stall` and `aggregate`. S5.2 records that reason and does not infer it. | Two fixtures assert different caller-visible reasons: one idle wait, and many short waits whose sum passes the aggregate. |
| P2-3 | Safety | Accept. Interaction spends neither stall nor aggregate. The endpoint forwards pool `begin_interaction` / `end_interaction` to the in-flight setup `Control`, which preserves the remaining stall allowance. | Interaction longer than the stall allowance does not expire the stall. The next network wait after `end_interaction` is still stall-bounded. |
| P3-1 | Safety | Accept. Late aggregate, stall, and cancel rejection is a setup timeout with its reason, not `PermissionDenied`. The install gate sees those three signals. Disabled network timing (`deadline` none) still drops a cancelled result. | Late success after aggregate and after cancel is not idle, `reusable()` is false, and the caller sees a setup timeout. |
