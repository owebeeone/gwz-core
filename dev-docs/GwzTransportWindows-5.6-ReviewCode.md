# Windows parity 5.6 — Code-AXIS REVIEW

**Review object:** Windows parity plan step 5.6 (TR4.7, the Windows implementation review). It is the single review point for everything built since the last accepted reviews:
- gwz-core `4c7f9f7d..37705303` (plus `caf1e4c8`), excluding the accepted adaptive step (`e043d952`, `0d7b3588`) and steps 4.1/4.2.
- gwz-transport `6146bd58..a27817b9`.
- gwz-cli `21d62310..220501a5`.
- gwz-py commits since 2026-10-09 up to `28aff11d`.

**Baseline (verified at start and at end, unchanged):**

| Repo | Commit |
|---|---|
| root | `42feaa9dd98b49b6cf2b214de2a6ecf808fde670` |
| gwz-core | `37705303f80d9f70d1ab4bcc0782323decaa6e2c` |
| gwz-transport | `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` |
| gwz-cli | `220501a5f00abd76d23f5b3ef75499d5f7a09d19` |
| gwz-py | `28aff11d7f441542ef4d14c492dda5f4419ba22a` |
| gwz-sspi | `d3038882` (reference only) |
| git2-rs | `d13951f7` (reference only) |
| gwz-core-evidence | `350703ca` (evidence, reference only) |

**Date:** 2026-10-13. **Axis:** Code (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only.

**Verdict: NO-GO.** P0 0, P1 0, P2 2, P3 12.

I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified below.

---

## Surface

I read these cold, before any code or design:
- gwz-cli `src/globalargs/transport_help.rs` and `retry.rs`.
- gwz-cli `docs/Troubleshooting.md`, `MachineOutput.md` and `Releases.md` (the diff, and the sections it touches).
- gwz-core `dev-docs/GwzTransportWindowsUserGuide-DRAFT.md`, in full.
- Texts T1–T9 (design §14 item 13) and where the code holds them (`transport_host/request/https_failure.rs`, `transport_host/session/driver.rs`, `agent_source/{words,pipe_name,pageant_words}.rs`).
- The worker-refusal path.
- `setup_retry/limit/{capacity,notes}.rs`.

**Does each message say what happened and what to do next?** Mostly yes. These are good and actionable:
- T1, T3, T4, T5, T8, T9.
- The stopped-ssh-agent text.
- The `SSH_AUTH_SOCK` refusal (it names the variable, the accepted form, and "unset it" or "set it to your pipe").
- The two host-key texts (D12).
- The three Capacity texts.

The exceptions are findings:
- **P2-1:** the Down / `not attempted` text is documented but does not exist.
- **P3-1:** the Pageant budget text gives advice that cannot work.
- **P3-2:** a Pageant that refuses gets the "Pageant is the SSH agent GWZ selected" text, which says nothing about what failed.
- **P3-3:** the D7 hint is appended to a local timeout.
- **P3-4:** the T5 text can be lost or attributed to the wrong member.
- **P3-6:** the local Capacity text names the wrong resource.
- **P3-7:** the over-30 s hold note understates the wait.
- **P3-11:** the worker-provenance refusals are opaque ("HTTPS endpoint request failed: Unavailable" or "Protocol").

**Defaults stated?** Yes in the CLI help:
- `--ssh-timeout` 9, or 3 with native.
- `--max-retries` 3.
- `--max-per-host` 32, or 8 with native.
- `--jobs` 100, or 50 with native.
- The 30 s setup budget.

The guide's "Setup limits" table matches the help.

**Is wording consistent between help, guide and messages?** These diverge:
- Troubleshooting.md:279 and Releases.md:37-38 say Capacity means "a 429 … on every attempt". MachineOutput and the code say "on its last attempt" (P3-12).
- The guide still says, at line 3, "these capabilities are not enabled in the current candidate build". Step 5.1 enabled them.
- The guide (lines 333-336 and 583) says GWZ's own transport "has not yet been tested through a proxy". Step 4.7's commit `bb1696bd` records 16 of 16 rows through the transport's own CONNECT on dabeest.
- The guide (line 12) calls `http://` "the one case" that goes native. File, local-path and `git://` remotes also go native (`transport_binding.rs`, after 5.1).
- Troubleshooting's "HTTPS Credential Failure" (the target of T8 and T9's "See Troubleshooting") still says "Windows transport parity remains under qualification". It does not explain the `declined` or `configuration_refused` cases.

**Misleading names?**
- The local Capacity text always says "64 connection-setup slots … 30 s" (P3-6).
- The hold note says "asked gwz to wait 30 s" for a `Retry-After: 120` (P3-7).

**U20 (error paths point to `--verbose`) is unmet. Does it matter?** Not for most paths, because the messages name the cause and the fix. It does matter for the residual opaque texts:
- "HTTPS endpoint request failed: Unavailable" or "Protocol" (a missing or mismatched SSPI worker, a non-ASCII challenge, P3-5).
- "…: Authentication" (a redirect during an SSPI exchange).
- "ssh setup timeout: aggregate".

In those cases the user has nothing to go on. A `--verbose` pointer alone would not fix them, because the cause is not in the verbose rows either. Better words are the fix.

