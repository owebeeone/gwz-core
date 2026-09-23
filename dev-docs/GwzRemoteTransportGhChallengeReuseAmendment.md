# Reuse of a clean gh discovery challenge

Status: **draft; requires Consistency and Safety review before implementation**.
Date: 2026-09-23. This amends only the physical disposition of a completed
anonymous HTTPS discovery `401` or `404` in
`GwzRemoteTransportHttpsDesign.md` §4 and §6. It does not authorize a
release or change the authentication replay count.

## Observed gap

The accepted HTTPS design starts each discovery anonymously and allows one
GET retry through gh after `401` or `404`. The current endpoint discards the
first connection for every non-success HTTP status. A disposable, authenticated
Git HTTP fixture demonstrates that a long-lived Python bridge can clone and
perform two unchanged fetches successfully, yet the authenticated GETs use
three distinct TCP sockets. The first unauthenticated GET of each fetch checks
out the idle socket, receives a challenge, and discards it. The core's new
anonymous fixture passes a reuse regression because it never challenges.

Raw failure and socket trace:
`gwz-core-evidence/campaigns/transport-qualification/runs/2026-09-23-python-native-integration/`
(including `gwz-py-native-https-trace2.log`). This fixture
uses a temporary CA, local Git HTTP server and fake gh helper; it contains no
real account credentials. The actual native extension was loaded, not a mock.

## Proposed correction

For the existing **once-only** anonymous discovery `401`/`404` → Gh transition,
the endpoint may release the first physical HTTPS lease as reusable only after:

1. The whole HTTP response body has been drained within the existing cleanup
   allowance and a fixed 64 KiB response-body cap. Truncation, invalid framing,
   body cap, deadline or cancellation discards it.
2. Hyper reports the sender ready for another request on that connection.
   `Connection: close`, peer closure or sender error discards it.
3. The challenge belongs to the same canonical HTTPS destination being retried;
   gh remains permitted by the endpoint policy. The retry is still a new
   logical stream, with the original request, operation, route and remaining
   cumulative budgets.

This is a narrow exception to §6's “no terminal failure” reuse condition: the
completed anonymous challenge is a Git authorization transition, not a
successful Git advertisement. The first typed failure receipt remains private
and the final Gh result is the only public operation result. A reusable TLS
socket carries no authentication state: gh credentials are looked up afresh
for the next HTTP request, and only that request carries Authorization. No
preemptive gh request, stored token, skipped challenge, redirect relaxation,
POST replay or network-error retry is added.

Every other failed HTTP response, including a Gh `401` and an anonymous
`403`/`5xx`, keeps the existing discard rule. A clean challenge socket may be
reused for its immediate Gh GET; if the latter fails, the failure keeps its
own disposition. The exact same rule applies across sequential operations,
allowing a later anonymous challenge on an idle pooled socket without granting
it credentials until its own once-only transition.

## Verification and compatibility

A causal red regression must use an authenticated local Git HTTP fixture and
a persistent core runtime: clone, fetch twice with no change, assert the
second fetch causes no new TCP/TLS connection, and assert each request still
makes the anonymous challenge and a fresh gh lookup. The actual Python native
extension must show the same result with one bridge. Add adverse cases for
oversized/truncated challenge bodies, server close, sender error, cancellation,
expired cleanup budget and Gh failure. They must discard and preserve the
final error. Existing anonymous, redirect, POST and credential-containment
tests remain green. The 64 KiB cap is only for draining this failed response;
it changes no normal successful body limit.

No Taut field, wire version, public Python API, CLI option, credential source,
transport placement or release order changes. The proposed reuse exception
requires a settled document review before the HTTPS code is changed.
