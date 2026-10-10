# WH2 helper owner review (Safety+State)

**Tuple.** Lane `/Volumes/projects/limbo/gwz-dev-wh2`; gwz-core HEAD `5b82fbca57b6e71278b21a7f86492501df489e87` at start and end; tree clean except the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`. Reviewed `10c078ce` (contract), `0c56fdb1` (owner) and `5b82fbca` (inventory rows), each against its first parent.

**What was run.** macOS candidate built in the scratchpad (`prepare.py`, `--cfg gwz_transport_candidate`), `cargo test --lib -- git::endpoint::https_auth`: 52 passed, 0 failed. Windows not run; the dabeest logs in `scratch/wh2-evidence/` show the red run failing 6 rows as expected, the green run passing 27, and 15 of 15 repeat runs. The one failure in `12-mac-transport-leg-real-git.txt` is an unmodified Unix test with a 150 ms budget whose heartbeat file was not written in time under load: a load flake, not a regression.

**Unix arm regression check: none found.** `spawn` still calls `process_group(0)`; `kill` is the same `killpg`; `retire` does nothing and `drained` always answers true, so `confirm_drained` never kills or waits on Unix; `terminate`, `Drop`, `reap_pending` and `reap_ready` behave as before. A hazard predating this change is unchanged: `terminate` calls `killpg` after the leader may already have been reaped.

## Findings

### F1 (P2) — OQ-A's "terminate on success" kills a browser the credential manager started
- **Root cause:** on success `ProcessTree::retire` calls `TerminateJobObject` (`process_tree.rs:162-166`, `owner.rs:228-244`). The contract's case for this (`Contract:48`) assumes Git for Windows helpers never leave a process running.
- **Violated:** C8 and OQ-A rest on a false premise; parity with Unix (which leaves the group alone) and with git itself (which does not contain helpers) is broken.
- **Sequence:** `git credential fill` runs `git-credential-manager`; for the GitHub/GitLab/Bitbucket OAuth flow it opens a browser through a shell launch; if the browser was not already running, the new browser main process is created inside the job (the fixture's own `Start-Process` descendant shows shell launches inherit the job); GCM gets the token and exits, then git exits; `retire` terminates the job and the user's browser dies with every window and tab. The same browser also dies on timeout or cancel, and when gwz exits (kill-on-close).
- **Impact:** a user application the user is now working in is killed without warning, losing unsaved browser state. Unix never does this on success.
- **Correction:** re-decide OQ-A with this case on the table. The parity option: on success, clear `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` with `SetInformationJobObject`, then close the job. Option (b) is safe only once F2 is fixed: otherwise a surviving process keeps gwz's inherited standard handles open, the classic hang credential-cache avoids by closing its descriptors.
- **Test:** a Windows fixture helper answers, then starts a long-lived process through `Start-Process`; assert the chosen policy for that descendant on the success boundary. Today `completed_leader_boundary(Success)` asserts nothing about the descendant on either platform, although C8 cites it.
- **Architectural:** yes (the contract decision).

### F2 (P2) — OQ-B's premise is false: the helper inherits every inheritable handle in the process
- **Root cause:** `process_tree.rs:96-108` spawns through std, which calls `CreateProcessW` with `bInheritHandles=TRUE` and no `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`. std's spawn lock only stops std-spawned children inheriting each other's pipes; it does nothing about inheritable handles created outside std.
- **Contract claims otherwise:** `Contract:32`, `Contract:49` ("gwz opens no other inheritable handle in the transport"), `process_tree.rs:22-25`.
- **Violated:** WH2's "allowlisted inherited pipe handles" (`GwzWindowsHttpsIntegrationDesign-DRAFT.md:211`, plan step 4.1).
- **Sequence:**
  1. The candidate links gwz-sspi (`prepare.py:92`).
  2. Its `create_in_job` (`gwz-sspi/src/supervisor/windows/launch.rs:32-47, 138-221`) makes both pipe pairs inheritable through `CreatePipe`. The parent ends are inheritable until `noninherit` runs; the worker's ends and its `NUL` stderr stay inheritable until dropped after its `CreateProcessW` (which includes `roots()`, the attribute list and `origin.verify()`).
  3. A credential-helper spawn on another runtime thread in that window inherits the SSPI worker's stdin read end and stdout write end; in the earlier window it can also inherit the supervisor's ends, which carry the SSPI request and token frames.
  4. Long-lived inheritable handles leak the same way: gwz's own redirected standard handles, and in gwz-py the inheritable duplicates Python's `subprocess.Popen` creates for its own children.
- **Impact:** SSPI authentication frames become readable or writable by the helper's process tree for its whole lifetime; the SSPI reader may not see EOF when its worker dies, delaying cleanup; host pipes can hang (a Python `communicate()` waits until the helper exits). The pre-assign window also leaves a suspended orphan holding these handles if gwz dies between `spawn` and `assign`, because the job is attached after creation, not at creation as the design asks.
- **Correction:** take OQ-B (b): a private `CreateProcessW` with `HANDLE_LIST` plus `JOB_LIST`. gwz-sspi's `attributes.rs` and `launch.rs` already contain this primitive and can be the template; it would not be a second SSPI owner. At minimum, correct the contract's premise.
- **Test (Windows):** create an inheritable event or pipe handle in the test process, run a helper that reports its handle count or probes the handle value, and assert it is not inherited.
- **Architectural:** yes.

### F3 (P3) — `reap_ready` never ends a retained tree that has not emptied
- **Root cause:** `owner.rs:141-151`. `HelperJob::Drop` (`owner.rs:273-296`) terminates the job once and hands it to the owner; `reap_ready`, which runs before every slot acquisition (`lookup.rs:70`), only checks `drained` and never terminates again.
- **Violated:** C3's "re-terminating each pass"; C5.
- **Sequence:** a lookup future is dropped while a member is creating a process; the new process joins the job after the single `TerminateJobObject` and survives; it holds one of the host's 8 slots until `reap_pending` (shutdown) or the owner is dropped.
- **Impact:** live helper code keeps running after its lookup ended, and host capacity leaks for a long-lived endpoint.
- **Correction:** in `reap_ready`, call `tree.kill()` on any retained entry whose leader has exited and whose tree is not drained (the kill is idempotent).
- **Test:** retain a `PendingChild` whose tree has a live member that was never killed, call `reap_ready`, and assert the member is ended.
- **Architectural:** no.

### F4 (P3) — The C5 tests cannot see early permit release
- **Root cause:** in every `process_tests.rs` row and in `assert_reaped` (`runner/process_tests.rs:30-58`), the `Runner` holds its own `Arc` of the permits until `drop(runner)`, and `assert_reaped` calls `reap_pending`, which terminates again and hides F3.
- **Violated:** C5 names `dropping_a_lookup_mid_io_...` and `assert_reaped` as coverage.
- **Impact:** a `Drop` or `retire` that released permits before the tree drained would still pass every test.
- **Correction and test:** drop the runner's `Arc` before the job's fate is decided; assert `available() == 7` while a live descendant exists; assert `reap_ready`, not only `reap_pending`, releases the slot.
- **Architectural:** no.

### F5 (P3) — C2 overclaims containment and is not tested
- **Root cause:** `process_tree.rs:16-18` ("no process can leave it") and `Contract:16`. Broker-started processes are created outside the job: WMI `Win32_Process.Create`, Task Scheduler, COM LocalServer, services, packaged or `DelegateExecute` shell activation. No test tries `CREATE_BREAKAWAY_FROM_JOB` from a grandchild, or nests under an outer job with `SILENT_BREAKAWAY_OK`.
- **Impact:** reviewers and operators rely on a containment guarantee broader than the Job Object provides, in a contract that governs credentials.
- **Correction:** scope the claim to processes created from members through `CreateProcess`.
- **Tests:** a grandchild using the breakaway flag must fail and stay in the job; a test under an outer job with `SILENT_BREAKAWAY_OK` must still be contained.
- **Architectural:** no.

### F6 (P3) — The Windows working directory's parent is creatable by any user
- **Root cause:** `runner/environment.rs:48-56`. The helper's working directory is the snapshot's `SystemRoot`, so git's repository discovery from `C:\Windows` reaches `C:\`, where Authenticated Users can create folders. Unix uses `/`, which has no parent. A repository config in `C:\.git` can name a `credential.helper`.
- **Violated:** OQ-E's "neutral directory" and the Unix guarantee it mirrors.
- **Impact:** Git 2.35.2 and later treats another user's `C:\.git` as not a repository, so impact is limited to older Git or directory-ownership edge cases. A defence-in-depth gap.
- **Correction:** set `GIT_CEILING_DIRECTORIES` to the drive root (or the working directory's parent), or let step 4.3 choose a directory with no user-creatable ancestor.
- **Test:** plant `.git/config` in the working directory's parent with a marker helper and assert the marker never runs.
- **Architectural:** no.

## OQ-A to OQ-G: is the recommended answer safe?

| OQ | Safe? | Why |
|---|---|---|
| A (terminate on success) | **No** | F1; decide together with B. |
| B (accept std's inheritance) | **No** | The premise is false (F2) and departs from the design's "allowlisted" requirement; recommend (b). |
| C (accept tokio's copies) | Yes, if recorded honestly | The record must say the stdout copy holds up to 16 KiB of the answer (password included) and is freed without wiping; stdin copies up to 2 MiB of the request (no secret: gwz sends only `fill`); stderr copies 4 KiB. Every pending call still owns its buffer. |
| D (`CREATE_NO_WINDOW`, `DIE_ON_UNHANDLED_EXCEPTION`) | Yes | The helper gets its own hidden console, so Ctrl+C on gwz's console does not reach it, and gwz's exit closes the job and ends the tree. |
| E (`SystemRoot`, refuse if absent) | Yes, for the fail-closed refusal | Add F6's ceiling. |
| F (refuse when nesting is denied) | Yes | The child is still suspended and never resumed, then terminated, so no helper instruction runs. Untested: dabeest cannot produce a denial. |
| G (no spawn-time size bound) | Yes | The view's own 4 MiB limit holds. |

Also checked, no defect: the windows between create-suspended and assign and between assign and resume (beyond F2's orphan case); nested jobs (the immediate job forbids breakaway); the 4 MiB write to a silent helper and the stderr flood, which end through tree termination; case-insensitive environment filtering (std's own environment keys also compare without case on Windows); the failed-spawn path, which never resumes the child.

Verdict: NO-GO
