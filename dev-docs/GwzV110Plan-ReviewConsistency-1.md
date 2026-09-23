# GwzV110Plan — Consistency-AXIS REVIEW (Round 2)

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c` (round-1 object was `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31`). Status: plan text after one remediation (`GwzV110Plan-RemPlan.md`). Verified at start and end of review; hash and repo HEADs unchanged.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Object + RemPlan read in full; controlling contracts re-checked only where a round-1 counterexample required it. RemPlan SHA-256 `7d878bb70e388da44475599651987287c4e8d04a124e973144f3863ec06684bc` (inspection only).
**Date:** 2026-09-23
**Axis:** CONSISTENCY — re-verdict after remediation; re-trace original counterexamples on the settled bytes. Independent, adversarial, read-only. The other axis’s round-2 report was not read. Filed verbatim by the lane owner.

**Verdict: NO-GO** — round-1 blocking findings closed on this text; **1 new P1 and 1 new P2** open. I pre-commit to GO on a revision that resolves **P1-5** and **P2-6** as specified.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P1-1 | One `gearu release` of `git2-rs`; workflow publishes sys then API from one tag; no same-repo `--dependency-tag` | Phase 8 step 1 (362–369) and S2.2/S2.3 (123–147) state single tag, ordered publish, no inter-package `--dependency-tag` | **CLOSED** |
| P1-2 | Phase 8 step for separate binding when S1.1 chooses it; in-`gwz-transport` uses that repo’s release | Step 3 (375–378) exists; step 7 waits on step 2 or 3 (396–398). Original “no publisher in the train” counterexample fails | **CLOSED** (see new P2-6 / P1-5 for residual packaging gaps) |
| P1-3 | Registry `version` pins only; gearu verifies tag, does not write `git` pin; dry-run must accept | S2.2 (126–132), Phase 8 steps 3–4 and product pins (375–398) require `version = "=…"` and no `git` key; dry-run gate stated | **CLOSED** |
| P1-4 | First publish of new names via operator-held token; trusted publisher after name exists | S2.3 (138–147) and Phase 8 step 1 (368–369) sequence bootstrap → TP → tokenless later | **CLOSED** |
| P2-1 | Phase 4 expands parent unix construction gates to unix\|windows under candidate; Phase 7 removes candidate only | S4.5 (215–225) names `transport_host`, `git::endpoint`, binding construction; S7.1 (303–310) removes candidate switch only | **CLOSED** |
| P2-2 | S3.1 reviews timeout S3.1/S3.3/S4.1; amend only “before rebuilding”; S3.3 imports production-graph stall | Authorities (21–26); S3.1 (155–163); S3.3 (172–182) match rem closure predicates | **CLOSED** |
| P2-3 | S5.3 on every path into S5.4 | S5.4 (250–252); sketch (414); §4 prose (423–424) | **CLOSED** |
| P2-4 | Sketch normative; S3.3 and S5.5 precede S7.1; prose/header agree | Phase 7 header (300–301); sketch (413–419); §4 (424–425); Phase 8 (354–355) | **CLOSED** |
| P2-5 | S7.4 observation recheck; S7.5 dual activation/release review before Phase 8; Phase 6 exit table | S7.4–S7.5 (326–335); Phase 8 depends on S7.5 (354); exit table (337–350) | **CLOSED** |
| P3-1 | On acceptance, amend CurrentProgramCheckpoint | Status (9–11) | **CLOSED** |
| P3-2 | S6.2 amends RELEASE.md; binding registry pin; D7 git-tag-only core not 1.1.0 form | S6.2 (287–291); Phase 8 step 7 (396–398) | **CLOSED** |

---

## Changed-range analysis

Relative to round-1 object `a52cd7a8…`, the settled text adds: rem-plan pointer and explicit timeout-plan timing amendment; evidence redaction rule; S1.1 publisher naming; S2.2/S2.3 monorepo+registry+bootstrap publish model; S3.1/S3.3 timeout discharge fidelity; S4.5 parent-gate expansion; S5.3→S5.4 and S5.5→S7; S5.6 §11 sign-off; S6.2 RELEASE.md/crates.io pin rewrite; S7.4/S7.5 and Phase 6 exit traceability table; Phase 8 single `git2-rs` release, optional binding step, stop/recovery rule, three-host registry smoke, vendored-hash gate.

Round-1 counterexamples were re-run against those ranges. Closures above hold. Two **new** defects remain in the remediated wiring of the optional separate-binding branch (P1-5, P2-6).

---

## 0. Evidence base

Same authorities as round 1, plus `GwzV110Plan-RemPlan.md` (accepted dispositions). Round-1 report `GwzV110Plan-ReviewConsistency.md` used only to re-trace IDs. End tuple: object `6ec8f7e7…`; repo HEADs identical to baseline.

---

## 1. Findings

### [P1-5] **NEW** — Phase 2 never provisions gearu (or a fifth crate identity) for S1.1’s separate binding package

**Location.** S1.1 (89–91) allows a separate binding package published in Phase 8. Phase 8 step 3 (375–378) requires `gearu release …` for that package. S2.1 (113–117) records only “the **four** crate names.” S2.2 (119–122) installs `gearu.toml` only in `git2-rs`, `gwz-transport`, and `gwz-git`. Phase 2 milestone (102): “`gearu plan` succeeds for **each** new package.”

**Violated invariant.** A deferred Phase 1 choice may not leave a mandatory Phase 8 `gearu release` without Phase 2 identity + `gearu.toml` (gearu `docs/ReleaseProcess.md` preconditions: committed valid `gearu.toml`). Same shape class as round-1 P1-2, but a **new root**: rem closed the missing *publish step*; it did not close missing *release-tooling provisioning*.

**Reproduction / state sequence.** (1) S1.1 GO chooses a separate package/repo. (2) Phase 2 completes as written (four names; three `gearu.toml` files). (3) Phase 8 step 3 runs `gearu release` in the binding repo → fails (no `gearu.toml` / never planned). Step 7’s wait on “step 2 or step 3” never clears on that branch.

**Impact.** Separate-binding design branch cannot reach v1.1.0; false claim that “either boundary has a publisher” (89–91) without a complete train.

**Remedy.** After S1.1, if separate: extend S2.1 to that fifth name/owner; extend S2.2 (and S2.3 workflow) to that repo; state Phase 2 re-entry when the boundary is “separate.” Or forbid the separate-package option in S1.1.

**Closure test.** For the separate-boundary option only, show S2.1 name list, `gearu plan` on that repo, and Phase 8 step 3 succeeding in the documented order before `gwz-py`.

---

### [P2-6] **NEW** — S1.1 points the separate binding at Phase 8 step 4; step 4 is `gwz-git`

**Location.** S1.1 (89–91): “a separate package is Phase 8 **step 4**.” Phase 8 body: step 3 = separate binding (375–378); step 4 = `gwz-git` (380–384). Step 7 correctly says binding publisher is “step 2 or step 3” (396–398).

**Violated invariant.** Internal cross-references that name the publisher must agree with the Phase 8 step list (rem P1-2 closure test: “Each S1.1 option **names** the Phase 8 step that publishes the binding”).

**Reproduction / state sequence.** Operator follows S1.1 to “step 4” for the binding → executes the `gwz-git` release as if it published the binding, or skips step 3. Step 7’s predicate (step 2 or 3) and S1.1’s label disagree.

**Impact.** False composition of the release checklist on the separate-binding branch; diagnosability failure even when step 3 exists.

**Remedy.** Change S1.1 to “Phase 8 step 3” (and keep step 7’s “2 or 3”).

**Closure test.** Grep the plan: every “separate binding” publisher cite is step 3; step 4 is only `gwz-git`.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| Round-1 P1-1…P1-4, P2-1…P2-5, P3-1…P3-2 | Closed on settled text (table above) |
| Phase 8 satisfiable for **both** S1.1 boundaries | **Fail** — P1-5 (separate branch); in-`gwz-transport` branch OK |
| S1.1 ↔ Phase 8 step numbering | **Fail** — P2-6 |
| Phase 5/7 discharge Phase 6 exit evidence | **Pass** on this text — table (337–350) + S5.6/S7.4/S7.5 |
| S3.1/S3.3 vs timeout S5.1/S5.2 | **Pass** — explicit timing amendment + production-graph stall |
| gearu / crates.io / bootstrap | **Pass** for the four always-on crates; separate binding still lacks Phase 2 (P1-5) |
| Windows cfg before Phase 7 | **Pass** — S4.5 expands; S7.1 demotes candidate |
| §4 sketch vs Phase 7 header | **Pass** — sketch declared normative; S5.3/S5.5/S3.3 aligned |

No P0. No reopen of closed round-1 IDs as the same root cause.

---

## 3. Risks and next action

**Risk if tagged as GO now:** choosing the separate Python-binding package leaves Phase 8 step 3 without gearu provisioning (hard stop), and S1.1’s wrong step index can skip or mis-attribute the binding release.

**Next action:** Fix P2-6 (S1.1 → step 3) and P1-5 (Phase 2 covers the optional binding repo/name/workflow, or drop that S1.1 option). Re-dispatch Consistency on the new hash; this axis pre-commits to GO when only those two are resolved as specified.

End-of-review tuple: object `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c`; repo HEADs unchanged from baseline.
