# Windows parity 5.6: State-axis re-verdict (round 1 remediation)

**Review object:** Windows parity plan step 5.6, after round-1 remediation. The diff since the reviewed revision is:
- gwz-core `37705303..45fe6e6a`
- gwz-cli `220501a5..cd72b954`
- gwz-py `28aff11d..ea94500a`

**Baseline:** checked at the start and the end of the review; nothing moved.

| Repository | Commit |
|---|---|
| root | 8fc5acb199cdedbe0f9374c413ebff333973f738 |
| gwz-core | 45fe6e6a233165c63e77783632d63b78f4a5b852 |
| gwz-transport | a27817b9c86fd60a07a9e77d62b1f5e94a5cf252 |
| gwz-cli | cd72b9548d3f4d29105c14d68a94f549d16fc022 |
| gwz-py | ea94500afdcfa60a163ac87514cbee153b6b1d7c |
| gwz-core-evidence | 299e986b595f14bc09f621324386b142fb81d85b |
| gwz-sspi | d3038882f7937be023d1835fe1b03e0309260acb |
| git2-rs | d13951f7e0bfb6e0efcee1207ac5b140adefa455 |

**Date:** 2026-10-14. **Axis:** State. This is an independent, adversarial, read-only review by the same reviewer as round 1, with that context intact.

**Verdict: GO.** 0 P0, 0 P1, 0 P2 open. All 7 round-1 P2s and all 8 P3s are closed. No new finding reaches P2, and 2 new P3 observations are recorded in §3.

