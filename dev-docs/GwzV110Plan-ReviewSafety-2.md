# GwzV110Plan — Safety-AXIS REVIEW (round 3 / remediation 2)

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62` (prior settled review object `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c`). Draft-stage re-verdict after `GwzV110Plan-RemPlan-1.md`. Not an implementation acceptance.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Start and end hashes/HEADs match.
**Date:** 2026-09-22
**Axis:** SAFETY — re-trace P1-4 and P2-6 only; do not reopen closed round-1 IDs unless regressed; second remediation — a new architectural root stops the lane. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — 0×P0, 0×P1, 0×P2, 0×P3 open. P1-4 and P2-6 closed as pre-committed. No round-1 Safety regression found. No new architectural root cause on this patch.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P1-4 | Unsupported mark on a §2 in-release cell fails S5.6 and blocks S7.5 / Phase 8 unless an accepted amendment first removes that cell from §1 and §2; Linux ARM64, macOS x86-64, and exact-agent-until-proven may still be marked without that amendment | S5.6 (274–278): fail + block S7.5/Phase 8 unless amendment removes the cell from §1 and the §2 “in this release” column; named exceptions match. Round-2 counterexample (mark gh-auth unsupported, still tag under §1/§2) does not survive. | **Closed** |
| P2-6 | S5.6 precedes S7.1 in the normative sketch and in §4 prose | Sketch (424): `S5.6 ── S7.1`; S5.6 text (278): “S7.1 waits on this step”; §4 prose (433–434): S7.1 waits on S3.3, S4.5, S5.5, **S5.6**, and S6; Phase 7 header still lists S5.6 (309–310). Round-2 counterexample (S7.1 without S5.6) does not survive. | **Closed** |

Round-1 Safety IDs (P0-1, P1-1, P1-2, P1-3, P2-1–P2-5, P3-1): spot-checked for regression on this patch (vendor SHA gate, S3.3 stall regression, S7.5, S5.6 advertising/real-`gh` rules, sketch S3.3 edge, `NAME=TAG`, Linux registry smoke, mid-train stop, exact-agent, redaction). **No regression.**

---

## Changed-range analysis

`GwzV110Plan-RemPlan-1.md` is remediation 2 on `6ec8f7e7…`. Safety-facing deltas only:

- **S5.6** gains the §2 in-release unsupported → fail/block/amendment rule, named exceptions, and “S7.1 waits on this step” (274–278).
- **§4 sketch** moves `S5.6` from `── S7.2` to `── S7.1` (424).
- **§4 prose** adds S5.6 to the S7.1 wait list (433–434).

Consistency-only deltas on this same file (S1.1 Phase 2 re-entry; Phase 8 step-3 cite) were not Safety round-2 findings; inspected only for a new Safety architectural root — none found (re-entry before Phase 8 step 3 keeps the binding publisher gated).

---

## 0. Evidence base

| Source | Role |
|---|---|
| `GwzV110Plan.md` @ `9d49af85…8f62` | Settled object |
| `GwzV110Plan-RemPlan-1.md` | Claimed dispositions for P1-4 / P2-6 |
| Own round-2 Safety report | Counterexamples re-traced |
| Controlling docs (Design §11–§12, Transport Plan Phase 6, L1-32) | Invariants behind P1-4 / P2-6 |

---

## 1. Findings

None open.

---

## 2. Invariant analysis

| Invariant | After rem-1 (`6ec8f7e7…`) | After rem-2 (`9d49af85…`) |
|---|---|---|
| §2 scope shrink only via accepted amendment | Broken (P1-4) | Holds (S5.6 fail/block) |
| Activation (S7.1) after matrix sign-off (S5.6) / L1-32 | Broken (P2-6) | Holds (sketch + prose) |
| Round-1 Safety closures | Held | Still held (no regression) |

Original P1-4 sequence (mark §2-promised cell unsupported → ledger omits → Phase 8 under unchanged §1/§2) now fails at S5.6. Original P2-6 sequence (S7.1 before S5.6 via sketch) is blocked by normative waits.

---

## 3. Risks and next action

This axis accepts the plan text at `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62` on SAFETY. Pre-commit condition met. Lane-owner merge with the other axis remains outside this report. Writing or accepting this plan still does not implement, tag, or publish.

**End-of-review tuple:** plan SHA-256 `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62`; repo HEADs unchanged from Baseline.
