# GwzTransportWindowsParityPlan: Consistency review, round 3

- **Object:** `/Volumes/projects/limbo/build-scratch/windows-plan-20261008/GwzTransportWindowsParityPlan.md`, revision 3 (956 lines). The remediation plan is `GwzTransportWindowsParityPlan-RemPlan.md`, §8.
- **sha256 at start:** `b3d0659e4ff0445de9a1634f58bd986681e8e6bd4cf94a3b8b76ee8a0e2cc883`
- **sha256 at end:** `b3d0659e4ff0445de9a1634f58bd986681e8e6bd4cf94a3b8b76ee8a0e2cc883` (unchanged)
- **Tuple verified at start and end:** root `04fb8daa`, gwz-core `0e21bdde` (clean apart from the untracked bug report, which I did not read), gwz-transport `9eef731`.
- **Axis:** Consistency. This round re-checks N2-1 and N3-1 and attacks the range revision 3 changed. This was the last remediation round allowed.

## 1. Closure of the open findings

| Finding | Status | Evidence at the tuple |
|---|---|---|
| **N2-1 (a).** `ssh_setup` needs `idle_watch` | **Closed** | `idle_watch` now compiles in 1.4 with `ssh_password` and `ssh_setup`, and 1.5 keeps the idle and close behaviour. U1, U15, G29 and 1.4's Goal agree. `idle_watch.rs` imports only `std` and `tokio` (`:5`, `:13`), and its tests import only `super` and `std`. Nothing it needs is gated later. |
| **N2-1 (b).** `ssh_tests/` files without their own gate | **Closed** | Table B.2 now has an `ssh_tests/mod.rs` table. The drafter's count is right and mine was wrong: the directory holds 33 files besides `mod.rs`, not 34. Exactly the 16 files I named have no gate of their own. I grepped their `endpoint::`/`super::` dependencies. Each needs only 1.1 (`ssh_fixture`), 1.4 (`ssh_setup`, `ssh_pool`, `ssh_worker` and the other modules that compile from 1.4) or a portable module, except `idle_loss` and `idle_loss_budget`, which use `cut_proxy` (0.5) and are now guarded until 1.5. `agent_fixture` gets 1.4's paired `cfg_if`, with 1.7 as owner, and G39 no longer calls `:6` a gate. 1.5's wording is corrected, and the edge from 0.5 (`cut_proxy`) to 1.5 is in §7.1. I accept the guard over a 0.5 → 1.4 edge, for the reason given: it keeps 1.4 off the HTTPS-port wait. Table B.1 still equals the grep exactly (76 rows, 161 lines, re-run on revision 3). A residual labeling error is N3-2. |
| **N2-1 (c).** 1.7 and 3.4 start too early | **Closed** | 1.7 now depends on 1.1 and 1.4, and 3.4 inherits 1.4 through 1.7. Both are stated in the steps' "Depends on" and in §7.1. "1.7 joins when 1.1 lands" is gone, replaced by "1.7 and 3.2a join when 1.4 lands". |
| **N2-1 (d).** `job_budget_wait_tests` | **Closed** | It moves to 1.8, with an explicit 0.5 edge for `https_fixture`. Its `/tmp` literal at `:350` moves to 1.8's list and out of 0.5's Files. The imports are verified: `ssh_setup::SetupConnector` at `:21`, plus `https_fixture`. |
| **N3-1.** Hot-spot order versus dependencies | **Closed** | The `transport_host` bullet now runs: 1.6 first; then the policy-free 3.2a and 4.3, merged when ready; then, after 2.4, 3.1, 3.2b, 4.4, 4.6 and 4.7. It names one hard constraint (3.1 before 4.6). It no longer contradicts §7.1, the Phase 2 milestone or the Phase 4 milestone. |

## 2. New findings

### N3-2 (P3, not architectural). Table B.2 calls two partly gated files "file-level gated"