**First-day walkthrough (Windows, help and guide only). These are the points where I had to guess:**
1. **SSH with Pageant.** The order is clear in both help and guide. With confirm-on-use and the default 9 s, the attempt fails with T6, which says "raise --ssh-timeout".
   - *Guess:* how far to raise it. Above 30 s the setup budget ends the attempt instead. The budget text then still says "raise --ssh-timeout", and only `0` works. Neither the help's Windows sentence nor the message states the 30 s ceiling (P3-1).
   - *Guess:* what "Pageant is the SSH agent GWZ selected …" means when it is the only text after Pageant refused the request (P3-2).
2. **SSH with the OpenSSH agent.** The service text, the `SSH_AUTH_SOCK` refusal and the RSA-only host-key texts are clear.
   - *Guess:* none for the transport.
   - gwz-cli Troubleshooting still advises "run `ssh -T git@<host>` once to record the host key". That suffix is only appended to libgit2's native text (`url_scheme_state.rs:244-249`), so it does not mislead on the transport. On Windows it would record an ed25519 key, which the guide says is unusable.
3. **HTTPS with Git Credential Manager.** GitHub offers Basic only, so the helper is asked. That is clear.
   - *Guess:* what to do after a cancelled GCM window. T8 points to Troubleshooting "HTTPS Credential Failure", which does not describe this case.
4. **HTTPS behind a machine proxy with an `http=`-only mapping.** T1 is clear and puts the administrator's fix first.
   - *Guess:* whether the transport works through a proxy at all, because the guide still says it was never tested (stale, P3-12).
5. **A throttled server.** The CLI help points to Troubleshooting, and its notes and Capacity sections are clear.
   - *Guess (wrong guidance):* a host that stops answering is documented to end its remaining members with `not attempted: … failed earlier in this run (…)`. That text does not exist. Members instead show another member's failure, as if they had made the attempt (P2-1).
   - The guide has no throttling content. It is Windows-specific and defers to the CLI docs, which is acceptable.

---

## 0. Evidence base

**Read fully:**
- Plan steps 3.1–6.5 and §5.
- Design §14 item 13 (T1–T9).
- The user guide.
- GwzNoFallbackPlan §6.
- The adaptive design's §4.5, §5.3 table, §9, gwz-py paragraph (line 492) and changelog 2026-10-11.
- gwz-transport `transport.taut.py`, `codec/failure_detail.rs` and `admission.rs` diffs.
- gwz-cli `transport_help.rs`, `retry.rs`, `dispatch.rs` and the docs diffs.
- In gwz-core:
  - `agent_source.rs` and `pageant_words.rs`
  - `words.rs` and `pipe_name.rs` (refusal and display)
  - `ssh_home.rs` (to line 200)
  - `kex_retry.rs`, plus the establish loop diff and `room.rs`
  - `capacity.rs` (to line 200), `requeue.rs` (to line 120) and `notes.rs`
  - `https_failure.rs` (parts) and `session/driver.rs` (lines 30-200)
  - `ssh_handoff.rs` trust log
  - `system_config.rs`, `native_commit.rs` dispatch and `exec.rs` (parts)
  - `endpoint_environment.rs` (head and `endpoint_config_native`)
  - step 5.1's source diff in all three repos
  - `error_code.rs`, and the Capacity mapping diff (`e1da9a6c`)

**Delegated to four read-only sub-readers, with key claims re-verified by me (file:line below):**
- Agent selection and channels.
- HTTPS policy, proxy, `git.exe` and redirects.
- Native commit/tag.
- The adaptive extensions (Down, RESTORING, Ns, holds, notes, Capacity).

Claims I re-verified myself:
- Down finish and sweep.
- The native.rs `to_str` refusal.
- The D7 predicate and the allocation-timeout path.
- The redirect log keying.
- The hold merge.
- The 8-slot HTTPS connector.
- `GIT_ATTR_`/`GIT_TEST_` prefixes.
- The `commit.cleanup` lower-casing.
- The two `signature()` calls.

**Sampled:** gwz-py 5.3 and Capacity regeneration; gwz-cli `worker_host.rs` and `progress_detail.rs`; `transport_setting/text.rs` Windows quoting; the `pageant_exchange` boundary.

**Not read:**
- The bulk of the new test modules.
- The `https_auth` view internals beyond `GIT_CONFIG_*` handling.
- The SSPI exchange internals beyond scheme selection.
- `process_tree`, which was accepted at 4.2.
- The Windows CI workflow beyond step 5.4's summary.

