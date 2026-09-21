# SSH agent A1 — bounded client and helper lifecycle

Date: 2026-09-21. Status: accepted after retained Code/State GO on remediation 1.
Authority: [accepted helper design](GwzRemoteTransportSshAgentDesign.md), A1.
This checkpoint does not authenticate SSH, activate production modules, modify
gwz-transport, or change the CLI/core API. A2 signing and A3 pool/backend
integration remain separate gates. Platform/source qualification remains the
operator-deferred batch.

## Implemented boundary

Three internal source files are compiled by the isolated transport_ssh fixture:

- agent_client.rs: bounded identity-list/sign protocol, checked parsing, one
  enumeration, signature algorithm/shape validation, explicit RSA SHA-2 flags,
  redacted failures, irreversible failure state and partial I/O handling.
- agent_socket.rs: owned nonblocking Unix socket created before connect, bounded
  poll waits, cancellation/deadline checks, connect completion checks and bounded
  backlog retry. No opaque ssh2::Agent calls. No Windows adapter is claimed.
- agent_job.rs: private generic setup job, monotonic cancellation, joined result
  transfer, one process-wide supervisor and 64 charged helper slots. The result
  type must have bounded, non-panicking destruction; A2 must prove its concrete
  native-owner implementation satisfies that precondition.

The helper closure owns its local I/O and returns one result; the supervisor
joins only a finished thread. Cancellation wins over unclaimed success. Results
are destroyed outside arbitration locks before disposal becomes observable.
Late/uncooperative work remains charged after caller loss; cleanup overrun returns
an error while the supervisor retains ownership and eventually joins/disposes it.
A3 must map that error to endpoint refusal/observable shutdown failure and keep
physical pool capacity charged; this standalone helper is not that integration.
A helper that has already transferred success releases only its setup slot;
the receiving owner remains responsible for the native connection.

Each fresh Arc-owned completion cell is its own unforgeable lifetime identity;
no reused integer slot or routable job ID is exposed. This is the concrete
realization of the design's generation/stale-completion invariant: old controls
can only reference the old cell, never a replacement job. It refines the private
ID representation without changing that invariant or introducing a wire ID.

The supervisor sleeps when no jobs exist and polls retained work at 20 ms.
Native socket cancellation uses the specified bounded polling fallback rather
than an extra wake descriptor. No real-time scheduling guarantee is claimed.
The internal start_with seam allows deterministic spawn failure; production start
always uses the standard thread builder. No caller can adjust the global cap.

## Local evidence

Rust 1.95 offline locked fixture gate passed 54 test executions, including 16 new
A1 tests and all retained stream/pool/native Git composition regressions. The
existing ignored fake-agent child is executed by its passing parent.

A1 exercises 64 reproducible fragmentation seeds, exact reconstructed request
bytes and decoded responses, malformed/truncated/oversized replies, signature
algorithm mismatch and RSA flags, oversized input before effects, poisoned-agent
refusal, partial write stalls, Unix partial header/body/no-reply cancellation,
absolute timeout, unavailable agent, helper panic, expired-before-start refusal,
late success disposal, cleanup-waiter wakeup, injected spawn failure and global
capacity through abandoned helpers and eventual reap. Native socket peers verify
EOF after cancellation; returning a timeout alone is not counted as cleanup.
Use GWZ_AGENT_SEED=<seed> to replay the printed seed and its following 63 cases.

On this Mac, a filled Unix listen backlog returns ConnectionRefused; the test
records bounded refusal, not an executed native EINPROGRESS cancellation path.
The code covers pending-connect waits, but their platform-specific execution
remains part of the deferred batch. Scripted partial-write waiting complements
native stalled-reply tests; it does not claim a native send-buffer exhaustion run.
No user SSH agent, key, configuration or remote host is involved in A1 tests.

Development failures preserved: missing implementation red, missing cleanup
wake, trailing failure bytes, expired callbacks executing effects, incorrect
native backlog assumptions, and test cancellation racing before helper entry.
The last case was corrected with explicit started barriers; product correctly
refuses callbacks already cancelled before entry. No known escaped defect.

557 production lines across three files (within the design's 20% allowance);
742 test lines across two files. Fixture-only dependencies add socket2 0.6.4 and
explicit libc; production dependencies remain inactive. No new public surface.
Aggregate review uses retained Code/State axes on one settled tuple; P0–P2 block
and at most two merged remediation rounds apply.

Raw red/green logs and final source hashes are in
[agent-a1](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-agent-a1/README.md)
(private archive). Build and runtime outputs remain outside the archive.

## Remediation 1

Code P2-1: channel ownership is sealed; tests observe shared fake-channel records
without extracting and resetting protocol state. State P2-1: a transient initial
supervisor spawn failure is retryable; serialized publication admits exactly one
successful supervisor. Code P3-1: deterministic barriers cover publication before
thread exit and joined completion before claim, plus transferred-owner lifetime.
The wrapper spawner holds the helper after publication; the completion waker
observes join without claiming success. No new production lifecycle hook is needed.

The test ceiling is refined from 700 to 750 lines for these review-requested
regressions (742 actual); production remains within the existing 20% allowance.
This supersedes only the A1 test line ceiling in design §8. No capability or scope
expansion. Two independent P2 findings, one P3 coverage finding, one merged
remediation; no blind convergence and no known escaped defect. Both retained reviewers verified closure. [Remediation evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-agent-a1-rem-1/README.md)
is private; original development evidence remains unchanged.

## Acceptance record

Accepted implementation tuple: root `d552bbda5c5b8c14291243cdf73b1c1955443222`,
core `14409399bc7404446200192ffaf585f9969eec49`, evidence
`d5605a5ad81feff445d0d712940ba050c849dec3`; transport/Rust/C pins unchanged
and listed in the reports. Retained [Code GO](../../dev-docs/GwzRemoteTransportSshAgentA1-ReviewCode-1.md)
and [State GO](../../dev-docs/GwzRemoteTransportSshAgentA1-ReviewState-1.md)
close both P2s and the P3 after one merged remediation. This accepts A1 only.
Native signing and authenticated session handoff are the next A2 checkpoint;
A3 pool/backend integration and deferred platform/source qualification remain.
Acceptance filing changes documentation only; reviewed implementation bytes are
unchanged. No push, release or production activation is part of this acceptance.
