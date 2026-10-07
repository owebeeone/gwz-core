# GWZ transport SSH background close design — STATE-AXIS REVIEW, ROUND 2

**Review object:** `gwz-core/dev-docs/GwzTransportSshBackgroundCloseDesign.md`, revision 1, 208 lines, sha256 `5dca6f80df3594f35b5c942274e7d0a4168de1c3e049ae592084ae30b318d42a`. The file is untracked on purpose and is marked DRAFT. Its hash was checked at the start and at the end of this review and did not change.
**Baseline:** gwz-core `f48cf5a6`, gwz-transport `ff6083b5`. Sources were read only through `git show`. libgit2 is the fork `b172e3d`. libssh2 is from the `libssh2-sys 0.3.3` registry source. The reference for the earlier findings is the round-1 report, `GwzTransportSshBackgroundCloseDesign-ReviewState.md`.
**Date:** 2026-10-07
**Axis:** State: state machines and adversity. Independent, adversarial, read-only. Filed verbatim by the lane owner.
**Verdict: GO.** The six round-1 findings are closed: P2-1, P2-2, P2-3, P3-1, P3-2 and P3-3. This round found 0 P0, 0 P1, 0 P2 and 3 P3. Nothing is architectural. The round-1 pre-commitment, GO once those six were resolved as specified, is honoured. The three new P3 findings should be fixed in the next revision. They do not block.

---

## Prior-finding closure table

| ID | Round-1 counterexample | Re-trace on revision 1 | Status |
|---|---|---|---|
| P2-1 | A Failed terminal (a refusal through `fail_terminal`, or a stream Timeout) is handed over. The pump retires the channel and returns `Ok`. No row of the old end-of-exchange table matched, and `closed_at` was never armed, so the lease was held until the worker stopped. | §4 lines 72–75 record `terminal_failed` at the handoff pass and arm `handoff_deadline = now + cleanup_ms` there, for any terminal. "Over" is defined as handed over and (`finished` or `retired` or Failed). Line 82 releases a Failed terminal `Discarded` in the handoff pass, which matches `runner.rs:193` today. Line 67 keeps the retire on a terminal that was not Closed, which sets `invalidated`. In the refusal interleaving, the handoff pass now gives Failed, then over, then `Discarded`, so the lease is freed at once. The timeout interleaving takes the same path. The bound now covers every terminal. | Closed |
| P2-2 | A receive-pack's libssh2 close is still waiting (`channel.c:2706`). At stop, `poll_dispose` calls `channel.close()` a second time, gets EAGAIN and `WouldBlock`, and stays Pending until the server's next packet or the Abort at `cleanup_timeout_ms`. The command's exit therefore paid the server's tail. | §5 line 99: `poll_dispose` reads the phase before `abort()` overwrites it, and `Close` or `WaitClose` goes straight to `force_dispose` without a second libssh2 close. At handoff, a closing exchange is always in `Close` or `WaitClose`: receive-pack calls `finish` in `finish_request`, before `finish_channel` in the same tick (`ssh_pump.rs:468-469`), and the early upload-pack close sets `Close` at its channel close. So stop, discard-after-use and a missed deadline all terminate at once. Rows 178, 179 and 181 use receive-pack with both EOFs read, which is the case that failed in round 1. | Closed |
| P2-3 | An ls-remote then a push on the same URL, below the per-host cap. The push finds nothing Idle (`allocation.rs:17-58`), so it creates a second connection with a full SSH setup. | §6 adds a worker-side deferral: same pool key, same pool identity (`runner.rs:131-146`, including the helpers-disabled partition), one claim for each closing exchange, bounded by `min(250 ms, handoff_deadline - now)`. Re-traced: the push is deferred, the close releases `Reusable`, and the checkout leases the Idle connection (`reused`). One gap remains (N-1 below): the text does not say that a request is deferred at most once. Rows 182–186 pin the reuse case, the wait bound, the cleanup clamp, exclusivity and identity. | Closed (residual N-1) |
| P3-1 | "No number differs" was wrong: `Connected` drives §4.2 to §4.5 of the adaptive design. | §7 line 125 now calls this a semantic change. It names the Closing → Idle transition that §4.1 cannot represent and asks for a §10.2 fairness case. | Closed |
| P3-2 | gwz-transport's published `complete_close` contract text (`README.md:61-62`, `machine.rs:175-176`) became false, but the fix was marked optional. | §8 line 136 makes the text change required. D7 now says no code change, rather than no change. TR2.9's disposition assertion is reworded. | Closed |
| P3-3 | Row 6 used `FakeResource`, which has no pump, so the worker could not attach it. Row 8 did not say the server was still running. No real-server row tested reuse after an early close. The patch order was ambiguous. | Row 6's worker half now runs on `dropped_close_fixture`. Rows 1 and 8 set `output_eof = false` (and row 1 `stderr_eof = false`), so the server is still running. Row 2 asserts `idle == 1` and `reused`. §4 line 65 names the order (drain until `channel.eof()`, then `wait_close`) and both libssh2 preconditions (`channel.c:2706`, `:2756-2760`; `packet.c:1234-1235`). See the note in §0 on `kill -9 $PPID`. | Closed |
| Notes | The citations to `smart_protocol.c` and the Clocks text, from round 1 §2. | §3 now cites `:753` and `:782`. §4's Clocks text states the receive-pack slot can be held for about 10 s. | Closed |

