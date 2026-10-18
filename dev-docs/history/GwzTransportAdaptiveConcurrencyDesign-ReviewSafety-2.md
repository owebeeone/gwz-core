# GWZ transport adaptive concurrency design, revision 3: safety-axis re-review (round 2)

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, the working-tree file. Revision 3, DRAFT ("under review"), 685 lines, sha256 `ee5b9258b9de1532e9f2296b6473ac478ec1813e3b06048fe3f3a5943172699f`. It is uncommitted. Re-reviewed 2026-10-07.
**Baseline:** root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`.
- Revision 2 was diffed from the saved copy, sha256 `19565018…`.
- The remediation plan read was `GwzTransportAdaptiveConcurrencyDesign-RemPlan.md`.
- Sources were read with read, grep, sed and diff only.
- The tuple was checked at the start and at the end and did not change.

**Date:** 2026-10-07
**Axis:** Safety: what the text allows to go wrong. This covers stuck states, hammering, the claim of no regression against the status quo, push replay, disclosure, and the race model. The review is independent, adversarial and read-only, and nothing here relies on the other axes. The lane owner files it verbatim.

**Verdict: NO-GO.**
- All 15 round-1 findings are closed on revision 3.
- The changed ranges contain new findings: 0 P0, 0 P1, 4 P2 (P2-9 to P2-12) and 2 P3 (P3-8, P3-9). None is ARCHITECTURAL.

I pre-commit to GO on a revision that resolves P2-9 to P2-12 as specified, with each P3 either fixed or recorded as accepted by the operator.

---
## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 3 | Status |
|---|---|---|---|
| P2-1 | Each attempt is judged against its own target. A probe test is fair iff `lo = N`, and a fair refusal backs `T` off. | §4.4 table (lines 180–184), the §4.6 edges, the §4.7 table. I re-traced the steady-limit-8 case: a test starts only when `Connected = N` and the key is quiet, its refusal has `lo = 8` and is fair, and `T` doubles. An unfair test needs a connection to leave during its window, which happens at most once per dead connection. Every `Retry-After` sets the hold, whatever the judgement (line 193). | Closed |
| P2-2 | No fill. `k := Connected + 1` once the key is quiet, and `k = 1` goes to the retry machine. | §4.5 rule 3 (lines 228–235). In the limit-8 reset case, the confirmation holds starts until the wave resolves, then tests at 9 with a fair refusal: Overload, `N = 8`, members charged once. In the RST outage, Connected is 0, so the test at `k = 1` goes to the retry machine and its waits apply. The fill is gone. The path where the confirmation closes because no carrier is left is a separate root cause: **P2-11**. | Closed |
| P2-3 | Routing: with Connected = 0 a failure goes to the retry machine; while anything stays Connected, the limit timer is the backoff. | §4.8 (lines 315–328). I re-traced the case of one long clone with new connects refused: confirmation, `k = 2`, a fair refusal, `N := 1`, and the T0-doubling tests are the backoff. R9 (line 200) matches the rules. | Closed |
| P2-4 | Down replaces Closed on exhaustion: one retest per 30 s, and later arrivals finish at once. | §5.5 (lines 406–418). Under `--jobs 1` against a dead host: 4 handshakes, then the members drain before `retest_at`. Handshakes are at most the wave + `R` + `ceil(elapsed / 30 s)`. The hammering and the `--jobs 1` rationale are closed. A new defect sits in the same range: **P2-9**. | Closed |
| P2-5 | A test-pending sub-state drives the key to quiet. | §4.5 "Test pending" (lines 217–223). I re-traced two alternating identities at `N = 8`: the fill stops at the base, evictions are held, the key goes quiet, `Connected = N`, and the test runs. Under `--ssh-timeout 0` the wait is not bounded (**P3-9**). | Closed |
| P2-6 | A leased window runs from lease to result; any `Retry-After` sets the hold. | §4.3 (lines 159–160), §4.4 line 193, §4.5 Holds. A keep-alive 429 with `Retry-After: 30` holds the key, and case 36 pins that no discovery is sent during the hold. | Closed |
| P2-7 | At `R = 0`, no refusal lowers `N`. | §5.3 line 391, case 11. Closed for `N`. The setup limit `Ns` is not covered: **P2-12**. | Closed |
| P2-8 | The restore is a tested doubling by non-final carriers, not a jump. | §5.5 line 420. The untested jump and its admission of final-attempt members are gone. The judgement of the restore's steps is undefined: **P2-10**. | Closed |
| P3-1 | `N_good` is the value at the last success before the retry machine left Cold or Healthy. | Line 420, case 27d. A cascade of Overloads no longer matters. | Closed |
| P3-2 | `N = C` by any route means SATURATED. | §4.6 line 276, §6, case 37. | Closed |
| P3-3 | No settle wait in SATURATED. | §4.5 line 211, §4.9, §8, §10.3 row 5 with mixed identities. The no-throttle wait is gone. A related claim about the race in SATURATED is overstated (**P3-8**). | Closed |
| P3-4 | An HTTP-date is taken relative to the response's `Date`. | §4.5 Holds, §5.1, case 38. | Closed |
| P3-5 | The detail and the note name the configured host, and omit the host after a cross-host redirect. | §5.3 line 386, §9, case 39. | Closed |
| P3-6 | §14 lists the clauses that widen retry. | §14 items 2, 14 and 15, and §5.4. | Closed |
| P3-7 | The edges use `max`. | §4.6 diagram (lines 254–264), §4.2. | Closed |

## Changed-range analysis

1. **The Down state (§5.5).**
   - Handshake rate: at most one retest per 30 s. Bounded.
   - `--jobs 1`: holds.
   - The retest carrier makes a first attempt with fresh clocks. Bounded.
   - Arrivals that finish at once: in a workspace where the host's remaining members arrive within 30 s, Down drains every remaining member in milliseconds. It then behaves exactly like the Closed state this revision set out to remove: **P2-9**.
   - §5.2 line 375 (members wait in Down until `retest_at`) contradicts §5.5 line 410 (they finish at once).
2. **The restore by doubling.** The text gives no judgement rule for starts above `N` during a restore step. Under the only rule that applies, which is §4.4's ordinary-start rule, their refusals are always inconclusive. So "a conclusive refusal … ends it" cannot fire, and a partly refused step has no defined outcome or backoff: **P2-10**.
3. **The quiet wait of the test-pending rule.** It is bounded by setup clocks, the cleanup allowance and `Ts`, except under `--ssh-timeout 0`, where one hung setup holds every new start on the key indefinitely: **P3-9**. I found no starvation and no livelock with the clocks enabled. Step 3's "base no longer holds → resume" cannot loop faster than connections are lost.
4. **SATURATED admits without settle waits (R1 there).** When Possible is at `C`, a refusal is inconclusive, as the design says. When Possible is below `C`, a server limit that is still counting the client's own Settling connection produces a conclusive refusal, and `N` is lowered by one below the true limit. D5 and line 211 say this cannot happen: **P3-8**. The probe repairs it.
5. **The confirmation's hold with no fill.** The quiet wait is bounded. The "closes with nothing changed" path re-admits at the width that was just refused, with neither of the two waits that §4.8 promises: **P2-11**.
6. **`--max-retries 0`.** Fixed for `N`. The setup limit `Ns` (§4.10) still lowers at once with no confirmation, and at `R = 0` no test can ever raise it again: **P2-12**.
7. **§14's replacements.**
   - Items 1–16: I checked each against its source and found the clauses cited and the replacements coherent with §5.5.
   - Item 12's count, the per-host limit plus 4 handshakes before Down, agrees with the flow: the wave's failures are Requeue, the `k = 1` test is attempt 1, then 3 probes.
   - I found no clause that widens a push path. A POST is still never requeued (§5.4), and `Effect::Possible` is preserved.
8. **The setup limit `Ns` (§4.10, rewritten).**
   - `Ns := max(1, lo_s)` depends on the order in which setups started.
   - In a wave whose start order matches the server's accept order, it settles near the server's `MaxStartups` start value. When the orders differ, it under-estimates, which is safe and is tested back.
   - I found no demonstrable over-admission. Not filed.
9. **Disclosure.** No new field or text beyond §5.3 and §9, which are now configured-host only. Nothing found.

---
## 0. Evidence base

- **Object:**
  - Revision 3 read in full.
  - Revision 2 → 3 diff: 680 lines of diff output.
  - Remediation plan: all dispositions.
- **Controlling documents:** as in round 1.
  - Retry plan §§4–6, including line 202, lines 209–212 (the `--jobs 1` rationale), line 266 (the per-key bound), and §6 ("Each worker pulls the next member whose hostname still has a free `--max-per-host` permit").
  - Amendment 2 §3.20 (OD18).
  - HTTPS design §§2, 6, 7.
- **Code used as evidence** (unchanged since round 1):
  - `setup_retry/machine.rs:104-243`: `abandoned` frees the Degraded probe slot, and `Closed` returns `Finish`.
  - `pool/allocation.rs:10-40`: reuse before create, same identity.
  - `https_worker/prepare.rs:176-204`: pool key after a redirect; `HttpsScoped` identity.
- I ran nothing and modified nothing.

## 1. Findings

### [P2-9] Down drains the rest of a single-host workspace within milliseconds of exhaustion; the transient-outage defect §5.5 names is not fixed

- **Location:**
  - §5.5 line 410: a member that selects the key before `retest_at` "is finished at once with the recorded failure".
  - Line 404, the defect §5.5 targets: "fails the rest of a long command on that host, however soon the host returns".
  - §1 line 18 and D8: "retested at least every 30 s instead of being closed for the rest of the command".
  - §11 line 546: "as 1.0.17's would fail on their own connect".
  - Contradicted by §5.2 line 375, under which members wait "in a hold (… or Down until `retest_at`)".
- **Violated invariant:** The operator's direction that a transient failure must not become a permanent degradation, and the design's own claims at §1 line 18 and §5.5 line 404.
- **Reproduction:**
  1. 200 members, all on github.com. `C = 32`, `--jobs` 100, `R = 3`.
  2. The host is unreachable from t = 0 to t = 10 s, with every setup failing at `hi = 0`.
  3. The queued members receive the failure of attempt 4 at about 7 s plus 4 attempts (well under 10 s with fast RST failures). The key goes Down with `retest_at` at about t + 30 s.
  4. `par_map_per_host` (retry plan §6) pulls the next member of the host as each one finishes. Each new member selects a key that is Down before `retest_at` and finishes at once.
  5. The other 168 members are pulled and failed within milliseconds, long before `retest_at`. Nobody remains to carry a retest.
  6. The host returned at 10 s, yet the command fails every member on it. That is the same outcome as the Closed state this revision replaces.
  7. 1.0.17 would have served every member that started after 10 s, so §11's comparison is false.
- **Impact:** For the common single-host workspace, Down is Closed with extra steps. A ten-second outage fails the whole remainder of the command. The retest helps only members that happen to select the key 30 s or more after exhaustion.
- **Required correction:** Choose and state a policy that does both: keeps a retest moving the drain, and keeps the dead-host bound. For example:
  - A member that selects a Down key parks, with its allocation clock stopped and a wait of at most 30 s, until the next retest.
  - All parked members share that retest's result.
  - A failed retest finishes them, and only arrivals after a failed retest finish at once (or after K failed retests).
  - State the `--jobs 1` cost of the chosen policy in OQ12, and reconcile §5.2 line 375 with §5.5.
- **Closure test:** A new L1/L2 case: 200 members, `C = 32`, an outage from 0 to 10 s. The members that start after the host returns succeed. Handshakes per key during Down are at most one per retest interval. The case 32 `--jobs 1` bound is re-asserted under the new policy.

### [P2-10] The restore's steps have no judgement rule; their refusals are always "inconclusive", so the restore has no exit and no backoff on a real limit

- **Location:**
  - §5.5 line 420: "each step admits up to `min(N_good, 2 x max(1, Connected))` … a step whose starts all succeed raises `N` … a conclusive refusal during the restore is an ordinary Overload and ends it".
  - §4.4 table (lines 180–184), which has rows only for ordinary starts (target `N`), probe tests and confirming tests.
  - §4.6 line 283: in DISCOVERING, "Ordinary starts fill to `N`".
- **Violated invariant:**
  - §4.8 line 328: "A refusal is never retried without one of the two waits".
  - §4.7's bounded refused attempts.
  - The stated outcome that a real limit is "found again by the first refusal".
- **Reproduction:**
  1. `N_good = 32`. The outage leaves `N = Connected = 4`. The host returns with a real limit of 4.
  2. Step 1 admits 8: 4 new starts, carried by non-final members. All are refused.
  3. Each refusal's `hi` is at least `4 + 3 = 7`, which is at least `N = 4`. Judged as an ordinary start, as §4.4 requires, every refusal is inconclusive: no input, requeued, charged.
  4. No refusal is conclusive, so "ends it" never fires. A step that did not "all succeed" has no defined next action.
  5. If the step is re-admitted, the same 4 refused starts repeat with no timer, until every queued member is on its final attempt.
  6. The design's own bound, "at most the step's new starts per step", says nothing about how many steps there are.
- **Required correction:**
  - Judge a restore start against the step's target: a refusal with `hi < target` is an Overload, with `N := max(1, Connected)`, STABLE, and `T := T0`.
  - Define a step with an inconclusive or unfair refusal: end the restore in STABLE at the current `N`, with `T := T0`.
  - Add a §4.4 table row for restore starts.
- **Closure test:** Case 27b extended to a limit equal to the post-outage `Connected`. Exactly one step is refused, the machine is in STABLE with `T = T0`, and the next start above `N` is a timed test.

### [P2-11] When the confirmation closes because no carrier is left, starts resume at the width that was just refused, with no wait and no test, and final-attempt members fail

- **Location:**
  - §4.5 rule 3, line 235: the confirmation "closes with nothing changed when no member that may carry it is queued (every queued member is on its final attempt …)".
  - §4.8 line 328: "A refusal is never retried without one of the two waits".
- **Violated invariant:**
  - §4.8 line 328 itself.
  - The operator's direction that a limit is learned by testing, not by failing members.
- **Reproduction:**
  1. `--max-retries 1`. HTTPS resets at a server limit of 8. 32 members in flight on the host (`par_map_per_host` caps it at `C = 32`, so no member is fresh).
  2. In the wave, 8 succeed. 24 are reset, which is a conclusive Suspect, and each is charged attempt 1, so its next attempt is its final one.
  3. The confirmation opens and holds starts until the key is quiet.
  4. Every queued member is now on its final attempt, so the confirmation closes with nothing changed.
  5. Admission resumes at SATURATED's `C`. All 24 start at once against a server holding 8. All are refused, and all fail as "throttled".
  6. The same happens at any `R` late in a command, whenever the only queued members have spent `R` attempts.
  7. Contrast the Throttle path in the same state: an Overload sets `N := 8`, and the 24 queue and succeed.
- **Required correction:** A closure for want of a carrier must not resume at the refused width. Options:
  - Let a final-attempt member carry the confirming test at `Connected + 1`, which risks at most one member per confirmation.
  - Or keep the hold for one `T0` and re-open the confirmation on the next refusal, with that hold as the wait.
  - State which, and correct line 328's invariant.
- **Closure test:** An L1 case with `R = 1`, a reset limit of 8 and 32 members. No member fails because of a re-admission after the confirmation closes, and no start re-offers more than `Connected + 1` before the Suspect is resolved.

### [P2-12] The `--max-retries 0` rule covers `N` only; `Ns` is still lowered at once and can never be tested back

- **Location:**
  - §5.3 line 391: "no refusal lowers `N`".
  - §4.10 line 347: a drop before authentication is "an overload of the setup limit at once, with no confirmation".
  - §4.10 line 348: `Ns` uses §4.7's tests, which §4.7 lines 300–302 forbid on a final attempt.
- **Violated invariant:**
  - The operator's direction that a transient failure must not become a permanent degradation.
  - The design's own rationale at line 391: "the operator's direction forbids an untested [decrease]".
- **Reproduction:**
  1. `R = 0`, an SSH workspace of 200 members, `C = 32`.
  2. One network blip resets two setups at the banner with `hi_s ≥ 1`. That is conclusive, so `Ns := max(1, lo_s)`, for example 2.
  3. At `R = 0` every attempt is final, so no setup test is ever carried.
  4. The successes' rule `Ns := max(Ns, lo_s + 1)` cannot rise above what admission (setups in flight `< Ns`) allows.
  5. The remaining members set up at most 2 at a time for the rest of the command. That is the permanent degradation round-1 P2-7 removed for `N`.
- **Required correction:** Extend §5.3's `R = 0` rule to `Ns`: a drop before authentication never lowers `Ns` at `--max-retries 0`.
- **Closure test:** Case 11 extended to an SSH drop before authentication at `R = 0`. `Ns` stays at `C`.

### [P3-8] D5's claim that a race in SATURATED "is inconclusive, not a decrease" is false whenever Possible is below `C`

- **Location:** D5 line 28; §4.5 line 211; R1 line 194 ("in SATURATED … the decrease is right").
- **Reproduction:**
  1. SATURATED. 20 Connected, plus Y, which the client closed and is Settling. The server's limit is 21 and it still counts Y.
  2. X starts with no settle wait and is refused. `hi(X) = 21 < C = 32`, so the refusal is conclusive.
  3. `N := 20`, one below the true 21. Y's presence in `S_hi` did not make the refusal inconclusive.
- **Impact:** One wrong decrease, repaired by the T0 test. The claim is wrong, and R1's "the decrease is right" is wrong in this case.
- **Correction:** State the residual race in SATURATED with Possible below `C` (one decrease, repaired at T0), or count Settling connections in SATURATED's admission whenever Possible is below `C`.
- **Closure test:** An L1 case of the interleaving above, asserting the stated outcome.

### [P3-9] Under `--ssh-timeout 0`, one hung setup holds every new start on the key indefinitely; "No wait is unbounded" is false

- **Location:**
  - §4.5 line 223: the quiet wait is "bounded by the setups' own clocks".
  - §5.2: behind a due test or a confirmation.
  - §5.3 line 393: "No wait is unbounded".
  - Retry plan §4: with `--ssh-timeout 0`, "An attempt that never returns is the hang that 0 already selects".
- **Reproduction:**
  1. `--ssh-timeout 0`. One setup hangs: the server accepts TCP and never sends a banner.
  2. A reset on another setup is a conclusive Suspect and opens a confirmation, which holds every new start until the key is quiet.
  3. The key is never quiet, because the hung setup never ends. Every member that needs a new connection on that key waits forever, with its allocation clock stopped.
  4. Today, only the hung member hangs.
- **Impact:** The hang spreads to every member on the key. The command already never completes in this mode, but more members are left without results. The bound claim is false.
- **Correction:** Exclude setups with no clock from the quiet condition (count them in `hi` only), or state the exception at line 393.
- **Closure test:** An L1 case with `--ssh-timeout 0`, one hung setup, and a confirmation open. The other members proceed under the stated rule.

## 2. Invariant analysis

| Invariant | Revision 3 |
|---|---|
| A throttle never fails a member while budget remains (spent only on backed-off attempts) | Holds for Throttle and confirmation with a carrier. Breaks on the closure for want of a carrier (P2-11) and on restore steps (P2-10). |
| A limit is tested, and a decrease is never permanent | Holds for `N`. Breaks for `Ns` at `R = 0` (P2-12). |
| A transient failure must not become permanent | Breaks via Down's drain (P2-9). |
| Client-versus-server count lag | Holds outside SATURATED. In SATURATED, one off-by-one decrease is possible (P3-8). |
| `N` never raised above what the server holds; never above `C` | Holds: the `max` rule, SATURATED at `C`, the pool raised only for an admitted test (§4.9). |
| Bounded hammering | Holds: the timer backs off, the confirmation has no fill, and Down allows 1 per 30 s. Exception: undefined restore steps (P2-10). |
| Push safety | Holds: a POST is never requeued, and `Effect::Possible` is preserved (§5.4). |
| Disclosure | Holds: configured host only, counts and milliseconds (§5.3, §9). |
| "Never worse than the status quo" | Holds for no throttle (SATURATED, no settle), the limit case and the `--jobs 1` dead host. Breaks for a transient outage in a single-host workspace (P2-9) and for `R = 1` with a Suspect limit (P2-11). |

## 3. Risks and next action

- **Highest risk:** P2-9. It undoes the purpose of §5.5 for the dominant workspace shape: a single host with more members than `C`. Resolving it needs an operator choice between bounding the drain under `--jobs 1` and serving members after a host returns. The cost of each option must be stated in OQ12. It is text-fixable, not architectural.
- **P2-10, P2-11 and P2-12** are bounded rule completions in §5.5, §4.5 and §5.3 respectively. Each has a closure test specified above.
- **Next action:** a revision 4 patch covering P2-9 to P2-12, the P3s fixed or accepted, and the closure cases added to §10.2. Then a focused Safety re-check of those ranges only.
