# Idle connections the server closed: notice them, and replace a dead lease

Status: **DRAFT, 2026-10-07, not reviewed.** The companion task the operator
decided as OQ17 of `GwzTransportAdaptiveConcurrencyDesign.md` (revision 8):
"the hosts notice EOF on idle sockets and report `idle_closed`, and a dead
leased connection is replaced rather than failing its member". It closes that
design's F14 and reduces its R7b cost. It changes no wire message, no retry
classification and no limit.

Citations are to gwz-core `db0f8447` and gwz-transport `ff6083b`.

## 1. Facts

- **The pool already has the callback.** `Pool::idle_closed`
  (`gwz-transport/src/pool/lifecycle.rs:200-214`, `PoolDriver::idle_closed`
  at `pool/asynchronous.rs:391`) removes an Idle or already-Closing entry and
  wakes capacity waiters. Its contract (`gwz-transport/README.md:190-196`):
  the host disposes the unusable resource first, then reports; if a checkout
  won the race, `WrongState` leaves that lease untouched, and the lease's own
  exchange meets the I/O failure, discards the lease and acknowledges cleanup.
- **No gwz-core host calls it.** `PoolHost::step_reported`
  (`src/git/endpoint/ssh_pool.rs:177-240`) does nothing for a `Ready` entry.
  Neither resource reads an idle socket for the host.
- **SSH.** An idle `NativeResource` is `State::Idle(Authenticated)`
  (`ssh_setup.rs:158-166`); its socket is read only by libssh2, during an
  exchange. The worker parks between passes, at most 1 s when idle and 1 ms
  otherwise; "The host has no socket readiness API yet"
  (`ssh_worker/runner.rs:200-212`).
- **SSH replies before the channel is open.** `attach` (`ssh_worker.rs:212-282`)
  creates the pump and replies `Opened` at once; the channel is opened later by
  the pump's `tick` (`SshPump::tick_inner`, `ssh_pump.rs:458-470`). A dead
  session therefore fails the open's stream after the client has the reply.
- **HTTPS.** Hyper's connection task reads the idle socket itself and ends on
  EOF, reset or unexpected bytes. `HttpResource::reusable` turns false when that
  task has finished (`https_connection.rs:375-382`), but the pool still holds the
  entry Idle and leases it. `HttpsPool::checkout_scoped` then refuses the dead
  lease as `ErrorCode::Io` with `Phase::Other` (`https_pool.rs:186-188`), which
  the retry machine Returns (`transport_host/https_endpoint/retry.rs:109-113`):
  the member fails without having reached the server. If the task ends after the
  lease, the discovery GET fails the same way (`https_worker/prepare.rs:306-314`).
- **Hyper says whether a request was started.** `SendRequest::try_send_request`
  returns the request inside its `TrySendError` exactly when "the message was
  never even started" (hyper 1.11.1, `proto/h1/dispatch.rs:731-741`, and the
  queued envelope's drop, `client/dispatch.rs:217-227`); otherwise the message
  is `None`.
- **The pool cannot be asked for a fresh connection.** `schedule`
  (`pool/allocation.rs:16-45`) leases any compatible idle entry before it
  creates one; `Request` has no field to refuse reuse (`pool/mod.rs:178-187`).
- **The HTTPS host is stepped by a 2 ms timer with a no-op waker**
  (`https_pool.rs:57-71`). Wakers registered by its resources wake nothing today.

## 2. Goals and boundaries

1. Each host notices EOF, reset, or unsolicited bytes on an idle connection when
   the operating system reports it, through readiness: no added polling loop,
   no added sleep, no global or thread-local. It disposes of the connection and
   reports `idle_closed`.
2. A leased connection that fails **before any request byte of its exchange is
   written** is discarded, and the member's open is retried **once**, on a
   **fresh** connection, charged no attempt by the key's retry machine.
3. Never retried: a POST whose body may have been read from Git; a push or any
   exchange after a Git byte was written; a fresh connection's failure; a second
   failure of the same open. Those keep today's handling. "The same open" is the
   whole open, across its attempts: a retry already used by an earlier attempt of
   the open, whose connection a 401 challenge carries into the next, is used.

## 3. The pool: one addition

`gwz_transport::pool::Request` gains `pub fresh: bool`, false from
`Request::new`. A fresh request is never given an idle entry; it is served by a
new connection, or by evicting an idle one, exactly as a request with no
compatible idle entry is now. Nothing else in the pool changes; `idle_closed`
keeps its contract. This is the only gwz-transport change (README "Requests").

