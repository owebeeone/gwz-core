# WH2 helper owner review (Code+Consistency)

**Tuple.** Lane `/Volumes/projects/limbo/gwz-dev-wh2`; gwz-core HEAD `5b82fbca57b6e71278b21a7f86492501df489e87` at start and end; tree clean except the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`. Reviewed `10c078ce` (contract), `0c56fdb1` (step 4.2) and `5b82fbca` (inventory rows), each against its first parent. The merge `30665a4e` does not touch `https_auth*` or `prepare.py` (checked with a diff).

**Checks run.**
- Parity guard: unported 140, paired 39, platform 26, "nothing new", exit 0.
- macOS candidate shape: `cargo +1.95.0 test --lib https_auth` with `--cfg gwz_transport_candidate`, 54 of 54 passed. The ordinary shape compiles none of `https_auth`.
- Code rules: every cfg inside `cfg_if`; no bare `#[cfg]` or `cfg_attr`; no statics or thread-locals; largest file 402 lines; fixtures only in `cfg(test)` modules.
- Windows, from the dabeest logs `02`, `03`, `04`: 27/27 passed, 15 of 15 loop runs, endpoint group 355.

**Unix move.** The production Unix behaviour is unchanged: `process_tree::spawn` is the same `process_group(0)` plus spawn; `kill` is the same `killpg(SIGKILL)`; `retire()` does nothing and `HelperJob::retire` matches the old `complete_if_exited`; `drained()` always returns true, so the new drain checks and the `confirm_drained` loop are no-ops; the `environment.rs` Unix arm reproduces the old byte compares, `/`, `/dev/null` and `from_bytes`; `PendingWorker` and `reap_ready_workers` are a verbatim move. The one Unix regression is in the tests (F1).

**Interface parity.** Both arms expose `spawn`, `kill`, `retire` and `drained` with matching signatures; the extra Windows items (`new`, `assign`, `raw`) are private. Sound.

## Findings

