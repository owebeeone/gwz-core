# Push producer-busy signal: design (DRAFT)

Status: **draft for lane-owner review, 2026-10-18. Nothing is implemented.** Authority: operator decision D42 (`GwzTransportTR18-OperatorDecisions.md`, option (a)). Trigger evidence: run `2026-10-17-blobpush` (`gwz-core-evidence/.../runs/2026-10-17-blobpush/README.md`). Origin: `GwzTransportSshPumpStall-ReviewState.md`, Follow-up 1 and P3-2(b).

## 0. Scope

| | |
|---|---|
| Deliverable | A push over SSH whose libgit2 thread is busy between stream calls (the large-blob read) no longer fails at `--ssh-timeout`. A dead server still does. |
| May change | gwz-core only: `git/endpoint/{ssh_pump,git_turns,stream_io,ssh_handoff}.rs`, one new small file, `transport_host/session/driver/{opening,pump}.rs` and `session.rs` (the `Entry`), `ssh_worker.rs` (one call), design §10.1, tests. |
| Reference only | gwz-transport (no change proposed, section 5), git2-rs, gwz-git, libgit2, the HTTPS path (already `Backpressure` while its producer is paused, `https_worker/serve.rs:186`). |

## 1. The problem in one paragraph

`write_object` calls `git_odb_read` on the blob (`pack-objects.c:343`), which inflates the whole object before libgit2 writes its first byte. The commands, pack header, commit and tree have already gone out, so `GitTurns` has stopped tracking (`git_turns.rs:87-90`), `F` and `R` are empty, and `set_io_state` reports `Network`. The pause is about 2.3 CPU-seconds per GiB; at the default 9 s a single object of about 2.3 to 3.9 GiB fails. The pump cannot tell a producer that is busy from a finished pack that waits for the server's report-status, because libgit2 sends no end-of-write before it reads. The only party that knows is the thread that owns the stream: it is inside `BlockingStream::read` (waiting for the server) or outside every stream call (producing).

## 2. The signal

A producer gate: one `Arc` of two atomics, shared by the Git-facing `BlockingStream` and the pump.

- **Writer side** (`stream_io.rs`). `BlockingStream` gets an optional gate (`with_producer`, like `with_stall_ms`). Each of `read`, `write`, `flush`, `end_write`, `close` takes an RAII guard that increments `in_call` on entry and decrements on exit, on every path including errors and unwinding. The gate is shared by all clones, so `close` and `cancel` from the transport's `active` clone count the same. Cost: two atomic operations per stream call.
- **Meaning.** `in_call == 0` means *the thread that owns the stream is outside every call on it*. That is the only fact the gate states. It does not say the thread is alive or making progress; the ceiling (section 4) deals with that.
- **Default is fail-closed.** A new gate reads "in a call" (not busy) until a call has returned once (an `armed` flag set by the first guard drop; the pump reads `armed && in_call == 0`). A stream with no gate reads the same. An unattached pump is today's pump.
- **Reader side** (`ssh_pump.rs`). The pump holds the other `Arc` and reads `in_call` once per tick.

### How the gate reaches the pump

The driver (which makes the `BlockingStream`) and the endpoint (which owns the pump) are separate sessions joined by the mux. In the local placement they share a process and already share a side channel: `Handoff` (`ssh_handoff.rs`), whose header states "nothing here crosses a carrier, so an endpoint in another process gets none" and which carries `UrlExtras` from `Session::open` to `Endpoint` under `(session, stream)`. The gate rides it:

1. `driver/opening.rs`: for an SSH open of `ReceivePack` on a session with a handoff, create the gate and put it in the open's `UrlExtras` (new field `producer`, new `with_producer`; the type's doc is widened from "URL extras" to "what the driver hands this open"). Today extras exist only for a password, a written host or a no-helpers open, so a push open now always has them. This is the one behaviour change outside the pump: check that `Some(extras)` selects no different pool, key or setup path (`runner.rs:77,157,172` use only `host()` and `password()`; the written host equals the key host when extras were absent).
2. The same function stores a clone in `Entry` (new field), and `driver/pump.rs:151` builds `BlockingStream::new(stream).with_stall_ms(stall_ms).with_producer(gate)`.
3. `ssh_worker.rs:324`, after `start_exchange`: `resource.pump()` gets `set_producer(request.url.producer(), ceiling)`. `ChannelResource::start_exchange` and its wrappers keep their signatures.

A fetch gets no gate. HTTPS is untouched.

## 3. What the pump charges

New arm in `set_io_state`, ahead of the final `Network` fallthrough. `Backpressure` is reported only when **all** hold:

1. a gate is attached and the service is receive-pack and `turns.pack_started()` (new, `!fetch && client_started`: the first client byte has gone, so tracking stopped);
2. the client has not half-closed (`client_ended`, set in `deliver` on `EndWrite`; the pack is complete by definition after it);
3. `forward` empty and `reverse` empty (nothing queued toward the server, nothing owed by it that the consumer holds);
4. `!server_done` (the existing arms already handle a finished server);
5. the gate is armed and `in_call == 0`;
6. `now - busy_since < ceiling`.

