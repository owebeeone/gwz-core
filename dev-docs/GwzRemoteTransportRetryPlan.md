# Connection retry and concurrency defaults

Status: **accepted as plan text at `ef29f8907875928b6e6891a2db12cbe3ca781fee`,
SHA-256 `08e198e00c5f6ff697dca6b71f8117ce8963afb91126a30af2b5ea94a6ac6619`,
after Consistency-3, Safety-3 and Surface-3 GO**. The operator subsequently
authorized implementation. This status annotation does not itself implement,
push, tag or publish anything.

Date: 2026-09-23. Safety-2 and Surface-2 reported GO on `b40b75c0…`;
[Consistency-1](GwzRemoteTransportRetryPlan-ReviewConsistency-1.md) retained
one blocking supersession gap. The current bounded correction is mapped in
[RemPlan-2](GwzRemoteTransportRetryPlan-RemPlan-2.md), following
[RemPlan-1](GwzRemoteTransportRetryPlan-RemPlan-1.md). All three axes verified the corrected text; their Review*-3 reports are
the acceptance evidence.

## 1. Outcome

A connection that fails before it is reusable is retried, with exponential
backoff, when the failure is in the closed retriable set below. The member
stays in progress across those attempts. The operation reports the failure
only after the attempt budget is spent, or immediately when the failure is
not retriable.

The same change raises two defaults, adds `--max-retries`, and makes the
pool use the operation's concurrency:

| Knob | Today | This plan |
|---|---|---|
| `--jobs` (`DEFAULT_JOBS`) | 50 | 100 |
| `--max-per-host` (`DEFAULT_MAX_PER_HOST`) | 8 | 32 |
| `--max-retries` | absent | 3 retries after the first attempt (4 attempts) |
| Pool `per_user_host` and `per_host` for the running operation | 8, and the flag does not change them | that operation's resolved `--max-per-host` |
| Pool `total` for the running operation | 256 | `max(256, resolved --jobs)` |
| Pool `max_requests` for the running operation | 1024 | `max(1024, resolved --jobs)` |
| `--ssh-timeout` stall default | 3 s | 9 s |
| Pool aggregate `connect_timeout_ms` when the stall is positive | 10 s | 30 s |
| Idle timeout | 60 s | 60 s |

100, 32, and 3 are defaults. A larger value that the flag accepts is the
value used. This plan adds no product ceiling and does not clamp a requested
value down to 32, 100, 3, 8, 1024, or 4096.

`--jobs` is a permit count inside the one `gwz` process. It is not a process
count. It is also the bound on live worker threads. `--max-per-host` is a
permit on how many of those workers may run members of one hostname at once.
It is not a thread spawn count. SSH sessions stay on the transport worker in
the same process.

## 2. Authorities

Stay in force, except the clauses named in §3:

- [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md)
- [GwzRemoteTransportPlan.md](GwzRemoteTransportPlan.md) Phase 6
- [GwzV110Plan.md](GwzV110Plan.md), accepted as text on
  `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62`
- [GwzRemoteTransportAlphaTimeoutPlan.md](GwzRemoteTransportAlphaTimeoutPlan.md)
- [GwzRemoteTransportPoolCapacity.md](GwzRemoteTransportPoolCapacity.md)
- [GwzRemoteTransportRequirements.md](GwzRemoteTransportRequirements.md) D2:
  one active exchange per physical connection. This plan does not add
  channels.

Evidence for the numbers, copied from the pool-capacity brief (the research
file is private): no-op `gwz fetch`, 32 small `github.com` repos, medians of
three runs. `gwz` 1.0.17 at `--max-per-host 32` was 3.1 s and GitHub accepted
the connections. The alpha at the same flag was 6.9 s because the pool stayed
at 8. `gwz` 1.0.17 at 16 members exited 1 in 2 of 5 runs with
`Timed out waiting on socket` at the 3 s stall; measured handshakes were
2.0–3.5 s.

