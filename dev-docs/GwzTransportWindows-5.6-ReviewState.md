# Windows parity 5.6: State-axis review

**Review object:** Windows parity plan step 5.6 (TR4.7), the single review point for:
- gwz-core `4c7f9f7d..37705303`, plus `caf1e4c8`, less the accepted adaptive step and accepted steps 4.1/4.2;
- gwz-transport `6146bd58..a27817b9`;
- gwz-cli `21d62310..220501a5`;
- gwz-py up to `28aff11d`.

**Baseline (checked at start and at end, unchanged):**

| Repo | Commit |
|---|---|
| root | 42feaa9dd98b49b6cf2b214de2a6ecf808fde670 |
| gwz-core | 37705303f80d9f70d1ab4bcc0782323decaa6e2c |
| gwz-transport | a27817b9c86fd60a07a9e77d62b1f5e94a5cf252 |
| gwz-cli | 220501a5f00abd76d23f5b3ef75499d5f7a09d19 |
| gwz-py | 28aff11d7f441542ef4d14c492dda5f4419ba22a |
| gwz-sspi | d3038882f7937be023d1835fe1b03e0309260acb |
| git2-rs | d13951f7e0bfb6e0efcee1207ac5b140adefa455 |
| gwz-core-evidence | 350703caad14385d28d05538e3f5839d1a5df3c8 |

**Date:** 2026-10-13. **Axis:** State. Independent, adversarial, read-only.

**Verdict: NO-GO.** 0 P0, 0 P1, 7 P2, 8 P3.

I pre-commit to GO on a revision that resolves P2-1 to P2-7 as specified. Each has a bounded, local remedy. None is ARCHITECTURAL.

## 0. Evidence base

**Read fully by the reviewer:**
- **Pageant:**
  - `src/git/endpoint/pageant_exchange.rs` and `pageant_exchange/sys.rs`;
  - `pageant_exchange/receiver.rs`;
  - `agent_pageant.rs` and `agent_pageant/{sys,pin}.rs`;
  - `agent_source.rs` and `agent_source/{select,fault,pageant_words}.rs`.
- **Agent pipe and agent client:**
  - `agent_pipe.rs`;
  - the WouldBlock loops of `agent_client.rs`;
  - the flow of `agent_auth.rs`.
- **Control and setup clock:**
  - `agent_job/control.rs` and `control/room.rs`;
  - `remaining` and `deadline_after` in gwz-transport `pool/setup_clock.rs`.
- **SSH setup:**
  - `ssh_network.rs` and `ssh_network/kex_retry.rs`;
  - `ssh_connection.rs`;
  - `ssh_local.rs`;
  - `ssh_handoff.rs`;
  - `ssh_host_key.rs` (the classification part);
  - the failure mapping in `ssh_setup.rs`;
  - `setup_retry.rs` (`classify`).
- **Machine proxy:**
  - `transport_host/machine_proxy.rs` and `machine_proxy/{proxy_string,bypass,winhttp}.rs`;
  - `https_connection.rs` (`Config`, `proxy_for`, the connector gate, the CONNECT tunnel);
  - `transport_host/endpoint_environment.rs` (the capture and `tls_config`);
  - `NativeCaller::capture`;
  - `request.rs:120-200`;
  - `session.rs` (the machine and home records);
  - `transport_host/mod.rs:300-345`.
- **Credential helpers (in `https_auth/`):**
  - the post-ba722da0 diff of the helper stack;
  - `view.rs` and `view/helpers.rs`;
  - `lookup.rs`;
  - `runner.rs:60-140` and `runner/environment.rs:1-140`;
  - `executable.rs`;
  - `https_worker/credentials.rs`.
- **Native exchange and failure wording:**
  - `https_worker/native/exchange.rs`;
  - `https_worker/prepare.rs:140-440`;
  - `transport_host/request/https_failure.rs:60-315`.
- **gwz-transport:** the codec and admission diff.
- **gwz-cli:** the `dispatch.rs`, `lib.rs` and `worker_host.rs` diffs.
- **Native commit:** `native_commit/{exec,staging}.rs`, `native_commit.rs:140-186`, and `regular_file.rs`.

**Delegated to three read-only helper passes (findings re-verified by the reviewer):**
- **Adaptive limit machine:**
  - read fully: `setup_retry/limit/**`, `machine.rs`, `requeue.rs`, `capacity.rs`, and the callers `https_endpoint/{retry,poll}.rs`, `throttle.rs`, `completion.rs`, `local_command.rs`;
  - sampled: `prepare.rs`, `serve.rs`, `admission.rs`, `ssh_pool.rs`, `https_pool.rs`.
