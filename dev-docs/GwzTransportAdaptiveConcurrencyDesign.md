# GWZ transport adaptive per-host concurrency — design

Date: 2026-10-06; revision 0. Status: **DRAFT, 2026-10-06, not reviewed.** It authorizes no implementation, commit, tag, push or publish. Review when it is taken up: dual Consistency and Safety (it changes how failures are classified and retried), plus Surface (it adds an optional wire field and a user-visible note).

- **The operator's direction (2026-10-06, verbatim):** "so "Capacity" failures should be the throttle, not an error, hence setting 32 gets limited to 8 dynamically which means it needs to be tested to see if it was a different issue so you dynamically detect the actual limit, it could be for another client running elsewere." Earlier the same day: "It may be that github finally started throttling connections - so now we have a signal at to the max number of concurrent connections allowed (per user I suspect, I hope not per IP addr)."
- **Authority.** [Retry plan](GwzRemoteTransportRetryPlan.md) §§4–6 (the closed retriable set, the per-key machine, the pool's caps), as amended by [amendment 2](GwzTransportReleasePlanAmendment-2.md) §3.20 (OD18, the cold start's first wave). [HTTPS endpoint design](GwzRemoteTransportHttpsDesign.md) §2 (amended 2026-10-06: capacity is a throttle, not an error, and "Adaptive limits are designed separately", which this is), §6 and §7's response table. [Transport setting design](GwzTransportOffSwitchDesign.md) (`--transport native`). The pending connection-statistics proposal TR2.24 ([revision 1](../../dev-docs/history/transport-handoff-2026-10-02/designs/GwzTransportConnectionStatsDesign-rev1.md)) is mentioned in §9 only.
- **Code lines** are at gwz-core `279860c1` (HTTPS setup-slot queueing), gwz-transport `ff6083b5` and the gwz-dev root `e4bab45b`. Paths under `src/` are gwz-core's, and under `pool/` are `gwz-transport/src/pool/`. Evidence lines are from `gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-06-tr8-1-linux` and `…/2026-10-06-tr8-1-macos-https` (their READMEs).
- This is a design. It has no phases and no steps; the operator takes it to a plan separately.

## 1. Outcome

Any capacity signal, local or from a server, slows the command down. None of them fails a member while budget remains. `--max-per-host` is a ceiling. The concurrency actually used per host is a limit the transport learns during the command, which can be lower than the ceiling because another client, on the same account or the same address, holds part of the server's budget. The transport finds the limit, confirms it before trusting it, raises it again when the server allows, and gives up with a clear error naming the host and the observed limit when the budget runs out.

Decisions, each argued below:

| # | Decision | Section |
|---|---|---|
| D1 | Every failure is classed **Queue** (local capacity), **Throttle** (the server said slow down), **Suspect** (looks like a limit, may be something else), **Transient** or **Permanent**. Only Throttle and Suspect move the learned limit, and Suspect only after confirmation. | §3 |
| D2 | A governor per (scheme, host, port) per command holds the effective limit `L`, `1 <= L <= C`, `C` being the configured ceiling. A decrease **fits** the limit to what the server was seen to accept, and an increase is one connection at a time, probed on a timer. | §4 |
| D3 | A throttled attempt is **requeued**, not failed. It counts as one of that member's `--max-retries + 1` attempts. It does not count against the key's retry machine, which a throttle must not close. | §5 |
| D4 | A throttled member that runs out of budget fails with `Capacity` plus an optional failure detail naming host, port, observed limit, configured limit, attempts and the last signal. It is never a bare `Capacity`. | §5.3 |
| D5 | The learned limit lives for one command. Nothing persists. | §6 |
| D6 | The local 64-caps stop failing members. Two are removed, one is made a wait with release at the end of the member's work, and the rest are already waits that need their expiry reported as a throttle. | §7 |
| D7 | The pool (gwz-transport) gains one numeric soft limit per (scheme, host, port) and learns nothing about error codes. Classification and the governor live in gwz-core beside the retry machine. | §4.6 |

## 2. What the code does today

### 2.1 Where capacity and limits are enforced

