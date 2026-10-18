# GwzRemoteTransportAlphaTimeoutPlan — SAFETY-AXIS REVIEW

**Review object:** Untracked working-tree draft `gwz-core/dev-docs/GwzRemoteTransportAlphaTimeoutPlan.md`, SHA-256 `cfdf028fb18557960da18a4682cb10f3e9e638ff197c4784efb76ac9a526984b` (supersedes `773639e1b545eeba776c3faeac1b0785aef50ee33faa0ba1e984b5ec214cac4a`). Reviewed 2026-09-22 (re-verdict round 1).
**Baseline:** `gwz-dev` `7b3f1bc723d6c846bd7d5a4f009602afd90a1593`; `gwz-core` `7bd9381f0593f759333c7d8d070f4962d4092df7`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Plan hash verified at start and end. Diagnosis `977d7884…8ad5f6` unchanged. Design §10–§10.2 working tree matches HEAD (empty diff). Remediation plan `GwzRemoteTransportAlphaTimeoutPlan-RemPlan.md` used as disposition map only; its claims were re-traced on the revised plan text, not treated as proof. Cited code inspected read-only where needed to confirm original counterexample premises. No builds, tests, network, or tree mutations.
**Date:** 2026-09-22
**Axis:** Safety: what the plan's rules permit to go wrong. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — 0×P0, 0×P1, 0×P2 open; prior P0-1, P1-1, P2-1, P2-2, P2-3, P3-1 all CLOSED on re-trace. No new blocking finding.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P0-1 | Accept: S3.3 before S5.2; refuse close on `connect_ms=10000`-only live pass | Original sequence (Phase 2 + scripted Control only → S5.2 pass) is forbidden: Outcome L33–35; S5.2 L255–260; dep sketch requires S3.3 | CLOSED |
| P1-1 | Accept: `begin_wait`/`complete_wait`; idle slices not completions; S3.3 on production path | §2 L51–58 and S3.1 L171–178 define the pair and forbid quantum/sleep as completion; S3.2/S3.3 name hung TCP and agent EAGAIN regressions | CLOSED |
| P2-1 | Accept: HTTPS widen intentional, bounded by pool aggregate; stuck-connect fixture | Phase 2 L137–141 and S2.2 L158–163 lock `connect_ms=10000` and require expiry at that bound, not later | CLOSED |
| P2-2 | Accept: distinct `stall`/`aggregate` reasons; S5.2 records, does not infer | Outcome L29–31; §2 L74–76; S3.1/S4.2/S5.2 carry and assert distinct reasons | CLOSED |
| P2-3 | Accept: interaction spends neither clock; stall pauses with pool begin/end | §2 L64–67; S3.1 L180–182; S3.2 interaction case L191–194 | CLOSED |
| P3-1 | Accept: late reject is setup timeout with reason, not `PermissionDenied`; gate sees aggregate/stall/cancel | S4.1 L221–229; cancel with absent aggregate still disposes | CLOSED |

---

## Changed-range analysis

Relative to the prior draft (`773639e1…`), this replacement adds: Outcome ban on cumulative-only close; §2 wait-completion rule and interaction pause rule; distinct timeout reasons; S1.1 quoted design edits; S2.1 stall argument into `Job`; S2.2 HTTPS bound + stuck-connect fixture; S3.1 `begin_wait`/`complete_wait` + deterministic clock + interaction forward; S3.2 idle/readiness/interaction cases; new S3.3 production-graph wiring; S4.1 install-gate reasons (not `PermissionDenied`); S4.2 dual-reason fixtures; S5.1/S5.2 S3.3 gate and unwired-stall refusal; §5 HTTPS-ceiling and cumulative-evidence carve-outs; dependency edges S3.3→S5.2 and S4.1←S2.2+S3.1.

All of that falls inside the remplan dispositions. No change was found that invents a new safety architecture outside those dispositions. Residual note (not a finding): `stall`/`aggregate` as a local failure detail without a new wire field is the plan’s stated channel; it is adequate for alpha ordinary output and S5.2 retention under the plan’s own §5 wording, and is not re-opened here.

---

## 0. Evidence base

| Item | How read | Result |
|---|---|---|
| Plan hash | `shasum -a 256` start + end | `cfdf028f…26984b` unchanged |
| Diagnosis hash | same | `977d7884…8ad5f6` unchanged |
| Repo HEADs | `git rev-parse` start + end | tuple unchanged |
| Design vs HEAD | empty `git diff` | matches |
| RemPlan | read as disposition map | Safety rows Accept for P0-1…P3-1 |
| Original couplers / gates | prior review evidence; spot-check `Failure` has `code`/`effect`/`facts` only | premises of closed findings unchanged in HEAD code |

---

## 1. Findings

None open.

Prior counterexamples re-traced:

- **P0-1:** S5.2 cannot close on default cold-fetch success after uncoupling alone; S3.3 production stall regression is a hard prerequisite; Outcome equates cumulative-only widen with the rejected workaround.
- **P1-1:** Wait lifecycle is named; idle slices are non-completions; S3.3 applies the rule on `ssh_network` / `agent_auth` / `agent_socket` with hung-TCP and EAGAIN tests.
- **P2-1:** HTTPS connect widen is explicit, capped at pool `connect_timeout_ms`, and must expire at that aggregate in a fixture.
- **P2-2:** Stall vs aggregate are distinct retained/caller-visible reasons; S5.2 records the reason rather than inferring it.
- **P2-3:** Interaction spends neither clock; stall remainder is preserved across pool begin/end forwarded to `Control`.
- **P3-1:** Late success after aggregate, stall, or cancel is a setup timeout with reason; not `PermissionDenied`; zero-timeout cancel still drops.

No new P0–P2 found on this revision.

---

## 2. Invariant analysis

| Claim / rule | Under revised plan rules | Result |
|---|---|---|
| Progressing setup can exceed 3 s stall and still succeed | S3.3 production completions + S3.2 readiness case | Held in text |
| No-progress wait expires at 3 s (`stall`) | Idle slice ≠ `complete_wait`; hung TCP / EAGAIN → `stall` | Held in text |
| Cumulative-only 3→10 s is not a fix | Outcome + S5.2 unwired refusal | Held |
| `--ssh-timeout 0`: dispose/cancel bounded; no install after cancel | S4.1 | Held |
| Late success not pooled | S4.1 install gate | Held |
| HTTPS connect budget | Intentional 10 s aggregate ceiling; not later | Held (accepted collateral) |
| Interaction excluded from network accounting | Stall pauses with aggregate | Held |
| Stall vs aggregate diagnosable | Distinct reasons through S4.2/S5.2 | Held |
| §5 out-of-scope (no ConnectClock progress reset; no §10.1 on setup) | Unchanged | Held |

---

## 3. Risks and next action

Residual implementation risk is execution fidelity (actually wiring S3.3 and refusing S5.2 without it), not a remaining plan-text permission to go wrong under the closed counterexamples.

**Next action:** Proceed to implementation under this plan; keep S3.3 and the S5.2 stall-path gate non-skippable. No further Safety-axis plan remediation required for GO.