## Changed-range analysis

The revision was treated as a closed set of rules. Each changed range was checked against code at the baseline SHAs.

- **§4, "over", the deadline and the end-of-exchange table (lines 67–91).**
  - Every state now has an exit. Running ends at the terminal handoff. Closing ends at over or at `handoff_deadline`. A Failed terminal or discard-after-use is over at the handoff.
  - A claimable exchange is "closing": handed over and not over. A Failed exchange or a discard-after-use exchange is therefore never claimed.
  - If over and the deadline fall in the same pass, either release is safe. A `finished` clean exchange may be reused, and `Discarded` is always safe.
  - Fail-closed direction is kept. Every table row other than "finished, clean" releases `Discarded`. A clean `Reusable` release still needs `reclaim`, then `into_owner`, which requires `!invalidated` (`ssh_pump.rs:223`). A missing state check in the implementation could therefore cost reuse, but could not cause a dirty reuse.
- **§5, forced disposal of a closing channel. Can it ever discard a connection that would have closed clean? No.**
  - `NativeResource::poll_dispose` runs only from the pool's Close or Abort action (`ssh_pool.rs:333-340`, then `:207-221`). That happens only after a `Discarded` release, a dropped lease, or `pool.shutdown()`.
  - A `Reusable` release goes through `reclaim`, which takes the pump and never disposes it (`ssh_setup.rs:332-357`).
  - The rule therefore only shortens the disposal of connections that are already being discarded. At normal command end those are stop disposals, and idle connections are disposed at stop anyway.
  - The phase cases:
    - `Finished` was already immediate, because libssh2 sees `local.close` and returns 0.
    - `Failed` comes from pump errors, which already force (`retire_channel`).
    - `Open`, `Exec` and `Active` keep today's path, as the doc says.
