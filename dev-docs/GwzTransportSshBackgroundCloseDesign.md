# GWZ transport SSH background close — design

Date: 2026-10-07; revision 1, 2026-10-07; revision 2, 2026-10-07. Status: **DRAFT, 2026-10-07. Revision 1 reviewed GO on the State axis (`-ReviewState-2.md`); revision 2 folds in that review's three P3s as text edits, which the reviewer said need no re-review.** The operator decided the seven open questions of §11 as recommended (2026-10-07). It authorizes no implementation, commit, tag, push or publish.

- **Review record.** Revision 0 went to the State review (`GwzTransportSshBackgroundCloseDesign-ReviewState.md`): NO-GO, 3 P2 and 3 P3, none architectural. Revision 1 resolves P2-1, P2-2, P2-3, P3-1, P3-2 and P3-3 as the review specifies, and its citation corrections. The operator decided P2-3 as option (a), "Wait briefly" (2026-10-07). §6 is new. The other sections that changed are §1, §3, §4, §5, §7, §8, §9, §10 and §11.
- **The operator's direction (2026-10-07):** option A for the SSH gap at 32 members. Member completion is decoupled from the physical close. The member completes as soon as its result is known. The graceful close continues off the member's path. The connection returns to the pool as reusable only after the close completes cleanly, and is discarded if the close fails or times out.
- **Authority.** The evidence run `gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-07-tr8-1-ssh-gap-32` (`README.md`, `report/tables.txt`, `report/per-connection-compare.txt`, the two prototype patches). [Adaptive concurrency design](GwzTransportAdaptiveConcurrencyDesign.md) §4.1 to §4.5 and §8. [Connection reuse design](../../dev-docs/GwzConnectionReuseDesign.md) §7 and §8. TR2.9 (`src/transport_host/close_tests.rs`).
- **Code lines.** gwz-core `f48cf5a6`, gwz-transport `ff6083b5`, read at those commits and not from the working tree. Paths are gwz-core's `src/git/endpoint/` unless they start with `gwz-transport:`. libgit2 lines are from the 1.9.x source (the fork `b172e3d` differs only in `local.c`). libssh2 lines are from `libssh2-sys 0.3.3`.
- This is a design. It has no phases and no steps.

## 1. Outcome

A member no longer waits for the server's exit status and channel close. Its `close()` returns once its result is known (§3). The connection stays leased, and counted, until the close finishes. It then goes back to the pool as reusable, or it is discarded. Nothing waits for it after that: not the member, not the command, not shutdown (§5 changes the disposal that makes this true). An open for the same key and identity that arrives during the close waits briefly for it and reuses the connection (§6).

Decisions:

| # | Decision | Section |
|---|---|---|
| D1 | The result is final when the member sends its Close (upload-pack) or when the server's output has ended (receive-pack). The exit status is never part of the result. | §3 |
| D2 | The pump sends the member its Closed message without waiting for the physical close, then keeps ticking until the exchange is over. | §4 |
| D3 | The worker keeps the lease and the pump until the exchange is over. "Over" is defined in §4. No new thread, no new pool state. | §4 |
| D4 | One bound for the background close: the exchange's `cleanup_ms`, armed at the terminal's handoff. A close past it is a discard. | §4 |
| D5 | Shutdown, pool cancellation, discard-after-use and process exit discard a closing exchange by terminating the socket at once. This needs one change in `SshChannel::poll_dispose`. | §5 |
| D6 | An open for the same key and identity waits for a closing exchange, at most `min(250 ms, the close's remaining cleanup_ms)`, before it opens a connection. The deferral is the worker's. | §6 |
| D7 | No gwz-transport code change. Its published `complete_close` contract text changes. | §8 |

## 2. What happens today

The member is libgit2 behind `RemoteTransport` (`ssh_remote.rs:84-96`). Its `close()` waits for the endpoint's Closed message (`stream_io.rs:38-40`). The chain that Closed waits for:

