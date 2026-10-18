# Adaptive step review (Code+State)

**Verified tuple (start and end):** gwz-core `e043d952aa95d5ea70faffcfa2ab85b0504afab4`, diffed against its parent `eb924c0e`; gwz-transport `77f89cdf20da6e69ab5bc91fd609918dbdcb6f04`. Both trees clean apart from the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`.

**How it was tested.** The candidate tree was prepared, copied with `cp -RL` to `scratchpad/review-adaptive/cand`, and built with `--cfg gwz_transport_candidate`. The implementer's suites pass: `setup_retry` 153; `placement_endpoint` 30; `https_endpoint` 29; `https_worker` 112 with 2 SSPI failures (below); `background_close` 20, three runs in a row. Eight `review_probe_*` tests were added to the copy (appended to `governor_tests.rs` and `requeue_tests.rs`); all 8 fail as predicted, output in `review-adaptive/probes.log`.

**The reported environment failures:**
- **`background_close` (both):** not caused by this change. Those tests drive `ssh_worker::Endpoint` directly and never begin an operation, so the governor has no scopes and `Observer::seen` does nothing; before this change it set the site limit to `per_host` on the first event, which equals the pool's own cap.
- **SSPI "external target required" (both):** environment. The assertion `!target.starts_with(…)` at `native/tests.rs:38` fails because the target directory sits beside the source tree.
- **`password_helpers`:** not examined.

**What holds:**
- The common path is unchanged in time and concurrency. With no throttle the key stays SATURATED: gate open, `room` always true, pool limit C, settle 0. The only additions are per-pass demand reports and per-event loops over the live scopes, both negligible.
- Lock order is host → governor → pool; no back-edge into a host or the governor was found.
- Hold propagation, and the hold kept after its operation ends, are correct.

## Findings

### P2-1 — A new operation's tables cannot see connections that existed before it began (architectural)
- **Root cause:** `Book::slot` gives each new scope an empty `Table` (`governor.rs:172-173`), and `Observer::seen` feeds only scopes already live (`governor.rs:383`). A connection that predates the operation, even one this operation leases, stays invisible to it.
- **Violated:** §4.1 and §4.3 (Connected/Possible and `S_hi` must be what the server may hold); the module's own claim that "each sees the same Connected and Possible".
- **Sequence (PROBE1):** operation `a` holds 20 answered connections; `b` begins, answers 2 of its own, and its third gets a 429. Result: `Overload{n:2}`; `b` sees 2 connected while `a` sees 22.
- **Impact:** a false decrease to about 2 while the server holds 22; the operation then climbs one connection per quiet test. Happens with overlapping requests (gwz-py), and with a later request on the same session that leases the previous request's idle connections.
- **Correction:** keep one connection table per site at governor level, maintained whether or not any operation is live; per-operation machines read that table, and their own windows open on it.
- **Test:** PROBE1, asserting `b.connected == 22` and no Overload; a variant where `a` ends with its connections idle and `b` leases them.

### P2-2 — Another operation's connection consumes an armed test (architectural, observer interface)
- **Root cause:** `seen(Started)` calls `take_armed()` in every scope (`governor.rs:392-395`). Over HTTPS, `answered`/`refused` judge only the reporting operation (`scoped.rs:87-90`, `113-123`).
- **Violated:** §4.7 (the carrier's own new connection is the test); §5.2 (waits bounded by the test's clocks).
- **Sequence (PROBE2):** the operation is STABLE at N=3 and arms a probe; a member of operation `b` starts the next connection and `b` answers it. The operation's test slot stays held, and its gate is still shut at 59 s and beyond, for as long as `b` keeps that connection.
- **Impact:** every member of the operation waits with its allocation clock stopped and no deadline, starved for the lifetime of the other operation's connection.
- **Correction:** carry the pool request's owner/operation in `Seen::Started`, and let only the arming operation's connection take its test.
- **Test:** PROBE2; the gate must reopen within the test's setup clock.

### P2-3 — HTTPS answers from another operation never reach this operation's success rule; SSH's do
- **Root cause:** SSH `Seen::Connected` applies `Succeeded` to every scope (`governor.rs:413-420`); HTTPS `answered` applies it only to its own (`scoped.rs:87-90`). A test is ready only when `connected == n` (`control.rs:226`).
- **Violated:** §4.2 (a success shows the server holds every Connected connection); SSH/HTTPS parity.
- **Sequence (PROBE7):** the operation is STABLE at N=3; `b` answers 4 more connections; the operation sees `connected=7`, `room=false`, `start_test=None`.
- **Impact:** over HTTPS a STABLE operation can neither start nor test until the other operation's connections drop below N; over SSH the same case raises N.
- **Correction:** apply §4.2's rule in every scope for a fresh HTTPS answer, as SSH does.
- **Test:** PROBE7, expecting `n == 7` (or room or a ready test); the same assertion over SSH.

### P2-4 — A refused setup is judged by FIFO order, not by its connection (architectural)
- **Root cause:** every `SetupEnded` or `Retired` is pushed into every scope's `ended` queue (`governor.rs:424-433`), and `setup_failed` pops the oldest (`scoped.rs:156`). The queue therefore holds other operations' failures; failures the endpoint never reports (an SSH reset before authentication, or an authentication failure; `completion.rs:223-224` reports only Suspects); and cancelled connects. Also, a timed-out connect fails the request before it is disposed of (pool `clock.rs`, `fail_request`), and SSH disposal waits for the setup job to join (`ssh_setup.rs:347`), so the endpoint's report comes before the `Retired` entry is filed: every SSH stall or aggregate Suspect gets either `None` (it goes to the retry machine) or someone else's window.
- **Violated:** §4.3 and §4.4 (a result is judged on its own window); the design changelog's reading (4).
- **Sequence (PROBE3):** a confirmation is open with 3 held; a wave setup ends unreported; the confirming test is armed (k=4) and refused fairly; its report pops the wave entry: `Inconclusive` where `Overload{n:3}` was expected; the test's own entry later expires as `Ended` and the test is re-armed.
- **Impact:** confirmations lost or misread, members charged attempts, and SSH stall/aggregate Suspect evidence effectively never reaches the machine.
- **Correction:** return the pool `ConnectionId` with the connect failure (`ConnectFailed`, `ConnectTimeout`, `SetupEnded`) and judge that connection's window; for a cancelled connect, freeze its window at the cancel.
- **Test:** PROBE3; an SSH variant where the report arrives before `Retired` must still give a ruling on its own window.

### P2-5 — An armed test is given back on only some carrier exits, which can hang the site
- **Root cause:** nothing ties the armed test to its carrier. These exits never call `test_unused`: SSH `start_attempt`'s `selected_path` error (`admission.rs:276-290`) and `start_endpoint_open` returning `Err` (`admission.rs:330`); HTTPS allocation expiry (`retry.rs:355`) fails the entry without it.
- **Violated:** §5.2 ("no wait is unbounded"); the changelog's reading (5), that a carrier with no connection gives the test back.
- **Sequence (PROBE8):** a test is armed and its carrier fails before it reaches the pool; ten minutes later the gate is still shut and `start_test` returns `None`. Every member's allocation clock is stopped, and because arming required a quiet key, no other connection ever starts.
- **Impact:** the operation hangs on that site until cancelled.
- **Correction:** an RAII carrier token, or `test_unused` on every non-start exit.
- **Test:** endpoint tests in which the carrier hits each of those exits; the other members must start within the next pass.

### P2-6 — On HTTPS the retry machine's Retry ignores the member's budget
- **Root cause:** HTTPS `Outcome::Retry` requeues without checking `entry.attempts >= allowed` (`retry.rs:173-181`); SSH checks it (`completion.rs:245-247` → `274`).
- **Violated:** §5.3 and §4.7 ("per member: no attempt count above R + 1"); SSH/HTTPS parity.
- **Sequence (PROBE4, same setup as the existing dead-key test):** with `--max-retries 2`, stream 1 makes 4 attempts and is reported as "attempt 3 of 3".
- **Impact:** the user's budget is exceeded and the reported count is wrong.
- **Correction:** apply SSH's `requeue` budget rule on HTTPS's Retry path.
- **Test:** PROBE4, asserting no member ever exceeds 3 attempts.

### P2-7 — An ended SSH request is revived as a permanent "zombie" scope
- **Root cause:** `Book::scope` creates any unknown operation (`governor.rs:157-162`). SSH `report_demand` iterates abandoned `opens` of cancelled requests (`admission.rs:446-479`), and `completion.rs:136-144` calls `test_unused` for them; both run after `end_operation` (`cancel.rs:28-31`).
- **Violated:** §4.1 (a machine is dropped with its operation); the pool's numbers are meant to leave with the last live operation.
- **Sequence (PROBE6):** an operation is at N=3; another request ends with an open still in flight; the next pass reports for it. The zombie stays alive and the pool limit becomes 8 instead of 3.
- **Impact:** for the rest of the session the pool limit is pinned at the ceiling and never cleared, so the pool stops enforcing any live operation's N; scopes accumulate in long-lived gwz-py sessions.
- **Correction:** never create a scope implicitly (`set_demand` and `test_unused` do nothing for an ended operation), and skip opens of ended requests in `report_demand`.
- **Test:** PROBE6, plus an endpoint test that cancels a request mid-open.

### P2-8 — A lone throttle without `Retry-After` is re-sent with no wait
- **Root cause:** `requeues(_, throttled=true)` is always true (`requeue.rs:37`). With `hi=0` the machine sets no hold and moves no N, and the retry machine is told `abandoned`.
- **Violated:** §4.8 and §5.1 (a throttle at `hi = 0` is the retry machine's, a counted failure that waits); §4.8 ("never retried without one of the two waits"). The changelog's reading (2) calls this "no rule change", but it is one.
- **Sequence (PROBE5):** one member against a server that returns a bare 429 sends 4 requests in 0.58 s, then fails with `Capacity`; main and 1.0.17 both send 1.
- **Impact:** a host that is already throttling is hammered.
- **Correction:** give a throttle at `hi = 0` the retry machine's wait (or a T0 hold) before the requeue.
- **Test:** PROBE5, at least 1 s between attempts.

### P3-1 — Three hooks the shipped adaptive path depends on are never called
- **Root cause:** `closes_suppressed` (`control.rs:254`), `is_test_carrier` (`:553`) and `connect_time` (`:139`) have no callers.
- **Violated:** §4.5 rule 2 (without close suppression, close-and-replace churn, for example mixed SSH identities, keeps the key from ever being quiet, so a lifted limit is never tested); the background-close design revision 3's carried rule "never defer a test carrier"; §4.1's Ts, which stays at 250 ms.
- **Correction:** wire all three.
- **Test:** STABLE with identity churn, asserting a probe runs; an SSH carrier that is never deferred.

### P3-2 — `--max-retries 0` against a dead host costs one connect timeout per member
- **Root cause:** a Suspect at `hi ≥ 1` gives `HoldOnly` (`filter.rs:83`), which is requeued and then spent, so the key never closes.
- **Impact:** a stalling host costs ⌈members / C⌉ connect timeouts, where main paid one. Parity with 1.0.17 holds, and Down is deferred to a later step.
- **Correction:** with `adaptive = false`, route these failures to the retry machine as before.
- **Test:** a dead-host endpoint test at `--max-retries 0` with more members than C.

Disk note: the data volume hit 100% during the build; the reviewer deleted only its own scratch target directory (about 6 GB freed). The probe sources and `probes.log` are kept in `review-adaptive/`.

Verdict: NO-GO
