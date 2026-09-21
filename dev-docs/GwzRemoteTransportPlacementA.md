# Endpoint placement — batch A

Status: **DRAFT correction 2; focused retained re-verdicts pending.**
2026-09-22. Implements batch A of the accepted
[placement design](GwzRemoteTransportPlacementDesign.md#8-implementation-batches-and-evidence).
The design acceptance does not imply implementation acceptance.

## Delivered boundary

- Taut adds opt-in `missing_ok` for newly optional fields. Existing optional
  slots remain required where they were before; malformed present values fail.
  Generated empty-message decoders now validate the enclosing map. Rust and
  strict Python codecs have executable compatibility tests. Other generator
  targets reject the new option rather than silently misgenerate it.
- The owner schema implements profile 2: identity-check request/results,
  RepositoryRefused, and optional terminal facts. Bootstrap stays profile 1,
  then chooses the highest shared supported version. Profile 1 cannot carry
  new failure meanings. Closed.facts is the single authority; even equal
  nested Failure.facts is rejected. `fail_terminal` emits typed failure before
  EOF, preserving first-terminal-wins and observation facts.
- `gwz-transport::mux` provides bounded session/request/operation/stream
  correlation, bootstrap cancellation, endpoint checks and async Owner/Port
  handoff over typed messages. Queue charges include decoded allocation and
  request overhead. Cancellation progresses independently of unrelated bulk
  traffic. The host supplies time, message forwarding and endpoint work.
- The isolated core consumer composes the full GWZ schema with the shared owner
  export, generating Rust and Python candidate projections of the frozen tags.
  Normal core/CLI/Python production artifacts and dependencies are unchanged.
  Candidate receiver admission exercises generation invalidation and dispatch
  affinity. It is a qualification fixture, not the production host facade.
- The old SSH proof remains compilable with absent Failure.facts. Its behavior
  and physical adapter implementation are otherwise unchanged.

The transport remains independent of credentials, Git repositories, sockets,
physical framing and executors. No service method or CLI command is added.
The core `transport_host` declarations remain proposed: batch B will implement
that facade, scoped backend enforcement, endpoint path/identity preflight,
physical cleanup containment and all SSH command funnels. A does not implement
that request guard merely by exposing the reusable lower-level Owner/Port.

## Lifecycle and limits

The default mux has 256 total unique request registrations per session (including
retired IDs), 64 active routes, 5-second bootstrap/cleanup limits and a maximum
120-second identity check. IDs are never reused. Each direction has its own
negotiated bounded queue and control reserve. At most 128 async waits are retained;
exhaustion closes the generation and wakes callers. Session replacement is an
explicit host operation, never a hidden retry or replay. The host propagates port
closure, schedules independent monotonic ticks and disposes all endpoint work;
a protocol cancellation receipt alone proves no physical cleanup.

[Transport README](../../gwz-transport/README.md#request-mux-and-application-ports-candidate)
describes the implemented primitives. The [core embedding guide](../docs/TransportPlacement.md)
labels its facade as proposed and now names both forwarding directions and the
Attachment request_id. `tests/mux_async.rs` executes those directions. Owner
profile 2 is distinct from outer `gwz.protocol/v0`.

## Verification and evidence

Private raw evidence, including failures, is in
[ssh-integration/runs/2026-09-22-placement-a](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-22-placement-a/README.md)
(private member access required). Public builds do not depend on that archive.
It records the source hashes, exact commands, runtime/build locations and results.

Final local gates: transport121 passed/two ignored; Taut68 passed; candidate
consumer Rust31/Python28 passed with regeneration verified; existing SSH126
passed/one ignored serially; backend candidate7 passed. Four generator checks
pass, including the immutable retained owner-reader hash.

The mux tests exercise both bootstrap cancellation winners, request isolation,
old-session rejection, duplicate Bound, check timeout, cancellation dispatch,
control progress under queued data, waiter exhaustion, owner/port drop and
in-memory reassembly with seeded random writes/reads and small credit windows.
Retained old Rust reader source and strict Python compatibility fixtures test
ordinary-local omission, malformed values and new-core/old-reader combinations.
The candidate guard tests explicit CLI refusal before dispatch after replacement.

TDD evidence includes causal reds for worker cancellation, duplicate Bound,
trust-owner mismatch, waiter exhaustion and cancellation-control priority.
The first mux build failed on an intermediate schema literal, not a mux
behavioral assertion. The Taut drafter did not capture its original red run;
its generated Rust/Python green tests are real, but no missing red is invented.
This is an explicit process deviation. Protocol development logs contain shared
working-tree intermediate failures and are not acceptance results.

One pre-existing parallel SSH address-fallback test failed on its success
assertion; the complete suite passed serially without a behavioral SSH change.
The failed log is retained; this does not establish a root cause or platform
qualification. The backend candidate gate is scoped to its seven integration
tests, not the full core suite.

## Remaining gates

Batch B supplies the real core host/API/backend integration and compiles the
full guide lifecycle example. Batch C qualifies a supplied real connection;
there is no authorized new carrier. Platform and selected-source checks remain
the operator-deferred single batch. Production capability advertisement,
dependency activation, HTTPS and release are later gates. The pinned Taut source
extension is committed locally; coordinated publication is required before the
updated remote CI recipe can retrieve it. No push, publication or remote CI
success is claimed. The unrelated CLI release-documentation checker debt remains.

## Aggregate review and correction 1

Initial [Code](../../dev-docs/GwzRemoteTransportPlacementA-ReviewCode.md) and
[State](../../dev-docs/GwzRemoteTransportPlacementA-ReviewState.md) independently
found that request finish could discard an admitted terminal reply. Code also
found typed binding rejection was collapsed into closure. The merged
[correction](../../dev-docs/GwzRemoteTransportPlacementA-RemPlan.md) preserves
terminal ownership until port/action handoff, expires stalled handoffs through
closure, and retains/exposes exact BindRejected failures. Unsupported negotiation
and malformed bootstrap now have distinct outcomes. No wire tags changed.

The [Surface](../../dev-docs/GwzRemoteTransportPlacementA-ReviewSurface.md) GO
carried one P3 setup-example gap, addressed by the README construction, bind,
check, finish and disconnect snippet. Its Rust doctest type-checks the example;
this does not compile or implement the proposed batch B facade.

Correction owner gates: 131 transport tests plus one README compile doctest
pass; two extended campaigns remain ignored. Scoped formatting and whitespace
pass. New causal tests cover every terminal family through direct/async ports,
facts, timeout terminals, local cleanup delivery, saturation, repeated cancellation,
shutdown/drop, incompatible versions/limits, malformed Bind and stalled rejection.
Private [correction evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-22-placement-a-rem1/README.md)
retains original failures and final results. Acceptance still requires re-verdicts.

## Correction 2 — bootstrap error domain

Round-one Code closed both prior P2s; State and Surface returned GO. Code found
one changed-range P2: retained BindRejected admitted operation-time errors outside
its frozen negotiation-only domain. [Correction 2](../../dev-docs/GwzRemoteTransportPlacementA-RemPlan-2.md)
centralizes admission to UnsupportedVersion/UnsupportedOperation, Effect::None,
and absent facts. Invalid endpoint limits/identity/capabilities fail locally;
malformed peer rejections close as Protocol without an authoritative failure.
Both permitted outcomes survive async handoff and closure. No schema/API change.

Causal red tests and final outputs are in the private
[round-two evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-22-placement-a-rem2/README.md).
The retained reviewers must verify closure before acceptance.
