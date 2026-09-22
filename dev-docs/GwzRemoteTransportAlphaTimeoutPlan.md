# Alpha SSH setup timeout plan

Status: **accepted at plan SHA-256 `cfdf028fb18557960da18a4682cb10f3e9e638ff197c4784efb76ac9a526984b` after [Consistency-1](GwzRemoteTransportAlphaTimeoutPlan-ReviewConsistency-1.md) and [Safety-1](GwzRemoteTransportAlphaTimeoutPlan-ReviewSafety-1.md) reported GO; this accepts the plan text only**. The status sentence was added after that GO. Round-1 NO-GO on `773639e1…` was remediated once. Implements the
correction named in
[GwzRemoteTransportAlphaTimeout.md](GwzRemoteTransportAlphaTimeout.md).
Round-1 reports:
[Consistency](GwzRemoteTransportAlphaTimeoutPlan-ReviewConsistency.md),
[Safety](GwzRemoteTransportAlphaTimeoutPlan-ReviewSafety.md). Remediation:
[RemPlan](GwzRemoteTransportAlphaTimeoutPlan-RemPlan.md).
Planning only; this document does not change timeouts, rebuild the alpha, or
resume the paused release work in
[GwzRemoteTransportReleaseReadiness.md](../../dev-docs/GwzRemoteTransportReleaseReadiness.md).

Authorities: [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md) §10
for timeout domains, §10.1 for the post-open active-I/O clock, and §10.2 for
disabled and maximum native values. The diagnosis is the observed failure.
Stable libgit2 at the pinned tree is the native semantics to preserve:
`GIT_OPT_SET_SERVER_TIMEOUT` bounds each blocking socket poll and
`libssh2_session_set_timeout`, and `GIT_OPT_SET_SERVER_CONNECT_TIMEOUT` bounds
one TCP connect. Neither value is a cumulative budget for DNS, TCP, handshake,
and agent authentication together.

## 1. Outcome

A default `--ssh-timeout` (3 seconds) allows an SSH setup that keeps completing
native calls to finish, including agent authentication after credentials have
been offered. A native call that makes no progress still expires at that
setting. Cancellation and physical disposal stay bounded. A result that arrives
after expiry or cancellation is not pooled. Ordinary output names a setup
timeout as a setup failure, and says whether the stall allowance or the
aggregate expired.

Raising `--ssh-timeout`, including the 15-second runs already recorded, is a
workaround. Widening only the cumulative connect budget from 3 seconds to 10
seconds is the same class of change. Neither closes this plan.

## 2. The two clocks

The alpha currently has one number for both clocks. `transport_timeout_ms()`
is the stalled-native-I/O setting. `SshEndpointConfig::from_environment`
(`gwz-core/src/transport_host/mod.rs`) writes it into both
`pool.connect_timeout_ms` and `io_timeout_ms`. Session Open
(`gwz-core/src/transport_host/session/driver.rs`) then sets both
`Deadlines.connect_ms` and `Deadlines.io_ms` from `io_timeout_ms`. The pool
starts one `ConnectClock::Network` deadline, and the setup `Job` spends that
instant across every stage. `Control::quantum` only returns whatever remains
of that same instant.

| Clock | Value | What it bounds |
|---|---|---|
| Native stall | `--ssh-timeout` / `io_timeout_ms`. Default 3,000 ms. Zero disables it | One blocking native attempt. The attempt completes when the socket is ready or the native call returns a finished result. A poll or sleep slice with no readiness does not complete it and does not reset the allowance |
| Aggregate connect | Pool `connect_timeout_ms`. Construction default 10,000 ms. Leave it there | The whole setup attempt, as the outer bound for cancellation and physical disposal |

A completed attempt starts a fresh stall allowance. DNS returning addresses,
TCP becoming connected, `handshake()` returning success, and an agent RPC
returning identities or a signature are completions. So is socket readiness
between handshake or agent `EAGAIN` attempts. `check`, `quantum`, and an
`EAGAIN` sleep are not completions.

The pool clock does not reset when a setup stage completes. §10.1 byte-progress
reset stays on the stream after Open. Setup does not grow a second copy of that
clock.

User interaction spends neither clock. Design §10 excludes it from network
timeout accounting, and the pool already pauses the aggregate in
`begin_interaction` / `end_interaction`. The stall allowance pauses with it:
remaining stall is preserved and resumes on `end_interaction`.

