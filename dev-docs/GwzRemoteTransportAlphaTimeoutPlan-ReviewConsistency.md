# GwzRemoteTransportAlphaTimeoutPlan — CONSISTENCY-AXIS REVIEW

**Review object:** untracked working-tree draft `gwz-core/dev-docs/GwzRemoteTransportAlphaTimeoutPlan.md`, SHA-256 `773639e1b545eeba776c3faeac1b0785aef50ee33faa0ba1e984b5ec214cac4a` (verified at review start and end; not in gwz-core HEAD).
**Baseline:** gwz-dev `7b3f1bc723d6c846bd7d5a4f009602afd90a1593`; gwz-core `7bd9381f0593f759333c7d8d070f4962d4092df7`; gwz-transport `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Diagnosis draft SHA-256 `977d7884877aad024261c8bf366918f4a1903a884aab6422f30422e7648ad5f6`. Design read from gwz-core working tree (`git -C gwz-core diff --stat -- dev-docs/GwzRemoteTransportDesign.md` empty). Code claims checked at the tuple HEADs above via read/rg only.
**Date:** 2026-09-22
**Axis:** Consistency: the plan against its controlling design, diagnosis, and the code it cites. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0×P0, 0×P1, 3×P2, 1×P3 open. I pre-commit to GO on a revision that resolves P2-1, P2-2, P2-3, and P3-1 as specified.

---

## 0. Evidence base

Tuple re-checked at end: plan/diagnosis hashes and three HEAD SHAs unchanged; design still clean vs HEAD.

| Source | Role | How used |
|---|---|---|
| Plan `@773639e1…` | Object | §§1–5, phases S1.1–S5.2, dependency sketch |
| Diagnosis `@977d7884…` | Controlling diagnosis | Coupling claim, PeerFailed claim, next-step list, pause claim |
| Design §10 / §10.1 / §10.2 (HEAD) | Controlling design | Verbatim quotes below; request-tightening; Open shorten; disabled-zero |
| `GwzRemoteTransportReleaseReadiness.md` | Pause claim only | Status **paused**; plan says it does not resume that gate |
| Cited code at tuple HEADs | Behavioral truth | `transport_host/mod.rs`, `session/driver.rs`, `agent_job.rs`, `ssh_setup.rs`, `transport_support.rs`, `pool/{mod,lifecycle,clock}.rs`, libgit2 socket/ssh timeout sites |

**Design quotes (verbatim):**

§10 connect/auth row (design L662–663):

> `| Connect/auth network | Socket setup and protocol progress; preserve configured native timeout semantics |`

§10.1 Open shorten (design L705–707):

> `` `Config::io_timeout_ms` defaults to 3,000 ms for a standalone stream; endpoint construction captures its configured native timeout and an Open request can only shorten it. ``

§10.2 disabled / representation / request-tighten (design L762–776, L779–784):

> `The pre-freeze implementation contact found that native GWZ accepts `--ssh-timeout 0` and core `configure_server_timeout_ms(0)` to disable network timeouts.`
>
> `Existing `Deadlines.connect_ms` and `io_ms`, pool `connect_timeout_ms`, and stream `io_timeout_ms` use zero for disabled and 1–2,147,483,647 for a finite allowance.`
>
> `A request can only tighten endpoint policy: zero is admissible only when the endpoint's corresponding network timeout is already disabled; a positive request may bound a disabled endpoint or shorten its positive configured value.`
>
> `` `Action::Connect.network_deadline` is `Option<u64>`: None means network timing is disabled, not connection completion. `` … `Cancellation/shutdown still disposes connections, and cleanup stays bounded when network timeouts are disabled.`

**Code claims checked (true as the plan states them):**

- `SshEndpointConfig::from_environment` copies `transport_timeout_ms()` into both `pool.connect_timeout_ms` and `io_timeout_ms` (`transport_host/mod.rs` L44–52).
- Open sets `connect_ms` and `io_ms` from `state.io_timeout_ms` (`driver.rs` L141–144).
- Pool `Config::default().connect_timeout_ms == 10_000` (`pool/mod.rs` L36); production SSH path overwrites it.
- Stream waiter uses `150_000 + state.io_timeout_ms` (`driver.rs` L175–176).
- `Control::quantum` returns only remainder of one `deadline: Option<Instant>` capped at 20 ms (`agent_job.rs` L47–52); `update` times out on that Instant (`L33–40`).
- Late success rejected when `NativeResource.deadline` has passed (`ssh_setup.rs` L162–179); `reusable()` only in `Idle` (`L241–243`).
- Non-auth `failure_io` wraps as `stream::Error::PeerFailed` (`driver.rs` L528–541).
- HTTPS shares initiator Open construction; endpoint also clones `config.pool` (`session.rs` L263–276).
- `apply_server_timeout` sets both libgit2 connect and server timeouts (`transport_support.rs` L56–62); socket poll / `libssh2_session_set_timeout` match the plan’s native characterization.

