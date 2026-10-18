# GWZ transport SSH background close design — STATE-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportSshBackgroundCloseDesign.md`, sha256 `2630194f1171290d9e7e9821ee2f756fb5c485715a4abb391c7fdbb8ecac7d6d`, 161 lines, DRAFT. The file is untracked on purpose. Its hash was checked at the start and at the end of the review and did not change.
**Baseline:** gwz-core `f48cf5a6`, gwz-transport `ff6083b5`, libgit2 fork `b172e3d` and libssh2 from `libssh2-sys 0.3.3`, the version in gwz-core's `Cargo.lock`. All gwz sources were read with `git show <sha>:<path>`; no working-tree source was used. libgit2 was read with `git show HEAD:` in `/Volumes/projects/limbo/gwz-dev/libgit2`. libssh2 was read from the cargo registry. The evidence run `2026-10-07-tr8-1-ssh-gap-32` (`README.md` and both patches) was read at gwz-core-evidence `4674d47`.
**Date:** 2026-10-07
**Axis:** STATE. The review attacks transition legality, races, event ordering, fail-closed direction and stuck states. Independent, adversarial, read-only. Filed verbatim by the lane owner.
**Verdict: NO-GO** — 0 P0, 0 P1, 3 P2, 3 P3. No finding is ARCHITECTURAL: option A can be realised, and every defect has a bounded text fix. I pre-commit to GO on a revision that resolves P2-1, P2-2, P2-3, P3-1, P3-2 and P3-3 as specified.

---

## 0. Evidence base

- **Member path.** `RemoteTransport::close` blocks in `BlockingStream::close` (`ssh_remote.rs:84-96`; `stream_io.rs:38-40`). libgit2 calls it only from `git_smart__close`, through `git_smart__reset_stream`.
  - `git_remote_fetch` calls `git_remote_disconnect` after `git_remote__download` and before it updates tips (`remote.c:1391-1398`). `git_remote_push` calls it under `done:`, after `git_remote_upload` and `update_tips` (`remote.c:3064-3072`).
  - `git_remote_disconnect` ignores the close's result.
  - `git_smart__close` writes a goodbye flush `0000` on both fetch and push before it closes (`smart.c:395-398`).
- **Fetch.** `git_smart__download_pack` reads to the flush at `smart_protocol.c:753` and commits at `:782`. Without side-band, `no_sideband` reads to the stream's EOF before it commits (`:630-645`). The fork's transports differ from v1.9.7 only in `local.c`. It has no protocol v2 (no `version=2` or `GIT_PROTOCOL` anywhere in `transports/`).
- **Push.** Without report-status, libgit2 sets `unpack_ok` and returns (`:1248`). Otherwise `parse_report` runs (`:1250`).
- **Pump.**
  - `finish_request` requires both EOFs plus the client's EOF, and turns a nonzero status into `Invariant` (`ssh_pump.rs:364-377`).
  - `complete_close` is called only once `finished` is set (`:378-392`).
  - The terminal short-circuit retires the channel when the close was not completed (`:448-455`).
  - The repository refusal goes out through `fail_terminal` with no Closed (`:301-314`).
- **Channel.** `SshChannel::finish` waits for EOF in `Active` (`ssh_channel.rs:131-158`). `poll_dispose` returns `WouldBlock` and keeps ownership while `close()` blocks (`:179-194`).
- **libssh2.**
  - `_libssh2_channel_close` waits only through `while(!channel->remote.close && !rc …) rc = _libssh2_transport_read(session)` (`channel.c:2706`). It returns EAGAIN until some packet arrives, and returns 0 after any single packet.
  - `channel_wait_closed` refuses unless `remote.eof` is set (`channel.c:2756-2760`).
  - A CHANNEL_CLOSE sets both `remote.close` and `remote.eof` (`packet.c:1234-1235`).
- **Worker.**
  - `transfer` gives `Ok(true)` once Closed or Failed is handed over (`ssh_worker.rs:333-352`).
  - The runner releases on that result (`runner.rs:181-206`).
  - On stop the runner calls `active.clear()` and `pool.shutdown()` (`runner.rs:30-37`), and keeps looping until `shutdown_complete` or `stopping_at` (`:89-93`).
  - `NativeResource::poll_dispose` returns Pending while the pump's disposal would block (`ssh_setup.rs:243-263`). It forces only on the pool's Abort (`ssh_pool.rs:333-340`).
  - `Session::cleanup` waits until `pending_local_work == 0` or the bound (`session/close.rs:47-70`), and `pending()` includes `pending_connections` (`placement_endpoint.rs:209-217`).