`--ssh-timeout 0` disables the stall allowance and the aggregate network
deadline together, matching stable `configure_server_timeout_ms(0)`. Cleanup,
cancellation, and shutdown disposal stay on their own clocks (§10.2). A positive
`--ssh-timeout` does not change the 10-second aggregate. There is no new flag.

Stall expiry and aggregate expiry are both setup-stage timeouts. Each carries
a distinct reason, `stall` or `aggregate`, visible to the caller. A later
failure record uses that reason. It does not infer which clock fired.

If a later cold fetch still fails because a progressing setup exceeds 10
seconds, and the retained reason is `aggregate`, the follow-up is a separately
qualified aggregate constant. It is not a larger `--ssh-timeout`. The recorded
15-second runs only show that the coupled clock sometimes needed more than 3
seconds and finished within 15. They do not choose the aggregate.

## 3. Phases

Each phase is a shippable increment. Each step is one goal with an aspirational
**< 500 LOC** budget, tests included. The dependency sketch in §4 is the
ordering. Steps that do not share a file can proceed independently once their
phase dependency is met.

### Phase 1: design states the split (milestone: §10 says the native timeout is a per-call stall, and the pool connect budget is a separate outer bound that is not initialized from it)

- **S1.1: amend design §10** *(GwzRemoteTransportDesign.md §10 and §10.2; ~120
  lines)*. Replace only the sentences listed here. No schema, tag, or message
  is added.

  Superseded, quoted from the design at gwz-core
  `7bd9381f0593f759333c7d8d070f4962d4092df7`:

  - §10 connect/auth cell: `Socket setup and protocol progress; preserve
    configured native timeout semantics`. Replacement: socket setup and
    protocol progress preserve the configured native timeout as the per-attempt
    stall in §2 of this plan. `Deadlines.connect_ms` and pool
    `connect_timeout_ms` are a separate aggregate. Endpoint construction must
    not copy `io_timeout_ms` into them.
  - §10.2 opening identification: `The pre-freeze implementation contact found
    that native GWZ accepts --ssh-timeout 0 and core
    configure_server_timeout_ms(0) to disable network timeouts.` Replacement:
    those calls disable both the per-attempt stall and the aggregate connect
    deadline. They do not disable cleanup, cancellation, or idle disposal.
  - §10.2 joint listing: `Existing Deadlines.connect_ms and io_ms, pool
    connect_timeout_ms, and stream io_timeout_ms use zero for disabled and
    1–2,147,483,647 for a finite allowance.` Replacement: each of those fields
    keeps that numeric range. `io_ms` and stream `io_timeout_ms` are the stall.
    `connect_ms` and pool `connect_timeout_ms` are the aggregate. Equal numbers
    are not required.

  Unchanged, and a design diff that edits them fails S1.1:

  - §10.1: `Config::io_timeout_ms defaults to 3,000 ms for a standalone stream;
    endpoint construction captures its configured native timeout and an Open
    request can only shorten it.`
  - §10.2 request-tighten: `A request can only tighten endpoint policy: zero
    is admissible only when the endpoint's corresponding network timeout is
    already disabled; a positive request may bound a disabled endpoint or
    shorten its positive configured value.`
  - §10.2 disposal: `Cancellation/shutdown still disposes connections, and
    cleanup stays bounded when network timeouts are disabled.`
  - §10.1 active-I/O byte-progress, pause, and post-open rules, in full.

### Phase 2: Open carries two values (milestone: a default endpoint Open has `io_ms` 3,000 and `connect_ms` 10,000; `--ssh-timeout 0` sends zero for both network deadlines; a stuck HTTPS connect expires at 10,000 and not later)

`RuntimeState` currently stores only `io_timeout_ms`, so Open cannot name the
aggregate even after construction stops overwriting it. This phase does not
change `Control` or the setup stages.

HTTPS uses the same Open path. Restoring the pool's own connect budget changes
a default HTTPS connect from the copied 3-second I/O value to 10,000 ms. That
widening is intentional and bounded: HTTPS `connect_ms` is the pool aggregate,
never `io_timeout_ms`, and never greater than `connect_timeout_ms`. This phase
does not redesign HTTPS authentication.

- **S2.1: stop overwriting the pool budget** *(`transport_host/mod.rs`
  `SshEndpointConfig::from_environment` and the runtime that builds the
  driver; ~150 lines with a construction test)*. `io_timeout_ms` remains
  `transport_timeout_ms()`. `pool.connect_timeout_ms` stays
  `pool::Config::default()` (10,000) when the native timeout is positive, and
  is zero when the native timeout is zero. Carry that connect budget into
  session state beside `io_timeout_ms`. Pass the stall allowance into setup
  `Job` construction as a separate argument from the aggregate instant; identity-check
  jobs that do not perform SSH setup keep a disabled stall unless they already
  had a deadline.