- The pump sends the reverse EndWrite only after both stdout and stderr have reached EOF (`ssh_pump.rs:286-323`).
- The stream machine refuses Closed until it has sent that EndWrite (`gwz-transport:src/stream/machine.rs:238-246`).
- The pump calls `complete_close` only when `finished` is set (`ssh_pump.rs:378-392`). `finished` is set by `SshChannel::finish`, which sends the channel close and then blocks in `wait_close` for the server's CHANNEL_CLOSE, and reads the exit status after it (`ssh_channel.rs:131-158`; `ssh_pump.rs:364-377`).
- `into_owner`, and so `reclaim` and the release as `Reusable`, need `finished && close_completed && closed_drained` (`ssh_pump.rs:222-230`; `ssh_setup.rs:332-363`; `ssh_worker/runner.rs:192`).

So the member's close, the lease release and the reuse all happen at the same moment: after the server's whole exit sequence. That sequence is what the traces show as the trailing receive span: p90 4 to 52 ms, with 7 to 12% of connections over 10 ms, against 0 for gwz 1.0.17 (`report/tables.txt`). Under 32-way load some connections pay 40 to 75 ms, and the command waits for its slowest member.

The stream machine gives the endpoint `cleanup_ms` (5 s) from the member's Close to Closed, then fails the stream with Timeout (`gwz-transport:src/stream/incoming.rs:99-113`, `machine.rs:335-346`). The stall clock is stopped by Close, so `--ssh-timeout` plays no part in the close.

## 3. When a member's result is final

The member is libgit2, so the member's own Close is its declaration that it has what it needs. The Close discards any response bytes it has not read (`gwz-transport:src/stream/machine.rs:154-173`).

| | upload-pack (fetch, ls-remote) | receive-pack (push) |
|---|---|---|
| What libgit2 does before it closes | Reads to the pack's closing flush-pkt (`smart_protocol.c:753`), then commits the pack through its indexer (`:782`). Without side-band it reads to the stream's EOF first (`:630-645`). `git_remote_fetch` disconnects after that and before it updates tips. | Writes the pack, then `parse_report` reads the report-status to its flush (`:959`, `:1250`). If the server did not offer report-status it does not read at all (`:1247-1249`), and closes at once. |
| Result is final when | The member's Close has been received and the client's EOF has gone to the server. The pump does not wait for the server's EOF. | The server's stdout and stderr have both reached EOF and the member has its EndWrite, as today. |
| Why | A fetch only reads. Ending it early is what Git does when a client goes away. Waiting for EOF only adds the server's exit tail. | A push without report-status, or with hooks still writing to stderr after the report, is complete only when the server process has finished its output. Closing sooner could cut a server that is still updating refs. |
| What it changes | The pump ends the reverse stream itself, drops what the server has not yet sent, and sends Closed. | Nothing before EOF. Only the exit status and CHANNEL_CLOSE leave the member's path. |

**The exit status.** It cannot change a member's result, in libgit2 or in gwz today:

- libgit2 reads no exit status. `git_remote_fetch` calls `git_remote_disconnect` and ignores its result before it updates tips. `git_remote_push` does the same under `done:`. A failed close therefore changes neither result.
- gwz compares the status with 0 only to decide reuse. `finish_request` turns a nonzero status into `PumpError::Invariant` (`ssh_pump.rs:369-371`). The tick fails, the pump disconnects the stream and the worker releases the lease as `Discarded` (`ssh_pump.rs:444-447`; `ssh_worker.rs:354-357`; `runner.rs:193`). The member's `close()` returns an error, which libgit2 ignores, and the fetch or push result stands. The existing test `nonzero_service_status_poisoned_channel_cannot_be_reused` (`ssh_tests/pump.rs`) pins the pump half of this. I did not read upstream Git's `finish_connect` handling.

So push does not need to wait for the exit status. The status becomes a reuse input only (§4).

## 4. Ownership, the end of the exchange, and the clocks

**Who owns the connection.** The worker thread does, through two things it already holds: the `Active` entry, which has the lease (`ssh_worker.rs:177-190`), and the `NativeResource` in `State::Active`, which has the pump and the channel (`ssh_setup.rs:158-166`). The pool sees the connection as `Leased` with no request. It is counted in the per-host and total caps and in the `ReservedResource` reservation (`shared_reservation.rs:128-146`) until its disposal. The pool's `Closing` state is entered only on a discard.

