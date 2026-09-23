## Prior-finding closure table
| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Safety-1 P2-1 | Key state **Closed** after budget exhaustion or non-retriable setup; later members get recorded failure, no setup; S3.1 covers `--jobs 1` late joiners incl. post-Healthy | Retraced §4/§5 Closed and S3.1 against original counterexamples (`--jobs 1` dead key; auth; Healthy then exhaust) | **CLOSED** |
| Safety-1 P3-1 | `--max-retries` long help states the wait schedule; `--ssh-timeout` only sets stall | Retraced §8; waits named on `--max-retries`; stall-only / “does not clear these waits” | **CLOSED** |
| Safety P2-1 (r1) | Workers ≤ `--jobs`; fallible spawn | §6/S1.5 unchanged by Closed | **CLOSED** (not regressed) |
| Safety P2-2 (r1) | `max_requests = max(1024, jobs)`; upper size bounds removed | §6/S1.3/S1.4 unchanged | **CLOSED** (not regressed) |
| Safety P2-3 (r1) | Install caps at start; refuse if non-idle | §6/S1.4 unchanged; Closed does not restore first-writer freeze | **CLOSED** (not regressed) |
| Safety P2-4 (r1) | Cold/Degraded one in-flight setup; stale success ignored | §5 Cold/Degraded/generation intact; Closed forbids return to Cold mid-operation | **CLOSED** (not regressed) |
| Safety P2-5 (r1) | Stall/aggregate only; timeout origin | §4/S3.2 unchanged | **CLOSED** (not regressed) |
| Safety P3-1 (r1) | 127.75 s / 627.75 s; `--ssh-timeout 0` unbound | §5 bounds unchanged | **CLOSED** (not regressed) |

## Changed-range analysis
From `a1421c53…` to `b40b75c0…` (`RemPlan-1`): adds per-key **Closed** (§4/§5) with late-joiner fan-out and next-operation Cold reset; extends S3.1 for `--jobs 1` / auth / post-Healthy exhaustion; rewrites `--max-retries` (and related `--ssh-timeout`) help so waits are not attributed to `--ssh-timeout`. **No new architectural root-cause defect:** Closed is the missing terminal state that Safety-1 P2-1 required, not a fresh unsafe architecture.

# GwzRemoteTransportRetryPlan — SAFETY-AXIS REVIEW

**Review object:** plan SHA-256 `b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3`, 2026-09-23
**Baseline:** gwz-dev `9c0008870ecd8f209611ebd99fb1598a14021da9`; gwz-core `102015a5ff3abc059be699f801d1412b0fd01c8a`; gwz-cli `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; gwz-transport `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Tuple and plan hash verified identical at start and end. Sources as in Safety-1 (committed via `git show` / member HEAD; authorized working-tree exceptions only). Did not read Consistency or Surface current-round reports.
**Date:** 2026-09-23
**Axis:** Safety re-verdict. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — P0: 0, P1: 0, P2: 0, P3: 0.

---

## 0. Evidence base

Read: revised plan `b40b75c0…`; `GwzRemoteTransportRetryPlan-RemPlan-1.md`; prior Safety-1 open IDs and round-1 closed IDs for regression. Focused on §4 Closed fan-out, §5 Closed state, §8 help, S3.1, and whether Closed weakens scheduler/pool/classifier sentences. No cargo. No writes. No peer current-round Consistency/Surface reports.

## 1. Findings

No open findings.

**Safety-1 P2-1 re-trace:** Non-retriable setup and retriable failure of attempt `R+1` both enter **Closed** for the rest of the operation; no further setup; every already-queued member and every later selector of that key completes immediately with the recorded failure; next operation starts Cold; Cold is not re-entered inside the operation after Closed, including after a prior Healthy. S3.1 requires `--jobs 1` × 32 members → exactly four handshakes and uniform failure, the same after Healthy-then-exhaust, and auth under `--jobs 1` → one handshake with uniform Authentication. Original counterexamples no longer hold under the text.

**Safety-1 P3-1 re-trace:** `--max-retries` long help states the 1/2/4…/30 s wait schedule and jitter; `--ssh-timeout` “sets only the per-attempt stall” and “does not clear these waits.” `--ssh-timeout` long help marks the wait as “not this flag.” Mis-attribution is gone.

## 2. Invariant analysis

- Round-1 Safety closures (thread bound, `max_requests`, install-or-refuse caps, cold single-probe / generation rules, timeout origin, dual duration bounds) remain intact; Closed does not reopen Cold mid-operation or restore first-writer pool freeze.
- Cancel during wait still suppresses probes; late success after a fired clock stays non-reusable; body/post-reusable failures stay non-retriable; Authentication/Trust stay non-retriable and now terminal via Closed.
- `--ssh-timeout 0` still clears both network deadlines without disabling retries; cleanup/cancel remain separately bounded under the accepted timeout plan.
- Shared-key Closed after exhaustion (including `R = 0`) is an explicit shared-fate policy, not an undefined stuck state.

## 3. Risks and next action

Safety axis is clear on this hash. No Safety remediation required. Proceed with lane-owner merge of axis verdicts on `b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3` only if the other axes also GO on the same object.