Everything else keeps today's result. In particular a non-empty `forward` is `Network` (a write to a dead server must time out), a partly full `reverse` is `Network`, and a producer blocked in `read` is `Network`. This is the P3-3 list, and each gets a row.

State in the pump: `now` (set in `tick`), `busy_since: Option<u64>` (set to `now` the first tick the arm is otherwise eligible, cleared by `record_progress`), `client_ended`, and the gate and ceiling. `record_progress` becomes `&mut self`; all callers already are. `Backpressure` does not charge the clock and keeps `network_remaining` (`io_clock.rs:23-37,56-67`); the next byte refills it as today.

**Each object is its own episode.** A written chunk is progress, so it clears `busy_since` and refills the budget. Many medium objects do not add up; each large object gets one pause of up to the ceiling.

## 4. The ceiling

`busy_since` measures wall time since the **last byte moved** (any `record_progress`), not time since the producer last entered a call. So a producer that makes empty calls, or an observer that happens to catch it inside a call, cannot restart the count: only a byte can.

- At `now - busy_since >= ceiling` the arm stops applying and the state is `Network`. The clock then charges the budget that was left: the push fails at about `ceiling + remaining` (at most ceiling + 9 s), with the usual `no progress for N seconds` stall error. A hung producer (stuck disk, deadlocked callback, a thread that died holding the stream) therefore cannot hold the clock open for longer than that.
- **Proposed value: 600 s** (`PRODUCER_CEILING_MS`, a constant next to the pump, not a user knob). Basis: the pause is about 2.3 CPU-s per GiB here (3 GiB: 11.8 s; 4 GiB: 27 s under load 116); a Raspberry Pi is 4 to 5 times slower, so 10 minutes covers an object of roughly 50 GiB there, far beyond what `git_odb_read` can inflate into memory. **This is a policy number and the operator should choose it** (open question 2). It is not scaled by `--ssh-timeout`; `--ssh-timeout 0` turns every stall clock off and so this one too.
- The ceiling is injected: `set_producer(gate, ceiling_ms)`. The driver takes it from one runtime field that defaults to the constant; a `cfg(test)` setter lets one end-to-end row use seconds.

## 5. gwz-transport: no change

The clock already has the states needed: `Backpressure` suspends the budget, and `Network` after the ceiling resumes it. The ceiling is pump-local, driven by the `now` the pump already receives. The signal needs no stream method because it never leaves the process; it does not pass through `Stream` or `MessageEndpoint`. Rejected: charging the producer to `Interaction`. Its budget is cumulative and visible and §10.1 forbids using it as a generic pause.

**Limit, to be decided (open question 1).** In-process only. The `Cli` placement (`install_cli`: a driver with no handoff, its endpoint in another process) and any future session host get no gate; their pushes keep today's behaviour, a stall at `--ssh-timeout` for a producer pause, which fails closed. The carried form is a protocol change in gwz-transport: a new `MessageKind` (and `Envelope` body, `MISSING_OK`) sent by the initiator side when its gate flips, a `MessageEndpoint` accessor for the last value, regenerated taut code, and a wire-version note, with per-step dual review for a wire format. I recommend recording it for 1.2.0 with the session host, not building it for 1.1.0. If the operator wants it in 1.1.0, gwz-transport needs rustfmt 1.96.0 and its own `cargo test`, and the in-process gate becomes its test double.

## 6. Fail-closed argument

The failure to exclude is a dead server that the new arm hides.

| Situation | State | Result |
|---|---|---|
| Server dies mid-pack, producer keeps writing | `F` fills (libssh2 window, then `WouldBlock`); `F` non-empty | `Network`; times out `STALL_MS` after the last byte. Unchanged. |
| Server dies, producer blocked in `write` (send buffer full) | in a call; `F` non-empty | `Network`. Unchanged. |
| Server dies after the pack, producer waits for report-status | in `read` | `Network`; times out. This is the case the old design could not separate, and the gate separates it. |
| Server dies while producer is outside calls | `Backpressure` | **Undetected for at most the ceiling**, then `Network` and a timeout. This is the one weakening, and it is bounded and opt-in by evidence (the thread is demonstrably not talking to the server). |
| Connection dropped by a NAT during a long pause | `Backpressure` | The next write fails (`PumpError::Io`): an error, not a hang. |
| Cancel | independent of the I/O state | `bridge_cancelled` discards the connection. The libgit2 thread returns when its read does, as today. |

Why the arm cannot spread: it requires `F` and `R` empty, so any queued byte or owed byte is `Network`; it requires a call count of zero, so a thread inside any stream call is `Network`; it requires the first client byte to have gone and no `EndWrite`, so fetch, the advertisement and the wait for the first byte (`Idle`) are unchanged; and the ceiling counts from the last byte, so no pattern of calls extends it. A missing or defaulted gate is `Network`.

