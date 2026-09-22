# HTTPS H2 — host and command integration

Status: **implementation in progress; not accepted or activated**.

Controlling design: [HTTPS design, §8](GwzRemoteTransportHttpsDesign.md#8-integration-and-implementation-gates).
H1 remains the accepted endpoint implementation. H2 connects it to the existing
placement host and request-scoped backend. No public constructors or message
fields change. Physical wire/iroh, activation, release, platform and selected
source qualification remain outside this gate.

## Scope

- Private HTTPS injection alongside SSH in local and carried endpoints. The
  existing registration, mux, request IDs and message attachments own delivery.
- One physical reservation authority for both schemes. Each protocol retains its
  existing pool and cleanup owner.
- Endpoint-owned asynchronous HTTP runtime, with bounded preparation/stream
  admission, message backpressure, fair scheme/stream dispatch and retained
  ownership through cancellation and physical cleanup.
- Git HTTP RPC through per-remote callbacks at the shared backend funnel;
  automatic discovery retry crosses two real mux Opens and shares one remaining
  budget. Route lifetime belongs to the registered request, not an RPC instance.
- All command funnels, Rust/Python in-process embedding, capability mismatch,
  observations, private access refusal and native local HTTP/git compatibility.
- Placement C State P3-1: assert cleanup snapshots and retained-resource retirement.

## Evidence and limitations

Private raw evidence: `gwz-core-evidence/campaigns/https-integration/runs/2026-09-22-h2/`.
Baseline tuple and final source fingerprints are recorded there. Focused gates
are recorded below; the candidate is not yet accepted.
Initial red is a captured compiler failure at the missing host/binding boundaries,
with source hashes; it is not a runtime regression. The small shared-authority
constructor/delegation changes preceded a distinct constructor regression, a TDD
process deviation. Existing disposal tests were extended afterward. First host
build ran out of disk space; its raw output is retained. The failed build's new
working cache was removed. A smaller debug profile allowed execution without deleting the preceding H1
cache. No older cache was removed. Intermediate failed compiler/runtime attempts
do not all have source fingerprints; they are retained without claiming exact
replay of every intermediate tree.

## Implementation findings

Owner testing corrected three production defects before review: opening receipts
must advertise the negotiated limits; cancellation must leave the HTTP owner able
to emit its authoritative terminal (otherwise the mux route leaks); and HTTPS
authentication failure must not enter legacy private-repository suppression.
Observation success now respects explicit owner facts even with dynamic endpoint
IDs. Fixture corrections covered CGI paths, runtime blocking, receive-pack enablement
and Git legitimately abandoning a response after receiving its pack. The added
post-push fixture initially mistook a remote name for a branch selector; it now
updates remote HEAD and verifies both advertised ref and new file contents.

## Validation

- Endpoint gate: 68 passed. Observation3, binding2 and identity6 passed.
- Native default core library check passed.
- Host integration gate: 45 passed, including the corrected pull/push/post-push
  command test and native local HTTP/git compatibility.
- Scoped Rust formatting, conditional-boundary inspection (including disabled
  token branches), diff whitespace and private archive verification passed.
- gwz-transport source is unchanged from accepted H1; its prior full-suite
  evidence remains applicable. No whole-core passing claim.

## Gate

One substantial settled Code/State review with retained reviewers, after the
focused native integration gates pass. Review tuple, counts, reports, remediation
and final acceptance will be recorded here. No production or release claim follows
from candidate tests.
