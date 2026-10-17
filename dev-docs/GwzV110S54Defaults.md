# 1.1.0 S5.4: the measure-then-select note for the transport's construction defaults

Date: 2026-10-17. Step: S5.4 (`GwzV110Plan.md` Phase 5; `GwzV110RemainingPlan.md` B7). Sources read at gwz-core `4f4b7e01`, gwz-transport `a27817b`, gwz-cli `279bf85`, gwz-core-evidence `a998f19f` (lane `s54`). Every `file:line` below is at those revisions.

**Outcome: no default changes.** Every value below is kept. For most of them the evidence is silent (the runs never moved the value, or never reached the code it bounds); for the rest it supports the current value. No product code, test or script was changed by this step, so there is no Windows receipt. The note records what would need to be true for a value to move, and which of the pending inputs could move it.

## 1. Scope, rules and the question A2 asked

**Deliverable.** This note. **Repositories that could have changed:** gwz-core, gwz-transport (report first). **Reference only:** the evidence runs, the plans, gwz-cli's help text.

**Rules applied.**
- `GwzV110Plan.md:439-441`: S5.4 does not change the 60-second idle default, and adds no frozen hard cap. Wire fields, tags and hard ingress caps stay (`GwzRemoteTransportPlan.md:359-360`: a changed hard cap is a design amendment). The stall default is 9 s and the aggregate default 30 s, per `GwzRemoteTransportRetryPlan.md:104-127`.
- `GwzTransportReleasePlanAmendment-2.md:264-268`: the defaults wait for TR8.1, TR8.4 and S5.3, and a 1.1.0 miss is retuned by S5.4 first.
- Operator rule for this step: change a default only where the measurements clearly favour it; where they are silent, keep it and say so.

**A2's question: did a retune close the TR8.1/TR8.4 miss?** No default was found that does. The misses are an SSH fixed cost per command (+0.008 s at 32 members on Windows, +20 to +30 ms, accepted by D29) and an HTTPS per-member cost (+4.7 ms per member on Windows, +0.165 s at 32; Pi +0.06 s at 32 after the fixed-cost fix). Both are costs of work done, not of a limit or a timeout: the HTTPS rows are unchanged by `--max-per-host` 32 against none (run `2026-10-16-tr84speed`), and unchanged with the adaptive limit off (`--max-retries 0`, run `2026-10-13-tr8-1-rerun`). The one construction candidate that could touch the HTTPS per-member cost is named in §3.7 and goes to step 6.1's profile (D29), not to this step. `GwzV110RemainingPlan.md:119` ("none is recorded") is answered by this paragraph: S5.4 looked, found no default to retune, and OD17 stands as the operator took it.

## 2. What the evidence covers, and what it cannot

| ID | What it measured | Bears on |
|---|---|---|
| `2026-10-04-tr8-1-macos-parity`, `2026-10-06-tr8-1-linux`, `2026-10-06-tr8-1-macos-https`, `2026-10-07-tr8-1-linux-https-rerun` | no-op fetch, 32/16/1 members, candidate against 1.0.17, with `--max-per-host` 4, 8, 16, 32 and the build defaults; SSH to GitHub (Mac), HTTPS to GitHub (Pi, Mac) | per-host ceiling, setup slots |
| `2026-10-06-tr8-1-ssh-gap-fix`, `2026-10-07-tr8-1-ssh-gap-32`, `2026-10-09-ssh-syn-loss` | SSH fixed waits (`db0f8447`), the endpoint coalescing window, 1.0.17's FIN burst | coalescing, connect |
| `2026-10-07-https-fixed-cost-profile`, `-fix`, `2026-10-13-https-fixed-cost-pi`, `2026-10-14-https-fixed-cost-fix` (+`-merged`, `-rem1`: gate logs) | where the HTTPS fixed cost goes on the Pi; 8 against 64 setup slots; the shared TLS connector | setup slots, runtime threads |
| `2026-10-13-tr8-1-rerun` | the TR8.1 rows on current main, Pi and Mac, SSH and HTTPS; an adaptive-off row (`--max-retries 0`) | per-host, adaptive machine |
| `2026-10-15-s51` | workspace fetch on Linux x86-64; push plus post-push reads; 400 MiB clone and 128 MiB push; connect counts | per-host, retry, buffers |
| `2026-10-15-s52` | coalescing windows 0, 10, 50, 100 ms, client end and both ends, SSH and HTTPS, two platforms, connect counts, bulk throughput | coalescing |
| `2026-10-15-s53` | process RSS under a slow or stalled reader and a flat-out writer, 22 MiB to 1.6 GiB, two platforms | buffers, backpressure, stall |
| `2026-10-16-tr84speed` | TR8.1's three targets on Windows against 1.0.17; connect counts; partial results | per-host, retry |
| `2026-10-16-winflakes` | 77,058 Windows establishes, the key-exchange failure rate (D14, D32) | retry |

