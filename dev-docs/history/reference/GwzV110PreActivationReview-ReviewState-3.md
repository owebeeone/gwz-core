# 1.1.0 pre-activation review (D19): STATE-axis re-verdict (round 2 remediation)

**Review object:** The round-2 remediation of the five D19 objects, reviewed in the frozen workspace `/Volumes/projects/limbo/gwz-dev-review19c`:
1. TR2.6, the Phase 2 transport tree.
2. S3.1, the timeout plan's S5.1 review.
3. S3.2, the Q6 aggregate.
4. WPP 4.8 / TR4.10, SSPI and gwz-sspi.
5. WPP 4.2/4.3, the WH2 helper owner.

The inputs are `GwzV110PreActivationReview-RemPlan-2.md`, the design amendments, and the diff since the round-1 re-verdict tuple:
- gwz-core `8e425e76..aee4c257`: `ecf1cdf7` (the remediation), `6560a132` (tests), `18c520b8` (documents);
- gwz-cli `b7c9edcd..8d909ec4`.

**Baseline:**
- root `90dff3787dd4be881a06e5ff8037af035cbe2b77`
- gwz-core `aee4c257d24ffa65ea77f1266363dca619f698b4`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (unchanged)
- gwz-cli `8d909ec4886118789c06295ccd13a87bb496432a`
- gwz-py `da098f5c52e822a3f7ed0920b016e9b0c5c5a10a` (unchanged)
- gwz-sspi `d3038882f7937be023d1835fe1b03e0309260acb` (unchanged)
- gwz-core-evidence `ce3b5d931833636bac12d6681e6bead5498d6481` (reference only)

**Date:** 2026-10-16. **Axis:** STATE. Independent, adversarial, read-only.

**Verdict: GO.** P0 0, P1 0, P2 0 open, P3 5 open.
- Three P3s sit with the operator: P3-1, P3-2 and P3-5.
- Two P3s are new: P3-8 and P3-9.

P2-3 is closed and verified, as I pre-committed in round 1. P3-6 and P3-7 are closed. I found no new architectural root cause on any object.

| Object | Verdict | Open findings |
|---|---|---|
| 1 (TR2.6 tree) | GO | P3-5 (operator) |
| 2 (S3.1 clocks) | GO | none |
| 3 (Q6 retirement fix) | GO | none |
| 4 (SSPI, gwz-sspi) | GO | P3-1 (operator); P3-8, P3-9 |
| 5 (WH2 owner) | GO | P3-2 (operator) |

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-3 (obj 4): a Basic credential was sent on the challenge's connection whether or not it could carry it, and an unusable one failed as `Authentication` | `ecf1cdf7`: the credential goes on the challenge's connection only if the connection is still held, `challenge_usable` holds and `send_once` starts the request. Otherwise `lease_again` takes a fresh connection scoped to the credential. An SSPI 401 marked `Connection: close` takes a fresh connection before the first token. | **Yes.** `exchange.rs` `answer_basic` (`challenge_usable`, then `send_once`, else `lease_again`). My two counterexamples are now rows in `native/tests/reconnect.rs:12` (`idle = 1 s`, helper 2 s) and `:37` (`Connection: close`, helper 0 s). Both authenticate, with `accepted == 2`, `credential_offered` true and `starts == 0`. The not-started send returns `None`, restores `credential_offered` and re-leases. That was verified by reading; it has no deterministic row (gap (c), accepted). The SSPI `Connection: close` row is `:122`. Run: pass. | Closed |
| P3-7 (obj 4): the challenge lease was held through the whole lookup | A biased `select!` of the lookup against `sleep(hold)`. When the hold lapses, the challenge is dropped and the lease finished `Discarded`. | **Yes.** `exchange.rs` `answer_basic`. Row `reconnect.rs:156`: 7 s into a 10 s helper the challenge connection is closed (`open == Some(0)`), and the open still authenticates on a fresh connection. Run: pass. | Closed |
| P3-6 (obj 4): a throttle after an SSPI exchange starts a new exchange with a fresh D, contrary to the design | The design is amended: Windows parity design §7 ("Amendment (2026-10-16 …)") and §9; the WH integration design's D clause. Rounds stay carried. | **Yes, for the cases the amendment names.** Row `native_round_tests.rs:179` pins the second attempt's own D. The amendment text omits two requeue causes the code has; filed as **P3-9**. | Closed (residual P3-9) |
| P3-1 (obj 4): gwz-sspi pipes created inheritable | With the operator (decide before S7.1). | Unchanged at `d3038882`. Acceptable for a P3. | Open (operator) |
| P3-2 (obj 5): WH2 spawn gate; C9 site list | With the operator (decide before S7.1). | Unchanged. Acceptable for a P3. | Open (operator) |
| P3-5 (obj 1, ARCHITECTURAL): exit joins async `call()` executor threads | With the operator. | Unchanged. Acceptable for a P3. | Open (operator) |

