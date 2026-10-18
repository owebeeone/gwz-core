# GwzTransportIdleLossDesign: State review

## Tuple (verified at start and at end, unchanged)

- gwz-core `b5e286b34a37371e3dedf9b4630ab4e74a2e02b3`, diff from `959171c634d7ac47f720279a77e8166bb43db826`: 23 files, +1652/−116.
- gwz-transport `2d41e93069c275ec59ce8897adf19613740b20c7`, diff from `ff6083b5230e06dccdddb251e5a76175535c6cf3`: 4 files, +87.
- Controlling note: `gwz-core/dev-docs/GwzTransportIdleLossDesign.md` at b5e286b3. It is a DRAFT and records the operator's decisions in §10.
- How it was read: through `git show`, `git diff` and `git grep` only. Nothing was built or run (see "Commands run").

## Verdict

**NO-GO.** One P2 is open. I pre-commit to GO on a revision that resolves P2-1 as specified.

## Summary

The state machinery holds up against attack:

- **Idle watch:** the SSH watch's fd lifetimes and the reactor's lifetime are sound.
- **Lost placeholder:** an entry marked `Lost` cannot be leased again, is released straight away, and ends in `closed()`.
- **Single fresh retry:** SSH retries exactly once, only before any Git byte, and charges no attempt.
- **Cancellation:** a held open is cancelled correctly.
- **Fail-closed direction:** nothing invents success or reuses a foreign session. Every defect found loses a connection or a retry, which the axis allows.

The one blocking defect is about composition, not runtime. b5e286b3 uses the new `Request::fresh`, but its own CI pin still names a gwz-transport commit that does not have the field. CI never ran on b5e286b3.

Five P3s record:
- a host state that the code reaches but its predicates ignore;
- tests that would pass with the host-side `fresh` flag removed;
- three places where the note's claims (retry count, fairness, deadlines) are looser than the code.

## Findings

### P2-1: b5e286b3 needs `Request::fresh`, but its CI pin names a gwz-transport commit without it

- **Root cause:** the change compiles against a gwz-transport field added in 2d41e93, and `.github/gwz-transport.commit` was not moved in the same change.
- **Where:**
  - At b5e286b3, `.github/gwz-transport.commit` pins `d0be15a8f76071a0a7e23ef39fd39678bada05fe`. In that commit, `src/pool/mod.rs` `struct Request` has no `fresh` field. The only match for "fresh" is the comment at line 157.
  - The field is used in `src/git/endpoint/https_pool.rs` (`checkout_scoped`: `request.fresh = fresh;` and `request.fresh = true;`) and in `src/git/endpoint/ssh_worker/held.rs` (`Unserved::settle`: `policy.fresh = true;`).
  - `.github/workflows/transport-candidate.yml` checks gwz-transport out at the pin. `tests/transport_backend/prepare.py:75` then builds against that checkout by path.
- **Violated invariant:**
  - A gwz-core commit's pin names a pushed gwz-transport commit it actually builds with ("upstream goes first").
  - The gates quoted in the commit message describe the composition that the commit declares.
- **Reproduction:**
  1. Put gwz-core at b5e286b3 and gwz-transport at d0be15a side by side.
  2. Run the prepare script, then build with `RUSTFLAGS=--cfg gwz_transport_candidate`.
  3. The build fails: `Request` has no field `fresh`.
- **Evidence:**
  - `gh run list --workflow transport-candidate.yml` shows runs for 959171c6 and 2a12006f and none for b5e286b3. The gates in the commit message ("candidate endpoint 451/451") were therefore run against a sibling gwz-transport checkout at 2d41e93, not the pin.
  - The later commit 2a12006f (out of scope) says it ".github/gwz-transport.commit moves to gwz-transport 9eef731, which carries Request::fresh". 9eef731 descends from 2d41e93.
- **Impact:** b5e286b3 is a pushed main commit whose own candidate CI cannot compile. A bisect, a release cut or any CI re-run from it fails.
- **Why P2 and not P0:** it fails closed and loudly at compile time, no binary is mis-composed, and the root lock (13117218) records the correct pair.
- **Required correction:**
  - Pin a pushed gwz-transport commit that has `Request::fresh` (2d41e93 or a descendant), in the same commit that first uses it.
  - Make the local gate refuse to pass when the sibling gwz-transport HEAD is not the pinned commit, so local results cannot stand in for CI's composition again.
- **Closure / regression test:**
  - Closure: a green transport-candidate run on a commit whose pin contains `Request::fresh`. The pin move in 2a12006f and its run of 2026-10-07T15:51:35Z already satisfy this.
  - Regression: a check in `run_tests.py` (or prepare.py) that fails when `git -C ../gwz-transport rev-parse HEAD` differs from `.github/gwz-transport.commit`.