- **Pool ceilings, installed per operation** from the resolved `--jobs` and `--max-per-host` (retry plan §6): `per_user_host` and `per_host` are the resolved `--max-per-host` (default 32), `total` is `max(256, jobs)`, `max_requests` `max(1024, jobs)` (`pool/mod.rs:36-47`). `schedule()` creates a connection only while the host's and user-host's counts, idle and closing included, are under the caps (`pool/allocation.rs:53-56`), evicting the oldest idle connection to make room (`pool/allocation.rs:101-134`). A waiting request stays `Waiting`; a connect failure reaches only the request that owns that `Opening` connection, as `RequestState::Failed(ConnectFailed{code, effect, setup_cause})` (`pool/allocation.rs:236-246`). Nothing else waiting hears of it. `request_until` is the one pool refusal, `Capacity`, at `max_requests` pending requests (`pool/machine.rs:195`), which one operation cannot reach because members in flight never exceed `--jobs`.
- **Endpoint admission, ahead of the pool.** Both endpoints queue an open and start it only while the opens in flight on its host are under the pool's per-host and per-user caps: SSH in `admits_open` (`src/git/endpoint/placement_endpoint/admission.rs:397-409`, with `queued_opens` at `:203,214`), HTTPS in `admits_attempt` (`src/transport_host/https_endpoint/retry.rs:217-226`, with `Held`). SSH also caps opens across all hosts at `open_ceiling`, the least of the pool total, `MAX_REQUESTS` (64, `placement_endpoint.rs:36`) and half the 64-job budget, so 32 (`placement_endpoint.rs:238-244`). An open held by these limits has its allocation clock running; a key the retry machine holds stops it (`admission.rs:178-215`, `retry.rs:188-215`). The member scheduler also limits members per hostname by the same number (`operation/par_map_per_host.rs`), so a held open is in practice one behind a limit lower than the configured one, which today means a local one.
- **HTTPS setup slots** (279860c). The connector holds 8 slots for the blocking resolver and TLS-configuration jobs. A connection past them now waits for a slot inside its connect deadline (`src/git/endpoint/https_connection.rs:240-262`). It no longer fails with `Capacity`; the 32-member HTTPS fetch at defaults was clean in 47 of 47 runs (commit message).
- **The retry machine** (`src/git/endpoint/setup_retry/machine.rs`): per operation, per pool key, states Cold (first wave parallel, OD18), Healthy, Degraded (one probe at a time), Waiting, Closed. A retriable setup failure counts one attempt for the key; a budget of `R + 1` (default 4) closes the key and finishes every member on it with the recorded failure. Waits are `min(30 s, 1 s x 2^(n-1))` plus 0 to 250 ms jitter (`setup_retry/backoff.rs`).
- **The classifier** (`src/git/endpoint/setup_retry.rs:55-72`): Retry for a setup-phase `Io`, `CarrierLost`, a stall or aggregate `Timeout`, or `Unavailable` caused by a refused connection; **Return** (the member's own result, the key does not move) for `Capacity`, `Cancelled` and an allocation `Timeout`; Close for everything else in setup. Only a failure of a *connect* (`FirstConnect::Failed`) is `Phase::Setup`; anything after the first request byte is `Phase::Other`, so Return (`https_endpoint/retry.rs:107-113`).

### 2.2 Which local limits still fail a member with `Capacity`

| Limit | Where | What reaches the member today |
|---|---|---|
| Supervised-job budget, 64 process-wide (`static COUNT`) | `src/git/endpoint/agent_job.rs:13,254-258,365-369` | `Job::start` returns `WouldBlock`; SSH maps it to `Capacity` (`ssh_setup.rs:441`, `ssh_worker/open_request.rs:77-79,93`), HTTPS likewise (`https_connection.rs:279`). Classified Return: the member fails. |
| Concurrent HTTPS operations per endpoint, 64 | `src/git/endpoint/https_operation.rs:38` (`Operations::acquire`) | `Capacity`, mapped at `src/transport_host/https_endpoint.rs:239-243`. A count of concurrent *commands*, not of repositories. |
| Live HTTPS streams and requests per endpoint, 64 | `src/transport_host/https_endpoint.rs:220,234` | `WouldBlock`, which the driver already waits on and retries until the allocation deadline (`session/driver/opening.rs:228-235`). |
| Distinct HTTPS repositories per request, 64, never released | `src/transport_host/request.rs:19,115-118` (`https_routes`) | `Capacity` on the 65th distinct canonical URL of one request. 1.0.17 has no such limit. |
| Route table, 64 routes per endpoint | `src/git/endpoint/https_policy.rs:85` (`Routes::admit`), built at `src/git/endpoint/https_worker.rs:88` | `Capacity` on the 65th (operation, URL, receive) route; routes are released only when the operation is sealed (`https_operation.rs:51-74`, `https_policy.rs:121-124`). |
| Credential answers per route, 6 | `src/git/endpoint/https_worker/credentials.rs:69` | `Capacity` on a 7th destination for one route. |
| Shared SSH/HTTPS reservation | `src/git/endpoint/shared_reservation.rs:83,111` | `Capacity` when the endpoint-wide reservation is full. |

A workspace with more than 64 HTTPS members fails every fetch past the 64th at `request.rs:116`, whatever the server allows. That is a defect on its own, and §7 fixes it.

### 2.3 How server signals are handled today

- **HTTPS status.** `https_policy::classify` maps 401 and 407 to `Authentication`, 403 and 404 on discovery to `RepositoryRefused`, and every other 4xx and 5xx to `Io` (`src/git/endpoint/https_policy.rs:44-52`). The design's table says "no Retry-After sleep or retry" (HTTPS design §7, line 322). No code reads `Retry-After`; a search of `src/` for `429`, `Retry-After` and `503` outside tests finds nothing. The status survives in `facts.http_status` (`https_worker/prepare.rs:352-353`). A 429 on discovery is then Phase `Other`, so Return: the member fails with `Io`, status 429.
- **HTTPS connect.** A refused, reset or timed-out connect, a TLS EOF, is a `Phase::Setup` failure with `Io`, `CarrierLost`, `Unavailable`+ConnectionRefused or a stall/aggregate `Timeout`, all Retry.
- **SSH.** A server's `MaxStartups` drop reaches the handshake as a reset or an end of stream, which libssh2 reports as its own error and `ssh2` as `ErrorKind::Other`; `ssh_setup` maps it to `Io`, which is Retry (`src/git/endpoint/ssh_tests/max_startups.rs`, header and `assert_dropped`). The test pins macOS and Linux only. `ssh_setup.rs:437` maps `ConnectionAborted` to `Cancelled`, which is **Return**, and the file is `cfg(unix)` throughout, so what Windows reports for the same drop is unpinned (§12, fact F3).
- **Pre-banner text.** OpenSSH writes one line before closing ("Not allowed at this time", OpenSSH 10.3; "Exceeded MaxStartups", 9.6p1; `max_startups.rs` header). The client does not surface it: the setup closure gets an error kind, not the line.

### 2.4 What the evidence shows about GitHub

Kept to what the runs recorded.

- **HTTPS, anonymous, public repositories, one macOS network path and one Linux (Raspberry Pi 5) path:** 1.0.17 opened one connection per member, 32 of 32 and 16 of 16, whatever `--max-per-host` said, with no failed member in any run (macOS README, Results; Linux README, row "32 members, `--max-per-host 32`", 0.37 s, 32 conns, 0 failed). The candidate's 24 failed members of 32 were its own 8-slot refusal, "a limit in the endpoint's connection pool", deterministic and not a network effect (Linux README, Defect 1). **So the evidence shows no GitHub limit on 32 concurrent HTTPS connections from one address.** Whether GitHub started throttling is not shown by these runs, and the operator's hypothesis ("so now we have a signal") is not supported by them: the signal was local.
- **SSH:** 1.0.17 returned a partial result in 9 of 54 SSH runs, each `failed to connect to github.com: Operation timed out`; the candidate's own transport had none in 54 (Linux README, Outcome and Defect 3). The README attributes this to GitHub's slow-login tail against the SSH library's per-step timeout. That is an ambiguous failure: a timeout under load is indistinguishable, at the client, from a refusal by a limit. §4.3's confirmation rule exists for it.
- **Assumptions to verify, none shown by the repository:** (a) GitHub documents secondary rate limits for its REST and GraphQL APIs, including a concurrency limit, with 403 or 429 and `Retry-After`; whether the git smart-HTTP endpoints enforce a similar limit, and with which status, is unverified. (b) Whether a limit on `github.com:22` is per account, per address or global is unverified. (c) Whether OpenSSH's `PerSourcePenalties` (the fixture turns it off, `ssh_tests/max_startups.rs` fixture config) penalises a client that keeps dropping out before authentication, so that retrying into a `MaxStartups` drop prolongs it, is unverified. §10.3 is the live check.

## 3. Throttle signals

### 3.1 The classes

- **Queue.** A limit this process imposes on itself and knows in advance: the 8 setup slots, the 64-job budget, the 64-caps. The right response is to wait for the slot. It is no evidence about the server, never moves the learned limit, never counts as a retry or a strike, and never becomes a failure while the wait is within its bound (§5.2).
- **Throttle.** The server said so in a form that cannot mean anything else. One occurrence acts (§4.2).
- **Suspect.** A failure that is what a limit looks like and also what a flaky network, a server restart or a slow login looks like. It acts only once confirmed (§4.3). It is also, as today, a retriable setup failure for the retry machine, which acts on it at once in its own way (§5.1).
- **Transient.** A failure that is not about concurrency. Retried by the retry machine as today, never moves the limit.
- **Permanent.** Not retried: authentication, trust, protocol, invalid request, repository refused, missing agent or `known_hosts`, `AddrNotAvailable`, cancellation. Unchanged (retry plan §4).

### 3.2 Per protocol

| Signal | Class | How it is recognised | Today |
|---|---|---|---|
| Setup slots full (HTTPS, 8) | Queue | The connector's semaphore is not available | Already a wait (279860c). On expiry it reports a **server** `Timeout` with `SetupFailureCause::Aggregate` (`https_connection.rs:246-250`), which the classifier retries as a server stall: a local wait indistinguishable from a server's. Must become a local expiry (§7.5). |
| Job budget full (64) | Queue | `Job::start` -> `WouldBlock` | `Capacity`, Return: a member fails. Becomes a wait. |
| `open_ceiling` (32), per-host and per-user caps | Queue | `admits_open`, `admits_attempt` return false | Already a wait. |
| The 64-caps in §2.2 | Queue, or removed | §7 | `Capacity`. |
| HTTP **429** on discovery | Throttle | Status 429 on the open's discovery GET, before any Git byte is delivered, `Effect::None` | `Io`, Return: the member fails. |
| HTTP **503 with `Retry-After`** on discovery | Throttle | Status 503 and a parseable `Retry-After` (delta-seconds or HTTP-date) | `Io`, Return. |
| HTTP 503 without `Retry-After`; 500, 502, 504 | Suspect | Status | `Io`, Return. |
| HTTP 429 or 503 on a **POST** (upload-pack or receive-pack exchange) | Throttle for the key's limit; **not requeued** | Status on an exchange whose request body was handed to the network | `Io`, Return. §5.4: the member reports `Throttled` once, and the key's limit still drops. |
| HTTP 403 | Permanent (`RepositoryRefused`, unchanged) | Unless it carries `Retry-After`: an **assumption to verify**, §2.4(a), OQ4. A misread would mark a private or missing repository as throttled. | `RepositoryRefused`. |
| TCP connect refused (`ECONNREFUSED`) | Suspect | `Unavailable` + `ConnectionRefused` in setup | Retry. |
| TCP reset, TLS EOF, peer close during connect or TLS | Suspect | `Io` or `CarrierLost` in `Phase::Setup` | Retry. |
| Connect stall or aggregate timeout (a full accept backlog drops SYNs, which looks like this) | Suspect | `Timeout` with cause Stall or Aggregate, **not** a local wait expiry (§7.5) | Retry. |
| SSH drop at banner or key exchange (`MaxStartups`-style) | Suspect | `Io` raised by `ssh_network::establish` (TCP, banner, key exchange: `ssh_network.rs:48-78`), as `ErrorKind::Other`, `ConnectionReset` or `UnexpectedEof`, **before authentication**. The pre-banner line, if later surfaced, makes it a Throttle (§12 F2). | Retry. |
| SSH refused connection | Suspect | `Unavailable` + `ConnectionRefused` | Retry. |
| SSH failure after key exchange | Transient or Permanent | Authentication phase (`ssh_local.rs:80-123`): `Authentication` is Permanent; other errors keep today's class | unchanged |
| SSH `ConnectionAborted` | Today: `Cancelled`, Return | A drop mapped to `Cancelled` ends the member (§2.3). Where a platform reports a `MaxStartups` drop this way, it must be Suspect. Pin per platform (§12 F3). | Return |

An abandoned setup (a timed-out job whose thread has not returned) still holds its socket, so it counts as an open connection for the limit until the job retires (`agent_job.rs`: the permit is held until the result is taken or disposed).

## 4. The governor

### 4.1 State

One governor per (scheme, host, port) per operation, created Cold with the retry machines and dropped with them (`Retries::remove`, `https_endpoint.rs:381-385`). Usernames share it: a `MaxStartups` limit and a connection limit are enforced before the server knows the user. SSH and HTTPS on one host are two governors; the pool's per-host cap still spans both (`pool/allocation.rs:55`).

- `C`, the ceiling: the pool's `min(per_host, per_user_host)` for the operation, then any local ceiling that applies (`open_ceiling`). It never moves during a command.
- `L`, the effective limit, `1 <= L <= C`, starting at `C`. **The floor is 1.** A command never drops below one connection to a host, and a member that cannot get through at 1 fails by budget (§5.3), not by a limit of 0.
- `c(a)`, an attempt's **concurrency**: the connections to the key that existed when attempt `a` started, itself and abandoned setups included.
- `G`, the **known-good** level: the greatest `c(a)` over attempts whose setup completed (HTTPS: and whose discovery was not throttled) in this command. `G = 0` until the first success.
- A **window**: the attempts started under one value of `L`. A window closes when every attempt in it has ended.
- `hold_until`, a time before which the key starts nothing; and `probe_gap`, the wait between probes up (§4.4).

### 4.2 Decrease on a Throttle

On the first Throttle signal from an attempt of the current window:

1. `L := max(1, floor(min(L, c(a)) / 2))`. This stops the stampede at once; new starts above `L` wait.
2. `hold_until := now + max(Retry-After, backoff)`, where `Retry-After` is the server's, and `backoff` is the retry plan's `min(30 s, 1 s x 2^(n-1))` plus jitter for the key's `n`th consecutive signal since its last success. The hold is capped at the hold cap, the aggregate default, 30 s. A `Retry-After` beyond the member's remaining budget fails that member at once as throttled (§5.3) rather than waiting; the key still holds for the cap.
3. A new window starts. Throttle signals from attempts of an older window do not decrease again: they started before the decrease and say nothing new.
4. **Fit at window close.** When the window the throttle fell in has closed, if it had at least one success (`G >= 1`) and the throttles were above it, set `L := min(C, G)`. This is the "32 gets limited to 8" step: eight attempts completed together, the ninth to thirty-second were refused, so the server allowed 8. The halving of step 1 may undershoot (the server allowed 20, and the first refusal arrived before the successes); the fit corrects it upward to what was proven, never above `C`.

Without a success in the window (`G = 0`) the fit does nothing: the halving stands, and the next window tries at that `L`. Every window that is refused again halves again, down to 1; the members' budgets (§5.3) end that descent.

### 4.3 Confirmation of a Suspect

A single Suspect failure never moves `L`. An ambiguous failure is attributed to concurrency only when both hold:

- it **started above the known-good level**, `c(a) > G`. A failure at a concurrency the server has already accepted in this command is about something else, and
- it is **corroborated**: at the close of a window, two or more *distinct* attempts of that window failed Suspect at `c(a) > G`, or one did in each of two consecutive windows with a success between them.

Confirmed, the governor decreases as in §4.2 steps 1 and 4 (and holds for the backoff, no `Retry-After`). Unconfirmed, nothing changes in the governor, and the retry machine's ordinary retry is the whole response.

How this treats the cases that matter:

- **One failure among 32** (a network blip, one dropped handshake): one strike at one window, no corroboration. `L` stays `C`. The member retries through the retry machine.
- **`MaxStartups` 10:30:100 against a first wave of 32** (OD18): about 10 attempts succeed (`G` ≈ 10) and about 22 drop above it, many distinct attempts in one window. Confirmed at the window's close: `L := G`. A server drops probabilistically past its start value, so `G` is noisy; the next window's drops confirm a lower `G`, or its successes raise it. It converges because each fit is bounded by what was observed.
- **A host that is down, or a dead key** (nothing succeeded, `G = 0`): not a limit. The retry machine's Degraded probe and budget decide it as today. The governor does not act.
- **The Linux SSH slow-login tail** (1.0.17's 9 of 54 timeouts): a stall `Timeout` at some `c(a)`. One in a window is a single strike. If several arrive above `G`, `L` is lowered, which is the right response to a login that is slow under load, and the probe-up of §4.4 restores it when the load passes. The cost is bounded: `L` never goes below 1 and recovers on the probe timer.

### 4.4 Increase

Additive, one connection per probe, on a timer, so a server that tolerates more is found and a limit is never overrun in bulk:

- When `L < C`, no signal has arrived for `probe_gap`, and at least `L` attempts have completed since the last decrease or probe, `L := L + 1` and a probe window starts. `probe_gap` starts at 5 s.
- A probe window that closes with no throttle or confirmed Suspect resets `probe_gap` to 5 s and may probe again at once. A probe window that ends in a confirmed throttle returns `L` to its value before the probe (the fit does this) and doubles `probe_gap`, to at most 60 s. A server whose limit is fixed is probed once a minute, not constantly.
- Every constant here is an initial value for the live check to tune (OQ6).

A command that finishes in a few seconds never probes, and does not need to. A long command (thousands of members, or large transfers) probes upward as another client's load passes.

### 4.5 Shrinking under live connections

A decrease does not close leased connections. It stops new starts above `L`, and the pool closes idle connections above `L` first (its existing `start_closing(Evicted)`, `pool/lifecycle.rs:44`), so that an idle connection of one identity does not hold the slot a waiting request of another needs.

### 4.6 Where it lives

- **gwz-core**, `src/git/endpoint/setup_retry/` beside `machine.rs`: a `Governor` state machine with an injected clock and jitter, exactly as `Machine` is built and tested (`machine_tests.rs`). A new `Verdict`/evidence pair out of `classify` (§5.1). Both endpoints consult it where they now consult the per-host cap: `admits_open` and `admits_attempt` read `min(C, L)`.
- **gwz-transport pool**, one narrow addition: a soft limit per (scheme, host, port), `set_limit(key, n)` and the count it compares, honoured in `schedule()` next to `per_host` and `per_user_host`, and in its idle eviction. A number in, nothing about codes, as retry plan §5 requires ("`gwz-transport`'s pool keeps granting leases. It does not learn error codes"). No change to the wire protocol.

## 5. A throttle requeues; it does not fail

### 5.1 The classifier gains a second answer

`classify(failure, phase) -> Verdict` (retry plan §4, `setup_retry.rs:55`) keeps its three verdicts. It gains a **throttle evidence** answer alongside: `Throttle`, `Suspect` or `None`. The endpoint passes the HTTP status and parsed `Retry-After` it already holds (`prepare.rs:352`), and the SSH endpoint the stage the failure came from. Both are facts the endpoint knows and gwz-transport does not.

| Failure | Retry verdict (as today) | Evidence | What the endpoint does |
|---|---|---|---|
| 429, or 503 + `Retry-After`, on discovery | new **Requeue** | Throttle | Member requeued; governor §4.2; the key's retry machine is told the attempt ended with no verdict (as `abandoned`, `https_endpoint/retry.rs:70-74`) and is **not** counted or closed. |
| Setup-phase `Io`, `CarrierLost`, refused, stall or aggregate | Retry | Suspect | As today through the retry machine; the governor records the strike (§4.3). |
| Queue | none | none | Member waits (§5.2). |
| Everything else | as today | none | as today |

Why a throttle does not enter the retry machine's counter: that counter is the key's `R + 1` budget, and exhausting it closes the key and finishes every member with the recorded failure (retry plan §4). Finding a limit that is 5 halvings below `C` would then close the key. A throttled key is healthy and narrow, not failing. The throttle has its own hold (§4.2) and its own per-member bound (§5.3).

### 5.2 Waiting, and what bounds it

A member waits for one of three reasons, each bounded:

- **Behind full slots** (`L` occupied by live connections, or the job budget held): the wait ends when an occupant finishes. It is bounded transitively by the occupants' own clocks (network idle, interaction, cleanup). The member's allocation clock **stops** while it waits this way, as it stops while the retry machine holds a key today (`retry.rs:188-196`, `admission.rs:178-186`). Without this, a member parked behind a slot that a long clone holds would fail at 30 s for no fault of its own.
- **In a hold** (`hold_until`, or the key Waiting): at most the hold cap, 30 s, per signal, and a member sees at most `R + 1` of them (§5.3). The allocation clock stops, as for Wait today.
- **For a local permit** that is not a slot (the 64-job budget): the wait is on the allocation clock, 30 s. Its expiry is reported as a local throttle (§5.3), not as a server stall.

Queue depth is bounded by structure: members in flight on a hostname never exceed the resolved `--max-per-host` (`par_map_per_host`), so at most that many opens are parked however large the workspace.

### 5.3 The member's budget, and what it finally reports

A member's attempts, retries and throttle requeues together, are bounded by `--max-retries + 1`, 4 by default (the operator's rule: requeues count against the existing retry budget). An attempt that was only held back by `L` and never started is not an attempt. A refusal that arrives from a window older than the last decrease counts as that member's attempt but moves no limit.

When the budget runs out, a Retry-After exceeds what is left, or a local permit wait expires, the member fails with:

- code **`Capacity`** (unchanged, so an older reader still renders something true), and
- an optional `FailureDetail` field, `throttle`, added beside `retry_attempt` (`gwz-transport/src/protocol.rs:851-857`): `{scheme, host, port, limit_observed, limit_configured, attempts, attempts_max, elapsed_ms, last_signal}`, where `last_signal` is one of `http_429`, `http_503`, `refused`, `reset`, `setup_timeout`, `local`, and an optional `retry_after_ms`. It carries counts, the pool key's host and port, and milliseconds; never a URL, a response body, or a credential. An optional field is additive, where a new `ErrorCode` or `SetupFailureCause` value is not: both decode an unknown value as `DecodeError::UnknownEnum` (`protocol.rs:240-336`), which would break an older peer. This follows the stats design's D5 and TR1.6's detail (the retry plan's own `retry_attempt` is the precedent).
- The message the user sees: `github.com:443: throttled: the server limited this command to 8 concurrent connections (32 requested); gave up after 4 attempts over 21 s`. For a local expiry: `local setup capacity exhausted for 30 s (64 supervised jobs in use)`. For a Retry-After beyond budget: `… asked to wait 120 s (more than this command's remaining budget)`.
- `--max-retries 0` means a Throttle fails the member at once with this error: consistent with "zero means the first failure is the member result" (retry plan §4). A Queue wait is not a retry and still waits.