**Design agreement that survives (not findings):**

- Plan “zero disables both network clocks; cleanup/cancel stay bounded” matches §10.2 disabled-network and disposal rules once construction sets both fields to zero.
- Plan “positive `--ssh-timeout` does not change the 10 s aggregate” does **not** violate §10.1’s Open-shorten rule: that sentence governs `Config::io_timeout_ms` / captured **native** (stall) timeout; after the intended split, Open `io_ms` equals capture and Open `connect_ms` equals endpoint `connect_timeout_ms` (10_000), so neither field lengthens its corresponding policy under §10.2 tighten rules.
- Those behaviors are **target** policy requiring S1.1’s amendment of the §10 connect/auth “native timeout semantics” cell; they are not present in current design text.

---

## 1. Findings

### P2-1 — Stall “completed wait” lifecycle is half-specified; live setup never demarcates waits

**Location:** Plan §2 table + S3.1 (`Control` in `agent_job.rs` only) + Phase 3 milestone; production wait sites in `ssh_network.rs`, `agent_socket.rs`, `agent_auth.rs`, etc. (uncited).

**Violated invariant:** A plan that claims §1 (“setup that keeps completing native calls to finish”) and Phase 3 (“progressing multi-stage setup succeeds when sum > stall”) must specify both halves of the stall lifecycle and wire them on the path cold fetch actually runs.

**Reproduction / state sequence:**
1. Today `Control` has a single aggregate `deadline` and only `check` / `quantum` / `cancel` — no wait-boundary API (`agent_job.rs` L27–53).
2. Live setup polls with many `check()` / `quantum()` calls **inside** one logical DNS/TCP/handshake/agent wait (e.g. `agent_socket` connect loop; `ssh_network` resolve→connect→handshake).
3. S3.1 adds stall expiry and says “A completed wait starts a new stall allowance” but names neither a `complete`/`advance_wait` primitive nor any step that teaches production stages to call it.
4. S3.2 only drives “four waits” in a **scripted test**. No phase edits the real stage graph.
5. Therefore after Phases 1–4 as written, a progressing multi-stage live setup still spends one stall budget across DNS+TCP+handshake+agent — the same coupling class the diagnosis names — while only the scripted test demonstrates reset.

**Impact:** The plan’s stated outcome and Phase 5 default-timeout cold-fetch gate are not entailed by the steps. Implementers can “pass” S3.2 and still ship an alpha that fails the reproduced defect.

**Required correction:** In S3.1, name the wait-completion API (and how stall start is recorded). Add an explicit step (or expand S3.1/S3.2 scope) that marks wait completion at production stage boundaries (DNS done, TCP connected, handshake done, each agent wait done) — or redefine “wait” to a precise, implementable rule that those call sites already satisfy, and show that rule against current `check`/`quantum` use. Require a live-path regression, not only a scripted four-wait harness.

**Closure / regression test:** Deterministic test on the **production** setup function graph (or a thin wrapper used by it): four real stage completions each < stall, sum > stall, aggregate ahead → success; one stage with no completion past stall, aggregate ahead → `TimedOut`, `reusable() == false`.

---

### P2-2 — S3.2 stall case is not satisfiable against Control’s hard-coded wall `Instant`

**Location:** S3.1 vs S3.2; `Control::update` / `quantum` use `Instant::now()` (`agent_job.rs` L36–51).

**Violated invariant:** A required regression that forbids wall-clock sleep must be executable against the clock seam the preceding step actually creates.

**Reproduction / state sequence:**
1. S3.2: stall case needs “one wait idle past the stall allowance”; “Use a deterministic clock or injected waits. Do not sleep on wall-clock GitHub.”
2. S3.1 only describes stall/aggregate remaining on `Control`; it does not require injecting a monotonic test clock (or equivalent non-wall accounting) into `update`/`quantum`.
3. Aggregate and stall expiry today/as specified compare to `Instant::now()`. Without a seam, the stall case must either sleep on the runner or invent an API S3.1 did not require.
4. “Injected waits” that merely call an undefined completion API still cannot expire a wall-Instant stall without advancing time.

**Impact:** Phase 3’s acceptance milestone is not mechanically closable as written; agents will either sleep (forbidden) or silently under-test stall expiry.

**Required correction:** S3.1 must add a deterministic time source (or quantum/budget accounting that tests can advance without wall sleep) used by both stall and aggregate checks on `Control`, and S3.2 must drive that seam.

**Closure / regression test:** Same stall case as S3.2 with fake clock: advance past stall while aggregate remains in the future → `TimedOut`; no `thread::sleep` / real GitHub.

---

### P2-3 — S4.1 retargets late-result to a distinct aggregate but §4 lets S4.1 ship without S2

**Location:** S4.1 text vs §4 dependency sketch; `ssh_setup.rs` L111–128, L162–179 (single Instant from pool `network_deadline`).

**Violated invariant:** Internal plan consistency: a step that depends on two distinct deadlines cannot be ordered before the step that creates the distinction.

