# HTTPS H2 — host and command integration

Status: **accepted candidate at the exact tuple below after retained Code/State GO; not activated**.

Controlling design: [HTTPS design, §8](GwzRemoteTransportHttpsDesign.md#8-integration-and-implementation-gates).
H1 remains the accepted endpoint implementation. H2 connects it to the existing
placement host and request-scoped backend. No public constructors or message
fields change. Physical wire/iroh, activation, release, platform and selected
source qualification remain outside this gate.

The sections before Acceptance retain the implementation and review chronology;
the final Acceptance section states the current result.

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
were recorded below before review; final acceptance is recorded at the end.
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

## Aggregate review and correction 1

[Code](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewCode.md) and
[State](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewState.md) found three blocking
roots: causal retry budgets/receipts (blind convergence), canceled queued opening
publication, and pre-send effect classification. The consolidated
[correction plan](../../dev-docs/GwzRemoteTransportHttpsH2-RemPlan-1.md) governs
closure; all closures were subsequently verified by the retained reviewers.

Automatic opening transitions are serialized per canonical request/URL; stream
exchange remains concurrent. The backend helper mode is fixed for that registered
route. Private direct callers changing explicit policy must register a fresh
request. The endpoint treats the next same-key Gh after a qualifying Anonymous
failure as its continuation; an expired/exhausted budget never silently refills.
This uses the existing protocol shape and adds no public constructor or field.

Correction-1 gates: host50, endpoint69, observation3, binding2, ordinary core
library check, scoped formatting and disabled-branch conditional checks pass.
Private evidence is in the H2 run's `correction-1/` directory. The final host
gate includes actual mux WouldBlock/cancellation handoff, retained first receipt,
fixed explicit policy, same-route concurrency, and pre-send receive-pack effect
regressions. Cleanup expenditure is deducted from the carried retry budget, and
all exhausted domains fail before new helper work. Retained re-verdicts closed
all original findings: [Code](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewCode-1.md)
and [State GO](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewState-1.md).
Code found one new P2 architectural root in gate allocation-time accounting;
[correction 2](../../dev-docs/GwzRemoteTransportHttpsH2-RemPlan-2.md) is in progress.
No acceptance or activation is claimed.


## Correction 2 — allocation accounting

The canonical-route gate and first session admission now share one allocation
deadline. After taking the session mutex, the first Open carries only its positive
remaining milliseconds. Expired or sub-millisecond allowance fails before Open.
The endpoint retains the remainder across the Gh continuation; helper, connect,
network and cleanup domains remain independent.

The runtime red reproduced 1.624 seconds for a 1-second admission budget. The
corrected real one-slot pool test requires gate plus pool waiting within that
original budget (with scheduling margin), observes the reduced admitted Open
allowance, and verifies exhausted gates issue no Open. Host52 and the default
library check pass, as do scoped formatting and conditional-boundary checks.
Private evidence: the H2 run's `correction-2/` directory, including per-command
source fingerprints and raw red/green output. Retained re-verdict remains required.

Final correction-2 focused gates: host52, endpoint69, observations3, binding2,
default library check, scoped formatting and conditional-boundary checks pass.


## Acceptance — 2026-09-22

[Code GO](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewCode-2.md) and
[State GO](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewState-2.md) accept H2 at:

| Repository | Reviewed revision |
|---|---|
| workspace root | `2380a234bf620bacf73a1924f4ac23000385f758` |
| gwz-core | `c92abc4110fc7c1ef89600118284724c942f8985` |
| gwz-transport | `aa40936d0805e8cb60f8027615abe20d4f2045e4` |
| taut | `bcf98b64d465fc54841121b6d1a2d46940f81a3c` |
| gwz-cli | `7db07bbdefd2897c07fd0f9e550bf032bd8b1314` |
| gwz-py | `d07d55dacb1725d9306be9c04d157ac29a78e000` |
| git2-rs | `ce78628308e11b4e8901d5061602619109bce21a` |
| libgit2 | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |
| gwz-core-evidence (private) | `3302b5d03f56590a6d521b1b52db302861775e15` |

Annotation commits do not expand this implementation. This acceptance covers
local and carried in-process HTTPS integration, command funnels, Rust/Python
embedding, shared physical capacity, truthful observations and cleanup accounting.
Placement C State P3-1 is closed by the embedding cleanup snapshots plus gated
physical-disposal regression verified in the aggregate review. No open H2 finding.

One aggregate review and two consolidated corrections. Initial Code2P2/State2P2
shared the retry-correlation root (three unique blocking roots). Correction-1
Code review found one new architectural allocation-accounting root; correction 2
closed it. All closures are reviewer-verified. Implementation-stage defects,
fixture corrections and evidence limitations remain recorded above. No known
released escaped defect; no end-to-end wall-clock/session measurement was captured.

Baseline-to-accepted diff: production-bearing Rust files (including inline tests)
+1541/-130 across 12 files; separate test/fixture files +2571/-24 across 12 files.
These are file-classified counts, not claims of production-only source lines.
Final gates: host52, endpoint69, observations3, binding2, default core library,
scoped formatting/conditional inspection and archive verification pass.

Next: Phase 6 aggregate local performance/readiness work. Keep platform and
selected-source qualification as one operator-deferred batch. Public construction,
production activation, physical wire/iroh, real-account qualification and release
remain separately gated; this candidate does not advertise those capabilities.