- **S2.2: Open sends each clock in its own field**
  *(`transport_host/session/driver.rs` deadline construction and the HTTPS
  budget that reads `connect_ms`; ~200 lines with tests)*. `Deadlines.io_ms`
  is the stall setting. `Deadlines.connect_ms` is the aggregate from S2.1.
  The existing 150-second stream waiter stays a stream waiter. One assertion
  covers an SSH Open and one covers an HTTPS Open: default `io_ms` is 3,000
  and default `connect_ms` is 10,000. A stuck HTTPS connect fixture, on the
  deterministic clock from S3.1 once that seam exists, expires at the aggregate
  and not later. Until S3.1 lands, the HTTPS assertion locks the Open value;
  the expiry fixture is finished in S3.2 and is part of the Phase 2 milestone.

### Phase 3: setup expires a stall, not a progressing attempt (milestone: production DNS, TCP, handshake, and agent stages that each finish inside the stall allowance succeed when their sum exceeds it; one stage that never becomes ready expires as reason `stall` while the aggregate is still in the future)

- **S3.1: stall allowance on the setup control** *(`git/endpoint/agent_job.rs`
  `Control`; ~350 lines with direct tests)*. `Job` keeps the aggregate instant
  it already receives from the pool. `Control` also takes the stall allowance
  and a deterministic clock: tests advance time without `thread::sleep` or
  wall `Instant::now` inside `check` / `quantum`. The first `begin_wait` starts
  the stall allowance at the clock's current time. `complete_wait` records
  completion and starts a fresh allowance. `check` fails with reason `stall`
  when the current wait has had no `complete_wait` for the allowance, and with
  reason `aggregate` when the aggregate instant has passed. `quantum` is the
  minimum of the usual 20 ms slice, the remaining stall allowance, and the
  remaining aggregate. Returning from `quantum`, `check`, or a sleep is not
  `complete_wait`. Disabled stall allowance (zero) does not invent a wait
  deadline; a zero aggregate still means no network deadline.
  `begin_interaction` preserves the remaining stall and stops charging it;
  `end_interaction` resumes that remainder. The endpoint forwards the pool's
  begin/end interaction to the in-flight setup `Control`.

- **S3.2: scripted clock regression** *(`agent_job.rs` and `ssh_setup.rs`
  tests; ~250 lines)*. Drive the S3.1 clock. Progressing case: four
  `complete_wait` calls, each shorter than the stall allowance, sum longer
  than the stall allowance and shorter than the aggregate, result accepted.
  Idle-slice case: repeated `quantum` returns with no `complete_wait`, advance
  past the stall allowance, aggregate still ahead, reason `stall`. Readiness
  case: several `complete_wait` calls inside one handshake, each under the
  stall allowance, sum over it, result accepted. Interaction case:
  `begin_interaction` longer than the stall allowance does not expire the
  stall; the next wait after `end_interaction` still expires if it does not
  complete. No `thread::sleep`. No GitHub.

- **S3.3: production wait boundaries** *(`git/endpoint/ssh_network.rs`,
  `agent_auth.rs`, `agent_socket.rs`, and the setup function that calls them;
  ~400 lines with a production-graph test)*. Feed `io_timeout_ms` into the
  setup `Control` as the stall allowance and the pool aggregate instant as the
  aggregate. Call `begin_wait` / `complete_wait` only under the §2 rule.
  `wait_socket`, `wait_session`, and the agent `EAGAIN` sleep call `complete_wait`
  when the socket is ready or the native call returns a finished result. A
  quantum slice with no readiness does not. DNS, TCP connected, handshake
  success, and an agent identity or signature return are completions. The
  regression uses the production functions or a thin wrapper they call, and
  the S3.1 clock: four of those completions each inside the stall allowance,
  sum beyond it, aggregate ahead, succeed and may be retained; a hung TCP
  connect with no readiness past the stall allowance returns reason `stall`
  and is not reusable; an agent `EAGAIN` sleep loop past the stall allowance
  with no auth progress returns reason `stall`.

### Phase 4: disposal and the reported stage (milestone: a disabled or cancelled setup still destroys the socket and is not reused; a setup timeout is visible as a setup timeout with reason `stall` or `aggregate`)

S4.2 follows S2.2 because both edit `driver.rs`. S4.1 follows S2.2 and S3.1
because late-result rejection uses the uncoupled aggregate.