- **Native commit and tag:** read fully: `native_commit/**`, the callers, and vendored libgit2 `index.c`, `repository.c`, `iterator.c`, `diff_generate.c` and `refdb_fs.c`.
- **Phase 1:**
  - read fully: `socket_wait.rs`, `regular_file.rs`, `idle_watch.rs`, `shutdown_watch.rs`, `ssh_shutdown.rs`, `ssh_connection.rs`, `ssh_key_container.rs`, `ssh_limits.rs`, and libssh2 `pem.c` and `wincng.c`;
  - sampled: `ssh_setup.rs`, `ssh_pool.rs` (disposal), `ssh_channel.rs`.

**What the reviewer re-verified:**
- P2-2, P2-3, P2-4, P2-5, P2-6 and P2-7: re-read at the cited lines.
- libgit2's `iterator.c:1380-1385` and `diff_generate.c:1128-1137`.
- libgit2's `repository.c:1650-1680` (the cached index) and `index.c:817-831` (no staleness check).
- libssh2 `pem.c:73-97` (`readline_memory`).

**Sampled:** `https_worker/{throttle,serve}.rs`, `native.rs`, the WH2 owner (unchanged since ba722da0), and the gwz-py commit list.

**Not read:**
- test files beyond test names;
- gwz-sspi;
- `fixture_*`, `ssh_controlled_server`, `agent_pipe_server` and the other test fixtures;
- the gwz-py source diff;
- docs other than the cited design and decision sections.

**Commands run and results:**
- **Repository checks** (`python3.13 -B scripts/checks/...`):
  - `check_windows_parity.py --root .`: "406 files … unported 1, paired 151, platform 39; nothing new".
  - `check_checked_artifact_boundaries.py --source src`: ok.
  - `check_cfg_boundaries.py`: "nothing new".
  - `check_candidate_switches.py`: 23 sites match.
  - `check_process_globals.py`: "nothing new".
  - `check_filesystem_boundary.py`: passed.
- **Targeted candidate tests:**
  - Prepared with `prepare.py /Volumes/projects/limbo/build-targets/review56-State-cand`.
  - Run with `RUSTFLAGS='--cfg gwz_transport_candidate' CARGO_TARGET_DIR=…/review56-State-target GWZ_TEST_GIT=real cargo +1.95.0 test --lib -- kex_retry agent_pageant pageant_exchange view::helpers machine_proxy agent_source ssh_handoff`.
  - Result: **116 passed, 0 failed**.
- **Real git 2.52 checks in scratch repos:**
  - `credential.<pattern>.helper` is applied by `git credential fill` for these patterns: `https://*.example.test`, `https://Example.Test`, `https://example.test:443`, and `https://example.test/org` (for the URL `…/org/repo`).
  - `git commit -a` with a tracked directory at mode 300 keeps `d/f`.
  - Config-defined hooks are ignored by 2.52 (exit 0).
- **git-scm.com:** the `git-hook` page documents `hook.<name>.command` and `hook.<name>.event` at 2.54.0; the 2.53.0 page redirects to the 2.43.0 page.

**Windows evidence consulted (gwz-core-evidence `campaigns/transport-qualification/runs/`):**
- `2026-10-12-pageant`: synthetic Pageant only. Rows for real Pageant 0.83 are owed. A cancel-mid-read flake is noted.
- `2026-10-12-sshagent`
- `2026-10-13-cngretry`: 6 of 600 first handshakes failed with -8; 0 of 600 failed with the retry.
- `2026-10-12-helpercause`
- `2026-10-12-proxy`

## Problem, requirement provenance, and scope

**Requested outcome.** gwz 1.1.0 is the CLI transport on macOS, Linux and Windows, with Windows parity against released 1.0.17.

**Authority for each area reviewed:**
- **Pageant, agent pipe, machine proxy, D12 and D14:** plan steps 3.3 to 3.6, 4.6 and 4.7, and operator decisions D5 (b), D6, D7, D12 and D14.
- **Helper-cause classification:** operator D13. Its stated contract is that T4 ("no helper configured") is shown only when git has no helper. The run `2026-10-12-helpercause` says the view applies "Git's own `credential.helper` and `credential.<url>.helper` rules".
- **Native commit/tag fast path:** the operator decision of 2026-10-12 in GwzNoFallbackPlan §6, which says "the native result must equal Git's … anything native cannot prove equal goes to Git".
- **Adaptive follow-ups:** the adaptive design §§4.5, 4.10, 5.3, 5.5 and 9, and its decisions D8 and D13.
- **Phase 1:** plan steps 1.1 to 1.8.

