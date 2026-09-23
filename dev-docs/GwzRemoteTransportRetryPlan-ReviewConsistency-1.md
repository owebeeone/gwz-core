## Prior-finding closure table
| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Consistency P2-1 | §3.4–§3.6 name v1.1.0 §5, AlphaTimeout 3,000/10,000 sentences, Design §10.1 3,000 ms; S2.3 applies | §3.4–§3.6 present; S2.3 applies §3.1 and §3.4–§3.6; residual non-verbatim Alpha line filed as new P3-1 | **CLOSED** |
| Consistency P2-2 | §3.7–§3.8 name Design §7.2, transport “cannot raise”, “Keep the pool's validation bounds”; S1.4 cites them | Named quotes present (whitespace-normalized); S1.4 cites §3.7–§3.8; validation-bounds replacement matches S1.4 | **CLOSED** |
| Consistency P2-3 | `--max-retries` long help states the wait schedule; `--ssh-timeout` only sets stall | §8 max-retries matches §5 waits (1/2/4, 30 s cap, ≤0.25 s jitter) and says `--ssh-timeout` is stall-only | **CLOSED** |
| Consistency P2-4 | S2.2 pins `--ssh-timeout`; S3.4 pins `--max-retries` + fetch/push/pull; §11 names those steps | S1.1 / S2.2 / S3.4 / §11 all pin as claimed | **CLOSED** |
| Consistency P3-1 | §3.3 quotes the full pool-capacity OOS bullet | Full bullet `` `--ssh-timeout` default of 3 s in 1.0.17 vs a 2.0-3.5 s handshake. `` present | **CLOSED** |

## Changed-range analysis

Relative to `a1421c53…`: §3 grew items 4–8; §4/§5 gained **Closed**; §8 help rewritten (stall scope, waits owned by `--max-retries`); S1.4 cites §3.7–§3.8; S2.2/S3.4 gained help pins; S2.3 broadened; §11 points at those steps. Rem map file is `GwzRemoteTransportRetryPlan-RemPlan-1.md` (plan header still names the old map — P3-2). New pressure surfaced only where §6’s idle reconfigure still collides with an unsuperseded Transport Plan sentence.

# GwzRemoteTransportRetryPlan — CONSISTENCY-AXIS REVIEW

**Review object:** plan SHA-256 b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3, 2026-09-23
**Baseline:** repo SHAs
**Date:** 2026-09-23
**Axis:** Consistency re-verdict. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: NO-GO** — P0: 0, P1: 0, P2: 1, P3: 2. Pre-commit to GO on a revision that resolves **P2-1** (P3-1 and P3-2 in the same edit).

---

## 0. Evidence base

**Tuple (start and end, identical):**

| Item | Value |
|---|---|
| gwz-dev HEAD | `9c0008870ecd8f209611ebd99fb1598a14021da9` |
| gwz-core HEAD | `102015a5ff3abc059be699f801d1412b0fd01c8a` |
| gwz-cli HEAD | `ab59011db0ee00ab0c032fc23fc06b2578bf7b68` |
| gwz-transport HEAD | `aa40936d0805e8cb60f8027615abe20d4f2045e4` |
| Plan SHA-256 | `b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3` |

Inspection only. Working-tree `GwzV110Plan.md` unchanged (`f0c57611…`). Remediation map: `GwzRemoteTransportRetryPlan-RemPlan-1.md`. Prior Consistency findings re-traced on this hash; other-axis current reports not read. Code outside the standing exceptions not re-litigated.

**§3 quote audit (new bullets):** §3.3, §3.4, §3.5 table cells, §3.5 §5 OOS, §3.6, §3.7 Design/Transport fragments, and §3.8 match the controlling text under whitespace normalization within the cited sentence. **Exception:** §3.5’s “sentence” quote inserts words not present on AlphaTimeoutPlan line 72 (see P3-1). Compact whole-file search can false-match that insertion across a line break; character-level line compare fails.

---

## 1. Findings

### [P2-1] (new architectural root cause) Transport Plan still forbids resizing the pool on a lower limit; §6 does exactly that

