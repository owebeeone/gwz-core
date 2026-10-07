# GWZ transport adaptive concurrency design, revision 4: safety-axis re-review (round 3)

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, the working-tree file. Revision 4, DRAFT ("under re-review"), 714 lines, sha256 `a0188888082ef78e5f83b30894183150a25fa82f2455e1471fa489a79e5d4333`. It is uncommitted. Re-reviewed 2026-10-07.
**Baseline:** root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`.
- Revision 3 → 4 was diffed from the saved revision 3 (sha256 `ee5b9258…`); the diff is 216 lines.
- The remediation plan read was `GwzTransportAdaptiveConcurrencyDesign-RemPlan-2.md`.
- The tuple was checked at the start and at the end and did not change.

**Date:** 2026-10-07
**Axis:** Safety: what the text allows to go wrong. This covers stuck states, hammering, the claim of no regression against the status quo, push replay, disclosure, and the race model. The review is independent, adversarial and read-only, and nothing here relies on the other axes. The lane owner files it verbatim.

**Verdict: NO-GO.**
- All 6 round-2 findings are closed on revision 4.
- The changed ranges contain new findings: 0 P0, 0 P1, 1 P2 (P2-13) and 1 P3 (P3-10). **Neither is ARCHITECTURAL**: both are bounded text-scope fixes. So the lane's architectural cap is not reached.

I pre-commit to GO on a revision that resolves P2-13 as specified, with P3-10 either fixed or recorded as accepted by the operator.

---
## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 4 | Status |
|---|---|---|---|
| P2-9 | Down parks arrivals for the next retest, and they share its result. After 2 failed retests in a row, arrivals fail at once. This is the operator's choice. | §5.5 lines 412–417, §5.2, D8, §11, §14 items 3/4/6/9/13. I re-traced the 10 s outage with 200 members, `C = 32`, `--jobs 100`: the queued members fail at exhaustion, and the next 32 pulled (`par_map_per_host`) park with allocation clocks stopped. The retest at about +30 s is carried by the first parked member, which is guaranteed to exist, so no state is stuck without a carrier. The retest succeeds and the parked members proceed. Under `--jobs 1` against a dead host: 4 handshakes, then 2 parked retests (about 60 s, each failing its one parked member), then a fast drain. Handshakes stay at one per 30 s. The text states the policy and bounds it correctly. | Closed |
| P2-10 | The restore step start gets a judgement row: target `S`, conclusive iff `hi < S`. Defined exits. | §4.4 line 186, §4.6, §4.9, §5.5 line 424. Re-traced `N = Connected = 4`, `N_good = 32`, limit 4: `S = 8`, 4 starts, and the first refusal has `hi = 7 < 8`, so it is conclusive. A Throttle gives an Overload with `N := min(4, 7) = 4`. A Suspect gives a confirmation at 5, refused fairly, so `N = 4`. Either way the restore ends in STABLE with `T0`. A step whose refusals are all inconclusive also ends in STABLE with `T0`. No repetition without a wait. | Closed |
| P2-11 | The confirmation never closes for want of a carrier; a final-attempt member carries the test when no other can. | §4.5 line 236, §4.7. Re-traced `R = 1` with a reset limit of 8: the confirmation stays open and holds new setups (leases proceed). Once the key is quiet, one final-attempt member carries the test at 9, which is refused fairly: Overload, `N = 8`, and the other 23 succeed. The refused width is never re-offered. | Closed |
| P2-12 | At `R = 0` a drop before authentication never lowers `Ns`. | §5.3 line 394, §4.10 line 350, case 46. | Closed |
| P3-8 | SATURATED's residual is stated: one decrease, repaired at `T0`. | D5, R1, case 22b. | Closed (residual accepted in text) |
| P3-9 | A setup with no clock does not keep the key from being quiet. | §4.5 step 2 (line 221), case 47. This holds for the quiet condition. The same exemption is missing from the new `Ns` hold: **P3-10**. | Closed |

## Changed-range analysis

1. **Down parking and its `--jobs 1` cost.**
   - A parked wait is at most 30 s plus one retest's attempt bound, with the allocation clock stopped.
   - Handshakes per key are at most one per 30 s.
   - The two-failure count resets on a success.
   - The `--jobs 1` cost (about 60 s more) is stated in OQ12.
   - Parked members hold `--jobs` threads for up to about 60 s. That is the existing OQ2 cost, bounded.
   - **No defect.**
2. **The restore steps and their exits.** Re-traced above. Each step starts only with non-final carriers. If none is queued, the key stays at `N`, which is the stated rule that "testing alone can never fail a member". A completed step either doubles or ends. **No defect.** The "at most two steps" claim in line 424 conflicts with "ends on a completed step with any refusal" (which allows only one), but nothing unsafe follows. Left to the Consistency axis.
3. **The confirmation's final-attempt carrier exception.** It costs at most one final-attempt member per confirmation. The test is still create-only, on a quiet key, at `Connected + 1`. **No defect.**
4. **Holds on leased exchanges.** This widens a hazard: **P2-13**.
5. **The HTTPS `Connected` definition** (the first exchange answered with a status other than 429 or 503), together with `N := max(1, min(Connected, hi(a)))`.
   - A throttled discovery's own connection is never counted.
   - A leased exchange's own connection is excluded by `hi`.
   - I found no interleaving in which `N` exceeds what the server held. `hi(a)` can at worst under-shoot `Connected` for a connection admitted after `a`'s result, and the next success restores it by §4.2.
   - **No defect.**
6. **The `Ns` estimator's hold.** With clocks enabled it is bounded by the setups' aggregate. `Ns` is taken from the co-setups that *succeeded*, so it is independent of drop order, and start-order mismatches can only under-estimate it, which is safe and is tested back. Under `--ssh-timeout 0` the hold has no exemption: **P3-10**.
7. **The create-only test request.** It closes the race where a test was served by reuse (case 48). **No defect.**
8. **Cases 43–50.** I checked each against the rules it cites, and each matches the text. Two gaps: no case covers P2-13's resumption after a hold (keep-alive shorter than the hold), and none covers P3-10.
9. **§14 items 3, 4, 6, 9, 12, 13 and 17–19.** They agree with §5.5's Down policy and with §5.4. Item 17 keeps POST replay excluded. Push safety is unchanged: a POST is never requeued, and `Effect::Possible` is preserved.

---
## 0. Evidence base

- **Object:** revision 4 read in the changed ranges and their dependencies (§§1, 4.1–4.10, 5.2–5.5, 10.2, 11, 12, 13, 14), the revision 3 → 4 diff, and RemPlan-2.
- **Controlling documents:** as in rounds 1 and 2.
  - Retry plan §§4–6 (§6 for `par_map_per_host`'s pull).
  - Amendment 2 §3.20.
  - HTTPS design §6, where each RPC is a sequential lease: "the endpoint may lease the same physical connection sequentially".
- **Design facts relied on:**
  - F14 (line 575): an idle connection the server closed is found only at its next lease, and "that lease's failure is `Phase::Other`, so Return: the member fails".
  - §2.5: no host calls `idle_closed`.
- **Code used as evidence** (unchanged): `pool/allocation.rs:10-40` (reuse before create), `setup_retry/machine.rs`.
- I ran nothing and modified nothing.

## 1. Findings

### [P2-13] A hold idles the key's pooled connections for up to 30 s, then resumes by leasing them; a connection the server closed meanwhile fails its member (F14). Revision 4 extends this to members partway through a fetch or push

- **Location:**
  - §4.5 "Holds", line 214, new in revision 4: "nothing starts on the key, **and no new exchange is begun on a leased connection of the key**, until `now + min(Retry-After, 30 s)`. Exchanges already in progress are not interrupted."
  - §12 F14, line 575.
  - §2.5: no host reports idle loss.
  - OQ17: "this design works without it, at the stated cost". The stated cost is R7b's one refused attempt, not a member failure.
  - HTTPS design §6: each RPC (discovery GET, each upload-pack or receive-pack POST) is its own sequential lease.
- **Violated invariant:**
  - §1: "None of them fails a member while budget remains".
  - The operator's direction that a throttle slows the command and does not fail members.
- **Reproduction:**
  1. A self-hosted Git server behind a proxy with a 5 s keep-alive (Apache's default). It rate-limits with 429 and `Retry-After: 20`, as GitLab does. `C = 32`, members on one HTTPS key, connections reused.
  2. Member A's discovery gets a 429 with `Retry-After: 20`, so the key's hold runs to `t + 20 s`.
  3. Members B to F have finished discovery and are between RPCs: an upload-pack negotiation POST, or a receive-pack POST after `info/refs`.
  4. Under line 214, each next POST is a "new exchange … on a leased connection" and is held. No exchange is "in progress" between two RPCs.
  5. During the 20 s hold, the server closes every idle keep-alive connection after 5 s. No host notices (§2.5).
  6. At `t + 20 s`, B to F, and the requeued discovery members, lease those idle connections, because the pool reuses before it creates (`allocation.rs:16-38`).
  7. Each exchange fails on a dead socket. By F14 that is `Phase::Other`, so Return, and the member fails.
  8. Budget remained for every one of these members, and none was refused by the server.
  9. For B to F (receive-pack), the failure comes before the first request byte, so `Effect::None`. There is no push hazard, but the member fails.
- **Impact:**
  - One server `Retry-After` turns into a burst of member failures whenever the hold is longer than the server's keep-alive.
  - The same mechanism applies, at a smaller scale, to revision 3's start-only hold: discovery opens resuming onto dead idle connections. Revision 4's extension adds the members partway through a fetch or push.
  - The status quo has no hold, so the connections are not left idle.
- **Required correction:** Both of the following:
  - (a) Scope the hold to an open's first exchange (its discovery on a new or leased connection). A member already past discovery continues its own RPCs, because the server has already admitted that operation.
  - (b) After any hold longer than a short bound (for example 1 s), do not lease the key's idle connections without replacement: discard them, or make a lease that fails on a dead socket before any request byte Requeue a fresh connection instead of Return.

  (b) brings the needed part of OQ17 into this design as a precondition of holds. Alternatively, state the cost explicitly and have the operator accept it.
- **Closure test:** A new L3-H case. Keep-alive is 5 s. A 429 with `Retry-After: 10` arrives while 5 members are between discovery and POST and 10 connections are idle. No member fails, no exchange is sent on the key during the hold except the exempted continuing RPCs, and no lease after the hold lands on a dead socket.
- **Classification:** A new root cause, NOT ARCHITECTURAL. It is a scope fix in §4.5 plus a precondition for reuse after a hold.

### [P3-10] The new `Ns` hold waits for a setup with no clock, freezing new setups on the key under `--ssh-timeout 0`

- **Location:**
  - §4.10 line 350: "record `S_lo_s(a)` and hold new setups on the key … when every setup in `S_lo_s(a)` has a result, `Ns := …`".
  - Compare §4.5 step 2 (line 221), which exempts a setup with no clock from the quiet condition. The `Ns` hold has no such exemption.
- **Reproduction:**
  1. `--ssh-timeout 0`.
  2. A wave in which one setup hangs (TCP accepted, no banner) and another is dropped before authentication with `hi_s ≥ 1`.
  3. The hung setup is in `S_lo_s` of the drop. It never has a result, so the hold never lifts.
  4. No new setup starts on the key for the rest of the command, and members that need a new connection wait forever with their allocation clocks stopped. This is the spread of a hang that round-2 P3-9 removed for the quiet condition, reintroduced here.
- **Impact:** The command already never completes in this mode. More members are left without results, and §5.3's claim that "no wait is unbounded" is false again.
- **Correction:** Exclude setups with no clock from `S_lo_s(a)`'s completion condition. Take `Ns` from the clocked co-setups only, and count the unclocked ones in `hi_s` as §4.5 does.
- **Closure test:** Case 47 extended to a drop before authentication: the `Ns` hold lifts when the clocked co-setups resolve.
- **Classification:** A new root cause in a changed range, NOT ARCHITECTURAL.

## 2. Invariant analysis

| Invariant | Revision 4 |
|---|---|
| A throttle never fails a member while budget remains | Holds for the limit, the confirmation, the restore and Down. Breaks when a hold is resumed onto dead keep-alive connections (P2-13). |
| A limit is tested; no permanent untested decrease | Holds for `N` and `Ns`, including at `R = 0`. |
| A transient failure does not become permanent | Holds: Down parks and the retest serves the members after a 10 s outage. |
| Client-versus-server count lag | Holds outside SATURATED. In SATURATED, one stated off-by-one decrease is repaired at `T0`. |
| `N` never above what the server holds; never above `C` | Holds: `min(Connected, hi)`, SATURATED at `C`, the pool limit raised only for an admitted test or a restore step. |
| Bounded hammering | Holds: the timer backs off, the restore ends on its first refused step, and Down allows one handshake per 30 s. |
| Push safety | Holds: a POST is never requeued, and `Effect::Possible` is preserved. P2-13's failures come before the first request byte. |
| Disclosure | Holds: configured host only. |
| Stuck states | None with clocks enabled. Under `--ssh-timeout 0`: P3-10. |
| "Never worse than the status quo" | Holds, except P2-13 (the status quo has no hold, so no stretch of idle time). |

## 3. Risks and next action

- **Highest risk:** P2-13. It is the one remaining path by which a server's throttle signal fails members that still have budget. Revision 4's widening of the hold makes it reachable for members partway through a fetch or push, against any server whose keep-alive is shorter than its `Retry-After`. The fix is a scope restriction plus a reuse precondition, both text-level.
- **Architectural status:** Neither new finding is architectural. Under the two-round cap, this round's findings are bounded completions, not a third architectural root cause.
- **Next action:** a narrow patch to §4.5 "Holds" (scope and reuse after a hold) and to §4.10's `Ns` hold (the exemption for setups with no clock), with the two closure cases added to §10.2. Then a focused Safety check of those two ranges only.
