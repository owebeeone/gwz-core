# Push producer-busy signal: adversarial review, STATE axis

- **Object:** `gwz-core/dev-docs/GwzPushProducerBusyDesign-DRAFT.md` at gwz-core `b8cb3967` (119 lines), lane `/Volumes/projects/limbo/gwz-dev-blobfix`
- **Date:** 2026-10-18
- **Axis:** STATE (clocks, the fail-closed direction, races, lifetimes, stuck states)
- **Verdict: GO.** 0 P0, 0 P1, 0 P2, 6 P3.

## 0. Evidence base

- **Commit check.** gwz-core HEAD was `b8cb3967f2ff…` at the start and at the end. Its only untracked file, `dev-docs/GwzRemoteTransportBugReport.md`, was there at the start and is not mine. gwz-transport was `a27817b9` at both checks. I modified nothing.
- **Code read:**
  - `ssh_pump.rs` (in full)
  - `stream_io.rs` (`BlockingStream`, in full)
  - `git_turns.rs`
  - `ssh_handoff.rs` (in full)
  - `ssh_worker.rs`, `ssh_worker/runner.rs`, `ssh_worker/endpoint.rs:200-275`, `ssh_worker/open_request.rs`
  - `ssh_setup.rs:440-475`, `ssh_local.rs:55-135`
  - `driver/opening.rs` (in full), `driver/pump.rs:120-200`, `driver/backstop.rs`
  - `ssh_remote.rs`, `transport_binding.rs:90-240`, `https_worker/serve.rs:170-200`, `cli_placement.rs:1-130`
  - gwz-transport `src/stream/io_clock.rs` and `tests/io_clock.rs`
  - the vendored libgit2 (`smart_protocol.c:1155-1275`, `pack-objects.c:320-390, 1386-1391`, `remote.c:3045-3073`) and git2-rs `transport.rs:315-362`
- **Documents read:**
  - D42 (`GwzTransportTR18-OperatorDecisions.md:85`)
  - `GwzTransportSshPumpStall-ReviewState.md` (P3-2, P3-3, §2, Follow-up 1)
  - design §10.1 (`GwzRemoteTransportDesign.md:685-735`)
  - timeout plan §2
  - the README of run `2026-10-17-blobpush`
- **Facts confirmed against the code:**
  - libgit2 runs `git_packbuilder__prepare` before the command list (`smart_protocol.c:1233-1239`), so delta search falls in the pre-first-byte `Idle` window. Only `write_object`'s `git_odb_read`, `get_delta` and the per-1 MiB deflate fall after `pack_started`.
  - `git_remote_push` disconnects on both paths (`remote.c:3064-3070`), so `close` and `EndWrite` always follow the pack.
  - git2-rs `stream_write` is `write_all` (`transport.rs:352`). Each partial write would take its own guard.
  - The worker ticks every 1 ms while any exchange is active (`runner.rs:307-318`), so the arm cannot get stuck between ticks.
  - The open backstop runs only until `opened`. The aggregate connect clock covers only setup. No pool idle timer runs during a lease.
  - The Cli placement is compiled only under `gwz_session_candidate` (`cli_placement.rs:1-7`), so it is not in the 1.1.0 build.
  - `Backpressure` keeps `network_remaining_ms` (`io_clock.rs:29-36, 61-64`). `record_io_progress` refuses in `Backpressure` (`io_clock.rs:48-49`, `tests/io_clock.rs:56-57`).

## Findings

### P3-1: The bound is stated inexactly in three places

- **Root cause.** The text measures the ceiling from "the last byte" and calls the bound absolute. The state rule actually anchors it at the first eligible tick and restarts it on any byte.
- **Where.**
  - §3 line 49: `busy_since` is set "the first tick the arm is otherwise eligible".
  - §4 line 55: "measures wall time since the **last byte moved**".
  - §4 line 57: "at most ceiling + 9 s".
  - §6 lines 76-77.
  - Code: `ssh_pump.rs:277` (`write_forward` records progress for every byte libssh2 accepts) and `:306-307, :387-388` (server bytes record progress too).
