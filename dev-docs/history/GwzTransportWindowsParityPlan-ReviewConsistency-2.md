# GwzTransportWindowsParityPlan: Consistency review, round 2

- **Object:** `/Volumes/projects/limbo/build-scratch/windows-plan-20261008/GwzTransportWindowsParityPlan.md`, revision 2 (933 lines). The remediation plan is `GwzTransportWindowsParityPlan-RemPlan.md`.
- **sha256 at start:** `4d653f96c0a3d827df4e17bab2ab80d39b57eb4246ad8ad3e9efd955e813e737`
- **sha256 at end:** `4d653f96c0a3d827df4e17bab2ab80d39b57eb4246ad8ad3e9efd955e813e737` (unchanged)
- **Tuple verified at start and end:** root `04fb8daa`, gwz-core `0e21bdde` (clean apart from the untracked bug report, which I did not read), gwz-transport `9eef731`.
- **Axis:** Consistency, single axis. This round re-checks round 1's findings and attacks the changed range.

## 1. Closure of the round-1 findings

| Finding | Status | Evidence in revision 2 |
|---|---|---|
| **P2-1.** gwz-sspi published before acceptance and qualification | **Closed** | Step 5.5 depends on 5.6, 6.2 and 6.3. The only edges into 5.5 in §7.1 are `5.6 + 6.2 + 6.3 ── 5.5`, and its only edge out is to 6.5. "Any time after 4.8" is gone. §2.5 and OQ14 quote `RELEASE.md:126-128` verbatim ("following implementation acceptance and Windows qualification"; checked). Both OQ14 options keep that precondition. O1 is kept as an ungated action. |
| **P2-2.** 3.2 fixes TR1.8 policy before GO | **Closed** | Step 3.2a adds the `None`/`Path` enum with no Windows rule and depends only on 1.4. Step 3.2b carries the three TR1.8 §4 tests and depends on 2.4 and 3.2a. Risk 1 is narrowed. Steps 4.2 and 4.3 now implement 4.1's WH2 contract, and 2.4's review scope checks that it agrees with TR1.8 §10. A residual merge-order contradiction is new finding N3-1. |
| **P2-3.** 3.4 starts before its fixture | **Closed as stated** | 1.7 is a policy-free pipe fixture. 3.4 depends on 1.2 and 1.7, and 3.3 no longer gates it. I accept the drafter's choice of route. However, 1.7's own edge (1.1 only) is wrong; it is part of new finding N2-1. |
| **P2-4.** TR1.8 §11's GO rule can't be met; B17 missing | **Closed** | B17 is now step 2.3b, with OQ9(4) and Appendix A. 2.3's goal now names B09 with P04, B15, B16, B18, P01, P03, P05 and P07. 2.1 dispositions P02, P04 and P06. "Provisional" is gone from every step. OQ9 states what declining (2) removes for P01. The remaining reading of §11 (a removed and listed claim discharges an unexecuted row, with amending §11 as the fallback) is flagged in 2.4's review scope. That is the route round 1 allowed, so I accept it. |
| **P2-5.** S4.5's "no skipped Unix-gated tests" has no owner; inventory incomplete | **Closed for every gate line; see N2-1 for the block-gated modules** | I reproduced Appendix B's grep at `0e21bdde`: 183 lines in 81 files. The file set equals Table B.1 (76) plus Table B.4 (5). All 161 in-scope gate lines per file equal Table B.1's "Gate lines" column exactly, with no mismatch. Steps 0.5, 1.8 and 4.11 own the ungating, and 5.1 carries the exit measure. I accept the "fixed differently" route (new steps rather than widening 1.6 or 5.1), since round 1 allowed a new step. I accept the `identity.rs` qualification: `validate_file` checks `metadata().is_file()` at `:195-197` before it opens the file. The 1.8 modules contain no `helper_script` or agent-fixture references, and the helper boundary is 28 files, as Table B.3 says. The defect is in how Table B.2 and 1.4 treat `mod ssh_tests` and `mod.rs:21`: see N2-1. |
| P3-1. Stale state | Closed | The header tuple is `04fb8daa`/`0e21bdde`. §2.1, 0.2, risk 10 and §7.1 agree. The Phase 0 review covers 0.1 to 0.5. §7.2 lists 0.5. |
| P3-2. Citations | Closed | Re-read at the tuple: `ssh_setup.rs:575-582` (`:578`); §10.2 item 20; `CurrentProgramCheckpoint.md:13-14`; 0.5's gates (`https_pool.rs:362`, `https_endpoint.rs:93, 517, 562`, `https_worker.rs:166, 417-419`, `native.rs:406, 412`); 4.4's `mod.rs:289-331`, `:428` and `session.rs:459-489`; B15 marked partial; ProofDispositions §5 attribution; `RELEASE.md` quote; §3.21 marked draft; `windows-2022` noted as a refinement. |
| P3-3. Parallelism and hot spots | Closed (the dispute is accepted) | 1.2 merges before 1.3. The list of shared files is complete. I accept the dispute over `transport_binding.rs:227`: 4.4 no longer touches that line. A new merge-order defect is N3-1. |
| P3-4. `libc` versus 0.3's checker | Closed | 3.4's files include `Cargo.toml:103-108` and `prepare.py:74-87`. 0.3 gains a `proof` field. |
| P3-5. TR4.10's review axes | Closed | 4.8's review is Code plus State, with the TR4.10 and TR4.7 quotes. |
| P3-6. Review tiers | Closed | Skim reviews now go through OQ16. 3.5 gets a per-step dual. 4.2 and 4.3 get a joint dual (WH1 design `:287` verified). §4 names the dual steps. |
| P3-7. OQ framing | Closed | OQ1 is now a citation plus one residual. OQ3 is a scheduling note. OQ4's interim is dropped. X10 is defined. |
| P3-8. Switch inventories | Closed | The files are 70, 40 and 26 lines, matching gwz-core, gwz-cli and gwz-py `HEAD`. The digest form at `CurrentProgramCheckpoint.md:1478-1481` is verified. |
| P3-9. S7.3 and post-release rows | Closed | 6.4 has all six rows, 6.5 has all four, the 6.2 ── 6.3 edge exists, and §9 includes 5.3. |
| P3-10. Off switch and global-config home | Closed | 3.1 covers `transport_setting/global.rs:63-81` (verified: it reads only `HOME` and `XDG_CONFIG_HOME` from the snapshot). 6.2 has the three forms end to end. |
| P3-11. WH1 gates and leg exclusions | Closed ("fixed differently" accepted) | §11 lines with owners. "Selected-source" is read explicitly, with a hand-off to the lane owner (acceptance `:28` verified). `test_prepare.py` goes to 0.2 and `publish_workflow` to 5.4. |
| P3-12. 0.5 helper split; `Suspect`; edges | Closed | 0.5 is split at the helper boundary, with 4.11 taking the rest. 3.7 maps the drop to today's `Retry` verdict (`max_startups.rs:1-12` verified). The edges 1.6→3.1, 1.6→4.5 and OQ6/1.1→2.2's server rows are added. |