**Commands and results:**
- `check_windows_parity.py --root .`: 406 files, unported 1, paired 151, platform 39, "nothing new". The one unported entry is 6.1's `throughput_tests.rs` HTTPS large-pack row, a deferred speed item.
- `check_checked_artifact_boundaries.py --source src`: ok.
- `check_cfg_boundaries.py`: 1599 files, 433 listed (gwz-core 431, gwz-cli 2, gwz-py 0), "nothing new". `cfg_boundaries_allowlist.json` is unchanged in the range.
- `check_candidate_switches.py`: 23 sites match the inventory.
- gwz-transport `scripts/regen.py --check`: "4 generated artifacts verified". Run with taut-proto 0.10.0 built from the local `taut` tag `v0.10.0` into a scratch venv (nothing downloaded) and rustfmt 1.9.0 `ac68faa20c`.
- gwz-core `protocol/regen.py --check`: OK.
- `candidate-regenerator.py … --check`: verified.
- `tests/transport_consumer/protocol/regen.py --check` (Rust 1.96.0): verified.
- gwz-py `check_protocol_drift.py`: OK.
- The `owner-schema-sha256` of both generator pins equals sha256 of gwz-transport's `transport.ir.json` (`48837b7e…`). `.github/gwz-transport.commit` is `a27817b9`.
- `tests/transport_backend/test_windows_qualification_boundary.py`: 6 OK.
- `git grep gwz_windows_https_qualification` finds nothing in gwz-cli, gwz-py or gwz-transport. In gwz-core it remains only in dev-docs (marked "Retired" in GWZDesign/GWZRequirements) and in the boundary test's split-string constant.
- No new `#[path]`: the set of `#[path]` attributes in `src/` is identical at base and tuple (57). Every added bare `#[cfg(test)]` is on a braced `mod tests { … }`; one `#[cfg(windows)]` sits on a test fn (`https_auth/runner/environment.rs:185`). Neither is a `use` or an unbraced declaration.
- Changed `.rs` files over 1000 lines: generated `protocol/{generated,candidate_generated}.rs`, `tests/protocol.rs` (1967, +17) and `crates/history-check/src/tests.rs` (1468, +19). All are pre-existing oversize files.
- Candidate tests (`prepare.py` into `/Volumes/projects/limbo/build-targets/review56-Code-cand`, target `…/review56-Code-target`, `RUSTFLAGS='--cfg gwz_transport_candidate' GWZ_TEST_GIT=real cargo +1.95.0 test --lib`), all passing:

  | Filter | Passed |
  |---|---|
  | `capacity` | 58/58 |
  | `pageant_words` | 5/5 |
  | `agent_source` | 37/37 |
  | `native_commit` | 82/82 |
  | `notes` | 22/22 |
  | `down_tests` | 14/14 |
  | `kex_retry` | 9/9 |
  | `machine_proxy` | 44/44 |

- Windows-only arms were reviewed by reading. Windows behaviour claims rest on the receipts named in the commits: `retire51-20261013-b`, `py53-20261013-a`, `cngretry-20261013-b`, `proxy-20261012-b`, `wh3-20261012-a`, `capacity-20261013-b`.
- An untracked `gwz-core/dev-docs/GwzRemoteTransportBugReport.md` appeared during the review. It was not written by me and is out of scope.

---

## Problem, requirement provenance, and scope

**Requested outcome:** 1.1.0 ships the CLI transport on macOS, Linux and Windows, with Windows parity against gwz 1.0.17.

**Provenance traced:**
- Steps 3.x, 4.x and 5.1–5.4 trace to the plan revision 4 and the accepted design (2.4).
- D1–D14 trace to GwzTransportTR18-OperatorDecisions.md.
- The Capacity code (76) and `CapacityDetail` trace to the adaptive design D12/D13 and the operator decision of 2026-10-13.
- The `HelperFailureCause` additions trace to D13.
- The native commit/tag route traces to the operator's 2026-10-12 rule in GwzNoFallbackPlan §6.
- The KEX retry traces to D14.

I found no unauthorized scope expansion. The native commit route is outside the Windows plan, but it is directly authorized by the operator's rule and is in this review's object.

**Each remedy against its problem:**
- The wire additions are additive and in-process only, since 1.1.0 has no session route.
- The 5.1–5.3 retirement is mechanical, and on Unix it reduces to the old union predicate.
- The deleted WH1 refusal is correct: file, local and `git://` remotes now take libgit2's route on Windows, as on Unix.
- The kept `cfg!(windows)` splits are justified:
  - The machine-proxy capture guard is required by "Windows reads no environment proxy".
  - The capability projection's `file_identity`/`exact_agent_identity` stay false on Windows pending GwzV110Plan S7.2's named fixture GO. gwz-py's capability check uses the ordinary `protocol::transport_capabilities::handle`, which says `file_identity: true`, so Python is not regressed.

The blocking findings are a parity/diagnosability defect in delivered behaviour (P2-1) and a fail-safe hole in an operator rule (P2-2). Both are bounded.

---

## 1. Findings

### P2-1 — A Down key's untried members never get the `not attempted` text, and gwz-cli documents that text as shipped

- **Location:**
  - gwz-core `src/git/endpoint/setup_retry/machine.rs:176-189`: `Decision::Finish(Final { attempt: 0, ..down.last.clone() })` after `failed_retests >= GIVE_UP_AFTER`.
  - `machine.rs:291-298`: a failed retest is `Outcome::Sweep(down.last.clone())` with no "still failing" wording.
  - `src/git/endpoint/placement_endpoint/completion.rs:288-301`: `sweep` finishes every queued member with `last.clone().wire_failure()`.
  - The wording sites `transport_host/request/https_failure.rs` (`reason()`) and `session/driver.rs` (`SshOpenFailure` Display) have no Down branch.
  - `git grep` for "not attempted", "failed earlier in this run" and "still failing after a retry" finds only the unrelated root-push sentence at `workspace_ops/push_member.rs:398`.
  - gwz-cli `docs/Troubleshooting.md:250-256` promises `not attempted: github.com:22 failed earlier in this run (<the reason>)`.