## 3. Superseded clauses

This document supersedes only these sentences. Everything else in the named
documents stays.

1. [GwzV110Plan.md](GwzV110Plan.md) §2, the unsupported cell "A new timeout
   flag, or a changed 3 s stall / 10 s aggregate / 60 s idle". Replacement
   cell: "No new timeout flag. Stall default 9 s, aggregate default 30 s,
   idle 60 s. A positive `--ssh-timeout` does not change the aggregate.
   `--ssh-timeout 0` disables both network deadlines. `--max-retries` is a
   separate flag and is not a timeout." On acceptance of this plan, that cell
   is replaced with those words and the acceptance line there gains
   "Amended 2026-09-23 by `GwzRemoteTransportRetryPlan.md` for the stall and
   aggregate defaults only."
2. [GwzRemoteTransportPoolCapacity.md](GwzRemoteTransportPoolCapacity.md)
   Phase 1 goal "raising `--max-per-host` raises pool capacity; defaults stay
   as they are." Replacement: raising `--max-per-host` raises pool capacity,
   and the defaults become `--jobs` 100 and `--max-per-host` 32. That brief's
   Phase 2 (concurrent channels) is not superseded and is not a step here.
3. The pool-capacity brief's out-of-scope line "`--ssh-timeout` default of
   3 s in 1.0.17 vs a 2.0-3.5 s handshake." Replacement: the stall default
   moves to 9 s in Phase 2 of this plan. The handshake measurement stays
   historical.
4. [GwzV110Plan.md](GwzV110Plan.md) §5 bullet "Changing the 3-second stall,
   the 10-second aggregate, the 60-second idle default, or a frozen hard
   cap." Replacement: "Changing the 60-second idle default, or adding a
   frozen hard cap. The stall default is 9 seconds and the aggregate default
   is 30 seconds, per `GwzRemoteTransportRetryPlan.md`."
5. [GwzRemoteTransportAlphaTimeoutPlan.md](GwzRemoteTransportAlphaTimeoutPlan.md)
   §2 table cells "Default 3,000 ms" on the native-stall row and
   "Construction default 10,000 ms. Leave it there" on the aggregate row;
   the sentence "`--ssh-timeout` does not change the 10-second
   aggregate. There is no new flag."; and the §5 bullet "Changing the
   default 3-second `--ssh-timeout` or the default 10-second pool connect
   budget." Replacement: the stall default is 9,000 ms and the aggregate
   construction default is 30,000 ms. A positive `--ssh-timeout` does not
   change that 30-second aggregate. `--max-retries` is not a timeout flag.
   The two-clock rules otherwise stay, including zero disabling both network
   deadlines.
6. [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md) §10.1 sentence
   "`Config::io_timeout_ms` defaults to 3,000 ms for a standalone stream"
   as it sets the product stall default. Replacement: the stall default
   captured at startup is 9,000 ms. The rest of that sentence stays: an
   Open can only shorten it, and zero disables the network timeout.
7. [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md) §7.2 sentences
   "The design starting value is eight physical connections per user/host,
   with an aggregate endpoint ceiling of eight per host across users, ports
   and schemes." and "The endpoint ceiling is an additional bound across
   operations; a request cannot raise it."
   [GwzRemoteTransportPlan.md](GwzRemoteTransportPlan.md) Phase 2 bullets
   "Endpoint construction policy starts at eight physical connections per
   user/host and eight aggregate per host across users, ports and schemes."
   and "These ceilings bound combined physical use across operations;
   requests cannot raise them." and "Its default eight is a separate
   concurrent-work limit." Also superseded: "A lower operation limit neither
   resizes the endpoint pool nor evicts another operation's connections."
   Replacement: the starting values are 32 per
   user/host and 32 per host. At operation start, when no lease is
   non-idle, the operation installs its resolved `--max-per-host` and
   `--jobs`, which can raise those ceilings. While a lease is non-idle, a
   request cannot raise them; that operation is refused. A lower limit
   installs at the next idle operation start and does not evict another
   operation's non-idle connections.
