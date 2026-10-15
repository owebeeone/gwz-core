# 1.1.0 pre-activation review (D19): STATE-axis re-verdict (round 1 remediation)

**Review object:** The round-1 remediation of the five D19 objects, reviewed in the frozen workspace `/Volumes/projects/limbo/gwz-dev-review19b`:
1. TR2.6, the Phase 2 transport tree.
2. S3.1, the timeout plan's S5.1 review.
3. S3.2, the Q6 aggregate.
4. WPP 4.8 / TR4.10, SSPI and gwz-sspi.
5. WPP 4.2/4.3, the WH2 helper owner.

The inputs are `GwzV110PreActivationReview-RemPlan.md`, decision D23, and these remediation commits:
- gwz-core `4518f665`, `93a08dbc`, `6778cbfa`
- gwz-cli `b7c9edc`
- gwz-py `da098f5`

**Baseline:**
- root `484a5d88e3ae9e6b303e6926700802a4c73f174a`
- gwz-core `8e425e7634656d0189d11b27f4708847985474df`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (unchanged)
- gwz-cli `b7c9edcd8d9da1adb2f04eb4ea6343b0762cc55a`
- gwz-py `da098f5c52e822a3f7ed0920b016e9b0c5c5a10a`
- gwz-sspi `d3038882f7937be023d1835fe1b03e0309260acb` (unchanged)
- gwz-core-evidence `9d33418978526cf01fa1d7dd5125fd439a5af62b` (reference only)

**Date:** 2026-10-16. **Axis:** STATE. Independent, adversarial, read-only.

**Verdict: NO-GO.** P0 0, P1 0, P2 1 open (new: P2-3), P3 5 open.
- Two of the open P3s are new: P3-6 and P3-7.
- Three were not remediated and sit with the operator: P3-1, P3-2 and P3-5.

All three round-1 P2s I can trace are closed and verified. That is State P2-1 and P2-2, plus the eight-round retry follow-up that the plan pairs with Code P3-2.

| Object | Verdict | Open findings |
|---|---|---|
| 1 (TR2.6 tree) | GO | P3-5 |
| 2 (S3.1 clocks) | GO | none |
| 3 (Q6 retirement fix) | GO | none |
| 4 (SSPI, gwz-sspi) | NO-GO | P2-3; P3-1, P3-6, P3-7 |
| 5 (WH2 owner) | GO | P3-2 |