- **Pool (gwz-transport).**
  - `schedule` leases only Idle entries, and creates a new connection under the caps otherwise (`pool/allocation.rs:17-58`).
  - `release` (`lifecycle.rs:156-184`). A Lease dropped without release counts as `Discarded` (`asynchronous.rs:271-278`).
  - The crate's contract: "The endpoint host calls `complete_close` only after backend cleanup has proved whether a connection is reusable" (`README.md:61-62`; `machine.rs:175-176`).
- **Tests and fixtures.**
  - `FakeResource::pump()` returns `None` (`shared_reservation.rs:328-330`). `ChannelResource::pump` is concrete `SshPump<SshChannel>` (`ssh_worker.rs:50`), and `attach` fails without a pump (`ssh_worker.rs:258-260`).
  - In `delayed_close_fixture`, `CLOSE_DELAY` is 1 s (`close_tests.rs:21-43`).
- **Adaptive design** at f48cf5a6: §4.1 (state table), §4.3 to §4.5 (`S_lo`, quiet, test bases) and §8.

## 1. Findings

### [P2-1] A Failed terminal never ends the background close: no ending event, no bound, the lease is held until the worker stops

- **Location.** Doc §4 "Changes in the pump", last two bullets (lines 65-66); "Changes in the worker" (lines 70-71); the "Close ends" table (lines 76-81); D3 and D4. Code: `ssh_pump.rs:448-455` and `:301-314`; `ssh_worker.rs:333-352`; `runner.rs:190-195`.
- **Violated invariant.** Every state the rules allow has a bounded exit, and a member's terminal never strands its lease.
- **Interleaving.**
  1. A member's repository is refused. `write_reverse` calls `fail_terminal`, and the stream turns terminal with a **Failed** message and no Closed.
  2. On the next tick, `tick_inner` sees `terminal && !close_completed`, runs `retire_channel` (force-dispose) and returns `Ok(())`. The pump's Close path never runs: `finished` stays false and no error is raised.
  3. `transfer` hands Failed over. Under the doc's rules this is "member done (terminal handed over, close still open)", because none of the four "Close ends" rows happens: there is no CHANNEL_CLOSE, no status, no error and no timeout.
  4. The only bound, `PumpError::Cleanup`, is armed from `closed_at`, which the doc sets "when it queues Closed". A Failed terminal never queues Closed, so the bound is never armed.
  5. The runner "releases only when over", so the exchange stays in `active` and the lease stays Leased indefinitely.

  A stream Timeout reaches the same state (`machine.rs:343-344`, `fail(Timeout, true)`). Example: a receive-pack whose server sends no EOF within `cleanup_ms`.
- **Impact.**
  - The connection keeps its per-host and total slot and its `ReservedResource` permit until the worker stops, and the worker polls at 1 ms the whole time.
  - With `--max-per-host 1`, or with at least as many refused repositories as the cap, every later member of that host waits for allocation and fails with a false `Timeout`.
  - Today the same Failed terminal releases at once (`Ok(true)`, `reclaim` false, `Discarded`).
- **Required correction.**
  - Define "over" as: the terminal has been handed over **and** (`finished`, **or** the pump is invalidated or retired, **or** the terminal was Failed).
  - Arm the `cleanup_ms` bound at the terminal's handoff, not only when Closed is queued.
  - State that a Failed terminal releases `Discarded` in the pass that hands it over, as today.
- **Closure test.**
  - At pump level: `FakeChannel` with an empty stdout, stderr `"repository not found."` and both EOFs. After Failed is drained, the pump reports over or invalidated within one tick.
  - At sshd level: the doc's `--max-per-host 1` fixture with a refused path first and then a valid open. The second open completes with `physical_count` 0 then 1, and has no allocation wait.

### [P2-2] Shutdown and discard still wait for the server's close: D5 and §1's "nothing waits for it" are false

