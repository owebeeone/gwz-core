# SSH N3 — aggregate backend attachment

2026-09-22. Implementation in progress. The operator combined the remaining
N3b/N3c work into one batch and one retained Code/State review gate. This
supersedes their separate checkpoint sequencing in N3a; accepted behavior and
the deferred platform/source-distribution batch remain unchanged.

Attach the shared endpoint to backend clones and operation scopes; preserve
isolated observations across invocations and nested materialize/pull responses;
report actual authentication attempts and failures as well as successful reuse;
wire clone/bootstrap, fetch/tags/pull, advertisement, manifest discovery and
push/publication through their common backend funnels. Preserve existing
progress, push negotiation/rejection, and non-SSH/local-family behavior.

Use an isolated full-core candidate build against the accepted fork. The
`gwz_transport_candidate` build configuration is supplied only by its explicit
test harness, on Unix. Ordinary manifests, dependency sources and default routes
remain unchanged. This permits real backend/driver tests without performing the
deferred production source switch. No CLI/core message or physical carrier change.

One shared lazily constructed endpoint belongs to the backend family. A fresh
operation retains that endpoint and starts fresh identity choices/observations;
nested drivers preserve their current response merge rather than duplicating
rows. Every changed route context uses a fresh native Remote. Callback assembly
adds the per-remote transport to existing callbacks instead of replacing them.

Setup progress belongs to the request that initiates a physical connection.
The pool supplies only a read-only opening-connection correlation, never a lease
or resource-access capability. The endpoint associates its bounded per-request
facts cell before starting that connection. Credential offers/rejections are
recorded at native authentication calls; authentication success is published only
after joined live handoff. Reused sessions retain proof without claiming a new
offer. Reports are copied at operation completion; late helpers cannot mutate a
published response. Trust/file/queue failures do not fabricate authentication.

TDD plus aggregate native tests cover backend operations, callback preservation,
selected and ambient failures/reuse, operation isolation and nested reporting,
all network-driver funnels and non-SSH coexistence. Tests use private fixture
keys/trust and loopback peers. Run the isolated SSH/transport regression suites
and relevant full-core tests once the batch settles; retain failures and final
evidence in the private SSH campaign. No intermediate review gates. Review on
one committed tuple; consolidate any findings into one correction.

Bounds: 1,200 added production lines across existing endpoint/backend owners and
small cohesive adapters, 1,600 test/harness lines. No new physical owner, protocol,
public feature, credential policy or replay behavior. Production activation,
platform/source qualification, HTTPS and CLI-hosted placement remain excluded.

The additive `Checkout::opening_connection()` accessor is part of this review:
it exposes correlation only while Opening, never a usable lease, and returns
None for waiting/ready/failed/consumed requests. Existing public methods and wire
types are unchanged. Backend key-availability preflight remains synchronous as
before, preserving whole-operation validation; worker admission still supervises
definitive snapshot acquisition before every checkout. No new boundedness claim
is made for inherited preflight calls.

## Local evidence before aggregate review

Full-core candidate gate: five tests pass, including the sequential driver test
through nested snapshot materialization and workspace clone. Ordinary backend
transport gate: eight pass. Full isolated SSH suite and full transport suite pass.
Authentication observations include concurrent accepted/rejected keys and agent
signing failure; only explicit native refusal maps to authentication rejection.
Arbitrary local PermissionDenied remains a network error.

Implementation-contact failures were retained: missing candidate API/field,
incorrect fixture assumptions (branch bootstrap, selection spelling, Noop fetch,
local snapshot identity), and overbroad PermissionDenied classification. An early
overbroad `candidate_` filter also ran an unrelated finalization tamper test that
failed its expected-crash assertion. The final gate uses the exact transport
candidate module; it is not a claim that the whole core suite passes.

Reproduction: [candidate harness](../tests/transport_backend/README.md).
Raw logs, source hashes and generated manifest/lock are archived under the private
evidence campaign `ssh-integration/runs/2026-09-22-backend-n3` (access required).
