# Push producer-busy signal: design (DRAFT, revised after the State review)

Status: **implemented 2026-10-18 in two steps, after this revision for `GwzPushProducerBusyDesign-ReviewState.md` (GO, six P3s, all folded in below) and operator decision D45** (ceiling 600 s; in-process only for 1.1.0; the separate `Handoff` map). The rows below are the ones that landed. Authority: D42 (`GwzTransportTR18-OperatorDecisions.md`, option (a)). Trigger evidence: run `2026-10-17-blobpush` (`gwz-core-evidence/.../runs/2026-10-17-blobpush/README.md`). Origin: `GwzTransportSshPumpStall-ReviewState.md`, Follow-up 1 and P3-2(b).

## 0. Scope

| | |
|---|---|
| Deliverable | A push over SSH whose libgit2 thread is busy between stream calls (the large-blob `git_odb_read`) no longer fails at `--ssh-timeout`, to a server that indexes the pack as it arrives (`receive.unpackLimit=1`, which the trigger row sets) and, as measured once, to a stock server as well: the trigger run with `GWZ_BLOBPUSH_SERVER_DEFAULTS=1` pushed 3 GiB over SSH at the default limit with a 30.8 s producer pause and exit 0 (run `2026-10-18-blobfix`, `raw/trigger/ssh-3072-stock-server.log`). That is one run, at one size, on one host under load; it does not show that a stock server is never silent past the limit after the last byte for other pack sizes or object counts, and the earlier 64 MiB HTTPS run that failed that way was a different row. A dead server still times out. |
| May change | gwz-core only: `git/endpoint/{ssh_pump,stream_io,ssh_handoff}.rs`, `ssh_worker.rs` and `ssh_worker/{endpoint,open_request}.rs`, `transport_host/session.rs` (`Entry`), `session/driver/{opening,pump}.rs`, design §10.1, tests, the trigger row, one evidence run. |
| Reference only | gwz-transport (no change, section 5), git2-rs, gwz-git, libgit2, HTTPS (reports `Backpressure` while its producer is paused, `https_worker/serve.rs:186`, with no ceiling: a limitation noted for the operator, not changed here). |

## 1. The problem

`write_object` calls `git_odb_read` on the blob (`pack-objects.c:343`), which inflates the whole object before its first byte is written. `git_packbuilder__prepare` (delta search) runs *before* the command list (`smart_protocol.c:1233-1239`), so it falls in the `Idle` wait before the first client byte; only `git_odb_read`, `get_delta` and the per-1 MiB deflate fall after the pack begins. The commands, pack header, commit and tree have gone out; `F` and `R` are empty; `set_io_state` reports `Network`. The pause is about 2.3 CPU-s per GiB; at the default 9 s a single object of about 2.3 to 3.9 GiB fails. The pump cannot tell a busy producer from a finished pack waiting for report-status, because libgit2 sends no end-of-write before it reads (`close` sends `EndWrite`, `ssh_remote.rs:99-101`, after `update_tips`). Only the thread that owns the stream knows: it is inside `BlockingStream::read` (waiting for the server) or outside every call on it (producing).

## 2. The signal

**`ProducerGate`** (defined in `stream_io.rs`, because the consumer proof loads that file alone and it may not import its siblings): an `Arc` holding one `AtomicU32` and the ceiling.

- **One word, one load.** Bit 31 is `ARMED` (a call has returned at least once); bits 0-30 are the number of calls in flight. `busy()` is one load: `armed && count == 0`. `enter()` is one `fetch_add(1)`. The guard's drop is one `fetch_update` that decrements and sets `ARMED`. There is no window in which a thread about to enter a call reads busy, and a gate that has never completed a call reads not busy. The two loads of the first draft (and the race they left before the first client byte) are gone.
- **Writer side.** `BlockingStream` gets an optional gate (`with_producer`, like `with_stall_ms`). `read`, `write`, `flush`, `end_write` and `close` each hold an RAII guard for the call, on every path including `Err` and unwinding. git2-rs's `write_all` loops `write`, so the brief zeros between partial writes cost at most one 1 ms tick with the budget kept. The gate is shared by all clones.
- **Default is fail-closed.** No gate, or an unarmed gate, is "not busy", which is today's `Network`. A lost wakeup inside `wait()` leaves `count = 1`, also `Network`.
- **`ceiling_ms` is carried in the gate**, so the driver, which makes the gate, is the only place that knows it (the endpoint worker needs no new field). The default is `PRODUCER_CEILING_MS = 600_000` (D45); a `cfg(test)` setter on the runtime changes it for one end-to-end row.

### How the gate reaches the pump (a separate `Handoff` map, D45)

