# Remote transport runtime and pool interface gate

Status: **draft gate package**, based on `gwz-transport` revision
`e8b9a1c5408cc9ea9528939b3a602acbeb697814`. The accepted stream and pool
checkpoints permit this interface review; they do not freeze the API or
authorize a socket, carrier, CLI, or core service implementation.

The current review object is the shared-schema consumer implementation and
this draft host contract. Acceptance of that checkpoint does not close either
the Phase 1 schema freeze or Phase 2 runtime/pool freeze in
[the plan](GwzRemoteTransportPlan.md). In particular, the unimplemented active
I/O clock semantics below must be settled before the Phase 2 interface freezes.

## Shared-schema consumer boundary

The test-only crate in `tests/transport_consumer` imports the owner's exported
IR into a small taut schema. Its generated `GwzTransportDelivery.message` field
has the native `gwz_transport::protocol::Envelope` type. The taut generator's
explicit external-type map replaces dependency declarations with Rust re-exports;
core does not own a second set of transport structs. The consumer also re-exports
the owner's CBOR runtime so generated codec calls use the same Rust types.

Cargo uses checked-in generated output and an exact package version, with no
generation hook, schema fetch, or sibling-path dependency. The explicit developer
regeneration command pins the taut source revision, extension file hashes, owner
package/schema and rustfmt version. This remains an unpublished-package proof:
the isolated archive runner supplies a temporary Cargo patch. Neither the
test-only delivery field nor its tag assigns a production GWZ protocol field.

## Implemented interface

The endpoint host constructs `pool::Pool::new(pool::Config)`, retaining the
`Pool` and its single `PoolDriver`. `Pool` clones share one endpoint pool.
`pool::Request::new(pool::Key, pool::Identity, pool::Owner)` creates a bounded
checkout request; the optional allocation, connect, and interaction budgets may
only shorten the configured values. `Pool::checkout` returns a cancellable
`pool::Checkout` future. Taking it yields an exclusive `pool::Lease`; a lease
must be consumed with `Lease::release(Disposition::Reusable)` only after the
exchange and backend cleanup prove reuse. Dropping a checkout cancels the
request, and dropping a lease discards its connection.

The full reuse key is `Key { scheme, username, host, port }`. The physical
capacity groups are separate: `PoolMachine::counts_for_key` reports reuse-key
entries, `counts_for_user_host` groups the same username and host across ports,
and `counts_for_host` groups a host across users, ports, and schemes. Repository
names do not enter any key. SSH explicit identity requests carry an
endpoint-resolved proof; HTTPS uses the no-username bucket and carries no
account identity claim.

Construction defaults are eight physical connections per user/host, eight per
host, 256 per endpoint, and 1,024 outstanding checkout requests. Counts include
opening, idle, leased and closing resources. Idle expiry is 60 seconds from
healthy release; a quiet leased stream never becomes pool-idle. Allocation,
connect, helper-interaction and cleanup budgets default to 30, 10, 120 and
5 seconds respectively. Capacity settings accept 1–4,096; outstanding requests
accept 1–16,384; each timeout accepts 1–86,400,000 milliseconds. Invalid
construction returns an error. These are endpoint limits; the operation's
existing fan-out limit remains independent.

The driver exposes these host-facing operations:

| API | Host duty |
|---|---|
| `next_action()` | Drain exactly one `Action` dispatcher and execute it outside the pool lock. |
| `connected(id, Result<Option<Identity>, Failure>)` | Acknowledge actual connect/auth completion; dispose partial resources before reporting failure, and close late success after cancellation. |
| `idle_closed(id)` | Report spontaneous disposal of an idle or eviction-closing resource; a `WrongState` result means checkout won and the host must route that exchange failure and discard its lease. |
| `closed(id)` | Acknowledge physical close; capacity is retained until this acknowledgement. |
| `advance(now_ms)` / `next_deadline()` | Drive the monotonic clock independently of command arrival and re-arm after every pool mutation. |
| `begin_interaction(id)` / `end_interaction(id)` | Pause network time only for a separately bounded helper interaction; repeated interactions share the configured allowance. |
| `Pool::shutdown()` | Stop admission and begin endpoint cleanup. |
| `PoolDriver::shutdown_complete()` | Report complete only after every physical disposal is acknowledged. The host still executes all close/abort actions by their deadlines. |

Deterministic hosts use the equivalent `PoolMachine` methods directly:
`new`, `request`, `take`, `next_action`, `connected`, `idle_closed`, `closed`,
`release`, `advance`, `next_deadline`, `begin_interaction`,
`end_interaction`, `cancel_operation`, `cancel_session`, `shutdown`,
`shutdown_complete`, `counts`, `counts_for_key`, `counts_for_user_host`,
`counts_for_host`, and `outstanding_requests`. The async facade delegates to
the same machine and adds waker-driven `Checkout`/`PoolDriver` ownership.

The action set is `Connect`, `CancelConnect`, `AbortConnect`, `Close`, and
`Abort`. `Abort` is an instruction to terminate a host-owned resource, never an
acknowledgement that termination already happened. The host cancels connectors
and disposes resources before dropping `PoolDriver`; driver loss invalidates
clients and does not perform physical I/O.

`Owner { session, operation }` is structured cancellation scope. The host
creates a fresh, never-reused session ID at each binding and stops admitting a
lost session before calling `cancel_session`. `cancel_operation` matches the
complete pair and preserves idle resources; `cancel_session` covers all active
work in that session and also preserves idle resources. Operation names may be
reused only in a fresh session.

The stream/lease seam is explicit: the host transfers typed stream messages,
drives `Stream::close`, waits for reverse cleanup, and has the endpoint backend
call `complete_close` after its cleanup proof. Only then may it release the
lease. A peer `Closed` or flush acknowledgement does not prove Git success or
connection health. A possible remote effect is surfaced and is never replayed
by this package.