- **Location.**
  - §1: "Nothing waits for it after that: not the member, not the command, not shutdown."
  - D5.
  - §5 bullet 3: "a channel close that may not block, then a forced socket termination … No wait for the server".
  - §5 bullet 4: "disposes locally and at once".
  - §4: "A discard-after-use exchange skips the graceful close".
  - Code: `ssh_channel.rs:179-194`; `ssh_setup.rs:243-263`; `ssh_pool.rs:333-340`; libssh2 `channel.c:2706`; `runner.rs:30-37` and `:89-93`; `session/close.rs:57-70`.
- **Violated invariant.** D5: shutdown, pool cancellation and process exit never wait for a close in flight.
- **Interleaving.**
  1. A receive-pack's pump has read both EOFs and calls `finish()`. libssh2 sends CLOSE and enters the `sent` state. The next server packet (exit-status, then CLOSE) is `CLOSE_DELAY` away, or up to about 52 ms (p90) on GitHub.
  2. The command ends: `request.finish()`, then `runtime.shutdown()`. The worker's `active.clear()` drops the Lease, which counts as `Discarded`.
  3. The pool's Close action leads to `poll_dispose(force=false)`, then `pump.poll_dispose`, then `SshChannel::poll_dispose`. That calls `channel.close()` again; libssh2 is still in `sent` with `!remote.close`, reads EAGAIN, and the call returns `WouldBlock`.
  4. The resource is kept and returns Pending until the server's next packet arrives, or until the pool's Abort at `cleanup_timeout_ms` (5 s).
  5. Meanwhile `Session::cleanup` waits on `pending_connections > 0`.

  So the command's exit pays the server's tail for every connection still closing. That is the tail option A removes from the member's path. A server that never closes costs the full 5 s at exit and can leave `pending_local_work > 0` in the cleanup report; today the connections are idle at command end and dispose at once. The same `WouldBlock` path holds a discard-after-use slot until the server closes.

  For an upload-pack early close, libssh2 usually leaves `sent` on the first packet after our CLOSE (`rc > 0`, so `local.close = 1`), so most fetches escape. The outcome depends on timing.
- **Impact.**
  - D5 and §1 are false.
  - §8's expected wall-time effect does not follow for the last members to finish, nor for push.
  - Test row 12 fails as soon as its exchange is a receive-pack, or any exchange whose both EOFs were read before `finish`.
- **Required correction.** Specify the disposal change. When the channel is in `Phase::Close` or `Phase::WaitClose` (a graceful close already under way), `SshChannel::poll_dispose` goes straight to `force_dispose` (socket terminate) without calling libssh2's channel close again. Equivalently, the worker's stop path force-disposes exchanges that are closing in the background before it drops their leases. State that the discard-after-use and stop paths use it.
- **Closure test.**
  - Row 12 run with `ReceivePack` on `delayed_close_fixture`: `runtime.shutdown()` returns in well under `CLOSE_DELAY`, and `pending_local_work == 0`.
  - Row 10 with a script that never exits: `cleanup_complete` well under `cleanup_ms` (for example 500 ms), not merely within it.

### [P2-3] A same-host exchange that starts during a background close opens a new connection instead of reusing: §6's "reuse timing is unchanged" is false

- **Location.** §6 "Reuse timing is unchanged" (line 102); §2 lines 32-34; D6. Code: `gwz-transport:src/pool/allocation.rs:17-58`; gwz-core `push_plan.rs:33,80` and `publication.rs:61,251,324` (ls-remote) followed by `push_member.rs:331` (push).
- **Violated invariant.** A healthy connection returns to service, reusable, for the next exchange of its key and identity. A defect may cost time, but option A must not trade a tail of tens of milliseconds for a fresh SSH setup.
- **Interleaving.**
  1. A member's ls-remote (preflight or push plan) closes. Under option A, `close()` returns at once and the connection stays Leased for the server's tail (4 to 75 ms).
  2. The member's push opens at once. `schedule` finds no Idle entry for the key and identity, and the host is below its cap, so it creates a new connection.
  3. Today the push always arrived after the close, so it leased the idle connection and reported `reused`.

  The effect is a full connect and authentication (GitHub: 2 to 3.5 s connect against 0.5 s on a reused channel, per the reuse evidence). It also adds an extra connection that the server counts and that later idles for 60 s. The 32-member fetch at the cap is not affected, since the cap makes later opens wait. Single-member and below-cap pushes, tags and publications are affected.