**Limits that apply to every row below.**
1. No workspace in the campaign had more than 32 members. `--jobs` 100 and the pool totals were never the binding limit. `--max-per-host` 32 was the binding limit only where a run set a lower value.
2. The SSH rows are a local sshd (S5.1, TR8.4) or GitHub over a home network (Mac, Pi); the HTTPS rows are anonymous GitHub. Nothing was measured over a high-latency or lossy path, and no host refused a connection on the candidate rows, so the adaptive machine's refusal path ran in no S5 or TR8 row.
3. Every S5 and TR8 row ran the CLI in process. No long-lived reuse, no server (1.2.0).
4. No run varied a buffer size, a timeout, a retry count or an adaptive parameter. The only construction value any run varied is `--max-per-host` (and, in the Pi profile, the HTTPS setup slots, in a scratch build), plus the coalescing windows through the S5.2 probe.

## 3. Every default

Format: current value (file:line), measurements, selected value and reason. "Silent" means the evidence does not move the value either way.

### 3.1 Concurrency: `--jobs` and `--max-per-host`

| Default | Current | Selected |
|---|---|---|
| `--jobs` / `DEFAULT_JOBS` | 100 (`gwz-core/src/operation/resolve_jobs.rs:2`); 50 only under `--transport native` (`gwz-cli/src/globalargs/transport_help.rs:37`) | keep 100 |
| `--max-per-host` / `DEFAULT_MAX_PER_HOST` | 32 (`gwz-core/src/operation/resolve_per_host.rs:2`); 8 only under `--transport native` (`transport_help.rs:39`) | keep 32 |
| pool `per_user_host`, `per_host` | 32, 32 (`gwz-transport/src/pool/mod.rs:50-51`); a request installs `per_user_host = per_host = --max-per-host` (`gwz-core/src/transport_host/mod.rs:415-423`) | keep 32 |
| pool `total`, `max_requests` | 256, 1024 (`pool/mod.rs:52-53`); a request installs `jobs.max(256)` and `jobs.max(1024)` (`transport_host/mod.rs:424-425`) | keep |

**`--max-per-host`, measurements (all candidate against 1.0.17, 32-member no-op fetch):**
- Mac, SSH to GitHub (`2026-10-04-tr8-1-macos-parity`, run 1, three rounds): candidate 2.71 s at its default 32, 7.70 s at `--max-per-host 4` (4 connections); 1.0.17 at its own default of 8 took 8.56 s.
- Pi, HTTPS to GitHub (`2026-10-07-tr8-1-linux-https-rerun`, before the fixed-cost fix): candidate 0.70 s at the default 32, 0.75 s at explicit 32 and at 16, 1.19 s at 8, 2.03 s at 4. Connection counts follow the setting (32, 16, 8, 4).
- Windows (`2026-10-16-tr84speed`): the candidate with no flag and with `--max-per-host 32` agree within the spread (SSH +0.022 against +0.008 s; HTTPS +0.122 against +0.165 s over 1.0.17). One connection per member at 16 and 32 members on both protocols.
- S5.1: 16-member push, 16 connections for the candidate and 32 for 1.0.17 (the candidate reuses the advertisement connection); post-push reads 16.