8. [GwzRemoteTransportPoolCapacity.md](GwzRemoteTransportPoolCapacity.md)
   Phase 1 step 2 sentence "Keep the pool's validation bounds."
   Replacement: the upper bounds 4096 and 16384 on `per_user_host`,
   `per_host`, `total`, and `max_requests` are removed. The check that
   remains for those four fields is `>= 1`.

## 4. Retriable failures

A failure is retriable only when all of these are true:

- The connection is still in setup. The session has not been admitted as
  reusable. On the SSH path that is the `Connecting` state in
  `gwz-core/src/git/endpoint/ssh_setup.rs`, before `setup_is_reusable` accepts
  the result. A late success after a fired clock stays non-reusable, as the
  accepted timeout plan already requires. A success from an attempt
  generation that has already moved on does not admit that session and does
  not mark the key healthy.
- `--max-retries` for this operation is at least 1. Zero means the first
  failure is the member result.
- The code is one of: a setup `Timeout` whose origin is stall or aggregate,
  `Io`, `CarrierLost`, or `Unavailable` caused by
  `ErrorKind::ConnectionRefused`.

The classifier receives the timeout origin. `ErrorCode::Timeout` alone is
not enough. Stall is `--ssh-timeout` / `io_timeout_ms`. Aggregate is
`connect_timeout_ms`. The reason string stays `ssh setup timeout: stall` or
`ssh setup timeout: aggregate`.

These are not retriable, in setup or after it:

- `Authentication`, including a host-key mismatch (`PermissionDenied`) and a
  `gh` credential rejection
- `Trust`
- `Cancelled`
- `Protocol`, `InvalidRequest`, `RepositoryRefused`
- `Capacity` (the pool already queues)
- A timeout whose origin is interaction or allocation. An interactive prompt
  pauses both network clocks and does not start a backoff. If the interaction
  allowance itself expires, that failure is returned once.
- `Unavailable` from `NotFound` (missing agent socket or known_hosts) or
  `AddrNotAvailable`
- Any failure after the session is reusable, including a timeout while a
  fetch or push body is in progress. A push may already have been accepted.
  This plan does not retry that.

`--ssh-timeout 0` produces no stall and no aggregate, so it produces no
`Timeout` to retry. It does not disable retries. `Io`, `CarrierLost`, and
`ConnectionRefused` are still retried until `--max-retries` is spent. An
attempt that never returns is the hang that 0 already selects. The retry
loop advances only when the attempt returns.

Cancellation during a wait cancels every member queued on that key and does
not start another attempt. A wake with no remaining member does not open a
probe.

HTTPS uses the same closed set for the connect that happens before the first
request byte. An HTTP authentication failure is `Authentication` and is not
retried.

A non-retriable setup failure, and a retriable failure of attempt `R + 1`,
both put the key in **Closed** for the rest of this operation. Closed opens
no further setup. Every member already queued, and every member that
selects that key later in the same operation, is completed immediately with
the recorded failure. The next operation starts the key at Cold. That stops
`--jobs 1` from running a fresh budget for each member, and it stops an
authentication failure from becoming a series of handshakes.

## 5. Backoff

One machine per pool key (scheme, username, host, port). SSH and HTTPS on
the same host do not share a machine. The operation's `--max-per-host` groups
members by the hostname `git_host` parsed from the remote URL
(`gwz-core/src/git/git_host.rs`): lowercased, no port, SSH config not
applied. The retry machine is the finer pool key. Members that share a
hostname and then share a pool key share one machine.

Let `R` be the resolved `--max-retries`. The number of attempts is `R + 1`.
The default `R` is 3, so four attempts. The first attempt starts immediately.
After a retriable failure of attempt `n` where `n <= R`, the key waits, then
one new attempt starts. After attempt `R + 1` fails retriable, every member
queued on that key receives that last failure. There is no further attempt
and no wait after the last failure.