### P3-1: a connection the host knows is lost can still be leased while its disposal is pending, and nothing recognises it

- **Root cause:** `PoolHost::lost()` and `PoolHost::release()` recognise only `Phase::Lost`. They do not recognise `Phase::Disposing { idle_lost: true }`. In that phase:
  - the pool still counts the entry Idle (the code comment says "Disposed in this same pass: the pool still counts it Idle");
  - so the pool can lease it.
- **Where:** `src/git/endpoint/ssh_pool.rs`, functions `lost`, `release`, `resource` and `allocation_reused`, and the `idle_lost` branch of `step_reported`.
- **Violated invariant:** after this change, a dead idle connection should never fail its member (note §7: "F14 is closed"). Instead, correctness rests on note §4.1's unenforced assumption that "an idle SSH or HTTPS resource disposes within the same pass".
- **Reachability today:**
  - It holds for `NativeResource`: disposing from `State::Idle` drops the session synchronously.
  - It holds for `HttpResource` only because the production HTTPS runtime is `new_current_thread` (`transport_host/https_endpoint.rs:126`). `poll_dispose` can return Pending when:
    - `Arc::strong_count(connection) > 1`;
    - `try_lock` fails;
    - the driver's `JoinHandle` is polled after the `ended` sender has dropped but before tokio has marked the task complete (possible on a multi-thread runtime).
  - The generic host supports Pending disposal on purpose: the new test `idle_loss_is_disposed_then_reported_and_frees_the_slot` exercises it.
- **Interleaving (with any resource whose idle disposal is Pending):**
  1. A step finds the connection lost and sets `Disposing{idle_lost:true}`; disposal is Pending.
  2. A checkout leases the entry, which the pool still counts Idle.
  3. `attach` or `adopt` calls `lost()`, which returns `None`.
  4. `allocation_reused()` returns `WrongState`.
  5. The open fails with Io and `Phase::Other`, which the retry machine Returns. This is exactly F14.
  6. Meanwhile the pool counts as Idle/Connected a connection the host already knows is dead.