**Scope.** I found no unauthorized scope expansion. Every remedy below restores a stated contract; none adds scope. Decision outcomes are treated as deferred: the Pageant wait on the network clock, retry after a Pageant stall (design §5 "cost, stated plainly", L20), refusing only https opens, the CNG -8 retry, and Capacity = 76.

## 1. Findings

### P2-1. D13's "helper in effect" test does not match how Git selects helpers, so T4 tells users with a configured helper that none is configured

**Location:** gwz-core `src/git/endpoint/https_auth/view/helpers.rs:18-55` (`split` and `matches`) and `:60-85` (`in_effect`). Consumed at `https_auth/lookup.rs:133-134`. Worded at `transport_host/request/https_failure.rs:128` (`NO_HELPER_WINDOWS`, `:304`). The test at `helpers.rs:170` (`assert!(!scoped("https://Example.test", URL))`) asserts the divergence.

**Violated invariant:** D13 says T4 is said only when git has no helper. The view must answer what Git itself would. A misclassification must not invent a state ("no helper is configured").

**Mechanism:**
- Git (since 2.26) reads `credential.<url>.*` through `urlmatch` with `url_normalize`. That means:
  - lowercase scheme and host;
  - default port elided;
  - path matched as a prefix;
  - `*` wildcard host labels.
- `matches` instead requires a byte-equal protocol and host (port included) and an equal path.

**Reproduction:** verified with git 2.52. With each of these, `git credential fill` uses the helper but `in_effect` returns false:
- `credential.https://*.example.test.helper=manager` with `https://git.example.test/r`;
- `credential.https://Example.Test.helper`;
- `credential.https://example.test:443.helper`;
- `credential.https://dev.azure.com/org.helper` with `https://dev.azure.com/org/proj/_git/r`.

Then:
1. The helper declines (cancelled sign-in, nothing stored).
2. `credential fill` exits non-zero, giving `HelperRejected`.
3. `prepared.helper` is false, so the failure becomes `NoHelper`.
4. On Windows the user sees T4: "no git credential helper is configured. Configure one (git config --global credential.helper manager …)".

**Impact:**
- The diagnosis is wrong and misdirects users with per-organization (Azure DevOps style) or wildcard helper setups on the Windows platform the step targets.
- Following the advice adds a second, global helper rather than fixing the declined one.
- No credential leaks.

**Required correction:** implement Git's urlmatch selection in `in_effect`:
- lowercase the scheme and host;
- normalize away the default port;
- match the pattern path as a prefix at `/` boundaries;
- support `*` per host label;
- keep Git's `match_partial_url` fallback.

Alternatively, ask Git with `git config --get-urlmatch credential.helper <url>` under the same view environment.

**Closure test:** table rows for wildcard, case, `:443` and path-prefix patterns expecting `held == true`, plus a row each for a different port, a different user and a non-prefix path expecting false. Remove or invert `helpers.rs:170`. Run a real-git row on Windows (Git for Windows 2.55/2.56) asserting `Declined` with a wildcard-scoped helper that declines.

### P2-2. Native commit/tag skips hooks that are defined in configuration (Git 2.54 and later)

**Location:**
- `src/git/gitbackend/native_commit/route.rs:162-183` (`hook_check`);
- `native_commit/hooks.rs:56-65` (`firing_hook`), which inspects only hook files in hook directories;
- no `hook.*` key check anywhere under `native_commit/`.

**Violated invariant:** GwzNoFallbackPlan §6 rule 1: "A hook would fire" means the operation goes to the CLI. The fail-safe rule is that anything native cannot prove equal goes to Git.

**Mechanism:** Git 2.54 and later (git-scm `git-hook` 2.54.0, 2.55 and 2.56) runs hooks declared as `hook.<name>.event` and `hook.<name>.command`. Git 2.52 does not, which I confirmed locally: a failing config hook was ignored. The Windows host and receipts use Git for Windows 2.55/2.56.

**Reproduction:** with Git 2.54 or later on PATH:
1. Set `git config --global hook.guard.event pre-commit`.
2. Set `hook.guard.command 'exit 1'`.
3. Stage a change and run `gwz commit -m x`.