The wait after attempt `n` (`n` from 1 through `R`) is
`min(30s, 1s × 2^(n-1))` plus jitter in `0..250ms`. With `R = 3` the waits
are 1 s, 2 s, and 4 s. The 30 s cap bounds the formula once `R` is large
enough for the delay to reach it. The cap is the aggregate default from
Phase 2. It is not a separate flag.

Key states:

- **Cold.** No successful setup yet for this operation. Exactly one setup is
  in flight. Other members on that key wait. They do not open a connection.
- **Healthy.** A probe from the current generation succeeded and was
  admitted. Normal allocation applies, up to the operation's per-host cap.
- **Degraded.** A retriable setup failure has been counted for this
  generation. Exactly one setup is in flight, or the key is waiting for the
  next one. Setups that were already in flight when the key left Healthy
  belong to that generation. Their retriable failures do not increment the
  attempt count. Their success completes that member and does not mark the
  key Healthy and does not reset the counter.
- **Waiting.** The generation's one probe has failed retriable and attempts
  remain. No connection opens until the wake.
- **Closed.** The budget is spent, or a non-retriable setup failure was
  recorded. No connection opens until the next operation. A member that
  arrives later is finished with the recorded failure. It does not become
  Cold again inside this operation. After Healthy, a later exhaustion
  enters Closed the same way. The absence of a successful setup is not
  permission to start another Cold probe once Closed.

A success admits the session only when it belongs to the generation the key
is still running. Then the key becomes Healthy and the attempt counter
returns to zero. Queued members take normal leases. A completion for an
older generation is ignored for the key's state.

Each probe gets a fresh stall clock and a fresh aggregate clock. The failed
connection is discarded and is not returned to the pool. Tests inject the
clock and the jitter. They do not sleep the real waits.

With the network clocks enabled, the network-only bound for one key is
`(R + 1)` times the aggregate, plus the `R` waits, plus at most `R × 250 ms`
of jitter. At the defaults (`R = 3`, aggregate 30 s) that is 127.75 s. The
full wall-clock bound adds, on every attempt, the interaction allowance
(120 s) and the cleanup allowance (5 s): `(R + 1) × 125 s`. At the defaults
that adds 500 s, so the full bound is 627.75 s. `--ssh-timeout 0` has no
network-clock bound. The full bound is not a claim about that mode.

The classifier and the machine live in `gwz-core`, beside the endpoint that
already sees the setup failure. `gwz-transport`'s pool keeps granting leases.
It does not learn error codes. A retriable failure drops its lease before
the wait.

No new error code. The final failure keeps its code. Its display gains a
suffix `attempt N of M`, where `M` is `R + 1` and `N` is the attempt that
just failed. Intermediate attempts do not complete the member and do not
produce `Partial`. They are progress on the existing diagnostic row.
`--verbose` and `--json` keep their current gates. The attempt number is
not a new secret-bearing field.

`--max-retries` is carried on the in-process operation policy, beside
concurrency. The generated policy gains an optional `max_retries`. Absence
means 3, so an older writer keeps this default. That is not a protocol
version bump.

## 6. Defaults and the pool

Today `gwz-cli` help says `--jobs` defaults to 50 and `--max-per-host`
defaults to 8 (`gwz-cli/src/globalargs/parser.rs`). `DEFAULT_JOBS` is 50
(`gwz-core/src/operation/resolve_jobs.rs`). `DEFAULT_MAX_PER_HOST` is 8
(`gwz-core/src/operation/resolve_per_host.rs`). The operation already
resolves those into `par_map_per_host`. The pool does not.
`pool::Config::default` is `per_user_host: 8`, `per_host: 8`, `total: 256`,
`max_requests: 1024` (`gwz-transport/src/pool/mod.rs`). Allocation refuses a
new connection when either per-host count is at its cap
(`gwz-transport/src/pool/allocation.rs`). `request_until` returns
`Error::Capacity` when `requests.len() >= max_requests`
(`gwz-transport/src/pool/machine.rs`). GitHub SSH is one user on one host,
so `per_user_host` is the cap that binds. The candidate CLI factory builds
that default and only overrides `connect_timeout_ms`
(`gwz-core/src/git/gitbackend/transport_binding.rs`).