- **Violated invariant.** The design text must be exact against the state machine. P3-2 of the stall review already showed a loose sentence turning into a fail-open "correction".
- **Credible sequences.**
  1. **Dead window.** The server dies silently mid-pack. Its SSH channel window (about 2 MiB for OpenSSH) still accepts writes, and each accepted chunk counts as progress, clearing `busy_since` and refilling the budget. Several large, compressible objects whose compressed bytes fit in the remaining window each get a full episode. Detection takes the sum of those pauses plus the stall limit, not ceiling + 9 s. The total is bounded by the window size, so this is bounded.
  2. **Chatty server.** A server byte mid-pause (stderr, or keepalives from a non-stock server) restarts the episode, and after the ceiling the state is `Network`, where server bytes refill the budget anyway. Against a chatty server, a hung producer is not bounded by the ceiling. If 64 KiB accumulate in `R`, the existing arm 1 (`ssh_pump.rs:469-476`) gives `Backpressure` with no ceiling at all. Both behaviours exist today; the design's universal wording hides them.
  3. **The NAT row (§6 line 77).** A silent NAT drop does not fail the next write. The write is absorbed by the window and socket buffers, then `WouldBlock`, `F` non-empty, `Network`, and a stall. The outcome is still bounded.
  4. **"A thread that died holding the stream" (§4 line 57).** That case is handled by `RemoteTransport::drop` and `cancel` (`ssh_remote.rs:107-117`), not by the ceiling. For a truly hung producer, the ceiling frees the lease, the pool slot and the server, but the user's operation returns only when the libgit2 thread does.
- **Impact.** A reviewer or implementer may over-read the guarantee. No new fail-open beyond the window-bounded chaining.
- **Correction.**
  - Either set `busy_since` in `record_progress` (an exact "last byte"), or restate §4 as "from the first eligible tick after the last byte".
  - State the bound per episode, noting that chaining is limited to what a dead peer's window absorbs.
  - Note that server bytes restart the episode, and fix the NAT and died-thread rows.
  - Say what the ceiling buys: release of the connection and slot, not a return to the user.

### P3-2: `pack_started` inherits GitTurns' early stop, so the arm can silently never apply