- **§6, the deferral.**
  - **Deadlock.** None. A deferred request holds only its admission permit (`OpenRequest.permit`, `open_request.rs:21`). The claimed exchange needs no permit and no pool action to end. The deferral is bounded by 250 ms and by the close's own deadline.
  - **Starvation.** One release is single-pass. N-1 covers the gap.
  - **The pool's own Waiting requests at the cap.** `release` runs `schedule()` inside the pool lock (`lifecycle.rs:181`). An older Waiting request with the same key and identity therefore leases the connection before the deferred request reaches checkout. The deferred request then queues normally. That is a bounded fairness loss of at most 250 ms, which §6 "What it costs" already admits. It is not a deadlock.
  - **Below the cap.** Requests that are not Idle-eligible are in `Opening`, not `Waiting` (`allocation.rs:47-58`), so the Idle connection goes to the deferred request as §6 says.
  - **Race with a close that ends `Discarded`.** The request proceeds to an ordinary checkout. The slot stays in Closing until the forced disposal (§5), so at the cap the request waits for `closed()` and then creates a connection. That is correct.
  - **Allocation clocks.** The pool's allocation timer starts at `checkout_until`, so the deferral comes out of the open's absolute `request.deadline`, at most 250 ms. Cancellation and expiry are covered, because `expired()` includes `cancelled` (`open_request.rs:171-189`) and §6 applies the pending expiry check to deferred requests.
  - **Wake-up.** The release changes the pool revision and wakes the driver waker (`asynchronous.rs:25-47`). That is the worker's `ThreadWake`. So a deferred request is not left parked for the 1000 ms idle wait (`runner.rs:209-219`) once its claimed exchange is released.
  - **Determinism.** Exclusivity is deterministic: one claim per closing exchange. Which exchange is claimed depends on the order of `active`, which `swap_remove` changes. That choice does not affect correctness, and row 185 asserts only the outcome.
  - **Interaction with the adaptive design.** See N-2.
- **§10, the rows.** Each row can now be built with the fixtures named, except row 176, which tests a different rule from the one its name gives (N-3).

## 0. Evidence base

- **Re-read at the baseline SHAs for this round:**
  - gwz-core: `runner.rs` (loop order: incoming checkouts, then the release loop, then park), `ssh_worker.rs:298-365`, `ssh_pump.rs:364-470`, `ssh_channel.rs:131-206`, `ssh_setup.rs:225-363`, `ssh_pool.rs:150-157` and `:333-357`, `open_request.rs:140-214`.
  - gwz-transport: `pool/allocation.rs:4-80`, `pool/lifecycle.rs:44-62` and `:156-184`, `pool/asynchronous.rs:25-52` and `:265-278`.
  - Adaptive design §4.5, on test carriers.
  - `ssh_fixture.rs:106-118` and `:270-325`; `helper_script.rs:36-42`.
- **Note on `dropped_close_fixture` (not a finding).** The helper scripts are `#!/bin/sh`, started by sshd through the user's shell with `-c`. `kill -9 $PPID` kills the per-connection sshd only if that shell execs the script. If it does not, the kill hits the shell instead, and sshd closes the channel cleanly with an exit-signal; libssh2 then reports status 0. In that case row 6's worker half fails red; it cannot pass falsely. The fixture already has process-tree tooling (`pause_process_tree`, `ssh_fixture.rs:293-325`) that can find the right process. Confirm when the fixture first runs.

## 1. Findings

### [P3-N1] §6 does not say a request is deferred at most once

- **Location:** §6, lines 109–114.
- **Invariant:** A deferral is bounded per open, so it cannot turn an open that would succeed into a timeout.
- **Interleaving:**
  1. The rule fires when the worker is "about to call `pool.checkout_until`". A request whose deferral has ended is, again, about to call it.
  2. It finds a different closing exchange that nobody has claimed, and claims that one.
  3. Assume a slow server, with closes over 250 ms (row 183 uses 600 ms), and many members on the key, so a new close begins in each window. Each wait then ends at 250 ms.
  4. The request can chain 250 ms deferrals until `request.deadline`. It then fails with `Timeout` without ever trying a connection. Read literally, the text allows this.
- **Impact:** A member can fail with a misleading Timeout under close churn. It is bounded by the open's deadline, and the likely implementation goes straight to checkout, so this is P3.
- **Correction:** Add "A request is deferred at most once. When its deferral ends it goes to `checkout_until` without looking for another closing exchange."
- **Closure test:** A slow-close fixture at 600 ms, three exchanges closing one after another on the key, then one open. The open waits about 250 ms once, then goes to the pool.

### [P3-N2] §6 calls the deferral invisible to the adaptive machine, but it can quietly turn an admitted test into a reused lease