`par_map_per_host` today starts `min(per_host, members on that host)`
scoped threads per host group before any thread takes the `--jobs` permit
(`gwz-core/src/operation/par_map_per_host.rs`). S1.5 replaces that spawn
shape. The logical limits stay. Live worker threads are at most the
resolved `--jobs` (and at most the member count). Each worker pulls the
next member whose hostname still has a free `--max-per-host` permit.
Spawn is fallible. If a worker thread cannot be created, the operation
returns a typed error and does not panic. The requested `--max-per-host`
is not reduced to make the spawn succeed.

`--max-per-host` counts concurrent member operations for one hostname.
`git_host` takes that hostname from the remote URL, lowercased, with no
port and without applying SSH config. `git@github.com:a.git` and
`ssh://git@github.com/b.git` share a limit. `git@gh-work:a.git` and
`git@gh-personal:b.git` do not, even when SSH config points both names at
one server. A URL whose host cannot be parsed is bounded only by
`--jobs`. With one exchange per connection, the pool's connection cap for
that hostname is the same number. The help text uses "member operations"
for both lines.

Phase 1 sets `per_user_host` and `per_host` from the operation's resolved
`--max-per-host`, `total` to `max(256, resolved --jobs)`, and
`max_requests` to `max(1024, resolved --jobs)`. The resolved `usize` values
are the ones `resolve_jobs` and `resolve_per_host` already return to fetch,
push, and pull. This plan does not read the field
`generated::OperationPolicy` currently writes as `None` in
`protocol/convert.rs`. The model policy the handlers already resolve is
the source. `max_retries` is added; `max_connections_per_host` is not given
a second meaning.

Caps are installed when an operation starts, before it opens a connection.

- The candidate CLI has one operation. Its factory uses that operation's
  resolved jobs, per-host, and max-retries. There is no new process global
  beside the existing server-timeout global.
- `SshEndpointConfig::from_environment` does not choose the caps. It still
  supplies the home directory and the agent socket.
- If the pool has no non-idle lease, idle connections above the new `total`
  or the new per-host cap are closed, and the four fields are set to this
  operation's resolved values. A sequential later operation therefore gets
  its own caps. The first operation does not freeze them.
- If any lease is non-idle, the new operation is refused with a typed
  error. It does not run on the other operation's caps. The CLI does not
  start a second operation. The host path that already rejects a second
  client endpoint keeps that rejection.

`Config::validate` today rejects `per_user_host`, `per_host`, and `total`
outside `1..=4096`, and `max_requests` outside `1..=16384`. Phase 1 removes
those upper bounds. The check that remains is `>= 1`. The CLI only produces
a positive `i64` for `--jobs` and `--max-per-host`, and on the supported
64-bit targets that value fits in `usize`. A resolved value above 4096 or
16384 is stored unchanged. Zero still fails validation. It is not clamped.
The timeout fields keep their current bounds. `DEFAULT_MAX_LANES` (hook
lanes, also 8) is a different constant and stays 8.

`pool::Config::default` uses `per_user_host` 32 and `per_host` 32 so a
forgotten override is not secretly 8. `total` stays 256 because
`max(256, 100)` is 256. `max_requests` stays 1024 because
`max(1024, 100)` is 1024. An explicit `--max-per-host 4` or `--jobs 1`
still overrides at operation start. `--jobs 0` and `--max-per-host 0` are
rejected by the existing positive parser. The smallest accepted value is 1.
`--jobs 1` runs one member at a time.

## 7. Clocks

Phase 2 triples the two network defaults and keeps them independent. It
does not add a timeout flag. `--max-retries` is not a timeout flag.

