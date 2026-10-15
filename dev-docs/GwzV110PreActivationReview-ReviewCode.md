# 1.1.0 pre-activation review (D19): code-axis review

**Review object:** the five D19 objects as they stand at the tuple (`GwzV110PreActivationReview-Package.md`): (1) TR2.6, the Phase 2 transport tree; (2) S3.1, the clock interfaces under timeout plan §2; (3) S3.2, the Q6 retirement fix; (4) WPP 4.8 / TR4.10, SSPI and gwz-sspi; (5) WPP 4.2/4.3, the WH2 helper process owner.

**Baseline** (frozen workspace `/Volumes/projects/limbo/gwz-dev-review19`):
- root `8fd99553cc1239f7242c7eb9e7427f2d8cb6e347`
- gwz-core `d6cb37e0f10a354730ef5086633c9f244b58373e`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252`
- gwz-cli `bebcc198baaacf75de9ab45113e4f26dcc051faa`
- gwz-py `37e4c151c92b157f8b9a24a49ed00e7ebc119b93`
- gwz-sspi `d3038882f7937be023d1835fe1b03e0309260acb`
- git2-rs `d13951f7e0bfb6e0efcee1207ac5b140adefa455` (reference only)
- gwz-core-evidence `1d8bd29d58d6bd9af8b5405b6f31c2d52208fc02` (evidence only)

**Date:** 2026-10-15. **Axis:** Code (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only.

**Verdict: NO-GO.** Counts: P0 0, P1 0, P2 2, P3 2.
- Object 1 (TR2.6, the Phase 2 tree): **GO** on this axis, within what I read (section 0).
- Object 2 (S3.1, the clock interfaces against timeout plan §2): **GO**.
- Object 3 (S3.2, the Q6 retirement fix's call graph): **GO**.
- Object 4 (WPP 4.8 / TR4.10, SSPI and gwz-sspi): **NO-GO**, on P2-1 and P2-2.
- Object 5 (WPP 4.2/4.3, the WH2 interfaces and 4.3's path rules): **GO**.

I pre-commit to GO on a revision that resolves P2-1 as specified, and resolves P2-2 in one of two ways: the self-provisioning correction, or an operator decision recorded in the TR18 decisions file that declares Windows crates.io builds unserved for Negotiate/NTLM, together with a refusal message and documentation that tell the truth.

## 0. Evidence base

**Tuple.** I checked the tuple at the start (about 17:15) and at the end (17:36 AEDT); every repository was at its tuple HEAD both times. The untracked files that were present at the start were still the same at the end:
- root: five `dev-docs/GwzRemoteTransportSshN2b-*` and `GwzWorkspaceRouteMappingDesign.md` files;
- gwz-core: `dev-docs/GwzRemoteTransportBugReport.md`;
- gwz-core-evidence: `runs/2026-09-22-alpha-setup-timeout/`.

None of them is mine. I wrote nothing in the workspace. The ignored `protocol/.regen-venv` and the `scripts/**/__pycache__` directories carry the clone's timestamp (17:15:28), so they were already there.

**Read fully:**
- gwz-core `src/git/endpoint/https_worker/native.rs` and `native/{exchange,owners,sspi}.rs` (1,165 lines), and `native/tests/rejection.rs`.
- gwz-sspi:
  - `src/{lib,secret,values}.rs`;
  - `src/worker/{bootstrap,framing,native,storage,session,platform,mod}.rs`;
  - `src/worker/windows/{mod,provider,conversation,owners}.rs`, plus `observation.rs` lines 1–80;
  - `src/supervisor/{api,values,context,kernel,futures,driver,dispatch,owners,ports,control,monitor,io,clock,platform,mod}.rs`;
  - `src/supervisor/windows/{mod,identity,inherited,launch,attributes,handles}.rs`.
- gwz-cli `src/worker_host.rs` and gwz-py `native/src/worker_host.rs`.
- gwz-core:
  - `https_worker/budget.rs` and `https_worker/credentials.rs`;
  - `transport_host/local_command.rs` and `transport_host/cancellable.rs`;
  - `git/gitbackend/transport_binding.rs`;
  - `transport_setting.rs`, plus lines 1–80 of `transport_setting/global.rs`;
  - `https_auth/process_tree.rs` and `process_tree/windows.rs`, `https_auth/runner.rs` and `https_auth/executable.rs` (to line 200).
- gwz-py `native/src/route.rs` and `route/transport.rs` (to line 260).
- gwz-transport `protocol/transport.taut.py`, the `binding.rs` profile modules and the `sequenced.rs` header.

**Sampled:**
- `native/tests/precedence.rs` (lines 1–245, plus every test name and assertion) and `native/tests/real_sspi.rs` (header and test names).
- `https_worker/prepare.rs` (lines 60–140 and 140–660).
- `transport_host/https_endpoint.rs` (lines 130–200, 310–360 and 455–490), `https_endpoint/retry.rs` (lines 50–260, 480–580) and `https_endpoint/poll.rs` (lines 120–260).
- `transport_host/request/https_failure.rs` (lines 150–370 and 740–800).
- `session/driver/pump.rs` (lines 320–400), `session.rs` (lines 120–150) and `agent_job/control.rs` (all of `Control`).
- `setup_retry.rs::classify`.
- `https_auth/owner.rs` (`HelperJob`, its `Drop` and `terminate`).
- `workspace_ops/handle_create_repo/response.rs` (the TR2.3/TR2.19 copy) and `handle_branch.rs` (its envelope construction).
- gwz-py `client_host.rs` (lines 25–60 and 160–185); gwz-cli `globalargs/dispatch.rs` (lines 40–80), `build.rs` and the `--max-retries` / `--transport` help text.
- Packaging and install documents: gwz-cli `docs/Install.md` and `docs/MachineOutput.md` §"Failed, rejected and partial results", and `GwzV110Plan.md` lines 395–410.
- The 4.8 evidence: `runs/2026-10-12-httpshelp/README.md` and `runs/2026-10-10-tr18-windows-auth-rows/README.md`.

**Not read:**
- `ssh_worker*`, `placement_endpoint*`, `ssh_destination.rs` (TR2.16/2.18), `ssh_admission.rs`, `https_opening.rs`, `https_pool*`;
- gwz-py `client_host/*` and `dispatch/*`;
- `session_host/*` beyond the entries;
- `process_tree/windows/{launch,pipe,attributes}.rs`, `lookup.rs`, `file_worker.rs`, `view*`, `secret.rs`;
- gwz-transport `pool/setup_clock*` in depth;
- TR2.7 (`ca_bundle`);
- the splits and dead-code removals;
- gwz-sspi's protocol codec and every gwz-sspi `*_tests.rs` file;
- generated protocol files (checked by regeneration instead).

I did not build gwz-cli or gwz-py.

**Commands and results** (in gwz-core unless stated):
- `check_windows_parity.py --root .`: 423 files, 194 occurrences, nothing new.
- `check_cfg_boundaries.py`: 1,625 files, 433 occurrences (gwz-core 431, gwz-cli 2, gwz-py 0), nothing new.
- `check_process_globals.py`: 1,413 files, 18 allowlisted, nothing new.
- `check_candidate_switches.py`, run with `--repo` for each repository: gwz-core 23 sites, gwz-cli 19, gwz-py 5. All match their inventories, and both switches are declared.
- `check_checked_artifact_boundaries.py --source src`: ok.
- `protocol/regen.py --check`: OK.
- `prepare.py` produced the candidate tree at `/Volumes/projects/limbo/build-targets/review19-code-cand`.
- Candidate library build, `--no-run`: exit 0.
- Candidate tests `-- https_worker::native https_failure transport_binding session_host`: 117 passed, 0 failed.
- gwz-sspi `cargo +1.95.0 test`: 118 passed, 0 failed.

## Problem, requirement provenance and scope

**Authority:**
- the operator's 1.1.0 scope: the CLI transport on macOS, Linux and Windows, with parity against released 1.0.17;
- D19, which defines this review point;
- for object 4: WPP 4.8 and amendment 2's TR4.10, with D3 (default identity only, no explicit-identity caller, Digest refused), TD1 (the logon session first, the helper not asked), OD16 (accepted), and TD11 (process and window checks only).

**What I traced:**
- Object 4's code stays inside D3. No caller of `Identity::Explicit`, `Package::Digest` or `DigestInput` exists in gwz-core, gwz-cli or gwz-py (grep). The worker refuses Digest twice: `session.rs:136` and `conversation.rs:85`.
- TR2.4 matches its text: a braced `mod profile` under the feature and under its negation, `#![cfg]` on `sequenced.rs`, CI runs `--features unstable-sequenced`, and `profile_three_binds_only_with_the_unstable_sequenced_feature` exists. The plan says `cfg_if`; the inner-attribute form is the checker's own documented equivalent.
- TR2.11 matches: `configure` returns before installing anything when there is no host context (`transport_binding.rs:238`).
- TR2.12 matches: three inventories, check-cfg declared in all three repositories, and the process-globals definition names both switches.
- TR2.3/TR2.19 match: the copy lives in the shared envelope builder. The one envelope I found that overwrites `errors` (`handle_branch.rs:597`) does so only for an `Ok` aggregate.

**Scope expansion and prerequisites:** I found neither an unauthorized expansion nor an unsupported prerequisite. P2-2 is the opposite case: a 1.1.0 release channel that the plan names (`GwzV110Plan.md:405`, `cargo install gwz --version 1.1.0 --locked … on dabeest`) is left unserved by the packaging design.

## 1. Findings

### P2-1 (object 4): on Windows, every non-200 answer to an authenticated request is classified as a credential rejection, bypassing the shared response classification

- **Root cause.** Under the native policies (`WindowsConfigured`, `WindowsDefault`), a 401 is answered inside `Client::authenticate`, and both arms judge the next response by a private rule instead of handing it to `https_policy::classify` and `tell_governor`, as the Unix `gh` path does:
  - **Basic** (`answer_basic`): any status other than 200 or a 3xx sets `credential.rejected`, sets `authenticated = Some(false)` and returns `Authentication`.
  - **SSPI**: `status != 401 || history.complete` returns `Authentication`.
- **Location:**
  - gwz-core `src/git/endpoint/https_worker/native/exchange.rs:358-371` (Basic);
  - `exchange.rs:272-278` (SSPI);
  - reached from `prepare.rs:484` (`if native_policy && response.status() == 401`);
  - worded by `transport_host/request/https_failure.rs:216-222`.
- **Violated invariant.** One server response must get the same error code and words on every platform. The Unix path classifies 403/404 on an advertisement as `RepositoryRefused` (`https_policy.rs:49`) and words a 403 with the "refused this account (HTTP 403)" text (`https_failure.rs:224`). It also judges a 429, or a 503 with Retry-After, as a capacity refusal (the accepted adaptive design §4.8). The helper text "(HTTP 401)" must only be said of a 401.
- **Reproduction.** Windows, a GitHub private repository, and a credential helper that answers with a token that lacks access to that repository:
  1. The anonymous GET gets `401` with `Basic realm="GitHub"`, and `authenticate` goes to `answer_basic`.
  2. The credentialed GET gets `404`, or `403` when SSO is not authorized.
  3. `answer_basic` sets `authenticated = Some(false)` and returns `Authentication`.
  4. The text shown is "The server rejected the credential your credential helper gave (HTTP 401) … Renew it … run `git ls-remote` … its first run fails and erases the old credential …".

  The same server, from macOS or Linux, gives `RepositoryRefused` and the repository or account words. A `503` or `429` answering the Basic credential or an NTLM token becomes `Authentication` (SSPI: "HTTPS endpoint request failed: Authentication"), and is never requeued. No test covers a non-200, non-401 answer on these paths: `precedence.rs`, `rejection.rs` and `https_native_tests.rs` contain no 403, 404 or 503.
- **Impact.**
  - The machine-output `code` differs by platform for the same server.
  - Windows users are told to renew a credential that is valid, with a recipe that does not apply: git does not erase a credential on a 404 or 403.
  - Server throttling during authentication fails the member at once on Windows, where Unix waits and retries.
  - 1.0.17 reports the HTTP status.
- **Required correction:**
  - In `answer_basic`, set `authenticated = Some(false)` and `credential.rejected` only for a 401. Return every other final response to `prepare`'s classification (`classify` and `tell_governor`), as the Unix path does; keep the 3xx return as it is.
  - In the SSPI exchange, keep the 3xx and "401 with no continuation token" refusals that design 7 requires. Hand a 403, 404, 429 or 5xx to the same classification, with the SSPI facts attached.
- **Closure test.** For each policy, fixture rows that answer the credentialed request with 403, 404, 429 (with Retry-After) and 503. Assert that the code and the words match the `Gh` policy's rows for the same responses (`RepositoryRefused`, the 403 text, a requeue). Assert that `authenticated` is not `Some(false)` and that the "(HTTP 401)" text does not appear.

### P2-2 (object 4, ARCHITECTURAL): a crates.io (or plain Cargo) build of the CLI on Windows can never answer Negotiate or NTLM, and TD1 leaves no other scheme

- **Root cause.** The worker's provenance contract requires a compile-time fingerprint that only the packaging producers supply:
  - gwz-cli `build.rs:6-20` sets `GWZ_SSPI_BUILD_FINGERPRINT` only from `GWZ_SSPI_PACKAGING_TARGETS`.
  - `worker_host.rs:22-25` and `packaging::build_fingerprint` refuse `None` with `WorkerUnavailable`.

  `cargo install gwz` builds on the user's machine, where that variable does not exist. TD1 then picks Negotiate or NTLM over Basic (`native.rs:216-220`), and `exchange.rs:119-122` fails with `Unavailable` and no other scheme to try.
- **Location:**
  - gwz-cli `build.rs:6-20` and `src/worker_host.rs:22-25`;
  - gwz-core `https_worker/native/exchange.rs:115-122`;
  - documentation and plan: gwz-cli `docs/Install.md` ("users with Rust can build and install the released CLI from crates.io") and `GwzV110Plan.md:405`.
- **Violated invariant.** Parity with 1.0.17 on every 1.1.0 release channel. 1.0.17 from crates.io authenticates the logon session through WinHTTP. A 1.1.0 install from crates.io refuses every server that offers Negotiate or NTLM, including one that offers `Negotiate, Basic`. The operator's rule of no native-route fallbacks for unserved setups also applies: the only remedy offered is `--transport native`.
- **Reproduction.**
  1. On dabeest: `cargo install gwz --version 1.1.0 --locked`, as the plan's Phase 8 step.
  2. Fetch from an IIS or Azure DevOps Server remote that offers Negotiate.
  3. `NativeCaller::capture` gets `Err(WorkerUnavailable)`, and `authenticate` returns `Unavailable` before sending any token.
  4. The text shown is "GWZ could not use its Windows sign-in worker … Reinstall gwz …". Reinstalling through the same channel reproduces the failure.
- **Impact.** A documented install path loses Windows integrated authentication by default, and the remedy in the message cannot work for that channel.
- **Required correction**, one of:
  - (a) Self-provision the CLI. Its worker is its own image (`current_exe`), so `build.rs` can generate a per-build random fingerprint when no packaging metadata is given and compile it in. The Hello of the same binary then matches, which keeps the "same build" guarantee. gwz-py's wheel keeps its producer.
  - (b) A recorded operator decision that Windows Cargo builds are unserved for Negotiate and NTLM. That also needs: a refusal that names the cause (an unprovisioned build, and how to get a provisioned one), `Install.md` and the 1.1.0 notes saying so, and Phase 8's dabeest `cargo install` proof restated.
- **Closure test.**
  - For (a): a gwz-cli test that a build without `GWZ_SSPI_PACKAGING_TARGETS` yields `Ok` from `executable()`, and that the worker's Hello matches. On dabeest, a `cargo install --path` build authenticates against the real-SSPI fixture.
  - For (b): a wording row for the unprovisioned case.

### P3-1 (object 4): an SSPI provider refusal on the first round is worded as a replaced worker, and its SECURITY_STATUS is discarded

- **Root cause.** `native_code` maps `ProviderRejected`, `CapacityUnavailable`, `ContainmentFailed` and `Protocol` all to `ErrorCode::Protocol` (`native.rs:331-343`). A round-1 step error carries History's facts, in which no token was offered, there is no status and the observation is `NotStarted` (`exchange.rs:195-200`). `sign_in_words` therefore treats it as "not began" and gives the replaced-worker text (`https_failure.rs:334-336`). `Failure::native_status()` is never read.
- **Location:** gwz-core `native.rs:331-343`, `exchange.rs:195-200`, `transport_host/request/https_failure.rs:316-337`.
- **Violated invariant.** A message must name the cause it has evidence for. "Did not answer as the one installed … Reinstall gwz" is misleading when Windows refused the sign-in.
- **Reproduction.** On a host where outgoing NTLM is restricted by policy, or where Negotiate has no usable mechanism, `InitializeSecurityContextW` or `AcquireCredentialsHandleW` fails. The worker sends an error frame with `ProviderRejected(status)`, and the user is told to reinstall gwz.
- **Impact.** An enterprise policy refusal cannot be told apart from a broken install, and the advised remedy does not help.
- **Correction.** Map `ProviderRejected` to a code or detail of its own, keep `native_status` (as a hex detail), and word it as "Windows refused the sign-in (SSPI status 0x…)", separately from the provenance refusals.
- **Closure test.** Make the fake port return `ProviderRejected(Some(0x8009030E))` on the first step. The text names the Windows refusal and the status, and does not contain "Reinstall gwz".

### P3-2 (object 4): the "retry resumes the eight rounds" claim is proven only on a path production never takes

- **Root cause.**
  - The comment on `Budget::native_rounds` (`https_worker.rs:262-264`) and the test `the_eight_rounds_are_the_operations_and_a_retry_does_not_get_eight_more` (`native/tests/rejection.rs:150-190`) claim that a setup retry carries the round count. The test passes the same `Budget` to two attempts by hand.
  - Production starts every retried attempt with `Held::new(None, …)` (`transport_host/https_endpoint/retry.rs:213, 247`), which makes a fresh `budget_for_open` at `retry.rs:562`.
  - Today no retry follows an exchange, because only a first-hop connect failure is `Phase::Setup` (`prepare.rs:263-276`). So there is no behavioural defect now.
- **Impact.** The test, and the 4.8 evidence README ("a retry of the open resumes the count"), give assurance for a carry that does not exist. A later change that makes a mid-exchange carrier loss retryable would silently get eight more rounds, and the test would stay green.
- **Correction.** Either carry the budget, or at least `native_rounds` and the logical deadline, into `Held` on a retry, or correct the comment and the test to the real invariant ("a retry happens only before any exchange"). Then drive the test through `HttpsEndpoint`'s retry machinery, not a hand-shared `Budget`.
- **Closure test.** An endpoint-level row: an exchange, then a forced retry. Assert the number of steps across both attempts is 8 or fewer, or that no retry is admitted after an exchange.

## 2. Invariant analysis (attacks that failed)

**Object 4, gwz-core side:**
- **Secret ownership.** Tokens and decoded challenges live in zeroed fixed storage (`Header`, `decode_owned`), and the worker's buffers are zeroized before `FreeContextBuffer` (`owners.rs:57-83`). Nothing that holds a secret derives `Debug`. `gwz_sspi::Failure` derives `Debug` but holds only a kind, a numeric status and a record id. There is no logging in the native files.
- **Channel binding.** `channel_binding` refuses a missing or empty binding (`exchange.rs:48-53, 132-139`); the binding is the connection's own.
- **Eight-round bound.** It is enforced both in core (`exchange.rs:100, 184`) and in the worker and kernel (`session.rs:56`, `kernel.rs:150, 193`).
- **Rejection is terminal.** A 401 without a token is terminal (`exchange.rs:290-296`), and a 3xx is refused.
- **Hidden panics.**
  - `Guard::retain`'s `expect` is safe: `_slot` is `Some` on every path into `authenticate`.
  - `take_session`'s `unreachable!` is safe: it runs only after Start returns `Ok`.
- **Cleanup probes.** These are not polled concurrently with the exchange; they are added only in `Guard::drop`.
- **Tombstone eviction.** gwz-sspi's tombstones are FIFO-bounded at 256 (`context.rs:347-349`), and `Unknown` is never treated as `Confirmed` by `NativeProbe`. That would wedge a slot, but it needs 256 reaps within one 5 ms endpoint reap tick with eight workers, which is not reachable.

**Object 4, gwz-sspi:**
- Kill-on-close Job attached at creation; `HANDLE_LIST` limited to the three child handles; a minimal environment; the absolute worker path refused if it contains a quote or NUL.
- The caller thread's impersonation and the primary SID, LUID and session are re-verified immediately before `CreateProcessW` (`launch.rs:197`).

**TD11, judged explicitly.** The only identity refusals are an impersonating caller thread and a changed primary token. There is no per-host or cross-user refusal, which matches "retain the proven process/window checks" for this component.

**OD16 hazard.** NTLM and Negotiate are answered for any https host with a binding, as accepted; plain `http://` goes by the native route with warning T2.

**Wire compatibility:**
- Every addition to `FailureDetail`, `CapacityDetail` and `Failure` is `MISSING_OK`.
- The new values of `SetupFailureCause` and `HelperFailureCause` are matched exhaustively by `timeout_origin` and `helper_cause_words`.
- `OperationPolicy.max_retries` exists only in the candidate composition (`candidate.taut.py:94-97`), as its accepted review says.
- The transport protocol is in-process in 1.1.0, so there is no cross-version writer.

**S6.1 and S6.2:**
- The cancellable entry closes every runtime it built under its own panic guards (`cancellable.rs:145-185`).
- gwz-py captures the environment and the `NativeCaller` while holding the GIL, before detach (`route/transport.rs:62-63`; `client_host.rs:173`).
- The CLI captures before fanout (`dispatch.rs:58-61`).

**Object 2.** `Control` exposes exactly §2's names: `complete_wait` is the only place a stall restarts, `check` and `quantum` restart nothing, and the aggregate is a fixed instant.

**Object 3.** `finish(&id)` is called only while the registration is not yet retired, and `mux_retired` latches. The monotonic `Instant` covers `CLEANUP`; on expiry the result is recorded with `peer_cleanup_confirmed: false`.

**Object 5:**
- N1 is closed: the parent ends are tokio `NamedPipeServer`s, which cancel on drop.
- N2 and N3 are closed, per the `ba722da0` diff and `process_tree/windows.rs`.
- The arms keep one interface (`spawn`, `kill`, `retire`, `drained`).
- 4.3's path rules hold: no `PATHEXT`, no current directory, relative `PATH` entries skipped, only `.exe` on Windows, `PATH` found by a case-insensitive name.

## 3. Follow-ups outside the finding bar

1. **cfg-boundary coverage.** `check_cfg_boundaries.py` scans gwz-core, gwz-cli and gwz-py only, not gwz-transport or gwz-sspi. Both comply today by inspection (braced inline modules, and the inner `#![cfg]`). Suggest adding them, for the reason the rule exists.
2. **Kerberos.** No real-Kerberos evidence exists; the dabeest rows are NTLM, including Negotiate selecting NTLM. A 200 without the final mutual token while `history.complete` is false ends `Protocol` (`exchange.rs:222-236`). This is a hypothesis about parity with servers that omit the token, and needs a domain row before Windows parity is claimed for Kerberos.
3. **Stale plan text.** WPP 4.10's "Open" item, about the refusals saying only `Unavailable`/`Protocol`, is stale: 5.6's P3-11 worded them.
4. **gwz-py supervisor in every build.** `ClientHost::new` creates the SSPI supervisor in every build, the non-candidate one included (`client_host.rs:49`), which spawns two threads on Windows that are never used there.
5. **gwz-py CI pin.** gwz-core's `.github/gwz-py.commit` pins `950064d`, behind the tuple's `37e4c151`. That is allowed by its bump rule, but CI then embeds an older gwz-py codec.
6. **Non-cancellable entry cleanup.** `local_command::run` drops a runtime without `shutdown` when `request()` fails (Drop only closes it). This is harmless for the CLI, but differs from the cancellable entry's "every path ends the runtime".

## 4. Risks and next action

**Risks:**
- P2-1 makes the outcome of Windows HTTPS authentication diverge from Unix and from 1.0.17 for common GitHub and enterprise responses.
- P2-2 removes Windows integrated authentication from a release channel the plan names.

Both should be fixed, or decided, before S7.1's switch removal, because they only become user-visible at activation.

**Coverage I could not give:** the unread areas in section 0 (`ssh_destination`, the HTTPS pool, gwz-py's dispatch, TR2.7, the splits). Windows-only code was judged by reading, with the dabeest evidence cited (`2026-10-12-httpshelp`, `2026-10-10-tr18-windows-auth-rows`, `2026-10-12-wh3`).

**Next action:**
1. Remediate P2-1 test-first, with rows for 403, 404, 429 and 503 under both Windows policies, asserting parity with the `Gh` policy.
2. Take P2-2 to the operator: self-provision the CLI, or decide.
3. Fix P3-1 and P3-2 in the same lane.
4. Re-review the remediation on the code axis, within the two-round cap.