**F1 (P2) — The success boundary test does not test C8, and on Unix it leaks a process that never exits.**
- **Root cause:** the `GroupGuard` that plan step 4.2 lists (`runner/tests.rs:151`) was deleted with nothing to replace it on the success path. The Unix `retire()` lets go of the group (`complete()` sets `tree = None`), and the only stop assertion is skipped for `Success`.
- **Where:** `src/git/endpoint/https_auth/runner/tests.rs:182` (the comment wrongly claims the job's drop ends the descendant), `:241`, `:262`; `helper_fixture.rs:102`, `:110`.
- **Violated:** C8's test mapping; the plan 4.2 Files row; "a test that actually tests it".
- **Reproduction:** running only `normal_final_admission_survives_later_cancellation_without_refusal` leaves an orphaned `/bin/sh -c while :; do printf x >> "$HEARTBEAT"…` whose cwd is the test cwd (PID 97658, killed by PID; also PID 83242 from the full run).
- **Impact:** this Mac already held 4 such loops from this lane's 21:46–22:18 runs, each forking `/bin/sleep` 50 times a second, permanently; a plausible cause of the "load-timing failure in an existing 150 ms runner test" the commit records. On Windows the C8 termination is never asserted.
- **Correction:** make the fixture loops end on their own when the fixture's directory is gone (`while [ -d "$dir" ]` on Unix; `if not exist` in `loop.cmd`). In the `Success` arm assert the platform's C8 behaviour: on Windows the descendant has stopped; on Unix it is still alive (the OQ-A divergence) and the test then ends it.
- **Test:** the same test, followed by a check that no heartbeat grows.
- **Architectural:** no.

**F2 (P2) — The plan 4.1 spike rows with Git for Windows `git.exe` were not run, so OQ-A, OQ-D and C2 rest on `.cmd` fakes only.**
- **Where:** contract §8 and §6 OQ-A (lines 48–62). Requirements: plan step 4.1 (dabeest: "Git for Windows' `git.exe` and a fake helper that hangs, spawns a descendant, or writes without reading"); `GwzWindowsHttpsIntegrationDesign-DRAFT.md:196` ("WH2 helper/Job/path primitives must be spiked before WH2's own contract freezes").
- **Gap:** the real tree is `git.exe` → `sh.exe` (MSYS) or `git-credential-manager.exe`. MSYS spawn adds `CREATE_BREAKAWAY_FROM_JOB` when the immediate job allows breakaway, which needs to be shown not to fire. OQ-A's premise that Windows has no daemonizing helper ignores Git Credential Manager's browser OAuth: when GCM starts a browser that was not already running, the browser becomes a job member, and C8's `retire` (and kill-on-close under either OQ-A option) would kill the user's browser. 1.0.17 on Windows runs helpers through git2 with no job, so the browser survives there: a possible parity regression against the 1.1.0 target, not only against Unix.
- **Correction:** run the 4.1 spike rows on dabeest: `git credential fill` through Git for Windows with a `!sh` helper that spawns a descendant; a GCM browser flow with no browser running; `CREATE_NO_WINDOW` on `git.exe` (do its children flash a console?). Record the results in §8 and revisit OQ-A and OQ-D.
- **Test:** a dabeest row per shape.
- **Architectural:** yes, through the OQ-A decision.

**F3 (P3) — C1's refusal path and the C2 and C7 job limits are untested, and C2 claims more than a Job Object gives.**
- **Where:** `process_tree.rs:101-106` (the assign or resume failure arm); contract lines 16 and 21.
- **Gaps:** no test makes assign or resume fail; C2's test is a nesting row, not a breakaway row; C7 is "by construction"; broker-created processes (WMI `Win32_Process.Create`, Task Scheduler, COM LocalServer, a shell hand-off to an already-running instance) are outside any job.
- **Correction:** a Windows unit test that queries `JobObjectExtendedLimitInformation` and asserts exactly `KILL_ON_JOB_CLOSE | DIE_ON_UNHANDLED_EXCEPTION` (no `BREAKAWAY_OK`, no `SILENT_BREAKAWAY_OK`); a seam that injects an assign failure and asserts no member and no process; narrow C2 to "no process created by a member through `CreateProcess`", naming broker-created processes as outside the contract, as `setsid` is on Unix.
- **Architectural:** no.

**F4 (P3) — The contract contradicts itself on the view's encoding, and the inventory moves G10 from step 4.3 to 4.2 without the plan recording it.**
- **Where:** contract line 5 (the scope defers "UTF-16 versus UTF-8 for the configuration view") and line 56 ("decides none of it") against line 40 ("The view is UTF-8 on Windows"); `runner/environment.rs:61-67`; `windows_parity/4.2.json` marks G10 paired under 4.2 while plan Appendix B gives its owner as 4.3.
- **Correction:** either move the view-encoding decision into the contract's scope and record the G10 owner change in the plan's Appendix B, or leave G10 with 4.3. The decision itself (UTF-8 → UTF-16 with no replacement character) looks right.
- **Architectural:** no.

**F5 (P3) — The Windows `starts_with` doc claim is false, and a non-Unicode `GIT_CONFIG_KEY_…` name passes the filter.**
- **Where:** `runner/environment.rs:37-43`. `GIT_CONFIG_KEY_` followed by a lone surrogate starts with the ASCII prefix, but `to_str()` returns `None`, so the name is not removed. No effect on Git today (the count is removed and the suffix is not numeric).
- **Correction:** `key.as_encoded_bytes().get(..n).is_some_and(|h| h.eq_ignore_ascii_case(prefix.as_bytes()))`.
- **Test:** a lone-surrogate row on Windows in `a_prefix_matches_a_name_that_starts_with_it`.
- **Architectural:** no.

**F6 (P3) — Several contract and doc statements do not match the code.**
1. §1 defines "retired" as "tree empty"; on Unix a retired helper's tree may not be empty (OQ-A).
2. C5 (line 19) and §3 (line 26) say an undrained tree "is"/"is reported as" `CleanupPending`; on the success path `HelperJob::retire` (`owner.rs:229-245`) retains the helper silently and returns `Ok`.
3. C6 names a test that shows only that the wait is bounded; that the buffer stays owned holds by tokio's `Blocking` design.
4. The Unix `ProcessTree` doc (`process_tree.rs:31`) says the group becomes `None` "once the helper's id is gone", but nothing sets it; `HelperJob::complete` gives the protection.
- **Correction:** reword each. **Architectural:** no.

**F7 (P3) — The evidence is not durable.**
- **Where:** contract §8 (line 62) cites the lane's `scratch/wh2-evidence/`, against `EVIDENCE.md` (raw runs in `gwz-core-evidence`) and the rule to archive every dabeest round. The receipt records base `eb924c0e` (the worktree), not `0c56fdb1`.
- **Correction:** archive the runs and cite the archive path plus the commit the worktree became.

**F8 (P3) — Product code now depends on Windows features listed only for the candidate and labelled as test-fixture features.**
- **Where:** `tests/transport_backend/prepare.py:11-15`. `JobObjects`, `ToolHelp` and `Threading` now serve product code (`process_tree.rs`) but are described as test-fixture features; the production `Cargo.toml` Windows `windows-sys` list lacks them.
- **Correction:** fix the comment, and record the features the candidate-to-production switch must carry.

## Operator questions
- **OQ-A:** do not decide on the current evidence; run the GCM browser and `git.exe` spikes first (F2). (a) remains likely, with the browser case recorded.
- **OQ-B:** (a) is acceptable for 4.2, but "gwz opens no other inheritable handle" needs a source check (inheritable-handle APIs, raw-handle `Stdio`) before freeze.
- **OQ-C:** (a); record the size bound of tokio's `Blocking` buffer in TR1.8 §4's dependency-copies list.
- **OQ-D:** keep both flags, after the spike confirms `CREATE_NO_WINDOW` on `git.exe` does not open console windows for its children.
- **OQ-E:** agree 4.3 decides. The snapshot's `SystemRoot` is caller-controlled; prefer `GetWindowsDirectoryW`, or check the snapshot value against it.
- **OQ-F:** refuse; add the injected-failure test (F3), since that path has never run.
- **OQ-G:** agree, no spawn-time bound; the `ArgumentListTooLong` mapping cannot occur on Windows.

Verdict: NO-GO
