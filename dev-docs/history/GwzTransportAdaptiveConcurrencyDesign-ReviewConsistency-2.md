# GWZ transport adaptive concurrency design, revision 3: consistency-axis re-review (round 2)

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, working tree, 685 lines, sha256 `ee5b9258b9de1532e9f2296b6473ac478ec1813e3b06048fe3f3a5943172699f`. Status: revision 3 DRAFT, under re-review, uncommitted.
**Baseline:**
- Repository HEADs: root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`. I checked the object's hash and all three HEADs at the start and at the end, and nothing moved.
- Revision 2 was diffed from the scratchpad copy `adaptive-rev2.md`, whose sha256 `19565018…` I verified.
- Code was read with `git show <sha>:<path>` at the SHAs the document names, and the controlling documents from the working tree.
- The remediation plan `GwzTransportAdaptiveConcurrencyDesign-RemPlan.md` was read as the disposition claim. Every disposition was re-traced against the text, not accepted on the plan's word.

**Date:** 2026-10-07
**Axis:** Consistency. The document is checked against itself and against the retry plan, amendment 2, the HTTPS design, the reuse design and session plan CS7.23, and the transport setting design. This review is independent, adversarial and read-only. Other axes run in parallel, and nothing here relies on them. The lane owner files this report verbatim.

**Verdict: NO-GO.** 0 P0, 0 P1, 2 P2 (both new), 7 P3 (all new). All 16 round-1 findings are closed on revision 3. Neither new P2 is ARCHITECTURAL. **I pre-commit to GO on a revision that resolves P2-6 and P2-7 as specified.**

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 3 | Status |
|---|---|---|---|
| P2-1 | Each attempt is judged against its admission target. A probe test is fair iff `lo = N`, and a fair refusal backs the timer off. | The §4.4 table (lines 181–185) and the bullets at lines 187–191: a quiet test at `Connected = N` with no departures has `lo = N`, so it is fair, `N` is unchanged, and `T` doubles (§4.6 line 261; §4.7 line 292). A DISCOVERING refusal goes to STABLE at `T0` (lines 257 and 291). I re-traced the original loop (`hi = N` was classed inconclusive and re-armed with no backoff). It no longer occurs: `hi` is not consulted for tests. | **Closed** |
| P2-2 | The confirmation holds every new start with no fill, and `k := Connected + 1` once the key is quiet. | §4.5 rule 3 (lines 229–235) and its rationale (line 238). Re-trace of case 2 (limit 8, a wave of 32 ended by resets): the confirmation holds; the wave resolves; quiet; `k = 9`; one fair refusal; Overload; `N = 8`. No refill wave. The §4.7 Suspect bound (line 306) holds for this path. A residual carrier ambiguity is filed separately as P3-14. | **Closed** |
| P2-3 | Bound quoted per key; retries counted per key; Down limits the key to one handshake per 30 s. | §5.5 lines 406–418. The 127.75 s figure is now attributed to the key for its first `R + 1` attempts, which matches retry plan lines 266–273. Per-member counting and the reset on an empty queue are withdrawn, so the arrival-wait counterexample is gone. A member's own time is "as today, plus at most one retest". The handshake arithmetic still has an off-by-one: P3-13. | **Closed** |
| P2-4 | §14 quotes each clause verbatim, with its replacement. | I checked every quote at its cited line: retry plan 166–168, 202, 206–212 (as OD18 rewrote it), 226–228, 248, 250–255, 266–273, 280–281, 439, 448–449; amendment 2 lines 540, 542, 547; reuse design line 201. All are verbatim, and all eight clauses I listed in round 1 are present. My closure-test search terms all hit §14 items or are argued compatible there (items 5, 7, 8). Three further clauses are missing: P2-7. | **Closed** (residual gaps filed as P2-7) |
| P2-5 | §14 also covers the requeue of a discovery 429, routing by window, OD18's Cold exit, HTTPS §6 and §7, and retry plan §10. | §14 items 1, 2, 10, 11, 12, 14 and 15 quote all six clauses from round 1. The quotes of HTTPS §6 line 261 and §7 line 322 are verbatim. §1 line 12 is corrected. OQ12 (line 579) covers them. | **Closed** |
| P3-1 | The edges use `min(C, max(N, Connected))`. | §4.6 lines 255 and 262; Appendix A line 649. | **Closed** |
| P3-2 | R5 states the real condition. | Line 199. Case 33 exists, but its asserted `hi` is wrong: P3-17. | **Closed** |
| P3-3 | One definition of an attempt. | §5.3 line 384. §5.5 no longer charges members for probes they did not carry. | **Closed** |
| P3-4 | `N_good`, SATURATED at `C`, R9 on the Suspect path. | Line 420 (`N_good` at the last success); line 275 (`N = C` means SATURATED); R9 at line 203 follows rule 3. The restore mechanism itself is now defective: P2-6. | **Closed** (the restore's new defect is filed as P2-6) |
| P3-5 | Recovery comes from the first test after the lag ends. | Case 26 (line 512) and R7a (line 201). Re-trace at 1 s of lag: the decrease lands at about 0.30 s; the test at 0.8 s is refused fairly (the server still counts Y), so `T` becomes 1 s; the test at 1.8 s succeeds. That is within lag + gap in force + setup (2.05 s). At 0.5 s of lag the test at 0.8 s succeeds. | **Closed** |
| P3-6 | Bounds with jitter; case 4 names its class. | §4.7 lines 299 and 303–308. At ×0.8 the tests fall at 0.4, 1.2, 2.8, 6.0, 12.4, 25.2 and 49.2 s, so 7 in 60 s, and `6 + floor(34.8/24) = 7`. Case 4: `24 + 7 = 31`, Throttle, jitter 0.8/1.0/1.2. | **Closed** |
| P3-7 | Status line and Changelog corrected; D4 restored. | Line 3 (Surface review required); D12 (line 35); the correction entry for revision 2 (line 685) matches my round-1 diff. The revision-3 entry has its own omissions: P3-18. | **Closed** |
| P3-8 | Citations fixed; the idle-loss fact stated. | `connected` at `pool/allocation.rs:136` (line 81); `Retries::remove` at `https_endpoint.rs:392` at `db0f8447`, verified; "no gwz-core host calls `idle_closed`" (line 82), verified (`git grep idle_closed db0f8447 -- src` is empty); `ssh_connection.rs:36-44` verified; `can_install_capacity` at `pool/machine.rs:130-137` verified; `install_capacity_pair` exists (`pool/asynchronous.rs:106`, called at `session/capacity.rs:219`); `CancelConnect`/`AbortConnect` at `ssh_pool.rs:314-332` verified. The header's SHA scope now covers every section. | **Closed** |
| P3-9 | Requeue is a fourth verdict. | §5.1 lines 354–356; §14 line 618. | **Closed** |
| P3-10 | OQ13(d) added. | Line 580. | **Closed** |
| P3-11 | D9 deferred to the reuse design. | §6 line 432; Authority line 11 now cites reuse §5 and §9 and CS7.23. | **Closed** |

## Changed-range analysis

- **§4.4 and §4.5, judging and the test-pending rule.** These are internally consistent with §4.6 and §4.7. A probe needs `Connected = N` and quiet (line 184); step 3 at line 221 re-checks the base. Confirmations at `k = Connected + 1 ≤ N` always hold, because a confirmation opens only after a conclusive refusal with `hi < N`. New defects: the "never without a wait" claim (P3-12) and the carrier and moot evaluation (P3-14).
- **§4.10, the setup limit.** On each conclusive drop `Ns := lo_s ≤ hi_s ≤ Ns − 1`, a strict decrease, so `Ns` converges at or below a deterministic `MaxStartups` start value. That makes case 3's "settles at most 10" derivable. Taking a drop without confirmation is a stated design choice (§3.1 line 92, §4.10 line 347), not a departure.
- **§5.5, Down.** It agrees with §14 items 3, 4, 6, 9 and 13 and with §5.3. The handshake bound in §5.5 disagrees with §14 item 12 (P3-13). The restore of `N` has no admission target or judgement (P2-6).
- **§14.** All sixteen items are verbatim at their cited lines. It is not yet complete: P2-7.
- **Cases 30–42.** Cases 30, 32, 34–42 are derivable from the rules. Case 31 is consistent but loose. Case 33 asserts a wrong `hi` (P3-17). Cases 27a and 27b assert a restore that the rules do not produce (P2-6, P3-16).
- **Other changed ranges.** §1 line 18 says "at least every 30 s" (P3-15). The revision-3 Changelog has omissions (P3-18).

---

## 0. Evidence base

- **Object, revision 3.** I read all of it, lines 1–685. I diffed it against revision 2 for §2.1–§2.4 (only the §2.5 heading changed), §3, §7, §9, §11, §12, §13 and Appendix A.
- **Retry plan.** Lines 153–200 (§4, including the non-retriable bullet at 188–190), 202, 206–212, 226–228, 246–281, 439 and 448–449.
- **Amendment 2.** Lines 30, 371 (the OD18 decision text the release plan's §7 gains), 539–562 (OD18: the Cold text at 540, the S3.1 rewrite at 542, the `MaxStartups` paragraph at 546, the hazard at 547).
- **HTTPS design.** Lines 261 and 322.
- **Reuse design.** Lines 197–203. **Session plan.** CS7.23, lines 805–809.
- **Code.** At gwz-core `db0f8447`: `ssh_pool.rs` lines 200–225 and 300–335; `https_endpoint.rs` (`retries.remove` at 392); `https_endpoint/retry.rs` 105–115; `https_worker/prepare.rs` 188–194; `machine.rs` and `backoff.rs` (unchanged since round 1). At gwz-transport `ff6083b5`: `pool/machine.rs` 120–139; `pool/asynchronous.rs` 100–106 and 401; `pool/allocation.rs` 95–131; `pool/lifecycle.rs` 185–214.
- **Arithmetic re-derived:** the §4.7 jitter schedule and bounds; case 4; case 5b; case 26 at 0.5 s and 1 s of lag; the §5.5 dead-host handshake count against §14 item 12; case 33's `hi`.

## 1. Findings

### [P2-6] The restore of `N` after an outage has no admission target or judgement: its refusals can never be conclusive, and the pool blocks its starts (new, not architectural)

- **Location.**
  - §5.5 line 420, the restore: DISCOVERING with a doubling target. Each step admits up to `min(N_good, 2 x max(1, Connected))`. "A conclusive refusal during the restore is an ordinary Overload and ends it."
  - Against it: the §4.4 table (lines 181–185), which has rows only for ordinary starts (target `N`), probe tests (`N + 1`) and confirming tests (`k`); §4.5 line 210 (an ordinary start needs `Possible < N`); §4.6 line 283 (DISCOVERING "climbed one connection per test"); §4.9 line 336 (the pool limit is "`N` normally, raised to `N + 1` (or `k`)" for a test).
  - Case 27b, line 513: "the restore's doubling meets a conclusive refusal at 16".
- **Violated invariant.** D4: every attempt is judged against the admission target it started under. §4.9: the pool's limit equals the machine's admission target.
- **Reproduction.** Take `N_good = 32`. Overloads during the outage leave `N = 8`. The retry machine's retest succeeds, and later the server's limit is 8.
  1. The restore step admits up to `min(32, 2 × 8) = 16`. Starts 9 to 16 are above `N`. §4.5 does not admit them (`Possible < N` fails), and §4.9's pool limit (`N` = 8) holds them in the pool, where §5.2 line 373 promises only tests that they never wait.
  2. Suppose they are admitted anyway. Each has `hi ≥ 8 = N`, so §4.4's only applicable row (ordinary, conclusive iff `hi < N`) makes every refusal inconclusive. "A conclusive refusal during the restore" can therefore never occur, and the restore never ends on a refusal.
  3. "A step whose starts all succeed raises `N`" leaves a step with some refusals undefined: it might repeat, stop, or go to STABLE.
- **Impact.**
  - The mechanism that §5.5, D8, OQ16 and case 27b rely on cannot be implemented from the text.
  - Followed literally, either the restore stalls in the pool with no defined outcome, or its refusals are charged to members (non-final carriers) with no transition.
  - Case 27b's expected "conclusive refusal at 16" cannot be reached.
- **Required correction.**
  - Add a fourth row to the §4.4 table: a **restore step** with target `S`. Its starts are admitted under the same lock with the pool limit raised to `S` (§4.9). A refusal is fair iff `lo = Connected` at the step's start, and a fair refusal is an Overload (`N := Connected`, STABLE, `T := T0`). A refusal with `lo` below that is unfair, and the step is re-armed when the key is quiet.
  - Say what a partly refused step does.
  - Add the restore's entry to §4.6 (an edge from any state on "retry machine Healthy and `N < N_good`"), and state that DISCOVERING's one-per-test climb does not apply while a restore runs.
- **Closure test.** Case 27b in L1 and L2 over the real `PoolMachine`, with `N = 8`, `N_good = 32` and a post-outage limit of 8:
  - the pool limit is 16 during the step;
  - the 9th to 16th starts are created;
  - the first fair refusal is an Overload with `N = 8`, STABLE;
  - no carrier is on its final attempt;
  - no request waits on `AllocationTimeout`.

### [P2-7] §14 still omits three accepted clauses that the design contradicts (new, not architectural)

- **Location.** §14 (lines 586–618), §1 line 12 ("§14 quotes every clause … that it would change"), and OQ12.
- **Violated invariant.** A supersession list must be complete. The omitted clauses are below.
  1. **Retry plan §4, lines 188–190**, among the not-retriable failures: "Any failure after the session is reusable, including a timeout while a fetch or push body is in progress. A push may already have been accepted. This plan does not retry that."
     - An HTTPS discovery GET runs after the connection's setup: today it is `Phase::Other` (`https_endpoint/retry.rs:109-113`, verified).
     - Revision 3 requeues a discovery 429 or 503 (§5.4 line 399), including on a **reused** keep-alive connection (§4.3 line 162, R10, case 36).
     - §14 item 2 rewrites only line 202's HTTPS sentence. The not-retriable bullet still forbids exactly this retry.
  2. **Amendment 2, line 371**, the OD18 decision entry the release plan's §7 gains: "the retry plan's single-probe rules apply from the wave's first retriable failure."
     - Under §5.1 and §14 item 11, a wave failure with `hi ≥ 1` is Requeue, and the single-probe rules start only from a failure at `hi = 0` or a confirming test refused at `k = 1`.
     - Item 11 rewrites the §3.20 Cold text (line 540) but not this second statement of the same decision.
  3. **Amendment 2, line 546:** "TR2.1 classifies such a drop as a retriable setup failure, and its tests include it."
     - Under §4.10, §5.1 line 362 and §14 item 1, a `MaxStartups` drop before authentication at `hi_s ≥ 1` is **Requeue (was Retry)** and feeds `Ns`. It is not a retriable setup failure the key counts.
- **Reproduction.** Apply §14 to the four documents as written:
  - retry plan line 188 still reads "not retriable … any failure after the session is reusable", beside the rewritten line 202 that retries a discovery 429;
  - amendment 2 lines 371 and 546 still describe the old routing.
  Accepted text then contradicts itself.
- **Impact.** If OQ12 is accepted as recommended, amendment 2's next revision records an incomplete change set. The release plan's OD18 record and the TR2.1 test description would then contradict the design that replaces them. This is the same class of defect as round 1's P2-4 and P2-5, and the first clause is newly reached by revision 3's leased-window change.
- **Required correction.** Add the three clauses to §14, quoted verbatim, with replacements:
  - retry plan line 188, an exception for a discovery GET answered 429 or 503 before any Git byte, `Effect::None`;
  - amendment 2 line 371, "from the first failure the retry machine counts (§4.8)";
  - amendment 2 line 546, "classifies such a drop as the setup limit's evidence (Requeue), and as a retriable setup failure only at `hi_s = 0`".
- **Closure test.** Search the retry plan, amendment 2, the HTTPS design, the reuse design and the session plan for "after the session is reusable", "single-probe", "retriable setup failure", "not retriable", "Return", "Closed", "attempts remain", "exactly four handshakes", "fresh budget", "one key", "just failed" and "lower `per_host`". Every hit is either in §14 or argued compatible there.

### [P3-12] §4.8's claim that "a refusal is never retried without one of the two waits" is false for inconclusive refusals (new)

- **Location.** §4.8 line 328 ("A refusal is never retried without one of the two waits, except the requeue of a wave's own co-refusals"). Against it: §4.4 line 188 (an inconclusive refusal is requeued and is no input), §4.5 line 211 (no settle wait in SATURATED), R1 at line 195, R5 at line 199.
- **Reproduction.**
  1. SATURATED at `C`, a server limit equal to `C`, and an accounting lag. A client close of Y is followed by X, which is refused while Y is Settling. That is inconclusive (R1), so X is requeued.
  2. SATURATED's admission does not wait for Settling, so X restarts at once and is refused again, inconclusive again.
  3. With L2's 50 ms setup, a member spends all `R + 1 = 4` attempts inside one `Ts` (250 ms) and fails `Capacity`.
- **Impact.** The claim is false. A member can fail on budget without any wait being applied, against §1's "None of them fails a member while budget remains" in this corner (limit = `C` with lag).
- **Correction.** Either requeue an inconclusive refusal behind the settle of the connections that made it inconclusive, or correct the claim and state the cost.
- **Closure test.** L2 at `C = 4`, a server limit of 4, 300 ms of lag, two identities alternating: no member fails `Capacity`, or the stated cost is asserted.

### [P3-13] The dead-host handshake bound in §5.5 is one less than §14 item 12 and the routing of §4.8 (new)

- **Location.** §5.5 line 418 ("at most the first wave, plus `R`, plus `ceil(elapsed / 30 s)`") against §14 item 12, line 606 ("at most the per-host limit plus four handshakes before the key is Down").
- **Reproduction.** A dead host with a wave of 32 handshakes, all failing with `hi ≥ 1`:
  - every wave failure is Requeue, and none is counted (§14 item 11);
  - the confirmation then tests at `k = 1`, which is the first counted attempt (§4.8 line 322);
  - `R` further probes follow;
  - total `32 + 1 + R = 36`, which is §14's figure, not §5.5's 35.

  Only for a wave of 1 (where `hi = 0` is counted directly, case 32) is "wave + `R`" right.
- **Correction.** "The first wave, plus one confirming handshake when the wave had more than one setup, plus `R`, plus …".
- **Closure test.** Case 31 asserts exactly 36 handshakes before Down at the defaults.

### [P3-14] The carrier and moot rule is evaluated per member, so a confirmation closes moot while members still need new connections, and case 2 depends on timing (new)

- **Location.** §4.5 line 217 ("needs a new connection (one that cannot lease an idle connection of its identity)"), line 235 (moot: "none needs a new connection"), §4.7 line 301. Case 2, line 488 ("one confirming test … each member used at most 3 attempts").
- **Reproduction.** Case 2 over L2 or L3-H with one identity:
  1. The wave's 8 successes finish their fetch and go idle before the key is quiet.
  2. Each of the 24 queued members can individually lease one of the 8 idle connections, so "none needs a new connection", and the confirmation closes moot with `N = 32`.
  3. Eight members lease. Sixteen are left with nothing idle and start in SATURATED. They are refused (a second wave, each charged).
  4. A new confirmation follows. A refused carrier then reaches attempt 3, and its success is attempt 4.
- **Impact.** Whether case 2 holds depends on fetch length against setup time, which the case does not state, and a second refused wave is possible.
- **Correction.** Count carriers as queued members minus idle connections of their identity (an aggregate), or state the fetch timing in case 2.
- **Closure test.** Case 2 with a fetch shorter than the wave's resolution: one confirmation, and at most 3 attempts per member.

### [P3-15] §1 says a down host is "retested at least every 30 s"; §5.5 says at most one per 30 s, and none without a member (new)

- **Location.** §1 line 18, against §5.5 line 418 ("at most one handshake per 30 s while members keep selecting it, and none while none do") and §4.7 line 297 ("No connection is ever opened only to test").
- **Impact.** The outcome statement promises a minimum retest rate that the design rules out.
- **Correction.** "Retested at most once per 30 s, by a member that selects it, instead of being closed."
- **Closure test.** The text reads that way.

### [P3-16] Case 27a expects a restore by doubling, but an outage that loses every connection never lowers `N` (new)

- **Location.** Case 27a, line 513 ("when the host returns, `N` is restored by doubling to `N_good = C`"). Against it: §4.5 rule 3 (a test refused at `k = 1` goes to the retry machine, and no Overload occurs) and §5.5 line 420 (the restore runs only when `N < N_good`).
- **Reproduction.** At `N = C`, every connection is lost and the setups are reset:
  - the confirmation tests at `k = 1`, and the refusal goes to the retry machine;
  - `N` stays `C` throughout;
  - on Healthy, `N = N_good`, so no restore runs, and SATURATED admits `C` at once.
- **Correction.** Make case 27a assert no restore and SATURATED admission. Alternatively, if a ramp after an outage is intended, state its trigger in §5.5.
- **Closure test.** Case 27a passes as restated.

### [P3-17] Case 33 asserts `hi = 8`; the scenario it describes gives 9 (new)

- **Location.** Case 33, line 519. Against §4.3 line 167 (`S_hi` keeps the connections that left and adds the starts).
- **Reproduction.**
  - At `N = 8`, an ordinary attempt starts with up to 7 others Possible.
  - Two of them close and are replaced. `S_hi` keeps both leavers and adds both replacements, so `hi = 7 + 2 = 9`.
  - `hi = 8` would need 6 others at the start, which the case does not state.
- **Correction.** Assert `hi = 9`, or state the starting count.
- **Closure test.** L1 case 33 with the stated counts.

### [P3-18] The revision-3 Changelog omits changes it made (new)

- **Location.** Changelog lines 667–684.
- **Diff evidence against revision 2:**
  - §3.1's Suspect bullet was rewritten (line 92). No §3 entry exists.
  - Appendix A's mapping changed (line 649: the `min(C, max(N, Connected))` formula and the OQ13(d) cross-reference).
  - OQ1–OQ4 and OQ7–OQ10 had revision 1's rationale restored (OQ1's TR2.24 sentence, OQ2's starvation, OQ3's "only path" reason, OQ4's misread risk, OQ7's options, OQ8's case-1 claim, OQ9's statics, OQ10's "with whoever wrote"). Line 684 lists only OQ5, OQ6, OQ11–OQ17.
- **Correction.** Add these to the revision-3 entry.
- **Closure test.** A section-by-section diff of revision 2 against revision 3 matches the entry.

## 2. Invariant analysis

- **One judgement per admission target (D4).** It holds for ordinary starts, probe tests and confirming tests. The fair-test definition (`lo` equals the base) is consistent across §4.4, §4.5, §4.6, §4.7 and the cases.
  - It fails for the one admission path revision 3 added outside §4.4, the restore (P2-6). It also fails for the setup limit's tests only in that `hi_s` and `lo_s` are introduced without their own row in the table. §4.10 states them adequately, so I file no finding there.
- **Pool limit equals the admission target (§4.9).** It holds for ordinary starts, tests and confirmations (cases 41 and 42 are derivable). It does not hold for the restore (P2-6).
- **Accounting for retries.** Key-level counting is restored, and Down bounds handshakes. §14 items 3, 4, 6, 9 and 13 agree with §5.5. Two exceptions: the dead-host count (P3-13), and the "never without a wait" claim (P3-12).
- **Controlling graph.** §14 now covers everything round 1 named, verbatim. The remaining gap is the not-retriable bullet in retry plan §4 and the two OD18 statements in amendment 2 that §3.20 itself does not hold (P2-7). Nothing in the transport setting design conflicts.
- **Test satisfiability.** Cases 1, 3, 4, 5, 11, 21–26, 28–32 and 34–42 are derivable from the rules. Case 2 depends on timing (P3-14). Cases 27a, 27b and 33 assert values the rules do not produce (P3-16, P2-6, P3-17).

## 3. Risks and next action

- **Risk.**
  - P2-6 leaves D8 and OQ16's restore unimplementable as written. An implementer would invent either the judgement or the pool limit, and those are exactly the places where the race model must be precise.
  - P2-7 would carry an incomplete amendment into amendment 2's next revision, which then needs its own dual review.
- **Next action.**
  - Add the restore-step row to §4.4, together with its pool-limit rule in §4.9, its §4.6 edge and its partial-step outcome (P2-6).
  - Add the three clauses to §14 (P2-7).
  - Fix P3-12 to P3-18 in the same revision. Each is a local text change.
- **Pre-commitment.** I pre-commit to GO on a revision that resolves P2-6 and P2-7 as specified. The P3s should be fixed but do not block.