The round-1 findings P2-1, P2-2, P3-3, P3-4 and follow-up 1 were closed in my previous report. The targeted run confirms their closure rows still pass (`backstop_tests`, `closure.rs`, `native_round_tests`, `https_auth`).

## Changed-range analysis

**What changed:**
- `ecf1cdf7`, the round-2 remediation;
- `6560a132`: test rows only (`ssh_fixture/dial*`), no product code;
- `18c520b8`: documents, including the amendments;
- gwz-cli `8d909ec`: a Troubleshooting entry and its pin test, documents only.

**The connection-usability decision in `answer_basic`.**
- The credential goes on the challenge's connection only when all three hold: the lease is still held (the hold has not lapsed), the challenge drained cleanly with the connection alive and without `Connection: close` (`challenge_usable`), and hyper actually started the request (`send_once`).
- Otherwise `lease_again` takes a fresh connection.
- A `challenge_usable` error, such as `Timeout` under D or `Cancelled`, returns that error. This is fail-closed and is not reported as `Authentication`.
- The credential is marked offered only when a send started.
- No regression found.

**The SSPI fresh connection and its channel binding.**
- `lease_again(prepared, None, …)` runs before the request is built.
- The binding is read afterwards from the new lease's connection (`exchange.rs`, the `channel_binding(...)` read follows the re-lease). So the token is bound to the connection that carries it, never the closed one.
- The fresh connection is unscoped during checkout, with `fresh = true`, so it is a new connection, not a pooled one. It is scoped to the exchange's unique scope once Start completes, and it stays leased throughout, so no other open can take it.
- The re-lease runs under D, and it runs before any SSPI work, so the `Guard`'s ownership is unaffected.
- A close in the middle of an exchange still ends it (`auth_send`). That is correct, because the context belongs to its connection. Row `reconnect.rs:143`.
- **Residual: a connection that dies without the header** before the first token still fails `Authentication`. Filed as **P3-8**; this is the lane's gap (b).

**`throttle::Setup` and its shared use by `prepare`.**
- `Setup::failed` and `Setup::connected` reproduce the old first-hop code line for line: the 407 naming, `setup_cause`, `FirstConnect::Failed`, `judge_setup` and `Connected`.
- `prepare`'s behaviour is unchanged.
- The re-lease uses `first = hops == 0`, the same rule the `gh` path's post-401 checkout follows (its loop `continue`s with `hops` unchanged). Its connect is therefore the open's setup, and a refused re-lease retries before effect, with nothing sent (row `reconnect.rs:176`).
- Overwriting an earlier `Connected` with `Failed` is the same semantics the `gh` path already has. Accepted.

**The spent-rounds settle path.**
- The requeue branch now finishes an open whose carried rounds are spent, with the throttle, through `spent` (`retry.rs`).
- **Gap (a): the `Outcome::Retry` branch still carries spent rounds.** Reaching it needs a setup-phase failure (`FirstConnect::Failed`) in an attempt whose carried rounds are already eight. That cannot be reached:
  - rounds rise only inside an exchange, and an exchange's end is `Phase::Other`;
  - the SSPI re-lease precedes the round check only when rounds are under eight, because the cap check comes first;
  - a carry of eight now ends at the requeue.

  At worst the outcome is fail-closed (`Protocol`, with no SSPI work). Not a finding.