I pre-commit to GO on a revision that resolves P2-3 as specified.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1 (obj 1): the backstop fires on held opens and seals the whole request | `93a08dbc`: the backstop ends only its own stream (its own Cancel, reply, `stream.cancel()`, peer disconnect; no `owner.cancel(request)`). It is charged only while the endpoint does not hold the open: the endpoint session publishes its held opens in the `Handoff` and `Backstop::look` pauses over them. | **Yes.** I re-traced my counterexample. `pump.rs` no longer calls `owner.cancel(request)`. `Backstop::look` (`driver/backstop.rs`) charges an interval only when the open was unheld at both looks, and never ends a held open. `held_opens` covers what the counterexample needs: HTTPS `entry.held` covers a room wait, a site hold and a key retry wait (`https_endpoint.rs`); SSH covers `queued_opens` (`placement_endpoint.rs`). Closure rows `backstop_tests.rs:120,183,253`: the stream mid-transfer completes, `sealed_for_test` is false, and the held open later starts and succeeds. Run: pass. | Closed |
| P2-2 (obj 4): the Windows Basic helper is charged to D | `4518f665`: no `timeout_at(D)`. The lookup runs on the helper's allocation and interaction allowance, D is moved on by the lookup's duration, and an expiry reports M4. | **Yes.** `exchange.rs:345-360`. Closure rows `native/tests/closure.rs:220,244`: a helper outlasting D but inside its allowance authenticates; past its allowance it fails `Timeout` with `Interaction` and `helper_budget_ms = 3000`; `starts == 0`. This is my 40 s against 30 s counterexample, scaled to 3 s against 1.5 s. Run: pass. | Closed |
| P3-1 (obj 4): gwz-sspi pipes created inheritable | Not remediated; with the operator (a gwz-sspi 0.1.1 and new pins, or defer to 1.2.0 with the risk recorded). | Unchanged at `d3038882` (`launch.rs:32-48`). The disposition is acceptable for a P3, which does not block. It must be decided before S7.1. | Open (operator) |
| P3-2 (obj 5): WH2 spawn gate owed; C9 misses libgit2's helper spawn | Not remediated; with the operator. | Unchanged. Acceptable as a P3, with the same condition: decide before S7.1. | Open (operator) |
| P3-3 (obj 5): the configuration view copied into unwiped allocations | `6778cbfa`: every environment value sits in a fixed-size wiping owner (`wiped.rs`, `EnvValue`). The view goes straight from borrowed bytes into an owner (`runner/environment.rs:165,211`). The Windows block is one wiping owner built from those owners, dropped right after `CreateProcessW`. `HelperCommand` is dropped as soon as the spawn returns (`runner.rs:107-110`). std's Unix copies are added to OQ-C's list. | **Yes.** I re-traced each copy I named in round 1; none remains on Windows. The audit-probe rows are in `runner/environment.rs` tests (run here) and in `process_tree/windows/tests.rs` (Windows only; receipt `r19pyfix-20261016-b`). | Closed |
| P3-4 (obj 1): gwz-py notes attach after the host closed at exit | gwz-py `da098f5`: `ExitGate` over `Operations.exiting`; `note_sink` drops notes and detail lines once it is set. | **Yes.** `route/transport.rs` `note_sink`. Rows `transport_tests.rs:218,236,250` cover before the exit close, after an ordinary close, and from another thread. Lock order checked: `Operations::cancel` and `close` call cancellers after dropping their lock (`operations.rs:225-227, 281-284`), so the gate's lock taken under the session lock cannot form a cycle. A check-then-attach window remains, of decision 14's class. | Closed |
| P3-5 (obj 1, ARCHITECTURAL): exit joins async `call()` executor threads before the hook | Not remediated; with the operator. | Unchanged (`bridge.py:298-315`). Acceptable as a P3 with an operator decision or an S7.2 note. | Open (operator) |
| Follow-up 1 (obj 4): the eight-round retry claim was proven only by a hand-shared `Budget` | `4518f665`: `Held::after_attempt` carries `Budget::native_rounds()`, and `start_attempt` calls `resume_native_rounds`. | **Yes.** `retry.rs:61-78, 224-228, 262-266, 527, 589`. The endpoint-level row `native_round_tests.rs` makes four rounds, then a 429, then a requeued retry: 8 steps in total and a final `Protocol`. Run: pass. | Closed (see P3-6) |
| Follow-ups 2-7 (WPP 4.2 permit text, password-helper cleanup loop, gwz-sspi ISC aliasing, per-request SSH count, gwz-py late success, the not-in-job comment) | Recorded, not in this round. | Not re-examined beyond confirming they are untouched. | Recorded |

## Changed-range analysis

**What changed.**
- The three gwz-core remediation commits, in the auth, backstop and secret-hygiene lanes.
- D23 self-provision in gwz-cli.
- gwz-py's exit gate.
- Non-remediation commits, checked only for regressions in the five objects:
  - `8e54e500`: transport dependencies moved into `Cargo.toml` behind the switch. The pins are identical and the lock adds no version. No object regression.
  - `665f42ae`, `7ff37f3f`: test fixtures and rows.
  - documents, and the gwz-cli and gwz-py lock and CI commits.

**The auth lane's re-lease path** (`lease_again`, `send_authorized`, `CHALLENGE_HOLD`):
- When the lookup reaches the hold, `lease_again` takes a fresh scoped connection. It:
  - discards the lapsed lease;
  - charges allocation and connect as `prepare` does, under D;
  - scopes once, and updates `connection_id` and `reused`.

  Its failures return with facts and are fail-closed (`Phase::Other`, no retry). It bypasses the throttle `Gate` and the setup tag, so the limit machine does not judge this second connect. That is acceptable, because the pool's per-host limits still apply.
- **Two defects remain.**
  - The choice between reusing the challenge connection and taking a fresh one is made on lookup time alone, not on whether that connection can carry the credential: **P2-3**.
  - The challenge lease is held through the whole lookup, not for the hold: **P3-7**.

**The backstop's holds record and its pause/resume accounting.**
- Holds are published only on change (`publish_holds`). A closed session publishes none (`close.rs`).
- Holds are keyed by carrier session and stream, and read only while an unopened stream has a backstop.
- An interval is charged only when the open was unheld at both looks. Passes run at least every 5 ms (`PARK`), so a hold boundary leaves a few milliseconds uncharged.
- A spent backstop whose Cancel finds the mux full retries on the next pass. Any other mux error still completes the open.
- A race with a late Opened is safe: the stream is cancelled and the endpoint receives the open's Cancel.
- **Residual, accepted:**
  - a hold is now bounded by no driver clock, so an endpoint accounting fault that holds an open forever would hang that member rather than time it out;
  - a Cli-placed endpoint reports no holds, so fix (b) does not apply there. Cli placement is outside 1.1.0's `local`-only scope, so the 1.2.0 record is acceptable.