## 4. The host: `PoolHost` and `Resource`

`Resource` gains one method with a default:

```rust
/// Ready when the peer has closed, reset, or sent unsolicited bytes on this
/// connection while it is idle; Pending registers `cx`. Never consumes bytes.
/// Pending whenever the resource is not idle (an exchange owns its I/O).
fn poll_idle_lost(&mut self, _cx: &mut Context<'_>) -> Poll<()> { Poll::Pending }
```

`PoolHost::step_reported` polls it for each `Ready` entry. On `Ready`:

1. The entry becomes `Disposing { idle_lost: true, .. }` and is disposed as
   usual (an idle SSH or HTTPS resource disposes within the same pass).
2. After disposal, the host calls `idle_closed(id)`.
   - `Ok`: the entry is removed (destruction still precedes the
     acknowledgement).
   - `WrongState`: a checkout won. The entry stays as a disposed tombstone,
     `Phase::Lost`. `resource()` and `allocation_reused()` return `WrongState`
     for it, which is how the lease's open learns it is dead (§6). `release` of
     a lease on a `Lost` entry releases it `Discarded` whatever was asked. The
     pool's `Close` or `Abort` for it re-enters `Disposing` (disposal is
     idempotent on both resources) and ends in `closed()`.
   - any other error ends the step, like the host's other driver calls.

`ReservedResource` forwards the method.

## 5. Readiness per protocol

### 5.1 SSH: an idle watch owned by the connector

- **`IdleReactor`** (new, `src/git/endpoint/idle_watch.rs`, Unix-only like
  `ssh_setup`, which is its only user): one thread
  running a current-thread tokio runtime with only the I/O driver enabled,
  blocked on a stop signal. It is created by `SetupConnector` on its first
  connect and held through an `Arc` by the connector and by each resource it
  made, so it outlives every registered socket. Dropping the last `Arc` stops
  and joins the thread. Not a global: each SSH endpoint's connector has its own.
