# SSH pump stall-clock fix: State-axis review

Status (lane owner, 2026-10-16): **accepted. The fix `d9d6ad47` reported GO here; the three P3s were closed in `d94d3816` with the closure rows this report names, each new Network-guard row shown red against a mutated pump (the reviewer pre-committed to GO on such a revision; P3s, so no re-verdict was run). Windows receipt `r19stall-20261016-c`. Follow-up 1 (a push producer pausing mid-pack is charged as Network) is an operator decision, with a large-blob push row as its trigger test; Follow-up 2 (pool idle measured from release) is recorded.**

**Review object:** gwz-core `git diff c570e45c..d9d6ad47` (one commit). It changes `SshPump::set_io_state` (`src/git/endpoint/ssh_pump.rs:461-490`), adds four rows and fixture support in `src/git/endpoint/ssh_pump_clock_tests.rs`, and edits design §10.1 (`dev-docs/GwzRemoteTransportDesign.md:704-714`, `:801-818`).
**Baseline:** gwz-core `c570e45c`. Reference only: gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252`. Out of scope: lane root `66176961b7914de16480e09a32a604d2179d291b`.
**Date:** 2026-10-16
**Axis:** State (state machines, races, fail-closed direction, stuck states)

**Verdict: GO.** 0 P0, 0 P1, 0 P2, 3 P3.

## 0. Evidence base

- **Tuple.** Checked at the start and at the end of the review in `/Volumes/projects/limbo/gwz-dev-r19stall`: lane root `66176961…`, gwz-core `d9d6ad47…`, gwz-transport `a27817b9…`. All three were unchanged. The untracked files in the lane root and in gwz-core were the same at both checks.
- **Sources read in full or in the relevant part:**
  - `ssh_pump.rs`, all of it.
  - `ssh_pump_clock_tests.rs`, all of it.
  - `git_turns.rs:1-250`.
  - `ssh_channel.rs:60-300`.
  - `ssh_worker.rs:160-500`, `ssh_worker/held.rs` and `ssh_worker/runner.rs:270-310`.
  - `transport_host/session/driver.rs:1-60`, `driver/opening.rs:270-340` and `driver/pump.rs:245-300`, for the open backstop.
  - `https_worker/serve.rs:170-275`.
  - gwz-transport `src/stream/io_clock.rs`, all of it, and `src/stream/machine.rs:280-390`.
  - Design §10.1 as edited, timeout plan §2 (lines 30-72), and `GwzTransportIdleLossDesign.md` §1-§3.
  - The evidence README `gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-16-r19stall/README.md`.
- **libssh2 behaviour.** I checked the vendored libssh2 in `libssh2-sys-0.3.3` (the version in the lock file), `libssh2/src/channel.c:2088-2104` and `:2222`. The receive window is charged when the application *reads* bytes, not when they arrive. So a read on stderr does not reopen the window while stdout is unread, and the server really is blocked by our window when the reverse buffer is full. This agrees with the run's TCP measurement (README §2.1).
- **Tests.** Candidate tree prepared with `prepare.py /Volumes/projects/limbo/build-targets/reviewstall-cand`. Ran `RUSTFLAGS='--cfg gwz_transport_candidate' CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewstall-target GWZ_TEST_GIT=real cargo +1.95.0 test … --lib -- ssh_pump git_turns`.
  - Result: **19 passed, 0 failed.** That is the 7 `ssh_pump_clock_tests` rows (the 3 existing ones and the 4 new ones) plus the 12 `git_turns_tests` rows.
- **Source checks** (`python3.13 -B scripts/checks/*.py` in the lane's gwz-core):
  - These passed: cfg boundaries ("nothing new"), process globals ("nothing new"), Windows parity ("nothing new"), candidate switches, crate versions, build identity, bazel pin drift, filesystem, checked-artifact, local-clone, merge docs, target selection, and `run_boundary_job.py` (4 tests OK).
  - `check_merge_compatibility_predicates.py` and `check_windows_receipts.py` need arguments and were not run.
- **Repository rules on the diff.** Braced control flow: every new `if`/`else if` body is braced. No `#[cfg]` was added. No global was added.
- **Read-only incident (mine).** The same `scripts/checks/*` loop also launched `run_r4bg_aggregate_gates.py`. That script runs `cargo test` with the lane's default target directory. I stopped it within minutes, by its task id.
  - It wrote build output under `gwz-core/target/` in the lane: for example `target/.rustc_info.json` at 08:41:59 and object files under `target/debug/deps/`.
  - No tracked or untracked source file changed; the `git status` of both repositories is identical to the start.
  - This broke the rule "test runs must use an external CARGO_TARGET_DIR". The lane owner may want to delete or keep that build output; I left it untouched.

## Problem, requirement provenance, and scope

- **Requirement.**
  - Timeout plan §2: the stall clock measures network inactivity, and user interaction spends neither clock.
  - Design §10.1: `Network` only while an outstanding backend operation can make peer progress. `Backpressure` and `Idle` keep the remaining budget.
- **Defect being fixed** (evidence README, Verdict 1 and 2):
  1. A full reverse buffer behind a consumer that has stopped reading was charged as `Network`, because the `Backpressure` arm required `stderr_eof`.
  2. Local work after the server's last byte was charged as `Network` until Close.
- **Scope of the object.** One function, `set_io_state`, its tests, and the §10.1 text. Nothing in the change touches the push producer path, pool policy, the retry machine or the close machinery. Those are reviewed here only for how they interact with the new states.
- **Provenance.** The fix traces to the plan §2 requirement and the measured triggers (README §2.3-2.4b). No scope widening was found.

## 1. Findings

### P3-1: At the client's turn, untaken bytes below the buffer cap still report `Network` (a fail-closed false stall that contradicts the edited §10.1)

- **Root cause.** `ssh_pump.rs:469-485`.
  - The `Backpressure` arm needs `reverse.len() >= mirror_cap`, or `server_done && !reverse.is_empty()`.
  - The `Idle` arm needs `reverse.is_empty()`.
  - Case: `forward` empty, `0 < reverse.len() < mirror_cap`, `turns.clients_turn()` true, server not done. This falls through to `Network`.
- **Violated invariant.** Edited §10.1 (`GwzRemoteTransportDesign.md:704-707`): "A direction is eligible only if the exchange is waiting on it". At the client's turn the exchange is not waiting on the server's standard output. The only pending work is bytes the consumer has not taken, which is the definition of `Backpressure`.
- **Interleaving.**
  1. A protocol v0/v1 upload-pack advertisement of more than 64 KiB (window) plus less than 64 KiB (pump buffer) arrives.
  2. Its flush sets `advertised`, and `waiting_for_client` is true.
  3. The consumer takes 64 KiB and pauses for longer than `--ssh-timeout`.
  4. `read_reverse` keeps polling, because the buffer is below cap. The server is silent because it is the client's turn, so the state is `Network`.
  5. The stream fails with `Timeout` at the last advertisement byte plus the limit.
- **Contrast.** The same pause with the tail of 64 KiB or more is `Backpressure` and survives. The reported state depends on a byte count, not on whose turn it is.
- **Impact.**
  - It only ever fails closed: progress is lost, and a dead peer is never hidden.
  - Reach is narrow. libgit2 drains the advertisement in `git_smart__connect` without calling back into gwz, so none of the measured triggers (stdout sink, terminal, slow pack write) fire there. ACK/NAK rounds are far below 64 KiB.
  - The run's own coverage is the full-buffer case and the server-done case. This one is the remaining gap of the same class.
- **Correction.** Add a `Backpressure` case: `forward.is_empty() && !reverse.is_empty() && turns.clients_turn()`. This is safe because `clients_turn()` is true only after the server has positively finished its turn.
  - Alternatively, if this is to be left as is, say in §10.1 that untaken bytes below the cap at the client's turn are charged as `Network`.
- **Closure test.** Add an `ssh_pump_clock_tests` row:
  1. Advertisement padded to about 100 KiB, consumer grants no window.
  2. Tick 64 times, then tick at plus 10 × `STALL_MS`.
  3. Assert the state is `Backpressure` and the stream is not terminal.
  4. Then grant the window, drain, and tick at plus `STALL_MS` + 1. Assert the state is `Idle` and not terminal (it is still the client's turn).

### P3-2: The edited §10.1 normative text is looser than the code in two places

- **Where.** `GwzRemoteTransportDesign.md:707-713`.
  - (a) "When both of the server's outputs have ended … report … `Backpressure` if bytes remain that the consumer has not taken." The code (`ssh_pump.rs:469-470`) also requires `forward.is_empty()`. With client bytes still queued toward an ended server it reports `Network`. That is the correct, fail-closed choice: a write to an exited server must be able to time out. The text, read literally, prescribes `Backpressure` there.
  - (b) "(and, for a push, the client's own pack bytes toward the server)". Read with "eligible only if the exchange is waiting on it", this implies a push whose producer pauses mid-pack with nothing queued is not `Network`. The code is `Network`: `GitTurns` stops at the client's first receive-pack byte (`git_turns.rs:87-90`), so `clients_turn()` is false and `server_done` is false. The 2026-09-29 note (`:791`, "On a push only the wait before the client's first byte is `Idle`") states the real behaviour. The new sentence blurs it.
- **Violated invariant.** The text must be exact against the code (review area "the design text").
- **Impact.**
  - A future change that follows (a) literally would turn a write to a dead server into a hang. That is fail-open.
  - (b) misstates a known, unfixed local-wait charge (Follow-up 1).
- **Correction.**
  - (a): add "and nothing is queued toward the server" to the server-done sentence.
  - (b): say explicitly that once a push has started, a pause of the client's own producer is charged as `Network`, because the pump cannot tell a paused producer from a finished pack that is waiting for report-status.
- **Closure.** Text review against `ssh_pump.rs:469-485`. Note that the implementation note at `:812-816` already carries the forward condition through "also".

### P3-3: No test pins the conditions that must stay `Network` under the new arms

- **Where.** `ssh_pump_clock_tests.rs`. Every new row asserts that the clock stops. The existing stall rows cover only `reverse` empty with the server's turn. No row covers any of these:
  - (i) `forward` non-empty with `reverse` full: a push or negotiation where the server stops reading.
  - (ii) `reverse` partly full, consumer paused, server alive but silent at its own turn.
  - (iii) `stdout_eof` without `stderr_eof`, or the reverse. Not real over libssh2, where EOF is per channel, but it is the guard `server_done` relies on.
- **Violated invariant.** The fail-closed direction: a defect may lose progress but never hide a dead peer. Today it is protected by the code alone.
- **Reproduction (reasoned, not mutated, since the review is read-only).**
  - Dropping `self.forward.is_empty()` from either new arm passes all 7 rows.
  - Weakening `server_done` to `self.stdout_eof` passes all 7 rows.
  - Both edits are fail-open.
- **Impact.** A regression to masking a dead server would pass the gate.
- **Correction and closure.** Add three rows:
  - (i) The fake `write_backend` returns `WouldBlock` once forward holds bytes. Fill the reverse buffer, then assert `Network` and a timeout at the limit.
  - (ii) Write 10 KiB of pack after `done`, grant no window, then let the server go silent. Assert `Network` and a timeout at the last byte plus `STALL_MS`.
  - (iii) A fake whose stdout returns 0 while stderr returns `WouldBlock`. Assert `Network` and a timeout.

## 2. Invariant analysis (attacks that failed)

**State table for the new function.** Close-received returns early. Abbreviations: `F` = forward queued toward the server, `R` = reverse bytes untaken, `cap` = 64 KiB, `SD` = stdout and stderr both at EOF, `CT` = `clients_turn()`.

| F | R | SD | CT | Reported | Correct? |
|---|---|---|---|---|---|
| >0 | any | any | any | Network | Yes. Bytes toward the server can only stall on the peer (or on an exited server, which must time out). |
| 0 | ≥cap | no | any | Backpressure | Yes. No stdout read happens. libssh2's window falls only on application reads (`channel.c:2222`), so the server is blocked by us. Any consumer read drops R below cap and the next tick is `Network`. |
| 0 | ≥cap or 0<R<cap | yes | any | Backpressure | Yes. Nothing more can come from the peer. |
| 0 | 0<R<cap | no | no | Network | Yes. The pump is reading and the server owes bytes; silence is the peer's. |
| 0 | 0<R<cap | no | yes | Network | Fail-closed but wrong: P3-1. |
| 0 | 0 | yes | any | Idle | Yes. See the post-pack analysis below. |
| 0 | 0 | no | yes | Idle | Unchanged from before. |
| 0 | 0 | no | no | Network | Yes for fetch. Push mid-pack: Follow-up 1. |

- **Stderr.** A stderr byte still reaches the clock: `drain_stderr` runs on every pass, and `report_network` is called before `record_progress` (`ssh_pump.rs:386-388`).
  - So `Backpressure` with stderr open never refuses progress (no `WrongState`). A chatty stderr refills the budget, and the next tick returns to `Backpressure`.
  - Dropping the `stderr_eof` guard is sound for SSH: extended data shares the channel window, so a full stdout backlog blocks stderr too.
- **Ordering within a tick.** The state is computed at tick start from the previous tick's buffers (`tick_inner:526`, before any I/O).
  - `deliver` runs only between ticks (`ssh_worker.rs` `transfer`), so `forward` cannot become non-empty inside a tick. `write_forward` therefore only ever records progress while the state is `Network`.
  - The read and stderr paths switch to `Network` before recording.
  - No path produces `WrongState`.
- **How long `Backpressure` can last.** `R ≥ cap` can only be reached through a positive read, and that read refills the budget. It persists only while the consumer takes nothing.
  - With a trickling consumer, `read_reverse` runs before `write_reverse`, so the next tick starts below cap and is `Network`.
  - Against a dead server, a refill can only come from bytes libssh2 already holds, which are bounded by the window.
  - No sequence keeps `Backpressure` alive while the consumer reads and the peer is silent.
- **Stuck states examined.**
  - *Server dead after the buffer fills, consumer resumes later.* `Backpressure` keeps the remaining budget (`io_clock.rs:23-37, 56-67`). After the resume, R drops below cap, the state is `Network`, and the buffered bytes are drained as real progress. The stream then times out at the limit after the last byte. The row `after_a_long_consumer_pause_a_silent_server_still_times_out` checks this:
    - after 10 × `STALL_MS` of pause it is not terminal;
    - after a drain it is not terminal;
    - at `STALL_MS` + 2 it is terminal with exactly `[Timeout]`.

    The lower edge of the boundary is not pinned. That is acceptable, because a budget refill on resume is harmless and a budget charged during the pause would be caught by the not-terminal assertion.
  - *Half-closed channel.* Over libssh2 `libssh2_channel_eof` hides EOF while either stream has data, so the two EOF flags converge. A one-sided EOF keeps `server_done` false and the state `Network` (untested: P3-3).
  - *Window full and server dead, consumer never resumes.* `Backpressure` lasts indefinitely. That is a local hang of the consumer, which plan §2 does not charge. Any resume restores `Network` with the preserved budget.
- **Post-pack `Idle`.** What can still be pending after `SD`:
  - *Bytes toward the server:* `F > 0` gives `Network`.
  - *The client's EOF* (`send_eof` `WouldBlock`): nothing waits on it before Close. After Close, `finish_channel` needs `end_sent`, which is bounded by the stream's close deadline (`machine.rs:300-302, 344-349`).
  - *A push's `finish()` waiting for exit-status or CHANNEL_CLOSE:* the consumer does not wait on it. After Close and handoff it is bounded by `Closing.deadline = now + cleanup_ms` (`ssh_worker.rs:448-453, 476-488`).
  - *Fetch:* its close runs `finish_early` at Close.

  Nothing needs a network timeout before Close, and everything after Close has its own bound. `Idle` is correct.
- **Backstop.** `open_backstop_ms` applies only while `!entry.opened` (`driver/pump.rs:268-276`) and is gone before the pump's exchange states matter. No interaction.
- **Retry machine.** It retries only setup, before any Git byte (`held.rs:28-39`). The new states arise only after Git bytes have moved. No interaction.
- **Cancellation.** It is independent of the I/O state: `bridge_cancelled` leads to `pump.cancel()` and a discard (`ssh_worker.rs:400-403, 438-441`).
- **Pool reuse.** A connection is now returned after long local work where before it was discarded.
  - It is reusable only if `finished && close_completed && closed_drained && !invalidated` (`ssh_pump.rs:248-256`) and `resource.reclaim()` (`runner.rs:279-283`). That is, the channel's close completed cleanly after the local work.
  - If sshd dropped the session during the work, for example keepalives went unanswered because nothing reads the socket after `SD`, then `finish_early` or `finish` fails, the error path invalidates the channel, and the connection is discarded.
  - A blackholed, NAT-dropped connection is Follow-up 2. It is not a defect of this object.
- **HTTPS parity.** HTTPS reports `Backpressure` while handing response bytes to the consumer and after the body ends (`serve.rs:259-266`). The SSH pump now matches this for a full buffer and after the server is done. It still differs for a partial buffer at the client's turn (P3-1) and for producer waits on a push (Follow-up 1, where HTTPS reports `Backpressure` at `serve.rs:186`).
- **The implementation note** (`:801-818`) is accurate against the code, including the exclusion of a stopped process (SIGSTOP).

## 3. Follow-ups

These are outside the object's scope, recorded rather than required. None blocks this change.

1. **Push producer pause mid-pack is charged as `Network`. ARCHITECTURAL limitation, hypothesis, not tested.**
   - Observation: once receive-pack bytes start, `GitTurns` stops tracking, so `F = 0` and `R = 0` give `Network`.
   - libgit2 compresses each object whole before writing it, so a large blob could pause the writer for longer than the limit.
   - The pump cannot tell "producer paused" from "pack finished, waiting for report-status": libgit2 sends no EndWrite before Close, and the stream carries no "reader blocked" signal. `Backpressure` there would hide a dead server (fail-open).
   - Practical impact: a possible `StreamStall` on SSH pushes that contain very large objects. HTTPS does not have it.
   - Suggested next step: an end-to-end row (push of a single blob of 1 GiB or more over the SSH fixture, default limit). If it reproduces, the fix is a stream-level read-demand signal (an architectural decision for the operator). Already listed in the evidence README §2.5 and "Limits".
2. **The pool idle bound does not bound network idleness. Limitation.**
   - A lease can now stay network-idle for minutes (post-pack `Idle`), and the pool's 60 s idle timer (`pool/mod.rs:54`, `clock.rs:73`) starts at release.
   - For a fetch whose server's CHANNEL_CLOSE was already buffered, `wait_close` can complete without a round trip, so a NAT-blackholed session can be pooled.
   - On reuse, the channel open stalls until the stream deadline. `Held::unserved(false)` then returns `Failed(TimedOut)` rather than `Dead` (`held.rs:28-38`), which costs one stall and a retry attempt (`DEFAULT_MAX_RETRIES = 3`).
   - The same exposure already existed for think-time `Idle`.
   - Suggested next step: measure idleness from the last network byte rather than from release, or treat a stalled first open on a reused connection as `Dead`.
3. **SIGSTOP / paused VM.** The monotonic clock jumps while no thread runs. This is recorded by the run as a separate limitation and is unchanged here.

## 4. Risks and next action

- **Risk.** Low. On the state axis the change is fail-closed:
  - every new `Backpressure` or `Idle` case either has the consumer holding all pending work, or has the peer finished with nothing queued toward it;
  - every case with pending peer I/O remains `Network`.
- **Residual risk.** The `Network` guards are unprotected against regression (P3-3), and the design wording is loose (P3-2). If someone "corrects" the code to match that wording, it could fail open.
- **Next action.**
  - GO for this checkpoint.
  - Fold P3-2 (two sentences) and P3-3 (three rows) into the next touch of the pump.
  - Decide P3-1: fix it with the one-condition change and its row, or record it in §10.1.
  - Route Follow-up 1 to the operator as an architectural decision with the large-blob push row as its trigger test.
  - I pre-commit to GO on a revision that closes P3-1 to P3-3 with the closure tests named above.
- **Process note.** This review wrote gitignored build output into the lane's `gwz-core/target/` (see §0). No source changed, and the tuple is intact.
