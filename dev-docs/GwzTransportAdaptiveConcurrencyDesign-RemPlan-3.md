# GWZ transport adaptive concurrency design: corrections for revision 4 (round 3)

Date: 2026-10-07. Object: `gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md` revision 4, sha256 `a0188888082ef78e5f83b30894183150a25fa82f2455e1471fa489a79e5d4333`.

Round-3 verdicts, filed verbatim:

| Axis | Report | Verdict | Findings |
|---|---|---|---|
| Consistency | `-ReviewConsistency-3.md` | GO | 7 P3 |
| State | `-ReviewState-3.md` | GO | 5 P3 |
| Safety | `-ReviewSafety-3.md` | NO-GO | P2-13 and P3-10; pre-commitment to GO on P2-13 |

No finding is architectural. The two-round cap's own rule allows this: "A third round confined to non-architectural corrections is permitted". Revision 5 is one narrow patch. Prefixes: C (Consistency), S (Safety), T (State).

## Dispositions

| Finding | Disposition in revision 5 | Closure test |
|---|---|---|
| **S P2-13** | §4.5 Holds now has two parts. (a) A hold stops new connections and an open's **first exchange**, its discovery, only; a member already past its discovery continues its own exchanges. (b) When a hold longer than 1 s ends, the key's idle connections are discarded rather than leased. | Case 51. |
| **S P3-10 = T P3-8** | §4.10: a setup with no clock is not waited for in the `Ns` window. It counts as not succeeded. | Case 52. |
| **C P3-19 = T P3-9** | §4.6: the diagram's Overload edges read `N := max(1, min(Connected, hi(a)))`. | Text: no bare `max(1, Connected)` remains. |
| **C P3-20, T P3-10** | D7 and §5.3: a probe test is never a member's final attempt; a confirming test may be, by §4.7's exception. §4.5's test-pending precondition now includes that exception. | Case 45, with the spy asserting the test-pending sub-state. |
| **C P3-21** | D8: after each 30 s, a retest is carried by a member that selects the key. | Text. |
| **C P3-22, T P3-11** | §4.6: the restore is the **RESTORING** sub-state. The probe timer is suspended, test-pending is not entered, and the only test that can run is a confirmation a step opened. Its exits are §5.5's. §4.5 and §4.8 name restore step starts in the evidence filter and in routing. §5.5: the first step with a refusal ends the restore. | Case 54; the restore case of C P3-22. |
| **C P3-23** | Case 3 runs on L1 and L3-R. L3-S checks only the drop's classification. | Case 3. |
| **C P3-24** | Case 22b names the Throttle class. Case 44 follows case 31's path. | Cases 22b, 44. |
| **C P3-25** | OQ13(e) quotes the question as asked and the option the operator chose, and states how the design reads them. | Text. |
| **T P3-7** | §4.6 and §4.10: the `Ns` test yields only to an `N` test that is *startable*. | Case 53. |
| **T risks** | §5.5: `N_good = C` when the key had no success. | Case 27c. |

Re-check: Safety re-checks P2-13 and P3-10 on revision 5. Consistency and State re-check that the diff from revision 4 keeps their GO and closes their P3s.