**Reading.** 32 is at least as fast as any lower value on every row that varied it, about 2 to 4 times faster than 8 and 4 against GitHub, and 16 against 32 is a tie on the one row that has both. The 8 that 1.0.17 used was a measured loser (8.56 s against 2.71 s). The candidate has not failed a member at 32 on any row since `279860c` (0 failed members, TR8.4 SSH 32 and HTTPS 32, 24 rounds each, and 10 attempts of init plus three rounds), where 1.0.17 lost members at 32 on a local sshd (S5.1 finding 4; TR8.4: 5 of 24 SSH rounds at 32).
**Selected: keep 32.** 8 is wrong for the candidate on the evidence; 16 is not separated from 32; above 32 was not measured (no workspace had more than 32 members), so the ceiling stays where the retry plan put it. The adaptive machine (§3.8) lowers the number used when a host refuses, so a ceiling of 32 does not trade safety for speed.

**`--jobs`.** Silent. The largest fan-out measured was 32 members on one host, so the global ceiling of 100 never bound. **Keep 100.** The pool totals (256, 1024) follow from it and are silent for the same reason.

### 3.2 Pool idle timeout

| Default | Current | Selected |
|---|---|---|
| `idle_timeout_ms` | 60,000 (`gwz-transport/src/pool/mod.rs:54`) | keep |

A non-goal of the release (`GwzV110Plan.md:439`). In 1.1.0 a command is one process and no connection outlives it, so no row could have measured an idle timeout; the reuse rows are TR8.2 (1.2.0). **Keep 60 s; silent.**

### 3.3 Connect, setup and allocation timeouts

