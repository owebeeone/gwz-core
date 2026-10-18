# WH2 helper owner review (Code+Consistency), round 2

**Verdict: NO-GO.** All eight round-1 findings are closed, but the rework has two new P2 defects (N1, N2), both local fixes.

**Tuple.** Lane `/Volumes/projects/limbo/gwz-dev-wh2`; gwz-core HEAD `115993d7fd2491e4595bb5308549c3fd051f08d2` at start and end; tree clean except the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`. Object: `115993d7` diffed against `5b82fbca`.

**What was run.**
- Parity guard: unported 140, paired 40, platform 26, nothing new. cfg-boundary guard: nothing new.
- Evidence archive: gwz-core-evidence `e26d260`; `verify_archive.py --tracked` passes.
- Mac: candidate tree under `/Volumes/projects/limbo/build-targets/review-wh2-a2/`, `cargo +1.95.0 test --lib https_auth` with `--cfg gwz_transport_candidate`: 58 passed, 0 failed.
- Leak check: a process listing before and after the run showed no orphaned heartbeat loop.
- Windows not run; Windows conclusions come from the code and the archived dabeest logs.

**Unix arm: unchanged in behaviour.** `spawn(&HelperCommand)` builds the same tokio `Command` (`/` as working directory, `env_clear`, the filtered snapshot, three pipes, `kill_on_drop`, `process_group(0)`). `Environment` on Unix matches the old filtering exactly (exact names, byte prefixes, `/dev/null`; `confine` does nothing). `retire()` and the new `reap_ready` branch reduce to the old behaviour because `drained()` is always true on Unix. `HelperChild` is tokio's `Child` on Unix.

## Closure of round-1 findings

| ID | State | Evidence |
| --- | --- | --- |
| F1 | Closed | Fixture loops run only while an `alive` file exists; `Fixture::drop` panics if a heartbeat keeps growing; the `runner_tests.rs` loop ends when the heartbeat file's directory is gone; the success arm asserts the C8 behaviour (`assert_alive`). No orphans after the run. |
| F2 | Closed | Spikes S1–S4 ran with Git for Windows' `git.exe` and are archived; the operator decided OQ-A as release on success. Caveat feeding N1: S1's survivor redirected both streams (`>/dev/null 2>&1`). |
| F3 | Closed | New tests `a_job_has_exactly_the_documented_limits_and_no_breakaway`, `a_grandchild_cannot_break_away_from_the_job` (access denied), `a_creation_that_fails_after_the_job_exists_runs_nothing`. C2 narrowed, brokers named outside. The `IsProcessInJob`-false arm (`launch.rs:188-193`) has no test; acceptable, since it cannot be provoked. |
| F4 | Closed | The contract's scope now decides the view encoding; both G10 rows are owned by step 4.3 in `4.3.json`. |
| F5 | Closed | `starts_with` compares `as_encoded_bytes()`, with a lone-surrogate test on Windows. |
| F6 | Closed | §1 separates retired, cleaned-up and retained; C5 and §3 report `CleanupPending` only on failure paths; C6's test wording fixed; the Unix `ProcessTree` doc corrected. |
| F7 | Closed | §8 cites the archive path; the archive has the inputs patch and both receipts. |
| F8 | Closed | `prepare.py` names the product features, the test-only features, and what the production switch must carry. |

## New findings

**N1 (P2, not architectural) — On Windows, a released survivor that holds the helper's stderr ties up a blocking-pool thread for good, and gwz's runtime cannot shut down until that survivor exits.**
- **Root cause:** on success `run` returns through `completed` and drops the stderr drain while it is still reading (`runner.rs:141, 157, 170`). On Windows that read is tokio's `Blocking` over std's synchronous read, in `WaitForSingleObject(INFINITE)` on the overlapped parent end (`process_tree/windows.rs:40-48`). Before OQ-A the tree was killed, so the read returned; with release on success, a descendant that inherited the helper's stderr keeps the write end open, and the thread and parent handle live until it writes or exits.
- **How a helper produces it:** Git hands its own stderr (our pipe) to every helper, so `sleep 300 >/dev/null &` with no `2>` is enough.
- **Impact:** one thread lost per such success. tokio's `Runtime` drop waits forever for blocking tasks, and gwz-core, gwz-cli and gwz-py call neither `shutdown_timeout` nor `shutdown_background`, so gwz exits only when the survivor does. On Unix the drain is non-blocking and simply dropped: also a parity gap.
- **Violated:** C6 ("cannot stall the lookup"; ending the tree is what makes the read return, and there is no ending on the success path) and C8.
- **Correction:** on success, `CancelIoEx` the stderr parent handle before dropping the drain (and any other pending pipe read); or drive the parent ends, already overlapped named-pipe server handles, as `tokio::net::windows::named_pipe::NamedPipeServer`, which cancels on drop.
- **Test (Windows):** a helper that answers, then starts a descendant with stdout redirected to `nul` and stderr inherited; assert the lookup returns `Ok` and that a current-thread runtime running it drops within a bound (drop on a thread, join with a timeout).

**N2 (P2, not architectural) — The environment block refuses names the session snapshot deliberately accepts.**
- **Root cause:** `environment_block` refuses any name containing `=` (`launch.rs:85`), but `EnvironmentSnapshot::checked` (`session_host/environment.rs:221`, test comment at `environment/tests.rs:178`) accepts a leading `=`, "as Windows' per-drive directories do", and `local_command.rs:61` captures `std::env::vars_os()`, which yields per-drive names such as `=C:`.
- **Impact:** once 4.3 and 4.4 pass the snapshot through, every helper start from a gwz launched under `cmd.exe` fails as `SpawnFailed`. Round 1's std `Command` and 1.0.17 pass these names through. Contract §5 states the stricter rule, so the two components of one contract disagree.
- **Reproduction:** `environment_block(&[("=C:".into(), "C:\\x".into())])` returns `Err`.
- **Correction:** refuse `=` only after the first unit, matching `checked`; or drop names starting with `=` in `Environment::snapshot`. Then align contract §5.
- **Test:** a unit test of `environment_block` with an `=C:` entry, plus a Windows spawn whose snapshot carries one.

**N3 (P3, not architectural) — C9 understates the inheritable window, and std spawns already exist in production.**
- **Where:** contract C9 (line 23) and §6; `pipe.rs:74-90` creates each child end inheritable at once.
- **Detail:** the window runs from the first `CreateNamedPipeW`, through the attribute list, until `CreateProcessW` returns: not "microseconds", since process creation alone takes milliseconds. `process_globals_allowlist.json` already lists production std spawns that inherit everything (`refs.rs`, `repository.rs`, `commit_log`, libgit2's `Cred::credential_helper`). A concurrent spawn that takes the helper's stdout write end holds back EOF until that process exits.
- **Correction:** create the child ends non-inheritable and set `HANDLE_FLAG_INHERIT` immediately before `CreateProcessW`; restate C9's window and cite these spawn sites in the open question.

## Operator question still open (C9's spawn gate)
No gate now, provided N3's narrowing is applied. The source check the contract recommends would currently fail on the allowlisted debt spawns, so tie it to their removal under RemPlan B5.

Verdict: NO-GO