- Stall default: `--ssh-timeout` omitted means 9. `gwz-cli/src/lib.rs`
  `unwrap_or(3)` becomes `unwrap_or(9)`. Help and long help are the §8
  text. `0` still disables the stall and the aggregate. It does not
  disable `--max-retries`.
- Aggregate default: `pool::Config::default` `connect_timeout_ms` becomes
  `30_000`. `apply_native_timeout` in `gwz-core/src/transport_host/mod.rs`
  keeps its rule: a positive native timeout leaves the aggregate alone, and
  `0` sets the aggregate to `0`. The test
  `positive_native_timeout_keeps_the_pool_aggregate` changes its expected
  aggregate from `10_000` to `30_000` and still asserts that `3_000` and
  `15_000` do not change it.
- The candidate factory in `transport_binding.rs` stops assigning
  `connect_timeout_ms: timeout`. It uses `apply_native_timeout` (or the same
  rule). Today that assignment makes both clocks equal to the stall, which
  contradicts the host path.
- Idle stays `60_000`. Cleanup stays `5_000`. Interaction stays `120_000`.

## 8. Help text

`--jobs` help: `Global ceiling on concurrent member operations (default 100)`.

`--jobs` long help: `Global ceiling on the total number of member repositories processed concurrently across all hosts. These are concurrent operations in this process, not extra processes. Defaults to 100. The smallest value is 1. 0 is rejected. Per-host concurrency is bounded separately by --max-per-host. Values above 100 are accepted.`

`--max-per-host` help: `Max concurrent member operations to any one hostname (default 32)`.

`--max-per-host` long help: `Maximum concurrent member operations against one remote hostname. The hostname is the host in the remote URL, lowercased, before SSH config is applied. Two URLs share this limit when they contain the same host. A URL whose host cannot be parsed is bounded only by --jobs. Defaults to 32. The smallest value is 1. 0 is rejected. Values above 32 are accepted. Each of these operations uses one connection.`

`--ssh-timeout` help: `Per-attempt stall limit for setup and for a body read; stalled setup is retried (0 = no timeout, default 9)`.

`--ssh-timeout` long help: `Seconds without progress before that read fails. The clock applies to setup, and it applies to a stalled read during a fetch, push, or pull body. A body stall aborts that repository and is not retried. A setup stall fails one attempt, and that attempt is retried. SSH and HTTPS use this same stall clock and the same 30 second setup budget. libssh2 has no timeout by default, so a missing ssh-agent identity or an unreachable host would otherwise hang forever. 0 disables this stall clock and the 30 second setup budget on both SSH and HTTPS. It does not disable retries. Defaults to 9. Retries are controlled by --max-retries (default 3 extra attempts). The wait after a failed setup attempt starts at 1 second and doubles, and does not grow past 30 seconds, plus up to 0.25 seconds of jitter. That wait is not this flag. At these defaults, a setup that makes no progress is reported after at most 4 times 9 seconds plus 1, 2, and 4 seconds of waits and under 1 second of jitter, about 44 seconds. A setup that keeps making progress but never finishes can use the 30 second budget on each attempt, about 128 seconds. A host-key prompt can add up to 120 seconds on each attempt.`

`--max-retries` help: `Extra setup attempts after the first failure (0 = do not retry, default 3)`.

`--max-retries` long help: `How many times to retry a connection that failed during setup, after the first attempt. Setup is before the session is reusable: for SSH, before it is authenticated; for HTTPS, before the first request byte. A stall or a reset during a fetch, push, or pull body is not retried. Defaults to 3, so four attempts. 0 reports the first failure and does not retry. Values above 3 are accepted. This is the only retry control. --ssh-timeout 0 does not turn retries off. A repository that succeeds on a later attempt counts as having answered. After each failed setup attempt the next one waits 1 second, then 2, then 4, doubling, and the wait does not grow past 30 seconds, plus up to 0.25 seconds of jitter. --ssh-timeout sets only the per-attempt stall. 0 on that flag clears the stall and the 30 second setup budget. It does not clear these waits.`

