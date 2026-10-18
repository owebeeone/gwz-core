# GWZ transport adaptive concurrency design, revision 2: consistency-axis review

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, working tree, 567 lines (`wc -l`), sha256 `19565018463220c3727435d31da649667f2e6a7f2e4ca90fab7d9485c7ebfea8`. Status: revision 2 DRAFT, not reviewed, uncommitted on purpose. Reviewed 2026-10-07.
**Baseline:** root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`. I checked the hash and all three HEADs at the start and again at the end, and none moved. Revision 1 was read with `git -C gwz-core show HEAD:dev-docs/GwzTransportAdaptiveConcurrencyDesign.md` (385 lines). Code was read with `git show <sha>:<path>` at the SHAs the document names. The controlling documents were read from the working tree.
**Date:** 2026-10-07
**Axis:** Consistency. The document is checked against itself, the accepted retry plan, amendment 2 (OD18), the HTTPS design, the transport setting design, and the documents that consume the retry machine (the reuse design and the session plan's CS7.23). The review is independent, adversarial and read-only. Other axes run in parallel, and nothing here relies on them. The lane owner files this report verbatim.

**Verdict: NO-GO.** 0 P0, 0 P1, 5 P2, 11 P3. I pre-commit to GO on a revision that resolves P2-1, P2-2, P2-3, P2-4 and P2-5 as specified below.

---

## 0. Evidence base

- **Object.** I read all of it: lines 1–567. Revision 1 was saved to the scratchpad and compared section by section: §2.1–§2.4, §7, §5.3, §9 and §13.
- **Retry plan** (`GwzRemoteTransportRetryPlan.md`). I read the status, §§1–6, §9 S3.1 and §10. The clauses cited here are at lines 202, 206–212, 226–228, 248–249, 258–259, 266–273, 280–281, 439 and 448–449.
- **Amendment 2** (`GwzTransportReleasePlanAmendment-2.md`). I read the status, §1 and §3.20 (lines 539–562: the OD18 Cold text, the S3.1 rewrites, the §4 rationale rewrite, the hazard at line 547, and the reuse-design note at line 562).
- **HTTPS design** (`GwzRemoteTransportHttpsDesign.md`). I read §2 (lines 33–61, including the 2026-10-06 amendment), §6 (line 261, "No transparent retry is permitted, including GET network failures") and §7's table (line 322, "Remaining4xx (408/409/410/413/429 etc.) and5xx | Io with status, no Retry-After sleep or retry").
- **Transport setting design** (`GwzTransportOffSwitchDesign.md`). I searched for the defaults, the note, and the retry statements, and found no conflict.
- **Reuse design** (`dev-docs/GwzConnectionReuseDesign.md`): §5 (line 124 onward), §9 (lines 197–203; line 201 is "A Closed key stops reuse too … 'is finished with the recorded failure' (RETRY:236-242)") and §15 items 8 and 18. **Session plan** CS7.23 (`dev-docs/GwzCoreSessionPlan.md:805-809`).
- **Process.** `dev-docs/AgentProcessRules.md`: the Surface axis amendment at lines 406–420. `dev-docs/GwzProcessOptimization.md`: lines 180–195.
- **Code citations that are new in revision 2,** verified at the stated SHAs:
  - `pool/lifecycle.rs:185-198`: `closed()`, with the quoted doc comment verbatim. ✓
  - `pool/lifecycle.rs:200-214`: `idle_closed()`. ✓ It accepts only `Idle` or `Closing` entries; see P3-8.
  - `pool/allocation.rs:97-131` (eviction) and `:47-95` (creation): ✓ to within one or two lines.
  - `src/git/endpoint/ssh_pool.rs:209-220`: "Destruction precedes the capacity acknowledgement.", line 214. ✓
  - `ssh_connection.rs:37-45`: `terminate`, actually at 36–44, with `shutdown(Shutdown::Both)`. ✓
  - `https_connection.rs:344-360`: `poll_dispose` cancels and joins its tasks. ✓
  - §2.1's retry-machine bullet: `machine.rs` `decide` returns `Decision::Finish(last)` in `State::Closed` (line 113), and the generation rule is at lines 189–191. ✓ `setup_retry/` is identical at `279860c1` and `db0f8447` (`git diff --stat` is empty).
  - `backoff.rs`: `wait_ms`, and `wait_bound_ms`, which encodes the per-key bound. ✓
- **Arithmetic** I re-derived: §4.7's gap sequence and bounds, §10.2 case 4's total, case 5b's test times, case 26's timeline, and §5.5's 127.75 s.

## 1. Findings

### [P2-1] A fair refused probe is "inconclusive" by §4.4's own rule, so the probe timer never backs off

- **Location.**
  - §4.4 line 174: "Inconclusive when `hi(a) >= N` … **`N` does not change, and the probe timer does not double:** if `a` was the test start, the test was not fair, and it is re-armed as soon as the key is quiet … with no backoff."
  - §4.5 line 191: the quiet test's "refusal has `hi = N` exactly (no decrease, timer doubles)".
  - §4.6 lines 228 and 232: the edges "test refused, **conclusive** … `T := T0`" and "`T := min(2T, Tmax)`".
  - §4.7 lines 261 and 274–275; §10.2 case 4 (line 438) and case 5b (line 439).
- **Violated invariant.** One definition of conclusive. The machine's refused-test edges must be reachable.
- **Reproduction.** Take a steady limit of 8 with `N = 8`, STABLE, and members queued.
  1. `T` expires and the key is quiet. The test starts at 9, and the 8 Connected connections are its whole window (§4.5).
  2. The server refuses it. Then `hi(test) = 8 = N`, so §4.4 classes it **inconclusive**: no input, `T` unchanged, re-armed "as soon as the key is quiet … with no backoff".
  3. The key is quiet again at once, so another test starts and is refused the same way.
  4. The loop repeats every setup time (50 ms in L1 and L2).

  The edges `PROBING -> STABLE` and `DISCOVERING -> STABLE` on "test refused, conclusive" need `hi < N`. A fair test never has `hi < N`, so those edges are never taken.
- **Impact.**
  - The operator's probe timer (D6, §4.7) never engages.
  - At a steady limit, refused tests run back to back. Each is charged to the queued member with the most attempts left (§4.7), until every queued member is down to its final attempt.
  - §4.7's bound of "6 + floor((D − 31.5 s)/30 s)" and the gaps that case 4 and case 5b assert cannot be produced by an implementation that follows §4.4.
  - The same root cause makes DISCOVERING re-test without limit once a lifting limit is reached.
- **Required correction.** Define a refusal of the test start (and of a confirming test) separately from an ordinary refusal:
  - A test refused with `hi = N`, its fair outcome, is the **refused-test** input. It leaves `N` unchanged and doubles `T`.
  - Only a test whose window was disturbed (`lo < N`, or `hi > N`) is unfair and re-armed.
  - Restate §4.4's "inconclusive" for ordinary starts only, and make the §4.6 edge labels name the refused-test input.
- **Closure test.** An L1 case: steady limit 8, `N = 8`, members queued, zero jitter injected. The refused tests are spaced exactly 0.5, 1, 2, 4, 8, 16 and 30 s apart. Add an explicit assertion that a refused fair test is never re-armed with no backoff.

### [P2-2] The Suspect confirmation takes `k` from the wave's `hi`, and "fill up to `k − 1`" makes repeated refused waves. Cases 2 and 3 cannot be met, and the §4.7 bound is false.

- **Location.**
  - §4.5 rule 3, line 201: `k = hi(a) + 1`, starts held to `k − 1`, and the test waits for "exactly `k − 1` connections Connected (if fewer … ordinary starts fill up to `k − 1` first)".
  - Line 206: co-refusals "may lower `k` to the least `hi + 1`".
  - R3, line 180 ("the rest of the wave is co-refusal: one observation").
  - §4.7 lines 273 and 276 ("a Suspect's confirmation: 1 refused test"; "two for a Suspect").
  - §10.2 case 2 (line 436) and case 3 (line 437).
- **Violated invariant.** A case in §10.2 must be satisfiable under the rules as written. The bound in §4.7 must hold.
- **Reproduction** (case 2: limit 8, 32 members, the refusal is a reset, which is Suspect):
  1. The first wave of 32 starts. Every attempt has `hi = 31`, and 24 are reset. The first reset is conclusive (31 < 32), so a confirmation opens with `k = 32`. All the wave's co-refusals also have `hi = 31`, so `k` stays 32.
  2. The confirming test needs exactly 31 Connected, but the server holds 8. "Fill up to `k − 1`" therefore starts 23 ordinary setups against a server at its limit, and all 23 are refused. Each has `hi = 30 < N = 32`.
  3. These fills started before the confirming test, so they are co-refusals, and `k` falls to 31. The fill to 30 then starts 22 more setups, all refused, and `k` falls to 30. The pattern continues.
  4. Each round charges every refused member an attempt. With `R + 1 = 4`, members are on their final attempt by about the third round. Ordinary fills may still use that final attempt, so members fail.
  5. When the existing connections plus the queued members drop below `k`, the confirmation turns "moot" and closes with `N` still 32, and the next refusal starts the cycle again.

  Case 3 is the same: an SSH `MaxStartups` drop is always Suspect (F2), with a limit of 10 against a wave of 32.
- **Impact.** In the most likely real scenario (SSH `MaxStartups`, which is always Suspect):
  - case 2's "test starts … with exactly `k − 1` connected; one test, then `N = 8`" is unreachable;
  - members fail by exhausting their budget, against D7 and §1's "None of them fails a member while budget remains";
  - the §4.7 per-command bound of `(C − L) + 1 + 6` refused attempts is exceeded by several hundred.
- **Required correction.** Take the confirmation level from what the server is visibly holding, not from the window's upper bound. For example, `k = Connected + 1` once the wave has resolved, and never fill above the level the refusal was conclusive against. Alternatively, define co-refusals as setting `k` from `lo`. Then re-derive §4.7's Suspect bound and R3's claim.
- **Closure test.** An L1 trace for case 2: limit 8, 32-member wave, reset. Assert one confirming test at 9 after the wave resolves, the total refused attempts, `N = 8`, and all 32 succeed. Run the same with a `MaxStartups` limit of 10 for case 3.

### [P2-3] §5.5's bound claim is false under its own rules, and it misquotes the retry plan's bound

- **Location.** §5.5 point 2 (line 364: "no member waits through more than `R` waits"), point 3 (line 365: the wait level is "the key's", reset only on a probe success or an empty queue), and "The bound" (line 368: "the retry plan's network-only bound of 127.75 s per member is unchanged").
- **Violated invariant.**
  - The retry plan's bound is "the network-only bound for **one key**" (line 266), not a per-member bound.
  - The document's own premise, "`R` waits of at most 30 s", gives 4 × 30 + 3 × 30 + 0.75 = 210.75 s, not 127.75 s.
- **Reproduction.** Use defaults and members arriving continuously at a down host.
  1. The key's level climbs with each failed probe and never resets, because the queue never empties.
  2. Member B arrives while the key is Waiting at level 3. B first waits out that wait, which is not one of its attempts.
  3. B's four attempts are the key's probes 4–7, separated by waits of 8, 16 and 30 s.
  4. B sees four waits (R + 1, not R), up to about 30 + 3 × 30 s, plus 4 × 30 s aggregate: about 240 s, against the claimed 127.75 s.
- **Impact.** It misstates a user-visible wall-clock bound that the retry plan pins and `backoff::wait_bound_ms` encodes. Any test written from line 368 would fail.
- **Required correction.** Either give each member its own wait schedule (the wait after a member's n-th attempt is `min(30 s, 2^(n−1) s)`, and arrival waits are capped), or state the true per-member bound, including the arrival wait and the key-level schedule. Either way, quote the retry plan's bound as per key and list it as superseded (P2-4).
- **Closure test.** An L1 case: a member that arrives at the key's third wait level. Assert the member's total network wait and its wait count against the bound the corrected text states.

### [P2-4] §5.5's list of superseded clauses is not exact or complete

- **Location.** §5.5 "What this changes in the retry plan" (line 370), §1 line 12, and OQ12 (line 515).
- **Violated invariant.** A supersession list must name every accepted clause that the change contradicts.
- **What the list names.**
  - Retry plan §4's Closed sentence and "the paragraph after it". Nothing follows that sentence in §4: lines 206–212 are one paragraph, and the next text is §5's heading. The referent is undefined, and the paragraph includes the rationale as OD18 rewrote it (amendment 2 line 544).
  - §5's Closed state.
  - "§5's counter". That sentence is actually Degraded's definition (line 246).
- **Contradicted but not listed:**
  1. **Retry plan §5, lines 226–228:** "After attempt `R + 1` fails retriable, every member queued on that key receives that last failure. There is no further attempt…"
  2. **Retry plan §5's Waiting state, line 248:** "…and attempts remain". §5.5 point 3 keeps the key in Waiting after exhaustion.
  3. **Retry plan §5, lines 280–281:** "`N` is the attempt that just failed". §5.5 redefines `N` as the member's own count but calls the display "unchanged".
  4. **Retry plan §5, lines 266–273:** the bound "for one key". See P2-3.
  5. **Retry plan §4's rationale** "That stops `--jobs 1` from running a fresh budget for each member" (line 210, as amended 2 line 544). §5.5 does exactly what it forbids: under `--jobs 1` each member arrives to an empty queue and gets a fresh budget.
  6. **Retry plan S3.1, line 439:** "With `--jobs 1`, those 32 members still cause exactly four handshakes … including when the key was Healthy earlier in the operation and then exhausted." Under §5.5 this becomes 32 × 4.
  7. **Amendment 2 line 547, OD18's hazard** for the migration notes: "before Closed stops the rest". A dead key no longer closes.
  8. **Reuse design §9 line 201** ("A Closed key stops reuse too … 'is finished with the recorded failure'") and **session plan CS7.23** (lines 805–809), which implements that sentence. Neither is cited.
- **Impact.** If the operator accepts OQ12 as recommended, amendment 2's next revision would carry an incomplete list. Accepted S3.1 rows and the reuse design would then contradict the design: the S3.1 tests fail as written, and CS7.23 implements the old Closed semantics in 1.2.0.
- **Required correction.** Quote every clause above, as current amended text, with its replacement. Replace "the paragraph after it" with the exact sentences. Add the reuse design and CS7.23 to §1's authority line and to OQ12.
- **Closure test.** Search the retry plan, amendment 2, the reuse design and the session plan for "Closed", "attempts remain", "exactly four handshakes", "fresh budget", "one key" and "attempt that just failed". Each hit is either listed in §5.5 or shown to be compatible.

### [P2-5] Changes to accepted text outside §5.5 have no inventory, contrary to §1's statement that §5.5 is the only proposed change

- **Location.** §1 line 12 ("**§5.5 proposes a change to the retry plan's §4 and §5** … this design does not amend them"). The changes themselves are in §5.1 (line 318, "new Requeue (was Retry) … The retry machine is told `abandoned`"), §4.8 (line 288, a `hi = 0` 429 or 503 goes to the retry machine as Retry), §5.4 (line 354, a discovery GET answered 429 or 503 is requeued), §4.5 rule 2 (the `Retry-After` hold), and §4.5 and §4.9 (the pool's `per_host` set to `N`, which a stall can lower).
- **Violated invariant.** Every accepted clause a design contradicts is named. These clauses are contradicted:
  1. **Retry plan §4.** The closed retriable set drives the key's count. Requeue plus `abandoned` means setup `Io`, reset and stall at `hi ≥ 1` are no longer counted.
  2. **OD18's Cold text, amendment 2 line 540:** "When a setup of the wave fails retriably, that attempt is counted, its member returns to the key's queue, the key leaves Cold". Under §5.1 every first-wave failure has `hi ≥ 1`, and `Machine::abandoned` (`machine.rs:209-213`) leaves the key Cold. OD18's S3.1 rewrite for a dead key ("at most the per-host limit plus three handshakes in total", line 542) no longer holds: see the fill loop in P2-2.
  3. **Retry plan §4, line 202:** HTTPS retries only "the connect that happens before the first request byte". A 429 on discovery is a response, which §4.8 and §5.1 make Retry when `hi = 0`.
  4. **Retry plan §10, lines 448–449:** "A timeout must not lower `per_host`." A confirmed stall or aggregate Suspect lowers `N`, and the pool's per-host cap is set to `N`.
  5. **HTTPS design §6, line 261:** "No transparent retry is permitted, including GET network failures."
  6. **HTTPS design §7, line 322:** "Io with status, no Retry-After sleep or retry". §2.3 quotes this as today's behaviour, but the design never marks it superseded. The §2 amendment ("Adaptive limits are designed separately") covers setup-slot capacity only and does not reach §7's 429 row.
- **Impact.** Taking §1 at its word, a reader would conclude that the HTTPS design and OD18 are untouched. The required dual review of "an amendment to a frozen contract" (OQ12) would then not cover these clauses.
- **Required correction.** Add a superseded-clauses section that covers the whole design, not only §5.5, quoting each clause above with its replacement. Correct §1's sentence. Extend OQ12, or add an OQ, so the operator decides the HTTPS design and OD18 changes explicitly.
- **Closure test.** Read the section against the six clauses listed here. Each appears verbatim with its replacement text.

### [P3-1] The edge labels in §4.6 set `N := Connected`, which can lower `N` on a success

- **Location.** §4.6 lines 226 and 233, and Appendix A line 550.
- **Violated invariant.** D3, §4.2 and R2 say `N` only rises on a success (`N := max(N, Connected)`).
- **Reproduction.** During a PROBING test at `N + 1`, the server closes two idle connections (`idle_closed`; client closes are blocked during a test, but server closes are not). The test succeeds with `Connected = N − 1`. The edge label then sets `N := N − 1`, while line 242's "Any success" rule leaves `N` alone.
- **Impact.** The rules conflict, and following the label gives a false decrease.
- **Correction.** Label both edges `N := min(C, max(N, Connected))`.
- **Closure test.** L1: two `server_close` events during a successful test. `N` is unchanged.

### [P3-2] R5's claim that an ordinary refusal "always has `hi(a) <= N − 1`" is false under the client's own churn

- **Location.** R5, line 182.
- **Reproduction.**
  1. During attempt `a`'s window (up to the 30 s aggregate), connection Y closes and settles to Gone, freeing room under `Possible < N`.
  2. Z starts in that room. `S_hi(a)` never removes Y and adds Z, so `hi(a) = N`.
  3. A refusal of `a` is then inconclusive.
- **Impact.** R8, another client taking slots, relies on conclusive refusals. Under churn, decreases are missed and members are charged attempts for inconclusive refusals.
- **Correction.** State the real condition: `hi ≤ N − 1` only if no start occurred during `a`'s window. Alternatively, hold starts during the windows of ordinary attempts.
- **Closure test.** L1: churn during a long window. Assert the classification the corrected text states.

### [P3-3] §5.3 and §5.5 define "an attempt" in conflicting ways

- **Location.** §5.3 line 339 ("An attempt that was only held back, and never started, is not an attempt") and §5.5 point 2, line 364 ("Each probe the key makes while a member is queued is one of that member's attempts, whether or not that member carried it").
- **Impact.** A queued member that never started is charged under §5.5 but not under §5.3. When members fail, and what they report as `attempt N of M`, depends on which rule an implementer picks.
- **Correction.** One definition, with the retry probe's charging rule stated as an exception in §5.3.
- **Closure test.** L1: a member queued through two failed retry probes it did not carry. Its count is asserted.

### [P3-4] Outage rollback (§5.5 point 4), R9 and case 27 disagree

- **Location.** §5.5 line 366, R9 line 186, and case 27 line 464.
- **The disagreements.**
  - R9 and case 27 say refusals at `hi ≥ 1` "lower `N` toward 1". The outage's failures (refused, reset, lost) are Suspects, which by §4.5 rule 3 lower `N` only after a confirmation; see also P2-2.
  - `N_before` is "the `N` it had before the most recent Overload". After two Overloads (32 to 10, then 10 to 3), that is 10, not "its value before the outage" (32), which case 27 asserts.
  - "Put in STABLE" when `N_before = C` contradicts STABLE's "admission to `N < C`" (line 250). That state should be SATURATED.
- **Correction.** Define `N_before` as `N` when the retry machine left Healthy (or when the first Overload of the episode happened). Restore into SATURATED when it equals `C`. Make R9 and case 27 follow the Suspect path.
- **Closure test.** L1: an outage with two Overloads. `N` is restored to the pre-outage value, and the state is SATURATED when that value is `C`.

### [P3-5] Case 26 cannot be met with a 1 s lag, and R7's repair claim holds only for short lags

- **Location.** Case 26, line 463, and R7, line 184.
- **Reproduction.** Y is disposed at 0. X starts at `Ts` = 0.25 s and is refused at about 0.30 s, which is the wrong decrease. The first test comes at 0.30 + `T0` = 0.7 to 0.9 s, inside the fixture's 1 s lag, so it is refused. "The first test at `T0` succeeds once the lag has passed" and "`N` is back within `T0` + one setup time" cannot both hold. R7's claim holds only for a lag ≤ `Ts` + setup + `T0`.
- **Correction.** Assert recovery by the first test after the lag ends, with a bound stated from the corrected P2-1 timer. Restate R7's bound in terms of the lag.
- **Closure test.** The re-stated case 26 passes at lags of 0.5 s and 1 s.

### [P3-6] The steady-limit bound in §4.7 ignores the ±20 % jitter on `T0`, and case 4 can exceed it

- **Location.** §4.7 lines 260, 274 and 275, and case 4 line 438.
- **Reproduction.** With −20 % jitter, `T` doubles from 0.4 s: tests come at 0.4, 1.2, 2.8, 6.0, 12.4, 25.2 and 55.2 s. That is 7 refused tests by 60 s, so 24 + 7 = 31 > 30.
- **A second gap.** Case 4 does not name the refusal class. A Suspect adds the "+1" that §4.7 counts and case 4 omits.
- **Correction.** State the bound with jitter, either from `0.8 × T0` or as "at most one more". Name case 4's class, and specify the injected jitter.
- **Closure test.** Case 4 with jitter injected at −20 % and at +20 %. Both stay within the stated bound.

### [P3-7] The status line and the Changelog's "unchanged" claims are inexact, and one sets review scope

- **Location.** Line 3 ("Surface is not reviewed in this round: the user-visible note (§9) and the optional wire field (§5.3) are unchanged from revision 1"), and Changelog line 567 ("Unchanged: §2.1 to §2.4 (except the retry machine's description …), §7, OQ1 to OQ4, OQ7 to OQ10").
- **What the comparison with revision 1 shows.**
  - §9 changed. The note's trigger is now an Overload, not a confirmed retest. It now prints the settled value and says "limit lifted" on SATURATED. Its TR2.24 names changed from `throttle_events`/`probes` to `overloads`/`inconclusive`/`tests`. Changelog line 563 itself lists §9 as changed.
  - §5.3 changed: the budget now includes tests, inconclusive refusals count as attempts, and `--max-retries 0` keeps evidence.
  - §2.4 changed: the SSH bullet's last sentence is new, and assumption (d) was added.
  - §7.5's row and §7's closing sentence were reworded.
  - OQ1–OQ4 and OQ7–OQ10 were condensed and lost rationale, such as OQ3's "the only path by which a throttle still fails a member" and OQ8's case-1 claim.
  - Revision 1's decision D4 (the final error, "never a bare `Capacity`") was dropped from the decision table without a record.
- **Impact.** Review routing rests on a false premise. The Surface axis is skipped "because" two surfaces it owns are unchanged, when both changed.
- **Correction.** Fix the status line and the Changelog's "Unchanged" list, and record D4's removal or restore it.
- **Closure test.** A section-by-section diff against revision 1 matches the Changelog.

### [P3-8] Code citations in §2.5, §4.9 and F10 are inexact

- **Location.** §4.9 line 298 and F10 line 497; §2.5 line 80; §1 line 13 (the SHA scope of citations).
- **The citations.**
  - `connected` is cited as in `pool/lifecycle.rs`. It is defined at `pool/allocation.rs:136` (`ff6083b5`).
  - §2.5 says "A connection the server closes is reported through `idle_closed`". `idle_closed` accepts only `Idle` or `Closing` entries (`lifecycle.rs:207`). A leased connection lost by the server goes through `release`, then `start_closing`, then `closed()`. The settle hold of §4.5 and §4.9 would then apply to it, against §4.1's "Gone at once".
  - §1 assigns SHAs only to §2.1–§2.4, §7 and §2.5. §4.1's `https_endpoint.rs:381-385` (`Retries::remove`) is right at `279860c1` but not at `db0f8447`, where the call is at `:392`.
- **Correction.** Fix the paths, state the leased-loss path and its settle rule, and give one SHA for every citation outside the named sections.
- **Closure test.** Each citation resolves at its stated SHA.

### [P3-9] §5.1 says the classifier "keeps its three verdicts", but its table adds a fourth

- **Location.** §5.1 line 313 against lines 317–318 ("new **Requeue**").
- **Impact.** It is unclear whether `Verdict` gains a variant, which matters for the retry plan's S3.2 "one function". This changes accepted text (see P2-5).
- **Correction.** Say which: a fourth verdict, or Retry plus evidence routed to the endpoint.
- **Closure test.** §5.1 has one consistent statement.

### [P3-10] Appendix A reads the operator's "reset timer" as backing off, and this reading is not put to the operator

- **Location.** Appendix A line 550 and §4.7 line 261, against the operator's edge `PROBING -> STABLE [label="Overload / 429\n(Drop probe, reset timer)"]` (line 541).
- **Impact.** Doubling departs from the operator's words. It is disclosed, but OQ13 asks the operator to confirm (a), (b) and (c) and does not include this reading, so the operator is never asked to confirm it.
- **Correction.** Add it to OQ13 as (d).
- **Closure test.** OQ13 lists the reading.

### [P3-11] Effects on the 1.2.0 reuse design go unstated (§4.1, §4.9, §6)

- **Location.** §4.1 line 124 (one machine per operation), §4.9 line 297 (`set_limit(key, n)` per (scheme, host, port) on the pool), and §6 line 380 ("held by the session that owns the connections").
- **The conflict.** The reuse design (§5 line 124 onward) makes limits per binding, travelling with each request on a pool instance shared across overlapping operations.
  - Two operations' machines would write one per-key pool limit, and the last writer wins.
  - Each window ignores the sibling operation's connections.
  - The design cites neither document.
- **Correction.** Name the reuse design §5 and §9 and CS7.23 as affected in 1.2.0. Either state how a per-operation `N` maps onto per-binding limits, or defer D9 to that design's next revision.
- **Closure test.** §6 cites the reuse design and states the mapping or the deferral.

## 2. Invariant analysis

- **The machine's fixed points.** Two definitions are wrong, and the race model works around both:
  - "Conclusive" is defined as `hi < N`. That is correct for ordinary starts and wrong for tests, whose fair refusal is `hi = N` (P2-1).
  - The confirmation level `k` comes from `hi`. For a first wave, `hi` is the wave size, not the server's limit (P2-2).

  The window sets themselves (`S_lo` and `S_hi`, §4.3) are sound as bounds. The R1 handling (`Possible` counts Closing and Settling, plus the pool's settle hold) is consistent with §2.5's verified code facts.
- **Retry machine.** The semantic change in §5.5 is clear. Its bound (P2-3) and its supersession inventory (P2-4) are not. The Requeue/`abandoned` routing changes the retry plan's §4 accounting and OD18's Cold exit, and the document does not acknowledge this (P2-5).
- **Controlling graph.** §1 claims to touch only retry plan §4 and §5. The actual touch set also includes:
  - retry plan §4 (HTTPS sentence), §5 (several clauses), §10 and S3.1;
  - amendment 2 §3.20 (Cold text, S3.1 rewrites, hazard);
  - HTTPS design §6 and §7;
  - reuse design §9 and session plan CS7.23.
- **Numbers.** Several hold: §4.7's (C − L) + 1 + 6 = 31; case 4's 24 + 6 + 0 = 30 at zero jitter; case 5b's 0.5, 1.5 and 3.5 s at ±20 % against a 2 s burst; the 127.75 s figure as the retry plan's per-key number. Three do not: the 127.75 s figure as a per-member claim (P2-3), case 4 with jitter (P3-6), and case 26 with a 1 s lag (P3-5).
- **Case 22.** It is satisfiable in L1: forcing admission while Y is Closing gives `hi = 8 = N`, so the refusal is inconclusive, as stated.
- **Code citations new in revision 2.** All are substantively accurate, apart from the items in P3-8.

## 3. Risks and next action

- **Highest risk.** P2-1 and P2-2 together mean that an implementation faithful to the text, in the operator's main scenario (a server limit below 32, especially SSH `MaxStartups`, which is always Suspect), would refuse in tight loops and fail members on budget. That is the outcome the design exists to prevent. Both fixes are local definitions: the test-refusal input, and `k` from `Connected`.
- **Next action.** Revise §4.4–§4.7 for P2-1 and P2-2 and re-derive §4.7's bounds and cases 2, 3, 4 and 26. Correct the §5.5 bound (P2-3). Add one superseded-clauses section that covers §5.1, §4.8, §5.4 and §5.5 against the retry plan, OD18, the HTTPS design, the reuse design and CS7.23 (P2-4, P2-5). Fix the status line and Changelog (P3-7).
- I pre-commit to GO on a revision that resolves P2-1 to P2-5 as specified. The P3s should be fixed in the same revision, but they do not block.
