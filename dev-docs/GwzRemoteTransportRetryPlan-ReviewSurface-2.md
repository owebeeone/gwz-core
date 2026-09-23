## Prior-finding closure table
| ID | Disposition claimed | Verified on corrected text | Status |
|----|---------------------|----------------------------|--------|
| P2-1 | FIXED (prior) — short names per-attempt + retries; long bounds defaults | Short still: per-attempt + `stalled setup is retried`; long still ≈44s / ≈128s / host-key ≤120s | **CLOSED** (no regression) |
| P3-1 | FIXED (prior) — scope, transports, `0`, disable, answered | Setup vs body; SSH+HTTPS; `--ssh-timeout 0` keeps retries; `--max-retries 0`; later success = answered; waits owned under `--max-retries` | **CLOSED** (no regression) |
| P3-2 | FIXED (prior) — ceiling vs “not a maximum” | Jobs / max-per-host unchanged from closed text | **CLOSED** (no regression) |
| P3-3 | FIXED (prior) — operations / hostname / host-key | Unchanged jobs/max-per-host; host-key bound still in `--ssh-timeout` long | **CLOSED** (no regression) |
| P3-4 | FIXED (prior) — `0` for jobs/max-per-host | Unchanged | **CLOSED** (no regression) |
| P2-2 | FIXED — body stall + short/long scope | Short: setup **and** body read; long: clock on setup and fetch/push/pull body; body abort, not retried | **CLOSED** |
| P3-5 | FIXED — pull in body non-retry | `fetch, push, or pull body` in both `--ssh-timeout` and `--max-retries` long | **CLOSED** |
| P3-6 | FIXED — HTTPS stall clock | Same stall clock + 30s budget on SSH and HTTPS; `0` clears both on both; HTTPS setup defined | **CLOSED** |

## Changed-range analysis

Relative to Surface-1 (hash `a1421c53…`) open items only; jobs/max-per-host unchanged:

| Flag | Was (open defect) | Now |
|------|-------------------|-----|
| `--ssh-timeout` short | “Per-attempt stall limit” without body; long narrowed to setup | Stall for **setup and body read**; setup retried |
| `--ssh-timeout` long | Setup-only opener; body “not retried” only; HTTPS unspoken | Clock on setup **and** fetch/push/**pull** body; body aborts repo; SSH+HTTPS same clock/budget; waits explicitly **not** this flag |
| `--max-retries` long | Pull omitted from body clause; HTTPS setup vague vs timeout | Pull in body non-retry; SSH vs HTTPS setup boundary named; waits restated; `--ssh-timeout` = stall only |

Live released wording (already recorded): abort stalled SSH/network read. Proposed short+long now cover setup stall and body read stall — supersedes without silent narrowing.

# GwzRemoteTransportRetryPlan — SURFACE-AXIS REVIEW

**Review object:** proposed help, plan hash b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3, 2026-09-23
**Baseline:** the proposed help above; live help only for the released wording already recorded
**Date:** 2026-09-23
**Axis:** Surface re-verdict. Help text only. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — 0 open P2, 0 open P3; 5 prior + 3 reopen IDs closed

---

## 0. Evidence base

| Item | Result |
|------|--------|
| Plan `shasum -a 256` (start) | `b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3` — match |
| Plan `shasum -a 256` (end) | same — match |
| Proposed paste | `--ssh-timeout` / `--max-retries` as given; jobs/max-per-host = prior closed text |
| Source / Safety / Consistency / plan body | not read |
| Live help | not re-run; released `--ssh-timeout` short/long from Surface-1 used only for P2-2 close |

## 1. Findings

No open findings.

**P2-2 closed:** Short and long agree the stall clock covers setup and body reads; body stall aborts that repository and is not retried; setup stall fails one attempt and is retried. Matches the released “stalled … read” contract without dropping body.

**P3-5 closed:** Pull named beside fetch and push for body stall / non-retry.

**P3-6 closed:** SSH and HTTPS share the stall clock and 30s setup budget; `0` clears both on both transports; HTTPS setup bounded “before the first request byte.”

Adversarial re-check that did not reopen: ≈44 vs ≈128 vs `0 = no timeout` remain distinct cases; waits attributed to retry path, not the stall flag; `--max-retries` remains the only retry control; jobs/max-per-host text not re-broken.

## 2. Invariant analysis

| Invariant | Result |
|-----------|--------|
| `-h` vs `--help` for `--ssh-timeout` scope | **Pass** — short and long both setup + body |
| Body stall: timeout vs retry | **Pass** — abort, not retried |
| Pull parity with fetch/push | **Pass** |
| HTTPS under `--ssh-timeout` | **Pass** |
| `0` / waits / retries | **Pass** — stall+budget off; waits remain; retries via `--max-retries` |
| Prior P2-1, P3-1…P3-4 | **Pass** — no regression on this paste |
| Released-flag meaning stability | **Pass** — body read remains in the contract |

## 3. Risks and next action

Surface help text is fit to proceed on this axis. Residual risk is implementation matching the paste (out of Surface scope). No Surface remediation required; lane owner may file and advance.
