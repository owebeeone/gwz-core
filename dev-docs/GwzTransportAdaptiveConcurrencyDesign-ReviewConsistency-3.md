# GWZ transport adaptive concurrency design, revision 4: consistency-axis re-review (round 3)

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, working tree, 714 lines, sha256 `a0188888082ef78e5f83b30894183150a25fa82f2455e1471fa489a79e5d4333`. Status: revision 4 DRAFT, under re-review, uncommitted. This is the second and last remediation round under the two-round cap.

**Baseline:**
- Root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`.
- I checked the object's hash and all three HEADs at the start and at the end, and nothing moved.
- Revision 3 was diffed from the scratchpad copy `adaptive-rev3.md`, whose sha256 `ee5b9258…` I verified. The diff has 133 changed lines.
- Code was read with `git show` and `git grep` at the stated SHAs. The controlling documents were read from the working tree.
- `-RemPlan-2.md` was read as the disposition claim. Every disposition was re-traced against the text.

**Date:** 2026-10-07
**Axis:** Consistency. The document is checked against itself and against the retry plan, amendment 2, the HTTPS design, the reuse design and session plan CS7.23, and the operator's recorded decisions. This review is independent, adversarial and read-only. Other axes run in parallel, and nothing here relies on them. The lane owner files this report verbatim.

**Verdict: GO.** 0 P0, 0 P1, 0 P2, 7 P3 (P3-19 to P3-25, all new).
- Both round-2 P2s (P2-6, P2-7) and all seven round-2 P3s are closed on revision 4.
- No new root cause is ARCHITECTURAL. Every P3 is a bounded text fix.
- The P3s do not block. They should be fixed before the design goes to a plan, and P3-19, P3-20 and P3-22 most of all, because each leaves two normative statements of one rule that disagree.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 4 | Status |
|---|---|---|---|
| P2-6 | A restore step row in §4.4 with target `S`, conclusive iff `hi < S`; the pool limit is `S`; step semantics and exits in §5.5; the restore's entry in §4.6. | §4.4 line 186; §4.9 line 339 (raised "to `S` for a restore step"); §5.5 line 424; §4.6 line 276.<br>**Original counterexample re-traced** (`N = 8`, `N_good = 32`, a post-outage limit of 8): `S = max(8, min(32, 16)) = 16`. The pool limit is 16, so starts 9 to 16 are created. Each refusal has `hi ≤ 15 < 16`, which is conclusive and goes through the filter to an Overload, `N := min(Connected, hi)`, and STABLE. The restore is admitted, judged and ended.<br>**Case 27b re-derived:** `S = 8` from `Connected = 4`. Two of the four starts are refused with `hi = 7 < 8`. One opens a confirmation; the other is a co-refusal and no input. Quiet comes at `Connected = 6`, so `k = 7`. A fair refusal is an Overload with `N = min(6, 6) = 6`.<br>**Case 27e:** `S = max(4, 2) = 4`. ✓ | **Closed.** A residual disagreement between §4.6's and §5.5's exits is filed as P3-22. |
| P2-7 | §14 items 17, 18 and 19. | Item 17 is verbatim with retry plan lines 188–190. Item 18 is verbatim with amendment 2 line 371 (the fragment "and the retry plan's single-probe rules apply from the wave's first retriable failure."). Item 19 is verbatim with line 546. The replacements agree with §4.8, §4.10 and §5.4.<br>**Search terms re-run:** "after the session is reusable" hits item 17. "single-probe" hits items 11 and 18. "retriable setup failure" hits item 19. "not retriable" is covered by item 17 (the list's only bullet the design contradicts). "Closed", "attempts remain", "exactly four handshakes", "fresh budget", "one key", "just failed" and "lower `per_host`" are as in round 2. | **Closed** |
| P3-12 | The next start after an inconclusive refusal waits for its window's Closing and Settling connections to settle; §4.8's claim corrected. | §4.4 line 189; §4.8 line 331; case 50. Re-trace at `C = 4`, a server limit of 4 and 300 ms of lag: the restart now waits for Y to settle, so the 50 ms burn loop is gone. | **Closed** |
| P3-13 | 36 handshakes before Down. | §5.5 line 422 ("plus one confirming handshake when the wave had more than one setup … 36 at the defaults"); case 31; it agrees with §14 item 12. | **Closed** |
| P3-14 | Carriers counted in aggregate. | §4.5 line 236: queued members less the idle connections their identities can lease. Case 2 with a short fetch: 24 queued, 8 idle, so 16 carriers; the test runs at 9. The exception it adds, a final-attempt carrier, creates a new contradiction: P3-20. | **Closed** |
| P3-15 | §1's Down sentence restated. | §1 line 18 is fixed. The same "every 30 s" claim survives in D8: P3-21. | **Closed at §1** (residual filed as P3-21) |
| P3-16 | Case 27a: no restore, SATURATED. | Case 27a; §5.5 line 424 ("An outage that lost every connection lowers `N` not at all"). | **Closed** |
| P3-17 | Case 33 starts with 7 others and asserts `hi = 9`. | Case 33, line 523. | **Closed** |
| P3-18 | A correction line in the revision-3 entry. | Line 701 lists §3.1, Appendix A, and OQ1–OQ4 and OQ7–OQ10. The revision-4 entry (lines 703–714) matches the diff, apart from §14's new header sentence, which is immaterial. | **Closed** |

## Changed-range analysis

- **§4.4 restore row and §5.5 restore.** The judgement and the pool limit are now defined and re-trace correctly (see the table above). §4.6 line 276 states the exits differently from §5.5 (P3-22). §4.8's routing table and the evidence filter's heading (line 226, "a conclusive refusal of an ordinary start") do not name restore starts. §4.4's row does route them through the filter, so the gap is only a cross-reference, recorded under P3-22.
- **§4.1, HTTPS Connected at the first exchange answered with a status other than 429 or 503.** This agrees with §4.2's new value, R3 and R10, and case 43. It is kept apart from the retry machine's `FirstConnect`, as line 136 states. ✓
- **§4.2 and §4.5 rule 4, `N := max(1, min(Connected, hi(a)))`.** Consistent in prose. The §4.6 diagram still uses the old formula (P3-19).
- **§4.5, confirmation carrier.** The aggregate count is consistent. The final-attempt exception contradicts D7 and §5.3 (P3-20).
- **§4.9 pool rules.** A test's request is create-only, a settling hold is bound to its evictor, and the limit can be raised to `S`. Each is consistent with cases 41, 42b and 48. F16 is verified: `counts_for_host` matches on `entry.key.host` only (`pool/machine.rs:256-258`), and `same_user_host` compares host and username (`pool/mod.rs:114-116`).
- **§4.10, the `Ns` estimator.** It takes the count of co-setups that succeeded once the window resolves. That converges as case 3 claims on a server that can authenticate. The scripted layer of case 3 (L3-S) cannot authenticate (P3-23).
- **§5.5, Down parking.** It agrees with §1, §5.2, §11, OQ12 and §14 items 3, 4, 6, 9 and 13. Case 32's "about 60 s" is derivable under `--jobs 1`: the parked member carries each of the two retests. D8 states one thing differently (P3-21). How the operator's four words are read is not put to the operator (P3-25).
- **§14 items 17–19.** Verbatim at their lines (table above).
- **Cases 43–50.**
  - Cases 43, 45, 46, 47, 48, 49 and 50 are derivable.
  - Case 44 stipulates a value the window rule computes, and case 22b leaves out the class it depends on (P3-24).
  - Case 42b is derivable from the evictor-bound hold.

---

## 0. Evidence base

- **Object, revision 4.** I read the full diff from revision 3 and the changed ranges in full: lines 18–35, 125–145, 146–156, 177–240, 255–300, 300–320, 328–356, 375–430, 490–540, 553–600 and 601–714.
- **Retry plan.** Lines 175–212, 226–228, 246–281, 439 and 448–449.
- **Amendment 2.** Lines 371 and 539–547.
- **HTTPS design.** Lines 261 and 322.
- **Reuse design.** Line 201.
- **Code at gwz-transport `ff6083b5`.** `pool/machine.rs` 120–139 and 256–265; `pool/mod.rs` 114–116.
- **Code at gwz-core `db0f8447`.** The header of `ssh_tests/max_startups.rs`: the scripted server writes one line and closes, and never authenticates.
- **Arithmetic re-derived:** the restore target for case 27b (8) and case 27e (4); case 27b's `k = 7` and `N = 6`; case 22b's `min(20, 21)`; case 31's 36; case 32's two parked retests; case 33's `hi = 9`; the convergence of the `Ns` estimator.

## 1. Findings

### [P3-19] The §4.6 diagram still sets `N := max(1, Connected)` on an Overload (new, not architectural)

- **Location.** §4.6 lines 260 and 266, the edges `STABLE -> STABLE` and `SATURATED -> STABLE`. Against them: §4.2 line 151, §4.5 rule 4 (line 237) and D3, which all say `N := max(1, min(Connected, hi(a)))`.
- **Violated invariant.** The machine is stated "with every action stated" (line 243), so an action stated twice must agree.
- **Reproduction.** Use §4.4 R10 and case 43: a 429 on a leased connection, with that connection Connected and excluded from `hi`. The diagram gives `N = Connected`, one higher than §4.2. That is exactly the defect revision 4 fixed for the State axis.
- **Correction.** Relabel both edges `N := max(1, min(Connected, hi(a)))`.
- **Closure test.** Case 43 asserts `N = 8` and is checked against the diagram's labels.

### [P3-20] D7 and §5.3 still say a test is never a member's final attempt; §4.5 rule 3 now makes an exception (new, not architectural)

- **Location.**
  - D7, line 30: "A test is never a member's final attempt."
  - §5.3, line 387: "A test is never given a member's final attempt (§4.7)."
  - Against them: §4.5 rule 3 (line 236, "otherwise by a member on its final attempt") and §4.7 line 304 ("The one exception is a confirming test").
- **Reproduction.** In case 45 (`--max-retries 1`), the final-attempt carrier the case requires is forbidden by D7 and by §5.3's budget rule.
- **Correction.** Say "a probe test" in D7 and §5.3, and add the confirming-test exception to both. Optionally, also say in rule 3 that §5.3's `--max-retries 0` rule (no confirmation opens) takes precedence over the exception.
- **Closure test.** A search for "final attempt" finds no unqualified "never".

### [P3-21] D8 says that after two failed retests "a retest still runs every 30 s"; §5.5 runs one only when a member selects the key (new; the class of P3-15, moved)

- **Location.** D8, line 31. Against it: §5.5 line 422 ("none while none do"), §5.5's retest bullet (a retest is carried by "the first member waiting or selecting the key"), §4.7 line 297 ("No connection is ever opened only to test"), and retry plan §4 line 199 ("A wake with no remaining member does not open a probe").
- **Impact.** The decision table promises a periodic retest that the rules rule out.
- **Correction.** "…while a member that selects it after each 30 s still carries a retest."
- **Closure test.** D8, §1 and §5.5 agree.

### [P3-22] The restore's exits in §4.6 do not match §5.5's (new, not architectural)

- **Location.** §4.6 line 276 ends the restore on "an all-inconclusive or partly refused step **without a conclusive refusal**". §5.5 line 424 ends it on "a completed step with **any refusal and no Overload**".
- **Reproduction.**
  1. A restore step's refusal is a conclusive Suspect, which opens a confirmation.
  2. The step completes before the confirming test's result.
  3. Under §5.5 the restore ends (STABLE, `T0`). Under §4.6 no exit applies, because a conclusive refusal exists and no Overload has happened yet.
  4. If the confirmation is then refuted, §4.6 never ends the restore, and a next doubling step is left undefined.
- **Secondary gap.** §4.8's table and the evidence filter's heading do not name restore starts, although §4.4's row routes them through the filter.
- **Correction.** Use §5.5's wording in §4.6. Add "or a restore step start" to §4.8's Throttle/Suspect row and to the filter's heading.
- **Closure test.** L1: a restore step with a conclusive Suspect whose confirmation is refuted. The restore ends at step completion in STABLE, `T = T0`.

### [P3-23] The scripted half of case 3 cannot produce the co-setups that "succeeded" that §4.10's estimator counts (new, not architectural)

- **Location.** Case 3, line 493 ("`Ns := 10` exactly (the co-setups that succeeded) … the second wave admits 10 setups"). Against it: §10.1 L3-S (line 484, "It cannot authenticate") and §4.10's estimator (line 350, "the number of them that succeeded"). Success for SSH is authentication (§4.1 line 135).
- **Reproduction.** On L3-S, the 10 setups that are not dropped never authenticate. The fixture writes one line and closes, or holds the connection (`max_startups.rs` header). The estimator's count is therefore 0, so `Ns := 1`, not 10, and "the second wave admits 10" cannot happen.
- **Correction.** Run the scripted half on L1 (scripted results) or on L3-R. Alternatively, define the estimator's "succeeded" as "passed key exchange (no longer counted by `MaxStartups`)", give it an event, and note F11.
- **Closure test.** Case 3 names layers able to produce a co-setup's success, and asserts `Ns = 10` on them.

### [P3-24] Cases 22b and 44 state premises the rules either compute differently or leave open (new, not architectural)

- **Location.** Case 22b, line 512, and case 44, line 534.
- **Reproduction.**
  - **Case 22b** does not name the refusal's class. As a Throttle, the decrease to 20 follows. As a Suspect, the confirmation waits for quiet, which needs Y to have settled. After `Ts` (250 ms, longer than the 100 ms lag) its test at 21 succeeds, the Suspect is refuted, and there is no decrease. The asserted `N := 20` holds only for a Throttle.
  - **Case 44** stipulates "every setup refused with `hi = 0`" for 200 members at `C = 32`. `hi` is computed from the window, and the first wave's refusals have `hi = 31`. They go the Suspect path (a confirmation, then `k = 1`), as case 31 shows. The handshake and timing assertions inherit the wrong premise.
- **Correction.** Name 22b's class as a Throttle, or assert the Suspect outcome. Restate case 44's outage as "every setup refused", with the wave-then-confirmation path of case 31.
- **Closure test.** Both cases are re-derived from §4.4 and §4.5 as written.

### [P3-25] The operator's "Park, give up after 2" is read one way and not put back for confirmation (new, not architectural)

- **Location.** D8, §5.5 lines 412–417, OQ12 line 592, RemPlan-2 line 13.
- **The ambiguity.** The answer allows two readings:
  - **(a) key level, the reading the design adopts:** each parked member shares one retest, and after two failed retests in a row new arrivals fail at once;
  - **(b) member level:** a parked member waits through up to two retests before it fails.

  The design adopts (a) without recording the question the operator was asked, and without listing (b) the way OQ13 lists its readings of the operator's machine.
- **Impact.** The decided outcome is deferred and not in dispute. What is in scope is whether the statement can be checked against the decision, and today it cannot.
- **Correction.** Record the question as it was asked, and add an OQ13-style line: "Read as (a); (b) would …".
- **Closure test.** OQ12 or OQ13 carries the reading.

## 2. Invariant analysis

- **D4, judgement per admission target.** It now holds for all four kinds of start (ordinary, probe, confirming, restore), and each has a pool limit in §4.9. The only remaining slip in how an Overload sets `N` is the stale diagram (P3-19).
- **Carrier rule.** Counting in aggregate removes round 2's moot gap. The final-attempt exception is deliberate, but it is not carried into D7 and §5.3 (P3-20).
- **Down.** §5.5, §14 items 3, 4, 6, 9 and 13, §1, §5.2, §11 and OQ12 agree with each other. D8's "every 30 s" is the outlier (P3-21).
- **Controlling graph.** With items 17–19, §14 covers every clause my search terms find in the retry plan, amendment 2, the HTTPS design and the reuse design. I found no further contradicted clause.
- **Test satisfiability.** All cases are derivable except the three noted: case 3's scripted layer (P3-23), and cases 22b and 44 (P3-24).

## 3. Risks and next action

- **Residual risk.** It is low on this axis. P3-19, P3-20 and P3-22 each leave two normative statements that disagree. An implementer following the minority statement would fail cases 43, 45 or the restore case, so the cases catch it. They should still be fixed before planning.
- **Architectural classification.** No new root cause in this round is architectural. All seven are local text changes.
- **Next action.** Apply P3-19 to P3-25 as specified, either in the acceptance commit or as cleared-without-round corrections recorded in the verdict. Then run the Surface review the status line requires before acceptance.
