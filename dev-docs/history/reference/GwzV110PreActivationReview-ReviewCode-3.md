# 1.1.0 pre-activation review (D19): code-axis re-verdict (round 2 remediation)

**Review object:** the five D19 objects at the round-2 tuple. That is the round-1 tuple plus gwz-core `ecf1cdf7` (the remediation), `6560a132` (the dial_retry test rows, not remediation) and `18c520b8` (documents), and gwz-cli `8d909ec` (the troubleshooting entry and its pin).

**Baseline** (frozen workspace `/Volumes/projects/limbo/gwz-dev-review19c`):

| Repo | Commit |
|---|---|
| root | `90dff3787dd4be881a06e5ff8037af035cbe2b77` |
| gwz-core | `aee4c257d24ffa65ea77f1266363dca619f698b4` |
| gwz-transport | `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (unchanged) |
| gwz-cli | `8d909ec4886118789c06295ccd13a87bb496432a` |
| gwz-py | `da098f5c52e822a3f7ed0920b016e9b0c5c5a10a` (unchanged) |
| gwz-sspi | `d3038882f7937be023d1835fe1b03e0309260acb` (unchanged) |
| gwz-core-evidence | `ce3b5d931833636bac12d6681e6bead5498d6481` (reference only) |

**Date:** 2026-10-16. **Axis:** Code (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only.

**Verdict: GO.**
- Prior findings: P3-3, P3-4 and P3-5 are closed and verified. P3-1 is deferred, and the condition I set on that deferral in round 1 is now met.
- New findings: 0 P0, 0 P1, 0 P2, 1 P3 (P3-6, documentation). No architectural root cause, so the cap's stop rule is not triggered.
- Object 1 (TR2.6): **GO**.
- Object 2 (S3.1): **GO**.
- Object 3 (S3.2): **GO**.
- Object 4 (WPP 4.8 / TR4.10, SSPI): **GO**, with one P3 open.
- Object 5 (WH2): **GO**.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P3-1 (an SSPI `ProviderRejected` is worded as a replaced worker; the status is dropped) | Deferred. The SSPI status goes on the 1.2.0 wire list beside D16. The "Reinstall gwz" wording is a known 1.1.0 limitation for S7.2's notes. | My round-1 condition was that the deferral be recorded in a controlling document. It now is: `GwzV110PreActivationReview-RemPlan.md`, "Outcome of round 1" (lines 42–44). gwz-cli's new Troubleshooting entry also states the "Windows refused to start the sign-in" cause beside "Reinstall gwz". The code is unchanged, as expected. | **Deferred (accepted; condition met)** |
| P3-3 (once throttled attempts had spent the eight rounds, a requeued retry failed as `Protocol`) | `settle` does not requeue an open whose carried rounds are spent. It finishes through `spent` with the throttle or capacity outcome. | **Yes.**<br>• My counterexample was `--max-retries 7`, with the AUTHENTICATE answered 429 every time.<br>• Re-traced: after attempt 4 the rounds reach 8, and `retry.rs:197` (`entry.attempts >= allowed \|\| carry.budget.native_rounds_spent()`) ends the open through `spent`. The result is `Capacity` with the server-capacity detail, not `Protocol`.<br>• Closure row: `native_round_tests::d19_code_p3_3_*`. It runs exactly my counterexample. It asserts 8 steps or fewer, that every step was sent, that the code is not `Protocol`, that the code equals `Gh`'s (`Capacity`), and that the words equal `Gh`'s once the attempt count and port are stripped. Ran: pass. | **Closed** |
| P3-4 (the Basic re-lease's connect was not the open's setup: no retry, no judgement, origin lost) | The re-lease carries `throttle::Setup` (member tag, `setup_cause`, `FirstConnect`, `judge_setup`), now shared with `prepare`'s first-hop checkout. | **Yes.**<br>• `lease_again` (`exchange.rs` lines 465–520 at the tuple) takes `setup.tag(...)`. On failure it calls `setup.failed(...)`, which keeps `setup_cause`, sets `FirstConnect::Failed` for a `Phase::Setup` failure and calls `judge_setup`. On success it calls `setup.connected(...)`.<br>• `prepare`'s own checkout now uses the same `Setup` (`prepare.rs` around line 234). That is a pure extraction with identical semantics, including the machine-proxy 407 rule.<br>• `classify(Unavailable, ConnectionRefused, Setup)` gives `Retry`, so the open retries as the `gh` path's does.<br>• Closure row: `reconnect.rs::d19_code_p3_4_*` (the server stops listening after the challenge; a 6 s helper). It asserts `Unavailable` with `setup_cause = ConnectionRefused`, `FirstConnect::Failed` and a limit-machine refusal. Ran: pass. The settle-level retry follows from `classify`, by trace. | **Closed** |
| P3-5 (the SSPI 403 sentence pointed to a section with nothing for it) | Add gwz-cli Troubleshooting "Windows Sign-In", point the sentence to it, and qualify the `git ls-remote` advice for NTLM-only servers. | **Yes.**<br>• `https_failure.rs:234` now ends "See Troubleshooting: Windows Sign-In." and includes the NTLM-only caveat.<br>• `Troubleshooting.md` has `## Windows Sign-In`, covering the 403 (including "it can come at any step"), a rejected logon, both worker texts, the redirect text and the escape hatch. "HTTPS Credential Failure" points to it.<br>• Pins: gwz-cli `tests/troubleshooting_windows_sign_in_pins.rs` (five rows) and gwz-core `closure.rs:144` (`ends_with("See Troubleshooting: Windows Sign-In.")`). The gwz-core row ran: pass. I did not run the gwz-cli pins; they are simple text pins, and I read them.<br>• Surface: the plan routes the sentence to S7.5. | **Closed** |

