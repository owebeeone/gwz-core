# Remote transport runtime and pool interface gate

Status: **Phase 1/2 interfaces accepted and frozen, 2026-09-19**.
Accepted implementation tuple: transport `28f5afb3938a2aa8af0e1e8d5b07779add6ab776`,
core `ace269896ad80aee923e2e8fd31e565c43de57ed`, taut
`733e8a78897a90f017f4726e4331aed95e8cb977`, root review inputs
`9d0dc7ef5c616d64d52c296ea2fa34d83d21d73e`. Original Code and State reviewers
and the Surface reviewer all returned GO with no remaining findings in
`GwzRemoteTransportInterfaces-ReviewCode-1.md`, `-ReviewState-1.md` and
`-ReviewSurface-1.md` in the workspace dev-docs directory. This status update
records acceptance only and changes no reviewed implementation.
This package extends the accepted stream, pool and shared-schema checkpoints
with the active-I/O host contract from design §10.1 and complete consumer
message/admission proofs. The workspace
`dev-docs/GwzRemoteTransportInterfaces-Checkpoint.md` records the settled review
tuple, executed gates and the Code / State / Surface verdicts when available.

The accepted freezes are the Phase 1 schema/types, admission and message-handoff
contract, and Phase 2 stream/pool runtime API. They do not authorize physical
message delivery, SSH/HTTPS adapters, production CLI/core surface changes,
publication or native-platform qualification. This accepts the named interfaces
and their local evidence only; remote CI execution and consumer CI activation
remain outstanding as recorded below.

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
package/schema and rustfmt version. It requires the checkout's canonical `src`,
rejects preloaded taut modules and checks imported module origins. This remains an unpublished-package proof:
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

Retain at least one `Pool` clone for the intended endpoint lifetime. Dropping
the final `Pool` clone initiates shutdown, stops admission and invalidates active
leases, even if a driver, checkout or lease still holds shared state. Those
objects do not count as Pool owners. This transition differs from losing the
`PoolDriver`: final Pool drop leaves the retained driver responsible for draining
cleanup actions; driver loss requires prior host disposal as described below.

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
accept 1–16,384; allocation/helper/cleanup/idle timeouts accept 1–86,400,000 milliseconds;
network connect accepts 0–2,147,483,647, where zero disables network timing. Invalid
construction returns an error. These are endpoint limits; the operation's
existing fan-out limit remains independent.

The driver exposes these host-facing operations:

| API | Host duty |
|---|---|
| `next_action()` | Drain exactly one `Action` dispatcher and execute it outside the pool lock. |
| `connected(id, Result<Option<Identity>, Failure>)` | Acknowledge actual connect/auth completion; dispose partial resources before reporting failure, and close late success after cancellation. |
| `idle_closed(id)` | Report spontaneous disposal of an idle or eviction-closing resource; a `WrongState` result means checkout won and the host must route that exchange failure and discard its lease. |
| `closed(id)` | Acknowledge physical close; capacity is retained until this acknowledgement. |
| `advance(now_ms)` / `next_deadline()` | Initialize the chosen monotonic origin before the first checkout and drive it independently of command arrival. `next_deadline` is a snapshot, not a timer subscription; use the host timer duties below. |
| `begin_interaction(id)` / `end_interaction(id)` | Pause network time only for a separately bounded helper interaction; repeated interactions share the configured allowance. |
| `Pool::shutdown()` | Stop admission and begin endpoint cleanup. |
| `PoolDriver::shutdown_complete()` | Report complete only after every physical disposal is acknowledged. The host still executes all close/abort actions by their deadlines. |

The machine starts at zero. Before accepting the first checkout, call `advance`
with the host's chosen monotonic value; all later values use that same origin.
Do not create requests at zero and subsequently switch to a process/system
epoch: that would immediately expire budgets which have not elapsed.

The host must run a periodic timer independently of incoming actions, or re-query
deadlines after every mutation it controls with a bounded periodic fallback for
mutations through independently held Pool/Checkout/Lease handles. Choose a tick
interval consistent with the required timeout responsiveness. A pending
`next_action` future supplies no timer service, and an earlier deadline can arise
while it remains pending. A timer-aware host may cancel that pending receiver,
advance the clock and query the new snapshot, then resume receiving actions.

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

## Active-I/O host contract

The stream starts `IoState::Idle`. Only an endpoint may call `set_io_state` or
`record_io_progress`; initiators receive `WrongSide`. Hosts initialize the
clock before work, advance before reporting events and provide an independent
timer service even while the message receiver is pending.