## Focused fake gate suite

The existing deterministic fake-host tests are the gate suite; they require no
network, credentials, sockets, or carrier framing:

| Obligation | Focused proof |
|---|---|
| Stream cleanup precedes reusable release | `tests/pool_stream.rs::endpoint_returns_lease_only_after_exchange_cleanup` |
| Full reuse key versus per-user/host and per-host capacity | `tests/pool.rs::opening_and_closing_count_against_key_and_host_limits`; `tests/pool_regressions.rs::user_host_capacity_spans_ports_in_every_physical_state` |
| Identity eligibility and no repository partition | `tests/pool.rs::incompatible_idle_identity_is_retired_without_creating_repository_partitions`; `unproven_connections_are_single_use_and_wrong_proofs_fail_closed` |
| Physical disposal acknowledgements and idle-loss race | `tests/pool_regressions.rs::spontaneous_idle_loss_prevents_reuse_and_validates_tokens`; `checkout_winning_idle_loss_race_keeps_exclusive_lease_responsibility` |
| Structured session/operation cancellation | `tests/pool_regressions.rs::late_session_and_operation_cancellation_cannot_touch_a_fresh_session`; `session_cancellation_covers_all_its_work_but_preserves_idle_and_other_sessions` |
| Clock, helper, allocation, and cleanup duties | `tests/pool.rs::queue_network_interaction_and_cleanup_use_independent_budgets`; `idle_expiry_runs_from_release_and_never_reclaims_a_quiet_lease` |
| Async ownership and driver wake slot | `tests/pool_async.rs::clones_share_capacity_and_lease_drop_discards_while_checkout_drop_cancels`; `full_request_capacity_does_not_consume_the_driver_wake_slot`; `spontaneous_idle_disposal_wakes_queued_checkout_and_schedules_replacement` |
| Bounded shutdown and late connector completion | `tests/pool.rs::shutdown_holds_capacity_until_abort_is_acknowledged`; `cancelling_an_open_keeps_its_reservation_until_late_success_is_closed` |

The accepted checkpoint records 66 passing tests and the 50,000-case replay.
This checkpoint reran all 27 pool/stream-seam tests above, followed by the normal
66-test locked suite on Rust 1.95; both passed. Transport source is unchanged,
so the extended campaign was not repeated.

The checked consumer manifest names the exact registry requirement
`gwz-transport = "=0.1.0"`; publication is deliberately absent from this
checkpoint. The explicit `package_proof.py` runner accepts a caller-verified
`.crate` archive, copies the consumer and archive into isolated temporary
paths, installs a temporary Cargo patch, and runs `cargo test --offline
--locked`. The workspace archive at transport revision
`e8b9a1c5408cc9ea9528939b3a602acbeb697814` (SHA-256
`24c9d7a839b1a23ae1f188541ac87092550dcae99bd9cf6e14df4c900b1a7dd9`) passed
all six consumer tests. These include the same bidirectional stream exchange
through typed values and encoded payloads. The runner also verifies the archive's
Cargo VCS revision and rejects dirty-source metadata before extraction.
This proves the checked consumer needs no sibling
checkout or schema download; registry resolution awaits a real release. The
repeatable runner invocation is:

```sh
gwz-core/protocol/.regen-venv/bin/python \
  gwz-core/tests/transport_consumer/package_proof.py \
  --archive gwz-transport/target/package/gwz-transport-0.1.0.crate \
  --archive-sha256 24c9d7a839b1a23ae1f188541ac87092550dcae99bd9cf6e14df4c900b1a7dd9 \
  --source-revision e8b9a1c5408cc9ea9528939b3a602acbeb697814
```

## Remaining evidence and gate decisions

This package still leaves the following explicit evidence gaps:

1. No physical connection, SSH/HTTPS adapter, supplied carrier, or CLI/core
   integration has been exercised.
2. No native Windows or cross-platform host qualification has been performed.
3. The pool traits and ownership semantics remain a draft interface until a
   dedicated interface review records the exact host callback and timeout
   contract.
4. The host has no production dispatcher yet; the fake tests establish duties,
   but not executor integration or shutdown wiring.
5. Active stream I/O clock/cancellation semantics remain unimplemented. Phase 2
   must define and test which waits count as network stalls, excluding deliberate
   backpressure and bounded helper interaction; later adapters bind their waits
   to that clock. This is an open runtime obligation, not only missing adapter
   evidence. Pool tests currently cover allocation, connect, interaction,
   cleanup and idle clocks; stream tests cover batching and close deadlines.
6. Payload codec tests do not qualify the supplied communication layer. That
   layer must bound allocation before constructing an outer GWZ message, include
   its wrapper overhead in its own budgets, preserve ordered delivery, provide
   bounded backpressure and notify closure while an operation is active. The
   transport envelope's admission limits do not bound an arbitrary enclosing
   message. No physical carrier implementation belongs in this package.
7. The local consumer tests are a focused proof, not the complete Phase 1/2
   message and runtime matrix. End-to-end Open admission/effect ordering, the
   full contract suite through encoded payloads, and CI generation-drift wiring
   remain phase-exit work. The local commands below do not establish CI results.

The current checkpoint may accept the shared-type integration proof and this
draft contract after focused tests and review. A GO at this checkpoint does not
freeze the runtime API or authorize dependent adapter implementation. Complete
the remaining Phase 1 message/admission evidence and Phase 2 runtime obligations,
then seek the separately named interface freezes in the plan before adapters.
Optional GWZ fields remain a Phase 4 decision; no new service method or physical
framing is proposed here.