**The P3-7 lease release.** After a lapse, `prepared.lease` is `None` until `lease_again`. An error in the lookup returns with no lease, so nothing dereferences it. `hold = min(cleanup, 5 s)`: a zero cleanup releases at once, which is safe.

**Common check (problem, necessity and authorized scope).** Every change traces to a RemPlan-2 row:
- State P2-3, P3-6 and P3-7;
- Code P3-3, P3-4 and P3-5.

No change widens credential or trust scope. A credential is sent only to the same origin, on a connection scoped to it. The re-sent logon identity on a requeue is the amended design's stated behaviour.

**The amendments.**
- **D clause (WH integration design, after the D clause; parity design §7 and §9).** The helper part is exact: D does not run during a Basic lookup and is moved on by the lookup's duration.
- **The requeue part is not exact.** It names "429, or 503 with `Retry-After`" and calls this "the only other case in which a new exchange starts". The code also requeues after an exchange in two more cases (P3-9):
  - a 403 with `Retry-After`, which `throttle.rs` `said` treats as a throttle;
  - a bare 503 that the limit machine takes as evidence (`Said::Unanswered`, then `requeues(ruling, false)`).
- Nothing else in the two documents is contradicted.

## 0. Evidence base

**Tuple.** I checked it at the start and at the end. It was unchanged, with only the untracked files that existed before I started (5 in root, 1 in gwz-core, 1 in the evidence member).

**Read fully:**
- RemPlan-2, and the "Outcome of round 1" section of the RemPlan;
- the three amendments;
- `git show ecf1cdf7` for `native/exchange.rs`, `prepare.rs`, `throttle.rs`, `budget.rs`, `native.rs`, `retry.rs` and `https_failure.rs`;
- `native/tests/reconnect.rs` (`:1-62`, `:150-201`);
- `exchange.rs:100-215` at the tuple;
- `throttle.rs` `said` and `tell_governor`.

**Sampled:**
- the `native_round_tests.rs` row names;
- `6560a132` and gwz-cli `8d909ec` (file lists).

**Not read:**
- the Troubleshooting text, which is the Code and Surface axes' concern;
- the r19auth2 README beyond the receipt.

**Commands:**
- `prepare.py` into `/Volumes/projects/limbo/build-targets/review19r3-state-cand`;
- the candidate `cargo +1.95.0 test --lib` (target `review19r3-state-target`, `GWZ_TEST_GIT=real`) with filters `backstop native_round https_worker https_endpoint transport_host::session::cleanup_tests https_auth setup_retry`: **559 passed, 0 failed**;
- the checkers:
  - Windows parity: 430 files, 196 occurrences, nothing new;
  - cfg boundaries: 1,637 files, nothing new;
  - process globals: 18 allowlisted, nothing new;
  - candidate switches: 23 sites match;
  - checked-artifact boundaries: ok.

Windows-only behaviour was judged from receipt `r19auth2-20261016-c` and run `2026-10-16-r19auth2`.

## Problem, requirement provenance, and scope

As in the earlier reports. P3-8's authority is RemPlan-2's own disposition ("… takes a fresh connection the same way, or the refusal is stated") and P2-3's rule (never report `Authentication` for a connection that could not carry the request). P3-9's authority is the instruction to judge whether each amendment states the behaviour exactly.

## 1. Findings

### P3-8 (object 4). Before the first SSPI token, a challenge connection that died without a `Connection: close` header still fails the open as `Authentication`

- **Root cause.** The SSPI path checks whether the challenge connection is usable only after Start, and has no fallback once the token is bound.
- **Location (gwz-core):** `src/git/endpoint/https_worker/native/exchange.rs`. On the first step, `auth_send(prepared, Some(response))` → `challenge_usable` returns `false` → `Err(Authentication)`. This is the lane's gap (b).
- **Violated invariant.** P2-3's rule: an open is not failed as `Authentication` for a connection that could not carry the request. RemPlan-2's disposition requires a fresh connection, or a stated refusal.
- **Interleaving.**
  1. A `Negotiate` or `NTLM` 401 arrives without `Connection: close`.
  2. Worker launch and Hello (Start) take longer than the server's idle timeout, and the server closes the connection.
  3. The first token cannot be sent.
  4. The open fails `Authentication` with SSPI facts. No token was sent and the identity was never judged.