`Network` charges the remaining I/O allowance. `Idle` and `Backpressure` preserve
it; `Interaction` spends the cumulative helper allowance instead. Repeated
state reports do not refill either budget. If either direction remains eligible
for peer progress, aggregate state is `Network`. Only positive actual backend
peer bytes in Network restore I/O allowance; local/message buffering, keepalives,
zero-byte reports and EOF do not. `io_status` exposes state, remaining budgets
and active deadline. Clock controls cannot restart active-I/O after close begins.

`Config::io_timeout_ms` defaults to 3,000 ms (0–2,147,483,647, zero disables);
`interaction_budget_ms` defaults to 120,000 ms (0–86,400,000). Construction
captures endpoint policy and Open can only shorten it. Design §10.2 preserves
native disabled and maximum network values: zero is allowed only under disabled
endpoint policy, while a positive request can bound a disabled setting. Connect
and I/O Open fields retain their tags/types. `Connect.network_deadline` is
optional; None means a started, network-untimed connection. Helper and disposal
deadlines remain bounded. Disabled stream timing retains Network state with
zero remaining network milliseconds and no active network deadline. Hosts subtract connect
helper time before constructing the active stream, carrying the same Open's
remaining allowance across the pool/stream seam. Zero allowance forbids waiting.
A new Open on a reused connection starts its own policy-capped allowance.

Expiry at the exact deadline wins over late progress/state reports. Endpoint
expiry emits `Failed { Timeout, Possible }`, preserves readable prefixes before
`Error::Timeout`, wakes waiters and requires lease discard. Capacity is held
until disposal is acknowledged. Close replaces active-I/O with its independent
cleanup deadline. All terminal paths preserve the first cause. These controls
add no wire field, physical transport, executor or retry policy.

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
| Final Pool owner initiates shutdown despite a retained driver/lease | `tests/pool_async.rs::dropping_last_pool_owner_shuts_down_even_with_a_live_lease` |
| Consumer host clock initialization, independent ticking and retained Pool lifetime | Consumer `tests/pool_host.rs`: `host_clock_keeps_large_nonzero_origin_for_connect_budget`; `periodic_tick_services_new_earlier_allocation_deadline_while_driver_waits`; `final_pool_clone_drop_shuts_down_live_lease_for_host_cleanup` |
| Bounded shutdown and late connector completion | `tests/pool.rs::shutdown_holds_capacity_until_abort_is_acknowledged`; `cancelling_an_open_keeps_its_reservation_until_late_success_is_closed` |

The prior acceptance record pins the original 66-test suite and 50,000-case
pool replay. Current evidence is recorded in the workspace interface checkpoint;
prior results do not by themselves qualify the new clock behavior.

## Package and generation gates

The checked consumer manifest names `gwz-transport = "=0.1.0"` without a path or
build hook. `package_proof.py` validates an explicitly supplied archive's digest,
package identity and clean source revision, rejects unsafe archive entries,
copies only the consumer and archive into isolated temporary paths, supplies a
temporary Cargo patch and runs the locked offline consumer suite. Registry
resolution requires a future published package and deliberate lockfile refresh.
Use the exact archive invocation in the consumer README/current checkpoint.

The owner's `scripts/regen.py --check` uses `taut-proto==0.9.1` and an exact
rustfmt build. `.github/workflows/contracts.yml` declares a standalone drift,
formatting, MSRV-test and packaging job. The consumer generator separately pins
the local taut extension checkout and owner IR digest. Local generation and
archive checks are the current executed evidence: transport has no remote,
and the extended taut revision is not established as remotely available. The
owner workflow has not run remotely and a multi-repository consumer workflow is
not activated. Neither missing inputs nor file-exists skips count as CI success.
Remote provisioning and publication remain separate operator actions.

## Scope remaining after this gate

1. Physical connections, SSH/HTTPS adapters and the supplied communication
   layer are not exercised. They are subsequent integration work.
2. Native Windows and cross-platform host qualification remain subsequent
   evidence; these fake hosts do not establish it.
3. The host must implement dispatch, timers, ownership routing, helper-budget
   transfer, disposal and shutdown. There is no production executor here.
4. The supplied layer bounds allocation before constructing outer GWZ messages,
   includes wrapper overhead in its budgets, preserves ordered delivery, reserves
   aggregate control capacity, provides bounded backpressure and notifies
   closure. Owner envelope limits do not bound an arbitrary outer wrapper.
5. The owner workflow is checked in, with remote execution and consumer CI
   activation outstanding as stated above. No remote/publish action is implied.

Code and State review must verify the gate's named schema/runtime obligations
and exact executed evidence. Surface review reads the public transport README
and consumer README cold. Only all required GO verdicts on the recorded tuple
can freeze the named interfaces. Optional GWZ fields remain Phase 4 work;
existing service methods remain unchanged.
