# Endpoint connection pool checkpoint

Status: **implemented, awaiting settled Code/State review, 2026-09-19; no API freeze**.

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
Closing physical resources count until closure is acknowledged. Abort is a host
instruction to terminate local resources, not a claim that they already closed.
The host must drive deadlines and promptly execute aborts for bounded shutdown.

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
and local-account context. IDs carry instance scope so stale or foreign tokens
cannot affect another pool. No DNS alias merging is performed.

SSH ambient and explicit identities have separate reuse eligibility within the
same key. Explicit identity requests carry an endpoint-resolved current key
proof, not a file path. The endpoint must resolve and validate that file before
each checkout; a cached connection cannot authorize a missing/changed file.
A connector reports proven reuse identity, or no proof for a one-use connection.
HTTP connections carry no authenticated-account claim; the adapter must apply
anonymous/gh policy independently on every request.

Defaults: eight connections per key, eight across a host, 256 across the endpoint,
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

The pool API is still draft. Use the original Code and State reviewers for a
settled implementation checkpoint; record their exact tuple and verdicts before
acceptance. The prior taut generator prototype, test-only core consumer and
physical adapters remain outside this pool checkpoint.

Executed locally on macOS: minimum Rust 1.95 full suite, Clippy with warnings
forbidden on Rust 1.96, and 50,000 pool lifecycle cases on a Rust 1.96 release
build using seed `0x202609195eed`. That run exercised 1,229,866 connects,
5,871 reuses, 259,119 late successes after cancellation and 807,455 abort actions.
No physical network or native Windows execution is claimed. Replays and exact
commands are in the transport README; the final acceptance record will pin the
reviewed source and verdicts. Generated schema and existing stream sources are
unchanged from the accepted memory baseline.

The driver deadline query is a snapshot. Hosts must drive clock ticks
independently of action arrival (or rearm timers when requests/releases change
deadlines); a pending action future alone is not a timer service. Repeated helper
interactions share the original total interaction allowance. Cleanup deadlines
are preserved when a cancelled connector completes late.