- **Impact.** A misleading refusal and no retry (`Phase::Other`). The window is narrow: the server's keep-alive must expire within the worker's start.
- **Correction.** Drain the challenge and test the connection before building the request (Start binds the connection's channel binding). If it dies during Start, either restart the conversation on a fresh connection, or report a connection loss (`Io`), not `Authentication`, and state that in the design's boundary text.
- **Test.** A fixture that closes the connection 200 ms after the 401, with a fake port whose Start takes 500 ms. The open does not end `Authentication`.

### P3-9 (object 4). The State P3-6 amendment understates when an exchange is followed by a new one

- **Root cause.** The amendment lists two throttle forms and calls them the only other case, while the code requeues on a wider set.
- **Location.**
  - The amendment: `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md:201` (§7), `:239` (§9), and `dev-docs/GwzWindowsHttpsIntegrationDesign-DRAFT.md:162`.
  - The code: `src/git/endpoint/https_worker/throttle.rs` `said` (`(503 | 403, Some(_)) => Throttle`; `(503, None) => Unanswered`), `tell_governor` (`Unanswered` → `Signal::Suspect`), and `setup_retry::requeues(ruling, false)`, which is true for any evidence ruling.
- **Violated invariant.** A design amendment states the behaviour exactly.
- **Reproduction.** An NTLM exchange answered with a 403 carrying `Retry-After`, or with a bare 503 while the site's limit machine has others in flight, is requeued. A new exchange then starts, which the text says happens only for 429 or 503 with `Retry-After`.
- **Impact.** Documentation only. The behaviour stays bounded: eight rounds, the same identity and origin, `--max-retries`.
- **Correction.** State the requeue set as the code has it: any refusal the adaptive design requeues (§4.8 and §5.1), meaning a throttle (429, or a 503 or 403 with `Retry-After`) or a refusal the limit machine takes as evidence. Or narrow the code to match the text.
- **Test.** A `native_round_tests` row for a 403 with `Retry-After` after an exchange, pinning whichever rule is chosen.

## 2. Invariant analysis (attacks that failed)

- **P2-3.** Every path that cannot carry the credential falls back to `lease_again` (a lapsed hold, an unusable connection, a send that never started). The credential is marked offered only on a started send. A fallback connect is the open's setup, so its failures are retried before effect.
- **The SSPI binding.** The binding always comes from the connection that will carry the token.
- **P3-7.** The lease is released at the hold even while the helper is still working.
- **Rounds.** Spent rounds end a requeue with the throttle. The `Outcome::Retry` carry of spent rounds (gap (a)) cannot be reached, and at worst fails closed.
- **The `throttle::Setup` refactor.** `prepare`'s behaviour is unchanged.
- **Unchanged since round 1:** the backstop, the Q6 retirement, the S3.1 clocks, WH2 and gwz-py. Their rows pass.

## 3. Follow-ups outside the finding bar

1. **Gap (c).** The not-started send has no deterministic row. It was verified by reading `send_once`. Add a row if a fixture can force hyper's `NotStarted`, for example by closing the connection between the drain and the send under test control.
2. **Earlier recorded follow-ups** remain as recorded in RemPlan-2's "Not in this round":
   - D23's value refresh;
   - a hold-independent ceiling;
   - `https_failure.rs` at 992 lines;
   - the exit gate window.

## 4. Risks and next action

No P2 remains. The open P3s are:
- **P3-8**, a narrow misleading refusal;
- **P3-9**, an amendment wording fix;
- **P3-1, P3-2 and P3-5**, which need the operator's decisions before S7.1.

**Next action.** Fix the P3-9 text, and P3-8 if it is cheap, in the normal course. Take P3-1, P3-2 and P3-5 to the operator before the S7.1 switch removal.

On the State axis this closes D19 at round 2 of the two-round cap: GO.