## Changed-range analysis

**What changed** (`git diff 8e425e76..aee4c257`, gwz-core `src/`):
- `exchange.rs`: from 552 to 657 lines;
- `prepare.rs`: `Setup` extracted, plus `closes()`;
- `throttle.rs`: `Setup`;
- `budget.rs`: `native_rounds_spent`;
- `retry.rs`: the spent-rounds guard;
- `https_failure.rs`: one sentence changed (992 lines, still under the split threshold);
- test rows: `reconnect.rs`, `closure.rs` and `native_round_tests.rs`;
- the dial_retry rows (`6560a132`, test-only);
- design amendments (`18c520b8`, and root `8f6e5b09`);
- gwz-cli: docs and a pin.

The areas I attacked:

**The connection-usability decision in `answer_basic`.**
- The lookup is raced against `sleep(hold)` (biased toward the lookup).
- Once a credential is in hand, it goes on the challenge's connection only if:
  - the challenge is still held;
  - `challenge_usable` (which drains, and checks for `Connection: close` or a dead connection) is true;
  - and `send_once` actually starts the request.
- Otherwise `lease_again` takes a fresh connection scoped to the credential.
- `send_once` returns `None` for `SendFailure::NotStarted` and restores `credential_offered`, so no credential is reported as offered on a request that never left.
- A request that fails after it started is `Io`, the same as on the `gh` path.
- The fresh path then calls `send_authorized`, which maps `None` to `Io`. That is correct: a fresh connection that cannot start its request is a network failure.
- I found no path that reports `Authentication` for a connection that could not carry the Basic request.

**The P3-7 lease release.**
- When the hold lapses, the challenge response is dropped and the lease is finished `Discarded`, while the lookup goes on.
- `lease_again` then finds `prepared.lease == None` and skips its own finish.
- If the lookup fails afterwards, the `Prepared` drops with `lease = None`, which `Drop` handles.
- If `finish` itself fails, the pinned lookup is dropped. The helper job's owner reaps it, exactly as a cancel would.
- No double finish, and no use of a taken lease.