The worst-case bound for a member is therefore its `R + 1` attempts, each bounded by its own setup clocks (retry plan §5's `(R + 1) x aggregate`, plus holds of at most the 30 s cap each, plus the jitter), plus waits behind occupants whose own clocks bound them. No wait is unbounded.

### 5.4 What is not requeued

A requeue is safe only before the member has delivered a byte of a Git exchange. So:

- **Requeued:** a connect or TLS failure, and a 429 or 503 on the open's **discovery GET**, where no Git byte reached the caller and `Effect::None` (HTTPS design §7's predicate for `RepositoryRefused` uses the same conditions).
- **Not requeued:** a 429 or 503 on an upload-pack or receive-pack **POST**. The request body was handed to the network and the bridge keeps at most one 16 KiB chunk (HTTPS design §6), so the exchange cannot be replayed; the design forbids transparent retry of a POST and never of an uncertain push (§§6, 7). Such a failure lowers the key's `L` and holds the key like any Throttle, so the other members slow down, and the member reports the throttled error once, `Effect::None` for upload-pack and `Effect::Possible` preserved for receive-pack. A fetch's no-op case, which is only discovery, never meets this. Replaying a read-only member as a whole is a separate decision (OQ3).

## 6. Scope of the learned limit: per command, per host, not persisted