Append this sentence to the long help of `fetch`, `push`, and `pull`, so
`gwz fetch --help`, `gwz push --help`, and `gwz pull --help` show it:
`A connection that fails during setup is retried before that repository counts as failed. See --max-retries and --ssh-timeout. A repository that succeeds on a later attempt counts as having answered.`

The flag stays a global option, in the same group as `--jobs` and
`--ssh-timeout`. It is not a new command.

## 9. Phases

Steps are serialized. Phase 2 edits the same factory and the same
`Config::default` as Phase 1. Phase 3's wait cap is the Phase 2 aggregate.

### Phase 1 — Defaults reach the pool (milestone: an omitted flag is 100 and 32, the pool allows that, and thread count follows `--jobs`)

- **S1.1** Failing tests: `resolve_jobs(None) == 100`, `resolve_per_host(None) == 32`, an explicit `1` is `1`, and the help strings in §8 are what `--help` renders for those two flags. Update the known assertion site `gwz-core/src/transport_host/driver_tests.rs` that sizes rows from `resolve_jobs(None)`. Hook `DEFAULT_MAX_LANES` stays 8. *(< 200 LOC)*
- **S1.2** Set the two constants and the `--jobs` / `--max-per-host` help in §8. Leave `--ssh-timeout` and `--max-retries` help to later phases. *(< 80 LOC)*
- **S1.3** Failing test: an operation resolved at the new defaults builds a pool whose `per_user_host` and `per_host` are 32, whose `total` is 256, and whose `max_requests` is 1024; the 33rd concurrent lease to that user-host waits; `--max-per-host 4` builds 4; `--jobs 400` builds `total` 400; `--jobs 1500` builds `total` 1500 and `max_requests` 1500, and 1500 checkouts are queued or leased with no `Capacity` from `max_requests`; `per_host` 5000 is stored as 5000; `per_host` 0 fails validation. *(< 250 LOC)*
- **S1.4** Install the resolved caps at operation start, as §6 and §3.7–§3.8 describe, on the candidate CLI and on the host pool. Close idle connections that exceed the new caps before accepting the operation. Refuse the operation when a non-idle lease exists. Remove the `4096` and `16384` upper bounds for the four size fields only. `Config::default` uses 32 and 32. No new global, no clamp, no first-operation freeze. *(< 350 LOC)*
- **S1.5** Replace the eager per-host `scope.spawn` in `par_map_per_host` with at most `--jobs` workers, fallible spawn, and a per-hostname permit of `--max-per-host`. A test with `--jobs 1`, a huge `--max-per-host`, and many members on one host keeps one live worker, does not panic, preserves result order, and cancels queued work when the operation is cancelled. *(< 400 LOC)*

### Phase 2 — Clock defaults (milestone: stall 9 s, aggregate 30 s, a positive stall does not move the aggregate)

- **S2.1** Update tests that pin the old pair, including `positive_native_timeout_keeps_the_pool_aggregate` and the `network_deadlines(3_000, 10_000, …)` fixtures in `transport_host/session/driver.rs`. A positive stall still leaves the aggregate at its default. `0` still zeroes both. *(< 150 LOC)*
- **S2.2** `unwrap_or(9)`, `connect_timeout_ms: 30_000`, the `--ssh-timeout` help in §8, and the candidate factory calls `apply_native_timeout` instead of copying the stall into `connect_timeout_ms`. A test asserts the rendered `--ssh-timeout` help matches §8. *(< 150 LOC)*
- **S2.3** After this plan is accepted, apply the replacements in §3.1 and §3.4 through §3.6 in the named documents. Do not rewrite the rest of those documents. *(< 80 LOC)*

### Phase 3 — Retry the connection (milestone: one stall is not the member result)

