# GwzRemoteTransportAlphaTimeoutPlan — SAFETY-AXIS REVIEW

**Review object:** Untracked working-tree draft `gwz-core/dev-docs/GwzRemoteTransportAlphaTimeoutPlan.md`, SHA-256 `773639e1b545eeba776c3faeac1b0785aef50ee33faa0ba1e984b5ec214cac4a` (not in `gwz-core` HEAD). Reviewed 2026-09-22.
**Baseline:** `gwz-dev` `7b3f1bc723d6c846bd7d5a4f009602afd90a1593`; `gwz-core` `7bd9381f0593f759333c7d8d070f4962d4092df7`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Plan and diagnosis read from the working tree at the stated hashes (verified at start and end). Design §10 / §10.1 / §10.2 read from `gwz-core` working tree with empty diff vs HEAD. Cited code inspected read-only at those HEADs. No builds, tests, network, or tree mutations.
**Date:** 2026-09-22
**Axis:** Safety: what the plan's rules permit to go wrong. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 1×P0, 1×P1, 3×P2, 1×P3 open. I pre-commit to GO on a revision that resolves P0-1, P1-1, P2-1, P2-2, and P2-3 as specified.

---

## 0. Evidence base

| Item | How read | Result |
|---|---|---|
| Plan hash | `shasum -a 256` start + end | `773639e1…14cac4a` unchanged |
| Diagnosis hash | same | `977d7884…48ad5f6` unchanged |
| Repo HEADs | `git rev-parse` start + end | tuple unchanged |
| Design §10–§10.2 | `GwzRemoteTransportDesign.md` L633–792; `git diff` empty | matches HEAD |
| Coupler today | `transport_host/mod.rs` L44–52; `session/driver.rs` L141–144 | `io_timeout_ms` copied into pool `connect_timeout_ms` and both Open deadline fields |
| Setup Job clocks | `ssh_setup.rs` L111–128; `ssh_pool.rs` L209–218; `agent_job.rs` L27–53, L181–242 | Job/Control receive only the Connect `network_deadline` Instant (+ cleanup); no stall input |
| Poll loops | `ssh_network.rs` `wait_socket` / `wait_session`; `agent_auth.rs` EAGAIN `sleep(quantum)` | slice waits return `Ok` / sleep completes with no peer progress |
| Late install gate | `ssh_setup.rs` L161–179 | `live = deadline.is_none_or(Instant < at)` plus identity/auth flags; miss → `PermissionDenied` |
| Pool interaction | `pool/clock.rs` `begin_interaction` / `end_interaction` | pauses Connect network budget; no `gwz-core` production caller today; API is live in the connect domain |
| HTTPS budgets | `https_worker.rs` `budget_for_open` / `configured_deadlines`; Phase 2 prose L83–85 | Open `connect_ms` feeds HTTPS connect budget |

Authorities checked: diagnosis; Design §10 (separate domains; interaction excluded from network accounting); §10.1 unchanged by plan; §10.2 zero/disabled and bounded cleanup. Deferred outcome (two-clock split is the permanent fix; raising `--ssh-timeout` is not) was not re-litigated; whether the plan’s rules still permit a worse or falsely qualified path is in scope.

---

## 1. Findings

### P0-1 — Phase 5 can close on a cumulative-budget lengthening that the plan itself rejects as a non-fix

**Location:** Plan §2 Phase 2 (L79–101) + S3.1 file scope (L105–114) + S5.1–S5.2 (L151–166), against Outcome (L20–25) and §5 (L189–190).

**Violated invariant:** Outcome: a no-progress native wait still expires at the 3 s stall setting; raising the global/setup budget is not the fix. Qualification must not accept evidence that only shows a longer cumulative setup window.

