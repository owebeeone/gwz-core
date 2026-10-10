# WH2 helper owner: remediation plan (round 1)

Object: gwz-core `10c078ce` (4.1 contract), `0c56fdb1` (4.2 owner), `5b82fbca` (rows). Reviews: `GwzTransportWindowsHelperOwner-ReviewCodeConsistency.md` (NO-GO) and `GwzTransportWindowsHelperOwner-ReviewSafetyState.md` (NO-GO).

**Blind convergence.** Both reviewers independently found the following, so treat them as the highest-confidence findings:
- OQ-A's "terminate on success" kills a browser that Git Credential Manager starts for OAuth (Code F2, Safety F1).
- C2 overclaims Job Object containment against broker-created processes (Code F3, Safety F5).

## Operator decisions needed first (architectural)

- **OQ-A, what happens to a helper's surviving descendants on success.** Recommended: release, not terminate. On success, clear `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and close the job, which is parity with Unix, git and 1.0.17. On timeout, cancel and gwz exit, the tree is still killed. This is safe only together with OQ-B (b), so that a survivor holds no gwz handle.
- **OQ-B, handle inheritance.** Recommended: (b), a private `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (only the three pipe ends) and `PROC_THREAD_ATTRIBUTE_JOB_LIST`. That puts the helper in the job at creation, so the suspended pre-assign window goes. The template is gwz-sspi's `attributes.rs` and `launch.rs`.

## Disposition

| Finding | Disposition | Closure test |
| --- | --- | --- |
| Code F2 / Safety F1 | Run the 4.1 spikes on dabeest first: `git credential fill` through Git for Windows with a `!sh` helper that spawns a descendant; a GCM browser flow with no browser running; `CREATE_NO_WINDOW` on `git.exe` and its children. Then implement the OQ-A decision. | A Windows fixture helper answers, then `Start-Process` starts a long-lived process; assert the decided policy on success, and assert the tree dies on timeout or cancel. |
| Safety F2 | Implement the OQ-B decision. | An inheritable event or pipe handle in the test process is not visible to the helper. |
| Code F1 | Fixture loops end when their directory is gone. Success arm: assert the decided C8 behaviour per platform; the test ends any survivor. Also fix the other heartbeat-loop fixture (`exec 0</dev/null …; while :` variant), which leaked 10 orphans today. | No heartbeat grows after the test. A ps check finds no orphan. |
| Code F3 / Safety F5 | A Windows test queries the job limits (exactly `KILL_ON_JOB_CLOSE \| DIE_ON_UNHANDLED_EXCEPTION`, no breakaway flags). An injected assign failure leaves no member and no process. A grandchild's `CREATE_BREAKAWAY_FROM_JOB` fails. C2 is narrowed to processes created through `CreateProcess`, with brokers named outside the contract. | Those tests. |
| Safety F3 | `reap_ready` calls `tree.kill()` on a retained entry whose leader exited and whose tree is not drained. | A retained live member is ended by `reap_ready`. |
| Safety F4 | The C5 tests drop the runner's `Arc` early and assert `reap_ready` releases the slot. | `available() == 7` while a live descendant exists. |
| Safety F6 | `GIT_CEILING_DIRECTORIES` is set to the working directory's parent on Windows. | A planted parent `.git/config` helper never runs. |
| Code F4 | Leave G10 with 4.3 and move it back in the inventory. Fix the contract's scope contradiction on the view encoding. | The parity check. |
| Code F5 | Byte-prefix comparison for the case-insensitive filter. | A lone-surrogate row. |
| Code F6 | Reword the four statements. | None (documentation). |
| Code F7 | Archive the dabeest runs in gwz-core-evidence and cite the archive path plus the commit. | `verify_archive.py`. |
| Code F8 | Fix the `prepare.py` feature comment, and record the features the production switch must carry. | None (documentation). |
| OQ-C, D, E, F, G | Keep the recommendations. Record OQ-C's buffer sizes honestly. For OQ-E, prefer `GetWindowsDirectoryW` or check the snapshot value against it. | None. |
