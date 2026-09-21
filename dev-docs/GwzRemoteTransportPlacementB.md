# Endpoint placement — batch B

Status: **accepted at the correction-1 tuple below after retained Code, State
and Surface GO; this accepts the Placement B candidate only.** 2026-09-22.

[Code GO](../../dev-docs/GwzRemoteTransportPlacementB-ReviewCode-1.md),
[State GO](../../dev-docs/GwzRemoteTransportPlacementB-ReviewState-1.md) and
[Surface GO](../../dev-docs/GwzRemoteTransportPlacementB-ReviewSurface-1.md)
close all initial findings after one consolidated correction. Annotation commits
do not expand the reviewed implementation.

| Repository | Accepted revision |
|---|---|
| . | `93334058352828b1069b198d795c5860a395dc81` |
| gwz-core | `4f06384397a67d3dcae4856a93fd032499fda5dc` |
| gwz-transport | `03d3011b3ae9b8205bcf07f7f7862194af114856` |
| taut | `bcf98b64d465fc54841121b6d1a2d46940f81a3c` |
| gwz-cli | `7db07bbdefd2897c07fd0f9e550bf032bd8b1314` |
| gwz-py | `d07d55dacb1725d9306be9c04d157ac29a78e000` |
| git2-rs | `ce78628308e11b4e8901d5061602619109bce21a` |
| libgit2 | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |
| gwz-core-evidence | `a2180f71f9f4f16ecc639eecd25125b980ea54f3` |

Implements batch B of [the accepted placement design](GwzRemoteTransportPlacementDesign.md).
Batch A is accepted; production activation, real supplied-carrier qualification,
HTTPS and the operator-deferred platform/selected-source checks remain separate.

## Intended aggregate boundary

- Candidate full-core build selects the already accepted shared generated schema
  and owner CBOR types. Normal production artifacts/manifests remain unchanged.
- TransportRuntime, CliEndpoint, request owners and application ports implement
  the embedding guide. In-process local delivery and externally supplied client
  delivery share the same SSH endpoint and scoped backend path.
- Request/operation/metadata binding prevents a scope being repurposed. All N3
  network entry funnels preserve whole-operation route and identity preflight.
- CLI-selected identity paths stay opaque in core. Endpoint checks and actual
  opens use endpoint-local home/path/credential context; no native fallback.
- Independent bounded supervisors drive message/SSH progress while Git blocks.
  Terminal dispositions and facts, including repository refusal, cross the shared
  protocol. Cancellation, last-owner drop and explicit cleanup retain outstanding
  work without claiming peer cleanup or successful Git completion.

## Verification and limits

One aggregate Code/State gate using the retained reviewers, after a settled
checkpoint. Cover bootstrap cancellation/late Bound, malformed/unregistered
messages, concurrent requests and many streams under one request, last-target
preflight failure before mutation, distinct credential contexts, connection reuse,
all N3 command funnels, canonical refusal and private omission, selected-file
change after check, active stream cancellation and shutdown. Compile the full
embedding guide against this candidate. Preserve failed attempts and final source
hashes in the private SSH integration evidence campaign.

Local gates pass: host23 (including 50 concurrent streams), existing N3 backend7,
SSH134/one ignored, preparation4, candidate regeneration, exact guide compilation
and default production library check. No whole-core or cross-platform pass claim.
The 50-stream fixture exceeds the ordinary per-host fan-out of eight and verifies
bounded setup-job queuing. Endpoint timeouts may tighten configured policy;
physical cleanup remains accounted after logical timeout/cancellation.

The preflight test directly invokes backend preflight before any target; it is
not a fault injected inside every ordinary handler. Ordinary command tests cover
init/fetch/materialize, push/post-push, snapshot nested materialization, pull-head,
workspace clone and private-member omission. Local repo sync retains its existing
non-network policy. The exact embedding example compiles as a fixture.

Raw failures, final commands and source/input hashes are in the
[private evidence run](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-22-placement-b-integration/README.md)
(access required). Tests authored before implementation did not all capture an
initial causal red; this is a TDD process deviation. Captured intermediate failures
remain archived. These initial gates preceded the review and correction recorded below; no production activation is claimed.


## Correction 1 — accepted after retained re-review

Initial Code/State reviews found four P2 findings across three roots: both axes
found missing effective-identity preflight in ordinary fetch; Code additionally
found identity-check timeout waiting for physical disposal; State found deadline
aggregation before endpoint policy checks. Surface reported GO without findings.
[The consolidated correction](../../dev-docs/GwzRemoteTransportPlacementB-RemPlan.md)
addresses all three without changing the public API or shared protocol.

Causal reds were captured for the ordinary two-member fetch, a FIFO without a
writer, an injected physically blocked check, and the supervisor overflow with
maximum admitted deadline values. Final gates: host25, SSH137/one ignored,
existing fetch9 and N3 backend7 pass; production library check and scoped formatting
pass. The maximum-deadline regression traverses a bound session, wakes its blocking
caller and proves subsequent request progress; the direct endpoint companion
asserts no queued/native work. Check timeout produces one terminal while the job
stays charged until physical disposal. Fetch preserves both tracking refs and
returns no attempt observations when its last configured identity is unavailable.

This adds ordinary-handler coverage missing in the initial test, superseding the
initial preflight coverage limitation above. Candidate/platform/release boundaries
remain unchanged. Private raw evidence: `2026-09-22-placement-b-rem1` in the SSH
integration campaign. All three retained reviewers reported GO on this tuple; no open B findings remain.


Review metrics: one aggregate three-axis gate, one consolidated correction.
Initial Code twoP2 and State twoP2 represent three roots, with independent fetch
preflight convergence. No new re-review findings or known released escaped defect.
Wall time was not captured. Automated path classification from the accepted A
annotation baseline counts implementation3397 added/65 deleted lines across36
files; tests/harness2628 added across17 files, documentation separately. No
repository-wide conditional-compilation migration is claimed.

Next is Placement C qualification using a host-supplied message connection.
Constructing a new physical carrier/interface is not authorized by this package.
Keep platform and selected-source qualification together in the deferred batch;
HTTPS, production activation and release remain separate gates.