## 2. New findings

### N2-1 (P2, not architectural). The block-gate split puts step 1.4, step 1.7 and one 0.5 module before what they need to compile on Windows

- **Root cause:** Appendix B (Table B.1, row G29; Table B.2) and step 1.4 assign the modules behind the block gates `git/endpoint/mod.rs:21` and `:47-62` by their gate lines, not by what each module needs to compile.
- **Location:**
  - Table B.1 G29 ("1.4 (`ssh_password`, `ssh_setup`), 1.5 (`idle_watch`)");
  - Table B.2's `ssh_tests` row ("its files carry their own gates (Table B.1); 1.4 takes the module out of the block");
  - step 1.4's Files ("each file inside keeps its own `cfg(unix)` until its owner step");
  - step 1.5's Files ("`ssh_tests/idle_loss.rs`, `idle_loss_budget.rs`, `pooled.rs` ... un-gated");
  - step 1.7's "Depends on. 1.1" and §7.1 (`1.1 ── 1.7`; "1.7 joins when 1.1 lands");
  - Table B.2's `job_budget_wait_tests` → 0.5 ("can start now").
- **Violated text:**
  - §4's definition of done: "the Windows CI leg (step 0.1) stays green".
  - The plan convention that no step depends on a later one.
- **Evidence at `0e21bdde`:**
  1. **`ssh_setup` needs `idle_watch`.** `ssh_setup.rs:4` imports `idle_watch::{IdleReactor, IdleSocket}`. So 1.4 cannot compile `ssh_setup` on Windows while G29 leaves `idle_watch` Unix-only until 1.5.
  2. **Most `ssh_tests` files have no gate of their own.** 16 of the 34 files under `ssh_tests/` other than `mod.rs` have no gate: `agent_capacity`, `agent_fixture`, `agent_keys`, `attachment`, `channel`, `cleanup_capacity`, `idle_loss`, `idle_loss_budget`, `placement_endpoint`, `pool_host`, `pooled`, `pooled_remote`, `pump`, `regression`, `remote_bridge`, `worker`. When 1.4 moves `mod ssh_tests` out of the `mod.rs:47` block, all of them compile on Windows.
     - `agent_fixture.rs:6` is an unconditional `use std::os::unix::net::{UnixListener, UnixStream};`, and `ssh_tests/mod.rs:17` declares it unconditionally. The Windows build fails at 1.4. Table B.1 G39 records `:6` as if it were a gate.
     - `idle_loss.rs` and `idle_loss_budget.rs` use `cut_proxy`, which stays in the `all(test, unix)` block until 0.5. They then run 1.5's idle-loss behaviour on Windows at 1.4.
     - Step 1.5's "un-gate" of `idle_loss.rs`, `idle_loss_budget.rs` and `pooled.rs` names files with no gate to remove.
  3. **1.7's files live inside `ssh_tests`.** Its files are `ssh_tests/agent_fixture_pipe.rs`, `ssh_tests/key_fixture.rs` and `ssh_tests/mod.rs:29`, so they compile on Windows only after 1.4 moves the module. Yet §7.1 schedules 1.7 after 1.1 alone. 3.4's test (`ssh_tests/agent_auth.rs`) has the same transitive need.
  4. **`job_budget_wait_tests` needs `ssh_setup`.** It imports `ssh_setup::SetupConnector` (`job_budget_wait_tests.rs:21`), which is 1.4's. Yet Table B.2 assigns it to 0.5, which can start now.