**The SSPI fresh connection and its channel binding.**
- A challenge with `Connection: close` (`prepare::closes`) leads to `lease_again(None, setup)` before the binding is read.
- `connection.lock().await.binding` is then read from the new lease. The token therefore carries the new connection's own `tls-server-end-point` binding, as design 8 requires.
- That happens before `Guard` and `Starting` exist, so no slot or cleanup state is affected.
- The new lease is unscoped until the exchange's scope is applied after Start, as before.
- A close in the middle of an exchange still ends it as `Authentication`, on purpose: the exchange belongs to its connection (row `d19_state_p2_3_a_close_in_the_middle_*`).

**`throttle::Setup` and its shared use by `prepare`.**
- `failed`, `tag` and `connected` move code verbatim from `prepare`. Tagging only on the first hop (`hops == 0`) and the 407 renaming are preserved.
- In `authenticate`, `Setup.first = hops == 0`, so a re-lease after a redirect is not tagged, which matches the `gh` path.
- On the first hop, a re-lease tags the same member a second time, and a failure overwrites the earlier `FirstConnect::Connected`. The `gh` path's re-checkout after a lapsed challenge does exactly the same, so the behaviour is parity, not a new limit-machine semantic.

**The spent-rounds settle path.**
- The guard is only in the requeue branch, which is where a post-exchange failure can be requeued.
- `spent()` with attempts below `allowed` produces the throttle's `Capacity` words with "attempt k of n". The user therefore sees, for example, "4 of 8": the operation's eight rounds (design 8) ended the open early. This is accepted under the amended design text below. A future wording could say so.

