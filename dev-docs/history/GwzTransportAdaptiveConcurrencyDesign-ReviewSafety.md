# GWZ transport adaptive concurrency design, revision 2: safety-axis review

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, the working-tree file. Revision 2, DRAFT, 567 lines, sha256 `19565018463220c3727435d31da649667f2e6a7f2e4ca90fab7d9485c7ebfea8`. It is uncommitted (`M` against gwz-core HEAD, which holds revision 1). Reviewed 2026-10-07.
**Baseline:** root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`. Sources were read from the working tree at those HEADs with read, grep and sed only. I checked the tuple at the start and at the end and it did not change.
**Date:** 2026-10-07
**Axis:** Safety: what the text allows to go wrong. This covers stuck states, hammering, the claim of no regression against the status quo, push replay, disclosure, and the race model. The review is independent, adversarial and read-only. Other axes run in parallel and nothing here relies on them. The lane owner files it verbatim.

**Verdict: NO-GO.** 0 P0, 0 P1, 8 P2, 7 P3. I pre-commit to GO on a revision that resolves P2-1 to P2-8 as specified, with each P3 either fixed or recorded as accepted by the operator.

---
## 0. Evidence base

- **Object:** I read the whole document, lines 1–567.
- **Controlling documents:**
  - `GwzRemoteTransportRetryPlan.md` §§4–6, lines 153–370. Key anchors: line 202, retry only "for the connect that happens before the first request byte"; lines 209–212, Closed "stops `--jobs 1` from running a fresh budget for each member"; line 266, "the network-only bound for one key".
  - `GwzTransportReleasePlanAmendment-2.md` §3.20, OD18, lines 539–550.
  - `GwzRemoteTransportHttpsDesign.md`: §2 lines 33–61; §6 line 261, "No transparent retry is permitted, including GET network failures"; §7 line 322, 429 means "no Retry-After sleep or retry"; §7 text "never raw URLs, Location".
- **Code used as evidence:**
  - `src/git/endpoint/setup_retry/machine.rs:104-243`. `abandoned` releases the Degraded probe slot, and `Closed` returns `Decision::Finish`.
  - `gwz-transport/src/pool/allocation.rs:10-40`. Reuse of an idle connection comes before creating one, and requires the same identity.
  - `src/git/endpoint/https_worker/prepare.rs:176-204, 382-411`. The HTTPS pool key is the post-redirect `destination.host()`. The identity is `HttpsScoped(credential scope)`, so anonymous and credentialed requests to one host are different identities.
  - `src/git/endpoint/https_destination.rs:120-153`. Cross-host redirects are allowed.
  - `src/git/endpoint/agent_job.rs` and `ssh_network.rs:44-78`.
- I ran nothing and modified nothing.

## 1. Findings

### [P2-1] A fair refused test is "inconclusive" by the design's own definition, so tests re-run with no backoff and ignore `Retry-After`

- **Location:**
  - §4.4 lines 173–174: conclusive iff `hi(a) < N`; inconclusive iff `hi(a) >= N`. An inconclusive test "is re-armed as soon as the key is quiet … with no backoff".
  - §4.5 line 191: a fair test's "refusal has `hi = N` exactly (no decrease, timer doubles)".
  - §4.6 lines 228 and 232: the edges are labelled "test refused, **conclusive**"; line 243 says an inconclusive refusal causes "no transition".
  - §4.7 lines 261 and 265; case 4 at line 438.
- **Violated invariant:** D6 and §4.7. A refused test backs off 0.5, 1, 2 … 30 s, and refused tests are bounded at `6 + floor((D−31.5)/30)`. The operator's direction is that a throttle slows the command.
- **Reproduction:**
  1. Server limit 8, `N = 8`, STABLE, 8 connections Connected, members queued.
  2. `T` expires and the key is quiet. A test X starts at 9. Its window holds exactly the 8 Connected connections, so `hi(X) = 8 = N`.
  3. The server refuses X. By §4.4 that is inconclusive, so: no transition (line 243), `T` unchanged, re-armed "as soon as the key is quiet". The refused connection is Gone at once, so the key is quiet at once.
  4. X′ starts, is refused with `hi = 8`, and so on. The edges PROBING→STABLE and DISCOVERING→STABLE (line 228) can never fire, because a fair test can never have `hi < N`.
  5. A 429 carrying `Retry-After` on these tests is ignored too. §4.5 item 2 sets a hold only for a *conclusive* Throttle.
- **Impact:**
  - Refused tests repeat at about one per setup round-trip until every queued member reaches its final attempt (§4.7 line 268). That is roughly `R` refused connections per member that passes through the key, against a server that is asking the client to back off.
  - On SSH these are pre-authentication drops, which is the pattern OpenSSH `PerSourcePenalties` penalises (§2.4(c)). §10.3 row 2 itself warns that "a sustained refusal can affect the account".
  - Case 4's assertions cannot hold under the normative rules.
- **Required correction:** Judge a refusal against the admission target the attempt started under: `N` for an ordinary start, `N+1` for a test, `k` for a confirmation.
  - A test refused with `hi = N` is a conclusive refused test: `N` unchanged and `T := min(2T, Tmax)`.
  - A test refused with `hi < N` is an Overload.
  - Any refused test with `Retry-After` sets the hold.
  - State this in §4.4 and make the §4.6 edge labels match.
- **Closure test:** An L1 case at limit 8: test refusals with `hi = N` follow the gaps 0.5, 1, 2, 4, 8, 16, 30 s, and a test refusal carrying `Retry-After: 5` produces no start before +5 s.

### [P2-2] The Suspect confirmation needs "exactly `k−1` Connected", which a real limit or an outage never allows; the fill-up rule then re-offers refused load with no backoff and uses up members' budgets

- **Location:** §4.5 item 3, lines 201–206: "if fewer than `k − 1` are Connected, ordinary starts fill up to `k − 1` first". Co-refusals "may lower `k`". The moot rule is at line 205.
- **Violated invariant:**
  - The operator's direction that a throttle never fails a member while budget remains, with budget spent only on bounded, backed-off attempts.
  - §4.7's bound "a Suspect's confirmation: 1 refused test".
  - Case 2 (line 436) and case 3 (line 437): "all members succeed".
- **Reproduction 1, a limit (case 2 as written):**
  1. HTTPS resets at a limit of 8. 32 members, `C = N = 32`, `R = 3`.
  2. In the wave, 8 succeed and 24 are reset with `hi = 31 < 32`. That is a conclusive Suspect, so a confirmation opens with `k = 32` and holds at 31.
  3. Connected is 8, which is below 31, so 23 fill starts begin. All are reset, with `hi ≈ 30`. They are co-refusals, `k` becomes 31, and each member is charged an attempt.
  4. The fill repeats at 30, then at 29. After four rounds, a few setup times in all, every refused member has spent `R + 1` attempts and fails with `Capacity` "throttled".
  5. No confirming test ever starts, because Connected never reaches `k − 1` while the server holds 8.
  6. Once the queue drains, the confirmation is moot and "closes with nothing changed": `N` is still 32. The next members are admitted at 32 again and the cycle repeats.
  7. The same path follows from a stock OpenSSH `MaxStartups 10:30:100` against the default `--max-per-host 32`, whether through `N` or through `Ns` (§4.10).
- **Reproduction 2, an outage:**
  1. The host restarts and answers with RST. The 32-member wave is refused at once with `hi = 31`, a Suspect, and a confirmation opens with `k = 32`.
  2. Connected is 0, so 31 fill starts begin and are refused within milliseconds, and so on.
  3. About 128 refused connects occur within well under a second. Every member fails, labelled "throttled", before the retry machine has waited at all. The retry machine was told `abandoned` for every one (§4.8 line 287; §5.1 line 318).
  4. The status quo did better: the candidate at HEAD sends the wave, then single probes at 1, 2 and 4 s, and members ride out a few seconds of outage. 1.0.17 makes one connect per member.
- **Without the fill rule,** the same states deadlock instead: the confirmation is never testable and never moot, and members wait "behind a test" with their allocation clocks stopped (§5.2 line 332).
- **Required correction:**
  - Run the confirming test at whatever is Connected once the key is quiet. Its target is `Connected + 1`, judged as in P2-1.
  - Remove the fill-up, or give it a backoff and make it uncharged.
  - Make a confirming test refused at `hi = 0` go to the retry machine.
  - State a bound on co-refusals per confirmation.
- **Closure test:**
  - Case 2 at L1 and L2: with resets at a limit of 8, all 32 members succeed, exactly one confirming test runs, and `N = 8`.
  - A new L2 case: an RST outage at `C = 32` produces at most the wave plus `R` probes, and no member fails before the retry machine's waits have elapsed.

### [P2-3] Routing by `hi` alone keeps an outage away from the retry machine; R9 and case 27 describe a path the rules do not produce

- **Location:** §4.8 lines 287–288; §5.1 line 318, where the retry machine is "told `abandoned`"; R9 at line 186, "Refusals at `hi >= 1` lower `N` toward 1"; §5.5(4) line 366, where the rollback triggers on "a failure at `hi = 0`"; case 27 at line 464.
- **Violated invariant:** The retry plan's per-key backoff applies to a host that is failing (§4: "After a retriable failure … the key waits"). §5.5's rollback must run after an outage.
- **Reproduction:**
  1. One long clone is Connected. New connects to the host fail with `ECONNREFUSED` or a stall, because a load balancer is draining or the network path is flapping.
  2. Each failure has `hi ≥ 1`, because the clone counts. Each is a Suspect, goes to the filter, and is requeued with only the 25–250 ms confirmation delay.
  3. The retry machine sees `abandoned` and stays in Cold or Healthy. `machine.rs:211-215` frees a Degraded probe slot on `abandoned`, so even a Degraded key admits the next probe at once.
  4. Outage failures are Suspects (refused, reset, stall; §3.2 lines 106–110), so they do not lower `N` without a confirmation. R9's "lower `N` toward 1" is therefore true only for 503 with `Retry-After`.
  5. The rollback trigger, the retry machine leaving Healthy, fires only once nothing at all is Possible. That includes the abandoned stalled setups that §4.4 R6 keeps Possible until their threads retire.
- **Impact:** For as long as anything else on the key is alive, a failing host gets no 1, 2, 4 s backoff, and §5.5's rollback cannot run in the cases R9 claims it covers. This root cause is independent of P2-2: fixing the fill rule still leaves outage failures at `hi ≥ 1` without the retry machine's waits.
- **Required correction:** Define when a Suspect is reported to the retry machine as a counted retriable failure even though `hi ≥ 1`. Examples: a confirming test refused, a refusal while Connected connections are being lost, or a Suspect refuted by no success within the retry machine's wait. Rewrite R9 and case 27 to match the rules.
- **Closure test:** An L2 case with one long clone Connected and refused connects for 5 s. Retried setups are spaced at no less than the retry machine's waits, and the outage reaches the retry machine.

### [P2-4] §5.5 removes the per-key bound: every member gets a fresh budget against a dead host

- **Location:** §5.5(2)–(3), lines 364–365: the wait level "is reset to the first wait (1 s) … also when the key's queue empties". Line 368: "The key has no bound of its own while members keep arriving, and needs none". Case 29 at line 466.
- **Violated invariant:**
  - Retry plan §4 lines 209–212, which OD18 keeps: Closed "stops `--jobs 1` from running a fresh budget for each member".
  - Retry plan §5 line 266: the 127.75 s bound is "for one key", not per member, as §5.5 restates it.
  - The design's own "never worse than the status quo" (§11 line 482).
- **Reproduction:**
  1. `--jobs 1`, 200 members on a host that black-holes SYNs. Each failure has `hi = 0` and goes to the retry machine.
  2. Member 1 makes 4 attempts with 1, 2 and 4 s waits and fails. The queue empties, so the level resets to 1 s.
  3. Member 2 arrives and repeats the same four attempts, and so on through member 200.
  4. Result: 800 failing handshakes and about 200 × 127.75 s ≈ 7.1 h.
  5. The comparisons:
     - Retry plan today: about 4 handshakes and about 128 s, then every member finishes at once.
     - 1.0.17: one connect per member, so 200 connects.
  6. With `--jobs` larger than `C`, the cost scales by about `members / C` batches.
  7. Against an OpenSSH server already dropping the client pre-authentication, through `MaxStartups` or `PerSourcePenalties`, this sustains the very pattern that prolongs the penalty (§2.4(c)).
- **Required correction:**
  - Keep a key-level backoff that does not reset when the queue empties, only on a probe success. After the first exhaustion, a dead key is probed at most once per `Tmax`, whatever arrives.
  - State the per-key handshake rate and the command-time bound against a dead host.
  - List the retry plan's §5 "for one key" bound and the `--jobs 1` sentence among the text §5.5 changes, so that OQ12 is decided with the cost in view.
- **Closure test:** An L1 case with `--jobs 1`, 50 members and a host that is always refused. Total handshakes are at most `R + 1 + ceil(elapsed / 30 s)`, and the elapsed time is stated and asserted.

### [P2-5] A due test can be starved forever: nothing drives the key to quiet

- **Location:** §4.5 line 191, where a test is allowed only when the key is quiet and ordinary starts are held only "while a test is in flight". §4.6 line 231, where STABLE→PROBING requires "key quiet". §4.7 line 262: "A command never goes longer between tests."
- **Violated invariant:** The operator's direction that a limit must be tested because it may have been temporary, and the stated `Tmax` bound.
- **Reproduction:**
  1. An HTTPS workspace on one host mixes anonymous public members with credentialed private ones. These are different pool identities: `Identity::Https` and `HttpsScoped(scope)` (`prepare.rs:191-204`, `pool/allocation.rs:18-24`).
  2. The machine is STABLE at `N = 8`, below the server's real limit, after another client's burst has ended.
  3. Each member of the other identity evicts an idle connection. That connection is Closing, then Settling for `Ts ≥ 250 ms`, then a new setup starts (§4.5 line 195 acknowledges this cost).
  4. If evictions arrive more often than once per `Ts` plus setup time, the key is never quiet. `T` expires and stays expired, and `N` stays at 8 for the rest of the command.
- **Required correction:** Once `T` has expired and a member is queued, hold ordinary starts and evictions until the key is quiet, then run the test, with the wait bounded as §5.2 bounds a test wait. Apply the same rule to a confirmation.
- **Closure test:** An L2 case with two identities alternating at `N = 8`, server limit 32, and the external client released at `t2`. A test runs within `T + Ts +` one setup time after `t2`.

### [P2-6] The attempt window, and so the routing, is undefined for a discovery GET on a reused connection, which is the main HTTPS throttle path

- **Location:** §4.3 line 156: "`W(a)` = from its socket connect to its result". §4.4 R10 line 187. §5.1 line 317 and §5.4 line 354: a 429 on the discovery GET is requeued. §4.5 line 195 and §8 line 405: reuse of an idle connection is the normal path.
- **Violated invariant:** Every Throttle must be routed deterministically, and a server's `Retry-After` must be honoured.
- **Reproduction:**
  1. SATURATED at 32. Members lease idle HTTPS connections (`allocation.rs:16-38`; `prepare.rs:179`, `lease.reused`).
  2. GitHub begins answering discovery GETs with 429 and `Retry-After: 30`.
  3. The open has no socket connect, so `W(a)`, `hi(a)` and the choice between `hi = 0` and `hi ≥ 1` are undefined.
  4. Under the natural reading, where the window runs from the connection's own connect, `S_hi` accumulates every connection since then. So `hi ≥ N`, the 429 is inconclusive, and §4.5 item 1 ignores the `Retry-After`.
  5. The member is requeued at once and charged, and the next member's 429 is treated the same way. Members burn `R + 1` attempts against a server asking the client to wait, and `N` never moves.
- **Required correction:** Define the window for a leased connection's exchange: from lease to result, with the leased connection itself counted in Connected. State whether a request-level 429 on an established connection is evidence about connection concurrency at all. Make every 429 or 503 carrying `Retry-After` set the key's hold, whether conclusive or not.
- **Closure test:** An L3-H case in which a keep-alive server returns 429 with `Retry-After: 2` to discovery on reused connections. No discovery is sent on the key within 2 s, and the routing is asserted from the log.

### [P2-7] Under `--max-retries 0`, one conclusive Throttle lowers `N` for the rest of the command, and nothing can ever test it again

- **Location:** §5.3 line 346, where a conclusive Throttle "still lowers `N` for the members that come after". §4.7 line 268: no test is ever given a member's final attempt, and at `R = 0` every attempt is final. §4.2 line 147: a success cannot raise `N` above an admission cap of `N`.
- **Violated invariant:** The operator's direction that a transient failure must not become a permanent degradation, and that a limit must be tested.
- **Reproduction:**
  1. `R = 0`, 200 members. A one-second burst of 429s during the first wave sets `N := Connected`, say 3.
  2. No carrier ever exists, so the machine never enters PROBING.
  3. The remaining members run at 3 for the whole command.
  4. The same happens after the wrong decrease of R7 (line 184): the design's repair, "the first probe … succeeds", never runs at `R = 0`.
- **Required correction:** At `R = 0`, either a Throttle sets the `Retry-After` hold without changing `N`, or a decrease made with no possible carrier expires at `T` with the risk stated. Choose one and state it in §5.3 and case 11.
- **Closure test:** Case 11 extended: with `R = 0`, a 1 s 429 burst, then no limit, the key is back at `C` within the stated rule, or `N` was never lowered.

### [P2-8] The outage rollback raises `N` without a test and admits members on their final attempt at the restored level

- **Location:** §5.5(4) line 366: "`N := N_before` … put in STABLE with `T := T0`, so the restored value is tested within 0.5 s". §5.5(2) line 364: every probe charges every queued member. §4.7 line 268: "testing alone can never fail a member".
- **Violated invariant:**
  - A raise of `N` happens only by an observation or a test (D3).
  - The carrier rule, under which members on their final attempt never carry an untested level.
- **Reproduction:**
  1. `N_before = 32`. A 503-with-`Retry-After` outage lowers `N`, and the retry probes charge every queued member, so they reach attempt `R + 1`.
  2. The host returns behind a new front end, or with other clients reconnecting, and its limit is now 8.
  3. The probe succeeds, `N := 32`, and admission starts up to 31 members at once.
  4. Twenty-four are refused. One Overload occurs, and the members refused on their final attempt fail as "throttled".
  5. The restored value is used at once, not tested. Only `N + 1` is tested after `T0`.
- **Required correction:** Restore `N` through tests, for example DISCOVERING from the post-outage `Connected` with a faster climb bounded by `N_before`. Alternatively, admit at the restored level only members that are not on their final attempt, and state the burst cost and its bound.
- **Closure test:** An L2 outage where the limit after the outage is lower than before. No member fails on its final attempt because of the restore, and refusals after the restore are bounded as stated.

### [P3-1] The definition of `N_before` restores only the last step of a cascade

- **Location:** §5.5(4) line 366: "the `N` it had before the most recent Overload that has not been followed by a success".
- **Problem:** R9 (line 186) describes a cascade of Overloads, for example 32 → 12 → 5 → 2 → 1. The most recent one is 2 → 1, so `N_before = 2`, not the pre-outage 32 that case 27 (line 464) asserts.
- **Correction:** Define `N_before` as `N` before the first Overload since the last success, or as `N` at the last success.
- **Closure test:** Case 27 with a cascade of at least 3 Overloads.

### [P3-2] STABLE with `N = C` is reachable but undefined, and would probe at `C + 1`

- **Location:** §5.5(4) line 366 and §6 line 380, where the 1.2.0 memory "starts in STABLE with that `N`". STABLE is defined as `N < C` (line 250), and PROBING tests `N + 1`.
- **Problem:** When `N_before` or the remembered `N` equals `C`, the machine probes `C + 1`. That violates "never raises it above `C`" (§8 line 402). Against the pool's `per_host` cap, the test parks while PROBING holds ordinary starts.
- **Correction:** At `N = C`, enter SATURATED.
- **Closure test:** An L1 case where the rollback or memory value is `C`, the machine enters SATURATED, and no test starts.

### [P3-3] Settle and `Possible` admission add latency with no throttle, contrary to §8 and §11

- **Location:** §4.5 lines 193–195, where the pool settle is unconditional per key (§4.9 `set_settle`). §8 line 402 and §11 line 481: "no added wait".
- **Reproduction:**
  1. 100 HTTPS members on github.com, `C = 32`, no throttle, mixing anonymous and credentialed members.
  2. Every cross-identity eviction, and every discarded connection (HTTPS design §7: "All terminal non-success responses discard the connection", for example a private member's 404), holds the slot for at least 250 ms before the replacement. Abandoned stalled setups (the slow-login tail) block admission at `C` until their threads retire.
  3. 1.0.17 has neither wait.
- **Correction:** Apply the settle only below `C`, that is outside SATURATED, or state the cost and add a parity row with more than `C` members and mixed identities.
- **Closure test:** §10.3 row 5 extended to 100 members of mixed identity, with no settle wait recorded in SATURATED.

### [P3-4] An HTTP-date `Retry-After` read against the local clock can fail every parked member

- **Location:** §3.2 line 102 ("delta-seconds or HTTP-date") and §5.3 line 341, under which a `Retry-After` over 30 s fails the parked members, and "nothing is sent to the host before it has passed".
- **Reproduction:** The client clock is 60 s behind the server. A `Retry-After` that is the server's now + 1 s reads as 61 s. Every parked member fails at once, and the key sends nothing for the rest of the period.
- **Correction:** Compute an HTTP-date relative to the response's `Date` header, falling back to delta-seconds or the 30 s cap. State this.
- **Closure test:** An L1 case with a skewed `Date` and an HTTP-date `Retry-After` 1 s after it. The result is a 1 s hold and no failure.

### [P3-5] The `throttle` detail and the note can disclose a host learned only from a redirect's `Location`

- **Location:** §5.3 line 344, which carries "the pool key's host and port", and §9 line 413, `gwz: <host>: …`.
- **Problem:** After a cross-host redirect (`https_destination.rs:120-153`), the pool key is `destination.host()` (`prepare.rs:192`), which is the redirect target. HTTPS design §7 retains "never raw URLs, Location". The new field and the note would publish a host that today's errors withhold.
- **Correction:** Report the member's configured host, or omit `host` when the key differs from it.
- **Closure test:** An L3-H case where a throttle arrives after a cross-host redirect. Neither the detail nor the note contains the target host.

### [P3-6] Requeueing a 429 on the discovery GET widens retry past the first request byte without amending the controlling text

- **Location:** §5.1 line 317 and §5.4 line 354, against:
  - retry plan §4 line 202 ("before the first request byte");
  - HTTPS design §6 line 261 ("No transparent retry … including GET");
  - HTTPS design §7 line 322 ("no Retry-After sleep or retry").
- **Problem:** §5.5's list of retry-plan changes (line 370) and the Authority bullet name neither change. The safety predicate moves from "no request byte" to "no Git byte, `Effect::None`", and that move is not under review as an amendment.
- **Correction:** List both clauses among the changes OQ12 asks the operator to accept, with the new predicate stated.
- **Closure test:** The amendment text names the clauses. A test pins that a receive-pack discovery 429 is requeued and a POST 429 is not.

### [P3-7] The diagram's success edges assign `N := Connected`, which can lower `N` on a success

- **Location:** §4.6 lines 226 and 233, against §4.2 line 147 (`max`) and line 149 ("never on demand alone").
- **Reproduction:** During a test window, the server closes two Connected connections. The test succeeds with `Connected = N − 1`. The edge sets `N := N − 1`: a decrease with no refusal.
- **Correction:** Label the edges `N := min(C, max(N, Connected))`.
- **Closure test:** An L1 case with two server closes during a successful test, where `N` is unchanged.

## 2. Invariant analysis

| Invariant | Holds? | Where it breaks |
|---|---|---|
| A throttle never fails a member while budget remains, and budget is spent only on bounded, backed-off attempts | No | P2-1 (test loop), P2-2 (fill rounds), P2-6 (ignored `Retry-After`), P2-8 (restore burst) |
| A limit is tested because it may have been temporary | No | P2-5 (starved test), P2-7 (`R = 0`) |
| A transient failure must not become a permanent degradation | No | P2-7 (command-long `N`), P2-2 (`N` never learned while members fail), P3-4 (skew closes the key) |
| The client-versus-server count lag is accounted for | Mostly | The R1 and R7 handling is sound for fresh connections. It is undefined for leased connections (P2-6). |
| The race model never raises `N` above what the server holds | Yes | The `Connected`-based raise is sound. I found no interleaving that raises `N` above the held count, apart from the `C + 1` probe (P3-2). |
| The race model never lowers `N` wrongly without repair | Partly | R7 is repaired only when a carrier exists (P2-7). P3-7 lowers `N` on a success. |
| Bounded hammering of the server | No | P2-1, P2-2, P2-4 |
| Push safety (no requeue after bytes of a receive-pack POST) | Yes | §5.4 forbids it, and nothing reviewed re-enters a POST. Routing a POST's 429 into the retry machine at `hi = 0` affects only the key's state, not the member. |
| Disclosure | Mostly | Counts, host and port only, with no URL or credential. P3-5 is the redirect host. |
| The "never worse than the status quo" claim | No | Dead host (P2-4), outage (P2-2 and P2-3), no-throttle waits (P3-3) |

## 3. Risks and next action

- **Highest risk:** P2-1 and P2-2. As written, the normative rules produce tight refusal loops against a server that is signalling overload. That is what the design exists to prevent, and on SSH it can provoke source penalties. Both contradict the design's own cases 2, 3 and 4, so an implementation driven by those cases would expose them. A text-driven implementation would not.
- **Next action:**
  1. Revise §4.4 to §4.6 so that conclusiveness is judged against each attempt's own target (P2-1).
  2. Replace the confirmation's precondition and fill-up rule (P2-2).
  3. Define the routing to the retry machine and the leased-connection window (P2-3, P2-6).
  4. Give §5.5 a key-level backoff that survives an empty queue (P2-4), and a tested restore (P2-8).
  5. Close the starvation and `R = 0` gaps (P2-5, P2-7).
  6. Add the closure tests above as §10.2 cases.
- **Re-review:** On this axis, after the revision.
- **Deferred items:** The outcomes of OQ1–OQ16 remain deferred. P2-4 and P2-8 bear on what OQ12 and OQ16 ask the operator to accept, and the revision should state their costs in those questions.