- **S4.1: disabled timing and late results** *(`ssh_setup.rs`
  `poll_connected` and the setup-job disposal path; ~300 lines)*. Depends on
  S2.2 and S3.1. The late-result instant is the pool aggregate connect deadline
  after uncoupling, not the 3-second I/O value. Cancel remains the existing
  cancel/cleanup path. The install gate treats aggregate expiry, stall expiry,
  and cancel as rejection signals. A rejected late success is a setup timeout
  with reason `aggregate`, `stall`, or cancel, not `PermissionDenied`.
  `reusable()` stays false and the resource is not `Idle`. Timeout zero runs
  a scripted progressing setup with no network deadline. Cancel during that
  setup disposes the job within the existing cleanup bound even though the
  aggregate instant is absent. With `io_ms=3000` and `connect_ms=10000`:
  success at 5 seconds after cancel is dropped; success after the aggregate
  instant is dropped; success before the aggregate is accepted.

- **S4.2: report the setup stage and the reason**
  *(`transport_host/session/driver.rs` `failure_io` and the Open failure path;
  ~200 lines with a driver test)*. An `OpenFailed` timeout during setup is
  returned as a timeout at the setup stage with reason `stall` or `aggregate`.
  It is not `stream::Error::PeerFailed`. Authentication failure stays
  authentication failure. Two fixtures assert different caller-visible reasons:
  one idle wait, and many short waits whose sum passes the aggregate.

### Phase 5: qualify the installed alpha (milestone: production stall enforcement is already proven, and a rebuilt alpha completes cold live fetches at the default timeout)

Depends on S2.2, S3.3, S4.1, and S4.2. One agent, after that clock change has
been reviewed. Broader platform and release work stays paused.

- **S5.1: review the bounded clock change** *(review note beside this plan;
  no product code)*. Review S3.1, S3.3, and S4.1 against §2: stall allowance
  resets only on `complete_wait`; idle slices do not reset it; the aggregate
  does not reset; interaction pauses both; zero disables network timing and
  still disposes; a late result is not reused; stall and aggregate reasons
  differ. Record the verdict before rebuilding.

- **S5.2: rebuild, reinstall, and repeat the cold fetches** *(alpha install
  plus retained evidence under
  `gwz-core-evidence/campaigns/transport-qualification/runs/`; no production
  source beyond the rebuilt tree)*. The installed alpha to replace is SHA256
  `5419fd0218ab06084bee9152b09a79864a2c165bce62d9d7924a70d4bb6dce8c`. This step
  stays open unless the S3.3 production-graph regression has passed: one idle
  stage expires with reason `stall` while the aggregate is ahead. A tree where
  that stall path is unwired fails this step even if default cold fetches
  pass. A default cold fetch that succeeds only because `connect_ms` is 10,000
  is not proof of the two-clock fix. Repeat the cold live fetches at the
  default timeout, with the same operator SSH agent, and retain the commands,
  results, and any setup-timeout reason. A pass with `--ssh-timeout 15` does
  not close this step. If the default still fails, record the retained reason.
  Do not raise the global timeout inside this step.

## 4. Dependency sketch

```text
S1.1
  ├─ S2.1 ── S2.2 ── S4.2
  │            │
  └─ S3.1 ── S3.2
       │       │
       └── S3.3
              │
S2.2 ──────── S4.1
S3.1 ─────────┘
S2.2, S3.3, S4.1, S4.2 ── S5.1 ── S5.2
```

S2.1 and S3.1 can proceed together after S1.1. S3.3 waits on S3.1. S4.1 waits
on S2.2 and S3.1. S4.2 waits on S2.2. The HTTPS stuck-connect expiry fixture
in the Phase 2 milestone is finished with the S3.1 clock in S3.2. S5.2 waits
on S3.3 having passed, not only on a live fetch.

## 5. Out of scope

- Changing the default 3-second `--ssh-timeout` or the default 10-second pool
  connect budget.
- Resetting the pool `ConnectClock` on setup progress, or applying §10.1
  byte-progress rules to handshake and agent authentication.
- A new CLI flag, schema field, or message. The `stall` / `aggregate` reason
  is a failure detail on the existing setup timeout, not a new flag.
- Preserving the copied 3-second value as an HTTPS connect ceiling. The HTTPS
  bound is the pool aggregate.
- Windows SSH, placement work, and the rest of the paused release gate.
- Treating the 2026-09-22 15-second fetches, or a default cold fetch after
  `connect_ms=10000` alone, as evidence that the defect is fixed.