- **Location:** Table B.2, the `ssh_tests/mod.rs` table, the rows `agent_client`, `agent_wait` (owner 3.3) and `local_endpoint`, `supervised` (owner 3.6). Also its closing sentence: "15 with a file-level gate".
- **Evidence:**
  - `ssh_tests/agent_client.rs` (588 lines) has a single `cfg_if! { if #[cfg(unix)] ...` at `:276-355`. The rest of the file is ungated.
  - `ssh_tests/supervised.rs` (320 lines) has two gated regions, from `:86` and `:203`. Its first 85 lines are ungated, as is the code between and after the two regions.
  - Table B.1 already shows this correctly: G38's lines are 277, 280 and 331; G55's are 87 and 204. The other 13 files in the group really are wrapped from their top line to their last brace.
  - The ungated parts contain no Unix-only calls. Every `UnixListener`, `Domain::UNIX`, `libc::` or `agent_socket::` use in `agent_client.rs`, and every `agent_fixture` use in `supervised.rs`, sits inside the gates. So nothing fails to compile.
  - But those ungated tests compile and run on Windows from 1.4, when `mod ssh_tests` leaves the block. The table attributes the whole files to 3.3 and 3.6.
- **Impact:** small. A portable test in those regions that misbehaves on Windows would fall to 1.4 without the plan naming it. Also, on Windows `supervised.rs:5` (`agent_auth, agent_socket`) and `agent_client.rs:3` (`agent_socket`) become unused imports of empty modules, and that warning is 1.4's.
- **Correction (text only):** describe both files as "partly gated (Table B.1 lines)". Note that their ungated tests run from 1.4. Count the group as "15 with their own gate, 2 of them partial".

### Changed range: no other new defect

I checked the rest of revision 3's changes against the step texts and §7.1:
- U1, U15, G29 and G39 agree with each other.
- 1.4's Files, 1.5's Files and its edge from `cut_proxy` agree.
- 1.7's and 3.4's "Depends on", §7.1, and the "can start" sentence agree.
- 1.8's Modules, Files and "Depends on" agree, and 0.5's Files no longer list `job_budget_wait_tests`.
- The changelog matches.

Moving 1.5 behind 0.5 (`cut_proxy`) lengthens Phase 1's chain only by a module of the tuple that "can start now". That is consistent with §7.1, not a defect.

## 3. Verdict

**GO.**
- N2-1 and N3-1 are closed.
- Every round-1 finding was already closed in round 2, and revision 3 reopened none.
- The only new finding is N3-2, a P3 that is not architectural. It does not block, and it can be fixed as a text edit at filing without another round.

No P0, P1 or P2 is open.

## 4. Commands run

- `shasum -a 256` on the object, at the start and the end; `wc -l`; `ls` of the review directory.
- `git rev-parse --short HEAD` in root, gwz-core and gwz-transport; `git -C gwz-core status --short`.
- `sed -n` and `grep -n` on revision 3 (U-table, §4 hot spots, steps 1.4, 1.5, 1.7, 1.8, 3.4, §7.1, Tables B.1 and B.2, the changelog) and on RemPlan §8.
- At gwz-core `0e21bdde`:
  - for each `ssh_tests/*.rs`, the line of its first `cfg(...unix...)` gate, its length and its last closing brace;
  - `grep` of the gate-less files' `crate::git::endpoint::`/`super::` dependencies;
  - `sed -n` on `idle_watch.rs`, `idle_watch/tests.rs`, `ssh_tests/{idle_loss,pooled,attachment,agent_keys,worker,agent_client,supervised}.rs`;
  - a map of the gated regions in `agent_client.rs` and `supervised.rs`.
- Re-ran the round-2 comparison of Table B.1 against the Appendix B grep output kept in this session's scratchpad. A temporary copy of the object made in the scratchpad was deleted.
- No repository file was written, nothing was built, no git state was changed, nothing was run on dabeest, and the bug report was not read.