**The native-round carry across a retry.** The count is carried and capped (verified above). Because a 429 after an exchange is now a requeue, a retry starts a **new SSPI exchange** with a fresh logical deadline D. Both conflict with the Windows parity design's text: **P3-6**.

**The new SSPI 403 wording** (`request/https_failure.rs:229-236`). It fires on `method == Sspi && credential_offered`, which covers both:
- an exchange's non-401 answer handed to classification;
- a POST on an authenticated route.

It does not mark the credential rejected and leaves `authenticated` unset, so it is fail-closed. "after gwz sent your Windows sign-in" is literally true even when the 403 answers an unfinished exchange (an NTLM type-1 carries no user). That is not a finding. The SSPI exchange's slot leaves `prepared` with the `Guard` on this path, so it is released or retained by the cleanup record, never leaked. I re-traced this.

**D23's per-build fingerprint** (`gwz-cli/build_support/self_provision.rs`, `build.rs:23-29`).
- **The guarantee holds.** The worker is the process's own image (`current_exe`), and the host and the worker read one compiled value (`worker_host.rs` `COMPILED_BUILD`). A foreign image is still refused before Begin (`worker_host.rs` tests). The value is an identity, not a secret.
- **Weakening, recorded as a follow-up.** The value is refreshed only when the build script reruns. `rerun-if-changed` lists gwz-cli's own paths, so a rebuild that changes only a path dependency (gwz-core or gwz-sspi) keeps the old value. Two such builds accept each other's worker. This matters only if the image at `current_exe` is replaced while a gwz is running. Packaged builds are unchanged.

**Common check (problem, necessity and authorized scope).**
- Every remediation change traces to a round-1 finding and its RemPlan disposition, or to D23.
- The new exchange-after-throttle behaviour traces to Code P2-1's disposition (shared classification, as `Gh` has). It contradicts the accepted parity design's wording, and no amendment records it: see P3-6.
- No credential or trust scope widened. The re-leased connection is scoped to the same credential, rescoping is still refused, and the new exchange is the same logon identity to the same origin within the shared eight rounds.

## 0. Evidence base

**Tuple.** I checked it at the start and at the end. It was unchanged. The only untracked files were ones that existed before I started:
- 5 root `dev-docs/` files;
- gwz-core `dev-docs/GwzRemoteTransportBugReport.md`;
- one evidence directory.

**Read fully:**
- the RemPlan; D23's entry in `GwzTransportTR18-OperatorDecisions.md`;
- `git show` of `93a08dbc` (all source, including `driver/backstop.rs` and the pump diff), `4518f665` (all source), `6778cbfa` (all source), gwz-cli `b7c9edc` (`build.rs`, `self_provision.rs`, `worker_host.rs`, `tests/self_provision.rs`) and gwz-py `da098f5`;
- `native/tests/closure.rs:160-309`, `https_endpoint/native_round_tests.rs`, `backstop_tests.rs:118-182`;
- gwz-py `operations.rs:199-300`;
- `exchange.rs:340-500` at the tuple;
- the r19auth evidence README.

**Sampled:**
- `8e54e500` (`Cargo.toml`);
- `prepare.rs:600-700`;
- the gwz-py `transport_tests.rs` exit rows.

**Not read:**
- the other evidence READMEs beyond their names;
- the non-remediation fixture commits' test bodies.

**Commands:**
- `prepare.py` into `/Volumes/projects/limbo/build-targets/review19r2-state-cand`. Then the candidate `cargo +1.95.0 test --lib` (target `review19r2-state-target`, `GWZ_TEST_GIT=real`) with filters `backstop native_round closure https_worker::native transport_host::session::cleanup_tests https_endpoint::retry_tests https_endpoint::requeue_tests https_auth`: **182 passed, 0 failed.**
- The checkers:
  - Windows parity: 429 files, 196 occurrences, nothing new;
  - cfg boundaries: 1,635 files, nothing new;
  - process globals: 18 allowlisted, nothing new;
  - candidate switches: 23 sites match;
  - checked-artifact boundaries: ok.
