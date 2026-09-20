# SSH pool and per-remote integration continuation

Date: 2026-09-21. Status: implementation authorized; acceptance pending.
Authority: operator direction to defer platform/source checks and continue SSH
pool/per-remote integration; RemoteTransport Plan Phase 3, accepted stream/pool,
SSH channel, blocking bridge and native per-remote binding checkpoints.

## Deferred qualification batch

Collect the outstanding platform matrix, selected-source distribution checks,
clean reconstruction without sibling repositories, package/installed-consumer
checks and source publication readiness in one later qualification batch.
[Q1](GwzGitLibraryQualification.md) remains its detailed inventory. The batch
must include the final integrated stack rather than repeatedly qualifying each
intermediate implementation. Fix the Q5 State P3-1 diagnostic-retention issue
before reusing that runner for the batch. Existing Q3–Q5 evidence remains valid
within its recorded scope. These checks are deferred, not passed or waived.
They do not block local SSH integration development. Publication/release and
general platform-support claims still need their corresponding evidence.

## Execution sequence and boundaries

1. Connect typed transport messages to the accepted nonblocking SSH channel.
   Validate incoming messages before native side effects, keep bounded mirrors,
   and consume transport bytes only after SSH accepts them. This preserves
   receive-window and flush acknowledgements. Drive reverse traffic and stderr
   independently; distinguish backend I/O from local backpressure. Preserve
   cancellation, EOF, cleanup and timeout ordering.
2. Connect the generic pool ledger to physical SSH ownership. Keep idle
   connections across operation lifetimes; one exclusive channel per lease.
   Acknowledge physical destruction before releasing capacity. Exercise idle
   reaping, limits, cancellation and failed/late completion. Healthy reuse needs
   complete channel cleanup and an eligible authenticated identity.
3. Attach the message stream's blocking adapter through the accepted per-remote
   callback, using the prepared local Rust/C fork in an isolated fixture. Prove
   real Git advertisement/fetch/push and sequential repository reuse over the
   pool. No global transport registration, synthetic scheme, Git subprocess
   fallback or CLI/core interface change. Fixture Git/sshd may act as server or
   oracle; client operations use libgit2 and the message-stream adapter.

Start with the existing injected, already trusted/authenticated session seam.
Production credential/known-host setup and full network-entry routing remain
explicit Phase 3 work; this does not claim that one fixture enables every entry.
No wire transport goes into gwz-transport. Core owns the SSH worker and socket.

## Bounded implementation and review

Scope refinement before the ceiling revision: native connection establishment,
credential negotiation and concurrent native multi-connection stress are excluded
from this checkpoint. Pool races are scripted with fake physical resources; the
native fixture proves sequential composition through one injected authenticated
connection. The bounded fixture worker stays test-only rather than adding a new
production worker owner. This trades the originally larger pool/bridge allowance
for pump cleanup detail and the explicit composition fixture.

Private-to-core modules under `src/git/endpoint`: pump (450 lines), physical
pool owner (250 lines), per-remote bridge (125 lines). Public qualification
fixture under `tests/transport_ssh`: at most 1,200 added Rust test/support lines,
plus small manifest/lock/README wiring. Existing SSH channel/connection APIs
and gwz-transport's frozen APIs remain unchanged unless a concrete counterexample
requires a separately recorded correction. No production dependency activation
or public endpoint capability advertisement is part of these fixture steps.

Tests precede implementation. Deterministic scripted partial reads/writes and
WouldBlock cases cover flow control, cleanup, active-I/O timing and pool
ownership. A local loopback native fixture then proves the composed path.
Run only focused local gates during development; the deferred platform/source
batch is not started here. Review coherent checkpoints with retained Code/State
reviewers; P0–P2 block acceptance, at most two remediation rounds. Freeze any
new public surface separately; implementation-only host seams remain internal.

## Progress

Implemented internal modules: message pump425 lines, pool owner225 lines,
per-remote bridge100 lines (750 production-source lines across three files,
not yet included in production module wiring). Added tests/support1,144 lines.
The independent fixture adds local dependencies on the existing transport and
prepared git2-rs/C checkout; no production manifest or source pin changed.

Local Rust1.95 macOS gate: **21 tests pass**. Native composition executes two
clones, two pushes and a fetch against two bare repositories, verifies object IDs,
and observes **five Git service channels on one physical/authenticated SSH
connection**. Clients use libgit2; fixture Git processes provide only server and
setup/oracle behavior. Scripted tests cover exact deadlines, capacity retention,
late connect cancellation, unsafe reuse refusal, disposal failure/forced cleanup,
partial writes, reverse credit, bounded stderr work, EOF WouldBlock, cancellation,
nonzero service status and per-remote open failures without fallback.

Development found and corrected a fixture API mismatch and the pump's use of
active-I/O reports after entering the distinct close phase. A close regression
also exercises retryable EOF and returning ownership only after Closed emission.
The final native fixture passes with that correction. Private raw evidence:
`gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-local-a/`
(access required); public tests run without this evidence repository.

Acceptance review is pending on the settled implementation. This checkpoint
proves controlled local composition only. Next is production endpoint wiring:
trusted credential setup, URL/identity resolution, bounded worker scheduling and
network-entry coverage. No production endpoint capability is advertised.
