# SSH N3a — local endpoint assembly and per-operation routes

Date: 2026-09-22. Status: accepted local assembly after retained Code/State GO.
Authority: GwzRemoteTransportSshProductionSetup.md N3 and accepted N1/N2b.

This bounded attachment step composes the accepted supervised network/trust,
agent authentication, selected snapshot authentication and shared worker. The
caller supplies owned known-host and optional agent-socket paths; construction
does not read environment, files or connect. All setup I/O remains supervised.
One endpoint serves ambient and selected routes; each route owns its observation
callback. Selected routes always use open_selected, including on reuse, and
cannot fall back to the agent. Ambient routes use only ambient pool authority.
Repository paths and observer closures do not become pool keys.
Each operation creates a fresh native Remote when changing its route/observer:
libgit2 retains its transport across disconnect, so replacing callbacks on a
previously connected Remote does not replace that transport's context. This is
the accepted fork API's existing lifetime rule, not a new API change.

Scope: one internal assembly module and the existing Route adapter, at most 220
new production lines and 500 test/support lines. No new physical-resource owner,
thread class, wire field, public surface or production dependency switch. This
is an aggregate composition checkpoint with retained Code/State dual review.

TDD gates: real libgit2 clone, advertisement, push and fetch through per-remote
callbacks with one selected-key connection across operations/repositories;
distinct route observation sinks; fresh and reused ambient authentication;
selected-file deletion refusing cached reuse without an agent fallback; selected
and ambient authority isolation; trust refusal before agent traffic; complete
endpoint cleanup. Existing isolated SSH suite remains the regression gate.
Native fixtures use temporary trust/keys and loopback peers only. Raw evidence
belongs to the private ssh-integration campaign; builds remain outside it.

## Remaining N3 attachment

N3b must attach monotonic authentication/failure observations to operation sinks,
including failures before Opened, and preserve the endpoint through backend
clones and nested with_transport scopes. N3c must wire and exercise every driver
in GwzRemoteTransportSshWorker.md's call-site map with existing progress and push
callbacks preserved and non-SSH transports unchanged. Endpoint path discovery
belongs to backend ownership, not this constructor or gwz-transport.

The current core Cargo graph still uses registry git2 without the accepted
per-remote callback API. GwzGitLibraryDesign.md requires distribution, consumer
source unification and platform qualification before that production switch.
Those checks remain in the operator-deferred single batch; source integration
may be prepared in the isolated consumer, but N3a does not activate the backend
or declare N3 complete. HTTPS and CLI-hosted placement remain later phases.

## Local results

The full isolated SSH suite passes under Rust 1.95.0, locked/offline. The new
local_endpoint tests drive native clone/push/fetch/advertisement with four
exchanges using one physical selected-key connection and two isolated observers.
They also exercise ambient reuse, unauthorized selected-key refusal with an
ambient connection already pooled, absent-agent refusal despite a pooled selected
connection, deleted-file refusal before selected reuse, no agent fallback and
trust refusal before agent I/O. All endpoints report complete cleanup.

The initial red gate was the absent assembly API. Test development corrected a
close-result type mismatch and an invalid observer-rebinding assumption on one
native Remote; no production policy change was needed. Intermediate failures
and final green are retained in the private run
`ssh-integration/runs/2026-09-22-local-endpoint-n3a` with source hashes.
Scoped rustfmt and whitespace checks pass. No platform/source qualification was
run. This is evidence for endpoint/route assembly, not backend-driver coverage.

## Acceptance

Accepted at root `514f3cfeb233acd4e3f6c9c7e1bc07f17275373a`, core
`dfe76d0fc4a440f04262d2e1a22e40542e051925`, evidence
`e6c9226bf204f9d96b4556878c272e6409636420`, transport
`16a383e7d1c0e7e3234006688986afc2c6e54ca5`; fork pins remain unchanged.
Retained [Code GO](../../dev-docs/GwzRemoteTransportSshN3a-ReviewCode.md) and
[State GO](../../dev-docs/GwzRemoteTransportSshN3a-ReviewState.md), zero findings.
Both independently reran the focused gate. One aggregate round, no remediation,
no blind-convergent or known escaped defect. Test-authoring errors are recorded
above; accepted production behavior required no correction after review.

Production additions120 in two files (seven removed); tests288 in one file.
This acceptance is confined to the stated N3a scope. N3b/N3c and the deferred
activation prerequisites remain. This annotation changes no executable code.