The governor belongs to one operation, as the retry machines do. Nothing persists after the command: not on disk, not in the user's configuration, not in a process-wide table.

Why not persist:

- The limit is a function of what other clients hold at that moment. The operator's own reading is that it "could be for another client running elsewhere". A limit learned a minute ago is stale in the direction that costs: too low wastes the command's speed, and a value from a different network is wrong.
- Rediscovery is cheap. §4.2's fit finds the limit in one or two windows, and a command pays one stampede's worth of refusals, each a fast failure that is requeued.
- A persisted file is global state with an identity and a host in it, which the credential-exposure and "no globals" positions argue against, and it would need invalidation by network, account and time.
- The 1.1.0 CLI process exits after the command; the 1.2.0 session host that keeps connections across commands is a separate design whose reuse rules (retry plan 2026-09-28 amendment, reuse design §9) do not yet say how a Cold key starts. If a host ever carries a limit across commands, it should carry a hint only, expiring with the pool's 60 s idle timeout, and never above `C` (OQ5). That is recommended against for now.

The key is (scheme, host, port), not the pool key with a username: pre-authentication limits do not see a user, and a post-authentication limit per account binds all of that user's connections to the host in the command anyway. Hostnames that resolve to one address are not grouped; the governor sees names, as `git_host` does (retry plan §6). That is a stated non-goal.

