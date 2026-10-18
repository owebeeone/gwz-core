# Remote transport: in-memory stream checkpoint

Status: **accepted at gwz-transport `aa9ecae65d6c0d568c5f4d738f9930d49f684f56`
after [Code](../../dev-docs/GwzRemoteTransportMemory-ReviewCode-1.md) and
[State](../../dev-docs/GwzRemoteTransportMemory-ReviewState-1.md) reported GO,
2026-09-19; this accepts the in-memory stream implementation only, with no
interface freeze**. The [acceptance record](../../dev-docs/GwzRemoteTransportMemory-Checkpoint.md)
pins the complete reviewed tuple, validation and remaining scope.

## Scope and authority

The operator prioritized building gwz-transport first with in-memory tests,
random write/read chunk sizes, credit pressure and exact reconstructed bytes.
They explicitly excluded physical transport: communication is supplied elsewhere
and no CLI/core interface is changed. The implementation follows the
[design](GwzRemoteTransportDesign.md), especially §§5–6, under that clarification.
The seeded test approach follows the reference in `sdax-wz/sdax-rs`.

The review object is the new gwz-transport crate, its schema, supporting admission
and binding code, stream implementation, tests and README, plus these scope
clarifications. The lane owner records the exact committed tuple in the review
dispatch and reports. The taut external-type generator extension and test-only
core consumer are separate unfinished work; they are not needed to build or run
this crate and are excluded from this checkpoint. Remote provisioning,
publication, pool mechanics, SSH/HTTP adapters, mux and production core/CLI
integration remain later work. The Phase 1 interface gate remains open.

## Implemented boundary

`StreamMachine` is a deterministic active-stream state machine. Construct it
after a host validates Open/Opened and supplies negotiated identity and limits.
Its file-like read/write, flush, half-close and graceful close operations exchange
the generated taut Envelope directly through receive/next_message. No encoding,
physical framing, socket, pipe, background task, clock or network is used on this
path. Generated codecs remain separate optional host utilities.

`Stream` wraps the same machine in executor-independent futures, with a paired
`MessageEndpoint` for asynchronous outgoing messages, typed incoming delivery,
clock notifications and backend close completion. Clones share one exchange.
Bounded waker registrations are removed when their futures are cancelled. The
final stream owner cancels; losing the message endpoint wakes blocked callers.

The host preserves ordering, supplies bounded delivery queues and aggregate
scheduling, advances the monotonic clock independently of application calls,
routes session/stream identity, and owns any real sink or connection cleanup.
No stream constructor allocates a lease, connects or reads credentials. The
endpoint adapter must read only into its sink or an explicitly bounded sink
budget. It must call complete_close only after actual cleanup has determined
reuse disposition. Tests supply these host responsibilities using memory alone.

Credit counts consumed payload bytes; decoding never grants it. Forward flush
waits for sink consumption; reverse flush acknowledges admission to the bounded
read adapter. Close starts its deadline before waiting for credit and drains
unread reverse data with an explicit discarded flag. Errors after an accepted
prefix remain errors, not clean EOF. Terminal streams ignore late traffic.

## Verification and replay

TDD deterministic cases cover tiny windows, bounded partial writes, first-byte
batching, flush barriers, both flush directions, read-triggered send, half-close,
cancelled prefixes, malformed offsets, data after EOF, premature Close, graceful
drain, close timeout, async wakeups, cancelled waiter removal, waiter caps,
last-owner cancellation and delivery loss.

The default seeded Monte Carlo test runs 3,000 bidirectional cases. Each case
randomizes bytes, write/read chunk sizes (including zero), independent windows
and buffers, common payload cap, coalescing delay, bounded FIFO delivery queues,
delivery schedule and flush/half-close points. Independent source/offset/credit
ledgers check exact bytes and legal emission; consumption and boundedness are
checked each step. There is a deterministic step bound. Every sixteenth case
is rerun and its event digest and coverage must match exactly.

Failures print generator version, run seed, case number, case seed, configuration,
inputs, failure step and recent trace, plus a directly runnable replay command.
The README documents run-seed, count and direct-case-seed overrides. A separate
ignored campaign defaults to 50,000 cases and prints its time-derived seed before
starting. Keep source revision with seeds; preserve discovered bugs as named
fixtures when generator evolution could otherwise change the reproduction.

Validation commands (workspace root):

```sh
cargo test --manifest-path gwz-transport/Cargo.toml --locked
cargo clippy --manifest-path gwz-transport/Cargo.toml --all-targets -- -D warnings
gwz-core/protocol/.regen-venv/bin/python gwz-transport/scripts/regen.py --check
GWZ_TRANSPORT_MC_SEED=0x202609195eed cargo test --manifest-path gwz-transport/Cargo.toml --locked --release --test monte_carlo extended_message_streams -- --ignored --exact --nocapture
```

Review tier: dual Code/State using the original reviewers per operator direction.
This accepts only an implementation checkpoint, not a frozen API, real network
behavior or completion of the transport programme. No native Windows execution
or real network performance claim is made by these platform-independent tests.