The native route commits; `git commit` refuses. The same gap applies to `commit-msg`, `post-commit`, `post-index-change`, and `reference-transaction` for `gwz tag`.

**Impact:** a policy hook is silently bypassed, for example a pre-commit secret scanner or a commit-msg policy. The result is a commit Git would have rejected or rewritten.

**Required correction:** route to the CLI when any `hook.*` key is present (`names_matching("^hook\\.")` non-empty, or at least any `hook.*.event` naming a commit or tag hook). Add the rule to §6's list.

**Closure test:** a route test with `hook.x.event=pre-commit` and `hook.x.command=…` in the repository config and in the global config, asserting `Route::Cli(Hook…)` for commit, and for tag with `reference-transaction`. A real-git differential row where Git is 2.54 or later.

### P2-3. Native `commit -a` stages the deletion of every tracked file under a directory it cannot read

**Location:** `native_commit/staging.rs:102-125`. The refused set at `:108-111` and the `WT_DELETED` arm at `:123-125`.

**Violated invariant:**
- §6: "a type change or unreadable path" goes to Git.
- The native result must equal Git's.
- Fail-closed: a defect may lose progress, never invent a change.

**Mechanism:**
- libgit2's workdir iterator turns any `opendir` error into "directory absent" (`iterator.c:1380-1385`, "Any error here is equivalent to the dir not existing").
- The diff then skips the directory (`diff_generate.c:1128-1137`).
- The directory's index entries come out as `WT_DELETED`, which become `Staging::Remove`.
- Git's `commit -a` checks each index entry with `lstat` and keeps them.

**Reproduction:**
1. `mkdir d; echo f > d/f; echo t > top; git add -A; git commit -m init`
2. `echo changed > top; chmod 300 d`
3. Run `gwz commit -a -m x`.

Git (verified, 2.52) commits `top` and keeps `d/f`. Native records the deletion of `d/f` and removes it from the index.

**Impact:**
- The commit deletes tracked files the user did not delete, and the commit may then be pushed.
- The index no longer lists the files, so they appear untracked.
- A realistic trigger is a directory left root-owned or with mode 0700 by a container bind mount.

**Required correction:** before planning `-a`, confirm with `symlink_metadata` that each `WT_DELETED` path is really absent (`ENOENT`/`ENOTDIR`). Any other result, or any unreadable directory met during status, refuses with `CliReason::AllCommit("unreadable path")`.