- **S3.1** Failing tests, clock injected. Default `R = 3`: a stall on attempt 1 does not finish the member; attempt 2 starts at the recorded 1 s wake; attempt 4's stall finishes the member as `Timeout` with suffix `attempt 4 of 4`. `--max-retries 0` finishes on the first stall. `--max-retries 1` allows two attempts. An authentication failure does not wait and does not open a second handshake for the queued members. An interaction timeout and an allocation timeout are returned once. Cancel during the wait opens no probe. Thirty-two cold members on one dead key open one handshake at a time and exactly four handshakes in total. With `--jobs 1`, those 32 members still cause exactly four handshakes, and every member finishes with that same failure, including when the key was Healthy earlier in the operation and then exhausted. An authentication failure under `--jobs 1` is one handshake, and every later member finishes with that authentication failure. A success from an older generation does not mark the key healthy. A timeout after the session is reusable is returned once. `ConnectionRefused` is retried. `NotFound` is not. *(< 400 LOC)*
- **S3.2** The closed classifier in §4, including the timeout origin. One function, used by SSH and HTTPS. *(< 150 LOC)*
- **S3.3** The per-key machine in §5, driven by the endpoint that owns the setup failure. Drop the lease before the wait. *(< 450 LOC)*
- **S3.4** The `--max-retries` flag, optional policy field, final display suffix, progress for intermediate attempts, and the fetch, push, and pull sentence in §8. A test asserts the rendered `--max-retries` help and those three long-help sentences match §8. No new error code and no protocol version bump. *(< 220 LOC)*
- **S3.5** HTTPS connect-before-request uses §4 and §5. An authentication failure on that path is not retried. Stall, aggregate, interaction, and allocation are distinguished on both schemes. *(< 200 LOC)*

## 10. Out of scope

- Concurrent channels on one SSH connection (pool-capacity brief Phase 2).
- A learned cap when a server refuses extra connections. A timeout must not
  lower `per_host`. Refusal detection is a later design.
- Retrying a fetch or push body after the session is reusable.
- A new error code, or a protocol version bump.
- Idle, cleanup, and interaction deadlines, other than counting them in the
  published bound.
- A live GitHub measurement. The pool-capacity brief still owns that table.
- Python bindings, crate publish, tags, and the v1.1.0 tag.
- `DEFAULT_MAX_LANES`.

## 11. Verification

Each touched crate: `cargo fmt --all -- --check`, `cargo test`, and
`cargo clippy --all-targets --all-features -- -D warnings`, on rustc 1.95.
`gwz-core`'s gate is `python3.13 run_tests.py`. The candidate cfg
`gwz_transport_candidate` is how the alpha builds the factory in S1.4 and
S2.2; the default cfg must still build, and its native transport must keep
today's libgit2 timeout call.

Help checks are the pins in S1.1 (`--jobs`, `--max-per-host`), S2.2
(`--ssh-timeout`), and S3.4 (`--max-retries`, and the fetch, push, and pull
sentences). Each compares the rendered help to §8. Retry tests use the injected clock. No step starts a
live fetch.

## 12. Acceptance

This text is not implementation authority until Consistency, Safety, and
Surface report GO on the same hash. GO accepts the plan text only. It does
not authorize the code, an alpha rebuild, a commit, or a tag.

## Candidate Python concurrency amendment (2026-09-24; pending review)

The [Python concurrency correction](../../dev-docs/GwzPyTransportConcurrencyDesign-1.md) proposes a narrow replacement for §3 item 7, §6 and S1.4's blanket refusal when any lease is non-idle. For a shared Python endpoint generation, an operation whose **resolved four-field physical capacity exactly equals the installed capacity** may join without reinstalling or resizing the pool, even while another operation has a non-idle lease. An operation needing different physical capacity is still refused before Git and credential effects until all conflicting operations and leases retire. Every initial and later installation is serialized and published atomically with its shared reservation authority. This candidate text is not implementation authority until the corrected design receives review GO and the core requirements/design are updated.