- Windows-only rows were judged from receipts `r19auth-20261016-d`, `r19backstop-20261016-a`, `r19pyfix-20261016-b` and `r19provision-20261016-a`, and from runs `2026-10-16-r19{auth,backstop,pyfix,provision}`.

## Problem, requirement provenance, and scope

As in round 1, plus:
- the RemPlan's dispositions;
- D23;
- for P2-3, the commit `4518f665`'s own stated goal: "a server that closes idle connections does not fail the authenticated request".

The authority for P2-3 and P3-7 is the parity requirement against the `Gh` path. That path holds a challenge's connection for `CHALLENGE_HOLD` and replaces an unusable one (`prepare.rs:612-681`, `https_worker.rs:233-246`, `prepare.rs:427-447`).

## 1. Findings

### P2-3 (object 4). On Windows a Basic credential is sent on the challenge's connection whether or not that connection can still carry it, and an unusable one fails the open as `Authentication`

- **Root cause.** `answer_basic` chooses between reusing the challenge's connection and leasing a fresh one by lookup time alone.
- **Location (gwz-core):**
  - `src/git/endpoint/https_worker/native/exchange.rs:369-378`: `lookup < hold` → `auth_send`;
  - `auth_send` at `:456-494`: `if !clean { return Err(Authentication) }`;
  - `prepare::clean_challenge` (`prepare.rs:703-754`) returns `Ok(false)` when the 401 says `Connection: close` or the connection has died;
  - a send on a closed connection in `send_authorized` maps to `Io`.

  Present at the round-1 tuple. I missed it in round 1; the remediation touched this function and did not cover the case.
- **Violated invariant.** A credential the server would accept is not reported as an authentication failure. Windows Basic matches the `Gh` path: when the challenge connection cannot carry the credential, the `Gh` path discards it and takes a fresh, scoped one (`prepare.rs:651-681`, and the `NotStarted` retry at `:427-447`). The remediation states the same goal ("a server that closes idle connections does not fail the authenticated request").
- **Reproduction.** Use `slow_helper` from `closure.rs` with `idle = Some(1 s)` and a 2 s helper, under `WindowsConfigured`.
  1. The lookup takes 2 s, which is under the 5 s hold, so the credential goes to `auth_send`.
  2. The server closed the connection at 1 s, so `clean_challenge` returns `false`.
  3. The open fails `Authentication` with a correct credential.

  The same happens at any lookup time when the 401 carries `Connection: close`, for example Apache with `KeepAlive Off` or some authenticating proxies. The same remote under `Gh` succeeds. Existing row `:296` uses `idle = 6 s` with a 1 s helper, so it never meets the case.
- **Impact.** On the default Windows credential path, some servers refuse a valid helper credential with a message that reads as a credential problem. The same initial `auth_send` gate also applies to a `Negotiate` 401 marked `Connection: close` (a Kerberos-only server).
- **Required correction.** When `clean_challenge` reports the challenge connection unusable, or the authorized send was not started, take the `lease_again` path, as the `Gh` path takes a fresh connection: drop the challenge, check out a fresh connection scoped to the credential, and send once. Never report `Authentication` for a connection that could not carry the request. For SSPI, either do the same before the first token, or state the refusal.
- **Closure test.** Two `closure.rs` rows under `WindowsConfigured`:
  - `idle = 1 s` with a 2 s helper;
  - a 401 carrying `Connection: close` with a 0 s helper.

  Both authenticate, `accepted == 2` (a fresh connection), `credential_offered` is true and `starts == 0`. Red at this tuple.

### P3-6 (object 4). A throttle after an SSPI exchange now starts a new exchange with a fresh D, contrary to the parity design's text, and no amendment records it

- **Root cause.** Code P2-1's disposition made a 429 after an exchange a requeue. The design text that forbids a new exchange except on network loss was not amended.
- **Location:**
  - gwz-core `native/exchange.rs:278-283` (`is_classified_elsewhere` returns the 429 to `prepare`, where `tell_governor` makes it a requeue);
  - `https_endpoint/retry.rs:224-228, 589` (rounds carried, D not);
  - the r19auth README: "D is not carried … each attempt anchors its own D".

  Against `GwzTransportWindowsParityDesign.md:201`: "No new exchange starts within the operation except TR2.1's retry on network loss before effect (section 9)". The Windows integration design also says redirects, 401s and native rounds never reset D.