**Reproduction (plan rules):**
1. Implement S2.1–S2.2 only: pool `connect_timeout_ms` and Open `connect_ms` become 10_000; Job still receives only Connect `network_deadline` (`ssh_setup.rs` / `ssh_pool.rs` as today).
2. Implement S3.1 only inside `agent_job.rs` as scoped; do not add a new step that feeds `Deadlines.io_ms` / endpoint `io_timeout_ms` into `SetupConnector`/`Job`/`Control` (no such step exists). Production setup is then bounded by the 10 s aggregate alone.
3. S3.2 passes with scripted Control tests that inject stall locally.
4. S5.1 reviews S3.1 text; S5.2 cold fetches that previously needed `>3 s` now succeed inside 10 s at default `--ssh-timeout`.

**Impact:** The alpha can be declared fixed while the only effective production change is widening the cumulative setup deadline 3 s → 10 s — the same class of change as the deferred `--ssh-timeout 15` workaround. False composition of release evidence; hung setups also run longer than today’s alpha (worse failure).

**Required correction:** Make production stall enforcement a dependent, shippable step before S5.2 (value path + stage boundaries; see P1-1). S5.2 must refuse close-out unless retained evidence shows a true per-wait stall expiry path (deterministic fixture or traced production signal), not merely default-timeout live success. Explicitly forbid treating “default cold fetch pass after connect_ms=10000” as proof of the two-clock fix.

**Closure / regression:** A qualification gate test: with stall wired and aggregate at 10 s, one injected idle stage expires at ~3 s; with stall deliberately unwired, S5.2 is marked failed even if live fetches pass.

---

### P1-1 — “Completed wait” has no Control signal and no production stage wiring; both wrong resets are permitted

**Location:** S3.1 (L105–114), §2 table (L43–45), Phase 3 milestone (L103), S3.2 (L116–123); production poll loops in `ssh_network.rs` / `agent_auth.rs` (cited by the plan as code an implementer would change, but not scheduled).

**Violated invariant:** Stall resets only when a native wait completes; a wait with no progress expires at the stall allowance while the aggregate remains in the future; Outcome’s 3 s no-progress claim.

**Reproduction:**
1. **Missing half of the pair:** S3.1 requires “a completed wait starts a new stall allowance” and “no completion for that allowance,” but names no `begin_wait` / `complete_wait` (or equivalent) on `Control`, and no default for when the current wait started.
2. **Idle-slice interleaving:** Under today’s `wait_socket` / `wait_session` / agent EAGAIN `sleep(quantum)`, each slice can return without peer progress. If an implementer treats slice completion or a successful `check`/`quantum` as “completed wait,” stall resets every ≤20 ms while TCP/handshake/agent make no Git progress; stall never fires; only the 10 s aggregate ends the attempt (worse than today’s 3 s cumulative kill).
3. **No-reset interleaving:** If nothing ever marks completion, stall is one 3 s window from Job start — the old cumulative coupling on Control — and Phase 3’s “sum > stall succeeds” milestone is achievable only in S3.2 scripts, not on the live `ssh_local` → `ssh_network` / `agent_auth` path.
4. No phase step edits those production stages to define the four waits named in §2.

**Impact:** Plan rules allow either an ineffective stall (hung path stretches to 10 s) or a still-cumulative stall (progressing multi-stage still dies at 3 s). Release blocker for the stated safety outcome.

**Required correction:** S3.1 must specify the wait lifecycle API and that idle poll/sleep slices are not completions. Add an explicit step (or expand S3.1/S3.2 dependencies) so production DNS, TCP, handshake, and agent stages begin/complete waits. Require a production-path regression, not only scripted Control tests.

**Closure / regression:** Hung TCP connect: stall `TimedOut` near 3 s with aggregate still ahead; four production-shaped stages each &lt; stall, sum &gt; stall, succeed; agent EAGAIN sleep loop for &gt; stall with no auth progress → stall timeout (not reset).

---

### P2-1 — Phase 2 widens HTTPS (and every Open caller) connect budget with no bound

**Location:** Phase 2 prose L83–85; S2.2 L99–101; §5 L183–184; no Phase 5 HTTPS work.