- **Violated invariant:** adaptive design §5.3 table, row "A Down key's member that made no attempt", requires the recorded failure's code, the message `not attempted: <host:port> failed earlier in this run (<reason>)`, and no `retry_attempt`. §14/line 659 adds a failed retest ending "still failing after a retry about 30 s later". gwz-cli's Troubleshooting states this as current behaviour.
- **Reproduction (interleaving):**
  1. An SSH host stops answering.
  2. Member A exhausts `--max-retries`, so the key goes Down.
  3. Two retests 30 s apart fail.
  4. Member M arrives before the next `retest_at`.
  5. `decide` returns `Finish(attempt 0, last)`. M's row reads `member 'M' at 'm': ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)`, which is A's stall worded as if M had stalled.
  
  Separately, at exhaustion every member queued on the key is swept with A's `retry_attempt`, so it reads "(attempt 4 of 4)" even if it never started. The design records that sweep as "as today".
- **Impact:**
  - With a network outage, every remaining member reports a failure it did not have.
  - The documented recovery text a user or script would look for never appears.
  - The retest failure is indistinguishable from a first failure.
  - A diagnosability and Surface-contract defect on a common path (any host outage).
- **Required correction:**
  - Word a finish with `attempt == 0` that came from a Down key as `not attempted: <host:port> failed earlier in this run (<recorded reason>)`, keeping the recorded code and no attempt suffix.
  - Word a failed retest's own member with the "still failing after a retry about 30 s later" ending.
  - Carry the "untried" fact from `Decision::Finish` to the wording site. This is a local carrier; the code and wire stay as they are.
  - Otherwise, amend Troubleshooting.md, the design §5.3 row and line 659 to what ships. The design is the controlling text, so the first option is expected.
- **Closure test:**
  - A down_tests row drives two failed retests, then an arrival.
  - Assert the arrival's rendered message starts with `not attempted: <host>:<port> failed earlier in this run (`, has no `(attempt`, and keeps the recorded code.
  - A second row asserts the failed retest's text.
  - Pin the Troubleshooting example against the produced string.

### P2-2 — The native commit/tag route is not fail-safe for Git system configuration files outside a fixed list

- **Location:** gwz-core `src/git/gitbackend/native_commit/system_config.rs`.
  - The Unix `candidates` (lines 45-62) is a fixed list: `/etc/gitconfig`, `/opt/homebrew/etc/gitconfig`, `/usr/local/etc/gitconfig` and two Apple paths.
  - `covered` (lines 59-61) is `git2::Config::find_system()`.
  - `check` (lines 98-130) passes when every existing listed file is one libgit2 reads.
- **Violated invariant:** GwzNoFallbackPlan §6, operator 2026-10-12: "the native result must equal Git's for the same inputs, and anything native cannot prove equal goes to Git … Fail safe."
  - A `git` whose system file lies outside the list is assumed to read nothing libgit2 does not. Examples: MacPorts `/opt/local/etc/gitconfig`, Linuxbrew `/home/linuxbrew/.linuxbrew/etc/gitconfig`, Nix or conda prefixes.
  - The converse is also unchecked: libgit2 reads `/etc/gitconfig` while Homebrew's `git` does not, so a system-only `commit.cleanup` or identity in `/etc/gitconfig` changes native output only.
- **Reproduction:**
  1. Linux with Linuxbrew `git` first on `PATH`.
  2. Put `[commit] gpgSign = true` or `[core] hooksPath = /opt/corp/hooks` in `/home/linuxbrew/.linuxbrew/etc/gitconfig`.
  3. Run `gwz commit -m x`.
  4. `route::plan_commit` sees no signing or hooks (libgit2 never read that file) and goes native. The commit is unsigned and the hooks never run, where gwz 1.0.17 (`git commit`) signed and ran them.
- **Impact:** silent divergence from git's result on macOS and Linux, including unsigned commits and skipped organisation-wide hooks. It is a correctness and parity defect against an operator rule. Windows is largely covered: `PROGRAMFILES`-based candidates plus libgit2's PATH-based `find_system`.
- **Required correction:** make the decision fail-safe for an unknown `git`.
  - Resolve the `git` that the CLI route would run, the first `git` on the snapshot's `PATH`.
  - Derive its system file: `<prefix>/etc/gitconfig`, plus the `share/git-core/gitconfig` form.
  - Go to the CLI unless that file is absent or is one `covered` names.
  - Also go to the CLI when a `covered` file exists that the resolved `git` does not read.
  - Keep `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` handling.
- **Closure test:**
  - Route tests with a fake `git` on a scratch `PATH` whose prefix holds an `etc/gitconfig` (expect `CliReason::SystemConfig`).
  - A fake `git` whose prefix has none while `covered` names an existing file it does not read (expect CLI).
  - A row with the Homebrew prefix that still goes native when the files agree.

### P3-1 — Pageant's budget text advises the one change that cannot help

- **Location:** gwz-core `src/git/endpoint/agent_source/pageant_words.rs:24-27` (`BUDGET_WORDS`). T6 `stall_words` (lines 13-21) has the same advice without its bound.
- **Violated invariant:** design §5 says "raise `--ssh-timeout` (within the 30-second budget, or `0`)". `apply_native_timeout` keeps `connect_timeout_ms = 30_000` for any non-zero `--ssh-timeout` (`transport_host/mod.rs:140-144`).
- **Reproduction:**
  1. Confirm-every-use Pageant, with the user taking longer than 30 s.
  2. Run `gwz --ssh-timeout 60 fetch`.
  3. The setup budget ends the attempt, and the user reads "…raise --ssh-timeout (0 removes the limit)…". Raising it further changes nothing.