**Changes in the pump** (`ssh_pump.rs`, `ssh_channel.rs`):

- `finish_channel` stops requiring `finished`. It calls `complete_close` as soon as the member's Close is in and the reverse end is sent (§3's rule per service).
- For upload-pack, the member's Close and the client's EOF are the trigger. The pump clears `reverse`, ends the reverse stream, and stops reading the channel for the member. It sends the channel close, then reads and drops what the server still sends until `channel.eof()` is true, then calls `wait_close`. That is the order in `report/early-close-partial-fix.patch` (`discard_unread`, then `wait_close`). It is not the order of `report/proto-early-close.patch`, which calls `wait_close` first. Two libssh2 facts decide it: the channel close returns 0 after any single packet and is not proof that the server closed (`channel.c:2706`), and `wait_closed` fails with INVAL unless the remote EOF is set (`channel.c:2756-2760`; a CHANNEL_CLOSE sets both flags, `packet.c:1234-1235`). The reuse gate is a successful `wait_close`, that is, the server's CHANNEL_CLOSE.
- For receive-pack, the existing gate stays (both EOFs, reverse end sent). `finish` runs as today, but Closed no longer waits for it.
- `tick_inner` returns at once when the stream is terminal (`ssh_pump.rs:449-455`): it retires the channel unless the close was completed. After this change a completed Closed keeps the tick calling `finish_request` until `finished`. A terminal that was not a completed Closed (Failed, Timeout) still retires the channel, which sets `invalidated`.
- The pump gains two reads for the worker: `finished()` and `retired()` (`invalidated`).

**Changes in the worker** (`ssh_worker.rs`, `runner.rs`):

- `Active` gains the pool key and pool identity (for §6), the exchange's `cleanup_ms` (`context.deadlines.cleanup_ms`), `terminal_failed`, and `handoff_deadline`.
- `transfer` hands the terminal over as today. At the pass that does, the worker records whether it was Failed and arms `handoff_deadline = now + cleanup_ms`. Armed at the handoff, the bound covers a Failed or Timeout terminal as well as a Closed.
- After the handoff `transfer` stops reading the bridge. Today a dropped attachment (`Disconnected`, line 316) or a set cancel flag (line 309) is an error. After the terminal the member drops the attachment as a matter of course, so neither may discard a clean close.
- **Over.** An exchange is over when the terminal has been handed over **and** (`finished`, **or** the pump is `retired`, **or** the terminal was Failed). `runner.rs:190-195` releases when the exchange is over, or when `now >= handoff_deadline`.
- The 1 ms poll keeps running while a closing exchange is in `active` (`runner.rs:209-219`). That is the existing cost, for the length of the close.

**What happens when it ends.**

| It ends how | Release |
|---|---|
| A Failed terminal (refusal, Timeout) is handed over | `Discarded`, in the pass that hands it over, as today (`runner.rs:193`) |
| Closed handed over, `finished`, status 0 or the close was early, not marked discard-after-use | `Reusable`, after `reclaim` |
| Closed handed over, `finished`, status nonzero after the server's EOF was seen | `Discarded` (today's rule, `Invariant`) |
| Closed handed over, error from the channel or the session, or the pump `retired` | `Discarded` |
| `now >= handoff_deadline` | `Discarded`; the pool's Close then terminates the socket (§5) |
| Marked discard-after-use | `Discarded` when the terminal is handed over; the graceful close is skipped |

After an early close the server may have been stopped by SIGPIPE, so its status says nothing about the connection. A discard-after-use exchange and the stop path both use the disposal change in §5.

**Clocks.** One bound, `cleanup_ms`: 5 s (`transport_host/session/driver.rs:29-36`), never 0 (`ssh_worker/endpoint.rs:109-113`). It runs from the terminal's handoff. For upload-pack that is at the member's Close, so the physical close has the time it has today. For receive-pack the stream's own `cleanup_ms` runs from the member's Close to Closed, which waits for EOF, and the background bound runs from Closed. A connection's slot can then be held up to about 10 s from the member's Close, against 5 s today. The member waits only for the first part. The stall clock (`--ssh-timeout`) is stopped by Close and never runs here. With `--ssh-timeout 0` the close is still bounded by `cleanup_ms`.

