# GwzV110Plan — Consistency-AXIS REVIEW (Round 3 / rem-2)

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62` (prior settled hash `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c`). Status: plan text after remediation 2 (`GwzV110Plan-RemPlan-1.md`). Verified at start and end; hash and repo HEADs unchanged.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Object + RemPlan-1 read; P1-5/P2-6 counterexamples re-traced on the new bytes. Other axis’s round-2 report not read.
**Date:** 2026-09-23
**Axis:** CONSISTENCY — second remediation re-verdict. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — 0 P0, 0 P1, 0 P2, 0 P3 open on this axis. Round-2 pre-commit conditions (P1-5, P2-6) met. No new Consistency architectural root. Closed round-1 IDs not regressed by this patch.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P1-5 | If S1.1 chooses a separate binding package, Phase 2 re-enters: fifth name in S2.1, `gearu.toml` + publish workflow in S2.2/S2.3, `gearu plan` before Phase 8 step 3 | S1.1 (92–95): re-entry for that package alone; S2.1 fifth name/owner; S2.2/S2.3 add `gearu.toml` and workflow before step 3; `gearu plan` on that repo is part of Phase 2 exit for that branch. Original sequence (finish Phase 2 as three-repo-only, then step 3) is forbidden on the separate branch | **CLOSED** |
| P2-6 | S1.1 names Phase 8 step 3 as separate-binding publisher; step 4 remains `gwz-git` | S1.1 (91): “Phase 8 step 3”; Phase 8 (384–393): step 3 = separate binding, step 4 = `gwz-git`; step 7 (405–406): “step 2 or step 3”. No remaining “step 4” cite for the binding | **CLOSED** |

Round-1 IDs (P1-1…P1-4, P2-1…P2-5, P3-1…P3-2) and rem-1 closures were spot-checked for regression only; monorepo single-tag publish, registry pins, bootstrap token, Windows gate expansion, timeout S5.1/S5.2 discharge, S5.3→S5.4, S5.5/S3.3→S7, S7.4/S7.5 + Phase 6 exit table remain intact. Not reopened.

---

## Changed-range analysis

Relative to `6ec8f7e7…`, this patch (RemPlan-1) changes:

- **S1.1 (91–95):** separate package = Phase 8 **step 3** (was wrongly step 4); Phase 2 **re-entry** for fifth name + gearu + publish workflow + `gearu plan` before step 3.
- **S5.6 / sketch / §4 (Safety rem, Consistency-visible):** unsupported mark on a §2 in-release cell fails S5.6 and blocks S7.5/Phase 8 unless amended; S5.6 → S7.1 in sketch and prose.

P1-5 and P2-6 counterexamples were re-run against S1.1 + Phase 8 step list + Phase 2 milestone. Both fail to reproduce. Safety’s rem items do not introduce a Consistency contradiction (sketch and Phase 7 header already listed S5.6; prose now matches).

---

## 0. Evidence base

Settled object `9d49af85…`; RemPlan-1 `67c4e2db…`; prior Consistency reports for ID text only. End tuple identical to start.

---

## 1. Findings

None.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| P1-5 (Phase 2 provisions separate binding) | Closed — re-entry stated in S1.1 |
| P2-6 (S1.1 ↔ Phase 8 step index) | Closed — step 3 only |
| Round-1 closed set | Not regressed |
| New Consistency architectural root | None found |

---

## 3. Risks and next action

No Consistency blockers remain on this plan text. Lane owner may treat this axis as GO for acceptance of the plan document (implementation and later gates remain as written in the phases).

End-of-review tuple: object `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62`; repo HEADs unchanged from baseline.