It also adds no `WrongState`: the arm needs `F` empty at tick start, `deliver` runs only between ticks, so `write_forward` records progress only when the tick began in `Network`; the read and stderr paths (`read_reverse`, `drain_stderr`) call `report_network` before `record_progress`.

## 7. Tests (written first, seen red, logs kept)

**Pump rows** (`ssh_pump_clock_tests.rs`, fake backend, `STALL_MS` 3 s). Red today means failing on the current pump.
1. Producer busy after the pack began, `F=R=0`: tick at `10 * STALL_MS`; state `Backpressure`, not terminal. *Red: terminal `Timeout`.*
2. Same with the gate in a call (waiting for the report): `Network`, terminal `[Timeout]` at last byte + `STALL_MS` + 1.
3. Ceiling edges: `Backpressure` at `ceiling - 1`; `Network` at `ceiling`; not terminal at `ceiling + remaining - 1`; terminal `[Timeout]` at `ceiling + remaining + 1`.
4. A byte at `ceiling - 1` clears the episode and refills the budget; a second pause gets a full ceiling. Two large objects in a row survive.
5. Guard rows, each shown red against a mutated pump: `F` non-empty and busy gives `Network` (drop `forward.is_empty()`); `R` partly full and busy gives `Network`; fetch with a busy gate gives `Network`; `EndWrite` received gives `Network`; before the first client byte the state is still `Idle`; a defaulted gate is `Network`.
6. Empty calls and observer luck: a producer that enters and leaves calls without moving a byte does not extend the ceiling (the mutation `busy_since` cleared on any call is red).

**Gate rows** (`stream_io_tests.rs`): `in_call` is 1 inside a blocked `read` and a blocked `write`, 0 after; the guard drops on `Err`, on cancel and on unwinding; clones share one gate; a stream with no gate behaves as before.

**Plumbing rows**: a driver with a handoff deposits the gate for a receive-pack open and not for upload-pack; the endpoint takes it once and the pump has it; a driver with no handoff opens with none; the existing extras tests still pass; a push open that now carries extras selects the same pool connection and key as one without.

**End to end, fast** (SSH fixture, `--ssh-timeout` 300 ms, a thread writing a real pack from `git pack-objects` in two parts with a sleep of several stall limits outside any call): green after, red before. A second row stops the fixture server during the sleep and sets the test ceiling to 2 s: the push fails with the stall error at about ceiling + stall, not before the ceiling and not never.

**Trigger row**: the blobpush lane's `a_large_random_blob_push_over_ssh` at 3 GiB and the default limit (its red log is `raw/final/ssh-3072-default.log`) passes after; record it in a new evidence run with the idle-machine CPU time if the host allows. Both ignored rows stay ignored.

**Gates**: fmt (+1.95.0), `scripts/checks/*.py`, boundary job, the ordinary leg, the transport-only leg with `GWZ_TEST_GIT=real`, the Windows lane check (the new rows must pass on Windows; if any is Unix-only it needs a row in `scripts/checks/windows_parity/<step>.json`; nothing in the design uses `getrusage`).

## 8. Documentation edits

§10.1: replace the P3-2(b) sentence ("...a pause of the client's own producer mid-pack is charged as `Network`...") with the new rule: once a push has started, a pause of the producer is `Backpressure` while the stream's owner is outside every call on it, up to the ceiling and only from the last byte; otherwise it is `Network`. Add the gate to the list of host-reported conditions, and the in-process limit with a pointer to open question 1.

## 9. Steps (each aspirationally under 500 lines)

1. Gate type (new file ~70), `BlockingStream` wiring (~25), `GitTurns::pack_started`, pump arm and ceiling (~60), pump rows and gate rows (~250). Red logs kept.
2. Plumbing through `UrlExtras`, `Entry`, `opening.rs`, `pump.rs`, `ssh_worker.rs` (~60), plumbing rows and the two end-to-end rows (~200), §10.1, evidence run for the trigger row.

## 10. Open questions for the operator

1. **In-process only for 1.1.0?** The gate covers the local placement (the 1.1.0 CLI). The `Cli` placement and a session host degrade to today's stall until a carried signal (section 5) is built. I recommend recording that for 1.2.0.
2. **The ceiling:** 600 s as proposed, or another number, and whether it should ever be a user setting. Without a value the fix has no bound for a hung producer, so a number is required.
3. A push open now always carries `UrlExtras`. If review prefers, the gate gets its own `Handoff` map instead, at about 30 more lines of deposit and drop handling.

Read loosely, not verified: that the `Cli` placement is outside 1.1.0's scope (read from `install_cli` and the release-scope notes only); the exact byte counts of libgit2's pre-pack writes beyond the evidence run.