**Closed's disposition.** Closed still carries `Disposition::Reusable` for a clean exchange. It reports the exchange, not the connection. Nothing in gwz-core outside tests reads it. The lease release is the only authority on reuse (§8).

## 5. Cancellation, shutdown and process exit

- **Before the member is done.** Unchanged. A cancel flag or a dropped attachment ends the exchange with `pump.cancel()`, a forced disposal and `Discarded` (`ssh_worker.rs:354-357`).
- **After the terminal is handed over.** The member can no longer cancel. A late `EndpointAttachment::cancel` is ignored (§4). The close finishes or reaches its bound.
- **The disposal change.** Today `SshChannel::poll_dispose` calls `abort()` and then `channel.close()` again. In libssh2 a second close, while the first is waiting for the server's CHANNEL_CLOSE, reads until some packet arrives and returns EAGAIN (`channel.c:2706`). So `poll_dispose` returns `WouldBlock`, and `NativeResource::poll_dispose` stays Pending until the server's next packet or the pool's Abort at `cleanup_timeout_ms` (`ssh_channel.rs:179-194`; `ssh_setup.rs:243-263`). A discard of a closing exchange would then wait for the server, which is the tail option A removes. The change: `poll_dispose` reads its phase before it calls `abort()` (which overwrites it). If the phase is `Close` or `WaitClose`, a graceful close is already under way, and it goes straight to `force_dispose`, which terminates the socket, with no second close. Exchanges not yet closing (`Open`, `Exec`, `Active`) keep today's path.
- **Worker stop or pool shutdown.** `runner.rs:30-37` clears `active` and calls `pool.shutdown()`. The pool starts closing every leased entry (`gwz-transport:src/pool/lifecycle.rs:222-239`). The host then calls `NativeResource::poll_dispose`, which for `State::Active` runs `pump.poll_dispose`, and with the change above a closing channel is terminated at once. An unfinished close is a discard.
- **Discard-after-use and a missed bound.** Both release `Discarded`. The pool's Close then takes the same path.
- **The bound on exit.** The worker's loop ends within `cleanup` of the stop. What is left goes to the retained supervisor (`ssh_shutdown.rs:52-111`). A closing connection disposes locally and at once, so it adds nothing to that bound.
- **The operation's report.** `pending()` counts `pending_connections`, which is every physical entry, idle ones included (`placement_endpoint.rs:209-217`; `ssh_shutdown.rs:42-51`). `Session::cleanup` waits until it is 0 or the bound (`session/close.rs:47-70`). With the change above a closing connection is gone within a pass, so `pending_local_work == 0` holds at command end, as TR2.9 asserts.

## 6. An open for the same key and identity during a close

Without this, an ls-remote followed at once by a push on the same URL would not find the connection Idle (the pool leases only Idle entries, `gwz-transport:src/pool/allocation.rs:17-58`). Below the cap it would open a new one, and pay a full connect and authentication where today it reuses (GitHub: 2 to 3.5 s against 0.5 s). The operator chose to wait briefly.

**Rule.** When the worker is about to call `pool.checkout_until` for a request, it looks for an exchange in `active` that is closing (terminal handed over, not over), has the same pool key and the same pool identity (the one computed at `runner.rs:131-146`, which includes the helpers-disabled partition), and has not been claimed. If it finds one, it claims it and defers the request. The request then proceeds to the pool when the first of these happens:

- the claimed exchange is released (`Reusable` or `Discarded`);
- `min(250 ms, handoff_deadline - now)` has passed, taken at the time of the claim.