**The design amendments** (parity design §7 and §9; integration design §4's D clause).
- They state the requeue, the carried rounds, the per-attempt D, the same identity and origin, and the "not requeued when spent" rule. That matches the code (`retry.rs:197`, `native_round_tests::d19_state_p3_6_*`).
- The integration design's D amendment (D moved on by the helper lookup) matches `exchange.rs`'s `*until += lookup`, and is phrased as an explicit exception to "never reset/extend D".
- **The triggers are inexact** (P3-6).

**The lane's three reported gaps, judged on my axis:**
- **(a) `Outcome::Retry` carrying spent rounds.** Not reachable on the current call graph. `Outcome::Retry` needs `Phase::Setup`, which is a first-hop connect failure. In an SSPI attempt the only connects are the anonymous request's and the pre-first-token re-lease, and both come before any round of that attempt. Carried rounds can only reach 8 through a requeue, and a requeue is now refused when the rounds are spent, or through an exchange that hits the cap, which ends `Protocol` and is not retried. So the next attempt can never start with spent rounds through this branch. **Accepted.** A defensive `native_rounds_spent()` check in that branch would cost one line, and is a follow-up.
- **(b) SSPI with a dead connection before the first token, and no `Connection: close` header.** `auth_send` then gets `challenge_usable == false` and returns `Authentication`. The generic text is "HTTPS endpoint request failed: Authentication", and it is not retried. Unlike the Basic path, the first token could safely go on a fresh connection. Reachability is narrow:
  - a server must close the connection between its 401 and the first token, within the SSPI Start time, and without the header;
  - or an HTTP/1.0 non-keep-alive server, over which NTLM cannot run anyway.
  
  **Accepted for 1.1.0 as a recorded limitation (P3-level, not filed).** A cheap fix: before the first token, treat `!challenge_usable` like `closes()` and call `lease_again`.
- **(c) No deterministic row for "the authorized send was not started".** The branch is three lines (`send_once` gives `None`, then `lease_again`), and its `credential_offered` restoration is read-checked. **Accepted.** A fake-sender unit row would be a follow-up.

**Not remediation** (`6560a132`, the dial_retry test assertions): test-only, outside the five objects. No regression.

### P3-6 (object 4): the State P3-6 amendments name the requeue triggers too narrowly

- **Root cause.** All three amendment texts say that the case in which a new exchange starts is "a throttle answering an exchange (429, or 503 with `Retry-After`)":
  - parity design §7 ("This is the only other case in which a new exchange starts");
  - parity design §9;
  - integration design §4.

  The code requeues more than that. `retry.rs`'s requeue branch requeues whenever `setup_retry::requeues(ruling, throttled)` holds:
  - every throttle, which `throttle::said` defines as a 429, or a 503 **or 403** with `Retry-After`;
  - **any** refusal that the limit machine takes as evidence (`ruling != RetryMachine`), which includes a bare 503 (`Said::Unanswered`).

  Each of these starts a new exchange after the requeue. In addition, "a new exchange on a new connection" (§7) overstates: the retry's anonymous request may be served on a pooled, unscoped connection. The throttled one is discarded, but the next is not necessarily newly dialled.
- **Location:**
  - gwz-core `dev-docs/GwzTransportWindowsParityDesign.md` line 201 (§7 amendment) and line 239 (§9);
  - root `dev-docs/GwzWindowsHttpsIntegrationDesign-DRAFT.md`, lines 162–166;
  - the code they describe: gwz-core `src/transport_host/https_endpoint/retry.rs:186-215`, and `src/git/endpoint/https_worker/throttle.rs:97-106`.
- **Violated invariant.** The design must state the behaviour exactly. §7's "only other case" is false for a 403 with `Retry-After` after a token, and for a bare 503 the machine takes as evidence.
- **Reproduction.** An NTLM server answers the AUTHENTICATE token with `403` and `Retry-After: 2`. `said` gives `Throttle`, the open is requeued, and the next attempt starts a new exchange, which is not a case the amendment allows.
- **Impact.** Documentation only. The behaviour is the adaptive design's and is consistent, but a reviewer or maintainer reading §7 would treat the 403 and bare-503 requeues as violations.
- **Correction.** In the three places, word the trigger as the adaptive design's requeue rule: "a refusal the endpoint requeues under the adaptive design §5.1 (a throttle: a 429, or a 503 or 403 with `Retry-After`; or a refusal the limit machine takes as evidence, such as a bare 503)". Say "another connection" instead of "a new connection".
- **Closure test.** None needed for the text. Optionally, a `native_round_tests` row with a 403 plus `Retry-After` answering the token, asserting a requeue with the rounds carried, to pin the behaviour the corrected text describes.

## Problem, requirement provenance, and scope

Each change in `ecf1cdf7` traces to a row of RemPlan-2: State P2-3, State P3-7, Code P3-4, Code P3-3, Code P3-5, and State P3-6's D row. The `Setup` extraction is the plan's own option for P3-4 ("as `prepare`'s first-hop checkout does"). The SSPI `Connection: close` handling is P2-3's "SSPI … takes a fresh connection the same way". The design amendments are authorized by the P3-6 disposition, which says "amend the design". `6560a132` and `18c520b8` are outside remediation, and are test-only and documents only. Nothing expands scope. No P2 is deferred, and the P3 deferrals are recorded (P3-1 in RemPlan "Outcome of round 1"; State's P3-1, P3-2 and P3-5 are with the operator). No new architectural root cause.

## 0. Evidence base

**Tuple.** Verified at the start (06:24) and at the end (06:29 AEDT): identical. The untracked files are unchanged and pre-existing (root 5, gwz-core 1, evidence 1).

**Read fully:**
- RemPlan-2, and RemPlan's "Outcome of round 1";
- the `ecf1cdf7` diff of `exchange.rs`, `prepare.rs`, `throttle.rs`, `budget.rs`, `native.rs`, `retry.rs` and `https_failure.rs`;
- the new `native_round_tests` rows, and `reconnect.rs` lines 1–40 and 120–201;
- the parity design diff (§7, §9), and the integration design lines 140–185;
- gwz-cli `8d909ec`: the docs diff, and the pin file's headings and assertions.

