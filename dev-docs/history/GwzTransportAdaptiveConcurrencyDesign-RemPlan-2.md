# GWZ transport adaptive concurrency design: remediation plan for revision 3 (round 2)

Date: 2026-10-07. Object: `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md` revision 3, sha256 `ee5b9258b9de1532e9f2296b6473ac478ec1813e3b06048fe3f3a5943172699f`. The re-verdicts are filed verbatim beside it:

- `-ReviewConsistency-2.md`: NO-GO. All 16 round-1 findings closed. New: P2-6, P2-7 and P3-12 to P3-18. Pre-commitment to GO on P2-6 and P2-7.
- `-ReviewSafety-2.md`: NO-GO. All 15 round-1 findings closed. New: P2-9 to P2-12, P3-8 and P3-9. Pre-commitment to GO on P2-9 to P2-12, with each P3 fixed or accepted.
- `-ReviewState-2.md`: NO-GO. All 8 round-1 findings closed. New: P2-6 to P2-8 and P3-4 to P3-6. Pre-commitment to GO on P2-6 to P2-8.

No reviewer classed any finding as architectural. This is the **second and last remediation round** under the two-round cap. Revision 4 is one patch. Prefixes: C (Consistency), S (Safety), T (State).

**Blind convergence:** C P2-6 = S P2-10 = T P2-7. All three reviewers found independently that the restore of `N` had no judgement rule, no pool limit and no exit.

**Operator decision (2026-10-07), for S P2-9:** the operator was asked how members that arrive at a Down key should be handled. The answer was "Park, give up after 2".

## Dispositions

| Finding | Disposition in revision 4 | Closure test |
|---|---|---|
| **C P2-6, S P2-10, T P2-7** | §4.4 gains a fourth row, the **restore step start**. Its target is `S := max(N, min(N_good, 2 x max(1, Connected)))`, and a refusal is conclusive iff `hi < S` and goes through the evidence filter. §4.9 sets the pool's limit to `S` for the step. §5.5 defines a step: it is complete when every start has a result, all successes lead to the next doubling, and a refusal with no Overload ends the restore in STABLE at `T0`. Any Overload ends it. Reaching `N_good` gives SATURATED or STABLE. §4.6 adds the restore's entry and states that DISCOVERING's climb does not apply during it. | Case 27b re-derived: limit 6 after the outage, one conclusive refusal, a confirmation at 7, `N = 6`, the pool limit at 8. Case 27e: the first step targets 4, not 2. |
| **S P2-9** | §5.5 Down follows the operator's choice. Arrivals park for the next retest (at most 30 s, allocation clock stopped) and share its result. After 2 failed retests in a row, arrivals fail at once, while a retest still runs every 30 s. §5.2, §1, D8, §11, §14 items 3, 4, 6, 9 and 13, and OQ12 are reconciled. | Case 44 (a 10 s outage, 200 members: members after exhaustion park and succeed). Case 29 restated. Case 32 (`--jobs 1`: about 60 s more, then a fast drain). |
| **S P2-11, C P3-14** | §4.5 rule 3. Carriers are counted in aggregate (queued members less the idle connections their identities can lease). The confirmation never closes for want of a carrier, and while there are none its hold applies only to new setups. A final-attempt member carries the confirming test when no other can, the one exception to §4.7's carrier rule (stated there). | Case 45 (`R = 1`, a reset limit of 8: a final-attempt carrier, `N = 8`, 23 succeed). Case 2 run with short and long fetches. |
| **S P2-12** | §5.3 and §4.10: at `--max-retries 0`, a drop before authentication never lowers `Ns`. | Case 46. |
| **T P2-6** | §4.1: for HTTPS, a connection is Connected only once its first exchange is answered with a status other than 429 or 503. §4.2 and §4.5 rule 4: an overload sets `N := max(1, min(Connected, hi(a)))`. R3 and R10 are restated. §4.5's hold also holds new exchanges on leased connections. | Case 43 (a request-level limit of 8 on new and reused connections: `N = 8` exactly, no start at 9 but the probe). |
| **T P2-8** | §4.10: `Ns` is taken when the drop's window has resolved, as the number of co-setups that succeeded, with new setups held meanwhile. Later drops of the wave are no input. | Case 3 re-derived (`Ns = 10` exactly, 22 refused setups, at most 2 attempts per member; drops-first order). |
| **C P2-7** | §14 gains item 17 (retry plan §4, lines 188–190, "after the session is reusable"), item 18 (amendment 2, line 371, the OD18 decision record) and item 19 (amendment 2, line 546, the `MaxStartups` paragraph). All three are verbatim, with replacements. | The closure-test search terms of C P2-7 all hit §14. |
| **C P3-12** | §4.4: after an inconclusive refusal, the next start waits for the connections that were Closing or Settling in its window to settle, in SATURATED too. §4.8's claim is corrected. | Case 50. |
| **C P3-13** | §5.5's bound adds the confirming handshake: 36 before Down for a wave of 32. | Case 31. |
| **C P3-15** | §1's Down sentence restated to match §5.5. | Text. |
| **C P3-16** | Case 27a: no restore, SATURATED at `C`. §5.5 states that an outage that lost every connection lowers `N` not at all. | Case 27a. |
| **C P3-17** | Case 33 states the starting count and asserts `hi = 9`. | Case 33. |
| **C P3-18** | The revision-3 Changelog entry gains a correction line, recorded in revision 4. | Diff against revision 3. |
| **S P3-8** | R1 and D5 state SATURATED's residual: with `Possible` below `C`, one decrease to one below the limit, repaired at `T0`. | Case 22b. |
| **S P3-9** | §4.5 step 2: a setup with no clock (`--ssh-timeout 0`) does not keep the key from being quiet. It is counted in `hi` only. | Case 47. |
| **T P3-4** | §4.9: a test's pool request is create-only. | Case 48. |
| **T P3-5** | §4.9: a settling hold carries its evictor's request id, and its lapse serves that request first. | Case 42b. |
| **T P3-6** | §4.6 and §4.10: the `N` and `Ns` tests share one test slot. The `Ns` test runs only when no `N` test is due, and DISCOVERING yields to a due `Ns` test every other turn. | Case 49. |
| **T risks** | F16 (the pool counts by host, not port). §4.5 bounds the quiet wait including the pool's idle expiry. | Text. |

No finding is disputed. Re-verdict: each reviewer is continued with revision 4's hash, this plan and the revision-4 Changelog entry. Revision 3 is saved for diffing.
