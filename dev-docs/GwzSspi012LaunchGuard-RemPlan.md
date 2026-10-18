# gwz-sspi 0.1.2 launch guard (D31): remediation plan, round 1

Reviews: `GwzSspi012LaunchGuard-ReviewCode.md` (NO-GO: P2 2, P3 4) and `-ReviewState.md` (NO-GO: P2 2, P3 6), peer-blind, 2026-10-18. One merged patch; each reviewer re-verdicts its own findings on the revised tuple.

**Blind convergence:**
- Code P2-1 = State P3-1: the identity recheck runs before an unbounded guard wait.
- Code P3-1 = State P2-1: C9 says a gwz-py Client's operations all exclude each other. Its non-network and off-switch backends have their own gates.

| Finding | Disposition | Closure |
|---|---|---|
| C P2-1 / S P3-1: recheck before the guard | Accept | `origin.verify()?` again after `hold()` and before `InheritWindow::open`. A failure drops the hold. Keep the pre-guard verify, so a refused launch takes no guard. Update `Architecture.md:59-60`, the CHANGELOG and `Supervision.md`. Regression row: a fake Origin that verifies once then fails, with the guard recorded as taken and released, `IdentityMismatch`, and Job `ActiveProcesses == 0`. Removing the second verify turns it red. |
| S P3-2: launch after the record ended | Accept | Recheck a stop predicate after `hold()`, passing `&record.stop`. Windows row: guard blocked past the deadline, Job `ActiveProcesses` stays 0. |
| S P3-4: window reset failure | Accept | Keep the hold until the child ends are closed. Move it to cover `start` plus `drop(ends)`. |
| C P2-2: gwz-core lock moves without the pin | Accept (push order) | gwz-sspi is pushed to main first. gwz-core's commit is then amended (lane owner) so the same commit moves `.github/gwz-sspi.commit` to the pushed gwz-sspi commit. A stock `windows_lane_check.py` runs on that amended commit. The push order is gwz-core, then gwz-cli and gwz-py, then root. |
| S P2-2: receipt not bound to the committed gwz-core | Accept | A fresh receipt on the final tree. Record a code-only digest (the diff excluding dev-docs, plus per-file hashes of untracked files) beside the full one. |
| C P3-1 / S P2-1: C9 overclaim for gwz-py | Accept (doc now) | C9 and OQ-H say a Client's NETWORK operations on the transport route exclude each other. Its non-network operations (CLI-routed `git commit`, local `git tag`/`-d`), its off-switch backends, and other Clients' spawns do not. The structural fix (a NativeHost-provided backend gate) goes to the operator. |
| C P3-2: no test through `NativeHost::new` | Accept | Route a Windows gated phase, or a crate-private builder test, through the production constructor. Dropping the guard in `new` must go red. |
| C P3-3: gwz-py's unused unguarded `Supervisor` | Accept (remove) | Dead code: make the non-candidate branch carry no supervisor (`()`). Remove-dead-code-now. |
| C P3-4: D31's mechanism replaced | Accept (record) | The lane owner annotates D31 "as built: `Supervisor::with_launch_guard`, because `Options` is Copy with public fields". |
| S P3-3: SSPI Timeout wording while the gate was held | Accept (doc) | Fix the C9 and `LaunchGuard` doc wording. The caller gets `Timeout` at its deadline, and the record stays charged until the hold ends. |
| S P3-5: gated phase does not assert its windows | Accept | Assert the window count is `GATED_LAUNCHES`, and that spawn spans fall inside the windows' range. |
| S P3-6: per-Client coupling | Record | Operator item, beside D25's helper-hold item. |
| C follow-up: `#[must_use]` on `hold`, the refused-shape test for a stale `=0.1.1` | Accept | Small. |