- **Location:** §6, line 118, "The deferral adds no connection and no wait that the machine can see". Adaptive §4.5 says a test carrier is a member that "needs a new connection (one that cannot lease an idle connection of its identity)".
- **Invariant:** A test the machine admits either produces a setup result or is re-armed.
- **Interleaving:**
  1. A key is quiet. A background close counts as Connected, not Closing, so it does not stop the key being quiet.
  2. A probe test is admitted for a queued member, because nothing is Idle.
  3. The worker defers the open on that member's identity, the close ends `Reusable`, and the open leases it.
  4. The test's setup never happens. Under §4.2 a lease is not a setup, so it has no result to judge.
- **Impact:** Once the adaptive design is implemented, the test slot can be held or misjudged. The adaptive design is not implemented yet, so this is P3.
- **Correction:** Pick one and state it in §6, and in amendment 2 of the adaptive design. Either an open admitted as a test carrier is never deferred, or an open admitted as a test that ends `reused` re-arms the test as not run.
- **Closure test:** Add an adaptive §10.2 case: a probe test admitted while a close for its identity is running either opens a new connection or is re-armed. Pair it with the §7 fairness case.

### [P3-N3] Row 176 tests the background-close deadline, not the stream-Timeout terminal its name claims

- **Location:** §10, row 176, `a_stream_timeout_terminal_is_bounded`. The fixture definition is at line 163, and rows 177–178 use the same fixture.
- **The defect:** Rows 177–178 show that `stuck_close_fixture` closes its outputs and never exits ("both EOFs read"). So in row 176 a receive-pack reaches Closed, and the exchange is released at `handoff_deadline`. That is table row 86, the same rule row 177 already tests. A real stream Timeout terminal is round 1's second P2-1 interleaving: the server sends no EOF within `cleanup_ms` of the member's Close. The table says that case is released `Discarded` in the handoff pass, not at the deadline (line 82). No worker row produces it. Only the refusal row (175) reaches the shared `retired()` path.
- **Impact:** A regression in the Timeout-terminal release would not be caught at the worker level.
- **Correction:** Either rename row 176 to describe the deadline it tests, or rebuild it on a fixture that never closes stdout (say `silent_fixture`). Then assert that a receive-pack Close with small `cleanup_ms` gives a Failed(Timeout) handoff and an immediate `Discarded`, with no wait for `handoff_deadline`.
- **Closure test:** The corrected row fails on `f48cf5a6` and passes on the implementation.

## 2. Invariant analysis

- **Result finality: unchanged and holds.** §3 is the same apart from its citations. Round 1's analysis stands: libgit2 fixes the result before `git_remote_disconnect`, and exit status is used only to decide reuse.
- **No dirty reuse.**
  - A `Reusable` release needs `finished`, `close_completed`, `closed_drained` and `!invalidated` (`ssh_pump.rs:223`).
  - `finished` comes only from a successful `wait_close`, which needs `remote.close` (§4 line 65).
  - A deferred open only leases an entry that the pool has already made Idle.
  - Forced disposal touches only connections already being discarded.
- **No stuck states.**
  - A running exchange ends at the terminal handoff, or by the existing cancel and error paths.
  - A closing exchange ends at over or at `handoff_deadline`.
  - A deferral ends at the claimed release or at `min(250 ms, remaining)`. N-1 makes that single.
  - On stop, deferred requests complete `stopped` and closing exchanges are terminated at once.
- **Lock scope.** All new state (`deferred`, claims, `handoff_deadline`) is owned by the worker thread. The pool is touched only through the existing `checkout_until` and `release`, so there is no new shared lock.
- **Fail direction.** Every new failure mode found costs at most time (N-1) or reuse (the cap fairness §6 admits). None loses or misreports a member's result. N-1's Timeout comes after the result is lost to the deadline, not by corruption.

## 3. Risks and next action

- **Residual risk:** In the field, the fairness cost of deferral at the per-host cap (§6, OQ7) is the one cost not yet measured. The rerun planned in §9 should record how often opens are deferred and reused.
- **Next action:**
  - The drafter folds N-1 and N-3 into revision 2 as text edits. No re-review is needed on this axis for those.
  - N-2 goes to amendment 2 of the adaptive design.
  - The lane owner checks `dropped_close_fixture` the first time it runs (§0).
- **Verdict:** GO.
