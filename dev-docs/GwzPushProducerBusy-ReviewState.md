# Push producer-busy signal, implementation: adversarial review, STATE axis

- **Object:** gwz-core `86bb198c`, one commit over `b8cb3967`: 19 files, +1780/−104. Lane `/Volumes/projects/limbo/gwz-dev-blobfix`. Evidence run `gwz-core-evidence/.../runs/2026-10-18-blobfix/` (`42759dee`).
- **Date:** 2026-10-19
- **Axis:** STATE (clocks, the fail-closed direction, races, atomics, lifetimes, stuck states, test validity)
- **Verdict: GO.** 0 P0, 0 P1, 0 P2, 5 P3.

## 0. Evidence base

**Tuple, checked at start and at end, unchanged:**
- gwz-core `86bb198c`. Its only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`, which was already there and is not mine.
- gwz-core-evidence `42759dee`. Its only untracked item is the older `2026-09-22-alpha-setup-timeout/`.
- gwz-transport `a27817b9`.
- root `e7a87556`.

I modified nothing in the lane.

**How I tested.** All builds and mutants ran on a `git archive 86bb198c` copy in my scratchpad, with the sibling repos symlinked. The target was `/Volumes/projects/limbo/build-targets/reviewd42i-mut`, built with `--cfg gwz_transport_candidate` and `--locked`. The target and both scratch copies are deleted. I did not run `run_r4bg_aggregate_gates.py`.

**Code read:**
- The full diff.
- `ssh_pump.rs`: `deliver`, `advance`, `tick_inner`, `write_forward`, `read_reverse`, `drain_stderr`, `record_progress`, `set_io_state`, `producing`.
- `stream_io.rs` in full.
- `git_turns.rs`.
- gwz-transport `stream/io_clock.rs`.
- `ssh_worker.rs` (`attach`, `transfer`, the clock at `:500`).
- `ssh_setup.rs:440-470`: a fresh `SshPump` is built for every exchange.
- `shared_reservation.rs`, `ssh_remote.rs`, `driver/opening.rs:155-345`, `driver/pump.rs`, `session.rs`.
- All the new rows.

**Documents read:**
- D42 (`GwzTransportTR18-OperatorDecisions.md:85`).
- The design `GwzPushProducerBusyDesign-DRAFT.md`.
- `GwzPushProducerBusyDesign-ReviewState.md`, P3-1 to P3-6.
- The §10.1 diff.
- Both run READMEs: `2026-10-18-blobfix`, and `2026-10-17-blobpush` in the blobpush lane.

**Runs on the commit's code** (a copy built with my mutation switch, which with `GWZR_MUT=none` behaves as the commit):
- `ssh_pump_clock_tests`: 25/25 pass.
- Filter `producer` (pump, gate, handoff and the 5 real-sshd end-to-end rows): 31/31 pass.
- `ssh_handoff`: 7/7 pass.

**My own mutants:**

| Mutant | Rows that went red |
|---|---|
| pack start = `!fetch && turns.stopped` | **none of 25 + 31: survives** |
| pack start = `turns.client_started` | `the_pack_having_begun_does_not_depend_on_the_turn_tracker` |
| ceiling test `<=` instead of `<` | 4 rows |
| no `last_byte` set in `deliver` | none (redundant: `write_forward` sets it in the same tick) |
| no guard in `close` or `end_write` | none |

The candidate row in F1 kills the surviving mutant and passes on the real code.

**Source checks on the clean archive** (`python3 -B`):
- `check_cfg_boundaries` (`--skip-repo` gwz-cli and gwz-py, since the copy has neither): nothing new.
- `check_process_globals`: nothing new.
- `check_windows_parity`: unported 1, nothing new.
- `check_candidate_switches`: OK.
- `check_filesystem_boundary`: OK.
- `test_consumer_bridge_standalone.py`: OK.

**Windows.**
- I recomputed the receipt's code-only digest from the commit (the modified files' diff plus the added files, in the format of `runner/digest.sh`). It gives `4507bc76…`, which equals `digest-at-staging.txt`, so `blobfix-20261018-a` tested exactly `86bb198c`'s code.
- I read the hosted-like log on dabeest (`/e/gwz-tests/blobfix-20261018-a/logs/hosted-like.log`, read-only grep). It shows every new pump, gate, handoff and end-to-end row as `ok`, and the two trigger rows as `ignored`.

## Findings

### P3-1: The "pack start is not read from `GitTurns`" property has one positive row; the stopped-tracker mutant survives

- **Root cause.** The only row for this property, `the_pack_having_begun_does_not_depend_on_the_turn_tracker`, checks one direction: a stopped tracker must not hide a pack that began. Nothing checks the other direction: a stopped tracker must not *start* the arm before any client byte. The row also never asserts its own precondition (that the stderr line actually stopped the tracker).
- **Location.** `ssh_pump_clock_tests/producer.rs`, the tracker row. `ssh_pump.rs` `producing()` (`self.pack_sent`).
- **Violated invariant.** Design §6: the arm needs "a first client `Data` … so … the wait for the first byte [is] unchanged."
- **Credible interleaving** (a later refactor, such as "use the tracker's stop as pack start"):
  1. After the advertisement, the server writes a stderr line at the client's turn, so `GitTurns` stops.
  2. libgit2 runs delta search, outside every call, so the gate reads busy.
  3. The server freezes.
- **Impact.** Today the state is `Network` and the stall fires at the stall limit. Under the mutant it is `Backpressure` for up to 600 s. Every existing row stays green (verified). The weakening is bounded, so this is P3.
- **Correction.** Add the row below, and in the existing tracker row assert that a tick between the stderr line and `send_head` is `Network`, not `Idle`, which proves the tracker stopped.
- **Regression test.** Already built and run: green on the commit, red under `turns_stopped`.

  ```rust
  let gate = busy_gate(600_000);
  let mut exchange = push_at_the_clients_turn(Some(&gate));
  exchange.server_writes_stderr(b"remote: hello\n");
  exchange.tick(START - 1);
  assert_eq!(exchange.tick(START), IoState::Network);
  exchange.tick(START - 1 + STALL_MS - 1);
  assert!(!exchange.pump.stream_stats().terminal);
  exchange.tick(START - 1 + STALL_MS + 1);
  assert!(exchange.timed_out());
  ```

### P3-2: The shipped ceiling (D45's 600 s) is pinned by no test; the `close`/`end_write` guards are untested

- **Root cause.** Every end-to-end rig calls `set_producer_ceiling_ms_for_test`, including the three that pass 600 000 explicitly (`producer_busy_tests.rs:117`). So no test reads `Session::new`'s default (`session.rs:497`) or `PRODUCER_CEILING_MS` (`stream_io.rs:18`). The gate rows exercise `read` and `write` only; removing the guard from `close` and `end_write` leaves all 31 rows green (verified).
- **Violated invariant.** D45 (the ceiling is 600 s) and D42 ("with a ceiling so a hung producer cannot hold the clock open"), in the configuration that actually ships.
- **Interleaving.** An edit sets the default to `u64::MAX`, or to a test value such as 2 000. Every row passes, the hung-producer bound disappears from the product, or real large pushes fail at 2 s plus the stall limit.
- **Impact.** No defect today. The guards on `close` and `end_write` are made redundant by `client_ended` and `close_received` once the message is delivered; they matter only for the in-transit millisecond.
- **Correction and regression test.**
  - A unit row: a fresh `Session` (no setter) opens an SSH push, and `Entry.producer`'s `ceiling_ms() == 600_000`. Or simply `assert_eq!(PRODUCER_CEILING_MS, 600_000)` beside a row that drives the default path.
  - A gate row: a thread blocked in `close()` on a gated stream reads `!busy()`.

### P3-3: The trigger evidence is incompletely durable and has no build provenance

- **Location.** Blobfix README "Tests" table and `runner/run-trigger.sh`. Blobpush lane `gwz-core-evidence`.
- **What is wrong.**
  - The red log `2026-10-17-blobpush/raw/final/ssh-3072-default.log` is still **untracked** in `/Volumes/projects/limbo/gwz-dev-blobpush/gwz-core-evidence` (`?? .../2026-10-17-blobpush/`) and absent from the blobfix lane. State review P3-4 is only half closed.
  - The green trigger ran a binary (`blobfix-t/.../gwz_core-0dbc0b6194d2fc17`) whose `GWZ_BUILD_PROVENANCE` source digest was never recorded, and its build directory is deleted. The trigger ran at 05:39Z, before the 06:19Z Windows staging digest. That the green trigger came from `86bb198c`'s code therefore rests on the narrative only.
  - Per-test Windows results exist only on the host. The archived receipt holds counts, not names.
- **Violated invariant.** `EVIDENCE.md` durability: the red and green pair must be reproducible and attributable.
- **Impact.** If the blobpush lane is retired, the red half is lost. A green trigger cannot be tied to the committed tree.
- **Correction.**
  - Commit the blobpush run, or copy its red log into the blobfix run.
  - Record `GWZ_BUILD_PROVENANCE`, or `digest.sh`'s output, in the trigger's `.env`.
  - Archive a `grep producer` extract of the Windows hosted-like log.
- **Regression.** A runner that refuses to run the trigger without writing the provenance line.

### P3-4: The design text and §10.1 do not match the code

- **Locations and corrections.**
  - **§7, end to end.** The design says "`--ssh-timeout` 300 ms … test ceiling 1 s". The landed rows use `STALL_MS = 1_000` and `CEILING_MS = 2_000` (`producer_busy_tests.rs:24-26`), yet the status line says "the rows below are the ones that landed". Correct §7 to 1 s and 2 s.
  - **§10.1.** It says "after its first **pack** byte". The code starts at the first client `Data`, which is the command list, before the pack (`ssh_pump.rs:207-211`; design §3 says the same). Change §10.1 to "after the client's first byte (its commands)".
  - **`session.rs:360-361`.** The doc comment "Has this driver's push opens carry a producer ceiling…" sits on `producer_gates_for_test`, and `set_producer_ceiling_ms_for_test` has none. Move the line down.
- **Impact.** A later reader or reviewer works from wrong numbers. This is the P3-1 class of risk from the state review. No behavioural effect.

### P3-5 (outside scope, for the operator): the `receive.unpackLimit=1` restriction rests on an unmeasured claim

- **Where.** Design §0 says a stock server "is silent after the last byte for the same order of time". The trigger ran only with `unpackLimit=1`. `GWZ_BLOBPUSH_SERVER_DEFAULTS` was never run at 3 GiB. The blobpush evidence for the claim is a 64 MiB HTTPS run under load 108.
- **Hypothesis (unverified).** As I recall, modern git's `unpack-objects` streams blobs above `core.bigFileThreshold` (512 MiB) as they arrive. If so, a multi-GiB push to a stock server may not be silent after the last byte, and the restriction may be unnecessary. It may also be the reverse. Either way the 1.1.0 notes would state it wrongly.
- **Impact.** Scope wording only. Not a defect of this change and not blocking.
- **Suggestion.** One run of the trigger with `GWZ_BLOBPUSH_SERVER_DEFAULTS=1`.

## Attacks that failed

**1. Implementation against the design.**
- The arm (`ssh_pump.rs:535-543`) is exactly the five conditions:
  - `pack_sent` (first non-empty client `Data`, `!fetch`);
  - `!client_ended`;
  - `forward` and `reverse` empty;
  - `gate.busy()`;
  - `now.saturating_sub(last_byte) < ceiling`.
- It sits after `Idle` and before `Network`.
- `last_byte` moves on the first `Data` and in `record_progress` for every channel byte: accepted toward the server (`write_forward`), and server stdout and stderr (`read_reverse`, `drain_stderr`). Client `Data` after the first sits in `F`, so the state is `Network` until a write records progress. There is no gap.
- `now = max(now, given)`, fed by `origin.elapsed()` (`ssh_worker.rs:500`): it is monotonic.
- `transfer` advances before `deliver`, so the first `Data` stamps a fresh `now`.
- **No `WrongState`.** `write_forward` records only from a tick that began in `Network`; the read paths call `report_network` first. The row for this passes.
- **Budget.** `io_clock.rs` keeps `network_remaining_ms` through `Backpressure` and `Idle`, with no refill, so the bound is `last_byte + ceiling + kept`, with kept at most the stall limit.

**2. Atomics.**
- `busy()` is one `Acquire` load; `enter` is `fetch_add`; the guard drop is `fetch_update((c|ARMED)-1)`.
- Only one word is shared and nothing is published through it, so the orderings are sufficient.
- Overflow needs 2^31 concurrent calls.
- An underflow is impossible, since a guard exists only from `enter`. If one did happen, it would clear `ARMED`, which fails closed.
- The guard is held across `wait()` and `map_err` on all five blocking methods, and drops on `Err` and on unwinding (rows pass). `cancel` and the accessors do not block, so they need none.

**3. Plumbing.**
- **Deposit.** It is made under the extras lock (lock order extras, then producers; `take_producer` takes producers alone), so the endpoint's `take`, which waits on that lock, orders `take_producer` after the deposit.
- **Failed open.** It deposits nothing (row).
- **Never taken.** `Deposit::drop` removes the entry (row). The capacity retry loop re-deposits the same, still unused, gate only after an `open` that failed, which deposited nothing.
- **Double take.** It returns `None` (row).
- **Re-attach.** An `Unserved::Dead` re-attach reuses the request's clone on a fresh pump before any `BlockingStream` exists.
- **Pool reuse.** `start_exchange` builds a new `SshPump` for every exchange, so no gate crosses exchanges. A push after a fetch is reused (row).
- **Placement.** The gate is made only for SSH `ReceivePackExchange` with a handoff. The only production `ReceivePack` producer is libgit2 through `ssh_remote.rs`.

**4. The fail-closed rows.**
- **Frozen server and hung producer.** Both assert that the failure falls between `ceiling + stall − 150 ms` and `+ 3 s`, measured from an instant at or before the pump's last byte, so the lower bound is conservative. Both were red at 1.1 s against the stub, and a no-ceiling mutant would hit the `expect`.
- **What the frozen row adds.** Its pump inputs equal the hung-producer row's (`F = R = 0`, busy, no bytes). It adds the stall wording on wake. The "dead server, live producer" case is pinned at pump level (`bytes_queued_…`).
- **The tracker row.** It is sound for the `client_started` mutant (verified red), not for `turns.stopped` (P3-1).

**5. The 3 GiB trigger.** It is valid evidence for its stated claim:
- the red and green runs used the same row and the same `unpackLimit=1`;
- the green run shows a 17.1 s producer pause against a 9 s limit, then success;
- the mechanism is platform-neutral, and the gate and arm are portable code;
- the Windows small end-to-end rows ran against `sshd.exe`.

Running on macOS only is not a defect; P3-3 and P3-5 cover provenance and scope.

**6. Windows.**
- No new `cfg(unix)`, `cfg!(windows)` or OS call, so the parity inventory needs no entry ("nothing new").
- The new rows ran on Windows and passed (host log).

**7. Repository rules.**
- `cfg_if` is used only for the new module declarations; there are no conditional imports.
- Every control-flow body is braced.
- There are no globals or thread-locals (the process-globals check finds nothing new).
- `ssh_pump.rs` is 641 lines; the largest changed file is `session.rs` at 673. All are under 1000.

**8. Stuck states.**
- The arm expires on its own as `now` advances, and the worker ticks every 1 ms.
- Only a byte moves `last_byte` (the calls row and its mutation).
- A chatty server and the full-`R` arm (with no ceiling) are pre-existing behaviour and documented in §4.

## Next action

GO. Fold P3-1, P3-2 and P3-4 into a follow-up commit in the lane: the stopped-tracker row (code above), the default-ceiling row, the `close` gate row, and the §7, §10.1 and doc-comment fixes. For P3-3, commit or copy the blobpush red log, record trigger provenance on the next trigger run, and archive the Windows per-row extract. Give P3-5 to the operator as an optional single stock-server trigger run. No re-review is needed for P3-only changes.