- **`IdleSocket`**: a duplicate of the session's socket (`try_clone`), set
  nonblocking and registered with that reactor (`tokio::net::TcpStream::from_std`
  inside the reactor's handle). `poll_lost(cx)` is `poll_peek` of one byte:
  `Pending` registers `cx` with the reactor; `Ready(Ok(0))` (EOF),
  `Ready(Ok(n > 0))` (unsolicited bytes: libssh2 is not reading, and an idle
  session cannot answer them) and `Ready(Err(_))` (reset) are all lost. Peek
  never consumes.
- **`NativeResource`** holds an `IdleSocket` only while `State::Idle`, created
  on its first `poll_idle_lost` in that state and dropped by `start_exchange`
  before the session moves into the channel. After `reclaim` it is created again
  on the next poll. A registration that fails is lost (the connection is not
  kept idle unwatched); a reactor that fails to start fails the connect.
- The worker's waker is its `ThreadWake`, so a loss unparks the worker at once,
  whatever its park timeout.

### 5.2 HTTPS: hyper's own read

Hyper already reads the idle socket. The connection task's exit is made
observable: `connect` gives it the sender of a `tokio::sync::oneshot`, dropped
when the task ends, and `HttpResource` keeps the receiver. `poll_idle_lost`
polls the receiver (registering `cx`) and is `Ready` when the task has ended
**and** the resource is idle (`reusable` flag set, connection present). While
leased, the exchange meets the failure itself; the ended task is recorded so
that the next idle poll reports it.

The supervisor still steps every 2 ms with a no-op waker, so HTTPS notice is
bounded by that tick, not by readiness. Making the supervisor waker-driven is
outside this task (§9).

## 6. Replacing a dead lease

### 6.1 HTTPS

- **(a) Dead at lease, every service.** In `checkout_scoped`, a lease of a
  reused connection whose resource is `Lost` or not `reusable()` is released
  `Discarded`, and the same request is checked out once more with
  `fresh = true`. Nothing has been sent and no Git body has been read: the
  POST's body is only read after `prepare` returns.
- **(b) Discovery GET not started.** In `run_attempt`, the advertisement GET is
  sent with `try_send_request`. When it fails with the request returned and the
  lease was reused (from the pool or carried after a challenge), the lease is
  finished `Discarded` and the loop starts over at the same hop with the next
  checkout fresh, keeping the attempt's slot, dependency and credential; the
  credential is not reported as offered. When the request is not returned, the
  failure is today's, also for a GET.
- The retry never reaches `RetryMachines::settle`: no attempt is charged, no
  strike recorded. A fresh connect at hop 0 is the open's setup, so its success
  or failure is the `FirstConnect` the key learns, as for any first connect.
- Once only, per open: the retry is fresh, and only a reused lease qualifies.
  The open's one retry is recorded on the lease it produces (`HttpLease::retried`),
  which a 401 carry keeps into the next attempt, and in a `retried` flag of the
  attempt. An open that has retried fails with `Io` where a second dead lease
  (at lease, or a GET not started) would have been retried again: a dead lease
  followed by a challenge carry is one retry, not two (P3-3 of
  `GwzTransportIdleLossDesign-ReviewState.md`).

### 6.2 SSH

- **The open's reply waits for the channel.** `attach` keeps the
  `EndpointAttachment` and `Opened` with the active exchange and sends them in
  the pass in which the pump reports its channel open (`SshPump::opened`, after
  `exec`). Git cannot send before it has the reply, so before the reply the pump
  has written no Git byte. The server speaks first in both services, so the
  client loses no time it could have used.
- **Dead at lease**: `attach` finds the lease's entry `Lost`
  (`allocation_reused` is `WrongState`).
- **Channel open failed**: the pump fails before `opened` (`channel_session` or
  `exec`; the session is dead).
- In both cases, when the lease was reused, the lease is released `Discarded`
  and the request goes back to the pending list with the same pool request,
  `fresh = true`. Otherwise the request completes with the failure (a reset,
  `ErrorCode::Io`, `Phase::Other`, as the stream failure reports today).
- An exchange that ends before the reply without a pump failure (its stream's
  I/O deadline) completes the open with `TimedOut`. A held open that is
  cancelled is released `Discarded` and completes as `Cancelled`.
- **The deadline over the held wait and the retry** is the open's attempt
  deadline: allocation plus connect plus interaction
  (`placement_endpoint/admission.rs::deadline_from_open`, also the worker's
  `request.deadline`). It bounds the wait for the channel and the fresh retry's
  new checkout. At that deadline the placement abandons the open as `Timeout`
  (`completion.rs::finish_opens`) and its held session, possibly a healthy one,
  is discarded. Before this change the reply went out at once, so that deadline
  could not fire after a lease. The exchange's stream I/O deadline bounds the
  wait for the channel too, from inside (P3-5).
- **Accepted limitation (P3-4).** A fresh retry is served by the pool after
  every other compatible waiter: a released connection goes to the earliest
  compatible non-fresh waiter (`gwz-transport:src/pool/allocation.rs::schedule`'s
  reuse loop skips a fresh request), and a fresh request gets a slot only from
  the creation loop (room) or the eviction loop (an idle entry nobody else took).
  Under sustained compatible demand on a busy key the retry waits until no such
  waiter remains, and is bounded by its allocation deadline, where it fails with
  `AllocationTimeout` as a request on a saturated key does. Before this change
  the member failed outright, so it is not a regression. Not changed here: the
  alternative, letting an earlier fresh waiter claim an idle entry ahead of later
  non-fresh waiters, is a pool scheduling rule and belongs to a change of
  gwz-transport that has a contention test of its own.

## 7. What the adaptive design gains

F14 is closed: a dead idle connection no longer fails its member, and is no
longer counted Connected once noticed. R7b's cost remains only for losses not
yet noticed (HTTPS within its 2 ms tick; SSH none once the OS has reported the
FIN or reset). §4.4 R7b and §12 F14 of that design should cite this note when
it is next revised; this note does not edit it.

## 8. Tests, written first

Each was written before its implementation and seen failing first (by a failed
assertion, or for a new function by not compiling), except where noted.

- gwz-transport `tests/pool_fresh.rs`: a new request is not fresh; a plain
  request reuses the idle connection; a fresh one opens a new connection beside
  it; a fresh one at capacity evicts it.
- `idle_watch/tests.rs`, on a loopback pair: a quiet peer is Pending and wakes
  nothing; the peer closing, resetting (`SO_LINGER` 0) or writing wakes the
  registered waker from the reactor, with no poll in between, and is lost; the
  written byte is still unread afterwards; a loss before the watch starts is
  reported; each watch keeps its reactor alive.
- `ssh_tests/pool_host.rs` (fake resource): idle loss is disposed, then
  reported, and frees the slot; a checkout that won keeps a `Lost` tombstone
  whose `resource()` and `allocation_reused()` are `WrongState`, whose release
  discards, and whose `Close` ends in `closed()` without a second disposal.
  `shared_reservation.rs`: the reservation forwards the loss.
- `ssh_tests/idle_loss.rs`, the sshd fixture behind `cut_proxy.rs`, a loopback
  proxy that ends connections from the server's side: the server closing an
  idle session frees the slot with no open in progress; a reused session dying
  at its channel open is replaced, and the open succeeds on a fresh connection
  (`reused` false, credential offered); a fresh session dying the same way fails
  the open, with no second connection (this one passed before the change too;
  it guards against retrying a fresh connection); an open cancelled while its
  reused session's channel stalls (the proxy stops forwarding) is `Cancelled`.
- `https_worker_tests/pool.rs`: the server closing an idle connection frees its
  slot with no open, and the next open is on a fresh connection. This replaces
  `idle_peer_close_is_a_typed_failure_without_a_hidden_get_retry`, which
  asserted today's F14 failure (`Io`, one connection). Its other intent is
  kept by `a_get_written_before_the_server_closes_is_not_retried` (the proxy
  closes as the GET arrives: `Io`, one connection; passed before too).
  `send_request` classifies a request on an already closed connection as not
  started, and one the peer received as sent, against hyper itself.
- `https_pool/idle_tests.rs`: the dead-lease race forced (the pool leases the
  idle connection, then the proxy cuts it, then the host finds it `Lost`):
  `adopt` reports it dead and the next checkout is fresh. Through
  `checkout_scoped`, with two idle connections of which the older is dead and
  its disposal pending (something holds it): the retry is a new connection (a
  third), is marked `retried`, and the other idle connection stays idle; with
  `may_retry` false the dead lease fails with `Io` and opens nothing. Removing
  `request.fresh = true` fails the first (P3-2).
- `ssh_tests/idle_loss.rs`, two idle sessions: both die at their channel open;
  the open leases one, and its retry is a third session while the other stays
  idle. Removing `policy.fresh = true` fails it (P3-2).
- `ssh_tests/pool_host.rs`: a lease taken while a lost connection's disposal is
  still pending (`Disposing { idle_lost: true }`) is known lost (`lost()`), is
  never reusable, and its release is a discard (P3-1).
- `https_worker_tests/pool.rs`: a carried lease found dead is retried once on a
  new connection; when the open has retried already it fails with `Io`
  and opens none (P3-3).
- Not forced end to end: §6.1 (b), a GET found not started on a lease that was
  alive when adopted. The window is between `adopt` and Hyper's write, which
  no fixture opens on demand; the classification and the loop are covered by
  the tests above and by reading.
- With HTTPS `poll_idle_lost` disabled, the HTTPS idle and dead-lease tests
  fail; restored, they pass.

## 9. Not in this task (for the operator)

- The HTTPS supervisor's 2 ms tick with a no-op waker (`https_pool.rs:57-71`)
  and the SSH worker's bounded park (`runner.rs:200-212`) are existing polling
  loops. This task adds wakers they could use; replacing the loops is its own
  change.
- **The keep-alive race after the request is written** (the server closes as
  the GET arrives) is not retried, by the boundary in §2. A discovery GET is
  idempotent, so it could be: an operator decision.
- `Endpoint::build` keeps its pre-existing `static NEXT_WORKER` counter
  (`ssh_worker/endpoint.rs`); this task adds no global.
- The SSH setup path (`ssh_setup`), and so the idle watch, is compiled on
  Unix only in this build (`endpoint/mod.rs`). When Windows gains it, the
  watch's duplicated socket under tokio's Windows driver needs its own run.

## 10. Decisions (operator, 2026-10-07)

1. Replacing `idle_peer_close_is_a_typed_failure_without_a_hidden_get_retry`:
   accepted; it asserted the behaviour OQ17 changes.
2. §6.1 (b) tested in parts (Hyper's classification, and the forced
   dead-at-lease race): accepted.
3. No retry once a GET is written: the "before any request byte" rule stands.
4. The HTTPS 2 ms step and the SSH bounded park stay; a separate HTTPS
   fixed-cost change replaces the 2 ms polls.
5. The HTTPS tests live in the approved `https_worker_tests/pool.rs` leaf:
   accepted.
6. The SSH idle watch is Unix-only, matching today's SSH setup path:
   accepted.
