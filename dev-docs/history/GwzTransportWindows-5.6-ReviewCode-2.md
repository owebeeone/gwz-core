# Windows parity 5.6 — CODE-AXIS RE-VERDICT (round 1 remediation)

**Review object:** the remediation of step 5.6 (TR4.7) round 1, which is the merged lanes A to D plus documents, against my round-1 report (`GwzTransportWindows-5.6-ReviewCode.md`) and the remediation plan `GwzTransportWindows-5.6-RemPlan.md`.

**Baseline** (verified at start and at end; unchanged):

| Repo | Commit |
|---|---|
| root | `8fc5acb199cdedbe0f9374c413ebff333973f738` |
| gwz-core | `45fe6e6a233165c63e77783632d63b78f4a5b852` |
| gwz-transport | `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (unchanged) |
| gwz-cli | `cd72b9548d3f4d29105c14d68a94f549d16fc022` |
| gwz-py | `ea94500afdcfa60a163ac87514cbee153b6b1d7c` |
| gwz-core-evidence | `299e986b595f14bc09f621324386b142fb81d85b` (reference only) |
| gwz-sspi | `d3038882` (unchanged, reference only) |
| git2-rs | `d13951f7` (unchanged, reference only) |

**Date:** 2026-10-14. **Axis:** Code. Independent, adversarial, read-only.

**Verdict: GO.** Open findings: P0 0, P1 0, P2 0, P3 0. Both round-1 P2s and all twelve round-1 P3s are closed. There are no new findings. In round 1 I pre-committed to GO on a revision that resolves P2-1 and P2-2 as specified. They are resolved as specified, and no new P0–P2 was found, so the pre-commitment is honoured.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| **P2-1** Down key: untried members never get the `not attempted` text | Lane B builds the §5.3 texts. A local `EndNote` (NotAttempted, StillFailing) is carried from endpoint to driver through the `Handoff`; no wire change. | Original counterexample re-traced: two failed retests, then an arrival.<br>• SSH: `machine.rs` `Decision::Finish(attempt 0)` → `placement_endpoint/admission.rs:197-201` `note_end(NotAttempted)` → `ssh_handoff.rs` `note_end`/`take_end` → `session/driver/opening.rs:328` → `note_ssh_end`.<br>• HTTPS: `https_endpoint/retry.rs:466-474` → `request.rs:220-314` `ended` → `https_failure.rs` `reason()`, which applies `Ended::down_words`.<br>• A failed retest's member and the members swept with it get `StillFailing` (`completion.rs:272-276,321-326`, `retry.rs:254-261,318-325`).<br>• Text is `not attempted: <host:port> failed earlier in this run (<reason>)` with no attempt suffix; the code is the recorded failure's.<br>• Closure tests exist and pass: `placement_endpoint/down_tests.rs:129` (full string), `transport_host/https_endpoint/down_tests.rs:147`, `session/driver.rs:420-444` (asserts no `(attempt`), `https_failure.rs:909-920`; `down_tests` 17/17.<br>• gwz-cli `src/tests/g15.rs:350-376` pins the Troubleshooting text against the built strings and passes. | **Closed (verified)** |
| **P2-2** Native commit/tag route not fail-safe for system configs outside a fixed list | Lane A decides from the `git` the CLI route would run. It derives `<prefix>/etc/gitconfig` (plus `share/git-core`, the resolved-link prefix, Apple and Git for Windows forms), and goes to the CLI on a file only one side reads, on no git or no prefix, and on `NOSYSTEM` with Windows `%PROGRAMDATA%`. | Re-traced my Linuxbrew/MacPorts counterexample through `system_config.rs` `find_git` → `derive` → `check`:<br>• the git on `PATH` under a non-listed prefix whose `etc/gitconfig` exists and is not in libgit2's `find_system()` gives `SystemFile{GIT_ONLY}`, so the CLI;<br>• the converse (`/etc/gitconfig` read by libgit2 but not by a Homebrew git) gives `NATIVE_ONLY`, so the CLI;<br>• no git or no prefix gives the CLI.<br>`GIT_CONFIG_SYSTEM` handling is consistent with libgit2 (`libgit2/src/libgit2/repository.c:1362-1385` honours both variables). Closure rows exist and pass: `system_config` 11/11 (`a_file_the_git_may_read_that_native_does_not_picks_the_cli`, `a_file_native_reads_that_the_git_does_not_read_picks_the_cli`, `no_git_or_no_prefix_picks_the_cli`, `no_system_leaves_only_the_files_native_always_reads`, `the_system_file_follows_the_prefix_of_the_git`, …). | **Closed (verified)** |
| **P3-1** Pageant budget text advises raising `--ssh-timeout` | Lane C: `BUDGET_WORDS` names the 30 s budget and `--ssh-timeout 0`. T6 and the Windows help say "up to 30 seconds; 0 removes the limit". | `pageant_words.rs:13-30`, gwz-cli `transport_help.rs` Windows sentence, and guide lines 166-176 all say so. `pageant_words` 6/6 pass. | **Closed (verified)** |
| **P3-2** A Pageant refusal is worded "Pageant is the SSH agent GWZ selected" | Lane C: a Denied text of its own. A UIPI refusal while `IsWindow` holds becomes Rejected, not "gone". | `words.rs:18` (`Pageant` with no cause gives `denied_reason`), `pageant_words.rs` `denied_words`, `pageant_exchange.rs` `send_failure` and `sys.rs` (`GetLastError` then `IsWindow`). Mapping and text rows are in `pageant_exchange/tests.rs` and `pageant_words_tests.rs`. The Windows call site is verified by reading only. | **Closed (verified; the Windows call is read only)** |
| **P3-3** D7 hint appended to a local allocation timeout | Lane C excludes Allocation and Interaction. | `https_failure.rs:343-363` (`waited_locally`). Row `a_local_wait_that_ended_an_open_is_not_a_failed_direct_connection` passes with `allocation_timeout()` under `Direct`; my original counterexample no longer reproduces. The plan's extra clause, "any failure without network facts", is not implemented, but no remaining local path reaches the predicate with Unavailable, Io or Timeout. | **Closed (verified)** |
| **P3-4** T5 kept per operation | Lane C keys it per open: operation plus canonical address. | `redirect_log.rs` is keyed `(operation, open)`. `prepare.rs:98` (`original.base()`) and `request.rs:309` (`parsed.base()`) use the same canonical form. Row `two_opens_of_one_operation_keep_a_refusal_each` passes, and an unrelated open takes none. | **Closed (verified)** |
| **P3-5** Non-ASCII `WWW-Authenticate` fails as `Protocol` | Lane C parses the header on bytes. | `native.rs:156-213` uses `as_bytes()`, `trim_ascii()` and byte scheme comparison. Row `a_basic_challenge_with_a_utf8_realm_still_asks_the_helper_once` (`native/tests/precedence.rs`) is a Windows-gated module; verified by reading and by the lane's Windows receipt. | **Closed (verified by reading; the row is Windows-only)** |
| **P3-6** Local Capacity text always says "64 slots … 30 s" | Lane B: the report carries which budget waited and for how long. | `Resource::local_budget` (`https_connection.rs`: SetupSlots or JobBudget; `shared_reservation.rs`: Reservation; `ssh_setup.rs`: JobBudget) → `ssh_pool.rs` records the wait at CancelConnect → `Governor::local_wait_ended` → `completion.rs` `take_local_wait`, and `poll.rs` on HTTPS → `LocalWait::message` gives one text per budget with the measured seconds. With no note, the fallback says only "a limit on gwz's own connections stayed full". `capacity` 62/62 and `job_budget_wait` 7/7 pass. | **Closed (verified)** |
| **P3-7** Hold note understates a `Retry-After` over 30 s | Lane B returns the uncapped value and words it. | `hold.rs` `set` returns `retry_after_ms`. `notes.rs` adds the "one command waits at most 30 s" arm. Rows pass (`notes` 30/30, `hold` 89/89). | **Closed (verified)** |
| **P3-8** POST-hold origin stickiness | Lane B keeps separate discovery and POST ends; a POST that sees a hold after checkout returns its lease and checks out again. | `hold.rs` `post_until` and `blocks_continuing`: interleaving discovery 30 s at t=0, POST 2 s at t=1 releases POSTs at t=3. `serve.rs` `renew_after_hold` and `throttle.rs` `recheckout`. `control_hold_tests` and `post_hold` 5/5 pass. | **Closed (verified)** |
| **P3-9** gwz-py drops notes | Lane B adds a noted cancellable entry; notes become `logging.getLogger("gwz")` records (detail at DEBUG). | `transport_host/cancellable.rs` adds `with_cancellable_local_transport_native_noted`. gwz-py `native/src/route/transport.rs:190-233` (`log_notes`, `catch_unwind`, `Python::attach`); operations run under `py.detach`, so there is no GIL inversion. The design's gwz-py paragraph (line 494) and changelog agree. The Python row is in `test_client_host_transport.py` (not run here; CI passes). | **Closed (verified by reading)** |
| **P3-10 (a)** `include.path` forms libgit2 does not expand | Lane A: CLI on `~user/`, `%(…)`, empty or valueless. | `route.rs` `include_paths` covers `include.path` and `includeIf.*.path`. `route_config.rs` rows pass. | **Closed (verified)** |
| **P3-10 (b)** `GIT_ATTR_*`, `GIT_TEST_*`, `attr.tree` | Moot for `-a`; prefixes removed from the harmless list. | `environment.rs:12-37`: neither prefix is listed, so both route to the CLI (row passes). No production input reaches native `-a`: `plan_commit` returns `Err(AllCommit)` when `all && !native_all`, and `native_all` is set only by the test-only `RoutePolicy::NATIVE_ALL` (`git grep native_all` finds no production setter). | **Closed (moot verified)** |
| **P3-10 (c)** `commit.cleanup` lower-cased | Lane A: compared case-sensitively. | `message.rs:16-26`; the row asserts `Strip`, `STRIP` and others give `None`, so the CLI. | **Closed (verified)** |
| **P3-10 (d)** Two "now" timestamps | Lane A: one clock reading for author and committer. | `exec.rs` `signatures` reads `clock()` at most once, through an injected `Clock`. Rows pass. | **Closed (verified)** |
| **P3-10 (e)** Leading-zero epoch accepted | Lane A: non-canonical epochs go to the CLI. | `ident.rs` `epoch`; rows include `012345678 +0000` and `@00`. | **Closed (verified)** |
| **P3-11** Opaque texts: worker provenance, SSPI redirect, `CliReason` invisible | Lane C words the worker refusals and the SSPI 3xx. Lane A plus gwz-cli surface `CliReason` under `--verbose`. | `https_failure.rs` `sign_in_words`: the worker is missing or mismatched with "Reinstall gwz, or <native hint>", and a 3xx during the sign-in has its own text. Rows pass (`https_failure` 26/26). `native_commit/notes.rs` `RouteNotes`; gwz-cli `dispatch.rs` `route_notes` gives a reader only in human mode with `--verbose`. The backend clone shares its observations (`Arc`), so no transport rows are lost. gwz-cli `tests/route_verbose.rs` passes 1/1 here. | **Closed (verified)** |
| **P3-12** Docs contradicting the code | Lane C, plus lane B for Capacity. | Guide banner (line 3), "the one case" (lines 12-15), proxy-tested (lines 361-364, 611); Troubleshooting "its last attempt was refused" and the HTTPS Credential Failure causes; Releases and MachineOutput; the `endpoint_environment.rs:10-22` comment (HTTPS needs a resolvable home); the boundary-test docstring. Pins pass: gwz-cli g15 9/9 and `troubleshooting_credential_pins` 4/4. | **Closed (verified)** |

## Changed-range analysis

**What changed:**
- gwz-core `37705303..45fe6e6a`: 117 files, +6501/−674. Commits:
  - `0d501ee6`: reviews and the remediation plan.
  - `d046c9e8`, lane D: helper view urlmatch, forced SSH disposal, CNG key-form guard, device-path spellings.
  - `94210d79`: D15, documents only.
  - `2fc2fc42`, lane C: texts, per-open T5, byte-wise challenges, docs.
  - `8cefbe40`, lane A: native commit fail-safe, `-a` always to the CLI, `RouteNotes`, transactional tag create.
  - `e29801f1`, lane B: Down texts, Capacity wording, local waits, holds, RESTORING, Ns start, the noted entry.
  - `45fe6e6a`: lane-check message.
- gwz-cli: `--verbose` route reason, docs, a help word change and rustfmt.
- gwz-py: notes as logging records.
- gwz-transport: unchanged, so the wire is unchanged.

**Every change maps to a RemPlan disposition or an operator decision:**
- OD17: release-notes matter, no code.
- `-a` narrowing.
- Capacity wording.
- Down texts built.
- gwz-py notes.
- D15.

Out-of-plan edits are test fixtures and tooling only: `cut_proxy.rs` reset mode and `windows_lane_check.py`. The native `-a` code kept unrouted follows the operator's "keep for later" decision.

**Regression hunt in the overlaps:**
- **`request/https_failure.rs`.** `reason()` = `plain_reason()` then Down words. `sign_in_words` comes first but is limited to `AuthMethod::Sspi` with Authentication+3xx, or Unavailable/Protocol before the exchange began. Capacity, helper timeouts and T5 are unaffected (rows pass). Down and Capacity cannot coexist, because Down is reached only through the Retry class.
- **`request.rs`.** The Anonymous→Gh second open resets `ended`, so no stale note crosses opens. The T5 key matches the endpoint's.
- **`session/driver.rs` and `opening.rs`.** `Refusal{failure,note}` is threaded through every early-return path (`.into()`). The note is taken once per open; `END_LIMIT` 256 bounds cancelled leftovers.
- **`https_endpoint.rs` and `retry.rs`.** `retest_refused` equals `abandoned` for non-retest flights. Refusal counting counts only throttled requeues.
- **`prepare.rs` and `serve.rs`.** `renew_after_hold` returns the lease Reusable only when no response is held and no native route is bound. It is cancellable on both awaits and uses the remaining allocation (minimum 1 s).
- **`ssh_pool.rs`.** The deferred `started` (State P3-3) can leave a connection cancelled while waiting unstarted. The governor's table ignores events for unknown connections (`states.rs:132`).
- **gwz-cli `dispatch.rs`.** Notes and route notes share one `StderrProgressSink` (`Arc`). Machine modes get neither.
- **gwz-cli `transport_help.rs`.** A single sentence changed, consistent with the guide.
- **Troubleshooting.** Every quoted text is pinned against the built strings.

I found no new defect at P3 or above.

**ARCHITECTURAL classification:** none. The `Handoff` side channel (end notes, beside D12's trust findings) is an in-process local carrier that the design records (§5.3 changelog). It is not a wire or contract change.

## 0. Evidence base

**Read fully:**
- The RemPlan.
- The diffs of `machine.rs`, `completion.rs`, `admission.rs`, `ssh_handoff.rs`, `end_note.rs`, `capacity.rs` (lines 100-300), `hold.rs`, `notes.rs`, `serve.rs`, `throttle.rs`, `prepare.rs`, `redirect_log.rs`, `native.rs`, `pageant_words.rs`, `words.rs`, `pageant_exchange{,/sys}.rs`, `request.rs`, `opening.rs`, `retry.rs`, `https_endpoint.rs`, `governor.rs` (local waits), `ssh_pool.rs` (local budget and pending start), `https_connection.rs`, `shared_reservation.rs`, `ssh_setup.rs`, `ssh_connection.rs`, `system_config.rs` (whole), `route.rs`, `native_commit.rs`, `notes.rs`, `exec.rs`, `ident.rs`, `message.rs`, `environment.rs`, `config_view.rs`, `cancellable.rs`, and `https_failure.rs` additions.
- gwz-cli `dispatch.rs`, docs diff and help.
- gwz-py `ea94500`.
- NoFallbackPlan §6 and adaptive design changelog additions.
- The guide's changed lines.

**Sampled:** lane D (`helpers.rs` urlmatch; `regular_file.rs`; `ssh_limits.rs`). These are State-axis findings, scanned for panics and cfg only (`helpers.rs` has no `unwrap`/`expect`).

**Not read:** test bodies beyond the closure rows named above.

**Commands and results:**
- `check_windows_parity.py`: unported 1 (6.1's throughput row, as before), nothing new.
- `check_checked_artifact_boundaries.py`: ok.
- `check_cfg_boundaries.py`: 433 listed, nothing new. No added `#[cfg]` or `#[path]` lines in the src diff.
- `check_candidate_switches.py`: 23 sites match.
- `check_process_globals.py`: nothing new. The only added `env::var_os` is in a hermetic child test, and every caller of `reset_global` sits behind `hermetic!()`.
- No changed `.rs` file exceeds 1000 lines.
- Candidate library tests (`prepare.py` → `/Volumes/projects/limbo/build-targets/review56r2-code-cand`, target `…/review56r2-code-target`, `GWZ_TEST_GIT=real`), all passing:

  | Filter | Passed |
  |---|---|
  | `down_tests` | 17 |
  | `capacity` | 62 |
  | `pageant_words` | 6 |
  | `agent_source` | 38 |
  | `native_commit` | 109 |
  | `system_config` | 11 |
  | `notes` | 30 |
  | `hold` | 89 |
  | `redirect_log` | 6 |
  | `https_failure` | 26 |
  | `https_redirect_tests` | 4 |
  | `session::driver` | 22 |
  | `post_hold` | 5 |
  | `job_budget_wait` | 7 |
  | `ssh_connection` | 6 |
  | `regular_file` | 14 |
  | `helpers` | 37 |
  | `machine_proxy` | 44 |
  | `kex_retry` | 9 |

- gwz-cli (target `…/review56r2-code-cli`): `--lib g15` 9/9, `--test route_verbose` 1/1, `--test troubleshooting_credential_pins` 4/4.
- The ordinary-shape full suite and gwz-py's Python tests were not run here. I rely on the cited CI and the receipt `fixb-20261014-g`.

**Disclosure.** While listing warnings I ran `touch src/lib.rs` in the prepared candidate directory. That path is a symlink into `gwz-core/src/lib.rs`, so the file's modification time changed. Its content did not, and `git status` is clean apart from the pre-existing untracked `dev-docs/GwzRemoteTransportBugReport.md`, which I did not write.

**Warnings.** The test-profile build emits 11 warnings (unused imports and test fixture fields). Blame places those I checked at pre-range commits, or at a re-flowed import list. They are not production code and not in scope.

**Flake claims.**
- `poll_dispose_completes_or_waits_without_releasing_session` ("temporary host key did not match"): the claimed cause is plausible. `ssh_fixture.rs:279-285` `free_port()` binds port 0 and releases it, a bind-race window between parallel fixtures. It is test-only.
- The gwz-py close-bound row and the `idle_loss` row are outside my axis; not checked.

## Problem, requirement provenance, and scope

The requested outcome is unchanged: 1.1.0 with Windows parity. Each change traces to a RemPlan row or to the operator's decisions of 2026-10-13 and 2026-10-14. Those outcomes are deferred and not re-litigated; their shape was checked:
- `-a` is truly unrouted in production.
- The Capacity "last attempt" wording is consistent across code, docs and pin.
- The Down texts match design §5.3.
- gwz-py notes match the design's gwz-py paragraph.
- D15 is recorded and matches `pipe_name.rs`.

No lane widened scope:
- New public surface is limited to `gwz_core::git::RouteNotes` (P3-11's disposition) and `with_cancellable_local_transport_native_noted` (P3-9's disposition).
- The `SshConnection.shutdown` function-pointer seam is a test seam; production always uses `shutdown_both`.

## 1. Findings

None open. No new findings.

## 2. Invariant analysis (attacks that failed)

- **Stale end notes.** A note left for an open that is never worded (cancelled) could be picked up by a later open with the same session and stream. Stream ids are per-session and the driver takes the note on the same open's failure; `END_LIMIT` bounds leftovers. Not reproduced.
- **Down on HTTPS naming the wrong site.** The words name the member's own URL host and port (`request.rs:166-168`), never a redirect target, which matches §5.3's "host as configured".
- **Native `-a` reachability.** No production path constructs `RoutePolicy{native_all: true}`. `Git2Backend::new` and `without_credential_helpers` use `Default`, and `with_host_context` copies the parent's policy.
- **`with_route_notes` clone loses observations.** No: `TransportObservations` is `Arc<Mutex<…>>`, and `Runtime` clones its `RequestContext`.
- **Byte-wise challenge parse admits a malformed Negotiate.** No: `Offer::initial` still refuses a non-empty first token, and duplicates are still `Protocol`.
- **POST re-lease outside its budget.** No: `recheckout` uses the prepare-time remaining allocation, and both awaits are cancellable.
- **`include.path` regex misses `includeIf`.** No: `^include(if\..*)?\.path$` covers both on libgit2's lower-cased keys.

## 3. Follow-ups outside the finding bar

- **Local-wait attribution heuristic.** `Governor::take_local_wait` matches the most recent expired wait for the same site within 10 s. It is not keyed by the open. `local_wait_ended` is also reported for connections cancelled for other reasons while waiting. So two concurrent members of one site waiting on different budgets could swap the named limit or duration. This is a hypothesis with no reproduction. Keying the record by the pool request, as T5 now is, would remove it.
- **D7 predicate.** It still admits a failure with no facts at all. The plan's "any failure without network facts" clause is not built. No current local path reaches it, but a future local Timeout without a cause would.
- The round-1 follow-ups the RemPlan lists (Gh/Anonymous on Windows, trust-map eviction, `ssh setup timeout: aggregate` wording, the blanket `#[allow(dead_code)]` on `agent_pageant`, process-global debt) remain as recorded.

## 4. Risks and next action

**Risks:**
- Several closures rest on Windows-only code read on macOS: the Pageant UIPI mapping call site, the UTF-8-realm row, and Git for Windows prefix derivation on a real install. They are backed by receipt `fixb-20261014-g`.
- The deferred items from round 1 (Pageant 0.83 rows, 6.1 throughput, Phase 6) are unchanged.

**Next action:** none on the Code axis. GO on this tuple.
