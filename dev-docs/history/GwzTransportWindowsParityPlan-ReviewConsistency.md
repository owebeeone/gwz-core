# GwzTransportWindowsParityPlan: Consistency review

- **Object:** `/Volumes/projects/limbo/build-scratch/windows-plan-20261008/GwzTransportWindowsParityPlan.md` (DRAFT, 703 lines)
- **sha256 at start:** `1b1d7998452485c13b9bf6220846f8e980e6567612082e93e7efdf32b4367642`
- **sha256 at end:** `1b1d7998452485c13b9bf6220846f8e980e6567612082e93e7efdf32b4367642` (unchanged)
- **Tuple verified:**
  - root `04fb8daa`;
  - gwz-core `0e21bdde`, whose tree is clean apart from the untracked bug report (not read);
  - gwz-transport `9eef731`;
  - gwz-cli `d702f53`, gwz-py `5950ba3`, gwz-sspi `364ccc7`.
- **Axis:** Consistency, single axis.

## Verdict

**NO-GO.** Five P2 findings are open (P2-1 to P2-5), and there are twelve P3s.

I pre-commit to GO on a revision that resolves P2-1 to P2-5 as specified below. The P3s should be fixed in the same revision, but none of them blocks.

## Summary

The plan's facts about the code are mostly accurate:
- Nearly all of §3's 27-row Unix-dependency table checks out verbatim at gwz-core `0e21bdde`.
- The qualification-switch counts are exact (41 sites in gwz-core `src/`, 17 files to change, 7 files in gwz-cli, 6 in gwz-py).
- The libssh2 facts check out: Pageant before the OpenSSH pipe, the `FindWindowA` probe, WinCNG without ed25519, and `ssh2`'s `AsRawSocket`.

The defects are in how the plan schedules work against the controlling documents:
- It publishes gwz-sspi before the implementation acceptance and Windows qualification that gwz-sspi's release gate requires (P2-1).
- It schedules TR1.8 policy work before TR1.8's GO (P2-2).
- It starts one step before the fixture it needs exists (P2-3).
- Phase 2 cannot satisfy TR1.8's own freeze rule as scheduled, and it omits a row amendment 2 requires (P2-4).
- It leaves S4.5's "no skipped Unix-gated tests" obligation without an owner (P2-5).

Many P3s trace to one edit: draft0 gained step 0.5 and two "State" paragraphs, and the rest of the plan (§2.1, §7, the Phase 0 review line, the §10 risks, the header tuple) was not updated to match.

## Findings

### P2-1. gwz-sspi 0.1.0 is activated and published before implementation acceptance and Windows qualification

- **Root cause:** step 5.5 is scheduled "any time after 4.8", ahead of 5.6 (TR4.7) and all of Phase 6.
- **Location:**
  - step 5.5;
  - §7.1, Phase 5 line ("5.5 any time after 4.8; 5.1..5.5 ── 5.6") and Phase 6 line;
  - OQ14's recommendation ("scheduled early (any time after 4.8)").
- **Controlling text:**
  - gwz-sspi `RELEASE.md`, "SSPI release gate": the release check refuses preparation and publication "until that guard is deliberately lifted following implementation acceptance and Windows qualification".
  - Amendment 2 §3.21 C (draft revision 7): "Step 5a is a new prerequisite of Phase 10 (1.1.0): the activation and its review (gwz-sspi's `RELEASE.md`: after implementation acceptance and Windows qualification) must be done first."