- **Impact.** A parity and time regression that §6 denies. It also raises `Possible` and `Connected` under the adaptive machine.
- **Required correction.** Choose one and state it:
  - (a) Let a checkout for a key and identity that has a lease in background close wait for that close, bounded by the close's remaining `cleanup_ms`, before it creates a new connection. This needs a pool-visible "closing, will be idle" state or a worker-side deferral, which makes D6 conditional.
  - (b) State the regression, measure it (single-member ls-remote then push, on the Pi and the Mac), and put it to the operator.
- **Closure test.** On `delayed_close_fixture` with `--max-per-host 32`: an ls-remote, then at once a second exchange on the same URL and identity. Under (a), `physical_count` stays 1 and the second exchange reports `reused`. Under (b), the test records the regression.

### [P3-1] "No number differs" (§6) is wrong: Connected and Closing differ in §4.1's quantities, so the adaptive revision is semantic, not wording

- **Location.** §6, second bullet (line 100).
- **Violated invariant.** The adaptive design's accounting (`Connected`, `S_lo`, quiet, test bases) matches the connection's real state.
- **The defect.** The doc's conclusion is right: the session is intact, so the connection is Connected. Its justification is not.
  - `Connected` feeds §4.2's success and overload rules, §4.3's `S_lo` ("removed when it … begins closing"), §4.5's quiet rule ("nothing Closing"), and the probe and confirming bases (`Connected = N`, `k = Connected + 1`).
  - If the adaptive implementation follows its own §8 ("stays Closing, then Settling"), every member completion makes its key non-quiet and makes in-flight tests unfair.
  - It also needs a Closing → Idle transition that §4.1's state table does not have.
- **Correction.** Replace "no number differs" with: "Connected, which §4.2 to §4.5 read; §8's Closing then Settling would be an illegal Closing → Idle transition and would break quiet and fairness". Send it to amendment 2 as a semantic change.
- **Closure.** The text change, plus a row in the adaptive §10.2 cases: a test window that overlaps a background close stays fair.

### [P3-2] D6 and §7: gwz-transport's published `complete_close` contract becomes false, and the doc calls the fix optional

- **Location.** §4 "Closed's disposition" (line 87); §7, third bullet (line 111); D6. Code: `gwz-transport` `README.md:61-62` and `src/stream/machine.rs:175-176`; gwz-core `close_tests.rs:133-140`.
- **The defect.** The crate states that the endpoint calls `complete_close` "only after backend cleanup has proved whether a connection is reusable". Option A calls it with `Disposition::Reusable` before that proof, and a later failure discards the connection. The design therefore relies on a contract it breaks, while calling the correction "optional, doc only". TR2.9's assertion "every close is clean" (`Disposition::Reusable`) silently changes meaning.
- **Correction.** Make the README and `machine.rs` contract change a required part of the design. D6 then reads "no gwz-transport code change". Re-word TR2.9's assertion: the Closed disposition reports the exchange; the pool counts carry reuse.
- **Closure.** The text changes, and TR2.9 asserts reuse through `ssh_counts_for_test`, as it already does at lines 149-157.

### [P3-3] The test plan cannot pin three rules as written

- **Location.** §9, rows 2, 6 and 8, and the fixture paragraph (line 137); §4, line 63.
- **Row 6** says the worker releases `Discarded` using `FakeResource`. `FakeResource::pump()` returns `None` (`shared_reservation.rs:328-330`), so `attach` fails at `ssh_worker.rs:258-260`. `ChannelResource::pump` is typed `SshPump<SshChannel>`, so no `FakeChannel` can reach the worker. As written, the worker-level release on a late failure has no unit fixture. The pump-level `into_owner()` check is the real pin.
- **Row 8** does not set `output_eof = false` and `stderr_eof = false`, the partial-fix patch's `server_still_running()`. Without that, the pump may read both EOFs before the Close, and the row then tests the after-EOF path rather than the early one.
- **No real-server row shows that an early-closed upload-pack connection is reused.** Row 2 checks only Closed's `Disposition::Reusable`, which §4 itself says is not the reuse authority. The doc names both patches as "this shape", but they differ where libssh2 bites. `proto-early-close.patch` calls `wait_close` before draining. libssh2's close can return 0 after one data packet (`channel.c:2706`), and `wait_closed` then fails with INVAL because `remote.eof` is not set (`channel.c:2756-2760`). The result is a discard on every such fetch, and only a real-server reuse check would catch it.
- **Correction.**
  - Row 6: drive the worker through the sshd fixture with a `finish` failure injected, or drop the `FakeResource` clause.
  - Row 8: set the server-still-running flags.
  - Row 2: add "after the delay, `idle == 1`, and the next open reports `reused`", using `delayed_eof_fixture`.
  - §4: name the partial-fix order (drain until `channel.eof()`, then `wait_close`) and state the two libssh2 preconditions.