In round 1 I pre-committed to GO on a revision that resolves P2-1 to P2-7 as specified. They are resolved as specified, and no new P0–P2 was found, so the pre-commitment applies.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1 helper view vs Git's urlmatch | Lane D: `in_effect` follows `url_normalize` and urlmatch (case, implied port, path prefix at `/`, `*` per label, `match_partial_url` fallback). The divergent assertion is removed. | **Re-traced.** `view/helpers.rs:264-286` (`Normal::is_matched_by`) and `:303-316` (`Request::is_matched_by`). The test table `helpers/tests.rs:190-512` (`GIT_SELECTS`, the Git 2.52 choices) holds all four of my round-1 counterexamples as `true`: wildcard, `Example.Test`, `:443`, and path prefix `/org` → `/org/proj/_git/r`. I re-ran 11 extra pattern/URL pairs against real git 2.52, none in the table (`*.*`, `/r`, `https:///`, user-in-URL, `.../ab/` vs `/ab`, `/org` vs `/organ`, `*.example.test` vs two labels, `%2F` in the path, `u@` vs `u:pw@`, a trailing-dot host, a partial `host:port`). Git's answer and the implementation's traced answer agree on each. `view::helpers` tests pass (10). | **Closed** |
| P2-2 config-defined hooks | Lane A: any `hook.*` key routes to the CLI. | **Re-traced.** `native_commit/route.rs:197-206` (`hook_check`) checks `names_matching("^hook\\.")` before the file check, for commit and tag. Route rows `tests/route_config.rs:210-262` cover repository and global scope, and tag with `reference-transaction`. They pass. | **Closed** |
| P2-3 `-a` stages deletions under an unreadable directory | Moot: `commit -a` always runs the CLI (operator decision 2026-10-13). | **Route gone.** `route.rs:317-319` returns `Cli(AllCommit("Git always runs it"))` when `all && !native_all`. `native_all` is true only in `RoutePolicy::NATIVE_ALL`, which is `cfg(test)` (`native_commit.rs:101-109`); production uses the default (false). Grep finds no other setter. `route_commit.rs:23` asserts the CLI route for `-a`. The native `-a` code is kept unrouted, per the decision. | **Closed (moot by decision)** |
| P2-4 `-a` without `index.lock` across read, modify, write | Moot, as for P2-3. | **Route gone**, by the same evidence. The index write in `exec.rs` happens only when `plan.staging` is non-empty, which needs `-a`, which is unreachable. Plain commit writes no index, and its compare-and-swap ref move is unchanged. | **Closed (moot by decision)** |
| P2-5 forced SSH disposal stuck on a reset socket | Lane D: `force_dispose` always ends `Disposed`, and `terminate` accepts `ConnectionReset` and `ConnectionAborted`. | **Re-traced.** `ssh_channel.rs:243-252` uses `let _ = connection.terminate();`, then drops the channel and session and sets `Disposed`. It is now infallible, so the pool's `Err` arm and the hot loop are unreachable from this cause. `ssh_connection.rs:60-75` accepts the three kinds. Closure rows: `regression::a_forced_disposal_reaches_disposed_whatever_the_socket_reports` (injected Reset, Aborted and PermissionDenied), `ssh_connection::tests` (6), and `idle_loss::a_connection_reset_in_the_middle_of_an_exchange_frees_its_slot_for_the_next_open` (a real RST through `cut_proxy`; the slot is reused). All pass on the Mac. The row says the Windows host returned success for a peer reset, so the injected rows are the discriminating evidence there. | **Closed** |
| P2-6 incomplete WinCNG key-form guard | Lane D: exact first non-blank header, matching footer, one RSA block, every line under 127 characters. | **Re-traced** against libssh2 0.3.3 `pem.c` and `wincng.c:947-981`. My deterministic counterexample (a trailing-space header over an OpenSSH body) is refused, because the first non-blank line ≠ `BEGIN`. The single-long-line body is refused because the line is over 126 characters. The DSA fallback runs only on a PEM-parse failure after the header. Footer, armor and base64 failures are now excluded: the footer is required, and `ssh_key_container::check` validates the base64 quartets. A DER-level import failure returns −1 and does not loop. `ssh_limits::tests` pass. | **Closed** |
| P2-7 Down retest pacing skipped on requeue | Lane B: `Machine::retest_refused`. | **Re-traced.** `machine.rs:330-349`: a retest flight of the current generation in Down gets `failed_retests += 1` and `retest_at = now + 30 s`; any other flight is released as before. Both requeue paths now call it: HTTPS `https_endpoint/retry.rs:183-185` and SSH `placement_endpoint/completion.rs:240-244`. My sequence (retest refused as evidence → immediate re-Start) now waits for `retest_at`. Closure rows: `setup_retry/down_tests.rs:264-295` (machine) and `https_endpoint/down_tests.rs:238ff` (endpoint). They pass. | **Closed** |
| P3-1 Pageant send failure worded as "gone" | Lane C: a non-timeout failure while `IsWindow` holds is `Rejected` (Denied words). | **Re-traced.** `pageant_exchange.rs` (`send_failure`) is called from `sys.rs:191-196`, with `GetLastError` read before `IsWindow`. The mapping table rows are in `pageant_exchange/tests.rs` and pass. Nothing is read from the mapping on any send failure, as before. | **Closed** |
| P3-2 open RESTORING step membership | Lane B: the step closes after S starts. | **Re-traced.** `restore.rs:43-62` and `:116-124` add a `quota`; `control.rs:220-243` (`step_has_starts` gating `n_room` and `has_room`) and `:365-370` (decrement on `Own::New`). No stuck state: quota reaches 0 only after a start was inserted into the step, and each step attempt reaches a result (connected, refused, or the 1 s sweep), so the step empties and the quota resets. `control_restore_tests` (11) pass. | **Closed** |
| P3-3 Ns counting setups waiting locally | Lane B: `started` is reported when the setup holds its place. | **Re-traced.** `ssh_pool.rs:411-422`: `PendingStart` is emitted when not waiting, before the Connected or failure notification of the same poll. A connection cancelled while waiting is never "started", and its later `Retired` is absorbed (`Table::apply` returns None for an unknown connection). `events_tests` (11) pass. | **Closed** |
| P3-4 POST hold waited out on a leased connection | Lane B: return the lease and check out again after the hold. | **Re-traced.** `serve.rs:66-99` (`renew_after_hold`): skipped for a native route or an existing response. Otherwise the lease is finished `Reusable` (nothing sent), the task waits on the hold or a cancel, then calls `recheckout` with the scope re-applied. `post_hold` rows (5) pass. | **Closed** |
| P3-5 mixed-separator device paths | Lane D: every separator position is `[\\/]`. | **Re-traced.** `regular_file.rs:18-60` (`after_namespace`, `in_nt_namespace`). Table rows cover all 8 spellings of `\\.\` and `\\?\` plus the NT prefix. `regular_file` tests (14) pass. | **Closed** |
| P3-6 tag existence check outside the lock | Lane A: a transaction with a must-not-exist check under the lock. | **Re-traced.** `exec.rs:165-196`: lock, then `find_reference` under the lock, then `set_target`. `differential_edges::a_tag_created_after_the_early_check_is_not_overwritten` and its `Locked`-stage sibling pass. | **Closed** |
| P3-7 two "now" readings | Lane A: one clock reading. | **Re-traced.** `exec.rs:28-47` (`signatures` takes one `now`); the injected-clock row at `differential_edges.rs:266` passes. | **Closed** |
| P3-8 `core.fsync` not routed | Lane A: route to the CLI. | **Re-traced.** `route.rs:139-143`; `route_config::core_fsync_settings_pick_the_cli` passes. | **Closed** |

## Changed-range analysis

These are the non-merge gwz-core commits in the range:
- `0d501ee6`: documents only.
- `d046c9e8`: lane D, 13 files.
- `94210d79`: D15, documents only.
- `2fc2fc42`: lane C, wording, redirect-per-open, docs.
- `8cefbe40`: lane A, native commit and tag.
- `e29801f1`: lane B, Down texts, Capacity wording, local waits, POST holds, restore, Ns start, gwz-py notes.
- `45fe6e6a`: `scripts/windows_lane_check.py` prints a refusal reason (tooling only).

Across all of it, 104 `src` files changed (+5224/−623).

**I hunted for regressions in each of these areas.**

**`force_dispose`**
- Now infallible, and both disposal paths (`poll_dispose`'s fallthrough and the pool's force) end `Disposed`.
- No caller relied on the error.
- The idle path already ignored it.

**Down and retest (`retest_refused`)**
- Every non-retest flight falls back to `release`, so nothing changes outside Down.
- Cancellation paths (`retry.rs:117`, `admission.rs:90`, `completion.rs:117`) still call `abandoned`, which is correct: there is no verdict.
- A refused retest does not sweep the parked members. That is consistent with "the site judged it", and they are re-tested 30 s on.

**End notes (`Handoff.ends`, keyed by session and stream)**
- `note_end` precedes `fail_open` or `fail` on every path (`completion.rs:271-290`, `admission.rs:197-201`, `retry.rs:193`, `:260`, `:323`, `:473`, `poll.rs:120-129`).
- Mux stream IDs are monotonic per session (`gwz-transport mux/mod.rs:356`), so a stale note can never be taken by a different open.
- The map is bounded at 256.

**Ns start reporting**
- See P3-3 above.
- A setup that fails while still waiting reports `started` once, just before its failure. That is harmless: a local Capacity failure is not suspect.

**POST re-lease**
- A hold seen after checkout no longer holds the lease.
- The relet keeps the credential scope.
- A cancel during the wait or the recheckout fails the stream terminal, with no lease leaked: the lease was finished before the wait.

**Holds (`hold.rs`)**
- `post_until` separates POST holds from discovery holds.
- `blocks_continuing` still ends when the idle connections are discarded (`sync`), so no hold can stick.

**Local waits (`Book.local_waits`)**
- Bounded at 64 entries and 10 s freshness.
- Attribution is "the most recent expired wait on the site" (see §3).

**Native commit**
- System-config detection (`system_config.rs`, Code P2-2) is filesystem-only, runs no subprocess, and fails toward the CLI on every unknown (`check`, lines 317-357).
- `include_paths` and `hook.*` also fail toward the CLI.
- No path routes `-a` natively.

**Pageant**
- The only change is error mapping. The mapping lifecycle is unchanged: never read after a failure, wiped only after a served return.

**gwz-py notes**
- Notes go to `logging.getLogger("gwz")` through `Python::attach` inside `catch_unwind`.
- Every caller of the transport route releases the GIL (`client_host.rs:89`, `:117`, `:134`, `:144`, `:275`), so a runtime thread taking the GIL for a note cannot deadlock against a waiting caller.

**gwz-cli**
- `--verbose` route reasons, docs and pins only. No state change.

**Merged overlaps** (`request.rs`, `session/driver.rs`, `driver/opening.rs`, `https_failure.rs`, `prepare.rs`, `https_endpoint.rs`)
- The lanes' edits compose. `Refusal{failure, note}` threads through `open_stream`. T5 is now per open, keyed by canonical URL in `redirect_log`. `ended` is reset on the second (Gh) open.
- I found no lost field, double take, or reordered refusal.
- `https_failure.rs` is 984 lines, within the split rule.

**Flake claims I checked:**
- **`regression::poll_dispose_completes_or_waits_without_releasing_session`** (CI run 38427769224) panics at `ssh_fixture.rs:214`, in `SshdFixture::session()`. That is setup, before any disposal call, failing on "host key did not match", which means a client reached another fixture's sshd. `free_port()` (`ssh_fixture.rs:279-285`) binds port 0 and releases it, a known race. Lane D's `force_dispose` change cannot reach that line. Lane D's new row adds three sequential fixtures to the module, which raises exposure to the pre-existing race slightly but does not cause it. **Claim supported.**
- **`idle_loss::the_retry_after_a_dead_lease_is_fresh_and_leaves_another_idle_session_alone`**: evidence `2026-10-14-fixB` (README, the line 75 table and lines 81-84) records it failing on `HEAD` = `0d501ee6`, which predates lane D. The force change only makes disposal complete sooner. **Claim supported.**
- **gwz-py `test_close_cancels_waiting_operations…`**: not on this axis's code path (the client-host close bound). Not assessed.

**Scope (common check).**
- Every production change maps to a RemPlan disposition or an operator decision of 2026-10-13 or 2026-10-14: the narrowing, the Down texts, the Capacity wording, gwz-py logging, and D15 (documents only).
- `45fe6e6a` (the lane-check script prints its refusal reason) is outside the dispositions. It is a 31-line diagnostic in a developer tool with no product effect, so it is not a scope finding.
- No lane widened product scope.
- Keeping the unrouted native `-a` code is the operator's explicit decision. It is release-scoped code kept for later, not dead code under the standing rule.

## 0. Evidence base

**Read fully:**
- the RemPlan;
- the lane D production diff (`helpers.rs`, `ssh_channel.rs`, `ssh_connection.rs`, `ssh_limits.rs`, `regular_file.rs`, `cut_proxy.rs`) and its test diff;
- lane A's `native_commit.rs`, `exec.rs`, `route.rs` and `system_config.rs` (the `check` and `derive` paths);
- lane B's `machine.rs`, `control.rs`, `restore.rs`, `hold.rs`, the governor diff, the `ssh_pool.rs` diff, `serve.rs`, `throttle.rs`, the `prepare.rs` diff, `admission.rs`, `completion.rs` and `retry.rs` call sites, and `ssh_handoff.rs`;
- `request.rs`, `driver.rs` and `driver/opening.rs`;
- the lane C Pageant mapping (`pageant_exchange.rs`, `sys.rs`);
- the gwz-py `route/transport.rs` diff and the GIL-release sites in `client_host.rs`;
- the gwz-cli diff summary;
- libssh2 0.3.3 `wincng.c:947-981`.

**Sampled:** `https_failure.rs` (the wording diff), `capacity.rs`, `end_note.rs`, `note_sink.rs`, `cancellable.rs`, and the evidence READMEs for fixd, fixB and fixB-merged.

**Not read:** gwz-cli docs, gwz-core docs other than the RemPlan, lane C's text bodies beyond Pageant, and the gwz-py Python tests.

**Commands and results:**
- `prepare.py /Volumes/projects/limbo/build-targets/review56r2-state-cand`, then a candidate build with `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/review56r2-state-target` (clean).
- Targeted rows (`GWZ_TEST_GIT=real`): `view::helpers ssh_limits ssh_connection a_forced_disposal regular_file setup_retry::down_tests control_restore_tests events_tests post_hold native_commit pageant_exchange agent_pageant pageant_words a_connection_reset_in_the_middle job_budget_wait capacity_tests https_endpoint::down_tests`. Result: **229 passed, 0 failed.** The listed counts include `native_commit` 109 and `view::helpers` 10.
- Real git 2.52: 11 extra urlmatch pairs, all agreeing with the implementation.
- Checks:
  - Windows parity: "nothing new" (unported 1).
  - Checked-artifact boundary: ok.
  - cfg boundaries: "nothing new".
  - Candidate switches: 23 sites match.
  - Process globals: "nothing new".
- I did not run the ordinary suite. The lane evidence (fixB-merged) and the stated CI runs report it passing.

## Problem, requirement provenance, and scope

These are unchanged from round 1. The decisions OD17, the native narrowing, the Capacity last-attempt rule, the Down texts, gwz-py logging and D15 are deferred outcomes. Their shape, as implemented, matches the recorded text. Scope is covered in the changed-range analysis.

## 1. Findings

No new P0, P1 or P2.

## 2. Invariant analysis (attacks that failed)

**On remediated code:**
- **`renew_after_hold`:** can cancellation leak the lease? No: the lease is finished before the wait.
- **RESTORING quota:** can it block forever? No: the step always empties.
- **Ns start:** can a cancelled waiter leave a phantom `pending`? No: it was never started.
- **End notes:** can a stale note reach another open? No: stream IDs are monotonic.
- **`retest_refused`:** can it mis-sweep? No: Down only, current generation only.
- **`cng_reads`:** can any admitted text reach the DSA loop? No: the footer is required and the base64 is validated.
- **Forced disposal:** can a terminal error re-enter the pool? No: the function is infallible.
- **System-config check:** any path to native on an unknown? No.
- **Tag under the lock:** can a concurrent create be overwritten? No: the check runs under the lock.
- **gwz-py notes:** can they deadlock on the GIL? No: callers detach.

**Round-1 invariants re-checked as unaffected by the diff:** the Pageant mapping lifecycle and thread retirement; the agent pipe's overlapped storage; machine-proxy refusal and capture; D12 and D14; the helper environment.

## 3. Follow-ups outside the finding bar

- **Local-wait attribution (new observation).** `Governor::take_local_wait` (`governor.rs:386-392`) gives a failing member the most recent expired wait on its site, not necessarily its own. With two members on one site expiring together, the words may name the other member's budget or duration. The impact is wording only. Possible fix: key the record by connection or member.
- **`recheckout` allocation (new observation).** `recheckout` (`serve.rs:66-99`, `throttle.rs` `Relet`) re-uses the allocation snapshot taken at prepare time, at least 1 s. After a hold of up to 30 s it does not subtract the time already spent. The member's allocation is therefore not strictly enforced across the relet. This is bounded by the 30 s hold cap.
- **Fixture port race (test infrastructure).** `ssh_fixture::free_port`'s bind-and-release race is the root of the Linux CI flake. Allocate ports through the listener itself, or retry on a host-key mismatch. A separate lane already owns this.
- **Carried by the RemPlan:**
  - the cancel-mid-read flake;
  - a CapacityDetail refused at encode closing the carrier;
  - stale `held_over` holds (1.2.0);
  - helper-failure caching;
  - no real Pageant 0.83 rows (deferred).

## 4. Risks and next action

**Risks:**
- P2-5's Windows trigger is still only documented behaviour. The dabeest host returned success for a peer reset, so the real-RST row cannot tell the fixed code from the old code there; only the injected rows do. The fix is unconditional, so this no longer matters for correctness.
- Windows rows of lane D's real-git urlmatch table rest on the lane's evidence. I did not run them on Windows, per the instructions.

**Next action:** the GO stands for the State axis. Land the separate test-flake lane (`free_port`, and the gwz-py close bound). Record the two new P3 observations with the existing follow-ups.