- **Impact:**
  - Followed as written, step 1.4 turns the Windows leg red, or it must add new, uninventoried gates. That is the very failure step 0.3 exists to catch.
  - Step 1.7 cannot start where §7.1 says it can.
  - 1.4 would silently take on 1.5's idle-loss proof.
- **Required correction:**
  - (a) Move `idle_watch` with `ssh_setup` to 1.4 for compilation, and keep 1.5 for behaviour.
  - (b) Give `ssh_tests/mod.rs` its own rows in Table B.2: list the 16 gate-less files with owners, and have 1.4 add a paired `cfg_if` around `mod agent_fixture` (owner 1.7). Either gate `idle_loss` and `idle_loss_budget` until 1.5 (and 0.5's `cut_proxy`), or add the 0.5 ── 1.4 and 1.5 edges.
  - (c) Add 1.4 ── 1.7, so that 3.4 inherits it, and correct "1.7 joins when 1.1 lands".
  - (d) Move `job_budget_wait_tests` to 1.8, or add 1.4 as its edge.

### N3-1 (P3, not architectural). The hot-spot merge order puts policy-free steps behind TR1.8-gated ones

- **Location:** §4, hot spots, the `transport_host` bullet: "1.6, then 3.1, then 3.2a and 3.2b, then Phase 4's 4.3 ..., 4.4 ..., 4.6 ... and 4.7".
- **Evidence:**
  - 3.1 waits for 2.4. 3.2a "does not wait for 2.4" (step 3.2a; §7.1: "3.2a when 1.4 lands").
  - 4.3 is in the 4.1 ── 4.3 chain, which per the Phase 2 milestone and the Phase 4 milestone "wait on no TR1.8 text".
  - Merging "in this order" therefore holds 3.2a and 4.3 behind 3.1 and 3.2b, and so behind TR1.8's GO.
  - The bullet's own reason concerns only 4.6 ("gets 3.1's `ssh_home` before 4.6 moves the proxy code").
- **Impact:** the hot-spot order contradicts §7.1 and the P2-2 split, and puts policy-free work back on the critical path.
- **Required correction:** order the list by dependency: 1.6, then 3.2a and 4.3, then after 2.4: 3.1, 3.2b, 4.4, 4.6 and 4.7. Alternatively, state that policy-free steps merge as soon as they are ready and that the order binds only the TR1.8-gated ones (3.1 before 4.6).

## 3. Verdict

**NO-GO.** One new P2 is open: N2-1, which is not architectural. All five round-1 P2s and all twelve round-1 P3s are closed. N3-1 is a new P3.

I pre-commit to GO on a revision that resolves N2-1 as specified in (a) to (d). N3-1 should be fixed in the same patch, but it does not block.

## 4. What I checked and found consistent (this round)

- **Appendix B:**
  - grep reproduced at `0e21bdde`: 183 lines in 81 files;
  - the file set equals Tables B.1 and B.4;
  - per-file line sets equal Table B.1 for all 76 rows (161 lines);
  - `helper_script` appears in exactly 28 files (Table B.3);
  - the 1.8 modules have no `helper_script` or agent-fixture references.
- **New citations, read at the tuple:**
  - `transport_host/mod.rs:289-331`, `:428`;
  - `session.rs:404`, `:459-489`;
  - `ssh_setup.rs:575-582`;
  - `identity.rs:193-197`;
  - `max_startups.rs:1-12`;
  - `transport_setting/global.rs:63-81` and `transport_setting.rs:283`;
  - `ca_bundle_tests.rs:118`, `native/tests.rs:63`, `cleanup_tests.rs:181`, `qualification_tests.rs:272`, `ssh_tests/agent_auth.rs:159`;
  - the `/tmp` literals at `budget_wait_tests.rs:187`, `job_budget_wait_tests.rs:350`, `placement_endpoint_tests.rs:28, 65, 289`, `driver_tests.rs:345, 370`;
  - `GwzV110Plan.md:322-323`;
  - WH1 design `:287`;
  - WH1 acceptance `:26-37`;
  - `CurrentProgramCheckpoint.md:12-14, 1478-1481`;
  - `RELEASE.md:126-128`;
  - amendment 2 `:654`;
  - Windows checkpoint `:39`;
  - baseline `:352`;
  - inventory file sizes 70, 40 and 26.
- **§7.1:**
  - no cycle among 5.5, 5.6, 6.x and 6.5;
  - 3.2b, 3.1, 3.3 and 3.5 depend on 2.4;
  - 3.2a, 3.4 and 3.7 do not;
  - 4.11 precedes 4.9;
  - B17 (2.3b) feeds 2.4.
- **Reviews:** the per-step duals named in §4 match the step texts (2.4, 3.5, 4.1, 4.2 with 4.3, 4.4, 4.8, 5.5, 5.6, 6.5). 4.8 is Code plus State, as TR4.10 requires.

## 5. Commands run

- `shasum -a 256` on the object, at the start and the end; `wc -l` on the object and the remediation plan.
- `git rev-parse` in root, gwz-core and gwz-transport; `git -C gwz-core status --short`; `git -C <repo> show HEAD:scripts/candidate_switch_inventory.txt | wc -l` for gwz-core, gwz-cli and gwz-py.
- Appendix B's stated `grep -rnE` over gwz-core `src/`. Its output and derived file lists were written to this session's scratchpad (`/private/tmp/claude-501/.../scratchpad/`, not in any repository) and compared against the plan's tables with `sed`, `diff` and a short `python3` line-set comparison.
- `awk` and `sed -n` line extraction at the cited lines; `grep` for `helper_script`, the agent and key fixtures, `cut_proxy`, `idle_watch` and `SetupConnector` across the named test modules; `ls` of `ssh_tests/`.
- No repository file was written, nothing was built, no git state was changed, nothing was run on dabeest, and the bug report was not read.