- **Impact:** wrong recovery advice. It is bounded because "0 removes the limit" is present.
- **Required correction:** in `BUDGET_WORDS`, name the 30-second setup budget and say only `--ssh-timeout 0` (or answering sooner) removes it. In T6 and the help's Windows sentence, add "up to 30 seconds; 0 removes the limit".
- **Closure test:** pin `BUDGET_WORDS` to name "30 seconds" and `--ssh-timeout 0`, and not "raise --ssh-timeout".

### P3-2 — A Pageant that refuses the request gets the "Pageant is the SSH agent GWZ selected" text, which does not say what failed

- **Location:**
  - `src/git/endpoint/agent_pageant.rs:132` maps `PageantError::Rejected` to `AgentFault::Denied` (kind `Unsupported`).
  - `ssh_setup.rs:607` maps that to `Unavailable` with no cause.
  - `agent_source/words.rs:18,46-56` then gives `pageant_selected`.
- **Violated invariant:** each message says what happened and what to do. The pipe analogue at `words.rs:28-32` does say "The agent may be running as another user, or it may refuse this account."
- **Reproduction:** a Pageant that cannot open the caller-only mapping refuses `WM_COPYDATA`, for example one run under another account on the same desktop, which the guide says is not refused at selection. The user then reads only "Pageant is the SSH agent GWZ selected, because a Pageant window is running. To use the OpenSSH agent instead, close Pageant."
- **Impact:** diagnosability on a documented configuration.
- **Required correction:** give Denied its own Pageant text: Pageant refused the request, it may be running as another user or at another privilege level, and what to do.
- **Closure test:** a `pageant_words` row with `ErrorCode::Unavailable`, no cause, for a `Pageant` source and a Denied fault asserts the new text.

### P3-3 — The D7 "direct access … that connection failed" hint is appended to a local allocation timeout

- **Location:**
  - gwz-core `src/transport_host/request/https_failure.rs:286-300`: `failed_direct_connection` accepts any `Timeout` with `facts` absent.
  - `src/transport_host/request.rs:179-191` builds `HttpsOpenFailure { machine: self.session.machine(), failure: setup_retry::allocation_timeout(), … }`, which has `facts: None` and cause `Allocation`, when the route gate is not acquired in time.
- **Violated invariant:** D7 adds the hint only to "a failed direct https connection under a direct setting". No connection was made here.
- **Reproduction:** Windows with `netsh winhttp` set to Direct (the common case). A member waits on the route gate for its canonical URL until its allocation expires. It reads "HTTPS endpoint request failed: Timeout: allocation … says direct access, so GWZ connected to the server directly, and that connection failed. If your network reaches this server only through a proxy, ask your administrator…".
- **Impact:** a false statement that sends the user to an administrator.
- **Required correction:** exclude `setup_cause` `Allocation` and `Interaction`, and any failure without network facts that is local, from `failed_direct_connection`.
- **Closure test:** under `MachineSetting::Direct`, `allocation_timeout()` does not contain the D7 words. The existing test runs with `Unread`, so it does not catch this.

### P3-4 — The T5 redirect refusal is kept once per operation, so concurrent members lose or swap it

- **Location:**
  - gwz-core `src/git/endpoint/https_worker/redirect_log.rs:59-71`: `refused` keeps the first per operation name with `or_insert_with`, at most 64.
  - `https_endpoint.rs:201-207`: `take_refused(&operation.name)`.
  - `transport_host/request.rs:288-293`: taken on any `InvalidRequest` failure of any member in the command.
- **Violated invariant:** design T5 is per refusal, with the member's own URL and the server's target.
- **Reproduction:**
  1. A server redirects all of a workspace's https repositories to `http://`.
  2. Run `gwz fetch`.
  3. The first refusal is stored and later ones are ignored.
  4. Whichever member's driver takes it first shows that one URL pair. Every other member shows "HTTPS endpoint request failed: InvalidRequest".
  
  A member failing `InvalidRequest` for another reason can also take another member's T5.
- **Impact:** wrong or missing T5 on a multi-member refusal.
- **Required correction:** key the refusal by the open (request plus stream, or canonical URL), not by the operation.
- **Closure test:** two members are both redirected to http in one operation. Each gets T5 with its own URL and target.

### P3-5 — A non-ASCII `WWW-Authenticate` byte fails the open as `Protocol`, and the helper is never asked

- **Location:** gwz-core `src/git/endpoint/https_worker/native.rs:160`, `let text = value.to_str().map_err(|_| ErrorCode::Protocol)?;`. `HeaderValue::to_str` refuses obs-text and UTF-8. The code predates the range, but step 4.4 made this the Basic-and-helper path on Windows.
- **Violated invariant:** 4.4: when Basic is the only workable scheme, the helper is asked. The Unix challenge parser works on bytes.
- **Reproduction:** a server answering `401` with `WWW-Authenticate: Basic realm="Zürich"`, under the `WindowsConfigured` policy. The result is "HTTPS endpoint request failed: Protocol" and GCM is never run.
- **Impact:** a parity and diagnosability edge on Windows.
- **Required correction:** parse the challenge as bytes, as the Unix path does; only scheme tokens need to be ASCII.
- **Closure test:** a Basic-only challenge with a UTF-8 realm asks the helper once.

### P3-6 — The local Capacity text always names "gwz's 64 connection-setup slots … for 30 s", for three different waits