**Violated invariant:** Design §10 keeps timeout domains separate, but a collateral widen of another scheme’s connect budget needs an explicit bound or qualification so blast radius does not grow unnoticed. “Never worse than status quo” fails for stuck HTTPS connects.

**Reproduction:** Today `from_environment` sets pool `connect_timeout_ms` from `transport_timeout_ms()` (default 3 s) and Open copies that into `connect_ms`. After S2.1–S2.2, positive `--ssh-timeout` leaves pool connect at 10_000 and Open sends `connect_ms=10000`. HTTPS `budget_for_open` uses `deadlines.connect_ms`. S2.2 only asserts the shared path does not copy I/O into `connect_ms`; nothing caps HTTPS at the prior effective 3 s or qualifies stuck-connect behavior.

**Impact:** Stuck HTTPS connect/auth holds pool capacity up to ~10 s instead of ~3 s per open; larger failure blast radius under the plan’s own Phase 2 rules. §5 forbids changing the 10 s default but does not bound this widening.

**Required correction:** State the HTTPS (and other Open) connect-budget change as an intentional, accepted collateral with a bound (preserve prior effective ceiling, scheme-specific override, or mandatory HTTPS stuck-connect qualification before alpha close). Do not leave it as an unbound side effect of the SSH fix.

**Closure / regression:** HTTPS Open deadline test locks the chosen connect budget; a stuck-connect fixture proves expiry at that bound, not an accidental further widen.

---

### P2-2 — S5.2 demands stall-vs-aggregate distinction the plan makes indistinguishable

**Location:** S3.1 L112 (“Aggregate expiry and stall expiry are both `ErrorKind::TimedOut`”); S4.2 L139–144 (setup stage only); S5.2 L165–166; diagnosis L29–31 (no per-stage timing).

**Violated invariant:** Failure follow-up must not choose “separately qualify the aggregate” vs “stall still broken” on evidence that cannot tell them apart.

**Reproduction:** Cold fetch fails at default. Both causes surface as `TimedOut` / setup-stage timeout. No step adds a distinct code, fact, or trace. S5.2 still says “record whether a single wait stalled or the 10-second aggregate expired,” then points aggregate follow-up at that record.

**Impact:** Operators can attribute a stall bug to the aggregate (or the reverse) and change the wrong constant; diagnosability defect with concrete wrong-next-action risk.

**Required correction:** Distinguish causes in the failure surface or retained trace (separate kinds, structured reason, or mandatory stage/wait instrumentation before S5.2). Until then, S5.2 must not claim that distinction.

**Closure / regression:** Two fixtures — idle single wait vs many short waits past 10 s — assert different caller-visible or retained causes.

---

### P2-3 — New stall clock has no rule for pool interaction pauses

**Location:** Plan silence vs Design §10 user-interaction row (L666) and §10.1 / pool `begin_interaction` / `end_interaction` (`pool/clock.rs` L121–191). Plan §2–§3 never mention interaction.

**Violated invariant:** Design: supported user interaction is excluded from network timeout accounting. Pool already pauses the connect network (aggregate) clock during interaction. The plan adds a second setup network clock (stall on `Control`) with no pause/resume rule.

**Reproduction:** During Opening, host calls `begin_interaction` (API exists; pool tests exercise it). Aggregate pauses. Control stall, under S3.1, continues to charge wall time via `check`/`quantum`. A visible helper wait longer than `--ssh-timeout` fails stall while aggregate is correctly paused — or an implementer invents an ad hoc pause, breaking §2.

**Impact:** Composition hole in the connect/auth domain this plan edits; permits setup timeout during legitimate interaction or inconsistent clocks. Not cured by “no production SSH caller today.”

**Required correction:** State whether interaction spends stall, aggregate, both, or neither; align with Design §10 (interaction excluded from network accounting) and with pool pause semantics; add a deterministic interaction-during-setup test when stall is introduced.

**Closure / regression:** `begin_interaction` for &gt; stall allowance with aggregate paused → no stall timeout; network wait after `end_interaction` still stall-bounded.

---

