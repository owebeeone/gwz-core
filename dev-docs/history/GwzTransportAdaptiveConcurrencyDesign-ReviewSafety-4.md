# GWZ transport adaptive concurrency design, revision 5: focused safety re-check

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, the working-tree file. Revision 5, DRAFT ("under focused re-check"), 724 lines, sha256 `e65043e667817d4c18842d89aa9442fd97357dd9472e2ed3ff0a99d85b94ff7e`. It is uncommitted. Re-checked 2026-10-07.
**Baseline:** root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`.
- Revision 4 → 5 was diffed from the saved revision 4 (sha256 `a0188888…`); the diff is 82 lines and 18 hunks.
- The corrections plan read was `GwzTransportAdaptiveConcurrencyDesign-RemPlan-3.md`.
- The tuple was checked at the start and at the end and did not change.

**Date:** 2026-10-07
**Axis:** Safety: what the text allows to go wrong. The scope is the revision 4 → 5 diff only. The review is independent, adversarial and read-only, and nothing here relies on the other axes. The lane owner files it verbatim.

**Verdict: GO.**
- P2-13 and P3-10 are closed on revision 5.
- The diff contains one new finding: P3-11. It is NOT ARCHITECTURAL, and it is a bounded robustness gap that is no worse than the status quo.
- No P0, P1 or P2 is open.

---
## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 5 | Status |
|---|---|---|---|
| P2-13 | §4.5 Holds: (a) a hold stops new connections and an open's first exchange (its discovery) only; a member past its discovery continues; (b) after a hold longer than 1 s, the key's idle connections are discarded rather than leased. | Line 214. See the re-trace below. Case 51 asserts the reproduction's conditions: keep-alive 5 s, `Retry-After: 10`, 5 members between discovery and POST, 10 idle connections, and no member fails. | Closed |
| P3-10 | §4.10: a setup with no clock is not waited for in the `Ns` window, and counts as not succeeded. | Line 350. See the re-trace below. Case 52 asserts it. | Closed |

**P2-13 re-trace.**
1. Keep-alive 5 s. A 429 with `Retry-After: 20` sets the key's hold. Members B to F are between `info/refs` and their POST.
2. By (a), B to F are not held. Their next exchange follows at once on the connection they release and re-lease, or on an idle one that is still fresh, so nothing they use idles across the hold.
3. If a continuing member's POST finds no idle connection, it needs a new connection, which the hold still stops. It waits with its allocation clock stopped (§5.2). When the hold ends, (b) means the idle connections are discarded rather than leased, so it creates a fresh connection.
4. At `t + 20 s`, the requeued discovery opens and the waiting POSTs meet no idle connection, because all were discarded (Closing, then Settling). They create fresh ones within admission: `Possible < N` outside SATURATED, with the settling hold; SATURATED's rule otherwise.
5. No lease lands on a connection that idled through the hold, so the F14 failure path is not reached.
6. The discards are client closes, so their windows count them in `hi`. A refusal right after the hold is therefore judged as R1 and never produces an unaccounted decrease.
7. Push: a held receive-pack POST waits before its first byte, so `Effect::None`, and nothing is replayed (§5.4).

**P3-10 re-trace.**
1. `--ssh-timeout 0`. A wave contains one setup hung after TCP accept, and a drop before authentication on another setup, with `hi_s ≥ 1`.
2. The window resolves when every *clocked* setup in `S_lo_s` has a result. The hung setup counts as not succeeded, so `Ns` can only be under-estimated, which is safe and is tested back by the `Ns` timer.
3. The hold lifts, and the other members proceed while the hung one hangs, as that mode selects.

## Changed-range analysis

| Hunk | Change | Safety result |
|---|---|---|
| Status line (line 3) | The review record. | Nothing to check. |
| D7 (line 30) | A probe test is never a final attempt; a confirming test may be, by the exception. | Agrees with §4.7 and §4.5 rule 3, and with round-3 P2-11's closure. No defect. |
| D8 (line 31) | A member that selects a Down key after each 30 s carries a retest. | Agrees with §5.5 and OQ13(e). The handshake bound is unchanged: one per 30 s. No defect. |
| §4.5 Holds (line 214) | P2-13's correction. | Closed, as above. Exempting members past discovery has one consequence on the POST-throttle path: **P3-11**. |
| §4.5 test-pending (line 218) | The carrier precondition includes the confirming test's exception. | Removes a gap in which a confirmation with only final-attempt carriers would not enter test-pending (no fill, nothing evicted). Nothing new stalls: the hold of rule 3 and the quiet requirement are unchanged. No defect. |
| §4.5 filter (line 226), §4.8 (line 324) | Restore step starts are named in the evidence filter and in routing. | Makes round-3 P2-10's closure explicit. A restore start refused with `hi = 0` goes to the retry machine, as any start does. No defect. |
| §4.6 diagram (lines 260, 266) | Overload edges carry `min(Connected, hi(a))`. | Agrees with §4.2. It cannot raise `N` above what the server held. No defect. |
| §4.6 RESTORING (line 276) | Probe timer suspended; no test-pending; only a confirmation opened by a step. | Exits: any Overload, the first step with a refusal, or `N_good`. Each exit sets `T := T0` or SATURATED. During a confirmation, new starts are held by rule 3, so no eviction churn can arise without test-pending. If no non-final carrier is queued, no step starts and the key stays at `N`. That is the same accepted rule as for probes ("testing alone can never fail a member"), and a fresh arrival unblocks it. No new stuck state. No defect. |
| §4.6, §4.10 `Ns` yields (lines 277, 351) | The `Ns` test yields only to a *startable* `N` test. | Removes a starvation path. One test slot remains, so there are never two tests in flight. No defect. |
| §4.10 (line 350) | P3-10's correction. | Closed, as above. |
| §5.3 (line 387) | The final-attempt wording matches D7. | No defect. |
| §5.5 (line 424) | `N_good = C` when the key had no success; the first refused step ends the restore. | With `N_good = C`, an outage during the first wave restores toward `C` through judged steps. The first refusal ends the restore, so it cannot hammer. No defect. |
| §10.2 (cases 3, 22b, 44, 51–54) | Restated and added cases. | Each matches the text it cites. Case 51 covers P2-13, and case 52 covers P3-10. |
| OQ13(e), Changelog | The operator's question as asked, and the record. | Nothing to check. Reading "Park, give up after 2" at the key level, with a success resetting it, agrees with the operator's quoted option. |

**Observation, not filed.** During a hold, connections no new discovery touches age without being refreshed. A continuing member whose post-discovery gap exceeds the server's keep-alive (for example, a long local negotiation) could lease one that has died. The same F14 path exists at baseline for any member that pauses longer than the keep-alive. The increment the hold adds is small and depends on that member's pause, and OQ17 already owns the general case. No defect follows from the diff alone.

---
## 0. Evidence base

- **Object:** the revision 4 → 5 diff, every hunk, together with the ranges it depends on (§4.5 rule 3, §4.7, §5.2, §5.4 line 403, OQ3 line 587, F14).
- **Corrections plan:** RemPlan-3.
- **My round-3 report** (`-ReviewSafety-3.md`): P2-13 and P3-10, with their reproductions.
- **Code evidence:** unchanged from round 3 and not re-read (`pool/allocation.rs:10-40`, reuse before create).
- I ran nothing and modified nothing.

## 1. Findings

### [P3-11] After a POST-level 429, exempting continuing members from the hold lets them POST into the throttle the server just signalled, and a throttled POST fails its member

- **Location:**
  - §4.5 Holds, line 214: "A member already past its discovery is not held".
  - §5.4 line 403: a 429 or 503 on a POST is "Not requeued"; the member reports the throttled error once.
  - OQ3 line 587.
- **Classification:** A new root cause, NOT ARCHITECTURAL.
- **Reproduction:**
  1. A server that throttles upload-pack POSTs, which §2.4(a) leaves open for git smart-HTTP, sends a 429 with `Retry-After: 10` to member A's POST.
  2. A fails (§5.4), and the key's hold is set.
  3. Members B to F, between `info/refs` and their POST, are exempt from the hold, so they POST at once.
  4. While the server's limit persists, each POST gets a 429. None can be replayed, so each member fails.
  5. Revision 5's reason for the exemption, idle connections outliving the keep-alive, is already covered by clause (b), which discards idle connections after the hold. So holding these POSTs would no longer strand them on dead connections.
- **Impact:**
  - Up to about `N` members fail on a throttle the client already knew about, with budget remaining.
  - This is no worse than the status quo or 1.0.17, where the same POSTs also fail. It is bounded by the members partway through a fetch, and §5.4 and OQ3 already name POST throttling as the one path that still fails members.
  - Hence P3, not P2.
- **Correction:** Choose one and state it:
  - (i) When the hold was set by a POST response, also hold continuing members' next POSTs. Clause (b) makes this safe: they resume on fresh connections.
  - (ii) Keep the exemption, and name this cost in OQ3 and §5.4.
- **Closure test:** An L3-H case where a POST 429 with `Retry-After: 5` arrives while 3 members are between discovery and POST. Either no POST is sent on the key during the hold and all 3 succeed afterwards, or the stated cost is asserted.

## 2. Invariant analysis

| Invariant | Revision 5 (diff scope) |
|---|---|
| A throttle never fails a member while budget remains | Holds for discovery throttles: no lease on a connection that idled through a hold. On POST throttles, a member partway through can POST during the hold (P3-11). That path is already named by §5.4 and OQ3, and is no worse than the status quo. |
| No stuck state | Holds: RESTORING has defined exits; the `Ns` window ignores setups with no clock; the `Ns` test yields only to a startable test. |
| Bounded hammering | Holds: the restore ends on its first refused step; one test slot; Down allows one retest per 30 s. |
| `N` never above what the server held | Holds: the diagram now matches §4.2. |
| Push safety | Holds: a held receive-pack POST waits before its first byte, and no POST is ever requeued. |
| Disclosure | Unchanged by the diff. |
| Architectural status | No new architectural root cause. |

## 3. Risks and next action

- **Residual risk:** P3-11 is a bounded choice the operator can make or accept (option i or ii), with a closure test. It does not block.
- **Next action:** Record P3-11's disposition, either fixed under option (i) or accepted under (ii) with the OQ3 wording. The Surface review the status line requires (§9 note, §5.3 final error) is still outstanding before acceptance.
- **Safety-axis verdict on revision 5:** GO.