**Closure test:** a Unix row with a tracked directory at mode 300 and at mode 000, asserting `Route::Cli` (or the same tree as Git's) and the index unchanged for `d/f`.

### P2-4. Native `commit -a` does not hold `index.lock` across read, modify and write, so it writes a stale cached index over concurrent changes

**Location:** `native_commit/exec.rs:56-68` (`repo.index()` returns the repository's cached index, `repository.c:1650-1680`; the lock is only an existence check), `:69-75`, `:78-80` and `:106-109` (the index is written after the ref moved, through `git_index_write` with no staleness check, `index.c:817-831`).

**Violated invariant:**
- Concurrent git work is never silently lost.
- Git's `commit -a` holds `index.lock` from refresh through the ref transaction.
- An interrupted or concurrent operation leaves a state Git would produce.

**Interleavings:**
- **Lost update:** plan_commit (status loads the index) → a concurrent `git add new` or `git rm --cached x` writes the index → exec stages onto the stale cached copy and writes it back. The concurrent staging is reverted silently.
- **Branch switch:** plan reads HEAD → `refs/heads/A` → a concurrent `git checkout B` → `move_reference` compares and swaps `refs/heads/A` (unchanged), so it succeeds → A's index is written over B's checkout. A later `git commit` on B records A's tree.
- **Lock appears after the ref move:** `index.write()` fails with `GIT_ELOCKED` after the ref moved. gwz reports failure while HEAD already has the commit.

**Impact:** silent loss of staged work, or a cross-branch index state. The window is small (plan to exec, in milliseconds), but there is no guard at all.

**Required correction:**
1. Create `index.lock` exclusively before the status read.
2. Re-read the index under the lock.
3. Plan and stage from it, and update the ref while still holding the lock, compare-and-swapping `HEAD`'s symref as well as the branch.
4. Write the index through the held lock and rename it.
5. On any failure, remove only the lock gwz created.

**Closure test:** a harness row that runs `git add` (and separately `git checkout B`) between `plan_commit` and `exec::commit`. It asserts either a refusal or that the concurrent change is preserved, and that the index matches `HEAD`'s branch.

### P2-5. On Windows, a forced SSH disposal can fail forever on a reset socket, leaving the pool in a hot loop with capacity leaked

**Location:**
- `src/git/endpoint/ssh_connection.rs:44-50`: `terminate` accepts only `NotConnected`.
- `ssh_channel.rs:240-249`: `force_dispose` returns `terminate()?` before dropping the channel and session and setting `Disposed`.
- `ssh_setup.rs:383-407`: the error restores the `Active` state.
- `ssh_pool.rs:466-472`: `Err` sets `force = true` and `progressed = true`, so the pool wakes itself and the entry is never removed.

Made reachable on Windows by steps 1.4 and 1.6 (bab81ec6).

**Violated invariant:** a forced disposal is a bounded local termination ("no peer acknowledgement"), and capacity is acknowledged after it.

**Interleaving:**
1. An Active exchange's peer, or a middlebox, sends RST.
2. The pump fails, and the entry moves to Disposing{force:false}.
3. `close()` fails with a non-WouldBlock error, so `force_dispose` runs.
4. `shutdown(SD_BOTH)` returns WSAECONNRESET or WSAECONNABORTED. Microsoft's `shutdown` documentation lists both for connection-oriented sockets. Rust maps them to `ConnectionReset` or `ConnectionAborted`.
5. The error returns, the state is restored, and the pool retries at once, forever.

On Unix the same socket gives ENOTCONN, which is accepted.

**Impact:**
- A spinning worker.
- A leaked per-host slot: with `--max-per-host 1`, later opens stall.
- `shutdown_complete()` never becomes true; the retained cleanup ends with TimedOut.

**Confidence:** medium. The code path is certain. The Windows return code rests on the documented behaviour, and no Windows row resets an Active SSH connection.

**Required correction:** `force_dispose` must drop its owners and set `Disposed` whatever `terminate` returns, and may log the error. Also treat `ConnectionReset` and `ConnectionAborted` as already terminated in `terminate`.

**Closure test:** a cross-platform unit test where `terminate` fails with `ConnectionReset`, asserting that `force_dispose` reaches `Disposed` and the pool acknowledges the capacity. A Windows row that resets an Active exchange with linger 0 and asserts the slot is reused.

### P2-6. The WinCNG key-form guard is incomplete, so libssh2's PEM parser can spin forever in a setup thread no clock can stop

**Location:** `src/git/endpoint/ssh_limits.rs:121-122` (`cng_reads`: `trim_start().starts_with("-----BEGIN RSA PRIVATE KEY-----")`). This is the only guard before `userauth_from_memory`, at `ssh_key_auth.rs:52-55`.

**Violated invariant:** the setup must return within its clocks. The code's own comment says that on any other form the library "does not return", and the guard exists to prevent exactly that.

**Mechanism** (libssh2 `pem.c:73-97`, `_libssh2_pem_parse_memory`):
- `readline_memory` always returns 0, keeps producing empty lines past the end of the buffer, and drops the 128th character of any line of 128 or more characters.
- The header and footer searches loop until an exact match.
- WinCNG then falls back to a DSA header search, which never matches.

**Reproduction:**
- **Deterministic:** the first line is `-----BEGIN RSA PRIVATE KEY----- ` (with a trailing space), followed by a valid unencrypted OpenSSH key block. `ssh_key_container::check` admits the OpenSSH block. `cng_reads` passes. libssh2 never matches the RSA header and spins.
- **Plausible:** a real traditional RSA PEM whose base64 body is on one line, for example a 3072-bit key. The dropped characters break the base64 decode, the DSA search follows, and it spins.

**Impact:** an uncancellable 100% CPU loop holding the session lock. Each retry leaks another thread. In a long-lived gwz-py runtime it persists.

**Required correction:** admit only text whose first non-blank line equals the RSA header exactly and whose matching footer exists, where the key block admitted by `check` is that RSA block, and where every line is shorter than 127 characters.

**Closure test:** `cng_reads` rows that refuse the trailing-space header with an OpenSSH body, and a single-line-body RSA PEM. A row that accepts a 64-column RSA PEM.

### P2-7. A Down key's retest that is requeued as evidence skips Down's 30-second retest pacing

**Location:**
- `src/git/endpoint/setup_retry/machine.rs:176-186` (`decide`, the Down arm) and `:324-329` (`abandoned`);
- `src/transport_host/https_endpoint/retry.rs:164-188`;
- `src/git/endpoint/placement_endpoint/completion.rs:216-229`.

**Violated invariant:** adaptive design §5.5 and D8:
- a Down key makes at most one handshake per 30 s;
- a failed retest advances `retest_at` and `failed_retests`;
- after two failed retests, arrivals finish at once.

**Sequence:**
1. The key is Down and `retest_at` has passed. `decide` says Start, and the retest flight is recorded.
2. The retest's connect is refused while another connection counts in the window (`hi ≥ 1` or `hi_s ≥ 1`). `setup_failed` gives a ruling for which `requeues(...)` is true.
3. The endpoint calls `machine.abandoned(&member)`. That removes the flight but leaves `retest_at` and `failed_retests` unchanged, because `release` acts only in Degraded.
4. On the next pass, `decide` sees no retest flight and `now >= retest_at`, so it says Start again immediately.

**Impact:**
- The server is retested about every hold or confirmation cycle instead of every 30 s.
- The give-up-after-two rule is never reached on this path.
- Parked members spend attempts through `spent(...)` instead of sharing a paced retest.

**Required correction:** when a requeued member's flight was a retest (or the state is Down), treat it as a failed retest. That means advancing `retest_at += 30 s` and incrementing `failed_retests`, via a dedicated `Machine::retest_refused`, not `abandoned`.

**Closure test:** a machine-level row that puts the key Down, starts a retest, abandons it as evidence, and asserts `decide` returns Wait until `retest_at + 30 s`. An endpoint row with a second connection counted (`hi ≥ 1`) that counts the handshakes.

### P3-1. A failed Pageant send other than a timeout is reported as "Pageant is gone"

**Location:** `src/git/endpoint/pageant_exchange/sys.rs:194-199`. Every failure of `SendMessageTimeoutW` other than ERROR_TIMEOUT becomes `NoWindow`, then `AgentFault::NotRunning`, then GONE_WORDS.

**Consequence:** with Pageant running at a higher integrity level than gwz, UIPI blocks WM_COPYDATA and the error is 5 (access denied). The user is told Pageant's "window is gone or now belongs to another program … Start Pageant". Nothing is delivered, so the direction is safe, but the words are false.

**Correction:** map ERROR_ACCESS_DENIED (and other codes while `IsWindow` still holds) to a distinct refusal, such as `Denied` or `Os(code)`, with words naming integrity or elevation.

**Test:** a unit test of the error mapping table.

### P3-2. A RESTORING step's membership is never closed, so final-attempt members can be held until the queue drains

**Location:**
- `setup_retry/limit/control.rs:355-358`: every Restore-kind start joins the current step;
- `limit/control/restore.rs:99-101`: the next step begins only when the step is empty;
- `governor.rs:115` (`holds_final_attempt`).

**Consequence:** under steady demand with poor connection reuse, the step rarely empties. N stays at the first S, and final-attempt members needing a new connection wait with their allocation clocks stopped, bounded only by the queue draining.

**Correction:** close the step at S starts; later starts belong to the next step.

**Test:** a trace row with continuous arrivals during RESTORING that asserts step progression.

### P3-3. Ns counts SSH setups still waiting locally for the 64-job budget as setups at the server

**Location:** `src/git/endpoint/ssh_pool.rs:545` (`observer.started`, called at Connect, before the job-budget place is held).

**Consequence:** under a full job budget, `lo_s`, `hi_s` and `pending` are inflated, so Ns is set too high (more drops follow) and setup holds last longer. Nothing gets stuck.

**Correction:** report `started` when the setup holds its job place.

**Test:** a row with the job budget full that asserts Ns after a conclusive drop.

### P3-4. A POST hold can be waited out on a connection that is already leased

**Location:** `src/git/endpoint/https_worker/serve.rs:140` (`wait_for_continuing` after checkout).

**Consequence:** §4.5 says the POST resumes on a fresh connection, but a hold of up to 30 s is waited out on a held lease. If keep-alive closed the connection meanwhile, the POST (which is not retried) fails the member. This is rare.

**Correction:** on a hold seen after checkout, return the lease (Reusable when nothing was sent) and check out again after the hold.

**Test:** a hold set between checkout and serve that asserts a fresh lease.

### P3-5. The device-path spelling check misses mixed-separator prefixes

**Location:** `src/git/regular_file.rs:27` and `:32` list only 4 of the 8 separator combinations for `\\.\` and `\\?\`.

**Consequence:** paths such as `/\.\pipe\x` pass the spelling check and are opened. The handle-type check still refuses them, so nothing hangs, but the open itself has already connected to a pipe server with default SQOS. This is defence in depth only, for paths that come from configuration.

**Correction:** treat each separator position as `[\\/]`.

**Test:** table rows for all mixed forms.

### P3-6. Native tag create checks for an existing tag outside the reference lock

**Location:** `native_commit/exec.rs:118-131`.

**Consequence:** a `git tag X` that lands between libgit2's existence check and its lock is overwritten, where Git refuses. The window is microseconds.

**Correction:** create the tag in a transaction with a must-not-exist check under the lock.

**Test:** a check-then-create race row.

### P3-7. Native commit takes "now" twice, for author and committer

**Location:** `native_commit/exec.rs:94-95` (two calls to `Signature::now`).

**Consequence:** across a second boundary, the author and committer times differ by one second. Git never produces that, because it caches one date per process, so this breaks "native equals Git".

**Correction:** take one `now` and build both signatures from it.

**Test:** assert the two times are equal under an injected clock.

### P3-8. `core.fsync` and `core.fsyncMethod` are neither honoured nor routed to the CLI

**Location:** `native_commit/route.rs:103` onward (`common`). libgit2 honours only `core.fsyncObjectFiles`.

**Consequence:** with `core.fsync=committed` or `all`, a power loss after a native commit can leave a ref pointing at an unsynced object, which Git's route would not.

**Correction:** route to the CLI when `core.fsync` or `core.fsyncMethod` is set to anything native does not honour.

**Test:** a route row.

## 2. Invariant analysis (attacks that failed)

**Pageant (3.5, 3.6):**
- **One send, no resend:** one `SendMessageTimeoutW` per channel request, with `SMTO_BLOCK | SMTO_ERRORONEXIT`. A channel that gave up is `Failed` for good.
- **Bound on the send:** `send_bound` is at least 1 ms, and `REQUEST_CAP` is 120 s even under `--ssh-timeout 0`.
- **Clocks during the wait:** the clocks run during the wait (`Control::wait_step`, `begin_network_wait` on the shared clock). A cancel returns within one 20 ms slice.
- **Thread lifetime:** the exchange thread owns only Arc'd values (the pin with its process handle, the SID, the request) and ends at the cap.
- **Mapping after timeout:** after a timeout the mapping is unmapped and closed without being read or wiped. That is correct: it is not quiescent, and Pageant keeps its own handle.
- **Mapping names:** names come from a random-prefix `IdSource`, created exclusively; ERROR_ALREADY_EXISTS is refused before any write. The reply is validated, and a mapping that still holds the request fails the reply-type check.
- **Ownership:** the owner is checked before and after the send, the reply is zeroed on replacement, and a pid stays unreused while its process handle is held. A gone Pageant maps to NotRunning and never falls back to another agent (`select` runs once).
- **Retry after a stall:** a Pageant stall timeout is retried by the retry machine. This is decided and documented (design §5, L20). The 120 s cap timeout maps to `(Timeout, None)`, giving Close: not retried.
- **Credential crossing sessions:** no path found by which a signature could cross sessions; there is one channel per setup.

**Agent pipe (3.3):**
- Every early return in `issue`, `read` and `write` either never started the I/O or observed its completion.
- `cancel_pending` waits on the request's own event and never trusts `CancelIoEx`. An unobserved request is forgotten, and its handle is kept.
- SQOS: std ORs in `SECURITY_SQOS_PRESENT`.
- PIPE_BUSY is waited in control slices.
- Note: the cancel-mid-read row flakes under `--test-threads=1` (run `2026-10-12-pageant`). That is a fixture fragility, not a production defect.

**Helper owner and D13:**
- The WH2 owner (`process_tree`) is unchanged since ba722da0.
- The `credential fill` environment:
  - drops `GIT_ASKPASS`, `SSH_ASKPASS` and the `GIT_DIR` family;
  - sets `GIT_TERMINAL_PROMPT=0`, `core.askPass=`, `GIT_CONFIG_GLOBAL=nul`, `NOSYSTEM`, and the view as `GIT_CONFIG_PARAMETERS`;
  - runs in the Windows directory with a ceiling.
- Executable discovery: `.exe` only, absolute `PATH` entries only, no current directory.
- No helper output reaches an error text. The Declined words carry only the host.
- An Authorization header sets `credential_offered` before send in `auth_send` and `prepare`. The hold and `NotStarted` paths reset it only when nothing was sent.
- A 407 to CONNECT returns before any helper.

**Machine proxy (4.6, 4.7, D6, D7):**
- Windows refuses to build a configuration without a captured setting.
- Every unreadable, automatic or ambiguous form refuses; nothing goes direct on a guess.
- Bypass matching is textual: a trailing dot or an IPv4-mapped address goes to the proxy, and own addresses are listed once.
- Refusal happens in the driver (`request.rs:160`) and again at the connector gate, before the setup slot and before name resolution. Redirects and pooled connections pass through the same connector.
- CONNECT carries `authorization: None`; the only setter is a test fixture.
- Capture happens once per entry (`NativeCaller::capture` or `Owned::build`), never on a poll.

**D12 and the driver record:**
- `Trust` is produced only by `HostKeyRefused`.
- Every SSH setup error passes `note_trust` first, so a record cannot be stale relative to the failure it words.
- The map is per runtime and capped at 256.
- The machine setting is recorded once per runtime build.

**D14:**
- Only -8, only once, from a single `Mark` taken before the loop.
- `has_room_since` compares the time spent with the nearer bound.
- Not retried when the open's facts show a credential offered.
- The failed session is dropped (socket shutdown, then free) before the reconnect.
- One setup attempt for the retry machine. The concurrency seen by Ns is unchanged, apart from the brief invisible overlap noted in §3.

**Adaptive machine:**
- No stuck `armed` or test slot; holds are capped at 30 s.
- `pause_until` re-polls and ends when its scope ends.
- No counter underflow; `spent()` stamps rather than adds.
- Notes are capped at 64, both per scope and for ended operations.
- No lock-order inversion found.
- `LocalWaitExpired` is bounded by the open's own allocation.
- Every CapacityDetail gwz-core builds satisfies a27817b's validation.

**Native commit:**
- **Crash points:** the order is objects, then reflog and ref, then index, which is Git's own order; every intermediate state is one Git accepts. The ref is updated by compare-and-swap under its lock.
- **In-progress state:** merge, cherry-pick, revert, rebase and bisect states, a held index lock, conflicts and intent-to-add entries all route to Git.
- **Signing:** routed to Git.

**Phase 1:**
- `socket_wait` slices are clamped; connect failure is read via SO_ERROR after the wait.
- No socket leaks.
- On Unix, `regular_file` opens with O_NONBLOCK then fstat; on Windows it checks the handle type.
- Idle disposal ignores the terminate error.

## 3. Follow-ups outside the finding bar

- **Capacity detail refused at encode:** a CapacityDetail refused by gwz-transport validation becomes `Error::Protocol` in `mux::send` and closes the whole carrier (CarrierLost for every member). This is only theoretical today: a WHATWG-legal host character such as `~` would fail `capacity_host`. Consider dropping the detail rather than the session.
- **Limit notes and redirect targets:** limit notes (`limit/notes.rs:40-62`) can name a redirect target's site in human output, whereas §5.3 hides redirect targets in Capacity reports. This is a policy question.
- **Stale long holds:** they can accumulate in `held_over` (`governor.rs:411` and `:430-433`). This only matters with overlapping operations (1.2.0).
- **D14 overlap:** the first TCP connection may still be counted by sshd while the second connects. This is not modelled by Ns and is low impact (about 1% of key exchanges, Windows only).
- **Cached helper failures:** `credentials.rs` caches any helper failure, including an allocation timeout, for the destination for the rest of the route. This predates the step; it is now active on Windows.
- **Cancel-mid-read flake:** the step 3.3 flake under single-threaded test runs should be root-caused (run `2026-10-12-pageant`).

## 4. Risks and next action

**Risks:**
- Windows-only behaviour (P2-5, P2-6, P3-1) is judged by reading and by documented Windows and libssh2 behaviour. P2-5's trigger is the one claim that most needs a dabeest row.
- No real Pageant 0.83 rows exist; that is deferred, and every Pageant claim rests on the synthetic receiver.
- P2-2 depends on Git 2.54 or later, which is the Windows platform's Git.

**Next action:** remediate P2-1 to P2-7 with the stated closure tests, plus a Windows receipt for P2-5 and P2-6. Then run a targeted re-review of those items; the P3s may land in the same revision or be recorded as follow-ups.