### P3-1 — Late-success rejection kept as `PermissionDenied`, and `live` is not liveness

**Location:** S4.1 L134–137 (“Keep the current late-result rejection”); `ssh_setup.rs` L161–179.

**Violated invariant:** Outcome: ordinary output names a setup timeout as a setup failure; late success must not become a reusable connection. Current gate is aggregate `Instant` (always true when deadline is `None`) plus `session().authenticated()`, not peer liveness or cancel beyond Job discard.

**Reproduction:** Success after aggregate expiry fails `live` and returns `PermissionDenied`, not timeout/setup. With `--ssh-timeout 0`, `deadline` is `None`, so `live` is always true; non-reuse depends entirely on Job cancel discard. A result that is still `authenticated()` for reasons S4.1 never names (stale libssh2 auth flag, identity match) passes the named gate if Job returns `Ok`.

**Impact:** Bounded mis-diagnosis (auth vs timeout) and a thin late-install gate the plan freezes in place.

**Required correction:** Map late aggregate/cancel rejection to the setup-timeout path; require cancel/aggregate (and stall, if applicable) to be visible to the install gate, not only `Instant` + auth flags.

**Closure / regression:** Late Ok after aggregate and after cancel → not `Idle`, `reusable()==false`, caller-visible setup timeout (not `PermissionDenied`).

---

## 2. Invariant analysis

| Claim / rule | Under plan rules | Result |
|---|---|---|
| Progressing setup can exceed 3 s stall and still succeed | Requires per-stage stall reset on production path | **Not delivered** by scheduled steps (P0-1, P1-1) |
| No-progress wait expires at 3 s | Ambiguous completion + possible aggregate-only bound | **Can fail**; hung path may run to 10 s (P1-1) |
| `--ssh-timeout 0`: no network deadline; cancel/disposal bounded | S4.1 states this; cleanup clocks remain separate (§10.2) | **Held** in text; install gate still weak when `deadline=None` (P3-1) |
| Late success / cancel not pooled | S4.1 requires drop; current `live` + Job discard | **Mostly held**; PermissionDenied and non-liveness `live` remain (P3-1) |
| Credential-offered then timeout: dispose, non-reuse, setup failure | S3.2 / S4.1 / S4.2 | **Held** at resource level; observation nuance not blocking |
| HTTPS / shared Open connect budget | Phase 2 admits widen to pool aggregate; no bound | **Worse than today** for stuck HTTPS (P2-1) |
| Interaction excluded from network accounting (§10) | Stall silent vs `begin_interaction` | **Hole** (P2-3) |
| Stall vs aggregate diagnosable at S5.2 | Both `TimedOut`; S4.2 stage only | **Broken** (P2-2) |
| §5 out-of-scope (no ConnectClock progress reset; no §10.1 on setup; no new schema/flag) | Steps stay off those | **No scope-creep finding** |
| “Not worse than status quo” | Phase 2 alone lengthens cumulative setup; stall may be ineffective | **Fails** (P0-1, P1-1, P2-1) |

Walks requested: progressing sum &gt; 3 s depends on P1-1; single never-completing wait depends on P1-1; `--ssh-timeout 0` cancel/disposal bounded in S4.1; stall-never-expires interleaving constructed under idle-slice completion (P1-1); late install via always-true `live` when deadline disabled (P3-1).

---

## 3. Risks and next action

Highest risk is shipping an alpha that only lengthens the cumulative setup budget to 10 s, passes S5.2 live fetches, and still lacks a real per-wait stall on the production SSH path — while hung and HTTPS opens get worse. Secondary risks: wrong aggregate follow-up from indistinguishable timeouts, and stall firing during interaction pauses.

**Next action:** Revise the plan to (1) wire stall end-to-end with an explicit wait lifecycle and production stage steps, (2) harden S5.2 against cumulative-only false closes and require distinguishable stall/aggregate evidence, (3) bound or qualify the HTTPS connect widen, (4) define stall vs pool interaction, then re-run this safety axis.