If the close ended `Reusable`, the connection is Idle and the checkout leases it (`reused`). Otherwise the checkout is the ordinary one, and may create a connection. One closing exchange is claimed by one request. A second request opens as it does today. **A request is deferred at most once:** when its deferral ends it goes to `checkout_until` without looking for another closing exchange. **An open the adaptive machine admitted as a test carrier is never deferred:** a test must be a new connection (adaptive design §4.5, §4.9's create-only request). A deferred request is expiry-checked like a pending one (`runner.rs:38-45`) and completes `stopped` on stop.

**Where it lives.** In the worker, as a `deferred` list beside `pending`. The alternative is a pool-visible "closing, will be idle" state in gwz-transport. That adds a lease state, a release path and a scheduling rule to a crate that knows nothing of SSH channels, to serve one gwz-core rule. The worker already owns every closing exchange and its `handoff_deadline`. So D7 holds: no gwz-transport code change.

**The adaptive design.** The deferral is below admission. The adaptive machine has already admitted the open, counting the closing connection as Connected and in `Possible` (§7). The deferral adds no connection and no wait that the machine can see, and it never touches a test (rule above), so an admitted test always produces a setup result. If the open then reuses the closing connection, `Possible` does not rise, which is the conservative direction. If it proceeds to a new connection, that start was admitted as one.

**What it costs.** At the cap the pool would hold the request anyway, and it would lease the first connection to go Idle. A deferral can miss another connection that goes Idle during its wait, for at most 250 ms, and only when a close is slow. The 32-member fetch at the cap is not affected: its opens all come before any close.

## 7. The adaptive count, and reuse

- **The count stays honest.** The connection is `Leased`, so it is in `Possible` and in the caps the whole time (adaptive §4.1). A discard then moves it to Closing and Settling as for any discard.
- **A semantic change for the adaptive design's next revision.** The connection is **Connected** for the whole background close, and Closing only if it is discarded. Its §8 says the connection "stays Closing, then Settling". That is not equivalent. `Connected` feeds §4.2's success and overload rules, §4.3's `S_lo` (a connection leaves it when it begins closing), §4.5's quiet rule (nothing Closing) and the test bases (`Connected = N`, `k = Connected + 1`). If an implementation followed §8's wording, every member completion would make its key non-quiet and make a test in flight unfair, and it would need a Closing → Idle transition that §4.1's table does not have. Amendment 2's next revision records §8 as a semantic change, with a §10.2 case: a test window that overlaps a background close stays fair.
- **No new signal for the machine.** A failed or timed-out background close is not evidence of a limit. It is a discard with no classification and it is not reported to the retry machine or the limit machine.
- **Reuse timing.** The connection returns to the pool when its close finishes, which is when it returned before (at Closed). A next open waits for it as §6 says, and no longer than today. Idle expiry (60 s from release) and the maximum age (reuse design §7) start at release as before.
- **Instances.** An instance with a closing connection has a connection, so it is not disposed as empty (reuse design §7). A host context's shutdown disposes it as in §5.

## 8. The pool contract

No gwz-transport code change is needed.

- `Lease::release(Reusable | Discarded)` is called once, when the exchange is over, as it is now at the end of the exchange (`gwz-transport:src/pool/lifecycle.rs:156-184`). `PoolHost::release` still refuses `Reusable` unless the resource is `Idle` (`ssh_pool.rs:150-157`; `ssh_setup.rs:277`).
- The lease is `Leased` for longer. The pool has no deadline on a lease, so nothing there expires it. The bound is the worker's (§4).
- **The contract text must change.** gwz-transport's `README.md:61-62` and `src/stream/machine.rs:175-176` say the endpoint calls `complete_close` only after backend cleanup has proved whether a connection is reusable. Option A calls it before that proof, and a later failure discards the connection. The text becomes: Closed reports the exchange; it does not assert the state of the physical connection, which the lease release alone settles. This is a required part of the design, with the crate's usual doc review.
- TR2.9's assertion "every close is clean" (`Disposition::Reusable`, `close_tests.rs:133-140`) is reworded to say the same. Reuse is asserted through `ssh_counts_for_test`, as it already is at `close_tests.rs:149-157`.
- In gwz-core, the comment on `ChannelResource::reclaim` ("acknowledged complete close", `ssh_worker.rs:51`) must change to "the channel's CHANNEL_CLOSE".

## 9. Expected effect, and what it does not fix

Numbers are from `report/tables.txt`, 30 rounds, 32 members, `--max-per-host 32`, paired against 1.0.17, medians.

| Host | Candidate now | Partial early-close prototype |
|---|---|---|
| Pi, 32 members | +0.177 s | +0.046 s |
| Mac, 32 members | +0.104 s | +0.079 s |
| Pi, one member | +0.046 s | +0.009 s |
| Mac, one member | +0.033 s | +0.007 s |

The partial prototype closed early but still waited for the server's CHANNEL_CLOSE on the member's path. Option A removes that wait as well, so I expect it to do at least as well as the prototype, with the connection back in the pool. The trailing receive span on the member's path goes to 0 for fetch, as for 1.0.17. The disposal change (§5) matters for the last members: without it the tail would move to the command's exit.

It does not fix:

- **Lost SYNs.** The candidate lost a SYN on 6.2 to 8.7% of Pi connections against 1.7 to 4.9% for 1.0.17, at about 1 s each (README finding 2). Why is not established. Spacing connects 1 ms apart helped the Pi. This is the larger part of the mean and of the runs over 3 s.
- **The worker's 1 ms poll** (`runner.rs:209-219`), about 3.5 times the baseline's CPU (README finding 3). A closing exchange keeps it running a little longer.
- **A push's server tail.** Receive-pack still waits for EOF (§3).

I cannot say that A closes the whole median gap. The wall times are dominated by the login tail and by SYN losses, so the paired median is the only figure to trust, and it needs a rerun of this run's runner after the change.

## 10. Tests (test-first)

Each must fail on `f48cf5a6`. Pump rows use `FakeChannel` and `pair()` (`ssh_tests/pump.rs`; fields `finish_status`, `output_eof`, `stderr_eof`, `eof_would_block`), with `finish()` held `WouldBlock` by a new flag. Real-server rows use the sshd fixture (`ssh_fixture.rs`, `common::SshdFixture`) with a forced-command script like `delayed_close_fixture` in `close_tests.rs`, which closes stdout and stderr, sleeps, then exits. That fixture takes its delay as a parameter (today the constant `CLOSE_DELAY`, 1 s). `delayed_eof_fixture` (new) sleeps before it closes stdout. `dropped_close_fixture` (new) closes stdout and stderr and then kills its parent sshd session (`kill -9 $PPID`), so the client sees the connection drop mid-close. A fixture whose script never exits is `stuck_close_fixture`.

| Test | Fixture | Asserts |
|---|---|---|
| `upload_pack_member_completes_before_server_close` | `FakeChannel`, `finish()` blocked, `output_eof = false`, `stderr_eof = false` (the server still running, as `server_still_running()` in the partial patch) | After the member's Close and the client's EOF, the stream is terminal and `close_result` is ready. `finished()` is false and `into_owner()` is `Err`. |
| `upload_pack_close_does_not_wait_for_server_eof` | `delayed_eof_fixture`, `UploadPack` | The member's `close()` returns in well under the delay. After the delay `idle == 1`, and the next open reports `reused` (this catches `wait_close` called before the drain, §4). |
| `receive_pack_close_waits_for_eof_not_for_close` | `FakeChannel`, then `delayed_close_fixture` with `ReceivePack` | No Closed before both EOFs. Closed arrives while `finish()` is still blocked. On the fixture `close()` returns before the delay. |
| `connection_is_reusable_only_after_the_close_finishes` | `delayed_close_fixture`, `ssh_counts_for_test` | Right after `close()`: `leased == 1`, `idle == 0`. After the delay: `idle == 1`. The next open reports `reused`. TR2.9's 8 and 32 cases gain a bound on each `close()` of well under the delay, and the reworded disposition assertion (§8). |
| `a_connection_in_background_close_counts_against_the_cap` | `delayed_close_fixture`, `--max-per-host 1` | A second open opens no second connection while the first closes (`physical_count` stays 1). It completes with `reused` after the close. |
| `a_failed_background_close_is_never_reused` | Pump: `FakeChannel`, `finish()` returns `BrokenPipe` after Closed. Worker: `dropped_close_fixture` | Pump: the tick errors, the channel is disposed, `into_owner()` is `Err`. Worker: `counts` go to zero, and the next open has `reused == false`. |
| `nonzero_status_after_completion_keeps_the_result_and_discards` | `FakeChannel`, `finish_status = 7`, receive-pack | Closed has been delivered. `into_owner()` is `Err`. (Replaces the pump half of `nonzero_service_status_poisoned_channel_cannot_be_reused`.) |
| `early_close_status_is_not_a_reuse_gate` | `FakeChannel`, upload-pack, `output_eof = false`, `finish_status = 141` | `into_owner()` is `Ok`. |
| `a_failed_terminal_ends_the_exchange_at_once` | Pump: `FakeChannel`, empty stdout, stderr `"repository not found."`, both EOFs. Worker: sshd fixture, `--max-per-host 1`, a refused path then a valid open | Pump: after Failed is drained, `retired()` is true within one tick. Worker: the lease is released `Discarded` in the handoff pass. The valid open completes with no allocation wait. |
| `a_stream_timeout_terminal_is_released_at_once` | Receive-pack on `silent_fixture` (never closes stdout), small `cleanup_ms` | The member's Close gives a Failed(Timeout) handoff, and the exchange is released `Discarded` in that pass, with no wait for `handoff_deadline`. |
| `a_closing_exchange_is_released_at_its_deadline` | Receive-pack on `stuck_close_fixture` (both EOFs, never exits), small `cleanup_ms` | The exchange is released `Discarded` at `handoff_deadline`, with no worker stop. |
| `an_open_is_deferred_at_most_once` | A 600 ms `delayed_close_fixture`; three exchanges closing one after another on the key; then one open | The open waits about 250 ms once, then goes to the pool. |
| `a_test_carrier_is_never_deferred` | A close in flight for an identity, and an open admitted as a test carrier for it | The open goes straight to a create-only checkout and makes a new connection. |
| `a_background_close_that_times_out_is_discarded` | `stuck_close_fixture`, `cleanup_timeout_ms` small as in `cleanup_tests.rs` | After the bound the connection is gone (`counts` zero) and the next open has `reused == false`. Run with `io_timeout_ms` 0 and again at 9000: same outcome. |
| `shutdown_discards_a_close_in_flight` | `stuck_close_fixture` with `ReceivePack`, both EOFs read, `cleanup_ms` 5000; `Endpoint::shutdown`, `shutdown_watch().status()` | `cleanup_complete` well under `cleanup_ms` (under 500 ms). `pending_connections == 0`. |
| `discard_after_use_does_not_wait_for_the_server` | `delayed_close_fixture`, `ReceivePack`, `discard_after_use` | The connection is gone in well under the delay. |
| `late_cancel_and_attachment_drop_do_not_discard_a_clean_close` | `delayed_close_fixture`, call `EndpointAttachment::cancel` and drop it after Closed | The connection still ends `idle == 1`. |
| `command_exit_does_not_wait_for_a_close_in_flight` | `delayed_close_fixture`, `ReceivePack`, `block_on(request.finish())`, `runtime.shutdown()` | `pending_local_work == 0`. `shutdown()` returns in well under the delay. |
| `ls_remote_then_push_reuses_the_closing_connection` | `delayed_close_fixture` at 100 ms, `--max-per-host 32`: an ls-remote, then at once a push on the same URL and identity | `physical_count` stays 1. The push reports `reused`. |
| `a_close_slower_than_the_wait_bound_opens_a_new_connection` | `delayed_close_fixture` at 600 ms, the same two exchanges | The push waits about 250 ms, then opens a second connection (`physical_count == 2`, `reused == false`). The first connection ends Idle after its close. |
| `the_wait_is_bounded_by_the_closes_remaining_cleanup` | `stuck_close_fixture`, `cleanup_ms` 100, the same two exchanges | The push proceeds at about 100 ms, not 250 ms. |
| `one_closing_connection_defers_one_open` | `delayed_close_fixture` at 100 ms, two pushes at once after one ls-remote | One reuses the connection. The other opens a new one without waiting. |
| `a_different_identity_is_not_deferred` | `delayed_close_fixture`, a second identity | The second open does not wait. |

## 11. Open questions for the operator

1. **Upload-pack closes on the member's Close, not on the server's EOF (§3).** The evidence cannot separate the server's EOF from its CHANNEL_CLOSE: the trailing span runs from last data to close. The prototypes skipped both. *Recommended:* yes. A fetch only reads. Without it, option A would leave an EOF wait that may hold much of the tail. **Decided (operator, 2026-10-07): as recommended.**
2. **Push keeps its EOF wait.** The alternative is to end a push at the report-status flush, which means parsing receive-pack output (`GitTurns` stops at the client's first byte, `git_turns.rs:87-91`). *Recommended:* keep the EOF wait. The 32-member gap is a fetch, and a push is cut off wrongly if it closes without report-status. **Decided (operator, 2026-10-07): as recommended.**
3. **The exit status as a reuse gate.** *Recommended:* ignore it after an early close, and keep today's discard for a nonzero status after the server's EOF (§4). **Decided (operator, 2026-10-07): as recommended.**
4. **The bound.** `cleanup_ms` (5 s) holds a stuck close's slot for up to 5 s, and a typical close takes 40 to 75 ms. A shorter reuse window frees the slot sooner but costs reuse on a slow server, and it needs a constant that no evidence yet supports. *Recommended:* `cleanup_ms`, with no new setting. Revisit after a live run. **Decided (operator, 2026-10-07): as recommended.**
5. **Late failures.** A background close that fails, times out or returns a nonzero status is a discard. *Recommended:* count it in the transport observations at debug level, and report nothing to the member, the retry machine or the user. **Decided (operator, 2026-10-07): as recommended.**
6. **Closed's disposition.** *Recommended:* keep sending `Reusable` for a clean exchange, and change the contract text (§8). A new variant is a wire change for no reader. **Decided (operator, 2026-10-07): as recommended.**
7. **The wait of 250 ms (§6).** The operator chose a bounded wait. *Recommended:* a constant in the worker, not a setting. If a live run shows the cost at the cap (§6), make the wait end when any connection of the key goes Idle, which needs the pool's help. **Decided (operator, 2026-10-07): as recommended.**

## Changelog

- **Revision 2, decisions (2026-10-07).** The operator accepted §11's seven recommendations.
- **Revision 2 (2026-10-07), after the State re-review (GO, 3 P3, `-ReviewState-2.md`).** N-1: a request is deferred at most once. N-2: an open admitted as a test carrier is never deferred (to be mirrored in the adaptive design's next revision). N-3: the stream-Timeout row is rebuilt on a fixture that never closes stdout; the deadline row is named for what it tests. Rows added for N-1 and N-2. No re-review needed on this axis for these (the reviewer's §3).
- **Revision 1 (2026-10-07), after the State review (NO-GO, 3 P2, 3 P3).**
  - **P2-1:** §4 defines "over" (terminal handed over and finished, or retired, or Failed), arms the bound at the handoff and moves it to the worker, and releases a Failed terminal `Discarded` in the handoff pass. D3, D4. New tests in §10.
  - **P2-2:** §5 specifies the `SshChannel::poll_dispose` change (a closing phase goes straight to `force_dispose`). §1 and D5 now say what that makes true. Stop and discard-after-use use it. §10 rows for shutdown, discard-after-use and command exit use receive-pack.
  - **P2-3:** new §6, option (a), "Wait briefly": the worker defers a same-key, same-identity open for `min(250 ms, remaining cleanup)`, one open per closing exchange. D6, D7. Its place in the worker, the adaptive interaction and its cost are stated. Tests added.
  - **P3-1:** §7 replaces "no number differs" with the Connected-versus-Closing semantic change for amendment 2.
  - **P3-2:** §8 makes the `complete_close` contract text change in gwz-transport required. D7 says no code change. TR2.9's disposition assertion is reworded.
  - **P3-3:** §10 rows 2, 6 and 8 corrected. §4 names the drain-then-`wait_close` order and the two libssh2 preconditions.
  - **Also:** §3 citations corrected (`smart_protocol.c:753`, `:782`). §4 "Clocks" corrected: up to about 10 s of slot time for receive-pack.
- **Revision 0 (2026-10-07).** First draft.