The driver makes the `BlockingStream`; the endpoint owns the pump. In the local placement they share a process and a `Handoff` (`ssh_handoff.rs`: "nothing here crosses a carrier, so an endpoint in another process gets none"). `UrlExtras` is not used: it carries a secret with wipe semantics and three consumers branch on its presence (`ssh_local.rs:67,73,86`, `opening.rs:200-208`), so a push open must not start carrying one. Instead:

1. `Handoff` gets a `producers: Mutex<BTreeMap<(String, i64), ProducerGate>>` beside `extras`, `ends` and `holds`. `Handoff::open` takes the gate as a third argument and deposits it under the same lock and the same `(session, stream)` key as the extras; `Deposit`'s drop removes both if the endpoint never took them. `Handoff::take_producer(session, stream)` is the endpoint's, once.
2. `driver/opening.rs` makes a gate for an SSH open of `ReceivePack` on a session that has a handoff, deposits it, and stores a clone in `Entry` (new field). `driver/pump.rs:151` builds `BlockingStream::new(stream).with_stall_ms(stall_ms).with_producer(gate)`.
3. `ssh_worker/endpoint.rs` (`start_endpoint_open_tagged`) takes it beside `url` into `OpenRequest.producer`; `ssh_worker.rs:324`, after `start_exchange`, calls `resource.pump().set_producer(gate)`. `start_exchange` and its wrappers keep their signatures.

**Equivalence the extras route would have needed, now trivial and asserted anyway:** a push open with a gate has the same `url` (`None` unless it had extras), the same pool key and identity, the same helpers partition and the same hashed host name as one without. The plumbing row asserts the pool identity (`runner.rs:156-186`) is unchanged and that no extras exist for a plain push open.

A fetch gets no gate. HTTPS is untouched.

## 3. What the pump charges

`set_io_state` becomes `&mut self` (it reads and writes episode state; `tick_inner` is already `&mut`). A new arm sits after the `Idle` arm and before the final `Network`. It reports `Backpressure` only when **all** hold:

1. a gate is attached and `pack_sent`: **derived in the pump**, set in `deliver` on the first client `Data` payload of a pump whose `set_service` was receive-pack. It does not depend on `GitTurns` still running (the draft's `pack_started` was lost whenever the tracker stopped early, for example on a stderr line at the client's turn: `git_turns.rs:80-90, 120-138`);
2. the client has not half-closed (`client_ended`, set in `deliver` on `EndWrite`; the pack is complete by definition after it);
3. `forward` and `reverse` are both empty;
4. the gate is busy;
5. `now - last_byte < ceiling`.

The first draft's "the server is not done" condition is **dropped as redundant**: the `Idle` arm above already takes `server_done` with `F = R = 0`, so no mutation can kill it and it has no row.

Everything else keeps today's result: a non-empty `forward` is `Network` (a write to a dead server must time out); a partly full `reverse` is `Network`; a producer inside `read` is `Network`; fetch, the advertisement and the wait for the first byte are unchanged.

State in the pump: `now` (set in `advance`, so `advance` becomes `&mut self`), `last_byte: u64` (set to `now` in `deliver` on the first `Data`, and by `record_progress` for every byte that moves in either direction), `pack_sent`, `client_ended`, and the gate. `record_progress` becomes `&mut self`; all callers already are.

**No `WrongState`.** `Backpressure` refuses `record_io_progress` (`io_clock.rs:48-49`). The arm needs `F` empty at tick start, `deliver` runs only between ticks, so `write_forward` records progress only when the tick began in `Network`; `read_reverse` and `drain_stderr` call `report_network` before `record_progress`. A row pins this with a stdout byte and a stderr byte arriving in a tick that began in the new `Backpressure`.

## 4. The ceiling, exactly

