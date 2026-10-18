# Reuse of a clean gh discovery challenge

Status: **accepted as a design at root `78775e818b8a6398aad0859aa46a6662440ab95a`
and core `e416faa1b23b578b5aa50eadea64a11c6561fdf9` after
[Consistency-1](GwzRemoteTransportGhChallengeReuseAmendment-ReviewConsistency-1.md)
and [Safety-1](GwzRemoteTransportGhChallengeReuseAmendment-ReviewSafety-1.md)
reported GO; this accepts the document only, not implementation**.
Date: 2026-09-23. This amends only the physical disposition of a completed
anonymous HTTPS discovery `401` or `404` in
`GwzRemoteTransportHttpsDesign.md` §§4, 6 and 7. It does not authorize a
release or change the authentication replay count.

## Observed gap

The accepted HTTPS design starts each discovery anonymously and allows one
GET retry through gh after `401` or `404`. The current endpoint discards the
first connection for every non-success HTTP status. A disposable, authenticated
Git HTTP fixture demonstrates that a long-lived Python bridge can clone and
perform two unchanged fetches successfully, yet the authenticated GETs use
three distinct TCP sockets. The first unauthenticated GET of each fetch checks
out the idle socket, receives a challenge, and discards it. The core's new
anonymous fixture passes a reuse regression because it never challenges. The
archived fixture records physical connection IDs only after Authorization
succeeds; it demonstrates authenticated-request socket churn, but does not
itself establish the identity of each preceding anonymous challenge socket.

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

This explicitly supersedes both §6's “no terminal failure” reuse condition and
§7's “All terminal non-success responses discard the connection” sentence for
only the clean anonymous discovery `401`/`404` taking §4's immediate once-only
Gh transition. The bounded 64 KiB drain is the sole exception to §7's rule
against consuming an unbounded error body to earn reuse; it grants no Git
success. The completed anonymous challenge is a Git authorization transition,
not a successful Git advertisement. The first typed failure receipt remains private
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

| Response and transition state | Physical disposition |
|---|---|
| Anonymous discovery `401`/`404`, immediate §4 Gh transition allowed, and the complete bounded drain, framing, sender readiness, keep-alive, destination, budget and cancellation checks above all pass | Release the first lease reusable for the new logical Gh GET. No success is reported for the challenge. |
| Same qualifying status and transition, but any reuse check fails | Discard the first lease. §4 alone governs whether the Gh GET can still start with the remaining budgets; a terminal cancellation, deadline or protocol failure retains its own result and permits no new request. |
| Anonymous discovery `401`/`404` with no permitted §4 transition | Discard the lease and report the original refusal. |
| Every other terminal non-success response, including a Gh failure, anonymous `403`/`5xx`, POST failure, or an unvalidated redirect | Discard the lease under unchanged §7 rules. |
| Successful Git response or validated redirect | Unchanged §6/§7 behavior; this amendment adds no reuse right. |

## Verification and compatibility

A causal red regression must use an authenticated local Git HTTP fixture and
a persistent core runtime: clone, fetch twice with no change. The fixture
records every request before the authorization branch, with ordered anonymous
or Gh policy, HTTP status, and an ID assigned at physical TCP/TLS connection
accept; it also counts every gh invocation without recording credentials.
For every qualifying challenge, assert the anonymous `401`/`404` and its
immediate Gh GET use the **same** physical ID, including when another idle
connection is available. Assert that each eligible read operation starts
discovery anonymously, takes only its allowed Gh transition with a fresh gh
lookup, and that the second fetch opens no new TCP/TLS
connection. The actual Python native extension must show the same ordered
identity and lookup results with one bridge. Add adverse cases for
oversized/truncated challenge bodies, server close, sender error, cancellation,
expired cleanup budget and Gh failure. They must discard and preserve the
final error; where another request occurs, it must use a different physical
connection after a required discard, or there must be no retry. Existing
anonymous, redirect, POST and credential-containment
tests remain green. The 64 KiB cap is only for draining this failed response;
it changes no normal successful body limit.

No Taut field, wire version, public Python API, CLI option, credential source,
transport placement or release order changes. The proposed reuse exception
requires a settled document review before the HTTPS code is changed.
