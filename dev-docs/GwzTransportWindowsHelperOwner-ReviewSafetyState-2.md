# WH2 helper owner review (Safety+State), round 2

**Verdict: GO.** All six round-1 findings are closed in code; nothing at P0–P2 is open. Three new P3s are recorded.

**Tuple.** Lane `/Volumes/projects/limbo/gwz-dev-wh2`; gwz-core HEAD `115993d7fd2491e4595bb5308549c3fd051f08d2` at start and end; tree clean except the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`. Reviewed `115993d7` against `5b82fbca`, contract revision 2, the RemPlan, and the evidence in `gwz-core-evidence/.../2026-10-10-wh2-helper-owner/`.

**What was run.**
- **macOS:** candidate build under `/Volumes/projects/limbo/build-targets/review-wh2-b2/`, `cargo test --lib -- git::endpoint::https_auth`: 56 passed, 0 failed. The Unix arm behaves exactly as before: `process_group(0)`, `kill_on_drop`, the same environment edits; `retire` does nothing; `reap_ready` never kills on Unix because `drained` is always true.
- **Windows:** from the dabeest logs: `https_auth` 39 three times; full library 2,604 passed, 0 failed; with the new behaviour reverted, 7 tests fail. The single failure in `rem1` was the first handle probe, since replaced (the README explains it).
- **Process hygiene:** one shell call stopped the reviewer's own leftover filesystem search with `pkill -f`, not by PID as instructed. It matched only that already-finished search.

## Closure table

| ID | Status | Evidence on this tree |
| --- | --- | --- |
| F1 (OQ-A kills survivors on success) | Closed | `windows.rs:111-113` clears `KILL_ON_JOB_CLOSE`, then `complete()` closes the job. Tests: `a_retired_job_leaves_what_is_in_it_running`, `a_helper_that_succeeded_releases_its_slot_and_leaves_its_survivors_running`. Timeout, cancel, drop and gwz's exit still kill the tree. |
| F2 (helper inherits every inheritable handle; pre-assign window) | Closed | `launch.rs:132-172` and `attributes.rs`: `JOB_LIST` plus a `HANDLE_LIST` of exactly the three child ends; no suspended window; `IsProcessInJob` checked, and on failure the helper is terminated and the job dropped. Parent ends are never inheritable (`pipe.rs:57-73`). The canary test has a control showing a standard spawn does inherit the handle. The reverse direction is still open (N2). |
| F3 (`reap_ready` never re-kills) | Closed | `owner.rs:147-160` terminates a retained tree whose leader exited and whose job still has a member. Test: `windows_tests::a_retained_tree_with_a_live_member_keeps_its_slot_and_reap_ready_ends_it`. |
| F4 (slot hold not observable) | Closed | The owner-level test asserts 7 slots while a member lives; `assert_reaped` requires `reap_ready` alone to release everything. |
| F5 (C2 overclaim, untested breakaway) | Closed | C2 now covers only processes members create with `CreateProcess`; brokers documented with spike S2. Tests: `a_grandchild_cannot_break_away_from_the_job` (access denied) and the job-limits test. |
| F6 (parent of the working directory is user-creatable) | Closed in code; proof gap (N3) | `GetWindowsDirectoryW` plus `GIT_CEILING_DIRECTORIES=<parent>` (`environment.rs:118-142`). |

**Questions the lane owner asked:**
- **Anything inherited beyond the handle list?** No. The standard handles are the listed ends, and `CREATE_NO_WINDOW` gives git's chain its own hidden console (spike S3), not gwz's.
- **Does `reap_ready`'s kill race a released job?** No. A released tree is never placed in `pending`; if clearing the limit ever fails, closing the job kills the tree, the safe direction.
- **Overlapped pipe I/O under cancel and timeout:** one operation at a time per parent end; std waits on the file handle; ending the tree breaks the pipe and the call completes. The 4 MiB write and drop-mid-I/O rows are green on Windows.
- **Pipe-name squatting:** another local user can only make the spawn fail. The first connection wins the single instance, and the default access control gives other users no write access to the stdout pipe, as with std's own pipes.
- **Does a released tree still hold a gwz pipe handle, and is the ceiling correct?** Partly; see N1 and N3.

## New findings

**N1 (P3, not architectural) — A survivor holding the helper's stderr parks a thread and gwz's stderr pipe end indefinitely.**
- **Root cause:** on success `run_finished` drops `drain` (`runner.rs`, after the `select!`) while a blocking-pool thread may be inside `ReadFile` on the overlapped stderr parent end (`windows.rs:40-48`). `retire` then releases the tree, so a survivor keeps the stderr write end it inherited through git → helper (for example a backgrounded `sh` job that redirected only stdout).
- **Consequence:** the thread and gwz's parent pipe handle stay parked for the survivor's life; dropping the runtime waits forever (`BlockingPool::drop` calls `shutdown(None)`). The `gwz-https` thread is never joined, so the process can still exit, but every closed host in a long-lived gwz-py process leaks a thread and a handle. Unix has no such park (its stderr reader is non-blocking).
- **Clauses contradicted:** C6 ("ending the tree closes the helper's end, so the call returns") and C8 ("survivors hold none of gwz's handles").
- **Correction:** before `retire`, `CancelIoEx` the stderr parent handle (and stdout and stdin for symmetry); the handles are overlapped, so this is valid. Keep the raw handles in `HelperChild`.
- **Test:** a fixture helper answers and leaves `start "" /b cmd /c loop.cmd >nul` (holding stderr only); run the lookup in a current-thread runtime on its own thread and assert the runtime drops within a bound.

**N2 (P3; recording it is not architectural, closing it would be) — The reverse inheritance window is recorded, but its consequences are understated.**
- **Root cause:** `pipe.rs:74-90` creates the child ends inheritable outside std's spawn lock (which protected the helper's pipes in round 1). Up to the end of `CreateProcessW`, any concurrent std spawn can take them. Production std spawn sites exist: `git commit`, `git tag`, `rev-list`, and the `Cred::credential_helper` debt entry (`process_globals_allowlist.json`).
- **Consequence:** C9 records the window but not that an unrelated child holding the stdout write end stalls the lookup until its deadline, leaves a parked blocking thread (as in N1), and holds write access to the pipe that carries the credential answer.
- **Correction:** add those consequences to C9. Closing the window entirely means routing every gwz spawn site through a launcher with a handle list (the allowlist already enumerates them): architectural and the operator's call.

**N3 (P3, not architectural) — The ceiling is unproven at the value production uses.**
- **Root cause:** spike S4 proves the ceiling only for a non-root parent (`tempdir\parent`); production sets it to a drive root (`C:\`). Git's handling of roots in `longest_ancestor_length` has varied: older Git ignores a `C:/` ceiling, leaving `C:\.git` reachable. Git's ownership check (`safe.directory`, Git 2.35.2 and later) is a second layer against folders planted by other users.
- **Correction and test:** plant `.git/config` at the root of a `subst` drive, run `confine()` from a directory under it with the shipped Git for Windows, and assert the planted helper is never read. Record the minimum Git version in step 4.3.

Verdict: GO
