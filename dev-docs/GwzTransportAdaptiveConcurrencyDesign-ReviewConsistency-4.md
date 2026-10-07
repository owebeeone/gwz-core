# GWZ transport adaptive concurrency design, revision 5: consistency-axis focused re-check

**Review object:** `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md`, working tree, 724 lines, sha256 `e65043e667817d4c18842d89aa9442fd97357dd9472e2ed3ff0a99d85b94ff7e`. Status: revision 5 DRAFT, under focused re-check, uncommitted.
**Baseline:**
- Root `189bbd9229d755691edb91634ed6a222c4cb9484`, gwz-core `db0f8447ccf653c066f69fd8f964843714d9b35f`, gwz-transport `ff6083b5230e06dccdddb251e5a76175535c6cf3`.
- I checked the object's hash and all three HEADs at the start and at the end. Nothing moved.
- Revision 4 was diffed from the scratchpad copy `adaptive-rev4.md`, sha256 `a0188888…`, which I verified.
- `-RemPlan-3.md` was read as the disposition claim, and each claim was re-traced against the text.
- Pool code was read with `git grep` at `ff6083b5`.

**Date:** 2026-10-07
**Axis:** Consistency. The scope is the revision-4 to revision-5 diff only, checked against the rest of the document and the controlling documents: the retry plan, amendment 2, the HTTPS design, and the reuse design with CS7.23. This check is independent, adversarial and read-only. Other axes run in parallel, and nothing here relies on them. The lane owner files this report verbatim.