- **Reproduction.** `native_round_tests.rs` itself: four rounds, then a 429, then a second SSPI exchange (a new worker and a re-sent identity) on a new connection, under a new D.
- **Impact.** The behaviour is bounded (eight rounds per Open; same identity; same origin; retries capped by `--max-retries`), but it is undocumented. It also replays the logon identity after a throttle, which register item 3's reasoning about replay was written to exclude.
- **Correction.** Amend §201 and §9, and the WH integration design's D clause, to admit a throttle requeue (its rounds carried, its D per attempt). Alternatively, do not requeue after an exchange.
- **Test.** The existing row stays; add a row asserting the second attempt's D is its own (a stepped clock) if the design is amended so.

### P3-7 (object 4). The Windows Basic path holds the challenge's connection through the whole helper lookup, not for `CHALLENGE_HOLD`

- **Root cause.** `prepared.lease` stays held across `self.credential(...).await`, and is given back only afterwards (`lease_again`).
- **Location.** gwz-core `native/exchange.rs:352-378`. The comment at `:352` says the connection is "kept for the `gh` path's hold … and no longer". The `Gh` path's `ChallengeLease` is dropped when it expires (`https_endpoint/poll.rs:32-42`, `retry.rs` `Held::lapse`).
- **Impact.** For an interactive helper (up to 120 s), one leased, idle connection per waiting member counts against its site's room. Other members to that host may wait for room meanwhile. The comment and the r19auth README describe a hold the code does not implement.
- **Correction.** Release the challenge lease when the hold lapses during the lookup, for example by selecting the lookup against `sleep(hold)` and then finishing the lease as `Discarded`. Or correct the text.
- **Test.** A helper of 10 s with a fixture that counts open connections: from 5 s onward the challenge connection is closed or back in the pool.

(P3-1, P3-2 and P3-5 remain open with the operator, as listed in the closure table.)

## 2. Invariant analysis (attacks that failed)

- **The backstop's pause and resume:**
  - a hold never spends budget (`Backstop::look` unit rows);
  - a request is never sealed by a per-open clock (`pump.rs`, no `owner.cancel`);
  - mux backpressure on the Cancel does not lose the expiry;
  - holds are cleared on close.
- **The native-round carry:** carried on requeue and on `Outcome::Retry`, preserved across `lapse` and re-holds, and fresh only for a first attempt (`Held::new` is 0).
- **The SSPI non-401 path:** the endpoint slot moves with the `Guard` and is released or retained by the cleanup record; the session is cancelled and its probe retained. No leak.
- **`lease_again`:** no rescoping of a scoped connection; the old lease is discarded before checkout; failures are fail-closed; D governs the checkout.
- **D extension:** D is extended by exactly the lookup's duration, and only on the Basic path. SSPI's D is untouched.
- **P3-3 owners:** no growth (fixed size), wiped on drop, `Debug` redacted, the block dropped right after spawn. A NUL in a value is refused.
- **The P3-4 gate:** no ABBA ordering with session locks. The gate only reads.
- **D23:** a foreign image is refused; an absent value refuses; no PATH or runtime fallback is reintroduced.
- **S3.1 clocks and the Q6 retirement:** untouched by the range (the pump change is limited to the backstop block). `cleanup_tests` pass.

## 3. Follow-ups outside the finding bar

1. **D23's value is not refreshed** when only a path dependency changes (`rerun-if-changed` lists gwz-cli's paths). Consider adding the dependency manifests, or mixing a source digest into the value.
2. **The backstop no longer bounds a held open.** An endpoint fault that holds an open forever now hangs that member. Consider a generous hold-independent ceiling, or an endpoint-side bound on room waits.
3. **The 403 wording under SSPI** names the account even when the 403 answers an unfinished exchange. The Surface axis may refine it.
4. **The exit gate's check-then-attach window** is decision 14's class. Record it beside decision 14 in S7.2's notes.
5. **Round-1 follow-ups 2-7** are unchanged.

## 4. Risks and next action

P2-3 is the remaining blocker. Without the fix, a Windows Basic credential fails as `Authentication` against servers that close the challenge's connection (a short keep-alive, or `Connection: close` on the 401), where the `Gh` path and 1.0.17 succeed. Its fix is bounded: reuse `lease_again` when the challenge connection is unusable or the send was not started.

**Next action:**
1. Fix P2-3, with the two closure rows.
2. Settle P3-6 (amend the design, or do not requeue after an exchange) and P3-7 (release the lease at the hold, or correct the text).
3. Obtain the operator's decisions on P3-1, P3-2 and P3-5 before S7.1.

I pre-commit to GO on a revision that resolves P2-3 as specified. This is round 1 of the two-round cap.