- **Location:**
  - gwz-core `src/git/endpoint/setup_retry/limit/capacity.rs:16-19,155-159` (constants and text).
  - The waits that become `LocalWaitExpired` and so get that text:
    - `shared_reservation.rs:274-278`, the endpoint-wide per-host and total reservation shared by SSH and HTTPS.
    - `https_pool.rs:53`, the HTTPS connector's `Semaphore::new(8)` setup slots.
    - The 64-job budget, the only wait the text describes correctly.
  - All three map to the local detail at `https_pool.rs:456-461` and `ssh_worker/open_request.rs:111-113`.
  - The wait ends at the open's remaining allocation (`pool/clock.rs` `LocalWait{until: allocation}`), which can be under 30 s.
- **Violated invariant:** the message names the cause (design §5.3).
- **Reproduction:**
  1. Run with `--max-per-host 4`.
  2. Four long SSH streams to `github.com` hold the shared reservation.
  3. An HTTPS member to `github.com` waits in `Slot::Waiting` until its allocation expires.
  4. It reads "gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out)". None of that is true: the connections are healthy streams.
- **Impact:** misleading cause and remedy, on a rare path.
- **Required correction:** carry which local budget expired (job budget, setup slot or reservation) in the local report, word each one, and state the actual wait.
- **Closure test:** a reservation-wait expiry and an 8-slot expiry each produce their own words.

### P3-7 — The hold note understates a `Retry-After` longer than 30 s

- **Location:** gwz-core `setup_retry/limit/hold.rs:34` (`wait = retry_after_ms.min(HOLD_CAP_MS)`) returns the capped value to `notes.hold(wait)` at `control.rs:426` and `control/verdict.rs:61`. `notes.rs:55-57` renders "asked gwz to wait {wait} s; waiting".
- **Violated invariant:** the comment at `notes.rs:54` ("the wait is never understated"), and the adaptive design's open Surface P3-2 ("word the over-30 s case").
- **Reproduction:** a `Retry-After: 120` prints `note: github.com:443 asked gwz to wait 30 s; waiting`. The member then fails with "asked to wait 120 s, longer than the 30 s one command holds a host".
- **Impact:** contradictory output.
- **Required correction:** pass the server's value to the note and word the capped case, for example "asked gwz to wait 120 s; one command waits at most 30 s".
- **Closure test:** a notes row for a 120 s `Retry-After`.

### P3-8 — The POST-hold origin never downgrades, so a discovery hold over-holds POSTs

- **Location:** gwz-core `setup_retry/limit/hold.rs:36-44`. When a hold is in force, `until = max(…)` and `origin` becomes `Post` if either was a POST. The origin and the end time are merged into one hold.
- **Violated invariant:** §4.5 says a hold set by a discovery response does not hold continuing POSTs, and a POST hold lasts the POST's own `Retry-After`.
- **Interleaving:** a discovery 429 with `Retry-After: 30` at t=0, then a POST 429 with `Retry-After: 2` at t=1. Continuing members' POSTs are now held until t=30, not t=3.
- **Impact:** POSTs are held up to 29 s too long while holding their leased connection, which lengthens the dead-keep-alive exposure that changelog reading (6) accepts. Bounded by the 30 s cap.
- **Required correction:** keep separate `until` values per origin, or one per origin.
- **Closure test:** the interleaving above releases POSTs at t=3.

### P3-9 — gwz-py drops every note, against the adaptive design's gwz-py paragraph

- **Location:** gwz-py `native/src/route/transport.rs:190` calls `with_cancellable_local_transport_native`, which has no `NoteSink`.
- **Violated invariant:** gwz-core `dev-docs/GwzTransportAdaptiveConcurrencyDesign.md:492`: "The note (§9) is a `logging` record on `gwz`". The 2026-10-11 changelog ("gwz-py … needs no change") contradicts it and nothing reconciles the two. Step 4.9's U9 redirect detail is also absent in Python.
- **Impact:** a Python user whose host throttles or goes Down gets no signal until the final error.
- **Required correction:** add a noted cancellable entry, map notes to `logging.getLogger("gwz")` (detail at DEBUG), or amend line 492.
- **Closure test:** gwz-py's limit-8 fixture row (§10.2 case 19) asserts a `gwz` log record.

### P3-10 — Native commit: some git inputs and value forms are not judged (fail-safe gaps beyond P2-2)

- **Location and reproductions** (git 2.52 rows reproduced by the sub-reader in scratch repos):
  - (a) `route.rs:145-158` checks only `includeIf`. libgit2 silently ignores an `include.path` of `~user/…` or `%(prefix)/…`, so a global `include.path = ~alice/shared.gitconfig` that sets `core.hooksPath` or `commit.gpgSign` runs hooks or signs under git and not natively.
  - (b) `environment.rs:34,36` treat the `GIT_ATTR_` and `GIT_TEST_` prefixes as harmless. Under `-a`, `GIT_ATTR_SOURCE=<tree with "*.txt ident">` stores `$Id$` with git and the raw text natively. `attr.tree` is not checked at all.
  - (c) `message.rs:18` lower-cases `commit.cleanup`. `Strip` is fatal in git (exit 128) and accepted natively; the unit test at line 144 pins the wrong behaviour.
  - (d) `exec.rs:94-95,15-23` calls `Signature::now()` separately for author and committer, so their times can differ across a second boundary. Git computes one date.
  - (e) `ident.rs:200` accepts a nine-digit date with a leading zero, which git refuses.
