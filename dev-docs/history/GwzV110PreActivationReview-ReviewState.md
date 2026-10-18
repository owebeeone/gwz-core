# 1.1.0 pre-activation review (D19): STATE-axis review

**Review object:** The five D19 objects as they stand at the tuple, reviewed in the frozen workspace `/Volumes/projects/limbo/gwz-dev-review19`:
1. TR2.6, the Phase 2 transport tree.
2. S3.1, the timeout plan's S5.1 review.
3. S3.2, the Q6 aggregate.
4. WPP 4.8 / TR4.10, SSPI and gwz-sspi.
5. WPP 4.2/4.3, the WH2 helper owner.

The package is `gwz-core/dev-docs/GwzV110PreActivationReview-Package.md`.

**Baseline:**
- root `8fd99553cc1239f7242c7eb9e7427f2d8cb6e347`
- gwz-core `d6cb37e0f10a354730ef5086633c9f244b58373e`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252`
- gwz-cli `bebcc198baaacf75de9ab45113e4f26dcc051faa`
- gwz-py `37e4c151c92b157f8b9a24a49ed00e7ebc119b93`
- gwz-sspi `d3038882f7937be023d1835fe1b03e0309260acb`
- git2-rs `d13951f7e0bfb6e0efcee1207ac5b140adefa455` (reference only)
- gwz-core-evidence `1d8bd29d58d6bd9af8b5405b6f31c2d52208fc02` (reference only)

**Date:** 2026-10-15. **Axis:** STATE (durable-state semantics and adversity). Independent, adversarial, read-only.

**Verdict: NO-GO.** P0 0, P1 0, P2 2, P3 5.

| Object | Verdict | Open findings |
|---|---|---|
| 1 (TR2.6 tree) | NO-GO | P2-1; P3-4, P3-5 |
| 2 (S3.1 clocks) | GO | none |
| 3 (Q6 retirement fix) | GO | none |
| 4 (SSPI, gwz-sspi) | NO-GO | P2-2; P3-1 |
| 5 (WH2 owner) | GO | P3-2, P3-3 |

I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.

## 0. Evidence base

**Tuple.** I checked it at the start and at the end with `git -C <repo> rev-parse HEAD`. Every repository was at the tuple both times. The only untracked files were ones that existed before I started:
- 5 root `dev-docs/` files;
- gwz-core `dev-docs/GwzRemoteTransportBugReport.md`;
- the evidence directory `runs/2026-09-22-alpha-setup-timeout/`.

Nothing was written in any member.

**The package file.** It is not in the frozen workspace. I read it at `/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzV110PreActivationReview-Package.md`. Every controlling document I cite was read from the frozen workspace's copy.

**Read fully by me:**
- **gwz-core, native HTTPS authentication:**
  - `https_worker/native.rs`, `native/{exchange,owners,sspi}.rs`, `native/tests/rejection.rs`
  - `https_worker.rs`, `https_worker/{prepare,credentials,budget}.rs`
- **gwz-core, transport host:**
  - `transport_host/session.rs` and `session/{requests,port,close}.rs`
  - `session/driver/pump.rs`
  - the open path of `session/driver/opening.rs:160-330`, and `session/driver.rs:15-45`
  - `transport_host/https_endpoint.rs` (lines 1-520)
  - `https_endpoint/{poll,retry}.rs`
  - `transport_host/local_command.rs:40-131`, `cancellable.rs:140-240`
- **gwz-core, endpoint retry and setup:**
  - `placement_endpoint/{admission.rs:1-400, completion.rs, cancel.rs}`
  - `setup_retry/machine.rs:100-405`, `setup_retry/{requeue,operations}.rs`, `setup_retry.rs` `classify` and `phase_of`
  - `agent_job.rs`, `agent_job/{supervisor,control,control/shared}.rs`
  - `ssh_setup_context.rs`, `ssh_password_helpers.rs`, `https_auth/lookup.rs`
- **gwz-sspi:**
  - `lib.rs`, `secret.rs`, `values.rs`
  - `supervisor/{mod,api,context,values,kernel,monitor,driver,dispatch,control,io,futures,owners,clock,ports,platform}.rs`
  - `supervisor/windows/launch.rs`
- **Hosts:**
  - gwz-cli `src/worker_host.rs` and `globalargs/dispatch.rs:47-100`
  - gwz-py `native/src/worker_host.rs` and `client_host.rs:1-200, 262-280`
- **gwz-transport:**
  - `pool/setup_clock.rs`, `pool/setup_clock/{state,transitions}.rs`
  - `mux/mod.rs:405-630` (cancel, finish, advance)

**Read fully by three read-only helper passes:** I ran these to widen coverage. I re-verified, at the cited lines, every item from them that I file below. Attacks they report as failed are marked "(helper pass)" in §2.
- **gwz-sspi:**
  - `worker/**` and the rest of `supervisor/windows/**`
  - `protocol/{supervision,worker_codec,framing,adapters,profile}.rs`, `bin/gwz-sspi-worker.rs`, `packaging.rs`
- **gwz-core WH2:**
  - `https_auth/{process_tree.rs, process_tree/windows/**, owner.rs, pipes.rs, runner.rs, runner/environment.rs, file_worker.rs, secret.rs}`
  - the WH2 contract, its round-2 reviews and RemPlan-2, and `git show ba722da0`
- **gwz-py:**
  - `client_host*`, `dispatch/mod.rs`, `route/*`, `error.rs`, `shims.rs`, `src/gwz/bridge.py:1-340`
  - design §2.1-§2.7, and gwz-core `transport_scope.rs`

**Sampled:**
- `ssh_network.rs` (the `complete_wait` sites), `agent_socket.rs:25-70`, `agent_auth.rs:80-225`
- `https_pool.rs` (scope and checkout), gwz-transport `pool/lifecycle.rs:7-35`
- `pageant_exchange/sys.rs:200-250`, `https_backend/native_stack.rs`, `gitbackend/transport_support.rs:255-280`, `gitbackend/refs.rs`, `repository.rs`
- the design documents:
  - `GwzWindowsHttpsIntegrationDesign-DRAFT.md:150-185`
  - `GwzTransportWindowsParityDesign.md` (lines 87, 116-124, 201, 213, 251)
  - `GwzTransportCredentialHelperTimingAmendment.md:40-100`
  - the Q6 DiscoveryState and RemPlan
  - `GwzRemoteTransportReleaseReadiness.md:201-243`
  - `GwzRemoteTransportAlphaTimeoutPlan.md` §2 and S5.1
  - `GwzTransportReleasePlanAmendment-2.md:183-191, 510-560`
  - `GwzTransportHandoff.md:160-170, 240-260`

**Not read:**
- `ssh_worker*`, `placement_endpoint.rs` beyond `pending*`, `ssh_destination.rs`, `ssh_admission.rs`, `https_opening.rs`
- `https_connection.rs` beyond grep, `setup_retry/limit/**` (accepted object), `transport_setting*`
- TR2.5, TR2.7 and TR2.11 code, the splits
- the gwz-py `dispatch` submodules, the gwz-sspi test bodies, the generated protocol files

**Commands and results.** All were run from the frozen gwz-core unless stated.

| Command | Result |
|---|---|
| `check_windows_parity.py --root .` | 423 files, 194 occurrences, nothing new |
| `check_cfg_boundaries.py` | 1,625 files, 433 occurrences, nothing new |
| `check_process_globals.py` | 18 allowlisted, nothing new |
| `check_candidate_switches.py` | 23 sites match |
| `check_checked_artifact_boundaries.py --source src` | ok |
| `protocol/regen.py --check` | committed artifacts current |
| `prepare.py` into `/Volumes/projects/limbo/build-targets/review19-state-cand`, then the candidate `cargo +1.95.0 test --lib` (external target `review19-state-target`, `GWZ_TEST_GIT=real`) | filters `transport_host::session::cleanup_tests https_worker::native https_endpoint::retry_tests https_endpoint::requeue_tests placement_endpoint::retry_tests setup_retry agent_job ssh_setup`: 348 passed, 0 failed |
| gwz-sspi `cargo +1.95.0 test` (external target `review19-state-sspi`) | 118 passed, 0 failed, across 7 targets |

The Windows-only gwz-sspi and WH2 code was reviewed by reading only. The Windows evidence cited is in `gwz-core-evidence/campaigns/transport-qualification/runs/`:
- `2026-10-03-sspi-native-bec3ba15`, `2026-10-03-sspi-native-remediation-1`
- `2026-10-03-tr1-8-sspi-job-list`, `2026-10-03-tr1-8-sspi-process-worker`
- `2026-10-10-tr18-windows-auth-rows`, `2026-10-10-wh2-helper-owner`
- `2026-10-11-phase4`, `2026-10-12-wh3`, `2026-10-12-wh3c`

None of these rows exercises the interleavings in P2-1, P2-2, P3-1 or P3-2.

## Problem, requirement provenance, and scope

- **Authority.** The 1.1.0 scope is the CLI transport on macOS, Linux and Windows, with parity against released 1.0.17. D19 defines this review point. The findings trace to:
  - the retry plan §5 and §5.3, as amended by OD18;
  - adaptive concurrency §5.2 (an open's allocation clock stops while its key or site holds it);
  - the timeout plan §2 ("User interaction spends neither clock");
  - the Windows parity design §251 ("local helper work is excluded from network clocks");
  - the helper timing amendment (M4 and `helper_budget_ms`);
  - the WH2 contract C9 and §6;
  - the gwz-py design §2.6.
- **No unauthorized expansion found.** The code I read implements what its controlling text asks. I found no step that widens trust or credential scope.
- **Open items I rule on:**
  - **HandOff 242-247, "the backstop excludes the allocation wait".** A defect, filed as P2-1. It is worse than recorded: the backstop also seals the whole request.
  - **HandOff 165-170, "on an HTTPS GET, the same allowance also times the wait for response headers".** Accepted. Waiting for headers is a period with no progress, and the HTTPS design makes active I/O one cumulative budget per open (`budget.rs:30-32`).
  - **The macOS keychain pin (Amendment :215).** It holds by construction. `native_stack.rs:23-36` keeps the built-in roots when `platform` is `None` and adds the file's certificates. The Code axis owns the test.
  - **The nine TR2.5 step-1 interpretations.** I did not read `transport_setting`. I identify no state consequence from the handoff text and leave them to the Code axis.
  - **TD11 B** (no cross-user or cross-logon-session refusal of a found Pageant). Judged acceptable on this axis because it fails closed:
    - the request mapping's DACL admits only the caller and SYSTEM, and the mapping is non-inheritable (`pageant_exchange/sys.rs:208-249`);
    - `FindWindowW` searches only the caller's desktop;
    - a foreign-SID receiver therefore cannot read the mapping;
    - the residual (same-SID code at a lower integrity level) is the accepted L9.
- **Deferred items** (TD/D/OD decisions, the adaptive limit machine, step 5.6) were not re-litigated. Where a finding touches one, it attacks the interaction, not the outcome.

## 1. Findings

### P2-1 (object 1, TR2.1/TR2.10). The driver's per-open backstop fires on legitimately held opens and cancels the whole request

- **Root cause.** The driver's open backstop is an operation-wide terminal. It is driven by a per-open clock that keeps running while the endpoint holds the open with its allocation clock stopped.
- **Location (gwz-core):**
  - `src/transport_host/session/driver/pump.rs:266-276`: on expiry, `entry.reply.complete(Err(Timeout)); let _ = owner.cancel(&entry.request); entry.peer.disconnect();`.
  - The bound: `session/driver/opening.rs:298-300`, set once and never refreshed. `session/driver.rs:21-32` makes it `(R+1)×164 s + waits`, which is 663.75 s at the default R = 3 and 164 s at `--max-retries 0`.
  - What it does not cover: `transport_host/https_endpoint/retry.rs:451` and `:493-501` (`waits_for_room` and the closed gate stop the allocation clock), and `git/endpoint/placement_endpoint/admission.rs:209-213` and `:240-263` (the same for SSH).
  - The scope of the action: gwz-transport `src/mux/mod.rs:467-517`. `cancel(request)` seals the request, gives every route of the request a Cancel, and `live_request` (`:621`) then refuses every later open of it.
- **Violated invariants:**
  - a member's failure is its own (retry plan §5.3);
  - an open held by its key or its site waits with its allocation clock stopped (adaptive §5.2; commit `766b2953`);
  - a terminal must not widen beyond its own stream.
- **Reproduction (defaults: jobs 100, per-host 32, `--max-retries 3`):**
  1. A workspace fetch has 32 HTTPS members on one host. gwz admits all 32 (`handle_fetch.rs:133-145`).
  2. Early throttling (429, or 503 with `Retry-After`) makes the site's limit machine narrow N to 4.
  3. 28 opens wait in the endpoint for room. Their allocation is stopped and the driver backstop runs.
  4. With transfers of about 100 s each, the last waves wait about 700 s.
  5. At 663.75 s the first such open's backstop completes that open with `Timeout` and calls `owner.cancel(request)`.
  6. The four members transferring at that moment get Cancel. Every other waiting member is cancelled. Every later open of the request is refused.

  At `--max-retries 0` the same happens after 164 s of hold.
- **Impact.** One member's wait turns into a whole-operation cancellation mid-transfer. For a push, the in-flight receive-pack ends with effect `Possible`, so the member results are mixed. Nothing is corrupted, but the operation fails for every member. HandOff:245 recorded the open edge ("the backstop excludes the allocation wait") without its request-wide action.
- **Required correction:**
  1. The backstop ends only its own stream. Complete its reply, cancel its own stream (`entry.stream.cancel()`, as `Session::cancel` already does per stream at `session/requests.rs:69`) and disconnect its peer. Never call `owner.cancel(request)` from a per-open clock.
  2. The bound must not run while the endpoint holds the open. Either re-arm it when the endpoint starts an attempt, or exclude held time. Record that the bound covers attempts, not holds.
- **Closure test.** In the candidate harness, with a stepped clock:
  - hold one open of a request at the endpoint (site full) past its backstop;
  - while that open is held, keep a second stream of the same request mid-transfer.

  Assert that the second stream completes, that the request is not sealed, and that the held open either starts once room appears or fails alone. Red before the fix (the second stream receives Cancel).

### P2-2 (object 4, steps 4.4/4.8). On Windows a configured helper answering `Basic` is charged to the 30-second network setup deadline D

- **Root cause.** `answer_basic` bounds the helper lookup by the native logical deadline D, a network setup clock, instead of the helper's own interaction allowance.
- **Location (gwz-core):**
  - `src/git/endpoint/https_worker/native/exchange.rs:336-343`: `timeout_at(budget.logical_deadline, lookup)` maps expiry to a bare `failure(ErrorCode::Timeout)`, with no facts, no setup cause and no `helper_budget_ms`.
  - D is anchored at `https_worker/prepare.rs:122-134` (`now + budget.connect`, 30 s by default, before the first checkout).
  - The behaviour is pinned by `native/tests/exchange.rs:103-133`.
- **Violated invariants:**
  - `GwzTransportWindowsParityDesign.md:251`: "local helper work is excluded from network clocks".
  - The helper timing amendment: the allowance is `min(configured, Open, 120 000)` and is reported as M4 (`setup_cause` `Interaction` plus `helper_budget_ms`).
  - The timeout plan §2: "User interaction spends neither clock".
  - Platform parity: the Unix `Gh` path asks the helper before checkout, on its own clocks (`prepare.rs:172-187`, `credentials.rs:96-109`).
- **Reproduction:**
  1. On Windows, use `WindowsConfigured` (the default helper policy) with an https remote that challenges `Basic` only, such as a private GitHub repository.
  2. The configured GCM has no cached credential.
  3. Connect, TLS and the anonymous GET spend part of D.
  4. GCM's interactive sign-in takes 40 s. At D the lookup future is dropped.
  5. The open fails `Timeout` with no setup cause. The classifier returns it (`Phase::Other`), so it is not retried. The user sees a generic timeout, not M4.

  The same remote under `Gh` on macOS or Linux waits up to 120 s. Under `--ssh-timeout 0`, D is `None` and the helper is not bounded at all, which shows D is the network setup clock.
- **Impact.** Windows users whose helper interacts fail in under 30 s, by a different rule from the other platforms, with a non-diagnostic message. This is a parity and diagnosability defect on the default Windows credential path, which the S7.1 activation ships.
- **Required correction.** Run the Basic lookup on the helper's allocation and interaction allowance, as `credentials.rs` already does, with D paused for the lookup. Either extend D by the lookup's duration or anchor D after the lookup returns. An interaction expiry then reports `Interaction` plus `helper_budget_ms` (M4).
- **Closure test.** Rework the pinned test into two rows:
  - a helper that outlasts the old D but stays within the interaction allowance authenticates;
  - a helper that outlasts the interaction allowance fails `Timeout` with `setup_cause = Interaction` and `helper_budget_ms` set.

  In both rows `port.starts == 0`.

### P3-1 (object 4, gwz-sspi). The worker's pipe ends are created inheritable, so concurrent std spawns inherit them

- **Root cause.** `pipe()` creates both pipes with `bInheritHandle: 1`.
- **Location (gwz-sspi):**
  - `src/supervisor/windows/launch.rs:32-48` (`bInheritHandle: 1`);
  - the parent ends are cleared only at `:140-141`;
  - the NUL handle is created inheritable at `:142-159`;
  - the child ends stay inheritable until they are dropped after `CreateProcessW` returns (`:217-220`).
- **Violated invariants:**
  - the SSPI design §4 rule that the parent pipe ends are not inherited;
  - the WH2 contract's rule (C9) that no end is inheritable outside the creation window.

  The WH2 contract's statement that gwz-sspi "does not take part" (contract :23) holds for what the worker inherits, but not for gwz-sspi as a victim of other spawns.
- **Interleaving:**
  1. The gwz-sspi launch thread is between `:138` and `:218`.
  2. Another thread spawns through std `Command`, with `bInheritHandles=TRUE` and no handle list. Examples are libgit2's `Cred::credential_helper` for a native-route `http://` member (`gitbackend/transport_support.rs:268`), the git CLI routes, or the gwz-py embedding process.
  3. That child inherits the worker's pipe ends, including the parent ends during the first window.
- **Impact.** A worker that dies mid-conversation no longer gives the parent reader end-of-file (`io.rs` `read_frame`), so the failure is found only at D. A third process holds the token-carrying pipes. The pipes outlive the record. The effect is bounded by D, and there is no exposure beyond same-user processes that gwz spawned.
- **Correction.** Create both pipes non-inheritable. Set `HANDLE_FLAG_INHERIT` on the two child ends and the NUL handle only around `CreateProcessW`, and clear it on every path, as WH2's `InheritWindow` does. Full closure needs P3-2's gate. Correct contract line :23.
- **Test.** The WH2 pair, mirrored for gwz-sspi:
  - after `pipe()`, no end is inheritable;
  - the child ends are inheritable only inside the creation window.

### P3-2 (object 5). The WH2 spawn gate is owed now, and C9's list of std spawn sites misses the libgit2 helper spawn

- **Root cause.** The spawn gate that §6 tied to step 4.4 was not added when 4.4 landed, and C9 omits the one std spawn site on a transport path.
- **Location:**
  - gwz-core `dev-docs/GwzTransportWindowsHelperOwnerContract-DRAFT.md:23` (C9) and `:63` (§6: "Recommend adding the gate when step 4.4 admits `WindowsConfigured` and `Gh` to HTTPS (before the policy is reachable in a build that ships)");
  - the window in code: `https_auth/process_tree/windows/launch.rs:189-209`;
  - the unlisted site: `src/git/gitbackend/transport_support.rs:268` (`git2::Cred::credential_helper`, a std spawn; allowlist `scripts/checks/process_globals_allowlist.json:22`).

  Step 4.4 landed (`34f6a403`), D19 precedes the switch removal that ships it, and no gate exists.
- **Interleaving.** A Windows workspace has an `http://` member (native route, D2) and an https member answered by `Basic` through the transport's helper. libgit2 spawns the user's helper while the transport's helper is inside `CreateProcessW`. The stray inherits all three ends.
- **Impact:**
  - the lookup sees no stdout end-of-file until the stray exits; GCM waiting for the user, or anything it spawns with inheritance, stalls it to its interaction deadline and it fails `Timeout`;
  - the stray holds write access to the answer pipe.
- **Correction.** Before S7.1, either implement the gate in the host context and route the std sites through it, or record the operator's acceptance of the open exposure with the libgit2 site added to C9.
- **Test.** The contract's control spawn (a standard spawn inherits an inheritable handle), run concurrently with a lookup under the gate, inherits none of its ends.

### P3-3 (object 5). On Windows the helper's configuration view, which can carry credentials, is copied into unwiped allocations

- **Root cause.** The Windows environment path copies `GIT_CONFIG_PARAMETERS` into plain `String`, `OsString` and UTF-16 buffers that are freed without wiping.
- **Location (gwz-core `src/git/endpoint/https_auth/`):**
  - `runner/environment.rs:152-156` (`parameters_value`, a String/OsString copy) and `:36` (`set` copies it again);
  - `runner.rs:111-114` (`HelperCommand.environment`);
  - `process_tree/windows/launch.rs:83-110` (per-entry UTF-16 copies) and `:156` (the block). All are freed unwiped.
- **Violated invariant.** `secret.rs:5`: "Every endpoint-owned allocation that contains credential bytes is wiped". The view itself is held as a `SecretBuffer` (`view.rs:51`, `view/framing.rs:128`). OQ-C's accepted copy list (contract :35) names only the pipe buffers and says the write carries "url=... only, no secret".
- **Reproduction.** Set `http.extraHeader = Authorization: Bearer <token>` in the user's configuration and run any Windows helper lookup. After it returns, freed heap holds the token in several copies, up to about 4 MiB each.
- **Impact.** Credential bytes remain in the process heap, outside the accepted list. Unix has the analogous std `Command` copies, also unlisted.
- **Correction.** Build the block and the copies in wiping owners, or add them, for both platforms, to OQ-C's accepted list.
- **Test.** An audit-probe row (as in `native.rs` `Header`) on the environment block owner.

### P3-4 (object 1, S6.2). gwz-py's transport notes attach to the interpreter after the host has closed at exit

- **Root cause.** The note sink takes the GIL unconditionally and never consults the host's exiting state.
- **Location:** gwz-py `native/src/route/transport.rs:217-233` (`log_note` calls `Python::attach` from the transport's pass thread).
- **Violated invariant.** gwz-py design §2.6 and `client_host.rs:12`: an operation that outlives the bound at exit attaches to no interpreter. Amendment-2's decision 14 covers only `dispatch::record`.
- **Interleaving.**
  1. An operation is stalled at a throttling host.
  2. The exit hook (`client_host.rs:267-276`) cancels it, waits `CLEANUP_BOUND` and returns.
  3. The session's own 5 s cleanup (`transport_host/session.rs:47`) is still running.
  4. The HTTPS pass delivers a queued governor note or redirect detail (`https_endpoint/poll.rs:247-256`). `log_note` then attaches during finalization.
- **Impact, at exit only.**
  - Python 3.13+: pyo3 refuses; the panic is caught by `catch_unwind`, and the panic hook prints to stderr.
  - Python 3.10-3.12: the thread attaches to a finalizing interpreter. CPython can end that thread inside Rust frames.
- **Correction.** The sink drops notes once the host has closed at exit (the flag `Ran.exiting` already carries).
- **Test.** A note emitted after `close(true)` returns makes no interpreter call.

### P3-5 (object 1, S6.2; ARCHITECTURAL). The exit hook cannot bound async `call()` operations, because CPython joins their executor threads first

- **Root cause.** The exit mechanism (`atexit`) runs after CPython has joined the executor threads that carry async `call()`.
- **Location:**
  - gwz-py `src/gwz/bridge.py:298-315` (`_run_native` on `asyncio.to_thread`; the cancel path waits for completion);
  - `native/src/client_host.rs:63-98` (a waiting `call` waits for a slot on that thread);
  - the hook at `:267-279`.
- **Violated invariant.** gwz-py design §2.6: at exit the callback refuses waiting operations and cancels running ones within the bound, and "finalization never waits on it".
- **Mechanism.** `Py_FinalizeEx` runs `threading._shutdown`, which runs `concurrent.futures`' `_python_exit` and joins every executor worker, before `_PyAtExit_Call`. The gwz hook therefore runs only after every running and queued `call()` has completed; a queued one is even admitted and run in full. `submit()` is unaffected.
- **Impact.** For async `call()`, interpreter exit is bounded only by the operations' own durations, not by `CLEANUP_BOUND`.
- **Correction.** Do one of the following:
  - run the close from a hook that executes before the executor join;
  - carry `call()` on gwz's own unjoined threads, as `submit` is carried;
  - state the limitation in S7.2's notes.
- **Test.** A Python row: an async `call()` blocked on a stalled fixture, then interpreter exit. The process ends within `CLEANUP_BOUND` plus a margin.

## 2. Invariant analysis (attacks that failed)

**Object 2: S3.1's checklist, traced in code.** The `agent_job` and `ssh_setup` rows were run (in the 348).

| Checklist item | Code that satisfies it |
|---|---|
| The stall resets only on a completed native attempt | `control.rs:141-159` → `network_progress` (`transitions.rs:118-133`); `begin_network_wait` arms only an Inactive stall (`:108-114`); `end_slice` completes only on readiness (`control.rs:164-169`); the `EAGAIN` sleeps are not completions (`agent_socket.rs:58-64`) |
| The aggregate does not reset | The aggregate `Timer` is only paused and resumed (`state.rs:17-29`, `transitions.rs:58-61, 84-89, 150-151, 164-169`) |
| Interaction pauses both | `publish_local` pauses both clocks; `publish_network` resumes both |
| Zero disables network deadlines and still disposes | A stall of zero is `Disabled` (`state.rs:73-77`); the aggregate is `None` (`mod.rs:141-145`); cleanup has its own bound (`agent_job.rs:333-352`) |
| A late result is neither reused nor `PermissionDenied` | `poll_result` returns the failure while the result is unconsumed, and the reaper discards it (`agent_job.rs:134-146, 312-318`); the error kinds are `TimedOut` and `ConnectionAborted` (`ssh_setup_context.rs:137-148`) |
| `stall` and `aggregate` stay distinct | `settle` (`state.rs:138-145`) and `control.rs:108-119` both resolve a tie to Aggregate; they map to `SetupFailureCause::{Stall, Aggregate}` (`ssh_setup_context.rs:97-104`) |

**Object 3: Q6.**
- `mux_retired` is set once and never cleared (`pump.rs:359-368`).
- A cleanup deadline closes the session only if retirement never completed (`:369-372`).
- The result is write-once (`get_or_insert`).
- A completed request no longer closes the shared session (`cleanup_tests.rs:180`, run).
- On interleavings of seal while closed (`requests.rs:95-103`), close mid-wait, and a late pass after the result: every path records once and never claims retirement it did not observe.
- Physical accounting: the per-request SSH count (`placement_endpoint.rs:233-246`) excludes pool cleanup. Production reports use the runtime's shutdown snapshot instead (`local_command.rs:110-124`, `cancellable.rs`), so no zero is invented. This is recorded as a follow-up.

**Object 4: native exchange and gwz-sspi.**
- **Cleanup ownership.** Start, Session and Finish are retained with the operation dependency and the endpoint slot (`exchange.rs:5-44`). They are reaped every 5 ms (`transport_host/https_endpoint.rs:163`), and runtime shutdown waits for confirmation (`:166-168`).
- **Double retain.** A step error with a pending record followed by `session.cancel()` drops only a second probe of the same record.
- **Tombstone eviction.** Eviction of the 256-entry FIFO, so that `Unknown` is never `Confirmed`, would need 256 reaps inside one 5 ms reap period with at most 8 workers. Not reachable.
- **Cancellation.** Every await selects cancel, lease cancel and D (`:170-175, 189-194, 243-248`). Retained futures are polled with a no-op waker after the record is cancelled.
- **Exchange bounds:**
  - rejection is terminal (`:272-296`);
  - a 3xx during the exchange is refused;
  - eight rounds per Open (`:100-106, 184-186`), and the worker enforces rounds 1 to 8 with no Continue at round 8 (helper pass, profile.rs and session.rs);
  - Digest is never answered (`native.rs:85-92`, `Offers::select`);
  - the channel binding is never empty (`exchange.rs:48-53`), and its length is checked in both processes (helper pass).
- **Identity.** gwz-core always sends `CurrentLogon` with `digest: None` (`exchange.rs:140-153`).
- **Scope.** A connection's scope is set once (gwz-transport `lifecycle.rs:19-24` refuses re-scoping). `Authenticated` retires its scope when dropped (`native.rs:389-398`). A POST on a different connection generation is refused, never re-authenticated (`prepare.rs:298-307`).
- **Worker containment (helper pass).** Created suspended with `JOB_LIST` and `KILL_ON_JOB_CLOSE`, no breakaway, `IsProcessInJob` checked before resume. ISC flags have no delegation. Every SSPI handle and buffer is freed or wiped on every path.
- **Supervisor accounting.** The kernel's terminal cannot be overwritten (`kernel.rs:99-106`). `reapable` requires full proof (`:252-254`).
- **Not attacked.** The retry machine's eight-round claim (see follow-ups).

**Object 1: retry machine, OD18, gh reuse, permits.**
- **HTTPS flights.** Every machine flight is taken on every settle path (`retry.rs:140-145, 179-186, 216-218`; cancel at `https_endpoint.rs:240-244`). There is no stuck probe.
- **SSH flights.** Cancel paths abandon the flight (`admission.rs:84-90`, `completion.rs:114-118`). A late success after abandonment is cancelled (`completion.rs:136-150`).
- **OD18 cold wave.** Implemented (`machine.rs:163-193, 253-322`).
- **Requeue.** A requeue counts against the member's own attempts but not the key's (`retry.rs:179-214`).
- **Challenge reuse.** Keyed by service, host, port, path and username (`https_endpoint.rs:520-529`). It is taken only for the same destination base, session and operation, and only while live (`https_worker.rs:233-246`). An unadmitted connection is dropped (`retry.rs:166-169`). An expired challenge is dropped (`poll.rs:32-42`).
- **Credentials.** Cached per route and destination, with rejection sticky and fail-closed (`credentials.rs:53-66`).
- **Job permits.** Released only after join, on consumption or disposal (`agent_job.rs:122-157, 306-331`). The `reserve` wakeup race is closed by its re-take (`supervisor.rs:118-135`).

**Object 5: WH2 (helper pass, sample re-verified).**
- Job with `KILL_ON_JOB_CLOSE` and membership from creation.
- `HANDLE_LIST` names exactly the three ends.
- `terminate` and `Drop` release only once the tree is drained; success releases at leader exit, as OQ-A decided.
- `reap_pending` and `pending_cleanup_count` are counted under one lock.
- `ba722da0` closes N1 (tokio named pipes, cancelled on drop), N2 (`=C:` names) and N3 in code (`pipe.rs:80-90`, `launch.rs:121-142, 189-209`). N3's contract text is incomplete: see P3-2.

## 3. Follow-ups outside the finding bar

1. **Eight-round test and comments.** `native/tests/rejection.rs:150-190` and `https_worker.rs:262-263` claim a TR2.1 retry carries the first attempt's budget. Production builds a fresh budget for every later attempt (`retry.rs:562-565`), so the round count and D both reset. The property holds only because a retry needs a failed first connect, which happens before any exchange. Make the test drive the retry machine, or correct the comment.
2. **WPP 4.2's state text.** `GwzTransportWindowsParityPlan.md:468` says permits are released only when the tree is drained. C8 and OQ-A release them at leader exit on success. Align the text.
3. **Unbounded cleanup loop.** `ssh_password_helpers.rs:77-84` loops with no deadline until the helper's cleanup count reaches zero. This is retention by design, but on Windows any undying job member pins the setup thread. Consider a retained cleanup record instead of a loop.
4. **gwz-sspi aliasing.** `worker/windows/conversation.rs:204-222` passes the same context as `*const` and `&mut` to ISC. This is UB by the Stacked Borrows model; it is the Code axis's to confirm.
5. **Per-request SSH cleanup count.** `placement_endpoint.rs:233-246` excludes physical pool cleanup. It is not used for any user-visible report at this tuple. Keep it that way, or include the per-request physical work.
6. **gwz-py, late success.** A late success is delivered after a close has reported the operation unfinished (helper pass, O1). This is a contract gap, not invented progress. §2.5 could state it.
7. **Not-in-job comment.** `launch.rs:224-233` misdescribes the branch for a helper that is not in the job. It is practically unreachable.

## 4. Risks and next action

**Risks for activation (S7.1):**
- **P2-1** turns adaptive throttling, which is the normal response to GitHub-style rate limits, into whole-operation cancellations on long workspace transfers.
- **P2-2** makes the default Windows helper path fail for any interactive sign-in that takes longer than about 30 s.
- **P3-1 and P3-2** are narrow inheritance races. Together they are the reason the WH2 contract asked for the spawn gate before shipping.

**Next action:**
1. Fix P2-1: stream-scoped backstop, with held time excluded.
2. Fix P2-2: the Basic helper on the interaction clock, with M4 reporting.
3. Rule P3-2: add the gate, or record acceptance with the libgit2 site added.
4. Schedule P3-1, P3-3, P3-4 and P3-5 with their tests.
5. Re-review only those diffs. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.