- **Root cause.** `client_started` is set only while the tracker is running: `client_bytes` returns early when `stopped` (`git_turns.rs:80-85`).
- **Where.** §3 condition 1 (line 40); `git_turns.rs:80-90, 120-125, 130-138` (stops on an `ERR` or non-v1 `version` line, malformed framing, or stderr at the client's turn).
- **Violated invariant.** The deliverable does not depend on Git framing. The gate is protocol-agnostic, but its enabling condition is not.
- **Sequence.** The server writes a stderr line after its advertisement while libgit2 builds the pack, so the tracker stops. Then the commands go out and a 3 GiB blob is read. `pack_started` is false, so the push gets `Network` and stalls at 9 s, exactly as today.
- **Impact.** Fail-closed, but the fix is silently absent with no diagnostic. That is a diagnosability gap.
- **Correction.** Derive "pack started" in the pump: set it on the first `Data` payload delivered on a pump whose `set_service` was receive-pack. Alternatively, set `client_started` before the `stopped` check. Add a row: receive-pack tracker stopped by stderr at the client's turn, then a producer pause, expecting `Backpressure`.

### P3-3: Two negative controls in the test plan cannot do what they claim

- **Root cause.** The rows are worded without the state machine's ordering and the window in mind.
- **Where.** §7 lines 91, 96, 98.
- **Problems.**
  - **(a) The dead-server end-to-end row (line 98).**
    - If "stops the fixture server" means killing it, the channel closes and the push fails with an I/O error or `server_done`/`Idle`, not at the ceiling.
    - If it means freezing it, a producer sleep of "several stall limits" (about 1 s at 300 ms) is shorter than the 2 s test ceiling. The producer wakes, the frozen server's window absorbs the rest of a small pack (counted as progress), and libgit2 enters `read`. The push then fails at sleep + stall, contradicting "not before the ceiling".
    - The row also tests a dead server with a live producer, while D42's ceiling is about a hung producer.
  - **(b) The "before the first client byte the state is still `Idle`" row (line 91).** It cannot kill the mutation that drops `pack_started`, because the `Idle` arm precedes the new arm (§3: "ahead of the final Network fallthrough").
  - **(c) No row pins "adds no WrongState" (§6 line 82)**, although `record_progress` and `set_io_state` both become `&mut self`. (`set_io_state` is `&self` today at `ssh_pump.rs:461`; the design does not mention that change.)
  - **(d) Condition 4 (`!server_done`) is redundant** given arm order, since the `Idle` arm at `ssh_pump.rs:477-484` already takes `server_done` with F=R=0. No mutation can kill it.
- **Impact.** The only end-to-end evidence for the fail-closed bound, the safety clause of D42, would pass or fail for the wrong reason. A mutation that weakens the fail-closed guard would survive.
- **Correction.**
  - (a): Freeze with `CutProxy::freeze_existing` (`cut_proxy.rs:212`) or `pause_process_tree` (`ssh_fixture.rs:576/634`, which has a Windows variant). Keep the producer outside every call for longer than ceiling + stall. Assert the failure between ceiling + stall − ε and ceiling + stall + ε after the last client byte. Add a hung-producer row that never wakes.
  - (b): Replace the row with one mid-advertisement: server's turn, consumer outside calls and armed, F=R=0, expecting `Network`.
  - (c): Add a row where a stdout byte and a stderr byte arrive in a tick that began in the new `Backpressure`.
  - (d): Say that condition 4 is redundant by ordering, rather than implying it has a row.

### P3-4: The trigger row and its red log are not committed, and the row's server setting is not stated

- **Where.**
  - §7 line 100, and the authority line 3.
  - `blob_push_tests.rs` is untracked in `gwz-dev-blobpush/gwz-core`, with `transport_host/mod.rs` modified there. It is absent from the blobfix lane.
  - The evidence run `2026-10-17-blobpush/` is untracked in the blobpush lane's `gwz-core-evidence` and absent from blobfix's.
  - The row sets `receive.unpackLimit=1` (`blob_push_tests.rs:85-95`).
- **Violated invariant.** Evidence durability (`EVIDENCE.md`). The red/green pair must be reproducible in the implementation lane.
- **Impact.**
  - If the blobpush lane is retired, the trigger and its red log exist only as copies in an uncommitted run.
  - A green trigger with `unpackLimit=1` does not show that a large push to a stock git server (fewer than 100 objects, so unpack-objects runs) succeeds. The README calls that server's silence a separate effect.
- **Correction.**
  - Step 2 should land the row from `runner/blob_push_tests.committed.rs.txt` and update its `#[ignore]` reason. Alternatively, commit the blobpush lane's row and evidence first.
  - State the `unpackLimit=1` condition in the trigger row and in §0's deliverable.

### P3-5: The `UrlExtras` equivalence claim cites the wrong evidence, and the ceiling plumbing is ambiguous

- **Where.** §2 step 1 (line 30), §2 step 3 (line 32), §4 line 59, §0 "May change".
- **What is wrong.**
  - `runner.rs:172-174` reads `helpers_allowed()`, not only `host()` and `password()`.
  - The setup consumers are `ssh_local.rs:67` (`is_none_or(helpers_allowed)`), `:73` (password), and `:86` (`map_or_else(key.host, url.host)`), plus the refusal at `opening.rs:200-208`.
  - Equivalence holds only if the push extras are built exactly as at `opening.rs:31`: `UrlExtras::new(written_host, password).with_helpers(helpers_allowed)`. `written_host` equals `key.host` whenever extras were absent before.
  - §4 says "the driver takes [the ceiling] from one runtime field", but `set_producer(…, ceiling)` runs in the endpoint worker. The value needs either a worker field (more than "one call" in `ssh_worker.rs`) or to ride in the gate.
- **Impact.** No defect today. The cited evidence would not catch a constructor that defaults `helpers_allowed` differently.
- **Correction.**
  - Cite the `ssh_local.rs` consumers.
  - Extend the plumbing row to assert the same written host, the same helpers path, the same pool identity and no partition.
  - Carry `ceiling_ms` inside the gate (the driver creates it), or name the worker field and widen §0.
  - Note that `set_io_state` becomes `&mut self`.

### P3-6: The 1.2.0 record is imprecise, and §10.1's `Backpressure` definition needs widening

- **Where.** §5 line 65, §10 question 1, §8.
- **What is missing.**
  - The record names no place where it lives (the release-scope record or the decisions table).
  - A carried form "sent … when its gate flips" would send two messages per stream call. It must be edge-filtered or debounced.
  - The ceiling must stay endpoint-local: an endpoint must not trust a remote initiator's busy claim without a bound.
  - The ordering of a busy message relative to `Data` must be specified.
  - §10.1 defines `Backpressure` as "consumer pressure, bounded message queues or exhausted credit" (`GwzRemoteTransportDesign.md:699-701`). A busy producer is none of those, and §8 only swaps the P3-2(b) sentence.
  - The "read loosely" note at line 119 can be retired: `cli_placement.rs:1-7` confirms the Cli placement exists only in the session candidate.
- **Correction.** Record the 1.2.0 item with these four constraints and a trigger row (a blob push over the Cli placement). Widen the §10.1 definition. Cite `cli_placement.rs`.

### Outside scope (not blocking)

HTTPS reports `Backpressure` while it awaits the producer, with no ceiling (`https_worker/serve.rs:186`). A hung HTTPS push producer therefore holds the HTTPS stall clock open indefinitely. D42's ceiling rationale applies there too. This is a limitation, not a demonstrated defect; record it for the operator.

## Attacks that failed

- **`in_call` as the oracle.**
  - During `git_odb_read`, `get_delta` and deflate, libgit2 makes no stream call. `write_all` loops `write`, and its brief zeros between iterations cost at most one 1 ms tick with the budget kept.
  - A `read` satisfied from libgit2's own buffer is not a network wait.
  - `close` sends `EndWrite` (`ssh_remote.rs:99-101`), so `client_ended` turns the arm off for good.
  - Without report-status, libgit2 skips `parse_report`. The local `update_tips` work before disconnect is correctly `Backpressure`.
  - RAII guards drop on `Err` and on unwinding; git2-rs `panic::wrap` catches inside the Rust frame.
  - A dropped stream cannot be mid-call. A lost wakeup in `wait()` leaves `in_call=1`, which is `Network` (fail-closed).
- **Transit buffering.**
  - Bytes in flight between the driver `Stream`, the mux and the bridge (`sync_channel(16)`) show `F` empty for a few ms, which is local and harmless.
  - A credit stall puts the producer inside `write`, which is `Network`.
- **WrongState.** The new state is computed at tick start after `deliver` (`ssh_worker.rs:398-409`). `write_forward` cannot record progress while `F` was empty, and the read and stderr paths call `report_network` first. This holds; only a row is missing (P3-3(c)).
- **Ceiling interactions.**
  - The aggregate connect clock (setup only), the open backstop (only until `opened`) and the pool idle timer (not running under a lease) do not interact.
  - Cancellation is independent of the I/O state (`ssh_worker.rs:395-397, 438-441`).
  - With `--ssh-timeout 0` there are no deadlines.
  - Backward ticks: the clock is monotonic `Instant`; use `saturating_sub`.
- **Stuck states.**
  - A gate is never armed before the advertisement read returns, so it reads `Network`.
  - Fetch is excluded twice: the driver creates a gate only for `ReceivePack` (`ReceivePackLs` maps there, `ssh_remote.rs:64-67`), and the pump requires `!fetch`.
  - The gate is per open, per stream. An `Unserved::Dead` re-attach reuses the same request and gate before any `BlockingStream` exists. `start_exchange` builds a fresh pump, so no gate leaks across exchanges on a reused connection, because it comes from `request.url`, not the setup's `Opening`.
  - The two separate loads (`armed`, then `in_call`) can race only before the first client byte. Packing both into one atomic would remove the race.
- **The `UrlExtras` side effect.** Pool identity (`runner.rs:156-186`), the helpers partition, the hashed known_hosts name and the password path are unchanged for a push open with no password, helpers allowed and the same written host (P3-5 covers only the citations).
- **The in-process limit.** It is fail-closed (no gate means `Network`) and precise for 1.1.0 (`cli_placement.rs:1-7`).

## Views on the design's decisions

- **In-process only for 1.1.0: agree.** It is fail-closed and exactly matches the 1.1.0 build, and a carried form is a wire change that needs its own dual review. Record it with the P3-6 constraints.
- **The 600 s ceiling: acceptable.** It is the operator's number. In practice it fires only for a hung producer, since `git_odb_read` needs the whole object in RAM; that is about 69 s for a 6 GiB object on a Pi without swap. Its value is releasing the connection, the site slot and the server, not returning control to the user. A swapping slow host could exceed it and would get a stall message that names `--ssh-timeout` after 10 minutes. Consider flagging a ceiling expiry in the facts or the message.
- **`UrlExtras` against a separate map: prefer the separate `Handoff` map.**
  - `UrlExtras` carries a secret with wipe semantics.
  - Three consumers branch on whether it is present (`is_some_and`, `is_none_or`, `map_or_else`).
  - The handoff already keeps per-(session, stream) side maps (`ends`, `holds`, `trust`) with the same lifecycle.
  - Keeping "extras absent means the default path" removes a future hazard for about 30 lines. This is not a blocker, because equivalence holds today.

## Next action

GO. Fold P3-1 through P3-6 into the draft before step 1, chiefly the restated bound, the realizable dead-server and hung-producer end-to-end rows, the replacement for the ordering-immune `Idle` row, and landing the trigger row. Then put the operator's open questions 1 to 3 to the operator. No re-review is needed for P3-only revisions.