## 7. The 64-caps

A queue works only where something frees entries while a command runs. Where nothing does, a queue would deadlock, and the entry must be released earlier or the cap removed. One lifecycle per cap:

| Cap | What it bounds | Release today | New lifecycle |
|---|---|---|---|
| **7.1** `https_routes`, 64 distinct canonical URLs per request (`request.rs:19,115-118`) | A per-request map from canonical URL to `{mode, resolved}` and the route lock that serialises one repository's discovery and fixes its auth policy (`request.rs:406-417`) | Never; it dies with the `RequestContext` | **Remove the cap.** The entry is a URL key and two enums, held for the request; memory is linear in the workspace's distinct HTTPS URLs and keyed by the caller's original URL, which a server cannot grow. A queue here would deadlock, since nothing frees an entry before the request ends. Eviction would drop `mode`, which guards "policy is fixed for this request/route" (`request.rs:144`). |
| **7.2** `Routes::new(64)` (`https_policy.rs:85`, `https_worker.rs:88`) | Routes of (operation, original URL, receive): the pinned base after redirects, a challenge, the credential answers and native auth state | At the operation's end, when sealed and its last dependent is dropped (`https_operation.rs:51-74`, `https_policy.rs:121-124`) | **Release a route when its last dependent ends**, and make `admit` wait, not fail, when every route has a live dependent. Each route counts its dependents (the remote and streams that admitted it, as `Operations` already counts the operation's, `https_operation.rs:19-50`). At zero it is dropped; a route with dependents is never evicted. When `admit` finds the table full of routes that all have dependents, the open stays queued (as an open held by a limit does) until one frees or its allocation clock expires, which then reports a local throttle (§5.3). This also drops credential material as soon as its repository finishes, instead of holding every repository's until the command ends. **To verify before this is taken up:** `https_operation.rs:1-2` and `https_endpoint.rs:~237` say routes must outlive a dropped remote so that "discovery routes used later by it" survive; the reason is not recorded in the dev-docs. If a later RPC of the same member can find its route only after the remote dropped, release at the request's end and remove the cap as in 7.1 (the credential-exposure window then lasts the command). |
| **7.3** Credential answers per route, 6 (`https_worker/credentials.rs:69`) | Distinct destination bases one route's credential cache holds | With the route | The bases are the original plus redirect targets, and redirects are bounded at 5 hops (`prepare.rs:375`), so the cache cannot exceed 6. It is not reachable by repository count. **Remove it as dead code once proved** (the redirect limit is the bound), or report it as `UnsupportedOperation` like the hop limit if it is kept. It must never present as `Capacity`. |
| **7.4** 64 operations and 64 streams per HTTPS endpoint (`https_operation.rs:38`, `https_endpoint.rs:220,234,239-243`) | Concurrent *commands* and live streams in one endpoint, not repositories | Operations at seal; streams at close | Already released as work finishes. Streams already wait (`WouldBlock`, the driver waits until its allocation deadline). `Operations::acquire` returns `Capacity` instead; it must return the same `WouldBlock`, so the host waits as it does for streams. |
| **7.5** Process-wide 64-job budget (`agent_job.rs:13`) and the endpoint-wide reservation (`shared_reservation.rs:83,111`) | Supervised setup threads, including abandoned ones, and the SSH and HTTPS aggregate caps | When a job's result is taken or disposed; abandoned jobs hold a permit until their thread returns | **A refusal is a wait.** `Job::start` returning `WouldBlock` leaves the open queued, as `admits_open` leaves it, and a permit release wakes it; `try_reserve` failing does the same. `open_ceiling` already holds SSH opens to 32, so under normal limits the budget is not reached. A throttle makes it reachable: stalled setups time out but their threads, and permits, live until the socket returns, and each holds a connection on the server. The governor counts them in `c(a)` and the pool in its limit until they retire, and the new wait covers the case where the budget is full of them. |

