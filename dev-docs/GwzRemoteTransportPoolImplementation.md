# Endpoint connection pool checkpoint

Status: **accepted at transport `e8b9a1c5408cc9ea9528939b3a602acbeb697814`
after original [Code](../../dev-docs/GwzRemoteTransportPool-ReviewCode-1.md) and
[State](../../dev-docs/GwzRemoteTransportPool-ReviewState-1.md) GO; this accepts
the in-memory pool checkpoint only, 2026-09-19; no API freeze**.
The [acceptance record](../../dev-docs/GwzRemoteTransportPool-Checkpoint.md)
pins the full reviewed tuple and verification evidence.

The operator authorized connection pooling after the accepted in-memory stream
checkpoint. This implements design §7 using fake connections and a controlled
clock, before any network adapter. No physical I/O or CLI/core surface is added.

## Boundary and ownership

The pool owns connection identities, capacity reservations, eligibility,
exclusive leases, wait queues, deadlines and cleanup state. The endpoint host
owns actual connection objects and executes Connect/CancelConnect/Close/Abort
commands outside the pool lock. Completion acknowledgements update pool state.
A cancellation never frees an in-flight reservation until its connector settles;
a late successful connection must close before its capacity is released.
Closing physical resources count until closure is acknowledged.
`idle_closed` reports actual spontaneous disposal of Idle or already-Closing
resources and reschedules waiters. If checkout wins the race, it returns
WrongState without stealing the lease; the host routes I/O failure to that
exchange and completes its discard/cleanup. Foreign or duplicate tokens are stale. Abort is a host
instruction to terminate local resources, not a claim that they already closed.
The host must drive deadlines and promptly execute aborts for bounded shutdown.

Requests carry `Owner { session, operation }`. The session is the fresh binding
ID, never reused by the host. Operations are scoped inside it and may own many
exchanges. `cancel_operation` matches the complete owner; `cancel_session`
matches that session's work across operations. Both retain idle resources.
The host stops routing work from a lost session before cancellation; no permanent
session tombstone registry is created inside the pool.

A deterministic PoolMachine supports custom adapters and tests. An async Pool
facade shares that machine across clones, returns exclusive RAII leases and
uses a dedicated driver wake slot for outgoing host commands. Dropping an
unclaimed checkout cancels it; dropping a lease discards its connection. Healthy
release is explicit and only follows completed stream/backend cleanup. Dropping
the final Pool owner initiates shutdown; driver loss fails pending callers and
requires its host to tear down the physical resources it owns.

## Keys, identity and bounds

A key contains scheme, SSH username, exact configured host and effective port;
repository paths and remote names are absent. Each pool instance is one endpoint
and local-account context. Reuse keys include ports, while the `per_user_host`
capacity grouping does not; HTTPS uses its no-username host bucket. IDs carry
instance scope so stale or foreign tokens
cannot affect another pool. No DNS alias merging is performed.

SSH ambient and explicit identities have separate reuse eligibility within the
same key. Explicit identity requests carry an endpoint-resolved current key
proof, not a file path. The endpoint must resolve and validate that file before
each checkout; a cached connection cannot authorize a missing/changed file.
A connector reports proven reuse identity, or no proof for a one-use connection.
HTTP connections carry no authenticated-account claim; the adapter must apply
anonymous/gh policy independently on every request.

Defaults: eight connections per user/host across ports, eight across a host, 256 across the endpoint,
1,024 outstanding checkout requests, 60-second idle expiry, 30-second allocation
wait, 10-second connect-network budget, 120-second interaction budget and 5-second
cleanup budget. Construction validates finite bounds. Requests may shorten
allocation/connect/interaction budgets. Operation fan-out policy stays outside
the pool and cannot resize it. Completed unclaimed results count toward the
request bound until taken or abandoned.

Waiting requests are ordered. Compatible idle resources are assigned to eligible
waiters before creation/eviction, so an incompatible head cannot prevent other
reuse. Creating and closing consume both capacity domains. If idle incompatibility
prevents allocation, retire an idle victim and wait for closure acknowledgement.
Each waiter can have at most one outstanding eviction victim. Idle time starts
only at healthy release; checking out removes idle state atomically. Old clock
notifications inspect current state and cannot expire a re-leased connection.

Allocation time applies until the connector starts. Connect-network time pauses
during an explicitly signalled, separately bounded helper interaction. No active
stream I/O deadline or physical network operation is implemented by the pool.

## Verification and review

Deterministic tests cover reuse, both capacity domains, identity
compatibility, fair waiting, cancellation before/during/after connection setup,
late completion, closing capacity, expiry/checkout races, interaction time,
owner loss, async wakeups and bounded shutdown. Seeded randomized schedules exercise
an independent fake-resource ledger, exact replay and teardown assertions.
The reference seeded-test pattern is the existing transport/sdax-rs framework.

The pool API is still draft. The original Code and State reviewers accepted
the corrected implementation, verifying all three findings and no new issues. The prior taut generator prototype, test-only core consumer and
physical adapters remain outside this pool checkpoint.

The initial tuple passed the minimum Rust 1.95 suite, Rust 1.96 Clippy/package,
and 50,000 pool lifecycle cases with seed `0x202609195eed`. Independent review
nevertheless found three P2 roots: missing spontaneous idle disposal, ambiguous
cancellation scope and a per-key ceiling where the authority required user/host.
The [merged remediation](../../dev-docs/GwzRemoteTransportPool-RemPlan.md)
corrects all three with regression tests. Generator `gwz-transport-pool-v2`
adds idle-loss and scoped cancellation events and checks user/host counts across
ports; its corrected-tuple evidence is recorded below and in the acceptance record.
No physical network or native Windows execution is claimed. Replays and exact
commands are in the transport README; the acceptance record pins the
reviewed source and verdicts. Generated schema and existing stream sources are
unchanged from the accepted memory baseline.

The driver deadline query is a snapshot. Hosts must drive clock ticks
independently of action arrival (or rearm timers when requests/releases change
deadlines); a pending action future alone is not a timer service. Repeated helper
interactions share the original total interaction allowance. Cleanup deadlines
are preserved when a cancelled connector completes late.

Corrected-tree local verification: 66 tests pass on Rust 1.95, including the
fixed 3,000 stream cases, fixed 2,000 pool cases and seven new regression/async
tests. Rust 1.96 fmt/clippy and the four-artifact regeneration check pass. The
v2 pool campaign passes 50,000 cases with seed `0x202609195eed`: 1,220,602
connects, 13,332 reuses, 204,345 late successes, 623,407 abort actions,
412,385 spontaneous idle disposals and 682,598 session cancellations.
Direct case `0x1234` reproduces on Rust 1.95. The standalone package builds;
its twelve focused regression/async tests pass on Rust 1.95. Both original
reviewers independently verified closure on the committed corrected tuple,
including full tests and separate 10,000-case randomized campaigns.