**Sampled:**
- `retry_tests.rs` (`endpoint_with`);
- `setup_retry/requeue.rs` (`requeues`, `spent`);
- `throttle::said`;
- `scoped.rs::tag`.

**Not read:**
- `https_fixture.rs`'s new fixture code (`Slow`, `start_closing_idle`, `stop_after`) in detail;
- `6560a132`;
- `runs/2026-10-16-r19auth2`.

**Commands:**
- `prepare.py` produced `/Volumes/projects/limbo/build-targets/review19r3-code-cand`.
- Candidate library build (`--no-run`, `CARGO_TARGET_DIR=…/review19r3-code-target`): finished.
- Tests `-- https_worker native_round_tests https_endpoint backstop https_failure`: **247 passed, 0 failed**. Every `d19_*` row passed: the five `reconnect` rows, the six `closure` rows, and the three `native_round_tests` rows.
- Source checks:
  - Windows parity: 430 files, 196 occurrences, nothing new;
  - cfg boundaries: 1,637 files, nothing new;
  - process globals: 18 allowlisted, nothing new;
  - candidate switches: 23 sites, matching.

## 2. Invariant analysis (attacks that failed)

- **No credential reported offered without a send.** `send_once` restores `credential_offered` on `NotStarted`.
- **No `Authentication` for a Basic credential that a connection could not carry.** Both the usable and unusable branches end in a started send or a fresh connection.
- **The SSPI binding is always the carrying connection's own.** It is read after any re-lease.
- **The SSPI slot and cleanup accounting are unchanged.** The re-lease precedes `Guard`.
- **The challenge lease is finished exactly once.** The hold-lapse branch and `lease_again` both check `prepared.lease.take()`.
- **`Setup` extraction parity.** Same tag rule, same 407 renaming, same `FirstConnect` and `judge_setup`.
- **The spent guard is only on the requeue branch, and the `Outcome::Retry` branch cannot carry spent rounds** (gap (a), traced above).
- **The re-anchored D per attempt is pinned** (`d19_state_p3_6_*`). The D move during a helper lookup is an explicit, documented exception.
- **The troubleshooting entry covers every message the module emits for `AuthMethod::Sspi`:** the 403, the rejected logon, both worker texts and the redirect.

## 3. Follow-ups outside the finding bar

1. **Gap (b).** Before the first SSPI token, treat `!challenge_usable` like `closes()` and call `lease_again` (a few lines). Record it as a known limitation otherwise.
2. **Gap (a).** Add a one-line defensive `native_rounds_spent()` check to `settle`'s `Outcome::Retry` branch.
3. **Gap (c).** A fake-sender unit row for `send_once`'s `NotStarted` path.
4. **Spent-rounds wording.** It reads "attempt k of n" with k below n, without saying that the eight native rounds ended the open. Consider one clause for that, through Surface.
5. **The term "per operation".** Design 8 uses it, but it is implemented per Open, across that Open's attempts. Each member's discovery Open has its own eight. A clarifying sentence would avoid misreading it as the gwz request.
6. **Carried from earlier rounds** (as RemPlan-2 records): D23's value under path-only rebuilds; a hold-independent ceiling for the backstop; `https_failure.rs` at 992 lines; the exit gate's attach window.

## 4. Risks and next action

**Risk.** Low on this axis. Every prior finding of mine is closed or deferred with a record. The round-2 code is a parity-preserving extraction plus narrowly scoped connection handling, each with a closure row, and all pass. The one new P3 is design-text exactness.

**Next action:**
1. Correct the three amendment sentences as P3-6 specifies (documents only).
2. Take follow-ups 1 and 2 at the lane owner's discretion.
3. Proceed to S7.1 on this axis's GO.
