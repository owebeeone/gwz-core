# HTTPS H2 — host and command integration

Status: **correction 2 in progress after Code re-review NO-GO and State GO; not accepted or activated**.

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

## Aggregate review and correction 1

[Code](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewCode.md) and
[State](../../dev-docs/GwzRemoteTransportHttpsH2-ReviewState.md) found three blocking
roots: causal retry budgets/receipts (blind convergence), canceled queued opening
publication, and pre-send effect classification. The consolidated
[correction plan](../../dev-docs/GwzRemoteTransportHttpsH2-RemPlan-1.md) governs
closure; no finding is yet self-closed.

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