- **Root cause:** `GwzRemoteTransportPlan.md` Phase 2 still states, in force per plan §2 except §3: “A lower operation limit neither resizes the endpoint pool nor evicts another operation's connections.” §3.7 supersedes the adjacent “cannot raise” / starting-eight / “default eight” fragments but **not** this sentence. Plan §6 / S1.4, when no lease is non-idle, close idle connections above the new caps and rewrite the four size fields — that is a resize on a later, possibly lower, operation.
- **Location:** Plan §3.7 (gap), §6 bullet “If the pool has no non-idle lease…”, S1.4; `GwzRemoteTransportPlan.md` Phase 2 capacity bullet (exact sentence above).
- **Violated invariant:** Exclusive supersession (§3) must name every controlling sentence the body overrides.
- **Reproduction:** (1) Confirm the Transport Plan sentence is absent from §3. (2) Read §6 idle reconfigure. (3) Note §2 keeps Transport Plan in force except §3.
- **Impact:** Same false-composition failure mode as the prior capacity supersession gap: an implementer obeying Transport Plan refuses sequential resize; obeying this plan performs it. Prior P2-2’s named closures do not cover this sentence.
- **Required correction:** Add the Transport Plan sentence to §3 (extend §3.7 or add a bullet) with a replacement that matches §6: at idle operation start the resolved caps are installed and idle excess may be closed; non-idle foreign leases still refuse; non-idle connections of another operation are not evicted.
- **Closure test:** Grep of §3 quotes that sentence verbatim; replacement text is what S1.4 implements; no remaining in-force “neither resizes” rule.

### [P3-1] §3.5 misquotes AlphaTimeoutPlan’s aggregate sentence

- **Root cause:** Plan §3.5 quotes `A positive `--ssh-timeout` does not change the 10-second aggregate. There is no new flag.` Controlling line 72 is `` `--ssh-timeout` does not change the 10-second aggregate. There is no new flag. `` — no leading “A positive”.
- **Location:** Plan §3.5; `GwzRemoteTransportAlphaTimeoutPlan.md` line 72.
- **Violated invariant:** Exactness of superseded-clause lists.
- **Reproduction:** Diff the §3.5 quoted sentence against AlphaTimeoutPlan line 72.
- **Impact:** Verbatim locate-and-replace on acceptance (S2.3) fails; auditors can disagree on which string is superseded.
- **Required correction:** Quote line 72 exactly; keep the replacement prose as already written.
- **Closure test:** §3.5 quote equals AlphaTimeoutPlan line 72 character-for-character (aside from allowed wrapping).

### [P3-2] Status blurb still points at the round-0 rem map and denies a Consistency report

- **Root cause:** Header still says the map is `GwzRemoteTransportRetryPlan-RemPlan.md` and “Consistency produced no report,” after Consistency NO-GO and `RemPlan-1.md`.
- **Location:** Plan lines 7–11.
- **Violated invariant:** Process/evidence pointers in the object under review must name the active remediation map.
- **Reproduction:** Open plan header; compare to `GwzRemoteTransportRetryPlan-RemPlan-1.md` existence and Consistency review on disk.
- **Impact:** Later auditors follow the wrong closure list.
- **Required correction:** Point at `RemPlan-1.md`; drop or update the “Consistency produced no report” sentence.
- **Closure test:** Header names the rem map whose SHA closes this hash.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| Prior P2-1 timeout supersessions | Pass (§3.4–§3.6 + S2.3); residual quote exactness → P3-1 |
| Prior P2-2 capacity / validation supersessions | Pass for the sentences rem named; new gap on “neither resizes” → P2-1 |
| Prior P2-3 §8 max-retries vs §5 | Pass |
| Prior P2-4 help pins S1.1 / S2.2 / S3.4 / §11 | Pass |
| Prior P3-1 full pool OOS quote | Pass |
| §3 quotes vs controlling docs | Pass except §3.5 Alpha sentence (P3-1) |
| AlphaTimeoutPlan Phase 2 milestone still saying 3,000/10,000 | Not a defect: historical phase exit criteria; normative defaults are the §2 table / OOS lines §3.5 replaces |
| D2 / no channels | Pass |

---

## 3. Risks and next action

- **Blocking risk:** Only P2-1 — authority graph still invents a conflict on sequential pool resize.
- **Next action:** One plan-only edit: supersede the Transport Plan “neither resizes…” sentence; fix the §3.5 Alpha quote; refresh the status/rem-map blurb. Re-hash; Consistency re-check.
- **Pre-commit:** GO on a revision that resolves **P2-1** as specified; include **P3-1** and **P3-2** in that same edit.