The setup-slot wait (§3.2) is the same kind: **its deadline is the open's allocation, not the connect aggregate**, and its expiry reports a local throttle, so a local wait never reaches the retry machine as a server stall and never gives the governor a strike.

Result: a fetch over more than 64 HTTPS repositories succeeds, including with `--jobs 100` on one host: opens beyond the effective limit are parked, routes are released as repositories finish, and no local count produces a bare `Capacity`.

## 8. Interactions

- **`--max-per-host`.** It is the ceiling `C`. The governor lowers the limit actually used and never raises it above `C`. An explicit `--max-per-host 4` is still 4, and the governor can take it lower. With no throttle, `L = C` for the whole command and the governor adds no wait and no connection. TR8.1's criterion (a 32-member fetch at defaults no slower than 1.0.17 at `--max-per-host 32`) is therefore unaffected: the 8-slot queue is a Queue, not a signal (§3).
- **`--transport native`.** The native route is 1.0.17's libgit2 path, with no endpoint, retry machine or governor, so it has no adaptation. It stays as the off switch (transport setting design). The operator rejects native routes as cover for setups the transport does not serve, so adaptation is built into the transport, not offered as a reason to use native. `--max-per-host` still limits members per hostname in the member scheduler there, as in 1.0.17.
- **The member scheduler.** `par_map_per_host` runs up to `--max-per-host` members per hostname, each parked in its open when `L` is lower, which holds a worker thread (`--jobs`). With `--jobs` at 100 and one throttled host, 32 threads are parked and the rest serve other hosts. With a small `--jobs` the parked threads can starve other hosts' members. The remedy is for the scheduler to read the learned limit and start fewer members of that host. Where the endpoint runs in the CLI process and core in another, a read-only limit has to cross the carrier. Endpoint-only queueing is correct without it, and the scheduler link is an optimisation (OQ2).
- **Connection reuse and pooling.** Idle connections count toward `L`; a decrease evicts idle ones first (§4.5); with a low `L` more requests lease an idle connection, which the pool already does before creating one (`pool/allocation.rs:12-38`). SSH `MaxStartups` counts unauthenticated connections only, and the governor counts all of them, so `L` can be lower than the server would allow for established connections. With reuse in a command that is acceptable, and a measurement can show otherwise (§10.3).
- **SSH and HTTPS paths.** The same governor, the same classifier, two classification tables (§3.2). The SSH endpoint's `admits_open` and the HTTPS endpoint's `admits_attempt` are the two consulting points. Neither endpoint changes how it sets up a connection.
- **gwz-py.** It takes the same per-operation transport entry in 1.1.0 (A2 §3.17, OD14), so the governor, the queueing and the error come with it and need no Python code. The final failure arrives as the message above; the optional `throttle` detail reaches Python only if the response projection exposes it (TR2.24's concern). The note (§9) is a `logging` record on `gwz`, as the transport setting's note is. gwz-py's candidate CI (TR2.21) gains one row (§10.1).
- **Cancellation.** Cancelling during a hold cancels every member queued on the key and starts no probe, as retry plan §4 says for a wait.

## 9. What the user sees

- **A note**, human mode only, once per host per change of the limit, on stderr, when a decrease is confirmed: `gwz: github.com: limited to 8 concurrent connections (32 requested)`. Printed once, not per member; and once more if the limit is later raised or lowered by a probe: `gwz: github.com: limit now 12`. A Queue wait prints nothing. Absent in `--json`, `--jsonl` and `--quiet`, and no JSONL event, the same rules as the transport setting's note (its §4 and §5, `merge_render.rs:245-251`).
- **JSON.** A default `--json` payload is unchanged byte for byte. A failed member carries the throttled message in its error row, and the optional `throttle` detail on the failure for consumers that read it (§5.3).
- **TR2.24.** The connection statistics proposal's `meta.transport_diagnostics`, under `--verbose`, is the natural home for per-host `limit_configured`, `limit_observed`, `throttle_events`, `probes` and `held_ms`. This design only names them; it does not design TR2.24's fields or payload.
- **`--verbose` human lines** may show the limit's history. Not designed here.

## 10. Testing

### 10.1 Fixtures

Three layers, all deterministic where they can be.

1. **The governor and classifier alone.** Pure state machines with an injected clock and jitter, in the style of `setup_retry/machine_tests.rs`: window closes, fit, confirmation, probes, caps, floor, stale-window refusals. No socket, no sleep.
2. **A fake connector with a concurrency limit**, at the `ssh_pool::Connector` trait that both endpoints already use. `LimitedConnector { limit: AtomicUsize, live, max_seen }` accepts a `start` while `live < limit` and otherwise fails with a chosen `Failure` (429 with `Retry-After`, reset, refused, stall). `set_limit` changes the limit mid-test, standing for another client arriving and leaving. It exercises both endpoints' admission, the retry machine and the governor together, with no network.
3. **Real protocol fixtures.**
   - **HTTPS:** extend `https_fixture::Server` (`src/git/endpoint/https_fixture.rs:62`, which already keeps a `connections` counter) into `LimitedServer`: it counts live connections from accept to close, keep-alive idle ones included, and at its limit either answers the discovery GET 429 with `Retry-After`, accepts then resets, closes before the TLS handshake, or stalls. It records the most connections it ever held at once, the refusals, and the order of events.
   - **SSH, scripted:** the one-connection `dropping_server` of `ssh_tests/max_startups.rs` generalised to count concurrent pre-banner connections and drop beyond a limit, reset or end-of-stream, with and without the pre-banner line. It cannot authenticate, so it tests classification and discovery of the setup limit only.
   - **SSH, real:** `SshdFixture::with_startups` (`ssh_fixture.rs:49`) with a `MaxStartups start:rate:full` line, `PerSourcePenalties no` as the existing test sets it. The `start:rate:full` form drops probabilistically, so a 100% rate just past `start` makes a deterministic limit (to verify against the OpenSSH on the CI legs). The fixture's default today is `MaxStartups 64`, set so the transport's 32 setups do not hit it (`ssh_fixture.rs:24-27`).

### 10.2 Cases

Each case asserts the outcome, the maximum concurrent connections seen, the refusal count and the elapsed time on the injected clock.

1. **Limit discovery, HTTPS Throttle.** Server limit 8, ceiling 32, 32 members, 429 with `Retry-After: 1`. All 32 succeed; `L` ends at 8; after the first window the maximum live connections is at most 8; each member used 2 to 3 attempts; no member failed.
2. **Limit discovery, ambiguous.** The same with a reset in place of the 429. The same outcome within two windows: the confirmation rule, not a single failure, lowers `L`.
3. **SSH.** Scripted server and real sshd, limit 10 against a first wave of 32. All members succeed; `L` converges near 10 within two windows; with `MaxStartups` probabilistic, `L` never exceeds the highest `G` seen and never falls below 1.
4. **A one-off failure.** One reset among 32 attempts, and a second test with one reset after `G` was reached at a higher concurrency. `L` stays `C`; the member retries via the retry machine; no note is printed.
5. **Recovery.** Server limit 4 for the first 10 s of injected time, then 16. `L` falls to 4, then rises one per probe interval up to 16 (and never above `C`), the probe gap doubling after any probe that is refused. A second case where the limit rises to 64 with `C = 32` stops at 32.
6. **Budget exhaustion.** A server that answers 429 always. Every member fails `Capacity` with the `throttle` detail naming the host, `limit_observed = 1`, `attempts = 4`, `last_signal = http_429`; no member ever receives bare `Capacity`; total injected time is within the §5.3 bound.
7. **Retry-After beyond budget.** `Retry-After: 3600`: members fail at once with the retry-after text; nothing sleeps beyond the 30 s hold cap.
8. **`--max-retries 0`.** The first Throttle fails the member; a Queue wait still waits.
9. **More than 64 HTTPS repositories.** 200 distinct repository URLs on one fixture host with `--jobs 100`: all succeed, none reports `Capacity`, and after the operation the route tables and `https_routes` hold no entries. A second run with a limit of 8 and 200 members. A test that holds more than 64 routes live at once and releases them (7.2).
10. **Local capacity.** The job budget filled by abandoned jobs through a test hook (64 permits held): opens wait and succeed after release; none fails with `Capacity`. A wait that expires reports a local throttle, and the retry machine's counter and the governor's strikes do not move (the §7.5 mislabel is pinned shut). `HttpConnector` with one setup slot and 32 connections succeeds.
11. **The floor.** A server limit of 1: all members complete serially, and `L = 1`.
12. **Stale windows.** 24 refusals from one window cause exactly one decrease and one fit.
13. **Separate keys.** A throttle on SSH to a host leaves that host's HTTPS limit at `C`.
14. **Cancellation during a hold** cancels the queued members and starts no probe.
15. **The `--transport native` path** is untouched: a fixture-limited fetch fails members as 1.0.17 does, and the test pins that no governor is constructed.
16. **gwz-py**, one row in the candidate CI: a limit-8 fixture fetch of 32 members passes through the Python client and its failure message names the host.
17. **Windows.** The drop's error kind is pinned on Windows, and `ConnectionAborted` is Suspect where it is a drop (F3).

### 10.3 Live check plan

Run only on the operator's go, from a dedicated machine, with the evidence archived in `gwz-core-evidence` like the TR8.1 runs.

1. **Anonymous HTTPS to a public repository set**, 1.0.17 and the candidate, a ramp of concurrent connections of 8, 16, 32, 64 and 128 (one connection per repository, repeated clones of a public set), recording each refusal's status, `Retry-After`, and its kind. This answers §2.4(a): whether github.com's smart-HTTP endpoint refuses concurrent connections at all, and with what.
2. **The same over SSH** at the same ramp, with the operator's agreement: the account's key is involved, and a sustained refusal can affect the account. Recording the pre-banner line if the client is made to read it (F2). Answers §2.4(b), (c).
3. **A second client from another host** (the Linux host) holding a fixed number of connections while the first runs, to confirm the limit is shared across addresses or accounts and that the adaptive run finds the lower figure and restores it when the second stops.
4. **Parity row.** The TR8.1 32-member fetch at defaults, no throttle, must show no governor event and no extra connection, and the same time as without the governor.

## 11. Parity with 1.0.17

- **Under a server limit,** 1.0.17 has no throttle handling. It opens one connection per member (32 of 32 whatever `--max-per-host` is, Linux and macOS READMEs), a refused or stalled connect fails that member (`failed to connect to github.com: Operation timed out`, 9 of 54 runs), and it does not retry. A 429, were GitHub to send one, would surface as a libgit2 HTTP error for that member (an assumption: not measured). The candidate is strictly better: it queues instead of failing, and adapts.
- **Without a limit,** the candidate does what 1.0.17 does at the same `--max-per-host`, with no added wait, because `L = C` and the governor is idle.
- **No 64-repository limit** in 1.0.17; the candidate has none after §7, which closes a regression.
- **Where the candidate cannot match 1.0.17:** none by design. A throttled receive-pack or upload-pack POST fails the member, as in 1.0.17 (§5.4), but with a named error.

## 12. Facts the lane owner should know

- **F1. `Capacity` was local, not GitHub.** The TR8.1 HTTPS failures were the connector's 8 setup slots (fixed in 279860c). 1.0.17 opened 32 connections to github.com with no failure, so the repository's evidence contains no GitHub concurrency limit yet.
- **F2. The `MaxStartups` line is not surfaced.** The client reads an error kind, not the server's one line, so SSH drops are Suspect, never Throttle. Reading the pre-banner text before libssh2 would make "Exceeded MaxStartups" a definite signal, at the cost of owning the banner read. Not assumed.
- **F3. Windows is unpinned.** `ssh_tests/max_startups.rs` is `cfg(unix)` throughout. `ssh_setup.rs:437` maps `ConnectionAborted` to `Cancelled`, which the retry classifier returns to the member. If Windows reports a `MaxStartups` drop as `ConnectionAborted` (WSAECONNABORTED), such a drop fails the member today.
- **F4. A local wait looks like a server stall.** `https_connection.rs:246-250` produces `Timeout` + `SetupFailureCause::Aggregate` when a connection still waiting for a setup slot passes its connect deadline (the connect stage's own deadline, `:318`, is a real server wait and is correctly Aggregate). The classifier retries that as a server stall, in `Phase::Setup`.
- **F5. A throttle makes the 64-job budget reachable.** Stalled setups hold a permit and a server connection until their thread returns (`agent_job.rs`). `open_ceiling` (32) holds this off in normal use.
- **F6. The 64-job budget and `Hub` are process-wide statics** (`agent_job.rs:14,350` and the `HUB` static at `:144`). Counted across endpoints and operations. The operator's no-globals rule applies; see OQ9.
- **F7. The route tables' lifetime is deliberate** (`https_operation.rs:1-2`, `https_endpoint.rs:~237`), its reason unrecorded. §7.2's release depends on it.
- **F8. Credential material lives for the whole command today,** per route (`https_policy.rs:127-133`, `Routes`), which the 64 cap incidentally limited and §7.2 shortens.

## 13. Open questions for the operator

- **OQ1. The wire shape of the final error.** `Capacity` plus an optional `throttle` detail (recommended), or a new `ErrorCode::Throttled`. A new enum value breaks an older reader's decode (`DecodeError::UnknownEnum`), and the detail field does not. **Recommended: `Capacity` + detail.** A distinct model-level status for scripts can follow with TR2.24.
- **OQ2. Should the member scheduler read the learned limit?** Without it, parked members hold `--jobs` threads and can starve other hosts under a small `--jobs`. With it, `par_map_per_host` starts fewer members of the host, which across the CLI carrier needs a small message. **Recommended: yes, as a read-only limit view the scheduler polls**, and endpoint-only queueing stands without it, so it can follow.
- **OQ3. A 429 or 503 on an upload-pack POST.** The exchange cannot be replayed. Options: report it once (as designed here), or re-run the whole read-only member (fetch, a clone's fetch, a pull's fetch half) when it fails throttled with `Effect::None`. Never a push. **Recommended: re-run read-only members once the rest lands,** since it is the only path by which a throttle still fails a member while budget remains.
- **OQ4. Does a GitHub 403 with `Retry-After` mean throttled?** It maps to `RepositoryRefused` today; misreading it would mark a private repository as throttled. **Recommended: no, until the live check (§10.3) shows GitHub sends it, and then only when `Retry-After` is present.**
- **OQ5. Persistence in the 1.2.0 session host.** **Recommended: none.** If a host later carries a hint, it expires with the 60 s idle timeout and never exceeds `C`.
- **OQ6. The constants.** Initial probe gap 5 s doubling to 60 s, a hold cap of 30 s, additive probing of 1, the fit at window close. **Recommended: ship these as initial values and tune from §10.3**; they are named constants in the governor, not flags.
- **OQ7. The note.** Print by default in human mode (§9), or only under `--verbose`. **Recommended: by default**, since the operator wants to see "limited to 8", it appears once, and it never prints when nothing was throttled.
- **OQ8. Requeues and `--max-retries`.** The operator's direction is that they share the budget (§5.3). With the default 3, a limit that must be discovered from five halvings of the ceiling would exhaust it, and the fit (§4.2) makes discovery one or two windows. **Recommended: share the budget, as directed,** and revisit only if the live check shows members failing at the default.
- **OQ9. The process-wide statics.** `agent_job.rs`'s job budget (`COUNT`, `CLEANUPS`) and `Hub::global` are globals, against the operator's rule. Making a refusal a wait (§7.5) does not change that. **Recommended: raise it as structural debt for its own decision** and keep this design's wait semantics independent of where the counter lives.
- **OQ10. Route lifecycle, 7.1 and 7.2.** Remove the `https_routes` cap and release routes when their last dependent ends, if the lifetime reason (F7) allows; otherwise remove both caps and release at the request's end. **Recommended: confirm F7 with whoever wrote the "routes outlive a dropped remote" comment, then release per route.**
- **OQ11. The governor's unit.** One limit on all connections to the key (recommended: simple, conservative for `MaxStartups`), or a separate limit on concurrent setups. **Recommended: one limit; separate setup limits only if §10.3 shows SSH leaving connections unused.**

## Changelog

- 2026-10-06: revision 0, DRAFT, not reviewed. Written from the operator's direction of 2026-10-06 and the code at gwz-core `279860c1`, with the TR8.1 runs of 2026-10-06. No code, test or plan was changed.