**Verdict: GO.** There are 0 P0, 0 P1, 0 P2 and 1 P3 (P3-26, new, not architectural).
- My round-3 GO holds.
- P3-19 to P3-25 are all closed on revision 5.
- The new hold scope in §4.5 contradicts no accepted clause and needs no §14 entry.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on revision 5 | Status |
|---|---|---|---|
| P3-19 | The diagram's Overload edges use `min(Connected, hi(a))`. | Lines 260 and 266 now read `N := max(1, min(Connected, hi(a)))`. A search for the bare `max(1, Connected)` finds nothing. The edges agree with §4.2 line 151, §4.5 rule 4 and D3. | **Closed** |
| P3-20 | D7 and §5.3 name the confirming-test exception; test-pending's precondition includes it. | D7 (line 30), §5.3 (line 387), test-pending (line 218: "for a confirming test, any member, §4.7's exception"), §4.7 (line 304) and §4.5 rule 3 (line 236) all agree. Case 45 is derivable. | **Closed** |
| P3-21 | D8: a member that selects the key after each 30 s carries the retest. | D8 (line 31) now agrees with §5.5 ("none while none do"), §4.7 ("No connection is ever opened only to test") and retry plan §4 line 199. | **Closed** |
| P3-22 | RESTORING sub-state with §5.5's exits; restore starts named in the filter and in routing. | §4.6 line 276: "on a completed step with any refusal and no Overload". This is §5.5's wording, and §5.5 now says "the first step with a refusal ends the restore". I re-traced my counterexample (a step's conclusive Suspect refusal, its confirmation refuted after the step completes). Both sections now end the restore in STABLE with `T0`. The filter heading (line 226) and the §4.8 row (line 324) name restore step starts. Case 54 is derivable. | **Closed** |
| P3-23 | Case 3 runs on L1 and L3-R; L3-S checks classification only. | Line 493. The scripted `Ns := 10` assertions now run on L1, which scripts results, so "co-setups that succeeded" can be produced. | **Closed** |
| P3-24 | Case 22b names the Throttle class; case 44 follows case 31's path. | Line 512: "the refusal a Throttle", so `N := min(20, 21) = 20` follows. Line 534: the wave goes the Suspect path, then the probes at `hi = 0`. Both are derivable. | **Closed** |
| P3-25 | OQ13(e) records the question and the chosen option, and states the reading. | Line 597: the question and the option are quoted, and the key-level reading is stated against the member-level alternative. It also says how "one retest per 30 s still running" is read. That matches D8 and §5.5. | **Closed** |

## Changed-range analysis

- **D7, D8, §5.3 and test-pending (P3-20, P3-21).** These now read the same way as §4.5 rule 3, §4.7 and §5.5 (see the table).
- **§4.6: the edges, RESTORING, and "startable".**
  - In RESTORING the probe timer is suspended and test-pending is not entered. The only test is a confirmation that a step's refusal opens. This agrees with §4.4's restore row ("Not a test: no test slot, no quiet requirement"), with §4.5 rule 3, and with §5.5's exits.
  - "Startable" is defined inline (the test's base holds and the key can be made quiet). It is used the same way in §4.10 (line 351) and case 53.
- **§4.10, a setup with no clock in the `Ns` window.** It counts as not succeeded and is not waited for. That agrees with §4.5 step 2's rule for setups with no clock (line 221) and with case 52. Under-counting is safe here, because the setup timer tests `Ns` back.
- **§5.5, `N_good = C` with no success, and one refused step ends the restore.** This replaces revision 4's looser "at most two". It agrees with §4.6 and case 27c.
- **Cases 3, 22b, 44 and 51–54.** All are derivable from the rules as written.
- **§4.5 Holds, the new scope (Safety P2-13), against the controlling documents:**
  - **Retry plan.** It has no clause on holds. §14 items 2 and 17 cover the requeue of a discovery 429 on new and reused connections. Leaving members already past their discovery unheld is a narrowing, so no further clause is contradicted.
  - **HTTPS design §6 and §7, as §14 items 14 and 15 replace them.** Item 15's replacement says 429 on discovery is "held … and requeued; on a POST, reported once with the hold set". That still holds: a POST's 429 sets the hold, which now stops new connections and discoveries only.
  - **Discarding idle connections early.** §6 ("retain its 60-second idle default") fixes the pool's idle timeout. It does not forbid discarding a connection sooner. §6's "otherwise discard" already allows discards.
  - **Reuse design §7 (stale idle replacement before send).** That is 1.2.0 and is compatible.
  - **Inside the document.** The new scope agrees with §5.4 (a POST's 429 is not requeued and sets the hold), §4.1 (HTTPS Connected comes at the first answered exchange), §4.7 ("the test is not due before the hold ends"), and cases 1, 7, 36 and 51.
  - **Conclusion.** No §14 entry is needed. One under-specification remains: P3-26.
- **Changelog.** The revision-5 entry (lines 719–724) covers every hunk of the diff.

---

## 0. Evidence base

- **Diff.** The full revision-4 to revision-5 diff, every hunk.
- **Unchanged context read again.** Lines 214–226 (§4.5), 255–280 (§4.6), 300–331 (§4.7–§4.8), 338–351 (§4.9–§4.10), 387 and 424 (§5.3, §5.5), and §14 items 2, 14, 15 and 17.
- **Controlling documents.** HTTPS design lines 245–262 (§6) and 322; retry plan lines 188–202.
- **Code at `ff6083b5`.**
  - The public pool API in `pool/asynchronous.rs`, `pool/lifecycle.rs` and `pool/machine.rs`.
  - The only closes a host can trigger are `release` with a `Disposition`, `retire_https_scope`, `cancel_operation` and `cancel_session`, plus `start_closing`, which is `pub(super)`.
  - The pool has no verb that closes the idle entries of one key.

## 1. Findings

### [P3-26] The hold-end discard in §4.5 is not reconciled with §4.9's pool interface or with test-pending's "closes nothing" (new, not architectural)

- **Location.**
  - §4.5 Holds, line 214: "When a hold longer than 1 s ends, the key's idle connections are discarded rather than leased (closed, then Settling)."
  - Against §4.9, line 338: "gwz-transport pool, numbers only", with only `set_limit` and `set_settle`.
  - Against §4.5 step 2, line 221 ("the client closes or evicts nothing on it, until the key is quiet") and its bound at line 224, which names `IdleExpired` as "a client close the hold cannot suppress".
- **Violated invariants.**
  - §4.9 lists the pool's changes, and the list is said to be exhaustive ("numbers only").
  - The quiet-wait rule names every close it cannot suppress.
- **Reproduction.**
  1. **The discard has no interface.** At `ff6083b5`, nothing in the public pool API closes the idle entries of one key (Evidence base). Two ways to discard are possible. A new pool verb would contradict "numbers only". Leasing each idle connection and releasing it `Discarded` from the endpoint would work, but that has to be stated, along with whether it counts as an attempt (§5.3).
  2. **The ordering at the hold's end is unstated.** The hold ends and the probe becomes due at the same instant, because §4.7 says the test is not due before the hold ends.
     - If test-pending step 2 wins, the discard is suppressed. Members then lease connections that the keep-alive may have closed, which is the failure Safety's P2-13 removed.
     - If the discard wins, the key cannot be quiet for one `Ts`. The probe's base, `Connected = N`, may fail, so the test stays due while admission refills. That outcome is fine, but line 224's bound does not mention it.
- **Impact.** A plan could not implement this sentence without inventing one of the two mechanisms and an order between them. Case 51 asserts the outcome but not the mechanism. This is bounded, and it does not reopen P2-13's defect if the discard wins.
- **Correction.**
  - In §4.9, name how the discard is done: a pool verb (and drop "numbers only"), or a lease-then-`Discarded` release by the endpoint that is not an attempt.
  - In §4.5, state that the hold-end discard is applied before test-pending is evaluated, and add it next to `IdleExpired` in line 224's bound ("adds at most one `Ts`").
- **Closure test.** Case 51 adds two assertions: the discard happens through the named mechanism, and a probe due at the hold's end starts after the discarded connections have settled. Optionally, add an L1 step that has the hold end and `T` expire at the same instant.

## 2. Invariant analysis

- **Judging attempts against their targets (D4).** Unchanged from round 3 and now consistent everywhere, diagram included (P3-19).
- **Carrier rule.** One statement, with one exception, everywhere: D7, §4.5, §4.7 and §5.3 (P3-20).
- **Restore.** One set of exits, in §4.6 and §5.5. The sub-state is named, and the probe timer and test-pending are suspended in it (P3-22).
- **Down.** D8, §1, §5.5, §14 items 3, 4, 6, 9 and 13, and OQ13(e) agree (P3-21, P3-25).
- **Holds.** The new scope is consistent with §5.4, §4.7, §14 items 14, 15 and 17, and the HTTPS design. The only open point is the mechanism and ordering of the hold-end discard (P3-26).
- **Controlling graph.** No clause newly contradicted. §14 needs no new item.

## 3. Risks and next action

- **Risk.** Low on this axis. P3-26 is a plan-level ambiguity in a path that only HTTPS `Retry-After` holds longer than 1 s reach.
- **Next action.** Apply P3-26 as specified, in the acceptance commit or as a correction cleared without another round and recorded in the verdict. Then run the Surface review that the status line requires before acceptance. My GO does not depend on P3-26.
