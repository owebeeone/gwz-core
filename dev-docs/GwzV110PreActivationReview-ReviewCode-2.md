# 1.1.0 pre-activation review (D19): code-axis re-verdict (round 1 remediation)

**Review object:** the five D19 objects at the revised tuple, which is the round-1 remediation merged.

**Baseline** (frozen workspace `/Volumes/projects/limbo/gwz-dev-review19b`):

| Repo | Commit |
|---|---|
| root | `484a5d88e3ae9e6b303e6926700802a4c73f174a` |
| gwz-core | `8e425e7634656d0189d11b27f4708847985474df` |
| gwz-transport | `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (unchanged) |
| gwz-cli | `b7c9edcd8d9da1adb2f04eb4ea6343b0762cc55a` |
| gwz-py | `da098f5c52e822a3f7ed0920b016e9b0c5c5a10a` |
| gwz-sspi | `d3038882f7937be023d1835fe1b03e0309260acb` (unchanged) |
| gwz-core-evidence | `9d33418978526cf01fa1d7dd5125fd439a5af62b` (reference only) |

**Date:** 2026-10-16. **Axis:** Code (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only.

**Verdict: GO.**
- Prior findings: three closed and verified (P2-1, P2-2, P3-2). One P3 is deferred with an acceptable disposition (P3-1), on condition that the deferral is recorded (see the closure table).
- New findings: 0 P0, 0 P1, 0 P2, 3 P3 (P3-3, P3-4, P3-5). None blocks.
- Per object:
  - Object 1 (TR2.6): **GO**.
  - Object 2 (S3.1, the clock interfaces): **GO**.
  - Object 3 (S3.2, the retirement call graph): **GO**.
  - Object 4 (WPP 4.8 / TR4.10, SSPI): **GO**, with P3-1 deferred and P3-3 to P3-5 open, all P3.
  - Object 5 (WH2 4.2/4.3): **GO**.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1 (Windows classified every non-200 answer to an authenticated request as a credential rejection) | `answer_basic` marks the credential rejected, and sets `authenticated = Some(false)`, for a 401 only. Every other answer (403, 404, 429, 5xx) goes back to `prepare`'s shared classification. The SSPI exchange hands 4xx/5xx other than 401 to the same classification (`is_classified_elsewhere`). It keeps its 3xx refusal and its "401 with no continuation token" refusal. gwz-core `4518f665`. | **Yes.** I re-traced my original counterexample (a GitHub private repository, a token that lacks access, 403/404) on the revised tree:<br>• `exchange.rs:390-400`: a non-401, non-200 answer returns `Ok(response)`.<br>• `prepare.rs` then marks the credential for a 403 exactly as the Unix path does.<br>• `classify` gives `RepositoryRefused`, and the words are the `Gh` 403 text, not "(HTTP 401) … Renew it".<br>• On the SSPI path (`exchange.rs:281`), a 403/404/429/503 goes to the same classification and `tell_governor`, so a throttle is requeued.<br>The RemPlan's closure test is `native/tests/closure.rs::d19_code_p2_1_a_non_401_answer_to_a_windows_credential_is_classified_as_the_gh_policy_does`. For both Windows policies and 403/404/429/503 it asserts: the code equals `Gh`'s, the limit-machine verdict equals `Gh`'s, `authenticated != Some(false)`, the HTTP status, no "(HTTP 401)", and the words equal `Gh`'s (except the new SSPI 403 sentence, P3-5). A companion row keeps a 401 as a rejection. I ran it: pass. | **Closed** |
| P2-2, ARCHITECTURAL (a Cargo-built Windows CLI could never answer Negotiate/NTLM) | Operator decision D23 (self-provision). gwz-cli `b7c9edc`: when `GWZ_SSPI_PACKAGING_TARGETS` is absent, `build.rs` compiles in a fresh per-build fingerprint (`build_support/self_provision.rs`). The host and the worker take their bytes from one `COMPILED_BUILD` constant. | **Yes.**<br>• Re-traced: every Cargo build now emits `GWZ_SSPI_BUILD_FINGERPRINT` (`build.rs`, the new `else` arm), so `executable()` no longer returns `WorkerUnavailable`.<br>• The table arm still panics on a missing target instead of degrading silently.<br>• `Cargo.toml`'s `include` carries `/build_support/**`, so a crates.io build compiles the generator.<br>• Closure tests:<br>&nbsp;&nbsp;– `worker_host.rs`: `a_build_without_packaging_metadata_provisions_its_own_worker`, and `the_worker_of_a_cargo_build_says_the_build_its_host_expects` (the Hello bytes are injected and compared);<br>&nbsp;&nbsp;– `tests/self_provision.rs`: form, and uniqueness over 2,600 values.<br>• On dabeest (`runs/2026-10-16-r19provision`): a `cargo install --path` build passes 10 of 10 rows, including clone, fetch and push over Negotiate and NTLM. The pre-D23 base fails with the "Reinstall gwz" text. The image-replaced row is refused before any native round.<br>• I did not run gwz-cli's tests myself; I relied on that run's logs. | **Closed** |
| P3-1 (an SSPI `ProviderRejected` on round 1 is worded as a replaced worker, and its SECURITY_STATUS is dropped) | Not remediated: the wire has no field for the status, so this becomes a gwz-transport change for 1.2.0, deferred with D16. | **Disposition acceptable for a P3.** I re-checked the lane's argument (`runs/2026-10-16-r19auth/README.md`, "Blocked"): `FailureDetail`'s fields are a closed vocabulary, `NativeFacts` has no status, and remapping `ProviderRejected` to an existing code cannot separate it from `IdentityMismatch` or the provenance refusals. That holds. The misdirection ("Reinstall gwz" for a policy refusal) remains in 1.1.0.<br>**Condition:** the deferral is recorded only in the commit message `4518f665` and the lane README. `GwzV110PreActivationReview-RemPlan.md` still lists P3-1 as fixed in lane `auth`. Record the deferral in the RemPlan (or in the 1.2.0 wire list beside D16), and list the wording as a known 1.1.0 limitation in S7.2's notes. | **Deferred (accepted)** |
| P3-2 (the eight-round carry across a retry was proven only on a path production never takes) | Carry `native_rounds` into `Held` (`Held::after_attempt`, `Budget::resume_native_rounds`), and test through `HttpsEndpoint`. | **Yes.**<br>• `retry.rs:224, 262` carry `carry.budget.native_rounds()` into the held open, and `start_attempt` resumes it.<br>• The path is now reachable, because a 429 after an exchange is requeued (P2-1).<br>• Closure test: `transport_host/https_endpoint/native_round_tests.rs::d19_code_p3_2_*`, through the endpoint's own retry machine. It asserts that the throttled attempt was retried, 8 steps in total, and that the result is `Protocol`. I ran it: pass.<br>• The comment on `Budget` is corrected. | **Closed** (see P3-3 for a consequence of the carry) |

## Changed-range analysis

**What changed.**
- gwz-core `4518f665` (lane `auth`): exchange classification, the Basic lookup on the helper's own clock, `CHALLENGE_HOLD` with `lease_again`/`send_authorized`, the round carry, and the SSPI 403 words.
- gwz-core `93a08dbc` (State P2-1): the per-open `Backstop`, the holds record in `Handoff`, `publish_holds`/`held_opens`, and a per-stream `Cancel` in place of `owner.cancel(request)`.
- gwz-core `6778cbfa` (State P3-3): wiping environment owners (`EnvValue`, `Wiped`), and the Windows block built in one owner.
- gwz-cli `b7c9edc` (D23).
- gwz-py `da098f5` (State P3-4, an exit gate on the note sink).
- Not remediation, checked for regressions in the five objects only: gwz-core `8e54e500` (C1: the transport's dependencies in `Cargo.toml` under `[target.'cfg(gwz_transport_candidate)']`), `665f42ae`, `7ff37f3f`, and documents.

**The re-lease path (`lease_again`, `send_authorized`, `CHALLENGE_HOLD`).**
- Ownership is consistent:
  - the lapsed lease is finished `Discarded` before the new checkout;
  - the new lease is scoped to the credential;
  - `connection_id` and `reused` are updated;
  - `send_authorized` is the unchanged second half of `auth_send`.
- `tell_governor` judges the new connection through `pool_connection`.
- A redirect after the re-lease still goes through `prepare`'s redirect arm.
- `prepared._slot` is untouched on the Basic path, so the slot accounting holds.
- **One divergence from the `gh` path:** the checkout's phase and `setup_cause` are discarded, and its connect is not the open's setup (P3-4).

**The SSPI non-401 path.** `Ok(response)` on 4xx/5xx returns with the session still in `Guard`. The drop cancels the session and retains or releases the slot as on every other error path. `prepared._slot` stays `None`, which is safe: `prepare` reaches only its `Fail` arm (`may_carry` needs a 401) and returns. I re-ran the cleanup assertion `shutdown == 0` with the closure rows: pass.

**The backstop and the holds record:**
- **Lock order.** `Handoff.holds` is taken last, under a session's state lock, in both `publish_holds` and `held_opens`, so there is no inversion.
- **Stale holds.** `close_state` publishes an empty set, so a closed endpoint session leaves none behind.
- **Mux race.** The per-stream `Cancel` relies on the mux leaving the route in place on Cancel (`routing.rs`: Cancel is not in `transition`'s removal list). A late `Opened` from the endpoint is therefore still a valid transition, and does not reach the pump's `Err(_) => close_state` arm, which would have closed the whole session.
- **The trade-off.** An open the endpoint reports as held is never ended by the backstop. That is State's disposition, and I record it as a follow-up.
- **Cli placement.** The residue (no holds reported) is documented in `backstop.rs` and the commit message as a 1.2.0 wire item. I accept that disposition on my axis: it is the pre-remediation behaviour, now ending only its own stream.

**The round carry.** It does what design 8 says ("the operation's eight"). Combined with the new requeue it produces P3-3.

**The new SSPI 403 words.** The condition is a 403 with `method == Sspi && credential_offered`, and the code mapping is `RepositoryRefused` to `RemoteRejected`, consistent with `Gh`. The sentence itself is not backed by the document it points to (P3-5), and Surface has not seen it.

**D23's per-build fingerprint and the provenance guarantee.** For the CLI, the worker is `current_exe`, so the guarantee changes from "a deterministic artifact-set identifier" to "the image at that path was compiled from the same build-script run". That is strictly at least as strict for crates.io installs, each of which builds afresh. It is unchanged for packaged releases, whose table arm is untouched. The value is not a secret.
- **Accepted consequence:** builds are not bit-for-bit reproducible. This is documented in `Install.md` and `HostPackaging.md`.
- **Overstatement:** because `build.rs` reruns only on `GWZ_SSPI_PACKAGING_TARGETS` or its own sources (`rerun-if-env-changed`), successive incremental builds in one target directory share a value. `Install.md`'s "a `gwz` built elsewhere is never used" therefore overstates for one developer's incremental rebuilds. It is harmless, because the worker is the running image's own path.
- gwz-py's producer is unchanged.

**Other changes:**
- C1's manifest moved the transport's dependencies behind the switch. The candidate tree still prepares and builds, and the source checks pass (section 0).
- `6778cbfa` keeps `process_tree`'s interface (`HelperCommand.environment` is now `Vec<(OsString, EnvValue)>`, used only inside `https_auth`).
- `da098f5` touches only gwz-py's note sink.

### P3-3 (object 4): once throttled attempts have used the operation's eight native rounds, a requeued retry fails as `Protocol` instead of ending with the throttle

- **Root cause.** Since the remediation, a 429 or 503 answering an SSPI token is requeued, and the retry resumes the rounds used (`retry.rs:224, 262`; `Budget::resume_native_rounds`). Nothing stops an open whose rounds are already spent from being requeued. Its next attempt then meets the up-front refusal (`exchange.rs:106`, `budget.native_rounds >= NATIVE_ROUNDS` gives `Protocol`) before any SSPI work. The merged facts (`credential_offered` true, the earlier `http_status` 429) give the generic "HTTPS endpoint request failed: Protocol". The `Gh`/Basic path on the same server would keep retrying up to the attempt limit and end with the capacity outcome.
- **Location:** gwz-core `src/transport_host/https_endpoint/retry.rs:186-215, 246-268` (requeue and retry); `src/git/endpoint/https_worker/native/exchange.rs:106`.
- **Violated invariant.** One server behaviour should give one outcome across policies (P2-1's parity rule): a persistent throttle ends with the throttle's capacity outcome, not `Protocol`.
- **Reproduction.**
  1. `--max-retries 7`, and an NTLM server that answers every AUTHENTICATE token (round 2) with `429 Retry-After: 1`.
  2. Attempts 1–4 use rounds 1–8 and are requeued (attempt 4 is below the allowed 8).
  3. Attempt 5 refuses at `exchange.rs:106` and the member ends `Protocol`.

  With the default `--max-retries 3`, the attempt limit and the rounds run out together, and the spent/capacity path wins. A variant: if a mechanism needs a third client step after the 200 (a final token), the 200 at rounds 7–8 ends `Protocol` at the cap even though the server accepted.
- **Impact.** Narrow: it needs a raised `--max-retries`, or an exchange of more than two steps, under persistent throttling. Where it happens, the member fails early and is told the wrong reason.
- **Correction.** In `settle`, do not requeue an open whose carried `native_rounds` are spent: finish it through the `spent` path with the throttle (or capacity) failure. Alternatively, have the up-front refusal return the carried failure rather than a fresh `Protocol`.
- **Closure test.** Extend `native_round_tests`: `max_retries = 7`, and a server answering 429 to every authenticated request. Assert at most 8 steps, a final code and words equal to the `Gh` policy's row for the same server, and no `Protocol`.

### P3-4 (object 4): the Basic re-lease's connect is not the open's setup, so its failure is neither retried nor judged, and loses its timeout origin

- **Root cause.** `lease_again` checks out with no limit-machine tag (`None`), then drops the checkout's phase and `setup_cause`: `exchange.rs:445`, `checkout.map_err(|(error, _)| with_facts(error.code, error.effect, &facts))`. The `gh` path's equivalent is `prepare`'s re-checkout after a lapsed `ChallengeLease`, which runs with `first = hops == 0` (`prepare.rs:233`). It keeps `setup_cause` (`:271`), sets `FirstConnect::Failed` and calls `judge_setup` (`:273`), so a transient connect failure there is a `Phase::Setup` retry.
- **Location:** gwz-core `src/git/endpoint/https_worker/native/exchange.rs:412-452`.
- **Violated invariant.** The P2-1 and State P2-2 remediation states that the Windows Basic path does what the `gh` path does once the credential is in hand. Setup-retry classification (`setup_retry::classify`) must see a connect failure as `Phase::Setup` with its cause.
- **Reproduction.**
  1. Windows, `WindowsConfigured`, a helper that takes more than 5 s (for example Git Credential Manager prompting).
  2. The fresh connection's connect fails transiently (connection refused, or a stall).
  3. The open fails at once with no retry. A timeout carries no "stall" or "aggregate" origin, and the limit machine is not told.
  4. On macOS or Linux, the `gh` policy retries the same failure.
- **Impact.** Narrow (a slow helper and a failing reconnect), but a parity gap introduced by the new path.
- **Correction.** Carry the phase and `setup_cause` from the checkout. Give the re-lease the open's member tag and `judge_setup`, as `prepare`'s first-hop checkout does. Or route the re-lease back through `prepare`'s loop, which is what the `gh` path does.
- **Closure test.** A `slow_helper` row whose fixture refuses the second connection once: the open retries as a setup failure, and `failure.setup_cause` is set when the retries are spent.

### P3-5 (object 4): the new SSPI 403 sentence points to a troubleshooting section with nothing for it, and has not had Surface review

- **Root cause.** `https_failure.rs:229-236` ends "See Troubleshooting: HTTPS Credential Failure". `gwz-cli/docs/Troubleshooting.md` §"HTTPS Credential Failure" (lines 130–196) covers credential helpers only. It has no entry for a 403, for a Windows sign-in, or for any `Sspi` text (grep). The advice "`git ls-remote` … shows the server's own explanation" also assumes git signs in silently with the logon session. That holds for Negotiate under git's default `http.emptyAuth`, but on an NTLM-only server git may prompt instead.
- **Location:** gwz-core `src/transport_host/request/https_failure.rs:229-236`; gwz-cli `docs/Troubleshooting.md:130-196`.
- **Violated invariant.** A message's pointer must lead to text about its case. New user-facing words go through Surface (plan §6).
- **Impact.** A Windows user refused with a 403 is sent to helper-repair instructions that do not apply.
- **Correction.** Add a Windows sign-in entry (403, rejected logon, worker texts) to the troubleshooting page, or point to an existing one. Qualify the `ls-remote` advice for NTLM-only servers. Take the sentence through S7.5's Surface items.
- **Closure test.** A row that the cited section heading exists and mentions the Windows sign-in 403. A docs link check on the message's pointer.

## Problem, requirement provenance, and scope

Every change in the remediation range traces to a filed finding (Code P2-1, P2-2, P3-2; State P2-1, P2-2, P3-3, P3-4) or to an operator decision (D23, for P2-2). There is one addition beyond the findings: the `CHALLENGE_HOLD` re-lease. It is stated in the brief as a connection-hold follow-up to State P2-2, and it follows directly from moving the Basic lookup off D (a lookup that takes longer than the server's idle timeout needs a fresh connection). It is within authorized scope. The non-remediation commits (C1, a fixture, two test rows) touch none of the five objects' behaviour, apart from building, which I checked.

There is no unauthorized expansion, and no P2 deferred without a decision. The two non-remediated items on my axis are P3-1 (a P3, deferral acceptable once recorded) and the Cli-placement backstop residue (no 1.1.0 driver places Cli, and it is recorded for 1.2.0).

## 0. Evidence base

**Tuple.** Verified at the start (04:24) and at the end (04:31 AEDT): identical. The untracked files are the same as at the start (root 5, gwz-core 1, evidence 1), all pre-existing and none mine.

**Read fully:**
- the RemPlan;
- D23;
- the diffs of `4518f665` (`exchange.rs`, `prepare.rs`, `budget.rs`, `retry.rs`, `https_failure.rs`, the closure rows, `native_round_tests.rs`, `rejection.rs`), `93a08dbc` (product files) and `b7c9edc` (`build.rs`, `self_provision.rs`, `worker_host.rs`, `tests/self_provision.rs`, `Install.md`);
- `native/exchange.rs` lines 318–552 as they stand at the tuple;
- gwz-transport `mux/routing.rs` lines 1–140 and 260–320, and `mux/mod.rs` lines 377–640 (the Cancel and route semantics);
- `runs/2026-10-16-r19auth` (the P3-1 and limitations sections) and `runs/2026-10-16-r19provision` (README).

**Sampled:**
- `6778cbfa` (`process_tree/environment.rs`, `windows/launch.rs`, `windows_parity` rows);
- `da098f5`'s stat;
- `8e54e500`'s stat and `Cargo.toml` headings;
- `throttle.rs::tell_governor`;
- `pump.rs` lines 75–300 and 300–360;
- `Troubleshooting.md` lines 130–200.

**Not read:**
- gwz-py's `da098f5` code;
- `6778cbfa`'s tests and `wiped.rs`;
- `backstop_tests.rs`;
- the r19backstop and r19pyfix READMEs;
- the C1 workflows and the `prepare.py` rewrite in depth.

**Commands** (run in gwz-core):
- `prepare.py` produced `/Volumes/projects/limbo/build-targets/review19r2-code-cand`.
- Candidate library build (`--no-run`, `CARGO_TARGET_DIR=…/review19r2-code-target`): exit 0.
- Tests `-- https_worker::native native_round_tests backstop https_failure transport_binding`: **100 passed, 0 failed**. Every `d19_*` closure row passed: the two P2-1 rows, the four State P2-2 rows and the P3-2 row.
- Source checks:
  - `check_windows_parity`: 429 files, 196 occurrences, nothing new;
  - `check_cfg_boundaries`: 1,635 files, 433 occurrences, nothing new;
  - `check_process_globals`: 1,421 files, 18 allowlisted, nothing new;
  - `check_candidate_switches`: gwz-core 23, gwz-cli 19, gwz-py 5, all matching;
  - `check_checked_artifact_boundaries`: ok;
  - `protocol/regen.py --check`: OK.
- gwz-cli's and gwz-py's tests were not run here; I cite the lane logs instead.

## 2. Invariant analysis (attacks that failed)

**P2-1 closure, by attack:**
- A 401 still marks the credential rejected (the closure row checks it).
- A 3xx after Basic still follows TR1.6 OQ4(a).
- A 3xx in the SSPI exchange is still refused.
- The SSPI 4xx path releases or retains its slot and the cleanup probe exactly as other errors do (`shutdown == 0` in the closure rows).
- A 429 after Basic reuses the cached credential on the retry, with no second helper call.

**Re-lease:** no unwrap on a `None` lease (`finish` errors propagate before the lease is reassigned); `protocol_error` is set only by `serve`; no exchange-hold rule applies to a fresh connection.

**Backstop:** no lock inversion; no session-wide close on a late `Opened` (the mux keeps the route on Cancel); `close_state` clears published holds; the retirement call graph (object 3) is unchanged apart from removing the `owner.cancel(request)` seal.

**D23:**
- the table arm still fails hard on a missing target;
- the crates.io package includes the generator;
- the Hello bytes come from one constant for host and worker;
- a foreign build's image is refused (dabeest image-replaced row).

**Round carry:** carried only on the requeue and Retry paths. It is consistent with design 8, except for the edge in P3-3.

## 3. Follow-ups outside the finding bar

1. **The backstop no longer bounds a held open.** An endpoint bug that leaves an open held forever is no longer caught by the driver. Room waits are still bounded by the allocation clock; key-retry and site holds are bounded by the retry machine and the Retry-After cap. Suggest a long absolute ceiling (for example cleanup plus the worst-case hold) as a pure safety net.
2. **Incremental builds share the self-provisioned value.** Successive builds in one target directory reuse it (`rerun-if-env-changed`). `Install.md` could say "a `gwz` built by another build" more precisely. Harmless for the CLI.
3. **`https_failure.rs` is at 992 lines**, 8 below the split threshold. The next wording change splits it.
4. **Round 1's section 3 follow-ups** remain as the RemPlan records them.

## 4. Risks and next action

**Risk.** Low on this axis. Both blocking findings are closed by code, with closure rows, and P2-2 has the dabeest installed-path evidence. The three new P3s are narrow edges of the new paths, and wording.

**Next action:**
1. Record P3-1's deferral in the RemPlan or the 1.2.0 wire list, and in S7.2's notes.
2. Fix P3-3 and P3-4 in a follow-up lane before or after S7.1, at the lane owner's choice; neither blocks.
3. Put P3-5's sentence and its troubleshooting entry into S7.5's Surface items.