- **Violated invariant:** GwzNoFallbackPlan §6, "anything native cannot prove equal goes to Git".
- **Impact:** bounded divergences, mostly in contrived configurations. (a) can skip hooks or signing.
- **Required correction:**
  - Route to the CLI on any `include.path` form libgit2 does not expand, and on `GIT_ATTR_SOURCE`, `attr.tree` or `GIT_TEST_*`.
  - Accept only git's case for `commit.cleanup`.
  - Take one "now" for both signatures.
  - Reject the leading-zero date form.
- **Closure test:** a route row for each of (a) and (b), and differential rows for (c) to (e).

### P3-11 — Opaque terminal texts remain where the user cannot act

- **Location:**
  - gwz-core `https_failure.rs:267` (generic `"HTTPS endpoint request failed: {:?}"`), reached by the worker provenance refusals (`native.rs:334-336`, Unavailable and Protocol; plan step 4.10 "Open").
  - A 3xx during an SSPI exchange (`native/exchange.rs:272`: "…: Authentication").
  - `native_commit.rs:149,173,181` drop the `CliReason` with `let Ok(..) else`, while §6 says "every refusal names its reason (`CliReason`), so a test or an operator can see why Git ran".
- **Impact:** a broken or quarantined wheel worker, a redirecting SSPI server, or a commit that unexpectedly ran git cannot be diagnosed. `--verbose` does not supply the missing cause either (see U20 above).
- **Required correction:**
  - Word the worker refusals: worker missing or mismatched, reinstall, or `--transport native`.
  - Word the SSPI redirect refusal.
  - Surface `CliReason` in a `--verbose` detail line or an observation.
- **Closure test:** text rows for each case.

### P3-12 — Documentation that contradicts the shipped code

- **Location:**
  - gwz-cli `docs/Troubleshooting.md:279` and `docs/Releases.md:37-38` say "on every attempt". The code and `MachineOutput.md` decide Capacity on the last attempt only, and "kept refusing connections" is printed after reset, reset, reset, 429, or after one 429 with `--max-retries 0` (adaptive P3-4/P3-5 left open).
  - gwz-core user guide line 3 ("not enabled in the current candidate build"), lines 333-336 and 583 ("GWZ's own transport has not yet been tested through a proxy"), and line 12 ("the one case").
  - gwz-cli Troubleshooting "HTTPS Credential Failure" says "Windows transport parity remains under qualification" and is the target of T8 and T9.
  - `src/transport_host/endpoint_environment.rs:18` says "It never derives a HOME or agent for HTTPS". In fact `endpoint_config_native` resolves the SSH home and captures the agent, including the Pageant probe, for every runtime. An unresolvable home on Windows (for example an explicitly empty `HOME`) now refuses HTTPS-only commands that WH1 and 1.0.17 ran.
  - `tests/transport_backend/test_windows_qualification_boundary.py:8-10` still describes the deleted `ConsumerRepositories`.
- **Impact:** users and auditors are misled about Capacity semantics, proxy support and the HTTPS home dependency.
- **Required correction:** align the texts with the code. For the `endpoint_environment` comment, either state that HTTPS needs a resolvable home on Windows or build the SSH settings lazily.
- **Closure test:** a docs pin for the Capacity rule wording, and removal of the stale guide sentences.

---

## 2. Invariant analysis (attacks that failed)

**Wire (area 1):**
- `HelperFailureCause` 9–11, `CapacityCause`, `CapacitySignal` and `FailureDetail.capacity` (field 6, `MISSING_OK`) are additive. regen checks pass in gwz-transport, gwz-core (protocol, candidate, consumer) and gwz-py.
- The digest pins equal the IR. `.github/gwz-transport.commit` is moved in the same commit.
- `failure_detail.rs` refuses a capacity detail on a non-Capacity code, beside helper fields, a partial host/scheme/port, a bad port or host, `limit > ceiling`, a negative value, or a Local cause with any field or a `retry_attempt`.
- The README states that unknown enum values fail decode, so peers must be one build. That holds in 1.1.0: the transport is in-process and there is no session route.
- `GwzErrorCode.capacity = 76` is appended and `check_log_additive` strips exactly 73–76. CLI `--json` writes `"Capacity"`, pinned by gwz-cli g15. gwz-py's `raise_for_response` is code-agnostic.
- `keeps_its_code_on_a_network_row` replaces four duplicated lists with the same three codes plus Capacity. The 75 helper text still wins over 76 for helper waits: helper slots are taken in prepare and never surface as `LocalWaitExpired`.

**Qualification retirement (area 2):**
- The cfg is gone from src, build.rs, Cargo, workflows and scripts in all three repos.
- Every former union gate is now `all(gwz_transport_candidate, any(unix, windows))`, which on Unix is identical to the old `all(unix, gwz_transport_candidate)` arm.
- The deleted refusal leaves the libgit2 callbacks untouched for non-SSH/HTTPS remotes, so Unix behaviour is unchanged.
- The Windows `file_identity: false` does not affect gwz-py, whose check uses the ordinary handler.

