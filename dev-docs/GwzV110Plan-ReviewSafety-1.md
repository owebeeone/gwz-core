# GwzV110Plan — Safety-AXIS REVIEW (round 2)

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c` (round-1 object was `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31`). Draft-stage re-verdict after `GwzV110Plan-RemPlan.md`. Not an implementation acceptance.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Plan + rem-plan read as files; repo HEADs via `git rev-parse`. Start and end hashes/HEADs match. Prior abort on a non-settled hash does not count as a round.
**Date:** 2026-09-22
**Axis:** SAFETY — re-trace round-1 counterexamples on remediated text; new architectural roots labeled. Independent, adversarial, read-only. No round-2 Consistency report consulted. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0×P0, 1×P1, 1×P2, 0×P3 open among new findings; all ten round-1 Safety IDs closed on this tree. I pre-commit to GO on a revision that resolves **P1-4** and **P2-6** as specified.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P0-1 | Phase 8 step 1 refuses `--push` unless vendored libgit2 `git rev-parse` equals `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` | Step 1 (362–365) states the full SHA and fail-closed refuse; `git2-rs` vendors via `libgit2-sys/libgit2` submodule (live HEAD equals that SHA). Original “tag without content gate” counterexample does not survive. | **Closed** |
| P1-1 | S3.3 imports timeout-plan S5.2 production-graph stall regression as hard precondition | S3.3 (172–182): idle stage → `stall` while aggregate ahead; unwired stall fails even if cold fetches pass; then default cold fetches; `--ssh-timeout 15` / `connect_ms=10000`-only do not close; S7/Phase 8 wait. | **Closed** |
| P1-2 | S7.5 dual activation/release review; Phase 8 refuses without that GO | S7.5 (331–335); Phase 8 depends on S7.5 (354–355); sketch `S7.5 ── Phase 8` (419). | **Closed** |
| P1-3 | S5.6 §11 cell map + real-account `gh` before advertising; fixtures ≠ proof; S7.2 bound to S5.6 | S5.6 (263–269); S7.2 (314–316); Phase 6 exit table (337–350); Phase 8 (404–406). Advertising-without-evidence counterexample closed. Residual scope-vs-unsupported hole → **P1-4 (new)**. | **Closed** (advertising gate); see P1-4 |
| P2-1 | Normative sketch; S3.3 precedes S7.1 | Sketch (413); prose (181–182, 300–301, 355–356, 424–425). | **Closed** |
| P2-2 | `--dependency-tag NAME=vX.Y.Z` where override exists; Cargo edge stays registry version | S2.2 (129–132); Phase 8 step 4 (380–384). | **Closed** |
| P2-3 | Registry install smoke on Linux x86-64 as well as macOS ARM64 and dabeest | Post-job paragraph (400–403); CI alone is not the Linux proof. | **Closed** |
| P2-4 | After failed push/publish: stop; no later product tag; no complete claim; new patch/RC; never move tag | Phase 8 (357–360). | **Closed** |
| P2-5 | S7.2 keeps Windows exact-agent false until named fixture GO | S7.2 (316–318). | **Closed** |
| P3-1 | Phases 4, 5, 8 redact agent sockets, known_hosts bodies, `gh` tokens/headers; secret fails the step | §2 (66–68); S6.3 tied to same rule. | **Closed** |

---

## Changed-range analysis

Remediation is one plan-text patch (no product code), accepting every round-1 finding. Safety-relevant deltas vs `a52cd7a8…`:

- Authorities / timeout: S5.2 stall regression named; rebuilt alpha does not close S5.1 (19–26).
- Disclosure rule for phases 4/5/8 (66–68).
- Phase 2: single `git2-rs` tag; registry pins; `NAME=TAG`; bootstrap token then trusted publisher (119–147).
- S3.1/S3.3 full timeout-plan closure predicates; S3.3 on path to S7/Phase 8 (155–182).
- S4.5 expands unix|windows candidate gates before Phase 7 removes the switch (215–225).
- S5.3 → S5.4; S5.5 → S7; **S5.6** §11 sign-off (246–269).
- S6.2 `RELEASE.md` registry pins (282–291).
- Phase 7: depends list; S7.4 observations; **S7.5** dual review; Phase 6 exit traceability table (298–350).
- Phase 8: mid-train stop; vendor SHA gate; one `git2-rs` release; optional binding step; `NAME=TAG`; three-host registry smoke (352–406).
- Normative sketch reconnects S3.3 and S5.5 to S7.1; S5.6 → S7.2 only (408–425).

New attack surface from those deltas: (1) S5.6 “unsupported” marks vs §2 “in this release” without an amendment/stop coupling; (2) Phase 7 header claims S5.6 dependency while normative S7.1 waits omit S5.6, allowing activation before matrix sign-off.

---

## 0. Evidence base

| Source | Role |
|---|---|
| `GwzV110Plan.md` @ `6ec8f7e7…1c4c` | Settled object |
| `GwzV110Plan-RemPlan.md` | Claimed dispositions (not proof) |
| Round-1 Safety report (own) | Counterexamples to re-trace |
| Controlling docs as in round 1 | Design §11–§12; Transport Plan Phase 6; readiness; timeout S5.2; L1-32 |
| `git2-rs/libgit2-sys/libgit2` submodule HEAD | Context for P0-1 vendor-gate realism only |

---

## 1. Findings

### [P1-4] [new architectural root cause] S5.6 may mark §2 in-scope cells unsupported without an amendment or Phase 8 stop

**Location:** §2 in-release column (51–57: “SSH and gh-only HTTPS…”); S5.6 (263–269: any cell may take an “explicit unsupported mark”); Phase 6 exit row “Unsupported capabilities recorded; scope reduction is an amendment | §2 and S5.6” (349) asserts the invariant without implementing the coupling; S7.2 only forbids *advertising* unsupported cells (314–316); Phase 8 still runnable after S7.5.

**Violated invariant:** Transport Plan Phase 6 — any reduction of agreed scope requires an **accepted amendment**, not a silent mark. §2 is this plan’s in-release scope. Marking a §2-promised capability unsupported while still tagging v1.1.0 under §1’s “ships … gh-only HTTPS” outcome is false composition of the release claim.

**Reproduction:**
1. Real-account `gh` evidence is unavailable or fails on one or more supported platforms.
2. S5.6 marks gh-authenticated HTTPS (and/or other §2-in-scope §11 cells) unsupported.
3. S7.2 passes (those cells are not advertised). S7.5 may GO on a narrowed ledger.
4. Phase 8 tags `v1.1.0`. §1/§2 still describe shipping SSH and gh-only HTTPS as supported 1.1.0 behavior; no amendment was filed.

**Impact:** Published product identity claims a supported authenticated-HTTPS/SSH scope the evidence table withdrew. Users and operators treat 1.1.0 as delivering §2 while capabilities/ledger omit it — worse than an honest narrower amendment.

**Remedy:** Bind S5.6 to §2: any unsupported mark on a cell covered by the §2 “in this release” column **fails S5.6 / blocks S7.5 / Phase 8** unless a filed, accepted scope amendment removes that cell from §2 (and from §1 outcome) first. Deferred-only rows (Linux ARM64, macOS x86-64, exact-agent until proven, etc.) stay markable without that bar.

**Closure test:** Checklist: for each §2 in-scope capability, S5.6 shows evidence IDs on all three platforms **or** an amendment ID + updated §2; a dry run that marks gh-auth unsupported with §2 unchanged cannot exit S7.5.

---

### [P2-6] [new architectural root cause] Normative DAG lets S7.1 activate before S5.6, contradicting the Phase 7 header and L1-32

**Location:** Phase 7 header “Depends on … S5.6 …” (300–301); normative sketch `S5.6 ── S7.2` only (415); prose “S7.1 waits on S3.3, S4.5, S5.5, and S6. S7.2 also waits on S5.6” (424–425). `AgentProcessRules.md` L1-32: keep disabled until the compatibility/platform matrix for the state is accepted; enable in a reviewed activation change.

**Violated invariant:** Activation construction (S7.1) must not precede acceptance of the matrix that defines which routes may be on (S5.6), when the phase header claims S5.6 is a dependency and the sketch is normative.

**Reproduction:**
1. Complete S3.3, S4.5, S5.5, Phase 6; skip or defer S5.6.
2. Per sketch/prose, run S7.1: remove candidate switch; ordinary macOS/Linux/Windows builds construct the transport.
3. Only later finish S5.6 (possibly marking large holes unsupported) and S7.2–S7.5.

**Impact:** Local/ordinary builds (and any consumer of that activated tree before S7.5) exercise production routes before §11 sign-off. Not yet an immutable crates.io tag (S7.5 still gates Phase 8), but it widens blast radius of an incomplete matrix and contradicts the plan’s own Phase 7 dependency claim — a stuck or half-activated workspace state under the text’s rules.

**Remedy:** Make the sketch and S7.1 waits match the header: `S5.6` precedes `S7.1` (or state explicitly that S5.6 blocks the whole Phase 7, including S7.1). Remove the “S7.2 also waits on S5.6” wording that implies S7.1 does not.

**Closure test:** Single normative wait list: S7.1 prerequisites include S5.6; a release checklist refuses S7.1 without an S5.6 evidence-table ID.

---

## 2. Invariant analysis

| Invariant | Round-1 | Round-2 |
|---|---|---|
| Vendor content identity before immutable tag | Broken (P0-1) | Holds (step 1 SHA gate) |
| Timeout S5.2 full closure before tag | Broken (P1-1) | Holds (S3.3) |
| S3.3 on path to activation | Broken (P2-1) | Holds |
| Activation/release dual review before publish | Broken (P1-2) | Holds (S7.5) |
| No advertise without §11 / real-GH evidence | Broken (P1-3) | Holds for ledger; **§2 vs unsupported mark still broken (P1-4)** |
| Activation after matrix acceptance (L1-32) | N/A (no S5.6) | **Broken for S7.1 ordering (P2-6)** |
| gearu `NAME=TAG` + registry pins | Broken (P2-2) | Holds |
| Linux registry install smoke | Broken (P2-3) | Holds |
| Mid-train fail-closed | Broken (P2-4) | Holds |
| Windows exact-agent false until proven | Broken (P2-5) | Holds |
| Evidence secret redaction | Broken (P3-1) | Holds |
| Q6 Batch A ≠ acceptance before S3.2 | Held | Holds |
| No second Python pool / divergent clocks | Held | Holds |

---

## 3. Risks and next action

Round-1 Safety blockers are closed on `6ec8f7e7…1c4c`. Remaining NO-GO is from remediation-introduced architecture: **P1-4** (unsupported marks can silently shrink §2 without amendment) and **P2-6** (S7.1 before S5.6). Fix those two couplings, then this axis pre-commits to GO without re-litigating closed IDs unless the patch regresses them.

**End-of-review tuple:** plan SHA-256 `6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c`; repo HEADs unchanged from Baseline.