| Default | Current | Selected |
|---|---|---|
| pool `connect_timeout_ms` (the setup's aggregate budget, SSH and HTTPS) | 30,000 (`pool/mod.rs:56`); the host copies it (`gwz-core/src/transport_host/session.rs:475`) and zero clears it when `--ssh-timeout 0` (`transport_host/mod.rs:132-136`) | keep |
| pool `allocation_timeout_ms` | 30,000 (`pool/mod.rs:55`; SSH `session.rs:476`) | keep |
| pool `interaction_timeout_ms` (a helper phase) | 120,000 (`pool/mod.rs:57`; `session.rs:477`; `session/driver.rs:45`) | keep |
| pool `cleanup_timeout_ms`, `OPEN_CLEANUP_MS` | 5,000 (`pool/mod.rs:58`; `session/driver.rs:39`) | keep |
| `OPEN_ATTEMPT_MS` (the open backstop's per-attempt allowance) | 150,000 (`session/driver.rs:37`) | keep |

**Measurements.** Every successful setup on the S5 and TR8 rows finished far inside these budgets: a local sshd login costs milliseconds, GitHub's SSH login about 2 s (a one-member SSH fetch is 1.96 to 2.07 s on the Pi and Mac in `2026-10-13-tr8-1-rerun`; the retry plan cites a 2.0 to 3.5 s handshake, `GwzRemoteTransportRetryPlan.md:104`), HTTPS setup tens of milliseconds, with a DNS stall of up to 5 s seen on the Pi (`2026-10-14-https-fixed-cost-fix`: 15 of 100 rounds dropped for it). No candidate row timed out (0 failed members across TR8.4, S5.1 and the Pi/Mac reruns). So the budgets do not bind on a healthy path, and the evidence has nothing to say about a path where they would. **Keep all; silent.** The values were set by the retry plan (`GwzRemoteTransportRetryPlan.md:104-127`), not by a measurement S5.4 could replace.

### 3.4 I/O (stall), aggregate, and the native timeout

| Default | Current | Selected |
|---|---|---|
| `--ssh-timeout` (the stall clock, SSH setup and body reads on both protocols) | 9 s (`gwz-cli/src/lib.rs:211` and `:220`); the transport runtime's copy 9,000 ms (`transport_host/session.rs:474`); the process-wide default before the CLI sets one `DEFAULT_SERVER_TIMEOUT_MS` 9000 (`gwz-core/src/git/gitbackend/transport_support.rs:9`) | keep 9 s |
| a standalone stream's `io_timeout_ms` | 3,000 (`gwz-transport/src/stream/mod.rs:78`); overridden to the product stall on the CLI's streams (`gwz-core/src/git/endpoint/ssh_worker.rs:262`, `https_local.rs:146`) | keep (not a product default) |
| stream `close_timeout_ms`, `interaction_budget_ms` | 5,000 and 120,000 (`stream/mod.rs:77`, `:79`); the CLI's SSH streams take them from the open's deadlines (`ssh_worker.rs:262-264`) | keep |
| `--transport native`'s timeout | 3 s in the CLI help (`transport_help.rs`), `DEFAULT_SERVER_TIMEOUT_MS` 9000 before the CLI sets it | out of scope (native route; no S5 row) |

**Measurements.** S5.3 is the only run that exercised the stall clock: a reader paused 2 s survived, a reader paused 8 s failed with `StreamStall`, and the same 8 s pause with the limit at 30 s survived (`2026-10-15-s53`, harness rows `stall2s-default-limit`, `stall8s-default-limit`, `stall8s-limit30s`; macOS and Linux). A probe for a real libgit2 pause (a clone of a 1.5 GiB zero blob that compresses to 7 MB) found no failure in 14 clones.

**Correction to the S5.3 record.** The S5.3 README says the stall limit is "3 s by default" and calls the 8 s failure "the exception worth a decision". The harness takes its limit from `S53_IO_TIMEOUT_MS` with a fallback of 3000 and a comment saying that is "the product's" (`2026-10-15-s53/runner/s53_harness.rs:7`, `:43`, `:126`). It is the test fixture's value (`transport_host/mod.rs:178`), not the product's: the CLI's default is 9 s (table above; `GwzRemoteTransportRetryPlan.md:122-127` moved it). The 8 s failure therefore happened at a 3 s limit the product does not ship; at the shipped 9 s an 8 s pause is inside the limit, and the 30 s row shows the stall clock is what bound. The run was not repeated at 9 s. The remaining concern is a consumer pause longer than 9 s, which no CLI flow showed; that is a behaviour of the stall clock (it charges a consumer's pause to the peer), not of its default, and it stays as S5.3 recorded it.
**Selected: keep 9 s.** The only measurement of the setup side is the handshake the retry plan cites, 2.0 to 3.5 s (`GwzRemoteTransportRetryPlan.md:104`), which 9 s clears by 2.5 times where 1.0.17's 3 s did not. Nothing paused near 9 s on the body side, so the evidence does not support raising it or lowering it. Otherwise silent beyond the correction above.

### 3.5 Retry budget

| Default | Current | Selected |
|---|---|---|
| `--max-retries` / `DEFAULT_MAX_RETRIES` | 3 (`gwz-core/src/git/endpoint/setup_retry/operations.rs:9`; `transport_host/request.rs:537-543`; `gwz-cli/src/globalargs/retry.rs:17`) | keep 3 |
| first wait, cap, jitter | 1,000 ms doubling, 30,000 ms, 0 to 249 ms (`setup_retry/backoff.rs:5-8`) | keep |
| a Down key's retest interval and give-up count | 30,000 ms, 2 (`setup_retry/machine.rs:85`, `:87`) | keep |
| key-exchange retry, Windows only | `KexRetry::PLATFORM` (`git/endpoint/ssh_network/kex_retry.rs:77`), one retry today (D14), two under D32 | not this step (D32) |

**Measurements.**
- S5.1 finding 4: a burst of 16 or 32 simultaneous SSH logins against a stock `sshd` (`MaxStartups` begin 10, full 64) lost members in 1.0.17 (2 of 16 at 32 per host, 5 of 16 at 16), and the candidate's setup retry absorbed it. That is a retry doing its job, once or twice per member; it does not reach a fourth attempt.
- TR8.4 (`2026-10-16-tr84speed`): 1.0.17 lost 6 of 960 member fetches (0.6 %) to the WinCNG key-exchange drop; the candidate lost none in 24 timed rounds plus 30 count-and-partial rounds per row, and made one connection more than members in 2 of 6 runs (33 of 32), the D14 retry.
- `2026-10-16-winflakes`: 0.75 % of 77,058 Windows establishes failed the first key exchange, D14's one retry left 2 of them (about 1 in 38,500).

**Reading.** The evidence shows a retry of one or two helps and is cheap, and shows no failure that needed the third or the fourth attempt. It does not separate 3 from 2 or 4: no row failed three times in a row on the candidate. **Selected: keep 3** (the operator's 2026-10-07 rule that tests and requeues share the budget makes a lower value costlier than it looks: `GwzTransportAdaptiveConcurrencyDesign.md:400`). Silent on the waits.
**Flag (D32).** D32 raises only the Windows key-exchange retry, which is inside one attempt (`ssh_network/observed.rs:4`) and does not touch `DEFAULT_MAX_RETRIES`. The open backstop (`session/driver.rs:28-36`) is written for `R + 1` attempts of `OPEN_ATTEMPT_MS`; whether a second key-exchange retry inside an attempt stays inside it is for the `d14two` lane to confirm. Nothing here moves with D32.

### 3.6 Coalescing window (S5.2)

| Default | Current | Selected |
|---|---|---|
| stream `coalesce_delay_ms`, the default | 100 (`gwz-transport/src/stream/mod.rs:76`) | keep |
| SSH bridge, client end | 100, the default (`gwz-core/src/git/endpoint/ssh_worker.rs:252-272`, the override only on the endpoint side) | keep |
| SSH bridge, endpoint end | 0 (`ssh_worker.rs:271`, since `db0f8447`) | keep 0 |
| HTTPS bridge, both ends | 100 (`git/endpoint/https_local.rs:146`, one `Config::new` cloned for both) | keep |

**Measurements (`2026-10-15-s52`).** Immediate emission (0) against 10, 50 and 100 ms at the client end, and at both ends, SSH and HTTPS, 16-member push and reads, no-op fetch, 400 MiB clone, 128 MiB push, macOS and Linux, 10 rounds for the small operations and 3 for the large; connect counts in every configuration; bulk throughput of the stream itself.
- Client end: `c100` minus `c0` is within +/-8 ms on commands of 0.09 to 0.5 s, with both signs; the large operations differ by 0.01 s (Linux clone) to 0.15 s of 17.7 to 18.4 s (push); bulk throughput is flat (Linux read 176 to 182 MiB/s, write 142 to 149; Mac 257 to 272 and 165 to 166) across all nine configurations.
- SSH endpoint end: `b100` costs 0.10 s per advertisement exchange on both platforms (no-op fetch +0.103 s; push, two exchanges, +0.19 to +0.20 s), `b50` about 0.05 s, `b10` 0.01 s. That is the defect `db0f8447` removed; the shipped endpoint value 0 is on the good side.
- HTTPS: `b100` minus `b0` is +0.00 to +0.03 s, inside the noise, because the HTTPS endpoint flushes its responses.
- Connect counts: 16 per 16-member command and 1 per clone, in all nine configurations on both platforms.

**Reading.** Nothing the CLI does releases a frame on the window's deadline: every exchange ends, flushes or reads, so the window is a no-op everywhere it was measured, and costs its length wherever a forced send is missed (the pre-`db0f8447` SSH endpoint). The one place a window could help, a burst of small writes with no read between, was not constructed, and the CLI's Git flows do not produce it. **Selected: keep every value.** The measurements do not favour a change; they favour no particular value except the endpoint-end 0, which is already set.

**Recorded, not done (needs an operator decision).** Both ends of an in-process bridge share one process, so there is nothing to coalesce for (the comment at `ssh_worker.rs:266-270` says so for the endpoint end). Setting the client ends and the HTTPS ends to 0 would remove a latent 100 ms cost from any future missed force-send at no measured loss. It is a behaviour-neutral change on current evidence, so it is not made on this step's rule. It also needs the windows as fields of the endpoint's construction config rather than a literal in each bridge: S5.2's probe through an environment variable was refused by `check_process_globals.py` (`2026-10-15-s52/proposal-attempt/`), which is correct, and an S5.2 re-run after such a change would need that field first.

### 3.7 Buffers, windows, payloads and backpressure (S5.3)

| Default | Current | Selected |
|---|---|---|
| stream `send_buffer`, `receive_window`, `peer_receive_window` | 65,536 each (`gwz-transport/src/stream/mod.rs:72-74`); the SSH bridge caps the negotiated window at 65,536 (`ssh_worker.rs:258-259`); the session host's `limits()` sets 65,536 (`gwz-core/src/transport_host/session.rs:69-74`) | keep |
| stream `max_payload` | 16,384 (`stream/mod.rs:75`; `ssh_worker.rs:260`) | keep |
| stream `max_waiters` | 64 (`stream/mod.rs:80`) | keep |
| negotiated limits (`default_limits`) | frame 128 KiB, Data payload 64 KiB, queued 4 MiB and 64 frames, receive window 1 MiB, control reserve 64 KiB and 8 frames (`gwz-transport/src/binding.rs:4-17`) | keep (these are the frozen caps) |
| mux | `max_requests` 256, `max_streams` 64, bootstrap 5,000 ms, cleanup 5,000 ms, `max_check_ms` 120,000 (`gwz-transport/src/mux/mod.rs:49-53`) | keep |
| session host | `MAX_HANDOFFS` 64, `MAX_STREAM_MESSAGES` 8, `CLEANUP` 5 s, `PARK` 5 ms (`transport_host/session/driver.rs:11`, `:16`; `session.rs:48`, `:51`) | keep (1.2.0 path) |

**Measurements (`2026-10-15-s53`, `2026-10-15-s52` bulk rows).** The client process's own RSS, with no fixture in it: 30.9 to 32.5 MiB (kernel peak) on Linux and 26.7 to 29.0 MiB on macOS in every read run (50 to 400 MiB, reader at 20 and 2 MiB/s, stalls); 29.0 to 30.0 MiB (Linux) and 22.5 to 22.7 MiB (macOS) in every write run (22 MiB to 1.63 GiB, sink at 1 and 8 MiB/s or fast). Thirds of a run within 0.6 MiB on Linux; no growth with size (50 MiB read 31.6, 400 MiB read 31.3). Backpressure was real: a 400 MiB read at 2 MiB/s for 64 MiB took 32 s against a 2 s unthrottled read. The CLI series: the transfer phase is flat in both builds; the end-of-command peaks (about the pack's size plus 25 to 35 MiB on a clone, 100 to 250 MiB while libgit2 builds a pack) are libgit2's in both builds. Throughput through the stream: read 176 to 182 MiB/s (Linux), 257 to 272 (Mac); write 142 to 149 and 165 to 166; neither varied with any setting measured.

**Reading.** Memory is bounded and independent of transfer size at these values, which satisfies S5.3. No run varied a buffer, window or payload size, so the evidence cannot say that a different size would be better. The one throughput gap in the campaign is HTTPS bulk on quiet Linux (400 MiB clone +17 %, 3.03 s against 2.60 s, 5 of 5 rounds; the 128 MiB fetch +20 %; S5.1 finding 2); on macOS it is at parity. S5.1 names a per-chunk path as a hypothesis and did not test it. A 16 KiB payload (`max_payload`) is a candidate cause by size alone (more frames per MiB), but no measurement connects them. **Selected: keep every value.** Recorded for the operator in §5.

### 3.8 The adaptive limit machine

The machine is `git/endpoint/setup_retry/limit/*` (design: `GwzTransportAdaptiveConcurrencyDesign.md`, revision 11, §4 and §5.5).

| Parameter | Current | Where | Selected |
|---|---|---|---|
| start state and `N` | SATURATED at `N = C`, `C` = the resolved `--max-per-host`, so the first wave is parallel (OD18) | `limit/fsm.rs:31-35` | keep |
| SSH setup limit `Ns` | starts at `C` (design §4.10) | `limit/control/setup_limit.rs`, `limit/control.rs:125` | keep |
| probe timer | `T0` 500 ms, `Tmax` 30,000 ms, jitter factor 0.8 to 1.2 | `limit/timer.rs:6-7`, `:23`, `:26` | keep |
| settle time `Ts` | `max(250 ms, 2 x SRTT + 100 ms)` | `limit/states.rs:50`, `:55-65` | keep |
| `Retry-After` hold cap, idle-discard threshold | 30,000 ms, 1,000 ms | `limit/hold.rs:4`, `:6` | keep |
| restore step | judged doubling toward `N_good` | `limit/control/restore.rs:55` | keep |
| notes kept, local-wait freshness, kept waits, ended-setup TTL | 64, 10,000 ms, 64, 1,000 ms | `limit/governor.rs:69-77` | keep |
| key retest and give-up | 30,000 ms, 2 failed retests | `setup_retry/machine.rs:85`, `:87` | keep |
| setup-job budget | 64 | `git/endpoint/agent_job.rs:10` | keep |

**Measurements.** None of these parameters acted in a timed S5 or TR8 row. On the happy path the machine stays SATURATED and adds no wait, no test and no connection, which the rows show: connection counts equal member counts everywhere, and on the Pi `--max-retries 0` (adaptive off) gives 0.461 s against 0.468 s with it on at 32 members, so the machine costs nothing measurable when idle (`2026-10-13-tr8-1-rerun`, attribution step). The refusal path (`T0`, `Tmax`, `Ts`, holds, restore) ran only in the machine's own fixture tests (design §10), where the design set `T0` from an SSH login it measured at 0.5 to 1 s (§4.7). **Keep every parameter; silent**, with one gap stated: S5.1's stock-sshd burst (finding 4) is the only real-server event on the candidate, and the run does not record whether it moved `Ns`.

### 3.9 HTTPS and SSH per-host limits and slots

| Default | Current | Selected |
|---|---|---|
| per-host connections, SSH and HTTPS | the same 32 (§3.1). The pool keys SSH by user and host and HTTPS by host. | keep |
| HTTPS setup slots (resolver and TLS-configuration jobs) | 8 (`git/endpoint/https_pool.rs:53`) | keep |
| HTTPS request slots, helper slots | 64 and 8 (`git/endpoint/https_worker.rs:118-119`); `HELPER_SLOTS` 8 (`https_auth.rs:34`, `:144`) | keep |
| SSH password-helper slots; key-snapshot slots and bytes | 8 (`ssh_password_helpers.rs:27`); 64 slots and 16 MiB (`ssh_key_snapshot.rs:57`) | keep |
| the HTTPS endpoint's runtime | one `current_thread` runtime on one thread named `gwz-https` (`transport_host/https_endpoint.rs:147-148`) | keep (see below) |

**Measurements.**
- Setup slots, 8 against 64 (`2026-10-07-https-fixed-cost-profile`, Pi, 4 cores, scratch build, before the connector was shared): 16 members, last GET sent at 203.1 ms with 64 slots and 201.7 ms with 8; wall difference against 1.0.17 +114.6 ms against a control of +167.7, +128.0 and +100.9 across groups. The profile's own rule: 16-member wall differences below about 35 ms are not resolved. After the fix that shared the connector, 64 slots added to "share + prebuild" read +38.0 ms against +61.3 ms ; with the supervisor wake as well, +18.7 ms pooled over 56 rounds against +32.2 ms for the four fixes over 88, again inside the unresolved band. So 8 against 64 was not separated on the Pi.
- The 2026-10-06 `Capacity` failure at more than 8 HTTPS members was a refusal where a queue was needed, not a bad slot count; `279860c` made it a queue, and 47 of 47 runs at 32 members were clean (`GwzTransportAdaptiveConcurrencyDesign.md:44`).
- The HTTPS per-member cost that remains: Pi, 32 members, CPU 229 ms against 161 ms for 1.0.17 and 2,526 voluntary context switches against 397 (`2026-10-14-https-fixed-cost-fix`), noted there as "one `gwz-https` runtime thread does every handshake"; Windows 4.7 ms per member (`2026-10-16-tr84speed`). Neither run varied the runtime's thread count or the slot count.

**Selected: keep.** The slot count was not separated and the runtime thread count was never varied. **This is the construction that could move, and step 6.1's profile (D29) is the right place to test it** (§4). It is not changed here because the evidence only shows that the work is per member and CPU-bound, and that the endpoint does it on one thread; it does not show that more threads or more slots would shorten the wall time, and a thread count above one changes the endpoint's ownership of its pool, which is not a default to flip on a hypothesis.

### 3.10 Other waits and caps in the same constructors

Kept, silent: `WAIT_FOR_CLOSE_MS` 250 (`ssh_worker.rs:187`); the pool's `validate` bound of 1 ms to 86,400,000 ms (`pool/mod.rs:64-75`); `stream::Config::validate` bounds (send buffer and windows at most 4 MiB, payload at most 64 KiB, waiters at most 1024; `stream/mod.rs:92-104`). These are the caps the defaults sit under; none was approached.

## 4. Pending inputs and what each could move

| Input | Defaults it could move | Status |
|---|---|---|
| Step 6.1's HTTPS per-member profile on dabeest (lane `https611`, D29) | HTTPS setup slots, 8 (`https_pool.rs:53`); the `gwz-https` runtime's one thread (`https_endpoint.rs:147-148`); HTTPS request slots, 64 (`https_worker.rs:118`); and, if it shows a bulk-path cost, `max_payload` and the stream buffers (§3.7). **No per-host, retry, timeout or adaptive default**: those were varied or ruled out above. | Do not wait. If the profile attributes the 4.7 ms to the runtime thread or the slots, the fix is a construction change on those two lines and is that lane's to propose, with the Windows receipt. |
| The D32 retry change (lane `d14two`) | `KexRetry::PLATFORM` (`kex_retry.rs:77`); the open backstop's arithmetic (`session/driver.rs:28-36`, `OPEN_ATTEMPT_MS`). `DEFAULT_MAX_RETRIES` does not move. | Do not wait. Confirm the backstop covers a second retry inside an attempt. |
| S5.5 / WPP 6.3 (B8) | None. It repeats TR8.1/TR8.4's speed targets and S5.1 to S5.3's rows on dabeest, "on the chosen defaults". With no change here, the chosen defaults are the ones TR8.4 already ran (`2026-10-16-tr84speed`: candidate, transport shape, release profile). | Its repeat of S5.1's push/reads/large packs, S5.2's windows and S5.3's memory is still to run; it needs no rebuild for this step. |

## 5. Observations outside this step's scope

Recorded for the operator; none is a prerequisite for the step and none was acted on.

1. **S5.3's stall exception is a harness default, not the product's** (§3.4). Confirmed from the harness source. Impact: the README's "possible product concern" holds only for pauses over the shipped 9 s. Suggested: a one-line correction in a later evidence note; no product change. Does not block.
2. **The in-process bridges still carry a 100 ms coalescing window with nothing to coalesce** (§3.6). Confirmed that no measured flow releases a frame on it; the cost of a missed force-send (0.1 s per exchange on SSH) was reproduced by S5.2's `b100`. Hypothesis, not tested: that a bridge setting of 0 is neutral everywhere. Suggested: windows as construction-config fields, then 0 on the in-process ends. Does not block.
3. **HTTPS bulk is 17 to 20 % slower than 1.0.17 on quiet Linux** (S5.1 finding 2), at parity on the Mac. Confirmed on one host; cause not isolated; `max_payload` 16 KiB is a hypothesis. Suggested: if the operator wants it closed, one dabeest-WSL run with the payload varied through a construction field. Does not block (the plan's targets are the no-op fetch rows).
4. **The `gwz-https` endpoint runs every handshake on one thread** (§3.9). Confirmed that it is one `current_thread` runtime; the link to the per-member cost is the Pi profile's attribution plus the Windows slope, not a test. Belongs to step 6.1.
5. **No row exercised the adaptive machine's refusal path against a real host** (§3.8). A limitation, not a defect: its parameters rest on the design's reasoning and its own tests. Suggested: if the operator wants evidence for `T0`, `Tmax` and `Ts` before the release, a controlled `MaxStartups` row on Linux (S5.1's fixture already has the knob). Does not block 1.1.0 on the plan's text.

## 6. What this step changed

Nothing in any repository except this file. No test was written (no behaviour changed); the gates for a code change were not triggered; no Windows receipt applies.