- **Closure.** The revised rows, failing on f48cf5a6.

## 2. Invariant analysis

- **Result finality (Item 1) holds. No finding.**
  - **(a) Fetch.** libgit2 sends the member's Close only from `git_smart__close`, after `download_pack` has committed (`:782`) or after it failed. Without side-band it commits only after stream EOF (`:630-645`). `git_remote_fetch` disconnects before `update_tips`. ls-remote (`advertised_refs`) collects its refs before the drop disconnects. Clone fetches through `git_remote_fetch`. A shallow fetch negotiates on the same stateful stream. There is no protocol v2 in the fork. An early close therefore cuts only the server's tail.
  - **(b) Push.** The result is fixed inside `git_smart__push`: `parse_report` (`:1250`), or success assumed when report-status is off (`:1248`). That happens before `git_remote_disconnect`. Waiting for EOF rather than exit status cannot change it. Hook output after the report is drained to EOF, as today.
  - **(c) Exit status.** It never reaches a result: libgit2 discards the disconnect's return (`remote.c:1394` and `:3070`), and gwz turns a nonzero status only into a discard (`ssh_pump.rs:369-371`).
  - **Side note.** Today, a failing close can overwrite the libgit2 error text of an already failed fetch or push, through git2-rs's last error. Option A makes this rarer and adds no case.
- **Reuse safety holds, given wait_close.** The reuse gate is `remote.close`, through `wait_close`. libssh2's `channel_free` purges the channel's queued packets, and channel ids only increase (`_libssh2_channel_nextid`). Nothing about the channel (window, pending data, exit status) reaches the next channel. A late failure invalidates the pump, and `into_owner` refuses it (`ssh_pump.rs:223`). Every failure therefore fails closed, provided the implementation keeps `wait_close` as the gate (P3-3).
- **Lease and connection overlap: none.** The lease is held until release, and `PoolHost::release` refuses `Reusable` unless the resource is Idle. A background close and a new lease of the same connection cannot overlap. The worker serves other exchanges in the same loop.
- **Cancellation is correct as designed.** A cancel after the terminal must be ignored (§4), and a cancel before it is unchanged. Placement `cancel_request` and `shutdown` cancel attachments of terminal requests, so ignoring late cancels is required, and it is safe.
- **Bounds.**
  - Gap: the Failed-terminal case has no bound (P2-1).
  - Correction to §4's "Clocks": for receive-pack the slot can be held for the stream's `cleanup_ms` from Close to Closed, plus a further `cleanup_ms` from Closed. That is up to about 10 s, not "no more time than today". It is a doc fix with no state consequence.
- **Citation errors.** "flush at line 77, commit at 106" points into `git_smart__store_refs` and `append_symref`. The lines are 753 and 782 in both 1.9.7 and the fork.

## 3. Risks and next action

- **Highest risk.** P2-2 can cancel most of option A's measured gain. The tail moves from the member's path to the command's exit unless disposal forces when a close is already under way.
- **Next most likely in the field.** P2-1 on refused repositories, and P2-3 on push, tag and publication commands below the per-host cap.
- **Next action.** The drafter revises §1, §4, §5, §6, §7 and §9 as specified under P2-1, P2-2, P2-3, P3-1, P3-2 and P3-3.
- **P2-3 (a) or (b) is the operator's decision.** Option (a) makes D6 conditional on a pool or worker deferral.
- **Re-review.** This axis re-reviews only the revised sections. GO is pre-committed on resolution as specified.