`last_byte` is the time of the last byte the pump moved in either direction (first `Data` delivered, a byte accepted by the channel toward the server, or a byte read from the server's stdout or stderr). The arm applies while `now - last_byte < ceiling`, and the bound is **per episode**:

- A silent episode ends at `last_byte + ceiling`; the state is then `Network` and the budget the clock kept (at most `STALL`) runs, so the stream times out at `last_byte + ceiling + remaining`, **at most `last_byte + ceiling + 9 s` at the default**. `Backpressure` suspends the budget without refilling it (`io_clock.rs:23-37, 56-67`).
- **Episodes chain.** A byte restarts the episode. So against a server that died silently, each chunk the producer writes that the dead peer's SSH channel window still absorbs (about 2 MiB for OpenSSH) is progress and gets a fresh ceiling: several large compressible objects whose compressed bytes fit in the remaining window each get a full episode. The total is bounded by the window, not unbounded, and after the window fills, `F` is non-empty and the state is `Network`.
- **A chatty server restarts it.** A server byte mid-pause (stderr, or keepalives from a non-stock server) restarts the episode, and in `Network` server bytes refill the budget anyway; against such a server a hung producer is not bounded by the ceiling. If 64 KiB accumulate in `R`, the existing full-buffer arm gives `Backpressure` with no ceiling at all. Both behaviours exist today; this change does not alter them.
- **Empty calls cannot extend it.** Only a byte moves `last_byte`; entering and leaving calls does not, and neither does an observer happening to catch the producer inside one.
- **A silent NAT drop** does not fail the next write: the write is absorbed by the window and socket buffers, then `WouldBlock`, `F` non-empty, `Network`, and a stall. The outcome is bounded; it is not an immediate I/O error.
- **A thread that died holding the stream** is handled by `RemoteTransport::drop` and `cancel` (`ssh_remote.rs:107-117`), not by the ceiling. For a truly hung producer the ceiling frees the **connection, the pool slot and the server**; the user's operation returns only when the libgit2 thread does.
- **Value: 600 s (D45).** It fires only for a hung producer, since `git_odb_read` needs the object in RAM (about 69 s for 6 GiB on a Pi without swap). A swapping slow host could exceed it and gets a stall message naming `--ssh-timeout` after 10 minutes; flagging a ceiling expiry in the facts is a possible refinement, not done here. `--ssh-timeout 0` turns the stall clocks off and so this one.

## 5. gwz-transport: no change

The clock already has the needed states, the ceiling is pump-local (driven by the `now` the pump receives), and the signal never leaves the process, so it needs no stream method. Rejected: charging the producer to `Interaction` (its budget is cumulative and visible; §10.1 forbids a generic pause).

**1.2.0 record (D45): the carried form.** In-process only for 1.1.0. The `Cli` placement (a driver with no handoff, `install_cli`) is compiled only under `gwz_session_candidate` (`cli_placement.rs:1-7`), so it is not in the 1.1.0 build; it and a session host get no gate and keep today's stall for a producer pause, which fails closed. A carried signal is a wire change in gwz-transport (a new `MessageKind` and `Envelope` body, a `MessageEndpoint` accessor, regenerated taut code, a version note) with its own dual review, and must meet four constraints:

1. **Edge-filtered or debounced.** The first draft's "sent when its gate flips" is two messages per stream call; it must send on a state edge that has lasted, not on every call.
2. **The ceiling stays endpoint-local.** The endpoint must never trust a remote initiator's busy claim without its own bound, measured from the last byte on the endpoint's clock.
3. **Ordering relative to `Data` is specified.** A busy message must not overtake the `Data` it follows nor be applied after the next `Data`; the endpoint applies it in message order.
4. **§10.1's `Backpressure` definition is widened** (section 8), since a busy producer is none of "consumer pressure, bounded message queues or exhausted credit".

Trigger row for it: a blob push over the Cli placement. Where it is filed (release-scope record or decisions table) is the lane owner's call.

## 6. Fail-closed argument

| Situation | State | Result |
|---|---|---|
| Server dies mid-pack, producer keeps writing | `F` fills (window, then `WouldBlock`) | `Network`; stall `STALL` after the last accepted byte (chaining bounded by the window, section 4). |
| Producer blocked in `write` (send buffer full) | in a call; `F` non-empty | `Network`. |
| Server dies after the pack; producer waits in `read` for report-status | in `read` | `Network`; times out. The case the old design could not separate. |
| Server frozen, producer outside calls | `Backpressure` to `last_byte + ceiling` | Then `Network`; fails at `last_byte + ceiling + STALL` (an end-to-end row freezes the server and holds the producer out). **The one weakening, bounded.** |
| Producer hung for good, server alive | same | Same bound; the ceiling releases the connection and slot. |
| Cancel | independent of the I/O state | `bridge_cancelled` discards the connection. |

The arm cannot spread: it needs `F`, `R` empty, a busy gate (so a thread in any stream call is `Network`), a first client `Data` and no `EndWrite` (so fetch, the advertisement and the wait for the first byte are unchanged), and the ceiling counts from the last byte. No gate is `Network`.

## 7. Tests (written first, seen red, logs kept)

**Pump rows** (`ssh_pump_clock_tests/producer.rs`, a child module of the existing fixture file; fake backend, `STALL_MS` 3 s). "Red" is against the current pump.
1. Busy gate after the first `Data`, `F = R = 0`: tick at `10 * STALL_MS`: `Backpressure`, not terminal. *Red: terminal `Timeout`.*
2. The same with the gate in a call: `Network`; terminal `[Timeout]` at `last_byte + STALL_MS + 1`.
3. Ceiling edges (a small ceiling): `Backpressure` at `last_byte + ceiling - 1`; `Network` at `+ ceiling`; not terminal at `+ ceiling + STALL_MS - 1`; terminal `[Timeout]` at `+ ceiling + STALL_MS + 1`.
4. A byte at `ceiling - 1` restarts the episode; two large objects in a row each get a full ceiling.
5. **`pack_sent` does not depend on `GitTurns`:** the tracker is stopped by a stderr line at the client's turn, then a push and a producer pause: `Backpressure`.
6. **Negative controls, each shown red against a mutated pump:** `F` non-empty and busy is `Network` (mutation: drop `forward.is_empty()`); `R` partly full and busy is `Network`; fetch with a busy gate is `Network`; `EndWrite` received is `Network`; **mid-advertisement** (server's turn, consumer outside calls and armed, `F = R = 0`) is `Network` (replaces the first draft's before-the-first-byte `Idle` row, which the `Idle` arm's order made unable to kill the mutation that drops `pack_sent`); a defaulted gate is `Network`; **a tracker stopped by a stderr line before any client byte, with the gate busy, is `Network`** (the converse of row 5: the arm must not start from the tracker's stop; red under the mutant that reads `turns.stopped` as the pack's start), and row 5 itself asserts the tracker did stop (`Network`, not `Idle`, before the first client byte).
7. Empty calls do not extend the ceiling (mutation: clear `last_byte` on entering a call).
8. **No `WrongState`:** from `Backpressure`, a stdout byte and a stderr byte in one tick are accepted and the stream is not terminal.

**Gate rows** (`stream_io_tests.rs`): `busy()` is false until the first call returns, false inside a blocked `read` and `write`, true after; the guard drops on `Err`, on cancel and on unwinding; clones share one gate; a thread blocked in `close()` and one blocked in `end_write()` read not busy; a stream with no gate behaves as before.

**Plumbing rows** (`ssh_handoff.rs`, `transport_host/producer_busy_tests.rs`): the handoff deposits a gate under its `(session, stream)` and the endpoint takes it once, with no extras riding along; a dropped deposit removes the gate; a failed open deposits none; a push open on a runtime nothing configured carries the shipped 600 000 ms ceiling; a push open carries a gate and a fetch open does not (`Session::producer_gates_for_test`); a push open reuses the connection a fetch left, so it is pooled and keyed as a fetch is (`Opened.reused`); the existing handoff tests pass. A driver with no handoff creates no gate (the condition in `opening.rs`); that is the fail-closed default and has no row of its own beyond the pump's no-gate row.

**End to end** (`transport_host/producer_busy_tests.rs`; real `sshd` fixture, `--ssh-timeout` 1 s, `request.context.open(.., ReceivePack, ..)`, a real pack from `git pack-objects`):
- *Green after:* write the commands and half the pack, sleep several stall limits outside any call, write the rest, read the report; the ref is updated. *Red before.*
- *Frozen server:* write half the pack, `CutProxy::freeze_existing`, keep the producer outside calls for longer than `ceiling + stall` (test ceiling 2 s); the stream's retained failure appears between `ceiling + stall - 100 ms` and `ceiling + stall + 2 s` after the last client byte, while the producer is still asleep.
- *Hung producer:* the server is alive, the producer never wakes (parked until the assertion is made); the same window, and the connection and slot are released.

**Trigger row:** `blob_push_tests.rs` landed from the blobpush lane's `runner/blob_push_tests.committed.rs.txt` (+3 lines in `transport_host/mod.rs`), its `#[ignore]` reason updated; `receive.unpackLimit=1` stated in the row and in section 0. 3 GiB over SSH at the default limit: red log `raw/final/ssh-3072-default.log` (blobpush run), green log in `2026-10-18-blobfix`.

**Gates**: fmt (+1.95.0), `scripts/checks/*.py` (including `test_consumer_bridge_standalone.py`: the gate lives in `stream_io.rs`), boundary job, ordinary and transport-only legs (`GWZ_TEST_GIT=real`), Windows lane check (`blobfix-20261018-a`). Nothing uses `getrusage`.

## 8. Documentation edits

§10.1: (a) widen the `Backpressure` definition to add "or, on a push after the client's first byte (its commands), the client's own producer demonstrably busy: the stream's owner is outside every call on it, nothing is queued either way, and no byte has moved for less than the ceiling (600 s)"; (b) replace the P3-2(b) sentence (a producer pause mid-pack is `Network`) with the new rule and the ceiling, keeping the statement that a producer inside a read, or a pause past the ceiling, is `Network`; (c) state the in-process limit and point at section 5.

## 9. Steps (each aspirationally under 500 lines)

1. `ProducerGate` and the `BlockingStream` wiring (stream_io.rs, ~70), the pump arm, `pack_sent`, `last_byte`, the ceiling (~60), pump rows and gate rows (~280). Red logs kept.
2. `Handoff` map, `driver`, `Entry`, `OpenRequest`, `ssh_worker` plumbing (~90), plumbing rows, the three end-to-end rows (~220), §10.1, the trigger row landed, the evidence run `2026-10-18-blobfix`.