- **Evidence:**
  - TR4.7, the Windows implementation acceptance, is step 5.6, and §7.1 orders it after 5.5.
  - Windows qualification (TR8.4, S5.5's dabeest repeat, S5.6's Windows column, S7.3's Windows rows) is Phase 6, after Phase 5.
  - OQ14's options cover only who reviews the activation. Neither option mentions the qualification precondition.
- **Impact:**
  - It schedules an irreversible crates.io publish before the gates the crate's own release document requires.
  - If a later Phase 6 row forces a gwz-sspi change, the immutable 0.1.0 forces a 0.1.1, and the `=0.1.0` pins in gwz-cli and gwz-py change.
- **Required correction:**
  - Make 5.5 depend on 5.6 (TR4.7 GO) and on Phase 6's qualification evidence (at least 6.2 and 6.3). Alternatively, record an operator decision that names a different gate.
  - Keep confirming O1 early, since it is an operator action with no gate.
  - Reframe OQ14 so that both options keep the "after implementation acceptance and Windows qualification" precondition.

### P2-2. Step 3.2 fixes TR1.8 agent-source policy before TR1.8 has GO

- **Root cause:** the `AgentSource` seam (3.2) depends only on 1.4. Yet its content is S4.3's agent forms as TR1.8 §4 designs them.
- **Location:**
  - step 3.2 (Goal, Tests first);
  - §7.1, Phase 3 line ("1.4 ── 3.2");
  - the Phase 2 milestone ("steps 3.1, 3.3 and 3.5 onward ... consume it"), which leaves 3.2 out;
  - §10 risk 1 ("run Phase 1 and the pure parts of Phase 3 meanwhile").
- **Controlling text:**
  - Amendment 2 §3.6 (line 237): "1.1.0 S4.3's agent forms and TR4.8–TR4.10 wait on TR1.8."
  - TR1.8 design header: "No downstream Windows implementation may consume this as GO."
- **Evidence:** 3.2's tests assert Windows policy that exists only in the unfrozen TR1.8 §4:
  - "Pageant beats the snapshot's pipe";
  - "the default is `\\.\pipe\openssh-ssh-agent`";
  - "a MinGW socket path and a UNC pipe path are refused before connect, naming `SSH_AUTH_SOCK` (TR1.8 §4)".
- **Impact:**
  - Implementation consumes a NO-GO design.
  - TR1.8's dual review can still change these rules (TR1.8 §4 itself flags the pipe-identity rule as possibly unfreezable), so the step may be redone.
  - Risk 1's "pure parts of Phase 3" widens OQ1 beyond what it asks the operator.
- **Required correction:**
  - Add the 2.4 ── 3.2 edge and list 3.2 among the steps that consume TR1.8. Alternatively, split 3.2 into a policy-free seam (the enum, with Unix unchanged) and a TR1.8-gated selection.
  - Limit risk 1's mitigation to what OQ1 actually asks.
  - Related, same class: steps 4.2 and 4.3 cite TR1.8 §10 as their specification (U20, U21, 4.3's Tests first) while depending only on 4.1. State that they implement 4.1's accepted WH2 contract, not TR1.8 §10.

### P2-3. Step 3.4 is scheduled to start before the fixture its test needs exists

- **Root cause:** §7.1 gives 3.4 the single dependency 1.2. But 3.4's test signs through the named-pipe agent fixture, which step 3.3 builds, and 3.3 waits on 2.4.
- **Location:**
  - step 3.4, Tests first ("A sign through the pipe fixture on Windows completes");
  - §7.1 ("1.2 ── 3.4"; "3.4 joins as soon as 1.2 lands");
  - step 3.3, Tests first, which introduces the "in-process named-pipe agent fixture (the Windows twin of `ssh_tests/agent_fixture.rs`)".
- **Controlling text:** the operator's plan convention (foundational first; no step depends on a later one), and the plan's own §4 parallelism claim.
- **Evidence:**
  - Windows has no other agent `Channel`: `agent_socket.rs` stays Unix-only (`agent_socket.rs:1-3`).
  - So no Windows sign can run before 3.3's fixture. 3.3 depends on 2.4, the TR1.8 GO.
- **Impact:** 3.4 cannot meet its definition of done ("tests written first, failing before") in the window §7.1 gives it. The "can start" list is false for 3.4.
- **Required correction:** either move the pipe agent fixture into its own early, policy-free step that 3.4 and 3.3 both depend on, or add 3.3 ── 3.4 and remove "3.4 joins as soon as 1.2 lands".

### P2-4. Phase 2 cannot satisfy TR1.8's GO rule as scheduled, and it omits the macOS and Linux rows the amendment requires

- **Root cause:** Phase 2's row list was taken from §2.4's "open rows". That list is incomplete, and step 2.3 lets GO proceed with provisional clauses.
- **Location:** §2.4 ("Open physical rows ..."); step 2.3 (Goal, Dependency); step 2.4; Appendix A; OQ9.
- **Controlling text:**
  - TR1.8 design §11: "Before GO, all baseline rows B01–B18 and primitive rows P01–P08 in the baseline record need executed results ... then replace provisional clauses with one physically proved design."
  - Amendment 2 §3.5, TR1.8 "Evidence first": "One 1.0.17 row on macOS and one on Linux run against the loopback `Negotiate` fixture." It also says "The same challenge on macOS and Linux ... TR1.8 records, with 1.0.17 ... whether 1.0.17 authenticates on each."
  - Amendment 2 §4 (Phase 4): "TR1.8's `HOME`-unset rows and its macOS and Linux 1.0.17 rows".
- **Evidence:**
  - **B17** (macOS/Linux, PARTIAL: "Mac ... HTTPS blocked by native trust; Linux unexecuted", baseline line 305) appears in no step, not in Appendix A, and not in §2.4.
  - OQ9 does not list the macOS native-trust approval it needs (the Windows checkpoint records "Root requested Mac-only approval, no answer received").
  - **B09/P04** (proxy grammar and loopback bypass, PARTIAL) and **P02** (numeric HWND reuse, unexecuted): Appendix A assigns them to step 2.3, but 2.3's goal lists only "B11 to B16 and B18, P05, P07, P01 ... and P03". **P04** and **P06** appear in no Phase 2 step.
  - Step 2.3 says rows that cannot run are recorded unexecuted "and the design marks the clause provisional or removes the claim". §11 forbids provisional clauses at GO.
  - OQ9 recommends approving the distinct account "only if OQ10 keeps Digest in scope", and OQ10 recommends refusing Digest. Together they leave **P01** (cross-SID Pageant) unexecuted. Yet TR1.8 §4's rule "A found Pageant owned by another SID ... refuses" rests on P01.
- **Impact:**
  - Step 2.4's GO is unreachable under the design's own rule, or it is reached in breach of it.
  - A TR1.8 release obligation (B17 and its migration-note outcome) has no owner.
  - Because Phase 2 is the critical path, this blocks Phases 3 to 6.
- **Required correction:**
  - Add B17 to Phase 2, with its approval in OQ9.
  - Put B09/P04 and P02 in 2.3's goal, or give them dispositions in 2.1 (as ProofDispositions §3 does for P02).
  - Say explicitly how P04 and P06 close.
  - Either have 2.1 amend §11's GO rule, with that change in 2.4's review scope, so that an unexecuted row removes its claim rather than leaving it provisional; or drop "marks the clause provisional" from 2.3.
  - Reconcile OQ9(2) with P01.

### P2-5. S4.5's "no skipped Unix-gated tests" obligation has no owner, and the §3 inventory is incomplete

- **Root cause:** §3 presents U1 to U27 as the complete list of Unix-only gates ("So the Unix-only pieces are a short list of OS calls"). Step 0.3 seeds its ratchet with "U1 to U26". But many gates in the scope 0.3 itself names are in neither, and no step ungates them.
- **Location:** §3 table and conclusion; step 0.3 (Design: "Seed it with U1 to U26"); step 0.5's scope (HTTPS only); steps 1.6 and 5.1 (S4.5).
- **Controlling text:**
  - V110 S4.5: "A Windows run that skips the Unix-gated tests is not this step."
  - Amendment 2 §3.14, OD13: "every transport behaviour that 1.1.0 ships on macOS and Linux ships on Windows x86-64, with the same tests".
  - TR4.6: "runs every candidate test that needs no fixture".
- **Evidence (gwz-core `0e21bdde`), gates with no owner step:**
  - `transport_host/mod.rs:15-40`: `cfg(all(test, unix))` over `tests`, `driver_tests`, `close_tests`, `throughput_tests`, `fault_tests`, `command_tests`, `fetch_preflight_tests`, `message_embedding_tests`, `cancellable_tests`, `endpoint_environment_tests`, `retry_tests`, `ca_bundle_tests`, plus `:28` `ssh_helper_projection_tests`;
  - `transport_host/session.rs:37` (`wake_tests`) and `:259` (test accessors);
  - `transport_host/request.rs:446`;
  - `git/endpoint/placement_endpoint.rs:284` (`check_tests`);
  - `git/gitbackend.rs:45` (`transport_candidate_tests`);
  - `git/endpoint/mod.rs:47-62`'s non-SSH-fixture modules (`budget_wait_tests`, `job_budget_wait_tests`, `git_turns_tests`, `ssh_pump_clock_tests`, `ssh_destination_tests`). U2 nominally assigns these to step 1.1, but 1.1's goal is only `SshdFixture`;
  - one production gate: `git/gitbackend/transport_support/identity.rs:204-206`, where the selected identity is opened with `O_NONBLOCK` on Unix only, so Windows has the same special-file blocking hazard that step 1.3 fixes elsewhere. 1.3's caller list does not include it.
- **Impact:**
  - S4.5 cannot meet its exit, and OD13's "same tests" parity is unscheduled for the integrated host and placement suites.
  - Step 0.3's ratchet, seeded as specified, either fails on its first run or records these gates with no owner, which defeats its purpose.
- **Required correction:**
  - Extend §3's table, or add an appendix, so it lists every gate in 0.3's scope (and `gitbackend.rs:45`), each with an owner step.
  - Add a step, or widen 1.6 or 5.1, to ungate the non-HTTPS host, placement and gitbackend test modules.
  - Add `transport_support/identity.rs` to 1.3's callers.
  - Seed 0.3 from the complete list.

### P3-1. State left stale by the draft0 edit

- **Location and evidence** (`diff draft0 current` shows only 0.1's State, 0.2's State and step 0.5 were added):
  - The header's read-only sources (root `1b019d67`, gwz-core `2a12006f`) are not the tuple the body cites (`0e21bdde`, `1cdb9557`).
  - §2.1 says another agent is adding the leg "now" and gives the red tail as "2170 passed, 125 failed", including the 10 `retry_tests`. But 0.1 says the leg is committed, and 0.2 says the tests were fixed in `1cdb9557`, an ancestor of `0e21bdde` (`retry_tests.rs:140-142` uses `std::env::temp_dir()`).
  - 0.2's Files and Tests first still list that fix as work to do, including "The 10 `retry_tests` fail on Windows before".
  - §10 risk 10 still cites "125 test failures".
  - §7.1 still says "0.1 (in progress)".
  - The Phase 0 review covers "0.1 to 0.4" even though 0.5 says "Review. Phase.".
  - §7.1 and §7.2 omit step 0.5.
- **Impact:** an auditor or agent cannot tell what is done. Step 0.5 has no review and no slot in the dependency sketch.
- **Correction:**
  - Re-baseline the header to the tuple.
  - Make §2.1, §10 risk 10, 0.2 and §7.1 agree with 0.1's and 0.2's State.
  - Add 0.5 to the Phase 0 review, §7.1 and §7.2.

### P3-2. Citations that do not match the tuple or misstate their source

- **Location and evidence:**
  - **U16 and step 3.7, `ssh_setup.rs:437`:** line 437 is `authority: _,`. The `ConnectionAborted | Interrupted => ErrorCode::Cancelled` mapping is at `ssh_setup.rs:578`. The stale line comes from the adaptive design's F3.
  - **§3.1 "§12 test 20":** test 20 is item 20 of the adaptive design's §10.2. §12 holds F3.
  - **Steps 1.5 and §10 risk 7, `CurrentProgramCheckpoint.md:7`:** option A's lane entry is at line 14, at both root `1b019d67` and `04fb8daa`.
  - **Step 0.5's "Why":**
    - `https_pool.rs:428-431` is past the file's end (393 lines);
    - `https_endpoint.rs:570-575` is past its end (570 lines);
    - `https_worker.rs:432` is not a test gate;
    - `git/endpoint/mod.rs:64` is a closing brace.
    These lines describe the unlanded HTTPS fixed-cost port (evidence run `2026-10-08-windows-httpsfix-port`, untracked in gwz-core-evidence), not `0e21bdde`. Step 0.5 does not state that it depends on that port landing.
  - **Step 4.4, "`transport_binding.rs:227` capability projection":** line 227 is the Windows SSH refusal (`smart_transport` returning "supports only HTTPS remotes").
  - **§2.3:** lists "B11 to B16, B18" as unexecuted, but B15 is PARTIAL (baseline line 303), as Appendix A itself says.
  - **§2.4:** "its own §11 says to refresh the controlling graph". That instruction is in ProofDispositions §5. TR1.8 §11 sets the GO rule and the post-GO sequence.
  - **§2.5:** puts in quotation marks "after implementation acceptance and Windows qualification" as `RELEASE.md`'s words. `RELEASE.md` says "following ...".
  - **§2.5 and 5.5:** cite amendment 2 §3.21 as if it controlled, without saying it is revision 7, "DRAFT, not yet reviewed".
  - **Step 0.1's Goal:** restates TR4.6's ordinary job as `windows-2022`, where TR4.6 says `windows-latest`, and does not flag the change.
- **Impact:** implementers are sent to wrong lines. Step 0.5's premise is not true at the tuple.
- **Correction:**
  - Fix each citation.
  - Make 0.5 depend on the port landing, or re-cite the gates in the tuple.
  - Mark §3.21 as a draft.
  - State that pinning `windows-2022` is a deliberate refinement of TR4.6.

### P3-3. The parallelism claim is false for the first parallel set, and the hot-spot list is incomplete

- **Location:** §4 ("Steps in a phase touch different files except where the hot-spot list says otherwise"); §7.1's "can start today" list; the Phase 4 milestone.
- **Evidence:**
  - Steps 1.2 and 1.3, both "start today", both rewrite `ssh_network.rs`. 1.2 drops its module-level `cfg(unix)` arm (U3 to U6). 1.3 replaces `read_regular` at `:133-157`.
  - Other shared files missing from the hot-spot list:
    - `ssh_setup.rs` (1.5, 3.2, 3.7);
    - `ssh_local.rs` (1.4, 3.2, 3.6);
    - `transport_binding.rs:227` (1.6, 4.4);
    - `endpoint_environment.rs` and `transport_host/mod.rs` edited by Phase 4 (4.3 at `:75-83`, 4.6, which restructures `:203-240` into `machine_proxy.rs`, and 4.7 at `:115`), while Phase 4 "does not wait on Phases 1 and 3".
  - The Phase 4 milestone's "does not wait on Phases 1 and 3" contradicts 4.5's edges to 1.4 and 1.1 in §7.1.
- **Impact:** merge conflicts between lanes the plan presents as independent.
- **Correction:** add these files to the hot-spot list with an order (for example, 1.2 lands `ssh_network.rs`'s Windows arm before 1.3 rewires `read_regular`). Correct the Phase 4 milestone sentence.

### P3-4. Step 3.4's preferred outcome conflicts with step 0.3's checker and with gwz-core's Cargo.toml

- **Location:** step 3.4; step 0.3, Tests first ("a `libc::` use inside a Windows arm fails"; "an entry whose owner step is recorded done" fails); U27.
- **Evidence:**
  - 3.4 keeps `libc::malloc` on Windows if the CRT heap is shared.
  - `libc` is a `[target.'cfg(unix)'.dependencies]` dependency (`Cargo.toml:103-104`), and the candidate's extras in `prepare.py:74-86` do not add it. So the ungated `agent_auth` does not compile on Windows. 3.4's Files omit `Cargo.toml` and `prepare.py`.
  - If `libc::malloc` is kept, 0.3's ratchet flags it permanently once 3.4 is recorded done.
- **Correction:** add the Windows `libc` dependency, or a `windows-sys`/CRT allocator, to 3.4's Files. Make 0.3's rule allow an inventory entry whose CRT-sharing proof is recorded, or make 3.4 use a non-`libc` allocator.

### P3-5. Step 4.8 misstates TR4.10's review axes

- **Location:** step 4.8, Review ("**Dual** (TR4.10's own review; secrets). Safety's list names ...").
- **Controlling text:** amendment 2 TR4.10: "its own dual Code and State review". TR4.7 waits on "TR4.10 with its own review's GO".
- **Impact:** if a Consistency and Safety pair is run under §4's tier definition, it is not the review TR4.7's precondition names.
- **Correction:** state that the review is Code plus State, and carry the hazard and SSPI-ownership items as that review's attack list.

### P3-6. Review tiers labeled without the basis the ruling requires

- **Location:** the Phase 0 and Phase 1 review lines ("skim"); steps 3.5, 4.2 and 4.3.
- **Evidence:**
  - GwzProcessOptimization §8 sets one Consistency plus Safety review per phase. A skim review replaces it only on the operator's instruction (memory `review-granularity`), and no OQ asks for that. Phase 1 changes product code: the socket readiness wait, the file reader that refuses devices, and admitting SSH through the qualification boundary.
  - Step 3.5 defines a frame that a released external program (Pageant 0.83) reads. That is "anything a released client or server reads" in §8's wire-format class, yet it is reviewed at the phase only.
  - Steps 4.2 and 4.3 are WH2's process and secret boundary. The WH1 design says "WH2's secret/process boundary has mandatory dual review". Step 4.3's review is conditional ("Dual if the step carries credential bytes") and 4.2's is Phase only.
  - §8 also says "The checkpoint names these steps when a phase starts". The plan neither names these steps nor records the lane owner's reason.
- **Correction:**
  - Make "skim" conditional on an operator decision, or add an OQ asking for it.
  - Name 3.5, and 4.2 and 4.3, as per-step dual, or record why the phase review is enough.

### P3-7. Problems in the OQs' framing

- **OQ1** asks the operator something accepted text already permits. Amendment 2 §3.13's "What can start now" lists "TR1.8 and S4.2–S4.4" together, and §3.6 gates only S4.3's agent forms and TR4.8 to TR4.10. The context also omits that TR1.8 §11 lists S4.2 after GO, adding "Dependencies come from the amendment, not this list's typography."
- **OQ3** re-asks an outcome already decided: TR4.8, under OD15, requires Pageant. Its option (b) is acknowledged to need an amendment.
- **OQ4:** "Until then 3.3 ships (b) with SYSTEM admitted" contradicts 3.3's dependency on 2.4. It also contradicts TR1.8 §4 ("cannot be frozen unchanged. Record a narrow verified-service alternative or seek an amendment"): admitting any pipe server running as SYSTEM is not a narrow verified-service rule.
- **OQ7:** "Find out first: X-row for the cause of the Unicode refusal" names a row that step 2.2's X1 to X9 do not define.
- **Correction:**
  - Turn OQ1 into a citation and confirm only the residual scope.
  - Turn OQ3 into a scheduling note.
  - Drop OQ4's interim, or make it conditional on the P03 row.
  - Add the Unicode-cause row to 2.2.

### P3-8. Unstated effect on TR2.12's candidate-switch inventories

- **Location:** steps 1.6 and 5.1 to 5.3.
- **Evidence:** `scripts/candidate_switch_inventory.txt` (70 lines) lists `gwz_transport_candidate` sites by file and symbol, including the qualification sites (for example `:46`, `capture_qualification_proxy`). gwz-cli and gwz-py each keep one too. Amendment 2 §3.13, rule (a): "the checkpoint records the files' digests."
- **Impact:** each step's source test fails, or the checkpoint's digests go stale, with no step owning the update.
- **Correction:** add the three inventory files and the checkpoint digest record to 1.6's and 5.1 to 5.3's Files.

### P3-9. The S7.3 and post-release Windows obligations are narrowed, and a dependency edge is missing

- **Location:** steps 6.4 and 6.5; §7.1, Phase 6 line; §9.
- **Evidence:**
  - S7.3 (1.1.0) applies "on each platform's consumer build". Besides the Windows bullet that 6.4 copies, it includes: CLI SSH and HTTPS in process; gwz-py SSH, HTTPS and two overlapping operations; a Rust caller with no runtime; the absence of 1.2.0 surfaces; a non-gh helper; an agent certificate key.
  - The Phase 10 post-release check also applies "on each host", including the installed CLI and one gwz-py operation. Step 6.5 lists only the dabeest-only bullet.
  - 6.3 (S5.6's Windows column) needs 6.2's parity evidence, but §7.1 has only 6.1 ── 6.3.
  - §9 omits S6.3's dabeest rows (step 5.3).
- **Correction:** list all of S7.3's rows and the post-release rows for Windows. Add 6.2 ── 6.3. Add 5.3 to §9.

### P3-10. The off switch's Windows forms and the shared home contract have no owner

- **Controlling text:**
  - Amendment 2 TR1.8: "Also: ... the off switch's three forms on Windows."
  - TR1.8 §10: "The global home resolution must share §3's Windows home contract."
  - ProofDispositions §1: cwd and path resolution "must serve SSH trust, `~/` identities and the accepted transport-setting global-config lookup consistently".
- **Evidence:** step 3.1's Files cover `known_hosts` and `~/` identities only. No step tests the flag, `GWZ_TRANSPORT` and the user-global setting on Windows; 6.2 has only "the off switch on".
- **Correction:** add the global-config lookup to 3.1. Add Windows rows for the three forms, in 3.1 or 6.2.

### P3-11. WH1's remaining gates and the committed leg's exclusions have no owner

- **Evidence:**
  - WH1 acceptance's NO-GO list names "the platform, performance, selected-source, package and aggregate release gates", and it discloses REDs it does not waive: Clippy 45, the owner-IR pin, and six candidate-leg failures. §2.2's restatement drops "selected-source", and no step owns it or the strict-Clippy item.
  - gwz-core `0e21bdde`'s `candidate-windows` job leaves three things to Linux: the byte-comparing generator and Python checks (CRLF checkouts), `test_prepare.py` (a path-escaping bug) and `publish_workflow` (an `include_str!` through the symlinked `tests/`). Yet 0.2's Tests first adds a Windows-copy-mode row to `test_prepare.py`, which the leg does not run on Windows.
- **Correction:** give these items owner steps, or explicit out-of-scope lines in §11, and make 0.2's test runnable where it is claimed to run.

### P3-12. Step 0.5 depends on Phase 4, and step 3.7 on an unimplemented class

- **Step 0.5:**
  - Its HTTPS helper-integration modules use `helper_script.rs` (U26, owned by 4.3) and process groups (U20, owned by 4.2). So 0.5 cannot finish in Phase 0 without taking over those steps' fixtures.
  - Correction: split 0.5 at the helper boundary, or add those edges.
- **Step 3.7:**
  - It "map[s] a drop to Suspect", but no `Suspect` class exists in gwz-core or gwz-transport at the tuple (no matches for `Suspect` in either source tree). The class comes from the adaptive design's limit-discovery machine, which is not a dependency of 3.7.
  - Its test ("a retried setup, not a failed member") can be met with today's Retry class.
  - Correction: map the drop to today's retriable class and leave Suspect to the adaptive machine's step, or name that step as a dependency.
- **Minor edges in the same class:**
  - 3.1 replaces 1.6's interim but lacks the 1.6 ── 3.1 edge.
  - 4.5's transport-side X6 run needs 1.6.
  - 2.2's SSH rows need OQ6 and 1.1's server spike, yet §7.1 says 2.2 "can start now".

## What I checked and found consistent

- **Operator decisions and quotes:**
  - OD13, OD15 and OD16 match amendment 2 §3.14 and the checkpoint (lines 1411-1413, 1445, 1453).
  - The amendment's line 63 quote is verbatim.
  - Every other quote checked is verbatim at its stated source, apart from the misattributions listed in P3-2. This covers "broker alone", the "not universal KEX" quote from the 2026-10-03 run's README, "no proxy credential mechanism", the "first runner's prediction" sentence, and "Negotiate selecting NTLM does not close it".
- **The §3 table, at gwz-core `0e21bdde`** (every row sampled; all correct except U16's line):
  - U1 to U3: `mod.rs:21`, `:47-62`; `ssh_network.rs:5,16-17,446`;
  - U4 to U6: `:213-216`, `:238-243`, `:321-334`, `:326`;
  - U7: `:137`;
  - U8: `ssh_key_snapshot.rs:152-172`;
  - U9: `ssh_worker/endpoint.rs:135-157`;
  - U10: `agent_socket.rs` (`Domain::UNIX`, `libc::poll`, `EISCONN`, `EINPROGRESS`), 75 lines;
  - U11: `agent_auth.rs:4,213-216,275-277`;
  - U12, U13: `ssh_key_auth.rs:19`, `ssh_local.rs:4`, `agent_job.rs:194`;
  - U14: `ssh_password_helpers.rs:30,140-143`;
  - U17 to U19: `endpoint_environment.rs:20-23,59-64,177-200,203-240`; `transport_host/mod.rs:43,85-100,87-88,138-141,188-190,222-224,280-282`; `session.rs:417-442,459-466`;
  - U20 to U22: `https_auth/owner.rs:244-255`, `lookup.rs:164-171`, `executable.rs:29-36`, `runner.rs:3`, `view.rs:3`, `view/framing.rs:4`, `file_worker.rs:5,68`;
  - U25: `ssh_fixture.rs:73-74,106-109,256,291,306,330`;
  - U26: `helper_script.rs:23,78`;
  - U27: `Cargo.toml:103-108`, `prepare.py:78,87`;
  - `agent_job/control.rs:153-190` (20 ms slice); `ssh_connection.rs:26-30,38-46`; `idle_watch.rs:66,79`; `agent_client.rs:9`.
- **1.0.17's credential callback:** `transport_support.rs:219-228,240-280` (agent once, username, helper, default; `None` passphrase).
- **libssh2-sys 0.3.3** (local registry copy): `agent.c:343` (`FindWindowA`), `:436-441` (Pageant then OpenSSH), `agent_win.c:124-139`, `wincng.h:74` (`LIBSSH2_ED25519 0`), `build.rs:97-114` (WinCNG unless `openssl-on-win32`). `ssh2-0.9.6` `session.rs:1140` (`AsRawSocket`). No repo enables `openssl-on-win32`; v1.0.17 used `git2` 0.21 with `ssh`.
- **Qualification switch:** 41 lines in gwz-core `src/`; the 17-file list in 5.1 is exact; gwz-cli's 7 and gwz-py's 6 files match 5.2 and 5.3.
- **Files the plan names:** `ssh_network.rs` 471, `ssh_setup.rs` 603 and `ssh_pool.rs` 442 lines (§11).
- **CI facts:**
  - `windows-matrix.yml` and `platform-matrix.yml` are dispatch-only, with `GWZ_TEST_GIT: real` and bash.
  - `transport-candidate.yml`'s `candidate-windows` job is as step 0.1 describes it (`windows-2022`, bash, three checks, the qualification count of at least 5, `run_tests.py --lib`, five integration targets).
  - `1cdb9557` is an ancestor of `0e21bdde`.
- **Baseline rows:** B03 to B10, B11 to B14, B16, B18, and P01 to P08 statuses in §2.3, §2.4 and Appendix A match the baseline's lines 289-329, apart from B15 (P3-2) and the omissions in P2-4.
- **Release-plan mapping:**
  - TR4.6 → 0.1 and 5.4; TR4.7 → 5.6 (Code and State, correct); TR4.8 → 3.5 and 3.6; TR4.9 → 4.6 and 4.7; TR4.10 → 4.8, with default credentials through 4.9 and Phase 6;
  - TR8.4 → 6.1 and 6.2 (targets match TR8.1 at `GwzTransportReleasePlan.md:386-389`); S5.5 and S5.6 → 6.3; S7.3 and S7.5 → 6.4 (partial, P3-9);
  - the Windows precondition and post-release dabeest rows → 6.5; S6.3's dabeest rows → 5.3; gwz-sspi O1 and O2 → 5.5;
  - TR1.8 → Phase 2, with dual review plus Surface and report names matching the baseline's §7;
  - S4.1 is executed (baseline B01 and B02), so TR1.8's "after S4.1" timing holds.
- **Option A and idle-loss fit:** §3.1 is consistent with GwzTransportIdleLossDesign §9 ("needs its own run"), §5.1 (a registration failure is lost) and §8, and with the background-close design §4 (the 1 ms park), §5 ("gone within a pass") and D5. Making step 1.5 conditional on option A's landing matches checkpoint line 14 (option A is in a lane, not landed).
- **Review tiers otherwise follow the ruling:** dual reviews at 2.4, 4.1, 4.4, 4.8, 5.5, 5.6 and 6.5; Surface at 2.4 and 6.4; evidence steps feed later reviews.

## Commands run

- `shasum -a 256` on the object, at the start and the end.
- `diff GwzTransportWindowsParityPlan.draft0.md GwzTransportWindowsParityPlan.md`.
- `git rev-parse HEAD` in root and each member; `git -C gwz-core status --short`, `diff HEAD --stat`, `log --oneline`; `git merge-base --is-ancestor 1cdb9557 0e21bdde`; `git diff --stat 1b019d67 04fb8daa`; `git show 1b019d67:dev-docs/CurrentProgramCheckpoint.md`; `git -C gwz-core show v1.0.17:Cargo.toml`; `git -C gwz-cli grep -l` and `git -C gwz-py grep -l gwz_windows_https_qualification HEAD`.
- `awk`/`sed -n` line extraction from the cited gwz-core sources, `Cargo.toml`, `tests/transport_backend/prepare.py`, `.github/workflows/{transport-candidate,windows-matrix,platform-matrix}.yml` and `scripts/candidate_switch_inventory.txt`.
- `grep -rn` over gwz-core `src/` for `cfg(unix)`, `cfg(all(test, unix…))`, `target_os`, `std::os::unix`, `libc::`, `ConnectionAborted`, `Suspect` and `gwz_windows_https_qualification`; over gwz-transport `src/` for `Suspect`; `wc -l` on the cited files.
- `sed -n`/`grep -n` on the controlling documents:
  - GwzTransportReleasePlan.md and GwzTransportReleasePlanAmendment-2.md (§2, §3.5 to §3.21, §4);
  - GwzV110Plan.md (§2, S4 to S7);
  - GwzTransportWindowsParityDesign.md (header, §2 to §5, §10, §11), GwzTransportWindowsBaseline.md, GwzTransportWindowsCheckpoint.md, GwzTransportWindowsProofDispositions-DRAFT.md, GwzTransportWindowsSspiWorkerFeasibility.md, GwzTransportWindowsAuthAlternativeFeasibility.md;
  - GwzWindowsHttpsIntegrationDesign-DRAFT.md, GwzWindowsHttpsIntegrationImplementationAcceptance.md, GwzWindowsHttpsIntegrationHandoff-2026-10-04.md;
  - CurrentProgramCheckpoint.md; GwzProcessOptimization.md §8;
  - GwzTransportAdaptiveConcurrencyDesign.md, GwzTransportIdleLossDesign.md, GwzTransportSshBackgroundCloseDesign.md;
  - gwz-sspi RELEASE.md and README.md;
  - the memory file `review-granularity.md`.
- Read-only inspection of the local cargo registry sources `libssh2-sys-0.3.3` and `ssh2-0.9.6`, and of `git2-rs/libgit2-sys/Cargo.toml`.
- `ls` and `git status`/`log` in gwz-core-evidence, to check that the cited 2026-10-08 runs exist.
- Nothing was written, built, fetched over the network or run on dabeest.