- **Required correction:** either
  - treat `Disposing { idle_lost: true }` like `Lost` in both `lost()` and `release()` (a release becomes Discarded; disposal then ends through the pool's Close, or through `idle_closed` if that arrives first); or
  - state the same-pass requirement on `Resource::poll_dispose` for idle resources and enforce it with a debug assertion.
- **Closure test:** in `ssh_tests/pool_host.rs`:
  1. Set `lost = true` and `finish_close = false`, then tick.
  2. Check out and take the lease.
  3. Assert `lost(&lease) == Some(true)` and that `release(lease, Reusable)` succeeds as Discarded.
  4. Set `finish_close`, tick twice, and assert `total() == 0` and `physical_count() == 0`.

### P3-2: no test sets or checks the host-side `fresh` flag, and §8's "the next checkout is fresh" is not what is tested

- **HTTPS:** `https_pool/idle_tests.rs` calls `pool.adopt` directly, then a plain `pool.checkout(...)`, which passes `fresh = false`. Its `!fresh.reused` holds only because the only idle connection is gone. The test never runs `checkout_scoped`'s `Adopted::Dead` branch.
- **SSH:** `ssh_tests/idle_loss.rs::a_reused_session_dying_at_its_channel_open_is_replaced_by_a_fresh_one` has a single idle session. The re-checkout gets a new connection whether or not `Unserved::settle` sets `policy.fresh = true`.
- **Violated invariant:** the note's exactly-once guarantee ("A fresh request never gets a reused one, so the retry cannot be Dead again") depends on the hosts setting `fresh`. Only gwz-transport's pool-level `tests/pool_fresh.rs` proves the pool honours it.
- **Concrete consequence:** both of the following regressions pass the suite:
  - deleting `request.fresh = true` / `policy.fresh = true`, which lets the retry lease another dead idle connection and so retry more than once;
  - replacing `checkout_scoped`'s Dead branch with an error, which restores F14 for HTTPS dead-at-lease.
- **Correction / closure:**
  - SSH: with two idle sessions (config `total: 2`), `arm_existing`, then one open. Assert it succeeds with `reused == false`, `proxy.connections() == 3`, and that the second idle session is still idle (so not leased by the retry).
  - HTTPS: the same through `checkout_scoped` with two idle connections, where the forced race kills one.
  - Each test must fail with the corresponding `fresh = true` line removed.

### P3-3: HTTPS retries once per request, not once per open as the note and the code comment state

- **Where:** `https_worker/prepare.rs::run_attempt`, the `SendFailure::NotStarted if prepared.opened.reused` arm, together with `ChallengeLease::take_for` (`lease.reused = true`) and `checkout_scoped`'s Dead branch.
- **Sequence:**
  1. Hop 0 reuses an idle connection that is found dead: either Dead at adopt, which retries fresh inside `checkout_scoped`, or the GET is NotStarted, which also retries fresh.
  2. The fresh connection answers 401 Basic and is carried, with `reused` set true.
  3. The credentialed GET on the carried lease is NotStarted, so it retries fresh a second time.
  4. Each redirect hop's non-fresh checkout can do the same again.
- **Conflicts with:**
  - note §2.2 ("retried **once**");
  - note §2.3 ("a second failure of the same open" is never retried);
  - note §6.1 ("Once only");
  - the code comment "a request is retried at most once" (true per request, not per open).
- **Consequence:** bounded (one carry, at most 5 redirects) and safe, because every retry is a request that was never started. But the uncharged retries per attempt can exceed one, against the documented boundary.
- **Correction:** either word the rule as "once per request" in the note and the comment, or keep a per-attempt `retried` flag that blocks a second uncharged retry.
- **Test:** the forced dead-lease race followed by a 401-carry NotStarted, asserting whichever rule is chosen.

### P3-4: under sustained compatible demand, a fresh retry is served only after every other waiter

- **Where:** gwz-transport `src/pool/allocation.rs::schedule`. The reuse loop skips a request with `fresh`, and every released connection goes to the earliest compatible non-fresh waiter. A fresh request is served only by:
  - the creation loop, when there is room; or
  - the eviction loop, when an idle entry survives the reuse loop.
- **Interleaving (SSH, per-host cap 8, a large fan-out, `admits_open` refilling waiters as replies arrive):**
  1. The retry F re-queues after other non-fresh opens are already waiting.
  2. Each release becomes Idle and is leased straight away to a non-fresh waiter.
  3. F gets a slot only once no compatible non-fresh waiter remains, or fails with `AllocationTimeout`, which is classified Return.
- **Consequence:** bounded by the allocation deadline and fail-closed. Before this change the member failed outright, so this is not a regression. But the recovery path can lose its member exactly when the key is busy, and no test covers contention.
- **Correction:** either
  - let a waiting fresh request that is earlier in FIFO order claim an idle compatible entry for eviction ahead of later non-fresh waiters; or
  - record the limitation in the note as accepted.
- **Test:** a pool-level case with an earlier fresh waiter, a later compatible waiter and one release, asserting whichever rule is chosen.

### P3-5: note §6.2 says the allocation deadline no longer applies once the open holds a lease; in fact the open's attempt deadline now covers the held wait

- **Where:**
  - `placement_endpoint/completion.rs::finish_opens` abandons an open at `attempt_deadline`, which is allocation + connect + interaction (`admission.rs::deadline_from_open`). It reports the open as `Timeout` and sets `cancelled`.
  - The worker's `request.deadline` is the same sum (`ssh_worker/endpoint.rs::start_endpoint_open`).
  - The reply is now held until the channel opens, and a Dead retry re-checks-out against that same absolute deadline (`held.rs::settle`).
- **Consequence:**
  - Before the change, this deadline could never fire after a lease, because the reply was immediate.
  - Now, an open leased near the deadline whose channel opens just after it fails with Timeout, and its held session (possibly healthy) is discarded.
  - The behaviour is fail-closed and reasonable, but §6.2 misstates the deadline that governs the wait and the retry.
- **Correction:** state in §6.2 that the attempt deadline (allocation + connect + interaction) bounds the held wait and the fresh retry, and that at that deadline the placement abandons the open as Timeout.

## Attacked and found sound

**SSH idle watch: fd and dup lifetime**
- `try_clone` gives a close-on-exec dup.
- `IdleSocket` declares `stream` before `_reactor`, so the stream deregisters before its reactor reference drops.
- tokio deregisters by token before closing the fd, so a reused fd number cannot alias a stale registration.
- The watch exists only in `State::Idle`. `start_exchange` drops it before the session moves into the channel, and any non-Idle poll clears it.
- `O_NONBLOCK` is shared between the dup and the original, but libssh2 already sets it at handshake.
- `SshConnection` drop calls `terminate()` (shutdown), which ends the TCP connection even while the dup survives in a `Lost` tombstone.

**SSH idle watch: races and wakes**
- Every watch and host mutation runs on the single worker thread; the reactor thread only signals wakers.
- A wake for a connection that is leased and attached is Pending because the state is not Idle.
- A lease that is not yet attached takes the designed `Lost` path. `attach` runs in the same pass as the lease, after `connected`, so a new connection gets no idle poll before its first exchange. This also means bytes sshd sends after authentication (`hostkeys-00@openssh.com`) cannot kill a new session.

**SSH idle watch: reactor shutdown**
- The last `Arc` drop joins the thread.
- The reactor thread holds no `Arc<IdleReactor>` (only wakers), so it can never join itself.
- The runtime is dropped on its own thread after `block_on`.

**SSH idle watch: keepalive and SSH_MSG_IGNORE**
- Any byte counts as lost, so a healthy session gets discarded. That costs a connection, never correctness.
- An idle libssh2 session cannot answer `keepalive@openssh.com`, and sshd would end it after ClientAliveCountMax anyway.

**Lost placeholder**
- The pool holds a `Lost` entry as Leased until the lease is released, so it cannot be leased again.
- `attach` and `adopt` release it synchronously.
- An unclaimed ready lease that is dropped reaches `start_closing` (`fail_request`).
- Close or Abort moves it to `Disposing`, and both resources dispose idempotently: `State::Disposed` is Ready for SSH, and `connection None` is Ready for HTTPS.
- `closed()` is accepted because `cleanup.sent` or `aborted` is set.
- `idle_closed` on a pool-Closing entry whose Close has not been dispatched is safe: `next_action` builds actions from entry state, so removing the entry drops the pending Close and no `Stale` arises.
- Pool shutdown closes `Lost` entries through the same path.

**Single fresh retry (SSH)**
- A retry is marked Dead only when `reused` is true (`entry.used` / `allocation_reused`).
- A fresh request never leases an idle connection, and an Opening connection stays bound to its own request (`connected`), so the retry's lease always has `used = false`. The retry cannot be retried and cannot loop.
- Cancellation while held: `Held::cancelled` is checked every pass, before `transfer`. `ConnectionAborted` maps to `ErrorCode::Cancelled` (`open_request.rs:94`).
- A pump that ends before the channel opens retires the channel. `into_owner` requires `!invalidated`, so such a session is never released Reusable.
- If the worker stops while an open is held, dropping the reply sender makes `PendingOpen` see Disconnected (`stopped()`).

**No byte written before the judgement**
- SSH: `write_forward` runs only after `opened`, and Git holds no attachment before the reply. An exec that reached the server runs receive-pack with no commands, so it updates no refs.
- HTTPS: path (a) judges at adopt, before any request. Path (b) relies on `try_send_request` returning the message only when the request never started. A POST is never retried.

**Attempt charging and the retry and limit machines**
- No retry reaches `attempt_failed` or `RetryMachines::settle`.
- The fresh connect's own failure carries `Phase::Setup` and is classified as the attempt's setup, consistent with the retry plan's §4.
- An idle loss never reaches the retry or limit machine as a refusal.
- `idle_closed` removes the entry in the same pass with no Settling, which is correct because the server closed it.
- A `Lost` entry's Close passes through Closing. That overstates Possible but never understates Connected, the conservative direction in the adaptive design's §4.1.

**Restart legality**
- A retry uses the same key, identity and HTTPS scope.
- The HTTPS credential comes back from the per-route cache with no reject side effect.

**Tests that fail with their mechanism removed**
- the `idle_watch` tests;
- `idle_loss_reaches_the_host_through_the_reservation`;
- the SSH idle-close test;
- the reused-dying and cancelled-held tests, both of which need the held reply;
- the HTTPS idle-close and forced dead-lease tests, which need `poll_idle_lost`.

**Deferred items:** the operator's decisions of 2026-10-07 (§10) are respected, and none of them is reported as a finding.

## Commands run (all read-only)

- `git -C gwz-core rev-parse | log | diff --stat | show <rev>:<path> | grep <pat> b5e286b3 | cat-file`, for revisions 959171c6, b5e286b3 and 2a12006f (that commit's message and pin file only).
- `git -C gwz-transport rev-parse | log | diff | show | grep | merge-base --is-ancestor`, for ff6083b, 2d41e93, d0be15a and 9eef731.
- `git show 13117218` in the root, for the stat and message.
- `grep` over `dev-docs/AgentProcessRules.md` and `dev-docs/GwzProcessOptimization.md`.
- `gh run list -R owebeeone/gwz-core --workflow transport-candidate.yml --json headSha,conclusion,...` and `gh run list --commit b5e286b3...` (no runs).
- No scratch build or test run was done. P2-1's compile failure follows from the source of the pinned commit. P3-1 and P3-2 come from reading, and each names the exact test or mutation that would show it.