**cfg discipline (area 3):**
- The checker shows nothing new, and its allowlist is unchanged.
- There is no `#[cfg]` on a `use`, no `cfg_attr`, and no new `#[path]`. `fb439146` removed the one `#[path]` that `dc17d80c` added.
- The Windows modules (`agent_source`, `agent_pipe`, `agent_pageant`, `pageant_exchange`, `machine_proxy`, `ssh_home`, `ssh_host_key`, `native_commit`, `kex_retry`, `https_auth/view`, paths) use `cfg_if` arms only.
- Dead-code note, a follow-up and not a finding: a blanket `#[allow(dead_code)]` on `agent_pageant` (`endpoint/mod.rs:94-95`), stale allows in `pageant_exchange.rs:170-172` and `sys.rs:127`, and the test-only `Exchange::sid()` (`pageant_exchange/sys.rs:121`).

**Agent selection (area 4):**
- The order is Pageant (`FindWindowW("Pageant","Pageant")`) even when `SSH_AUTH_SOCK` is set, then the `SSH_AUTH_SOCK` pipe, then `\\.\pipe\openssh-ssh-agent`. It is chosen once per runtime and cloned into the endpoint and the handoff. `open` matches exactly one variant, with no fallback.
- The grammar admits only `\\.\pipe\<name>` with a case-insensitive namespace. It refuses UNC (including localhost), `\\?\`, `//./`, MinGW and file paths, empty, `.`, `..`, a name containing `\`, `/` or NUL, non-Unicode values and over 256 UTF-16 units. An empty `SSH_AUTH_SOCK` is unset, as before on Unix.
- A refusal is carried by `AgentSource::Refused` and raised only by `open()` in the agent step, after the handshake and before any key is offered. HTTPS and local operations are unaffected. This matches design B07 ("no key is offered"); the plan's "before any connection opens" wording is looser than the design.
- The Pageant pin is checked before and after each exchange, and a mismatch refuses and never selects the pipe.
- The send cap is a fixed 120 s, never 0. Mapping and `SendMessageTimeoutW` stay inside `pageant_exchange`.

**HTTPS (area 5):**
- Scheme selection is case-insensitive. Negotiate and NTLM use `Identity::CurrentLogon` only (no `Identity::Explicit` caller). Basic is answered only under `WindowsConfigured`. Digest alone is refused and beside another scheme is ignored. A rejected logon is terminal.
- Proxy: the grammar, D1, D6 (a refusal applies to https opens only, and the configuration builds), bypass rules, `GlobalFree` on every exit, and environment and `http.proxy` ignored on Windows. I found no form that goes direct where the guide says refuse.
- `git.exe` is found on absolute `PATH` entries only, `.exe` only. `GIT_CONFIG_GLOBAL=nul` and `GIT_CONFIG_NOSYSTEM=1` are set only for the view-verified invocations, with the discovered view (including `credential.helper=manager`) passed in `GIT_CONFIG_PARAMETERS`, so GCM is still found.

**KEX retry (area 8, D14):**
- It is retried once, only for `Session(-8)`, only while facts show no credential offered, only with budget left greater than what the first try spent, and inside one setup attempt. Unix is `Never` through a `cfg_if` policy arm. The `Steps` seam is test-only injection over the two network calls.

**T7:** the stall text names `--ssh-timeout` and its value with the attempt inside the brackets. With `0` it names no bound.

**D11:** the Windows quoting refuses `$`, backtick, `%`, `!`, straight and curly quotes, non-drive paths, `\\` runs and control characters.

**D12:** the trust finding rides in the handoff, bounded to 256 entries, keyed by lower-cased host and port.

---

## 3. Follow-ups outside the finding bar

- gwz-transport `Gh` and `Anonymous` are still advertised on Windows (`transport_host/mod.rs:120-131`). Under them, a Negotiate plus Basic challenge goes to the helper. This is unreachable from gwz-core's own callers, but it matters for the 1.2.0 session route (TD1).
- The Pageant pin compares the creation time only through `PartialEq`, and relies on the held process handle (`agent_pageant/sys.rs:130-134`). Without a handle it is HWND plus PID.
- A note printed after `OperationFinished` redraws the cleared progress line (`progress_detail.rs:251-262`).
- New process-globals debt entries: `env::vars_os` in `native_commit/environment.rs`, and `env::current_dir` in `local_command.rs` and `transport_host/mod.rs`. The native route opens with libgit2 `FROM_ENV` (live environment) while deciding from a snapshot. These should go to the operator as structural debt.
- `ssh setup timeout: aggregate` is not worded; T7 covered stalls only.
- `ssh_handoff` evicts trust findings with `pop_first` (by key order) beyond 256.
- Plan 4.7 and the guide should be updated with 4.7's proxy evidence.

## 4. Risks and next action

**Risks (not findings):**
- The real Pageant 0.83 rows are owed to the operator's console session.
- 6.1's HTTPS large-pack throughput row is the one `unported` entry.
- Two hypotheses on the Down machine are unconfirmed. Under `--ssh-timeout 0` a retest with no clock parks arrivals indefinitely. A retest whose throttle the limit machine takes as evidence is not counted as a failed retest (`completion.rs:219-226`, `https_endpoint/retry.rs:167`). Both deserve a test.
- All Windows behaviour in this review rests on code reading plus the dabeest receipts named in the commits; nothing Windows-only was run here.

**Next action:** fix P2-1 (implement the §5.3 Down wording, or amend the design and docs) and P2-2 (a fail-safe system-config decision from the resolved `git`), each with the closure test named. The P3s can land in the same revision or be scheduled. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.