**Reproduction / state sequence:**
1. Today Job/`NativeResource` deadline **is** the coupled connect Instant (Open/`from_environment` copy the I/O setting into connect). There is no separate “3 s I/O Instant” on that path to retarget away from.
2. S4.1: “point it at the aggregate and cancel deadlines rather than at the 3-second I/O value.”
3. Only S2.1/S2.2 make pool/`Deadlines.connect_ms` the 10_000 aggregate while `io_ms` stays the stall setting.
4. §4 attaches S4.1 only under S3.1→S3.2, **not** under S2.2. S4.1 can be “done” while Open still sends `connect_ms == io_ms == 3000`, so late-result still keys off the coupled 3 s Instant.
5. S4.1’s “success after the aggregate Instant” claim is then either vacuous or false relative to §2.

**Impact:** Disposal/late-result work can merge without the domain split; S5.1 can “review aggregate non-reuse” against code that never received a distinct aggregate.

**Required correction:** Make S4.1 depend on S2.2 (and S3.1). State that the late-result Instant is the pool aggregate connect deadline after uncoupling, and cancel remains the existing cancel/cleanup path.

**Closure / regression test:** With `io_ms=3000`, `connect_ms=10000`: success arriving at t=5 s after cancel → dropped, `reusable()==false`; success after aggregate Instant with stall disabled/not fired → dropped; progressing success before aggregate → accepted.

---

### P3-1 — S1.1 does not name the exact §10 / §10.2 sentences it supersedes

**Location:** S1.1; design L658–668, L760–792.

**Violated invariant:** Supersession exactness — a reviewer must see which controlling sentences are replaced, not only a topical paraphrase.

**Reproduction / state sequence:** S1.1 says amend “§10 and §10.2”, “extend the connect/auth row”, and assert aggregate/`io` split and coupled zero-disable, while “§10.1 is unchanged.” It does not quote or pinpoint: the connect/auth cell (L663); §10.2’s identification of `--ssh-timeout` / `configure_server_timeout_ms` with undifferentiated network timeouts (L762–767); the joint listing of `Deadlines.connect_ms` and `io_ms` (L770–771); or whether request-tighten sentences (L774–776) stay verbatim.

**Impact:** A later design edit can over- or under-amend (e.g. accidentally rewrite §10.1 Open-shorten, or leave L663 readable as “connect_ms preserves native stall”), defeating the graph the plan claims to update first.

**Required correction:** S1.1 must list superseded clauses by section + quoted sentence (or stable anchors) and the replacement sentences, including an explicit “unchanged” list for §10.1 L705–707 and §10.2 tighten/disposal lines if they remain.

**Closure / regression test:** Design diff review checklist: every S1.1 listed quote appears struck/replaced; every “unchanged” quote still present; no silent edit to §10.1 Open-shorten.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| Internal §§1–5 / phase / §4 agreement | **Fail** — P2-3 (S4.1 vs deps); P2-1 (Phase 3 milestone vs steps) |
| Verbatim design §10 / §10.1 / §10.2 agreement for zero-disable and positive aggregate independence | **Pass** for target policy relative to quoted §10.1 Open-shorten and §10.2 tighten/disable, **conditional on** S1.1 actually rewriting L663 native-semantics (P3-1 blocks auditability) |
| S1.1 supersession exactness | **Fail** — P3-1 |
| Cited current-code behavioral claims | **Pass** (coupling, 10_000 default, 150 s waiter, quantum, late-result, PeerFailed, HTTPS shared Open/pool, dual libgit2 apply) |
| S3.1 / S3.2 / S4.1 satisfiable as written | **Fail** — P2-1, P2-2, P2-3 |
| Pause claim vs ReleaseReadiness | **Pass** — readiness is paused; plan scopes S5 to this timeout program and keeps broader gate paused |
| Diagnosis alignment | **Pass** on defect class and “do not raise global timeout as fix”; diagnosis next-steps assume production multi-stage regress — plan under-delivers that (P2-1) |
| Unstated impacts | HTTPS connect budget lengthening is **stated**. Unstated residual: every `Job::start` caller (identity-check jobs in `ssh_worker.rs`) must learn the new stall argument/default; pool `ConnectClock` interaction pause is unused on the SSH setup path today, so no separate finding, but a wall-Instant stall would not honor §10 “interaction excluded from network accounting” if that path appears later |

Operator decision that the permanent fix is the two-clock split (not raising `--ssh-timeout`) was deferred and not re-litigated.

---

## 3. Risks and next action

Blocking risk: implementing the plan as written can produce a reviewed, tested “fix” that still couples stall lifetime to the whole setup on the live path, with Phase 3 tests that cannot deterministically prove stall expiry, and late-result work that never sees a distinct aggregate.

**Next action:** Revise the plan to close P2-1, P2-2, P2-3, and P3-1; re-run this consistency axis on the new draft hash. No code or design edits were made in this review.
