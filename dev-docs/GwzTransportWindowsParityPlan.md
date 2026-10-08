# GWZ transport: the rest of Windows parity for 1.1.0, a phased plan

Date: 2026-10-08. Status: **revision 3, reviewed GO on the Consistency axis in round 3 (GwzTransportWindowsParityPlan-ReviewConsistency-3.md), its one P3 (N3-2) applied at filing; the operator decided every open question of section 8 as recommended on 2026-10-08.** Revision 2 was reviewed NO-GO on N2-1 (GwzTransportWindowsParityPlan-ReviewConsistency-2.md). Revision 1 was reviewed NO-GO in round 1 (GwzTransportWindowsParityPlan-ReviewConsistency.md). Drafted for the lane owner, who reviews it and files it in `gwz-core/dev-docs`. It decides nothing: every choice that needs the operator is an OQ in section 8, with options and a recommendation. It authorizes no implementation, commit, push, tag or publish. Revision 2 resolved round 1's five P2s and twelve P3s, which round 2 confirmed closed; revision 3 resolves round 2's one P2 (N2-1) and one P3 (N3-1); `GwzTransportWindowsParityPlan-RemPlan.md` maps each finding to its fix (changelog at the end).

Read-only sources: workspace root `04fb8daa`; gwz-core `0e21bdde` (its tree equals HEAD apart from one untracked bug report, which this plan does not read); gwz-transport `9eef731`; gwz-cli `d702f53`; gwz-py `5950ba3`; gwz-sspi `364ccc7`. Every `file:line` below is at that tuple unless another commit is named. Nothing was run on dabeest for this plan. Where a fact needs a Windows check, the plan has a step or an OQ for it.

**Labels.** Step numbers (`0.1`, `3.5`) are this document's own. A letter suffix (`2.3b`, `3.2a`, `3.2b`) splits or extends a revision-1 step; no revision-1 step number moved. Steps 0.5 (added by the lane owner after revision 0), 1.7, 1.8 and 4.11 are new. Requirement IDs (TR1.8, TR4.6 to TR4.10, TR8.4, S4.1 to S4.5, OD13, OD15, OD16) are the release plan's and its amendments', cited as they stand. Baseline rows `B01` to `B18` and primitives `P01` to `P08` are `GwzTransportWindowsBaseline.md`'s. New baseline rows this plan proposes are `X1` to `X10` (step 2.2). Table rows `U1` to `U27` (section 3) list OS calls; rows `G1` to `G76` (Appendix B) list every Unix-only gate in the transport's scope.

## 1. Why this plan, and what is already decided

- **OD13 (operator, 2026-10-01).** 1.1.0 is the transport used in process by the `gwz` CLI on macOS ARM64, Linux x86-64 and Windows x86-64, with Windows parity (`dev-docs/CurrentProgramCheckpoint.md:1411-1413`; amendment 2 header).
- **OD15 (operator, 2026-10-01).** Windows parity is built into the transport. Native routes are not the way to it (`CurrentProgramCheckpoint.md:1445`; memory `no-native-route-fallbacks`). TR1.8 designs it, TR4.8 to TR4.10 implement it (`GwzTransportReleasePlanAmendment-2.md` §3.5).
- **OD16 (operator, 2026-10-02).** No zone bound: the logon session's default credentials go to any host that answers `Negotiate` or `NTLM`, as 1.0.17 does (`CurrentProgramCheckpoint.md:1453` ff.).
- **What has happened since.** WH1 (Windows HTTPS integration) was accepted with limits on 2026-10-04. Every transport change after it (option A, idle loss, the adaptive concurrency design and its Phase 1, the HTTPS fixed-cost fix) was built and tested on macOS and Linux only. Windows has no SSH transport at all.
- **The 2026-10-08 check on dabeest.** The Windows HTTPS candidate at gwz-core `2a12006f` equals WH1 on every row (`gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-08-windows-candidate-check/README.md`, uncommitted at the time of writing). So Unix-only design has not yet broken what Windows has. It has also not moved Windows forward, and each new Unix-only piece makes the later port larger.
- **What this plan is for.** Schedule the remaining Windows work now, foundational first, so that several agents can take steps at once, and so that the next transport change cannot add a Unix-only dependency without it being counted (step 0.3).

## 2. Where Windows stands today

### 2.1 Build and CI

- **No push-triggered Windows job for the ordinary builds yet.** Amendment 2 §2: "No push-triggered CI job builds or tests gwz-core on Windows"; `windows-matrix.yml` and `platform-matrix.yml` run on dispatch only (`GwzTransportReleasePlanAmendment-2.md:63`). The candidate part of TR4.6 is committed, not yet pushed: gwz-core `0e21bdde` adds the `candidate-windows` job (on `windows-2022`) to `transport-candidate.yml` (step 0.1). The ordinary part, gwz-core's and gwz-cli's ordinary suites on push, is in progress in lane `gwz-dev-winci`.
- **The Windows candidate is HTTPS-only.** `src/git/mod.rs:5` compiles `endpoint` on Windows only under `all(windows, gwz_transport_candidate, gwz_windows_https_qualification)`. The qualification cfg appears at 41 sites in gwz-core `src/` and in gwz-cli (7 files) and gwz-py (6 files).
- **The Windows full-lib red tail is cleared in the tree.** At gwz-core `2a12006f`, `cargo test --lib` on Windows gave 2170 passed, 125 failed. 115 were `workspace_ops::merge::v1_lifecycle` tests that need the real-Git fixture (the fake-Git default; `windows-matrix.yml` sets `GWZ_TEST_GIT=real`). 10 were `git::endpoint::placement_endpoint::retry_tests`, which failed with `Kind(InvalidInput)` because `scripted_endpoint` passed `PathBuf::from("/tmp")` as the SSH home, which is not absolute on Windows. gwz-core `1cdb9557`, an ancestor of `0e21bdde`, fixed the second group (`placement_endpoint/retry_tests.rs:140-142` uses `std::env::temp_dir()`). With it and `GWZ_TEST_GIT=real` the whole Windows candidate lib suite passed 2,295, with 0 failed and 2 ignored, on dabeest (evidence run `2026-10-08-windows-httpsfix-port`, README; that run's tree is the lane's working tree, not `0e21bdde` itself, so step 0.2 asks for a hosted-runner confirmation).
- **Dead-code noise.** The Windows build compiles SSH and `agent_job` modules it never reaches; gwz-core lib warnings went from 143 to 152 (same check). They disappear when SSH is wired in.

### 2.2 What the Windows transport does today (WH1)

Accepted limited (`dev-docs/GwzWindowsHttpsIntegrationImplementationAcceptance.md`): HTTPS with `Anonymous` and `WindowsDefault` (SSPI NTLM or Negotiate-to-NTLM in the contained `gwz-sspi` worker, with the final-origin channel binding), WinHTTP machine proxy admitted only if verified DIRECT (`transport_host/endpoint_environment.rs:203-240`). SSH, Gh and configured-helper policies refuse (`transport_host/mod.rs:87-88, 188-190, 222-224, 280-282`; `session.rs:459-462` schemes `vec![Scheme::Https]`).

Still NO-GO there: ordinary Windows activation, **WH2** (configured helpers on Windows: helper Job and path portability, `GwzWindowsHttpsIntegrationDesign-DRAFT.md` "WH2"), **WH3** (integrated deadline, cancellation and identity adversity; installed paths with spaces or Unicode; worker provenance; pool reuse and concurrency; effects and retry adversity), provider parity (Kerberos/domain, Digest, proxy and native 407, Windows SSH and Pageant, differing identity, stalled provider), TR1.8, the platform, performance, package and aggregate gates.

### 2.3 What 1.0.17 does on Windows (the parity target)

Known from executed rows (`GwzTransportWindowsBaseline.md` §3, released 1.0.17 archive `bb9e3720...`):

| Behaviour | Row | Result |
|---|---|---|
| SSH home with `HOME` unset: `HOMEDRIVE`+`HOMEPATH`, or `USERPROFILE` alone | B03 | authenticates |
| Missing, nonexistent, relative, spaces-only `HOME` | B04 | authenticates |
| Empty `HOME`; Unicode-only or Unicode-plus-spaces path; an existing first home without a good `known_hosts` | B04 | refuses before any key offer |
| Pageant alone, pinned PuTTY 0.83 | B05 | authenticates (RSA SHA-256 agent signature) |
| Neither agent | B07 | refuses, no key offered |
| Machine proxy for a reserved `.invalid` origin despite a conflicting `HTTPS_PROXY`; numeric loopback origins bypass it | B09 | partial |
| Proxy 407 `Negotiate` or `NTLM` | B10 | refuses: zero credential offers, a native string-conversion error |
| Pageant and OpenSSH agent both running | B06 | **unexecuted** |
| An OpenSSH-for-Windows agent pipe (native service absent on dabeest, error 2) | B08 | **partial**; no pipe authentication ran |
| Negotiate-only, NTLM-only, mixed, Digest, EPA, POST challenge | B11 to B14, B16, B18 | **unexecuted** |
| Redirect across zones | B15 | **partial**: URLMON zone classification executed (`gwz-tr18` Intranet, `gwz-tr18.invalid` Internet); the released redirect and authentication assertions are not (`GwzTransportWindowsBaseline.md:303`) |
| The same challenge on macOS and Linux | B17 | **partial**: Mac plain-HTTP `Negotiate` characterized as unsupported; Mac HTTPS blocked by native trust; Linux unexecuted (`GwzTransportWindowsBaseline.md:305`) |

What 1.0.17's SSH credential callback offers (`src/git/gitbackend/transport_support.rs:240-280`, and the selected-identity form at `:219-228`): the ssh-agent once (`Cred::ssh_key_from_agent`, `:260`), a username, the configured credential helper for user and password, and default credentials. It has **no keyboard-interactive branch** and passes `None` as the passphrase for a selected key file (`:228`, error text at `:224`). So parity needs neither keyboard-interactive nor passphrase prompts. A URL password is offered through libssh2 first when the server lists `password` (`ssh_password.rs:1-20`; platform-neutral).

What libssh2 does on Windows (`libssh2-sys` 0.3.3, the version both 1.0.17's `libgit2-sys` feature `ssh` and the candidate use, `git2-rs/libgit2-sys/Cargo.toml:26,37`; candidate pin `tests/transport_backend/prepare.py:78`):

- **Agent order.** `supported_backends` is Pageant, then OpenSSH, and `libssh2_agent_connect` takes the first that connects (`agent.c:436-441, 816-826`). Pageant "connects" if `FindWindowA("Pageant","Pageant")` finds a window (`agent.c:343`), so a visible Pageant with no keys never falls through to the pipe. The OpenSSH backend uses `SSH_AUTH_SOCK`, else `\\.\pipe\openssh-ssh-agent` (`agent_win.c:124-139`), connects with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, and checks nothing about the pipe's server.
- **Pageant's wire shape.** A mapping named `PageantRequest%08x` (the thread ID) with no security attributes, 8192 bytes, `SendMessage` with no timeout (`agent.c:366-394`). TR1.8's design is deliberately stricter (section 2.4).
- **Crypto backend.** WinCNG unless the `openssl-on-win32` feature is on (`libssh2-sys` `build.rs:97-114`). WinCNG defines `LIBSSH2_ED25519 0` (`wincng.h:74`). So **ed25519 host keys and ed25519 file keys are not available in libssh2 on Windows**, in 1.0.17 and in the transport alike. The transport's host-key preference list names `ssh-ed25519` (`ssh_network.rs:24`, `preferences` at `:346`), and `libssh2_session_method_pref` strips unsupported names and fails if none are left (`kex.c:4192` ff.). What 1.0.17 does with an ed25519-only `known_hosts` entry on Windows is unknown (new row X2, step 2.2).

### 2.4 Windows design state (TR1.8)

`GwzTransportWindowsParityDesign.md` is a DRAFT and **NO-GO**. Its header: "No downstream Windows implementation may consume this as GO." It predates MAIN's accepted helper-timing, configuration-view and SSH-clock amendments and carries an SSPI-only supersession list; `GwzTransportWindowsProofDispositions-DRAFT.md` §5 says to refresh its controlling graph before settling, and its own §11 sets the GO rule (every baseline row B01 to B18 and primitive row P01 to P08 executed, and provisional clauses replaced by one physically proved design) and the post-GO sequence. It proposes, among others: Pageant through `WM_COPYDATA` with a `Local\` mapping that has an ACL, a single `SendMessageTimeoutW`, and a pinned window identity (§4, §5); an owned `AgentSource` enum rather than a `PathBuf` (§4); the machine proxy winning over the environment, with 407 refused unless 1.0.17 authenticates (§6); helper-identity precedence over `Negotiate`, `NTLM`, `Digest`, `Basic` (§7); a channel binding taken from the final origin handshake (§8); HOME order `HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE` (§3).

Open physical rows before it can freeze (baseline §3, §4, and its remaining-rows table, `GwzTransportWindowsBaseline.md:335-359`): B06, B08, B09 with P04, B11 to B14, B15 (partial), B16, B17 (partial), B18; P01 (cross-SID), P02 (numeric HWND reuse, unexecuted), P03 (native service identity), P05 (blocked provider), P06 (MD5 peers, EPA), P07 (Digest). Blocked on operator approvals recorded in the baseline: Windows native trust ("held"), a distinct account, a coordinated service or agent setup, and macOS native trust for B17's Mac row (root's request is unanswered, `GwzTransportWindowsCheckpoint.md:39`). `GwzTransportWindowsProofDispositions-DRAFT.md` proposes dispositions for HOME (§1, B04), Pageant timeout ownership (§2, P02), window identity (§3, P02) and weak certificate hashes (§4, P06); none is accepted. Under §11 no row may stay open at GO: step 2.1 gives every row that cannot run a recorded disposition, and a claim whose row stays unexecuted is removed from the design, not marked provisional.

### 2.5 gwz-sspi

- Status line: caller values, private codec, parent supervision, the serial Windows Negotiate/NTLM worker and installed-host packaging "accepted within their bounded gates. HTTP/core composition, full Windows qualification and release remain gated"; Digest is refused (`gwz-sspi/README.md`; its body also says installed-host packaging "has its own pending review").
- Publication gate: `publish = false`; `release_checks.py` refuses preparation and publication until the guard is deliberately lifted "following implementation acceptance and Windows qualification" (`gwz-sspi/RELEASE.md:126-128`, "SSPI release gate").
- Registry: the placeholder `0.0.0-bootstrap.1` has been on crates.io since 2026-10-04; trusted publishing only; the publisher's environment name is unconfirmed (O1). gwz-cli's crates.io publication of 1.1.0 waits on `gwz-sspi 0.1.0`, and gwz-py appears to (O2). The activation's review route and date are unset (O3). Release step 5a is the new prerequisite of step 6. O1 to O3 and step 5a are in amendment 2 §3.21, which is revision 7 of that amendment, "DRAFT, not yet reviewed" (its status line); the checkpoint entries it draws on are the firmer source. Step 5.5 schedules the activation after the implementation acceptance and the Windows qualification evidence that `RELEASE.md` requires.

## 3. Why SSH setup is Unix-only, and what maps to what

The SSH path is not process- or fd-heavy. It is **threads plus non-blocking sockets plus sliced waits**: setup runs as a supervised job on its own thread (`agent_job.rs:194` `start_setup`, `Job::start`), every wait goes through `Control::wait_step`, which hands a closure at most 20 ms (`agent_job/control.rs:153-190`), and the closure is the only place the OS appears. libssh2 does the protocol and the I/O on a `std::net::TcpStream` made non-blocking (`ssh_connection.rs:22-33`). The worker loop is a parked thread (`ssh_worker/runner.rs`). Idle sockets are watched by a small tokio reactor on a duplicated socket (`idle_watch.rs`). So the Unix-only pieces are a short list of OS calls, not an architecture. Each, with its Windows counterpart:

| # | Where | Unix dependency | Windows counterpart | Step |
|---|---|---|---|---|
| U1 | `git/endpoint/mod.rs:21` | `idle_watch`, `ssh_password`, `ssh_setup` compiled on Unix only | the same modules, once U3 to U9 are portable; `ssh_setup.rs:4` imports `idle_watch`, so all three compile together | 1.4 (compile), 1.5 (idle and close behaviour) |
| U2 | `git/endpoint/mod.rs:47-62` | SSH, HTTPS and password test fixtures compiled `all(test, unix)` | Windows fixtures (U25) | 1.1 (the SSH fixtures); the block's other modules by Table B.2 |
| U3 | `ssh_network.rs:5, 16-17, 446` | whole module is a `cfg(unix)` arm; `os::fd::AsRawFd`, `os::unix::fs::OpenOptionsExt` | a Windows arm in the same `cfg_if`, sharing the portable body | 1.2 |
| U4 | `ssh_network.rs:213-216` | connect in progress is `EINPROGRESS` or `EALREADY` | `WSAEWOULDBLOCK`, which `std` reports as `ErrorKind::WouldBlock` (already matched at `:213-214`) | 1.2 |
| U5 | `ssh_network.rs:238-243` | `libc::poll` on the connecting socket | `WSAPoll`, or `select` with an except set (see step 1.2 on the failed-connect defect) | 1.2 |
| U6 | `ssh_network.rs:321-334` | `libc::poll` for libssh2's blocked direction; `session().as_raw_fd()` at `:326` | same wait on `as_raw_socket()` (`ssh2` `Session: AsRawSocket`, `ssh2-0.9.6/src/session.rs:1140`) | 1.2 |
| U7 | `ssh_network.rs:137` | `known_hosts` opened with `O_NONBLOCK` so a FIFO cannot block the open | open, then `GetFileType`; refuse device names and `\\.\` or `\\?\` device paths before opening | 1.3 |
| U8 | `ssh_key_snapshot.rs:152-172` | key file read with `O_NONBLOCK`; Windows arm is `Unsupported` | the same regular-file reader as U7 | 1.3 |
| U9 | `ssh_worker/endpoint.rs:135-157` | identity-file check with `O_NONBLOCK`; Windows arm `Unsupported` | the same reader | 1.3 |
| U10 | `agent_socket.rs:3-75` | `AF_UNIX` connect, `libc::poll`, `EISCONN`, `EINPROGRESS` | OpenSSH-for-Windows agent: named pipe `\\.\pipe\openssh-ssh-agent` or the pipe `SSH_AUTH_SOCK` names, overlapped I/O, `WaitNamedPipe`, `CancelIoEx`; Pageant: `WM_COPYDATA` and a mapping. Both implement `agent_client::Channel` (`agent_client.rs:9`) | 3.3, 3.5, 3.6 |
| U11 | `agent_auth.rs:4, 216` | module is `cfg(unix)`; the signature buffer is `libc::malloc`ed because libssh2 frees it ("Windows is deliberately unadmitted", `:213-215`) | the same call is correct if libssh2's C code and Rust's `libc::malloc` share a CRT heap; prove it, do not assume it | 3.4 |
| U12 | `ssh_key_auth.rs:19`, `ssh_local.rs:4` | `cfg(unix)` around code that uses only `ssh2` and `ssh_network::wait_session` | ungate; no new OS call | 1.4 |
| U13 | `agent_job.rs:194` | `start_setup` is `cfg(unix)`, only because `ssh_setup` is | ungate | 1.4 |
| U14 | `ssh_password_helpers.rs:30, 142` | the password-only helper `lookup` is `cfg(unix)` (it reuses the HTTPS helper machinery, U20 to U23) | WH2's helper runner | 4.5 |
| U15 | `idle_watch.rs` | `tokio::net::TcpStream::from_std` on a `try_clone` and `poll_peek` | the same code under tokio's Windows driver; `GwzTransportIdleLossDesign.md` §9 says it "needs its own run" | 1.4 (it compiles with `ssh_setup`), 1.5 (the run) |
| U16 | `ssh_setup.rs:578`; `ssh_tests/max_startups.rs` (`cfg(unix)` throughout) | a `MaxStartups` drop that a platform reports as `ConnectionAborted` maps to `Cancelled`, which the retry machine returns to the member | pin what Windows reports (`WSAECONNABORTED`, `WSAECONNRESET`) and map a pre-authentication drop to the `Io` code that today's `classify` returns as `Verdict::Retry`, as macOS and Linux already get (`max_startups.rs:1-12`; `GwzTransportAdaptiveConcurrencyDesign.md` F3, §12, and §10.2 item 20) | 3.7 |
| U17 | `transport_host/endpoint_environment.rs:20-23, 59-64, 177-200` | SSH home is `HOME` when absolute; agent is `SSH_AUTH_SOCK`; Windows `platform` has only `direct()` (`:203-240`) | HOME order and an `AgentSource` (TR1.8 §3, §4) | 3.1, 3.2a, 3.2b |
| U18 | `transport_host/mod.rs:43, 87-88, 138-141, 188-190, 222-224, 280-282` | `ssh_local` import is `cfg(unix)`; `SshSettings.agent: Option<PathBuf>`; the qualification refusals | `AgentSource`; admit SSH | 1.6, 3.2a |
| U19 | `transport_host/session.rs:417-440, 459-462` | SSH engine construction is `cfg(unix)`; Windows schemes are `[Https]` | construct the SSH engine; schemes `[Ssh, Https]` | 1.6 |
| U20 | `https_auth/owner.rs:245-254`; `https_auth/lookup.rs:165-170` | helper tree killed with `killpg(SIGKILL)`; `process_group(0)` | Job Object with kill-on-close, assigned at creation (4.1's accepted WH2 contract; P08 executed) | 4.2 |
| U21 | `https_auth/executable.rs:30-35` | executable means `mode & 0o111` | `git.exe` found on the captured absolute `PATH`, no implicit cwd or shell-script extension (4.1's accepted WH2 contract) | 4.3 |
| U22 | `https_auth/runner.rs:3`, `view.rs:3`, `view/framing.rs:4`, `file_worker.rs:5, 68` | `OsStrExt` bytes; `O_NONBLOCK` | UTF-16 round trip without lossy conversion; the U7 reader | 4.3 |
| U23 | `https_auth.rs:10, 110, 128`; `https_worker.rs:60, 98, 113, 118, 135, 245, 252`; `https_worker/credentials.rs:19` | helper owner wiring behind `cfg(unix)` | admit `WindowsConfigured` and `Gh` | 4.4 |
| U24 | `endpoint_environment.rs:115, 122-170` | environment proxy | WinHTTP machine proxy (TR1.8 §6) | 4.6, 4.7 |
| U25 | `ssh_fixture.rs:73-74, 106-109, 256, 291, 306, 330` | test server is `/usr/sbin/sshd`; also `ssh-keygen`, `kill -STOP`, `ps -axo` | a Windows SSH server fixture and process controls (OQ6) | 1.1 |
| U26 | `helper_script.rs:23, 78` | `PermissionsExt`, Linux-only warm-up | `.cmd` or `.exe` fake helpers | 4.3 |
| U27 | `Cargo.toml:103-108`; `tests/transport_backend/prepare.py:87` | `windows-sys` features are `Win32_Foundation`, `Win32_Globalization`, `Win32_Storage_FileSystem`; the candidate adds only `Win32_Networking_WinHttp` | add `Win32_Networking_WinSock`, `Win32_System_Pipes`, `Win32_System_IO`, `Win32_System_Threading`, `Win32_Security`, `Win32_UI_WindowsAndMessaging`, `Win32_System_Memory`, `Win32_System_JobObjects` as each step needs them, in the candidate's extra list first | 1.2 |

**Conclusion for the readiness model (OQ2).** Nothing in the SSH path needs a different architecture on Windows. The same thread-per-setup-job model, the same 20 ms sliced waits and the same tokio idle reactor are portable. Recommend reusing them; the choice is open only for the wait primitive inside step 1.2.

**This table lists OS calls, not gates.** The gates the transport's tests and fixtures carry, and the production gates beside them, are many more than the 27 rows above: Appendix B lists every one at gwz-core `0e21bdde` (76 files, 161 gate and OS-call lines, rows `G1` to `G76`), each with an owner step. Step 0.3 seeds its inventory from Appendix B, and S4.5's "no skipped Unix-gated tests" is measured against it (step 5.1).

### 3.1 How option A, idle loss and the adaptive machine reach Windows

They are designed in by being OS-free above the closure, then proved on the Windows leg, not retrofitted:

- **Option A (`GwzTransportSshBackgroundCloseDesign.md`).** A fetch completes at libssh2's close, a closing exchange is discarded by terminating the socket (D5, §5), and the worker's 1 ms park keeps running during a close (`GwzTransportSshBackgroundCloseDesign.md:76`). The only OS touches are `SshConnection::terminate` (`ssh_connection.rs:44`, `shutdown(Both)`) and the `SshChannel::poll_dispose` change. Step 1.5 runs option A's tests on Windows, including "a discarded connection is gone within one pass". TR8.4 measures it.
- **Idle loss (`GwzTransportIdleLossDesign.md`).** `IdleReactor` and `IdleSocket` (`idle_watch.rs`) are Unix-only today only because `ssh_setup` is (§9, decision 6). Step 1.5 runs the idle tests under tokio's Windows driver, as §9 asks.
- **Adaptive concurrency (`GwzTransportAdaptiveConcurrencyDesign.md`).** The window sets, filter, machine and retry machine are pure state machines (L1, L2) and OS-free. Windows owes the L3-S SSH row on its own leg (§10.2 item 20) and the pinned error kind for a `MaxStartups` drop (F3, in §12). Phase 1's removal of the 64-caps, the per-host `Supervisor` and the local-wait clock are portable. Step 3.7 pins the Windows error kinds and maps a drop to today's retriable class; the design's `Suspect` class is not in gwz-core or gwz-transport at the tuple, and arrives with its limit-discovery machine. Windows connect failures under 32 to 64 simultaneous connects (ephemeral ports, `TIME_WAIT`) are a TR8.4 observation, not a design change.

## 4. How the plan is shaped

- **Phases** are milestones, foundational first. **Steps** have one goal and aim at under 500 lines of change (an aspiration). Steps in a phase touch different files except where the hot-spot list says otherwise, so several agents can take them.
- **Every step's definition of done** (not repeated below):
  - tests written first, failing before the change;
  - the ordinary suite and both candidate legs stay green on macOS and Linux (`scripts/run_tests.py`; `transport-candidate.yml`'s two legs);
  - the Windows CI leg (step 0.1) stays green;
  - `check_cfg_boundaries.py` and `check_process_globals.py` stay clean. New Windows code sits in a `cfg_if` arm or an enclosing Windows module, never a bare `#[cfg]` on an import, and control-flow bodies are braced (root `AGENTS.md`). No `static` mutable state and no `thread_local!`; unique names such as mapping names come from the runtime's `IdSource` (TR1.8 §2);
  - any new `cfg(unix)` or `cfg(windows)` gate is in the step 0.3 inventory with its owner step;
  - fixtures and helpers live in `src/` test modules, never under `tests/` as production code (memory `no-production-code-in-tests`); dead code found on the way is removed in the same change;
  - the dabeest proof, when the step names one, is archived in gwz-core-evidence under a new immutable label, never overwriting a receipt (`EVIDENCE.md`).
- **Review tiers** (`GwzProcessOptimization.md` §8; memory `review-granularity`). **Phase review** is the one Consistency plus Safety review of the phase's whole diff at its end. A **skim** review replaces it only where the operator decides so (OQ16); until then every phase review is the standard one. **Dual** is a per-step dual review, used only for the wire format (anything a released client or server reads), secrets and the release gate; §8 says the checkpoint names those steps when a phase starts. This plan names them: 2.4, 3.5 (a frame Pageant 0.83 reads), 4.1, 4.2 with 4.3 (WH2's secret and process boundary: "WH2's secret/process boundary has mandatory dual review", `GwzWindowsHttpsIntegrationDesign-DRAFT.md:287`), 4.4, 4.8, 5.5, 5.6, 6.5. A step that a reader might expect to carry a dual review and does not says why (for example 3.6). **Surface** is added where a phase freezes something a Windows user sees.
- **dabeest rules** (V110 §2; memory `dabeest-windows-builds`): `ssh gianni@dabeest` with `ClearAllForwardings=yes`, MinGW bash, work under `/e/gwz-tests/<label>`, Rust `+1.95.0` MSVC, no ancestor Cargo config, never change accounts, trust, services, proxies, zones or policy, install nothing globally. Section 7.2 lists which steps need the host, because it is shared.
- **Hot spots** (files several steps touch, with the order they land in; the first step lands the structure, later steps fill their own part). Memory `candidate-prepare-symlinks`: the candidate tree links `src` and `tests` into the real gwz-core, so copy with `cp -RL` before editing in one.
  - `git/endpoint/ssh_network.rs`: 1.2 lands the Windows arm (U3 to U6), then 1.3 rewires `read_regular` (`:133-157`). The two can be developed in parallel; they merge in that order.
  - `git/endpoint/ssh_setup.rs`: 1.5 (`:401-417`), then 3.2a (the agent path), then 3.7 (`:578`). `git/endpoint/ssh_local.rs`: 1.4, then 3.2a, then 3.2b and 3.6.
  - `transport_host/mod.rs`, `transport_host/session.rs`, `transport_host/endpoint_environment.rs`: the order follows dependency. 1.6 lands first. Then the policy-free steps, 3.2a and 4.3 (`endpoint_environment.rs:75-83`), merge as soon as each is ready, in either order, and never wait for TR1.8's GO. After 2.4, the TR1.8-gated steps merge in this order: 3.1, 3.2b, 4.4 (`mod.rs:289-331`, `session.rs:459-489`), 4.6 (moves `endpoint_environment.rs:203-240` into `machine_proxy.rs`) and 4.7 (`:115`). The order binds only the gated steps, and its one hard constraint is that 3.1 adds `ssh_home` to the Windows `platform` module before 4.6 moves the proxy code out of it. Phase 4 lanes are developed beside Phases 1 and 3.
  - `git/endpoint/mod.rs:21` and `:47-62`: each step edits only its own modules of the block (Appendix B, Table B.2): 0.5, 1.1, 1.4, 1.5, 1.7, 1.8, 4.3, 4.11.
  - `tests/transport_backend/prepare.py:87` and `Cargo.toml:103-108` (the `windows-sys` feature list; step 1.2 turns the list into one constant in `prepare.py`, so later steps add a line, not a conflicting edit; 3.4 also touches the `libc` dependency).
  - The three candidate-switch inventories (`scripts/candidate_switch_inventory.txt` in gwz-core, gwz-cli and gwz-py): 1.6, then 5.1, 5.2 and 5.3 (any other step that moves a `gwz_transport_candidate` site's file or symbol updates it too).

## 5. Phase 0: Windows CI and compile gate (TR4.6)

**Milestone.** Every push and pull request builds and tests the Windows shapes, and a lane cannot add a Unix-only dependency to the transport unseen. This is the prerequisite for everything below: without it each Windows step is judged by a manual dabeest run.

### Step 0.1: the `windows-2022` leg (TR4.6)

- **State (2026-10-08).** The candidate part is committed, not yet pushed: gwz-core `0e21bdde` adds `candidate-windows` to `transport-candidate.yml` (prepare, the candidate checked three ways, WH1's five qualification tests with a run-count check, `run_tests.py --lib` with real Git and the source checks, five integration targets; bash steps; about 15 minutes of steps on dabeest, 25 to 35 expected on a hosted runner). The ordinary part, gwz-core's and gwz-cli's ordinary suites on push, is in progress in lane `gwz-dev-winci`.

- **Goal.** Land TR4.6: on every push to main and every pull request, a Windows job builds gwz-core's and gwz-cli's ordinary builds, runs their ordinary suites, and runs the conditional-compilation check; after S4.5, it also builds the candidate and runs every candidate test that needs no fixture (`GwzTransportReleasePlanAmendment-2.md` §3.5). **The image is pinned.** TR4.6 says `windows-latest`; the leg uses `windows-2022` (`transport-candidate.yml`, job `candidate-windows`). That is a deliberate refinement of TR4.6, not a change of its intent: this plan reads it as pinning the runner image so that an image upgrade cannot change a green leg unannounced (the commit message of `0e21bdde` gives no reason). If the operator wants the label as TR4.6 writes it, one line changes.
- **Files.** `gwz-core/.github/workflows/transport-candidate.yml` (the leg), possibly `gwz-cli`'s workflow.
- **Acceptance this plan adds** (so the leg is useful to every later step): it builds three shapes (ordinary; `--cfg gwz_transport_candidate`; and that plus `gwz_windows_https_qualification`), as the 2026-10-08 check did; it sets `GWZ_TEST_GIT=real` as `windows-matrix.yml` does; it runs under bash, not PowerShell, so a failed command fails the step (`CurrentProgramCheckpoint.md`, "CI repaired after the push"); it records the three shapes' warning counts.
- **Tests first.** The existing workflow-text tests (`scripts/checks/test_check_candidate_switches.py`) gain a row that the Windows leg exists and names the three shapes.
- **dabeest.** None.
- **Review.** None beyond Phase 0's.

### Step 0.2: make the Windows leg green enough to require

- **Goal.** Confirm on a hosted runner that the red tail the 2026-10-08 check found is gone, make `prepare.py` run on a hosted runner without symlink privilege, and run `test_prepare.py` where this step claims it runs.
- **State (2026-10-08).** Done in the tree: the 10 `retry_tests` (gwz-core `1cdb9557`, `placement_endpoint/retry_tests.rs:140-142`); with it and `GWZ_TEST_GIT=real` the whole Windows candidate lib suite passed 2,295 on dabeest (evidence run `2026-10-08-windows-httpsfix-port`). The 115 `v1_lifecycle` failures were the fake-Git default, not a Windows defect. Still to do: the hosted-runner confirmation (the dabeest dry-run of the leg ran `prepare.py` there), and the exclusions below.
- **Files.** `tests/transport_backend/prepare.py` (it creates symlinks with `symlink_to`, which needs privilege on Windows: add a copy mode used on Windows); `tests/transport_backend/test_prepare.py`; the workflow from 0.1.
- **The leg's three exclusions** (the `0e21bdde` commit message and the comments in `transport-candidate.yml`), each with an owner: (1) the byte-comparing generator and Python checks stay on the Linux legs, because the Windows checkouts of gwz-transport and taut are CRLF; they test platform-neutral artifacts, so section 11 records them as out of scope; (2) `test_prepare.py` is left to Linux because of "a Windows path-escaping test bug": this step fixes that test bug and adds `test_prepare.py` to the Windows leg, so the row below runs where it is claimed to run; (3) `publish_workflow` is left to Linux (an `include_str!` through the symlinked `tests/` that Windows resolves differently): step 5.4 owns it, since TR4.6 has the leg run every candidate test that needs no fixture.
- **Tests first.** `test_prepare.py` gains a Windows-copy-mode row (the mode injected, so it also runs on macOS and Linux) and a row for the escaping fix; both run on the Windows leg. The hosted-runner run is the confirmation of the count: no `retry_tests` and no `v1_lifecycle` failure with `GWZ_TEST_GIT=real`.
- **dabeest.** One cold run of the three shapes on the leg's tree, only if the hosted-runner run differs from the lane's 2,295 (about 15 minutes of host time); otherwise none.
- **Review.** Phase.

### Step 0.3: the Windows-parity inventory and its ratchet

- **Goal.** A shrink-only inventory of every Unix-only gate in the transport, seeded from Appendix B (complete at gwz-core `0e21bdde`), so "Unix-only design stops piling up" is enforced, progress is countable, and S4.5's "no skipped Unix-gated tests" has a measure.
- **Files.** new `scripts/checks/check_windows_parity.py`, `scripts/checks/windows_parity_inventory.json`, `scripts/checks/test_check_windows_parity.py`; one line in the lane gate (`scripts/checks/check_lane_commits.sh`) and in `run_tests.py`'s checks.
- **Design.** Syntax-aware, as `check_cfg_boundaries.py` is (it already inspects disabled platform arms). It records each `cfg(unix)`, `cfg(all(test, unix...))`, `cfg(not(windows))`, `target_os`, `std::os::unix` and `libc::` use under `src/git/endpoint`, `src/transport_host`, `src/transport_setting` (the global-config lookup is part of the Windows home contract, step 3.1) and `src/git/gitbackend/transport_*`, plus `src/git/gitbackend.rs`, keyed by file, gate text and target (not line). The wider `src/git/gitbackend` grep also finds `preservation*.rs` and `commit_tag_characterization.rs`; they are outside the scope, with reasons (Appendix B, Table B.4). Each entry has an owner step from this plan and a state: `unported` (a Unix-only gate), `paired` (the gate has a Windows arm in the same `cfg_if`), or `platform` (permanent, with a recorded reason, for example `agent_socket.rs`'s `AF_UNIX` or an OpenSSL-only trust branch). It fails on: a new gate with no entry; an `unported` entry whose owner step is recorded done; a `paired` entry that loses its Windows arm; and a `libc::` or `os::unix` use inside a Windows arm unless its entry carries a `proof` field naming the evidence label (step 3.4's CRT-sharing proof is the first). Seed it with Appendix B's `G1` to `G76`, not with `U1` to `U26`; `U27` and the 41 qualification-switch sites (step 5.1) are separate counts.
- **Tests first.** Planted-defect rows in the style of `test_check_process_globals.py`: a new `cfg(unix)` import with no owner fails; a `libc::` use inside a Windows arm fails, and passes with a `proof` entry; a removed gate with a stale entry fails; a `cfg(all(test, unix))` module missing from the inventory fails. The seeded inventory passes at the tuple, so the checker's first run is clean, which is what "seeded from the complete list" means.
- **dabeest.** None.
- **Review.** Phase.

### Step 0.4: the Windows compile gate on every lane

- **Goal.** A lane that touches the transport cannot merge without a Windows compile of the three shapes.
- **Files.** new `scripts/windows_lane_check.py` (a `git archive` of the lane head, copied to a new `/e/gwz-tests/<label>` on dabeest, `cargo check` of the three shapes, receipt archived); a paragraph in gwz-core `AGENTS.md` naming the paths that trigger it (`src/git/endpoint/`, `src/transport_host/`, `Cargo.toml`, `tests/transport_backend/prepare.py`). The runners are the 2026-10-08 check's `wcc_*` runners, generalized (that run's `runner/`).
- **Design.** Two layers, both cheap. The static layer is step 0.3 and runs per commit in the lane gate. The compile layer is the CI leg for anything pushed, and `windows_lane_check.py` for a lane that has not been pushed. Cross-compiling from the Mac is not used (V110 S4.1: "Do not compile Windows on the Mac").
- **Tests first.** `windows_lane_check.py`'s argument handling and its refusal to reuse a label are unit-tested without a host; the host run is the proof.
- **dabeest.** One timed `cargo check` of the three shapes, warm cache, to record the cost the lane owner pays per lane.
- **Review.** Phase.

### Step 0.5: the HTTPS endpoint's tests on Windows (no helper process needed)

- **Goal.** The HTTPS candidate's endpoint-level unit and integration tests that need no fake helper process run on Windows, so a Windows HTTPS change is judged by its own tests and not only by the end-to-end qualification rows. The modules that start a fake `git` or `gh` helper or a helper process tree (the "helper boundary", Appendix B, Table B.3) wait for 4.2, 4.3 and 4.4 and belong to step 4.11. The host-level modules that build a runtime with an SSH configuration follow 1.6 and belong to 1.8.
- **Why.** At the tuple the HTTPS test modules are gated `cfg(all(test, unix))`, so none of them compiles on Windows: `git/endpoint/https_pool.rs:362` (`idle_budget_tests`, `idle_tests`); `transport_host/https_endpoint.rs:93, 517, 562` (a test accessor, the test clock, and `cancellation_tests`, `https_cancel_mux_tests`, `retry_tests`, `stale_action_tests`, `wake_tests`); `git/endpoint/https_worker.rs:166, 417-419`; `https_worker/native.rs:406, 412`; `transport_host/mod.rs:15-40`; `transport_host/session.rs:37, 259`; `git/endpoint/mod.rs:47-62` (`cut_proxy`, `https_fixture`, `https_local`, `https_opening`); and `git/gitbackend/transport_observations.rs:197` and `transport_binding.rs:306`. So the shared TLS connector on schannel, the pool's wakes and the idle-loss HTTPS paths are proved on Windows only through the CLI and wheel rows (the 2026-10-08 checks). **The HTTPS fixed-cost port** (lane `gwz-dev-httpsfix`, evidence run `2026-10-08-windows-httpsfix-port`) is not in the tree at `0e21bdde` (`CurrentProgramCheckpoint.md:13`). It adds more `cfg(all(test, unix))` modules and test-only accessors that only those modules use (101 against 96 test-build warnings in that run). **This step depends on the port landing for those additions.** The part of the step that covers the tuple's modules can start now; the part that covers the port's modules starts when the port is in the tree.
- **Files.** The gates Appendix B assigns to 0.5: `https_pool.rs:362`; `https_worker.rs:166` and `setup_slot_tests` at `:419`; `https_worker/native.rs:406, 412`, `native/tests.rs:6, 63`, `native/tests/protocol.rs:4`, `native/tests/retention.rs:4`; `https_endpoint.rs:93, 517, 562`; `transport_host/mod.rs:15-40` for `fetch_preflight_tests`, `cancellable_https_tests` and `ca_bundle_tests`; `session.rs:37, 259`; `git/endpoint/mod.rs:47-62` for `cut_proxy`, `https_fixture`, `https_local`, `https_opening`; `transport_observations.rs:197`; `transport_binding.rs:306`. Whatever Unix-only facility a fixture uses (Unix sockets, `/tmp`, signals, symlinks) gets a portable form or a Windows arm inside a `cfg_if!`; `native/tests.rs:63` is the real SSPI request validator, which spawns `cargo` and needs an external `CARGO_TARGET_DIR` (two of WH1's disclosed failures): the leg sets it.
- **Design.** Inventory first: each module's gate and the reason it is Unix-only are rows of step 0.3's inventory, owned by this step. Ungate module by module, keeping a module Unix-only only with a recorded reason that is a real platform difference (for example `ca_bundle_tests.rs:118`, an OpenSSL-only trust branch), never a fixture convenience. A module found to build the SSH engine or to use `helper_script` moves to 1.8 or 4.11, and the move is recorded in the inventory.
- **Tests first.** Each ungated module runs in the `candidate-windows` leg; the leg's test count rises and is recorded per module.
- **dabeest.** One run of the ungated set per batch of modules, before the leg proves it.
- **Review.** Phase.

**Phase 0 review.** One Consistency plus Safety phase review of 0.1 to 0.5. They change no product code (a CI leg, a checker, a script and test gates), so OQ16 asks whether a skim review may replace it; until the operator says so, it is the standard review.

## 6. Phases 1 to 6

### Phase 1: portable SSH setup (S4.2, S4.4 in part, S4.5's SSH half)

**Milestone.** SSH setup, trust, URL-password and explicit-key authentication, option A's background close and the idle watch run on Windows through the same worker, pool and supervisor as Unix. Agents are not yet reachable (a missing agent is a refusal, as S4.3 says). Nothing in this phase fixes a Windows policy that TR1.8 has not frozen: it ports Unix behaviour, which accepted text allows before TR1.8's GO (amendment 2 §3.13 and §3.6 gate only S4.3's agent forms and TR4.8 to TR4.10 on it). OQ1 cites that text and asks the operator to confirm the interim choices. The phase also lands the policy-free agent fixture (1.7) and the integrated host, placement and gitbackend test modules that need no helper script (1.8).

#### Step 1.1: a Windows SSH test server

- **Goal.** `SshdFixture` on Windows: a loopback server on a high port with temporary host and client keys, a `known_hosts` that trusts only its key, and a bare repository, as the Unix fixture does (`ssh_fixture.rs:1-7`).
- **Files.** `src/git/endpoint/ssh_fixture.rs` (a Windows arm; the Unix arm uses `/usr/sbin/sshd`, `ssh-keygen`, `kill -STOP` and `ps`, `:73-74, 106-109, 256, 291, 306, 330`), `ssh_password_fixture.rs` (its `sshd`-based password server, the same arm), a process-control helper for stop and kill (Job Object, as step 4.2's primitive; here a small test-only one), and `git/endpoint/mod.rs:47-62`, where only `ssh_fixture` and `ssh_password_fixture` leave the `all(test, unix)` block in this step. The block's other modules have their owners in Appendix B, Table B.2: U2 named them here, but this step's goal is the server.
- **Tests first.** The fixture's own readiness test (the server answers a libssh2 handshake and serves `git-upload-pack` for the bare repo) fails on Windows before the arm exists.
- **dabeest.** A spike first (OQ6 picks the server). Existing baseline runs used a Paramiko server with `diffie-hellman-group14-sha1` and `strict_kex` off, and say it "is not universal KEX coverage" (`2026-10-03-tr1-8-windows/README.md`). Whether Windows' OpenSSH server is installed on dabeest is unknown (only the agent pipe was found absent, error 2), and the host rules forbid enabling a service.
- **Review.** Phase.

#### Step 1.2: portable socket readiness and connect (S4.2)

- **Goal.** Replace the `libc::poll` calls and the `EINPROGRESS` tests in `ssh_network.rs` with one portable readiness helper that has a Unix and a Windows arm, keeping `Control::wait_step`'s contract (a closure bounded by the slice, true when ready).
- **Files.** new `src/git/endpoint/socket_wait.rs` (about 150 lines: `wait_readable`, `wait_writable`, `connect_wait`); `ssh_network.rs` (U3 to U6; drop the module-level `cfg(unix)`; it lands before 1.3 changes `read_regular`, section 4's hot-spot order); `Cargo.toml` and `prepare.py:87` (the `Win32_Networking_WinSock` feature, via the constant described in section 4).
- **Primitive.** Prefer `WSAPoll`, whose shape matches `poll`. WSAPoll reportedly does not report a failed non-blocking `connect` on older Windows 10 builds; `select` with an except set does. Decide by test: the failed-connect row below must pass on dabeest and on `windows-2022` with the primitive chosen. Unverified here; step 1.2 is where it is found out.
- **Tests first.** Portable rows over a loopback pair, on all three platforms: readable after a write, writable, EOF, timeout within the slice, **connect to a closed port reports failure inside the `Control` bound**, connect to a listener succeeds, a cancelled `Control` ends a wait inside one slice. The Unix-only `ssh_tests/network.rs` rows that need no `sshd` move out of `cfg(unix)`.
- **dabeest.** The failed-connect row on the host (Windows 11 build 10.0.26200) and the result on `windows-2022` recorded side by side; both bounds.
- **Review.** Phase.

#### Step 1.3: opening regular files without blocking on special files

- **Goal.** One reader that opens a `known_hosts`, key or identity file, refuses anything that is not a regular file, and cannot block on a pipe or device, with Unix and Windows arms.
- **Files.** new `src/git/endpoint/regular_file.rs` (about 200 lines); callers `ssh_network.rs:133-157` (`read_regular`), `ssh_key_snapshot.rs:152-172`, `ssh_worker/endpoint.rs:135-157`, and `git/gitbackend/transport_support/identity.rs:204-206` (`validate_file` pre-checks `metadata().is_file()` and then opens with `O_NONBLOCK` on Unix only; on Windows the same check-then-open window is left open, so it becomes a caller, which closes it the same way). The Windows arm refuses `\\.\...` and `\\?\...` device paths, reserved names (`CON`, `NUL`, `COMn`) and, after opening, anything but `FILE_TYPE_DISK`. It is the same module step 4.3 uses for `https_auth/file_worker.rs:68`, so write it once.
- **Tests first.** Cross-platform: a regular file reads; a directory refuses; a file over the cap refuses; paths with spaces and non-ASCII characters read. Unix: a FIFO refuses without blocking (the existing rows). Windows: `CON`, `NUL`, `\\.\pipe\x` with no server, and a path to a pipe created by the test refuse without blocking.
- **dabeest.** The named-pipe and device rows, which hosted runners can also run; record both. WH1's design (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §5) wants "reject pipe/device/unsupported path classes before blocking opens" proved, not assumed.
- **Review.** Phase.

#### Step 1.4: ungate the setup chain

- **Goal.** Compile and run `ssh_key_auth`, `ssh_password`, `ssh_setup`, `idle_watch` (which `ssh_setup.rs:4` imports, so it compiles here; 1.5 owns its behaviour), `ssh_local` and `agent_job::start_setup` on Windows with no agent source, by removing their `cfg(unix)` (U1, U12, U13, U15).
- **Files.** `git/endpoint/mod.rs:21`; `ssh_key_auth.rs:19`; `ssh_local.rs:4`; `agent_job.rs:194`; `ssh_password_helpers.rs:30` (Windows arm returns the existing `Unsupported`, with a message naming WH2, until 4.5). `agent_auth` stays Unix until 3.4, and `agent_socket` stays Unix permanently (its Windows siblings are 3.3 and 3.6); `ssh_local` takes `agent: None` on Windows, and an open that needs an agent is refused with the existing "no agent" refusal (S4.3). The test modules of Appendix B owned by 1.4: `ssh_tests/{host_case,key_container,selected_key,selected_pool}.rs` and the `sshd`-needing rows of `network.rs`; `mod ssh_tests` leaves the `all(test, unix)` block at `git/endpoint/mod.rs:47-62` here, and each file inside keeps its own `cfg(unix)` until its owner step. Moving the module out of the block also compiles the 16 files of `ssh_tests/` that have no gate of their own (Table B.2). Three of them need a guard, which this step adds in `ssh_tests/mod.rs`: a paired `cfg_if` around `mod agent_fixture` (`agent_fixture.rs:6` imports `std::os::unix::net` unconditionally; owner 1.7), and one around `mod idle_loss` and `mod idle_loss_budget` (they use `cut_proxy`, which is 0.5's, and run 1.5's behaviour; owner 1.5). I gated them rather than adding 0.5 ── 1.4: 0.5 can wait on the HTTPS port, and Phase 1 is on the critical path, so 1.4 must not wait on it, while 1.5 needs only `cut_proxy`, a module of the tuple.
- **Tests first.** The existing `ssh_tests` rows for explicit key, URL password, trust refusal, `known_hosts` case folding (TR2.18) and setup cancellation, un-gated for Windows and failing on a Windows build before this step because the modules do not exist there.
- **dabeest.** Explicit-key and URL-password fetch against the step 1.1 server; `known_hosts` mismatch refusal before any key offer.
- **Depends on.** 1.1, 1.2, 1.3.
- **Review.** Phase.

#### Step 1.5: option A and the idle watch on Windows

- **Goal.** Run the background-close and idle-loss behaviour on Windows, and fix whatever tokio's Windows driver does differently.
- **Files.** `idle_watch.rs` (expected: no change; possible: how a `try_clone`d socket is registered), `ssh_connection.rs` (`terminate`, `watch_socket`), `ssh_channel.rs` (`poll_dispose`), `ssh_setup.rs:401-417`; the guard 1.4 put around `mod idle_loss` and `mod idle_loss_budget` in `ssh_tests/mod.rs` comes off here (the files have no gate of their own), and `pooled.rs` and the `ssh_worker` tests, which have none either, run from 1.4 and gain Windows rows here if the behaviour needs them. This step needs `cut_proxy` from 0.5 (the tuple's module, which does not wait for the HTTPS port).
- **Tests first.** The idle-loss rows: the server closes an idle connection and the host reports `idle_closed` without a lease; a reused dead connection is replaced by one fresh retry; a reset is lost as EOF is; a registration failure marks the connection lost (`GwzTransportIdleLossDesign.md` §5.1, §8). Option A's rows: a fetch completes at libssh2's close; a push waits for the client's EOF to reach the server; a discarded connection is gone within one pass; shutdown and discard terminate a closing exchange (`GwzTransportSshBackgroundCloseDesign.md` §5, §10). Only the option A rows that are in the tree when this step runs apply; if option A has not landed, this step covers idle loss and adds a line to option A's lane owner (it is in lane `gwz-dev-bgclose` today, `CurrentProgramCheckpoint.md:14`).
- **dabeest.** The idle and close rows, plus a socket-count observation (one connection per member, the connection dies within a pass) with `netstat` on the host; this is the Windows half of TR8.1's "physical connection counts recorded".
- **Review.** Phase.

#### Step 1.6: SSH through the Windows qualification boundary (S4.5, SSH half)

- **Goal.** Admit `Scheme::Ssh` on Windows in the candidate: construct the SSH engine, list `Ssh` in the endpoint's schemes, stop refusing SSH-only runtimes, and give `SshSettings` a Windows `home`. The qualification cfg keeps its current name until step 5.1 retires it (renaming it now would touch 41 sites for a switch with weeks to live; OQ15).
- **Files.** `transport_host/mod.rs:43, 87-88, 138-141, 188-190, 222-224, 280-282`, the capability projection at `:289-331` (`schemes` gains `Ssh`; `file_identity` and `exact_agent_identity` keep their Windows-qualification value of false, since "exact-agent stays false ... until a named fixture GO", `GwzV110Plan.md:322-323`), and `:428`; `transport_host/session.rs:404, 417-440, 459-462, 470-473`; `endpoint_environment.rs:20, 59-64` (Windows arm builds `SshSettings` with `HOME` absolute as an **interim** that step 3.1 replaces; the inventory marks it); `gitbackend/transport_binding.rs:227` (the Windows SSH refusal) and the `transport_support.rs`, `transport_observations.rs` sites that gate SSH; `transport_host/qualification_tests.rs`. **The candidate-switch inventory.** Removing or moving a `gwz_transport_candidate` site changes gwz-core's `scripts/candidate_switch_inventory.txt` (70 lines; it lists the qualification sites, for example `capture_qualification_proxy` and the `qualification_tests` functions), which `scripts/checks/check_candidate_switches.py` holds equal to the source; the checkpoint records the file's digest (amendment 2 §3.13 rule (a); `CurrentProgramCheckpoint.md:1478-1481` is the form). **Edges.** 3.1 replaces this step's interim `HOME`, so 1.6 precedes 3.1; 4.5's transport-side X6 run and 1.8's runtime-building modules need SSH admitted, so they follow 1.6.
- **Tests first.** `transport_host/qualification_tests.rs` gains rows: SSH is offered on Windows in the candidate; an SSH-only runtime builds; the capability projection lists `Ssh`. Existing rows asserting SSH refusal flip.
- **dabeest.** gwz-cli (built as in the 2026-10-08 check) clones, fetches and pushes over SSH with an explicit key against the 1.1 server, `--max-per-host 1`, with the `--verbose` row asserting the transport route.
- **Review.** Phase.

#### Step 1.7: a Windows agent fixture (policy-free)

- **Goal.** The Windows twin of the fake agent the endpoint's agent tests use (`ssh_tests/agent_fixture.rs`, a proxy in front of a private `ssh-agent`, `:1-3`, and `key_fixture.rs`): an in-process agent served on a named pipe, with scripted behaviour (identities, a sign, a partial reply delivered 1 byte then 2, a stall, a vanishing pipe, a damaged signature), plus a **test-only** `Channel` client over a pipe handle. It encodes no TR1.8 policy: no server-identity rule, no source selection, no UNC rule, no precedence. It implements no agent form of S4.3 and adds no product code. Step 3.3 builds the product channel against it and 3.4 signs through its test channel, so neither waits for the other, and this step waits on neither TR1.8 nor Phase 2.
- **Files.** new `src/git/endpoint/ssh_tests/agent_fixture_pipe.rs` and a Windows arm in `ssh_tests/key_fixture.rs` (about 350 lines together); the Unix reaping in `key_fixture.rs:187-204` (`libc::kill`, `waitpid`) gets a Job Object twin from 1.1's process-control helper; `ssh_tests/mod.rs:29`. The Unix files stay (kind `platform` in Appendix B).
- **Design points.** Signing: either an in-process signer with a test-only dev-dependency, or a pipe proxy in front of a private `ssh-agent` from Git for Windows (the Unix fixture is the second shape). A spike picks, and the pick is recorded in the inventory. A test `Channel` over a socket already exists in `ssh_tests/agent_auth.rs:159`; this is its pipe twin.
- **Tests first.** The fixture's own rows: it lists its keys; a sign verifies against the key's public part; each scripted fault occurs as scripted. They fail on Windows before the arm exists, by not compiling.
- **dabeest.** The fixture rows and the spike; hosted `windows-2022` can run them too.
- **Depends on.** 1.1 (the SSH server and the process-control helper) and 1.4. Its files live in `ssh_tests/`, which compiles on Windows only after 1.4 takes the module out of the `all(test, unix)` block and puts the paired `cfg_if` around `mod agent_fixture`; 1.7 then supplies that arm's Windows twin.
- **Review.** Phase.

#### Step 1.8: ungate the integrated host, placement and gitbackend test modules (S4.5, no skipped Unix-gated tests)

- **Goal.** Every test module that Appendix B assigns to 1.8 runs on Windows, so the integrated host, placement and gitbackend suites are not skipped when S4.5 admits the build (`GwzV110Plan.md` S4.5: "A Windows run that skips the Unix-gated tests is not this step"; OD13: "with the same tests").
- **Modules.** `git/endpoint/mod.rs:47-62`: `budget_wait_tests`, `git_turns_tests`, `ssh_pump_clock_tests`, `ssh_destination_tests` (U2 nominally gave these to 1.1, whose goal is only the server) and `job_budget_wait_tests` (it imports `ssh_setup::SetupConnector`, `job_budget_wait_tests.rs:21`, which is 1.4's, and `https_fixture`, which is 0.5's). `transport_host/mod.rs:15-40`: `tests`, `fault_tests`, `throughput_tests`, `message_embedding_tests`, `https_route_scale_tests`, `https_compat_tests`, `endpoint_environment_tests`, `retry_tests`, which build a runtime with an SSH configuration and so follow 1.6. Also `transport_host/request.rs:446` (`https_budget_gate_tests`), `cleanup_tests.rs:181`, `placement_endpoint.rs:284` (`check_tests`, with its FIFO row at `placement_endpoint_tests.rs:105-129`), `ssh_tests/retry.rs`, and `git/gitbackend.rs:45` (`transport_candidate_tests`, whose one script-writing test, `drivers.rs:254`, keeps a nested gate owned by 4.11).
- **Files.** The gates above. Literal Unix paths in these modules become portable in the same change (the class of defect `1cdb9557` fixed in `retry_tests`): `/tmp` at `budget_wait_tests.rs:187`, `job_budget_wait_tests.rs:350`, `ssh_destination_tests.rs:213` and `placement_endpoint_tests.rs:28, 65, 289`. A FIFO row gets a Windows twin from 1.3's device-file rows, or stays `platform` with a reason.
- **Tests first.** The modules themselves, which do not compile on Windows before. The leg's test count rises and is recorded per module, and the checker (0.3) shows each module's entry leave `unported`.
- **dabeest.** One run of the ungated set against 1.1's server, before the leg proves it.
- **Depends on.** 1.1, 1.3 (device-file twin), 1.4, 1.6, and 0.5 for `https_fixture` (a module of the tuple, which does not wait for the HTTPS port).
- **Review.** Phase.

**Phase 1 review.** One Consistency plus Safety phase review of 1.1 to 1.8, not a skim: Phase 1 changes product code (the socket readiness wait, the file reader that refuses devices, and admitting SSH through the qualification boundary).

### Phase 2: TR1.8 design freeze (design and evidence; no product code)

**Milestone.** TR1.8 has GO. This is the critical path. The steps that consume TR1.8's text are 3.1, 3.2b, 3.3, 3.5 and 3.6, and Phase 4's policy steps 4.4, 4.6, 4.7 and 4.8; they wait for 2.4. The steps that consume none of it do not wait: 1.7, 3.2a, 3.4 and 3.7 (policy-free), 3.8 (it waits on 2.2's rows, not on the GO), and 4.1 to 4.3, where 4.2 and 4.3 implement 4.1's accepted WH2 contract, not TR1.8 §10.

#### Step 2.1: refresh TR1.8 against MAIN

- **Goal.** One revision of `GwzTransportWindowsParityDesign.md` that carries the accepted helper-timing, configuration-view and SSH-clock amendments, the SSPI-only supersession list folded into the text, the `AgentSource` enum, and every disposition in `GwzTransportWindowsProofDispositions-DRAFT.md`, with the changes since 2026-10-03 reconciled: option A (§3.1), idle loss, the adaptive Phase 1, the per-host `Supervisor`, WH1's accepted shape (the qualification cfg, `Owner::send_if`). **It also gives every row that 2.2, 2.3 and 2.3b cannot execute a recorded disposition,** so that none is left provisional (TR1.8 §11): P02's numeric HWND reuse stays UNEXECUTED, as ProofDispositions §3 says, and the claim is revised to what the executed rows prove (the identity check before a send is not atomic with the send; no claim that reuse is prevented); P06's MD5 peers keep native TLS refusal and the RFC 5929 §4.1 hash rule is a pure DER and OID test (ProofDispositions §4), while P06's integrated adapter lifetime (the Hyper and pool integration) is a product test owned by 4.9, so no design clause rests on it; P04's immutable candidate snapshot is a clause that 2.3's B09/P04 rows prove, with 4.6's product test; B04 follows OQ7. Where step 4.1 has accepted the WH2 contract, §10's helper paragraphs point at it.
- **Files.** `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`, `...Baseline.md`, `...UserGuide-DRAFT.md`, `...Checkpoint.md`.
- **Tests first.** n/a (document). Its check: every clause cites an executed baseline row or an accepted disposition, and no clause is marked provisional. A clause whose row stays unexecuted at GO is removed from the design and listed, with the row, in 2.4's review scope.
- **dabeest.** None.
- **Review.** The dual review of step 2.4.

#### Step 2.2: the remaining SSH baseline rows on 1.0.17

- **Goal.** Execute, with released 1.0.17 (`bb9e3720...`), the rows SSH parity depends on, and file each as evidence.
- **Rows.** B06 (Pageant and an agent on a pipe both running, different disposable keys: Pageant must win, the pipe records no request); B08 with an **owned pipe fixture** (a named-pipe agent served by a script, selected through `SSH_AUTH_SOCK`; the baseline §4 says own selected-pipe fixtures can characterize selection); B04's dispositions as accepted by OQ7; and new rows:
  - **X1** a `known_hosts` with CRLF line endings (common from Windows editors; `knownhost.c` has no `'\r'` handling, so the result is unpredictable without a run);
  - **X2** an ed25519-only `known_hosts` entry, and a host offering rsa and ecdsa too (WinCNG, section 2.3);
  - **X3** selected file keys by type: RSA, ECDSA, ed25519 (expected refused by WinCNG), with a path with spaces, a Unicode path, a `~/` path and a relative path;
  - **X4** agent keys by type through Pageant and through the owned pipe: RSA SHA-256 and SHA-1 fallback, ECDSA, ed25519, a certificate, a security key (TR2.8's list);
  - **X5** a URL password against a server that lists `password`;
  - **X6** a password-only server with a configured credential helper (the TR2.18/TR2.23 gap);
  - **X7** the error 1.0.17 reports for a `MaxStartups` drop on Windows (F3);
  - **X8** `SSH_AUTH_SOCK` set to a MinGW Unix-socket path (the design refuses it; libssh2 fails to open it as a pipe);
  - **X9** an agent that exists but holds no key, with Pageant visible and a working pipe agent behind it (libssh2 does not fall through, `agent.c:343, 816-826`);
  - **X10** the cause of B04's Unicode refusal (OQ7), which the baseline does not establish: the same Unicode `HOME` reached through an ASCII-only name for the same directory (a junction or an 8.3 name), a Unicode path whose characters all lie in the active ANSI code page, and one that does not, recording which call refuses (candidates: the narrow path expansion in libgit2's `sysdir`, `sysdir.c:328-330`, or the open of `known_hosts`).
- **Files.** `gwz-core-evidence/campaigns/transport-qualification/runs/<new label>/`; runner scripts reuse the `ssh_baseline_v5.py` shape.
- **Tests first.** The runner's own expectations are written down before each run and recorded as predictions, as the baseline's HOME rows were ("the first runner's prediction that empty HOME would authenticate failed and is retained").
- **dabeest.** All of it. About 1 day of host time. It starts now for the rows the baseline's Paramiko server can serve. The rows that need a server whose host keys, key types, password-only mode or `MaxStartups` drops the test controls (X2, X3, X6 and X7 at first reading; the step's first task decides) wait for OQ6 and step 1.1's server spike. OQ9(3) applies only if the native pipe row is wanted.
- **Review.** None (evidence); step 2.4 reviews it.

#### Step 2.3: the remaining HTTPS and authentication rows on 1.0.17

- **Goal.** Execute B09 with P04 (the machine-proxy grammar, the implicit loopback bypass, and the capture and immutable-snapshot fixtures), B11 to B14, B15's released redirect and authentication assertions, B16 and B18, and P05 (provider blocked inside a call), P07 (Digest with a provider-accepted identity), P01 (cross-SID) and P03 (native service identity) to the extent OQ9 allows. P02 is dispositioned in 2.1, not executed. P06's executable part is B16's three EPA controls (positive, missing and wrong binding); its MD5 and integrated-lifetime parts are dispositioned in 2.1.
- **Files.** evidence run as above.
- **Dependency.** These need the approvals the baseline lists: a one-certificate Windows trust transaction (the guarded proposal exists, `NATIVE_TRUST_TRANSACTION_v2/v3.md` in the 2026-10-03 run), a distinct account for helper-identity and Digest rows and for P01's cross-user negative, and a coordinated service for the native OpenSSH agent. Any of the three may be denied. A row that cannot run is recorded **unexecuted**, and the design removes the claim it supports (step 2.1 lists it); S5.6 forbids advertising a cell without evidence. No clause is left provisional, because TR1.8 §11 allows none at GO.
- **How the primitive rows close.** P04: the B09/P04 rows execute the grammar and bypass counterexamples and the snapshot fixture, under the root-serialized save, guard and restore protocol; what remains is 4.6's product test. P06: B16's controls execute here; 2.1 dispositions the MD5 and lifetime parts. P01: the cross-user negative needs the distinct account (OQ9(2)); if that is declined, P01 stays PARTIAL and TR1.8 §4's refusal of a Pageant owned by another SID is removed from the design. P03: the native service identity needs OQ9(3); if declined, TR1.8 §4's pipe-identity rule is removed or replaced by a narrow verified-service alternative (the design itself says it cannot be frozen unchanged). P02: step 2.1.
- **Tests first.** As step 2.2.
- **dabeest.** About 1 to 2 days, in the proxy-serialized form the baseline §5 requires (exact prior proxy state saved, restoration guard armed before each mutation).
- **Review.** None (evidence).

#### Step 2.3b: B17, the macOS and Linux rows on 1.0.17

- **Goal.** Execute with released 1.0.17 one row on macOS and one on Linux against the loopback `Negotiate` fixture, and record whether 1.0.17 authenticates on each (amendment 2 §3.5, "Evidence first" and "The same challenge on macOS and Linux"). If it does, the migration notes list the case as unsupported on the transport, refused with a message naming the off switch (step 6.4). Amendment 2's Phase 4 evidence list names these rows beside TR1.8's `HOME`-unset rows (`GwzTransportReleasePlanAmendment-2.md:654`).
- **Rows.** B17 as the baseline states it (`GwzTransportWindowsBaseline.md:305`): the Mac plain-HTTP `Negotiate` characterization exists; the Mac HTTPS row is blocked by native trust (OQ9(4)); the Linux row needs a host assignment (baseline `:352`).
- **Files.** evidence run, under the same rules as 2.2.
- **Tests first.** As step 2.2 (predictions recorded before each run).
- **Dependency.** OQ9(4); the loopback `Negotiate` fixture's source from 2.3, built for each host.
- **dabeest.** None: these rows run on a Mac and a Linux x86-64 host.
- **Review.** None (evidence); step 2.4 reviews it.

#### Step 2.4: settle, review, GO

- **Goal.** Root-settled tuple, the canonical dual Consistency and Safety review of the revision from 2.1 with the evidence from 2.2, 2.3 and 2.3b, and Surface on the user guide alone (`GwzTransportWindowsBaseline.md` §7). At most two architectural remediation rounds.
- **Files.** the reports beside the design: `GwzTransportWindowsParity-ReviewConsistency.md`, `-ReviewSafety.md`, `-ReviewSurface.md`; the verdict.
- **Review scope that this plan adds.** The reviewers receive (a) the list of claims 2.1 removed because their rows stayed unexecuted, and (b) the reading of §11 the plan relies on: a row that cannot run discharges §11 by having its claim removed and listed, not by leaving a provisional clause. If the reviewers read §11 as requiring an executed result for every row, the fallback is an amendment of §11 by the lane owner or the operator, never a provisional clause. They also check that §10's helper paragraphs and step 4.1's accepted contract agree.
- **dabeest.** None.
- **Review.** **Dual** (TR1.8's own: it designs Pageant's window and shared memory, the machine proxy's bypass rule, and the default-credential exchange; amendment 2 §3.5), plus Surface.

### Phase 3: Windows SSH parity (agents, home, algorithms, errors)

**Milestone.** On Windows, SSH authenticates through a visible Pageant first, else the OpenSSH agent pipe `SSH_AUTH_SOCK` names or the default pipe, resolves its home and `known_hosts` as libgit2 does, signs with the key types libssh2 on Windows can handle, and reports a dropped connection the way the retry machine expects. TR4.8 and S4.3 are inside it. Steps 3.2a, 3.4 and 3.7 are policy-free and may land before 2.4; 3.1, 3.2b, 3.3, 3.5 and 3.6 consume TR1.8 and wait for it.

#### Step 3.1: the SSH home resolver and the global-config lookup (S4.4; TR1.8 §3, §10)

- **Goal.** Resolve the SSH home on Windows as libgit2 does (`HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE`; first existing directory) from the captured environment, with TR1.8's accepted dispositions for empty, relative, Unicode and spaces (OQ7), and make the transport-setting global-config lookup use the same home contract. Replace step 1.6's interim.
- **Files.** new `transport_host/ssh_home.rs` (about 200 lines, a pure function of the snapshot and a filesystem probe port); `endpoint_environment.rs:177-200` (the Windows `platform::ssh_home`); `transport_host/mod.rs:85-100` (`SshEndpointConfig::from_environment`, which must use the same resolver or be retired, TR1.8 §3); `known_hosts` and `~/` identity resolution; `transport_setting/global.rs:63-81` (the global-config lookup reads `HOME` only from the snapshot, and an empty or relative one names no file; on Windows that finds nothing unless it takes the resolved home, ProofDispositions §1: the resolution "must serve SSH trust, `~/` identities and the accepted transport-setting global-config lookup consistently") with the test gate at `transport_setting.rs:283` and the Unix-only test helpers under `transport_setting/tests/` (Appendix B).
- **Tests first.** A table of B03/B04 rows reproduced as pure tests (candidates in order, skip empty, existence probe, no fallthrough to another home's trust file when the first has no good `known_hosts`, relative resolved against the captured cwd, UTF-16 preserved), then the same rows through the fixture. For the lookup: with `HOME` unset and `USERPROFILE` set, `~/.gitconfig` and `XDG_CONFIG_HOME/git/config` are found under the resolved home, and an unresolvable home names no file. For the off switch (amendment 2 TR1.8, "the off switch's three forms on Windows"; TR1.8 §10): the flag, a captured `GWZ_TRANSPORT` and the user-global `gwz.transport` each select the native route on a Windows snapshot, with precedence flag, then environment, then global, then default `gwz`, and a repository-local value ignored. The end-to-end rows are 6.2's.
- **dabeest.** B03 and B04 against the transport, side by side with 1.0.17; one row with `HOME` unset (TR8.4 asks for it); the global-config lookup under that same session.
- **Depends on.** 2.4, 1.6 (it replaces 1.6's interim).
- **Review.** Phase.

#### Step 3.2a: the `AgentSource` seam (policy-free)

- **Goal.** Replace `agent: Option<PathBuf>` by an owned enum chosen once per runtime, before any connection (amendment 2 §3.5), with two variants only: `None` and `Path` (today's Unix socket path). On Unix nothing changes; on Windows the value is always `None` until 3.2b. It adds no Windows rule: no pipe, no Pageant, no precedence, no default, no refusal text for a path form. This is the part of the old 3.2 that consumes nothing from TR1.8.
- **Files.** `transport_host/mod.rs:138-150` (`SshSettings`, `SshEndpointConfig`); `endpoint_environment.rs:177-200`; `ssh_local.rs` (`connect_with_helpers`'s `agent_socket` parameter); `ssh_setup.rs` (where the agent path is used); `agent_socket.rs`.
- **Tests first.** The Unix selection rows pass unchanged through the enum; `None` and `Path` round-trip through the runtime's configuration; a Windows build with `None` refuses an open that needs an agent with the existing "no agent" refusal (S4.3).
- **dabeest.** None (pure logic).
- **Depends on.** 1.4 (the setup chain is ungated). It does not wait for 2.4.
- **Review.** Phase.

#### Step 3.2b: Windows agent-source selection (TR1.8 §4)

- **Goal.** Add the `Pageant` and `NamedPipe` variants and the rule: a visible Pageant first, else the pipe `SSH_AUTH_SOCK` names, else `\\.\pipe\openssh-ssh-agent`; chosen once per runtime, before any connection; a failure never falls back to another source.
- **Files.** As 3.2a (without `agent_socket.rs`), and `transport_host/mod.rs:138-150`'s Windows arm.
- **Tests first.** Pure selection tests with an injected "Pageant window visible" probe and a snapshot: Pageant beats the snapshot's pipe; the pipe beats the default; the default is `\\.\pipe\openssh-ssh-agent`; a MinGW socket path and a UNC pipe path are refused before connect, naming `SSH_AUTH_SOCK` (TR1.8 §4); the Unix selection rows pass unchanged.
- **dabeest.** None (pure logic). The probe's real implementation is step 3.6.
- **Depends on.** 2.4 (it consumes TR1.8 §4) and 3.2a.
- **Review.** Phase.

#### Step 3.3: the OpenSSH agent pipe (S4.3)

- **Goal.** `agent_pipe.rs`: a `Channel` over a local named pipe, overlapped, one request owner per handle, bounded by `Control`.
- **Depends on.** 2.4 (TR1.8 §4), 3.2a, and 1.7 (the pipe fixture, which it does not build).
- **Files.** new `src/git/endpoint/agent_pipe.rs` (about 350 lines); `agent_client.rs` (no change expected); `agent_socket.rs` stays Unix; the Windows twins of the real-socket rows in `ssh_tests/agent_client.rs:277-331` and `agent_wait.rs`.
- **Design points** (TR1.8 §4; `agent_win.c:124-139` is the 1.0.17 behaviour to match): connect with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`; `ERROR_PIPE_BUSY` waits inside `Control`, not libssh2's fixed 1 s; local pipe names only (reject UNC, traversal); `CancelIoEx` on the owner's `OVERLAPPED`, and the buffer, `OVERLAPPED` and event stay owned until completion is observed (P03 executed); the server-identity rule is OQ4's.
- **Tests first.** Agent-codec tests over step 1.7's named-pipe agent fixture: identities, a sign, a partial reply delivered 1 byte then 2 (P03), a cancel mid-read that completes before the storage is released, a wrong-name refusal, a server that never answers (stall bound), a pipe that vanishes mid-request poisons the channel.
- **dabeest.** B08 and B06 with the owned pipe fixture against the transport and against 1.0.17; the native service row only under OQ9.
- **Review.** Phase.

#### Step 3.4: `agent_auth` on Windows, and the allocator question

- **Goal.** Ungate `agent_auth` for Windows, and prove the `libc::malloc` that hands libssh2 its signature buffer is the allocator libssh2 frees with (U11). This step implements no agent form of S4.3: it adds no product `Channel` and no source selection, and signs through step 1.7's test channel.
- **Files.** `agent_auth.rs:4, 213-220, 275-277`; `Cargo.toml:103-108` (`libc` is a `[target.'cfg(unix)'.dependencies]` entry at `:103-104`, so the ungated module does not compile on Windows until a Windows `libc` dependency is added, or the CRT accessor below replaces it); `tests/transport_backend/prepare.py:74-87` (the candidate manifest copies `Cargo.toml`'s tables and its extras at `:74-86` add no `libc`; the `windows-sys` feature list at `:87` is the 1.2 constant); `ssh_tests/agent_auth.rs`.
- **Tests first.** A sign through step 1.7's fixture on Windows completes, and a debug-CRT run (MSVC debug heap checks) of one thousand signs shows no heap mismatch or leak. If `libc::malloc` and libssh2 differ (for instance a static-CRT build), the fallback is to allocate through the session's allocator via a small accessor, not to copy a Rust `Vec` into native ownership (the existing comment forbids that). **The inventory:** a `libc::malloc` that stays in the Windows arm is recorded in step 0.3's inventory with a `proof` field naming the debug-CRT run's evidence label, which is the one case the checker allows; if the accessor is used instead, no `libc::` remains and no `proof` is needed.
- **dabeest.** The debug-CRT run, and a `+crt-static` release build, since cargo-dist may build Windows that way (check `gwz-cli`'s dist config as the first task).
- **Depends on.** 1.2 (the Windows network arm) and 1.7, and through 1.7 on 1.4 (`ssh_tests/agent_auth.rs` compiles only after 1.4). It does not depend on 3.3 or 2.4.
- **Review.** Phase.

#### Step 3.5: Pageant's exchange primitive (TR4.8, part 1)

- **Goal.** `pageant_exchange.rs`: given a window handle, send one agent message and read one reply through a mapping, with TR1.8 §5's rules: `WM_COPYDATA` with `dwData 0x804e50ba`, an 8192-byte `Local\` mapping with a unique name from the runtime's `IdSource`, exclusive creation, an ACL that grants the caller and SYSTEM only, payload at most 8188 bytes, reply length checked 1..8188, one `SendMessageTimeoutW` with `SMTO_BLOCK | SMTO_ERRORONEXIT`, no resend, no read after a timeout, no name reuse.
- **Files.** new `src/git/endpoint/pageant_exchange.rs` (about 350 lines); Cargo features `Win32_UI_WindowsAndMessaging`, `Win32_System_Memory`, `Win32_Security_Authorization` (candidate extras).
- **Tests first.** Against a synthetic receiver (a test window class on a thread): a round trip; a timeout; the late write after timeout is never read; collision on the mapping name refuses without writing (error 183); an oversized request refuses before dispatch; a reply with a bad length or a bad message type ends the channel; same-queue window refused. Against actual Pageant 0.83: the P01/P02 rows the baseline marks executed, re-run through the product code.
- **dabeest.** Pageant 0.83 (pinned, official SHA-256 manifest `putty-0.83-sha256sums.txt`), disposable key; the encrypted-key prompt row (the 2037 ms timeout, mapping alive until reap).
- **Review.** **Dual** (wire format: the frame and mapping that a released external program, Pageant 0.83, reads, which is "anything a released client or server reads" in `GwzProcessOptimization.md` §8). Phase 3's review still reads the composition.

#### Step 3.6: Pageant as the session's agent source (TR4.8, part 2)

- **Goal.** `agent_pageant.rs`: the `Channel` adapter over 3.5 (a written frame is buffered; the first read performs the exchange and serves the reply bytes), the real window probe for 3.2b (`FindWindowW("Pageant","Pageant")`; "visible" means discoverable by this protocol, not `IsWindowVisible`, TR1.8 §4), the window identity pin (HWND, process handle, creation identity, SID, logon `AuthenticationId`; checked before each request; a vanished or replaced window refuses and never selects the pipe), and the refusal texts: "Pageant has no key this transport can use", a timeout naming the bound and the confirmation or key action.
- **Files.** new `src/git/endpoint/agent_pageant.rs` (about 300 lines); `ssh_local.rs` (open the channel for the selected source); `ssh_tests/local_endpoint.rs` and `supervised.rs` (un-gated; they use step 1.7's fixtures).
- **Tests first.** Selection and failure rows over the synthetic receiver: Pageant visible with keys signs, and the pipe fixture records **no** request (B06); Pageant visible with no keys refuses and the pipe is not tried (X9); a replaced owner refuses; the confirmation timeout error names no fallback; neither agent present refuses before any connection opens (B07).
- **dabeest.** B05, B06, B07, X9 and X4's Pageant column with the transport, side by side with 1.0.17.
- **Review.** Phase. OQ5 applies (OQ3 is a scheduling note). Not a per-step dual: 3.5 carries the wire-format dual, and the window identity pin is on Phase 3's Safety finding list.

#### Step 3.7: Windows error kinds and the retry machine

- **Goal.** Pin what Windows reports for a `MaxStartups` drop, a refused connect, a reset and an abort, and map a pre-authentication drop to the `Io` code that `setup_retry::classify` returns as `Verdict::Retry`, as macOS and Linux already get (`ssh_tests/max_startups.rs:1-12`). Where a platform reports the drop as `ConnectionAborted`, today's mapping to `Cancelled` (`ssh_setup.rs:578`, a `Return` verdict) would fail the member (`GwzTransportAdaptiveConcurrencyDesign.md` F3). **`Suspect` is not built here:** that class does not exist in gwz-core or gwz-transport at the tuple (it belongs to the adaptive design's limit-discovery machine), so this step does not depend on it, and that machine's step later maps the same drop to `Suspect`.
- **Files.** `ssh_setup.rs:575-582` (the `failure` mapping, with `ConnectionAborted` at `:578`), `ssh_tests/max_startups.rs` (un-gated, with a Windows branch like its macOS one at `:107`), `setup_retry/` classification tests.
- **Tests first.** The L3-S row for `MaxStartups` on the Windows leg (`GwzTransportAdaptiveConcurrencyDesign.md` §10.2 item 20): a server that drops the Nth unauthenticated connection yields a retried setup (`Verdict::Retry`), not a failed member; a unit row for each Windows error kind (the Windows-numbered ones go in a table keyed by `raw_os_error`).
- **dabeest.** X7 on 1.0.17 first (what it reports), then the transport. If the step 1.1 server has no `MaxStartups`, the row uses a TCP shim that resets the Nth connection.
- **Depends on.** 1.6, 1.1. Policy-free: it consumes nothing from TR1.8.
- **Review.** Phase.

#### Step 3.8: host-key preferences and the key-type matrix under WinCNG

- **Goal.** Make `preferences()` and `HOSTKEYS` (`ssh_network.rs:24-30, 346`) safe on a libssh2 that lacks ed25519, and run TR2.8's matrix on Windows against what X2 to X4 found 1.0.17 doing.
- **Files.** `ssh_network.rs` (the preference list filtered by what the session supports, or a refusal that names the algorithm), `ssh_tests/key_types.rs`, `rsa_sha1.rs`, `key_files.rs` (un-gated here; they use step 1.7's key fixture; `selected_key.rs` and `host_case.rs` were un-gated in 1.4 and gain their Windows key-type rows here).
- **Tests first.** An ed25519-only `known_hosts` entry yields the outcome X2 recorded for 1.0.17 (this step is blocked on that row); mixed entries select the supported algorithm; each key type outcome equals 1.0.17's.
- **dabeest.** The matrix side by side with 1.0.17.
- **Review.** Phase.

**Phase 3 review.** One Consistency plus Safety review of 3.1 to 3.8 (3.5 has already had its dual review; this one reads the composition), with Pageant's window and shared memory and the pipe's server identity on the Safety finding list. Surface covers the Windows messages for Pageant and agents (S7.5 (1.1.0) names "TR1.8's Windows messages for Pageant and the machine proxy").

### Phase 4: HTTPS parity residuals (WH2, TR4.9, TR4.10, WH3)

**Milestone.** On Windows, HTTPS does what 1.0.17 does: configured credential helpers, the WinHTTP machine proxy, helper-identity and default-credential authentication, and the integrated adversity and installed-path rows are qualified. Phase 4 does not wait on Phase 3. It waits on Phase 1 only where a step says so: 4.5 needs 1.1, 1.4 and 1.6 (an SSH server, the setup chain, and SSH admitted), and 4.11 needs 0.5 and 1.8. Steps 4.1 to 4.3 wait on no TR1.8 text; 4.4 and 4.6 to 4.8 wait on Phase 2 for their policy choices only; 4.5 follows 4.4. Phase 4's edits to the shared `transport_host` files merge in the hot-spot order of section 4.

#### Step 4.1: the WH2 contract

- **Goal.** The WH2 spikes and accepted contract (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §3 "WH2", §5), which is the specification that 4.2 and 4.3 implement (TR1.8 §10's helper paragraphs are an input to it, and 2.1 points §10 at it): Job attached at process creation before the child runs, allowlisted inherited pipe handles, descendant survival, cancellation and reaping with capacity retained, Git-for-Windows origin and UTF-8 versus UTF-16 rules, quoting, the null config source, the executable extension rule, regular-file refusal, non-ASCII paths and case-varied environment keys.
- **Files.** a WH2 design and baseline section; evidence run.
- **Tests first.** n/a (design); the spikes are its rows. P08 (helper Job Object primitive) is already executed, so this is composition.
- **dabeest.** The helper spikes, with Git for Windows' `git.exe` and a fake helper that hangs, spawns a descendant, or writes without reading.
- **Review.** Dual Consistency plus Safety (the contract handles credentials).

#### Step 4.2: the helper process owner

- **Goal.** Implement 4.1's accepted WH2 contract, not TR1.8 §10: replace `killpg` and `process_group(0)` with a Job Object owner: start suspended, assign to a kill-on-close Job before resume, no breakaway, kill and wait for the whole job on timeout or cancel, pipe buffers owned until I/O and reaping complete.
- **Files.** `https_auth/owner.rs:245-254`, `https_auth/lookup.rs:165-170`, `https_auth/runner.rs`, `https_auth/runner/tests.rs:151` (the test guard's `libc::kill`); Cargo feature `Win32_System_JobObjects`.
- **Tests first.** The Unix process-group rows, generalized: the helper's descendant dies on cancel; capacity is retained until reaping; a drop mid-I/O does not free a buffer a pending read still uses. Windows variants: direct, spaces in the path, shell, GUI, nested Job (P08's four shapes).
- **dabeest.** The P08 shapes through product code.
- **Review.** **Dual**, taken together with 4.3 at the end of 4.3 (WH2's secret and process boundary has mandatory dual review, `GwzWindowsHttpsIntegrationDesign-DRAFT.md:287`).

#### Step 4.3: helper discovery, environment and paths, framing

- **Goal.** Implement 4.1's accepted WH2 contract, not TR1.8 §10: `git.exe` found on the captured absolute `PATH` (no implicit cwd, no shell-script extension); the environment and arguments passed as UTF-16 without lossy conversion; the helper protocol framing and the config "view" carried as wide paths; the U7 reader for the config files.
- **Files.** `https_auth/executable.rs:30-35`, `runner.rs:3`, `view.rs:3`, `view/framing.rs:4`, `file_worker.rs:5, 68`, `endpoint_environment.rs:75-83` (the Windows `auth` config), `helper_script.rs:23, 78` (Windows fakes).
- **Tests first.** Non-ASCII and spaced paths and case-varied keys survive a round trip into the helper; a path that cannot convert refuses with `ConfigurationRefused` and never substitutes a replacement character; a missing `git` reports the setting that names it (4.1's contract).
- **dabeest.** The WH2 path rows with Git for Windows.
- **Review.** **Dual**, together with 4.2 (see 4.2): the helper environment and arguments cross the secret and process boundary. 4.4 has its own dual.

#### Step 4.4: admit `WindowsConfigured` and `Gh`, and the precedence rule

- **Goal.** Offer the configured-helper policies on Windows, with TR1.8 §7's rules: ask the helper once for the URL the credential goes to, after a redirect; pick the highest offered scheme (`Negotiate`, `NTLM`, `Digest`, `Basic`); a `Negotiate`-only challenge asks no helper; no helper identity falls to the logon session; a helper timeout or cancel ends the request; a rejected helper identity is never replaced by another.
- **Files.** `https_worker.rs:60, 98, 113, 118, 135, 245, 252`, `https_worker/credentials.rs:19`, `https_auth.rs:10, 110, 128`, `transport_host/session/driver/opening.rs`, `https_worker/prepare.rs`, `transport_host/mod.rs:289-331` (the capability projection: `schemes` and `auth_policies`) and `:428` (the Windows backend built `without_credential_helpers`), `transport_host/session.rs:459-489` (the endpoint's schemes and policies), and the WH1 refusals that keep these policies off.
- **Tests first.** The B11 to B13 matrix as fixtures: Negotiate-only with a helper credential present (helper not asked), NTLM-only with and without a helper, `Negotiate, Basic` with a fake `gh` then a non-gh helper (helper identity over `Negotiate`, no default-credential offer). A forged `WindowsConfigured` Open still refuses where the policy is not offered.
- **dabeest.** The same matrix over loopback HTTPS (needs OQ9's trust approval for 1.0.17 side-by-side).
- **Review.** **Dual** (secrets).

#### Step 4.5: SSH password-only servers with a helper

- **Goal.** `ssh_password_helpers::lookup` on Windows (U14), so a server that lists no `publickey` uses the configured helper, as the accepted ambient password-only route does (`ssh_password.rs:15-17`).
- **Files.** `ssh_password_helpers.rs:30, 142`.
- **Tests first.** The Unix `ssh_tests/password_helpers.rs` rows on Windows; X6 expected result.
- **dabeest.** X6 side by side with 1.0.17 (its transport-side run needs SSH admitted, so this step also follows 1.6).
- **Review.** Phase.

#### Step 4.6: machine proxy capture (TR4.9, part 1)

- **Goal.** Read `WinHttpGetDefaultProxyConfiguration` once beside the environment snapshot, into an owned value: `Direct`, or `Named { proxy, bypass }`, or a refusal that names the unsupported form (PAC, automatic, multiple applicable proxies, userinfo, a path, a bad port, unparseable bypass syntax), `GlobalFree`d on every exit (the WH1 `direct()` already does this for the DIRECT case, `endpoint_environment.rs:203-225`). The machine setting wins over `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` (TR1.8 §6), which the notes state as the inverse of Unix.
- **Files.** `endpoint_environment.rs:203-240` (extend, and move into a `machine_proxy.rs` of about 250 lines), `transport_host/mod.rs` (the snapshot field).
- **Tests first.** A grammar table (bare host, `host:port`, scheme-mapped entries, bracketed IPv6, default port 80, `<local>`, wildcards, trailing dots, ports in bypass entries) as pure tests; the B09 observation that numeric loopback origins bypass the proxy even with an empty bypass list is decided (OQ11) and then asserted.
- **dabeest.** The proxy rows with the serialized save, guard and restore protocol; every attempt restores and verifies the prior state.
- **Review.** Phase.

#### Step 4.7: the machine proxy in the tunnel (TR4.9, part 2)

- **Goal.** Use it as the environment proxy is used on Unix: a CONNECT tunnel to the origin through the chosen proxy, the bypass list applied to the canonical destination after a discovery redirect; no origin credential accompanies CONNECT; a 407 is refused before any token is generated, naming the setting and the off switch (B10), unless OQ11 says otherwise.
- **Files.** `https_connection.rs` (the proxy path Unix already uses), `https_destination/`, `endpoint_environment.rs:115`.
- **Tests first.** A loopback CONNECT proxy fixture records the CONNECT for a proxied host; a bypassed host connects directly; a 407 `Negotiate` reply makes zero credential offers; the restore step ends every row.
- **dabeest.** B09 and B10 side by side.
- **Review.** Phase; Safety's list names the bypass rule and the 407 path.

#### Step 4.8: helper-identity SSPI and Digest (TR4.10, residual)

- **Goal.** The part of TR4.10 WH1 left: SSPI with an explicit identity (`SEC_WINNT_AUTH_IDENTITY_W`) over `Negotiate` and `NTLM` for a helper's credential, with the zeroizing owners TR1.8 §8 requires, and the Digest decision (OQ10): either WDigest in HTTP mode with the helper identity only, or Digest refused with the existing unsupported-authentication message.
- **Files.** `gwz-sspi` (request type and worker), `https_worker/native/{sspi,exchange,owners}.rs`, `https_worker/credentials.rs`.
- **Tests first.** The fake-port contract rows in `gwz-sspi/tests/contract` for the explicit identity (secret never in `Debug`, logs or errors; bounded frames); core rows that a helper identity authenticates over `Negotiate` and a rejected one is terminal with no default-credential retry.
- **dabeest.** B12, B13, B14 and the identity-isolation rows (P05 residual), per OQ9.
- **Review.** **Dual**, Code plus State: amendment 2 TR4.10 calls for "its own dual Code and State review", and TR4.7 waits on "TR4.10 with its own review's GO"; a Consistency and Safety pair is not that review. The hazard and ownership items are that review's attack list: the forced-authentication hazard OD16 accepted, SSPI secret ownership (TR1.8 §8's zeroizing owners), and secret-leak rows (`Debug`, logs, errors).

#### Step 4.9: WH3, integrated adversity and the HTTPS rows

- **Goal.** The WH3 matrix (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §6) at product level, on the final route: original deadline D across reuse, 401 and native rounds; cancel during HTTP and native IPC; cleanup retained before replacement admission; pool reuse within an opaque scope and no cross-operation credential reuse; receive-pack POST failure never retried; EPA matched and mismatched (B16); a discovery redirect between names (B15); the Windows POST challenge (B18).
- **Files.** new rows in the test modules under `src/transport_host/` and `src/git/endpoint/https_worker_tests/`, which step 4.11 has already ungated for Windows (4.11 precedes this step); fixtures in `src/` test modules.
- **Tests first.** Each case is a row; the cases that already pass on the WH1 route (the 2026-10-08 check) are kept as regression rows.
- **dabeest.** The full set, in the form the 2026-10-08 check ran.
- **Review.** Phase.

#### Step 4.10: WH3, installed paths and worker provenance

- **Goal.** The installed CLI and the Python wheel in a path with spaces and a non-ASCII path, the worker's provenance refusal and its missing and mismatched cases, and wheel concurrency.
- **Files.** gwz-cli and gwz-py install rows; `gwz-sspi/docs/HostPackaging.md` rows.
- **Tests first.** Packaged-artifact rows (the worker found at its packaged path, no `PATH` discovery for SSPI).
- **dabeest.** Both artifacts installed under `E:/gwz-tests/<label>/with spaces/...` and a Unicode directory.
- **Review.** Phase.

#### Step 4.11: ungate the helper-dependent test modules (S4.5, no skipped Unix-gated tests)

- **Goal.** Every test module that uses `helper_script.rs`, an auth owner or a process group runs on Windows. These are the files on the helper boundary: the 28 files that mention `helper_script` at the tuple (Appendix B, Table B.3), which need 4.3's Windows fake-helper form (U26), 4.2's Job owner, and 4.4's policy admission. Step 0.5 could not take them without taking over those steps' fixtures.
- **Modules (owner 4.11 in Appendix B).** In `git/endpoint`: `https_auth_integration_tests`, `https_budget_tests`, `https_lifecycle_tests`, `https_opening_tests`, `https_worker_tests` (with `configured_helpers`), `https_worker/{credential_tests, helper_budget_tests, retry_tests}`, `https_worker/native/tests/exchange.rs`. In `transport_host` (`mod.rs:15-40`, and the nested gate at `:28`): `driver_tests`, `close_tests`, `command_tests`, `cancellable_tests`, `https_tests`, `https_policy_tests`, `https_helper_projection_tests`, `ssh_helper_projection_tests`, `https_negotiate_projection_tests`. In `git/gitbackend`: `transport_candidate_tests/drivers.rs:254`.
- **Files.** The gates above. `/tmp` literals go the way of 1.8's: `driver_tests.rs:345, 370`.
- **Tests first.** The modules themselves, which do not compile on Windows before. The leg's count rises and is recorded per module; the checker shows each entry leave `unported`.
- **dabeest.** One run of the ungated set, with Git for Windows and the SSH fixture.
- **Depends on.** 4.3, 4.4, 0.5 and 1.8.
- **Review.** Phase.

**Phase 4 review.** One Consistency plus Safety review of 4.1 to 4.11 (4.1, 4.2 with 4.3, 4.4 and 4.8 already had their dual reviews; this one reads their composition). It is TR4.7's input.

### Phase 5: Windows activation, gwz-sspi publication, the Windows review

**Milestone.** The Windows build ships the transport on SSH and HTTPS with no qualification switch, S4.5's "no skipped Unix-gated tests" holds, and TR4.7 has GO. Step 5.5 (gwz-sspi's activation and 0.1.0) belongs to this phase's release-gate work but is scheduled after Phase 6's qualification evidence (6.2 and 6.3), because gwz-sspi's `RELEASE.md` requires both implementation acceptance and Windows qualification before the guard is lifted.

#### Step 5.1: retire the qualification switch in gwz-core (S4.5)

- **Goal.** Turn every `all(windows, gwz_transport_candidate, gwz_windows_https_qualification)` site (41 in `src/`) into the plain `cfg_if` unix or windows arm under `gwz_transport_candidate` that S4.5 specifies (`GwzV110Plan.md` S4.5), and delete the refusals that WH1 added (SSH-only runtime unavailable, HTTPS-only schemes, verified-DIRECT only).
- **Files.** gwz-core `src/lib.rs`; `src/transport_host/{mod,session,endpoint_environment,qualification_tests}.rs` and `session/driver/opening.rs`; `src/git/{mod,gitbackend}.rs` and `gitbackend/{backend,transport_support,transport_observations,transport_binding}.rs`, `transport_support/identity.rs`; `src/git/endpoint/https_worker.rs` and `https_worker/prepare.rs`; `build.rs`; `tests/transport_backend/test_windows_qualification_boundary.py` (17 files); gwz-core's `scripts/candidate_switch_inventory.txt` (70 lines; the qualification sites leave it) and the checkpoint's digest record for it (amendment 2 §3.13 rule (a)).
- **Tests first.** `test_windows_qualification_boundary.py` flips from "the boundary exists" to "the cfg name is gone"; the Windows leg builds only the two remaining shapes; `check_candidate_switches.py` passes against the updated inventory. **Exit measure for S4.5's "no skipped Unix-gated tests":** step 0.3's checker reports no `unported` entry from Appendix B (every owner step 0.5, 1.1 to 1.8, 3.x and 4.x is done), and each `platform` entry carries its reason, which the phase review reads.
- **dabeest.** The 2026-10-08 rows plus SSH.
- **Review.** Phase.

#### Step 5.2: gwz-cli

- **Goal.** gwz-cli's seven files (`build.rs`, `src/globalargs.rs`, `src/lib.rs`, `src/globalargs/{dispatch,transport,transport_help}.rs`, `src/tests/g09.rs`) drop the switch; help and `--verbose` rows describe Windows SSH, Pageant and the machine proxy.
- **Files (inventory).** gwz-cli's `scripts/candidate_switch_inventory.txt` (40 lines) and the checkpoint's digest record.
- **Tests first.** The existing help-text and `--verbose` tests gain Windows rows.
- **dabeest.** The installed CLI runs one SSH and one HTTPS command and shows the transport route.
- **Review.** Phase.

#### Step 5.3: gwz-py

- **Goal.** gwz-py's route and client-host sites (`native/src/{route,client_host,lib}.rs`, `dispatch/{mod,merge}.rs`, `Cargo.toml`) drop the switch, and S6.3's dabeest rows (which "wait on S4.5") run.
- **Files (inventory).** gwz-py's `scripts/candidate_switch_inventory.txt` (26 lines) and the checkpoint's digest record.
- **Tests first.** The S6.3 rows for the Windows wheel: an SSH and an HTTPS operation and two overlapping operations, each asserting the transport route.
- **dabeest.** The wheel, built as the 2026-10-08 check did (`CARGO_INCREMENTAL=0`).
- **Review.** Phase.

#### Step 5.4: the candidate leg on Windows CI (TR4.6, second part)

- **Goal.** After 5.1, the Windows leg builds the candidate and runs every candidate test that needs no fixture, and the SSH fixture rows that 1.1 made runnable on a hosted runner run too if the server is installable there (OQ6).
- **Files.** the workflow; `scripts/run_tests.py` Windows mode if needed; `publish_workflow` (left to Linux by 0.1's leg because its `include_str!` goes through the symlinked `tests/`): it joins the leg, resolving `.github` and `scripts` from a root `prepare.py` provides rather than through `..`, or its exclusion is recorded as a reading of TR4.6 and read in the phase review.
- **Tests first.** The workflow-text test from 0.1 gains the candidate-on-Windows row.
- **dabeest.** None.
- **Review.** Phase.

#### Step 5.5: gwz-sspi activation and 0.1.0 (release step 5a)

- **Goal.** Lift `publish = false` in a reviewed activation change, publish `0.1.0` through `release.yml`, and unblock gwz-cli and gwz-py's pins (O2). This is amendment 2 §3.21 part C's step 5a (a draft section), scheduled here so it is not found at release time. **It runs after the preconditions `RELEASE.md` names, "following implementation acceptance and Windows qualification" (`gwz-sspi/RELEASE.md:126-128`):** implementation acceptance is 5.6 (TR4.7's GO), and Windows qualification is Phase 6's evidence, at least 6.2 (parity rows) and 6.3 (S5.5's repeat and the S5.6 Windows cells). If a Phase 6 row forces a gwz-sspi change, it lands before the activation, so that `0.1.0` is published once; publishing earlier would make a `0.1.1` and change the `=0.1.0` pins in gwz-cli and gwz-py. The activation review's text can be prepared beside Phases 5 and 6.
- **Early, ungated action.** Confirming the trusted publisher's environment (O1) is an operator action on crates.io with no gate; it is asked for now (OQ14) and recorded before this step.
- **Files.** `gwz-sspi/Cargo.toml`, `scripts/release_checks.py`, `RELEASE.md`, `README.md`; gwz-cli and gwz-py CI checkouts of gwz-sspi (the checkpoint lists them as open follow-ups).
- **Tests first.** `release_checks.py`'s unit tests for the lifted guard (the checks refuse while `publish = false`, accept after).
- **dabeest.** gwz-sspi's native opt-in fixtures (`tests/native/`) on the final tuple, plus the packaged worker qualification (its `RELEASE.md` says "qualify the packaged library and Windows worker").
- **Depends on.** 5.6, 6.2 and 6.3.
- **Review.** **Dual** (release gate; OQ14).

#### Step 5.6: TR4.7, the Windows implementation review

- **Goal.** The plan's Phase 4 exit: a dual peer-blind Code and State review on the settled tree after S4.2 to S4.5, TR4.6, TR4.8, TR4.9 and TR4.10 with its own GO (`GwzTransportReleasePlanAmendment-2.md` §3.5). It also reads the state of the REDs that WH1's acceptance disclosed and did not waive (strict core Clippy, the generator owner-IR pin, the candidate-leg failures; section 11).
- **Files.** reports beside the plan.
- **dabeest.** None; the tree is already proved.
- **Review.** **Dual** (TR4.7 by name).

### Phase 6: Windows qualification and release gating

**Milestone.** The Windows cells of the 1.1.0 release table have evidence, and the Windows preconditions of Phase 10 hold.

#### Step 6.1: TR8.4, speed and no partial result

- **Goal.** TR8.1's three targets on dabeest against 1.0.17's Windows build: a no-op fetch of 32 small repositories over SSH at default settings is no slower than 1.0.17 at `--max-per-host 32`; no partial result in three rounds at 16 and 32 members; physical connection counts recorded (`GwzTransportReleasePlan.md` TR8.1; amendment 2 TR8.4).
- **Files.** evidence run; the runner reuses the macOS and Linux harness (`2026-10-06-tr8-1-ssh-gap-fix`) with `netstat` in place of `connlog.dylib`.
- **Dependency.** OQ12 (what the 32 repositories are served from and how dabeest authenticates); the live fetch needs the operator's go (amendment 2 §2). TR2.1, TR2.9, TR2.10 have landed; option A should have, since it changes the SSH number.
- **dabeest.** About 1 day for SSH and HTTPS, interleaved rounds, with the host otherwise idle; section 7.2.
- **Review.** None (evidence); S5.4's retune reads it.

#### Step 6.2: TR8.4, parity rows

- **Goal.** One row per behaviour TR1.8 designs, each running on the transport and succeeding as on 1.0.17: Pageant, the OpenSSH agent pipe, the machine proxy, default credentials (an Internet-zone `NTLM` challenge, OD16), a helper identity; at least one row with `HOME` unset; the native-route row with the off switch on, at 16 and 32 members, three rounds, no partial result; and the off switch's three forms on Windows (the flag, `GWZ_TRANSPORT` and the user-global `gwz.transport`, each selecting the native route with no transport runtime built, in the precedence flag, environment, global; a repository-local value ignored), the last with `HOME` unset (amendment 2 TR1.8, "the off switch's three forms on Windows"; TR1.8 §10; the unit side is 3.1's).
- **Files.** evidence run.
- **dabeest.** The rows; proxy rows use the serialized protocol.
- **Review.** None (evidence).

#### Step 6.3: S5.5 and the Windows cells of S5.6

- **Goal.** Repeat S5.5's rows on dabeest after S5.4 (the defaults, and 6.2's parity evidence, which S5.6's Windows cells cite), and fill the Windows column of S5.6's table with an evidence ID or an explicit unsupported mark. A cell the release's scope puts "in this release" and marks unsupported blocks S5.6 unless an accepted amendment removes it (`GwzV110Plan.md` S5.6); OQ10 names the likely ones (Digest, Kerberos).
- **Files.** the S5.6 table (evidence document).
- **dabeest.** S5.5's repeat.
- **Review.** Phase.

#### Step 6.4: S7.3 and S7.5 on Windows, notes

- **Goal.** All of S7.3 (1.1.0)'s rows "on each platform's consumer build" (`GwzTransportReleasePlanAmendment-2.md` §3.11 text, lines 427-433 of the release plan), on dabeest's Windows CLI and extension builds: (1) one CLI network operation in process over SSH and one over HTTPS, each asserting the transport route through its observations; (2) one gwz-py SSH operation and one HTTPS operation, and two overlapping gwz-py operations, each asserting the transport route; (3) one Rust caller of gwz-core that opens no runtime, asserting the native route (TR2.11); (4) the absence of the 1.2.0 surfaces (`gwz server` unknown, `--server` and `--no-server` unknown, `GWZ_SERVER` without effect, no `SocketCoreBridge` or `server` entry in gwz-py, `transport_capabilities` reporting no session route); (5) the Windows bullet: one operation per behaviour TR1.8 designs asserting the transport route, one Internet-zone `NTLM` challenge asserting that the transport authenticates (OD16), and one SSH operation with `HOME` unset; (6) one HTTPS operation through a non-gh credential helper and one SSH operation through an agent certificate key, each asserting the transport route. Also the Windows messages Surface reviews (Pageant, the machine proxy), and migration notes that state the inverse proxy precedence, the OD16 hazard, WinCNG's missing ed25519, the unsupported cells, and B17's macOS and Linux outcome (2.3b).
- **Files.** gwz-cli docs pages, help text, `docs/` migration notes; S7.3's tests.
- **dabeest.** The rows.
- **Review.** Surface (S7.5).

#### Step 6.5: the release preconditions

- **Goal.** Before step 5's gwz-core tag, `windows-matrix.yml` is dispatched on the release commit and passes, and the checkpoint records the run (amendment 2 §3.12). `gwz-sspi 0.1.0` (step 5.5) is visible on crates.io before step 6 (§3.21). After release, on dabeest, from the installed 1.1.0, all of the post-release check's rows that apply to Windows (amendment 2 §3.12, "on each host"): one SSH and one HTTPS network command from the installed CLI, each taking the transport route by its `--verbose` row; one gwz-py network operation taking the transport route; the absence of the 1.2.0 surfaces, as S7.3 checks it; and the dabeest-only row, one behaviour TR1.8 designs on the transport and one SSH command with `HOME` unset.
- **Files.** the checkpoint; the release scripts' checks.
- **dabeest.** The post-release rows on the installed 1.1.0.
- **Depends on.** 6.4, 5.5, 5.6.
- **Review.** **Dual** (release gate; the Phase 10 review).

## 7. Dependencies, parallel lanes and host load

### 7.1 Sketch

```text
Phase 0   0.1 (candidate part committed; ordinary part in lane gwz-dev-winci) ── 0.2 ── (everything else's Windows leg)
          0.3, 0.4 independent;  0.5: the tuple's modules start now, the HTTPS port's modules wait for it to land
Phase 1   1.1 ┐
          1.2 ├── 1.4 ── 1.5 ── 1.6 ── 1.8          1.2 lands ssh_network.rs's Windows arm before 1.3 rewires it
          1.3 ┘
          1.4 ── 1.7 (policy-free agent fixture; 1.7 also needs 1.1, which 1.4 already follows);  0.5 (cut_proxy) ── 1.5;  0.5 (https_fixture) + 1.1 + 1.3 + 1.4 + 1.6 ── 1.8
Phase 2   2.1 ┐
          2.2 ├── 2.4  (TR1.8 GO)        2.2 starts now; its server-dependent rows wait for OQ6 + 1.1's spike
          2.3 ┤                          2.3 waits on OQ9
          2.3b┘                          2.3b waits on OQ9(4)
Phase 3   1.4 ── 3.2a;  1.2 + 1.7 ── 3.4;  1.6 + 1.1 ── 3.7;  2.2 + 1.6 + 1.7 ── 3.8      (policy-free: 3.2a, 3.4, 3.7)
          2.4 + 1.6 ── 3.1;  2.4 + 3.2a ── 3.2b;  2.4 + 3.2a + 1.7 ── 3.3;  2.4 ── 3.5
          3.2b + 3.3 + 3.4 + 3.5 ── 3.6
Phase 4   4.1 ── 4.2, 4.3 ── 4.4 (also 2.4) ── 4.5 (also 1.4, 1.1, 1.6);  2.4 ── 4.6 ── 4.7;  2.4 + 4.4 ── 4.8;
          4.3 + 4.4 + 0.5 + 1.8 ── 4.11;  4.4 + 4.7 + 4.8 + 4.11 ── 4.9 ── 4.10
Phase 5   3.* + 4.* ── 5.1 ── 5.2, 5.3, 5.4;  5.1..5.4 ── 5.6
Phase 6   5.1 + TR2.1/2.9/2.10 + option A ── 6.1, 6.2;  6.1 + 6.2 ── 6.3 ── 6.4;  5.6 + 6.2 + 6.3 ── 5.5;  5.5 + 5.6 + 6.4 ── 6.5
          (5.5 is numbered with Phase 5 and runs after 6.3; O1's confirmation is asked for now, with no gate)
```

Steps that can start today, in parallel: 0.2, 0.3, 0.4, 0.5 (the tuple's modules), 1.1, 1.2, 1.3, 2.1, 2.2, 4.1. That is ten lanes; 1.2 and 1.3 share `ssh_network.rs` and merge in that order, and dabeest can serve about two of them at a time (section 7.2). 1.7 and 3.2a join when 1.4 lands, and 3.4 when 1.2 and 1.7 have landed (so after 1.4); none of the three waits for TR1.8.

### 7.2 What needs dabeest

| Step | Host time | Notes |
|---|---|---|
| 0.2, 0.4, 0.5 | 15 to 30 min each | CI can substitute once the leg is green |
| 1.1 | spike, about half a day | OQ6 |
| 1.2, 1.3 | short | also run on `windows-2022` |
| 1.4, 1.5, 1.6, 1.8 | 1 to 2 h each | after 1.1 |
| 1.7 | short, plus a half-day spike | the fixture rows can also run on `windows-2022` |
| 2.2, 2.3 | 1 to 2 days each | serialized proxy and trust transactions; the biggest user |
| 2.3b | none | runs on a Mac and a Linux x86-64 host |
| 3.1 to 3.8 | short, per step | Pageant rows need a console session with Pageant |
| 4.1 to 4.11 | 4.6, 4.7, 4.8 need the serialized protocol; 4.11 one run | |
| 5.x, 6.x | 6.1 about a day, host otherwise idle | do not overlap another agent's build |

Hosted `windows-2022` can run: 0.5, 1.2, 1.3 (device paths), 1.7, 3.2a, 3.2b, 3.3 (pipe fixture), 3.4, 3.5 (synthetic receiver only, and only if the hosted session can create windows), 4.2, 4.3, 4.6 grammar, 5.4. It cannot run: Pageant 0.83 (needs the pinned release and a desktop session), the machine proxy (shared state), the logon session's SSPI rows, and anything timed.

## 8. Open questions for the operator

Each gives 1.0.17's behaviour on Windows where known, how to find out where not, the options, and a recommendation.

**Decided (operator, 2026-10-08): every OQ below as recommended.** In particular, OQ1 (a): Phase 1 starts now; OQ6: the Win32-OpenSSH download for step 1.1's spike is approved, with a Rust SSH server crate as the fallback; OQ9: approve (1) the one-certificate Windows trust transaction and (4) macOS native trust for B17, with (2) the distinct local account only if OQ10 keeps Digest in scope and (3) the OpenSSH Authentication Agent service start for OQ4's P03 row; OQ16: Phase 0's review is a skim.

**OQ1. Phase 1 before TR1.8's GO: a citation and one residual to confirm.**
- 1.0.17: not applicable (a scheduling question).
- Citation. Accepted text already permits starting Phase 1. Amendment 2 §3.13 ("What can start now") lists TR1.8 and S4.2–S4.4 together, and §3.6 gates only S4.3's agent forms and TR4.8–TR4.10 on TR1.8. TR1.8 §11's post-GO list puts S4.2 after GO, and adds "Dependencies come from the amendment, not this list's typography." TR1.8's header says no downstream implementation may consume it as GO; Phase 1 consumes no TR1.8 rule, since it ports Unix behaviour. The same holds for the policy-free steps of later phases (1.7, 3.2a, 3.4, 3.7), which no OQ needs to cover.
- Residual. Phase 1 makes three Windows choices that TR1.8's GO will confirm or replace: the wait primitive (1.2), the regular-file reader (1.3), and the interim Windows `HOME` in 1.6 (replaced by 3.1). Step 0.3's inventory marks each as interim.
- Options: (a) confirm the residual and start Phase 1 now; (b) wait for 2.4.
- **Recommend (a).** The schedule is the point of this plan, and Phase 1 is a prerequisite of every Phase 3 test.

**OQ2. Does Windows SSH reuse the Unix worker architecture or need a different readiness model?**
- 1.0.17: libssh2 blocks or polls on its own; the comparison is behavioural.
- Options: (a) reuse the thread-per-job model, 20 ms sliced waits and the tokio idle reactor, changing only the wait primitive; (b) overlapped I/O or IOCP for sockets and a different supervisor.
- **Recommend (a).** Section 3: every OS call is inside a closure, the pool and worker are OS-free, and (b) would fork the architecture that the adaptive and idle-loss designs rely on. Revisit only if TR8.4 shows a latency the slice wait causes. Find out: TR8.4's connect-latency trace on dabeest.

**OQ3 (a scheduling note, not a question).** OD15 requires both agents, and amendment 2 TR4.8 requires Pageant. 1.0.17 tries Pageant, then the pipe (`agent.c:436-441`, `agent_win.c:124-139`). B05 shows Pageant authenticating; B08 (the pipe) has not run against 1.0.17 with a real service. The order is the pipe first (3.3 is smaller and has no shared-memory risk), Pageant second (3.5, 3.6). If Pageant slips because TR1.8 needs a broker (OQ5), the alternatives, holding 1.1.0 or refusing Pageant with a message naming the off switch, each need an explicit amendment (the second is a native-route-flavoured exception that OD15 rejects). That would be the operator's decision at that point; this plan does not open it now.

**OQ4. What must a pipe server's identity be?**
- 1.0.17: nothing. libssh2 connects with `SECURITY_IDENTIFICATION` and checks no server (`agent_win.c:142-153`).
- TR1.8 §4 proposes the pipe server's token has the caller's SID and logon `AuthenticationId`, and says that if the native OpenSSH service runs as SYSTEM "this proposed same-SID rule cannot be frozen unchanged".
- Options: (a) no check, as libssh2; (b) caller SID, or the well-known local SYSTEM SID for a service pipe; (c) caller SID only (breaks the native service).
- **Recommend (b)**, conditional on the P03 row: find out on dabeest whether the OpenSSH Authentication Agent service runs as SYSTEM, which needs the service started (OQ9(3)). Step 3.3 waits for 2.4, so no interim ships. If P03 is unexecuted at GO, TR1.8 §4's same-SID rule "cannot be frozen unchanged", and 2.1 must record a narrow verified-service alternative or seek an amendment (TR1.8 §4's own words); admitting any pipe server that runs as SYSTEM is not such a rule.

**OQ5. Pageant: an in-process request, or an isolated broker?**
- 1.0.17: `SendMessage` with no timeout and a thread-ID-named mapping with no ACL (`agent.c:366-394`).
- Evidence: a send timeout does not stop Pageant's confirmation prompt, and Pageant kept the mapping alive after the sender closed (baseline §2, P02; ProofDispositions §2). A broker process would bound GWZ's lifetime but cannot retire the external receiver.
- Options: (a) in-process, bounded by the remaining `Control` allowance, with the documented limit (never reuse the name, never read after a timeout, never resend); (b) a broker.
- **Recommend (a).** The broker adds nothing the receiver boundary lacks (ProofDispositions §2: "a broker alone is not that primitive"), and it adds a second secret owner to review.

**OQ6. Which SSH server do the Windows tests use?**
- 1.0.17: not applicable.
- Context: the Unix fixture is `/usr/sbin/sshd` (`ssh_fixture.rs:73-74`). The earlier Windows baseline used a Paramiko server limited to `group14-sha1`. Whether Win32-OpenSSH's `sshd.exe` is present on dabeest is unknown (the agent pipe was absent, error 2; baseline §2), and the host rules forbid enabling a service.
- Options: (a) a pinned Win32-OpenSSH release unpacked under `E:/gwz-tests` and run unprivileged on a high port (a download, so the operator's go); (b) Paramiko or asyncssh; (c) a Rust SSH server crate as a dev-dependency of a test-support module; (d) the runner's optional OpenSSH Server feature in CI only.
- **Recommend: spike (a) in step 1.1, fall back to (c).** (a) keeps the Rust fixture's shape and tests the same libssh2 against a real OpenSSH. Find out: whether `sshd.exe -d` runs as a normal user on dabeest with `StrictModes no`, and whether it honours `MaxStartups`.

**OQ7. HOME dispositions: preserve 1.0.17's refusals or fix them?**
- 1.0.17 (B04): empty `HOME`, a Unicode-only path and an existing first home without a good `known_hosts` all refuse before any key offer; missing, nonexistent, relative and spaces-only `HOME` authenticate.
- ProofDispositions §1 proposes: keep the first-existing-home rule; resolve a relative `HOME` against the captured cwd; preserve spaces; **fix** Unicode (the refusal looks like a conversion bug, cause not established); refuse an explicitly empty `HOME` with a specific diagnostic.
- Options: (a) adopt those dispositions; (b) preserve every 1.0.17 refusal exactly.
- **Recommend (a)**, with the Unicode and empty-`HOME` differences listed in the migration notes. Reproducing a likely bug as parity serves no user. Find out first: row X10 (step 2.2) for the cause of the Unicode refusal (candidate: the UTF-8 narrow path in libgit2's `sysdir`, `sysdir.c:328-330`).

**OQ8. Key types and host-key algorithms: match 1.0.17's WinCNG limits or exceed them?**
- 1.0.17: libssh2 on WinCNG, `LIBSSH2_ED25519 0` (`wincng.h:74`); ed25519 host keys and file keys cannot be used. Agent-signed ed25519 may still work, because the agent signs and libssh2 forwards the blob (to be shown by X4).
- Options: (a) match: the transport uses the same `libssh2-sys` and gets the same limits; document them; (b) build libssh2 against OpenSSL on Windows (`openssl-on-win32`), a build and packaging change that makes the transport support more than 1.0.17 does.
- **Recommend (a).** Parity is the rule (OD13) and (b) is its own project (vcpkg, licensing, a divergence between the transport and the native route). Step 3.8 only has to avoid failing differently from 1.0.17 (the `method_pref` strip, section 2.3).

**OQ9. Which dabeest authorizations does the baseline need?**
- Context: the baseline lists three as unanswered or held: (1) one-certificate Windows trust transaction (guarded proposal in the 2026-10-03 run) for 1.0.17's native HTTPS rows; (2) a distinct local account for helper-identity and Digest rows, **and for P01's cross-user negative**; (3) starting the OpenSSH Authentication Agent service for the native pipe row. Also: (4) macOS native trust for B17's Mac row (the private `MACOS_TRUST_APPROVAL_v1.md` proposal; root's request is unanswered, `GwzTransportWindowsCheckpoint.md:39`), with a Linux host assignment for B17's Linux row (baseline `:352`); and downloading the pinned Win32-OpenSSH release (OQ6).
- Options: approve each separately, or decline and mark the rows unexecuted. A declined approval removes the claim its rows support from the design (step 2.1 lists it); it never leaves a provisional clause (TR1.8 §11).
- **Recommend: approve (1) now**, because B11 to B16 and B18 cannot run without it and they decide the HTTPS precedence rules; **approve (4)**, because amendment 2 §3.5 requires one 1.0.17 row on macOS and one on Linux; approve (2) only if OQ10 keeps Digest in scope; (3) only if OQ4's SYSTEM question matters (it does for the native service). This is the largest schedule lever in Phase 2. **Consequence to decide knowingly:** (2) is also the only way to execute P01's cross-user negative, which TR1.8 §4's refusal of a Pageant owned by another SID rests on. Under this recommendation (OQ10 (a), and (2) declined) P01 stays PARTIAL, that refusal is removed from the design, and the Safety review (2.4) judges the ownership rule that remains. If the operator wants the refusal kept, (2) is approved for P01 alone.

**OQ10. Digest and Kerberos: qualify, or mark unsupported?**
- 1.0.17: B14 is unexecuted. The only evidence: WDigest refused four synthetic credential shapes with `SEC_E_UNKNOWN_CREDENTIALS`, and a direct WinHTTP Digest GET and POST stayed at 401 without an offer (baseline §3, P07). So 1.0.17 may not authenticate Digest in the fixture at all. Kerberos needs a domain; dabeest has none, and "Negotiate selecting NTLM does not close it" (WH1 design §6).
- Rule: an "in this release" cell marked unsupported blocks S5.6 unless an accepted amendment removes it (`GwzV110Plan.md` S5.6).
- Options: (a) refuse Digest with the existing unsupported-authentication message and mark Kerberos "not qualified, works through the OS provider when domain-joined", both removed from the "in this release" column by amendment; (b) obtain a provider-accepted Digest fixture and a domain fixture first.
- **Recommend (a)**, decided only after step 2.3 has tried the 1.0.17 Digest row with an accepted identity if OQ9 allows it. Find out: B14 with a disposable local account.

**OQ11. Proxy forms outside DIRECT and a plain named proxy, and 407.**
- 1.0.17 (B09, B10): machine proxy applies to a `.invalid` origin; numeric loopback origins bypass even with an empty bypass list; a 407 with `Negotiate` or `NTLM` fails with zero credential offers.
- Options: (a) refuse PAC, automatic, ambiguous and unparseable forms and every 407 before any credential; reproduce the loopback implicit bypass; (b) implement proxy authentication.
- **Recommend (a)**: it matches the measured 1.0.17 and TR1.8 §6 ("no proxy credential mechanism is designed as supported"). Find out: the implicit loopback bypass's grammar (wildcards, `localhost`, IPv6 `::1`, trailing dots) needs counterexample rows, which step 4.6 lists.

**OQ12. TR8.4: what serves the 32 repositories, and how does dabeest authenticate?**
- 1.0.17: TR8.1's macOS and Linux runs fetched 32 public repositories from GitHub; the live fetch "needs the operator's go" (amendment 2 §2). The Linux SSH runs forwarded the operator's agent (authorized 2026-10-06, `CurrentProgramCheckpoint.md`); the dabeest rules use `ClearAllForwardings=yes`, so no forwarded agent reaches it today.
- Options: (a) a LAN fixture of 32 bare repositories on an SSH server the lab controls (no account), with the speed target measured on that; (b) GitHub, with the operator's agent forwarded or a disposable read-only key loaded into Pageant on dabeest; (c) both.
- **Recommend (c) with (a) first.** The target is relative to 1.0.17 on the same path, so (a) is a valid relative measurement and runs unattended; (b) confirms GitHub latency. TR8.1 states no tolerance; the Windows rows inherit whatever OD17 settles for the other two platforms.

**OQ13. When is the Windows CI leg a required check?**
- 1.0.17: not applicable.
- Options: (a) required for the compile shapes from day one, the test run advisory until step 0.2 confirms on a hosted runner that the red tail is gone; (b) required entirely once green; (c) advisory.
- **Recommend (a).** The compile shapes are cheap and stop the `cfa14b8`-class break (the `#[cfg(not(windows))]` that attached to the next import and broke v1.0.5). Lanes are local clones that never trigger CI before merge: for those, either push a lane branch (the lane owner's call) or run `windows_lane_check.py` (step 0.4).

**OQ14. gwz-sspi 0.1.0's activation: one review, or two?**
- Context: TR4.10 has its own dual review (secrets); gwz-sspi's `RELEASE.md:126-128` refuses preparation and publication until the guard is lifted "following implementation acceptance and Windows qualification"; amendment 2 §3.21 (revision 7, a DRAFT not yet reviewed) leaves the activation's route unset (O3) and says gwz-py also waits (O2). Under either option below that precondition holds: 5.5 runs after 5.6 (TR4.7's GO, the implementation acceptance) and after 6.2 and 6.3 (the Windows qualification evidence).
- Options: (a) step 4.8's dual review also accepts the implementation, and 5.5 is a short release-gate check; (b) a separate dual activation review at 5.5.
- **Recommend (b)**, with its review text prepared beside Phases 5 and 6 and run after 6.3, because its failure mode is a release that cannot publish gwz-cli. Confirm O1 (the trusted publisher's environment) now: it is an operator action on crates.io with no gate.

**OQ15. Keep the name `gwz_windows_https_qualification` while SSH is admitted?**
- Options: (a) keep it until step 5.1 retires it; (b) rename to `gwz_windows_qualification` now.
- **Recommend (a).** The switch has 41 sites in core and 13 files in gwz-cli and gwz-py; renaming it costs a review and a three-repo change for a switch whose life is a few phases. Note the name misdescribes the state for the duration, in the inventory and the checkpoint.

**OQ16. May the Phase 0 review be a skim?**
- Context: `GwzProcessOptimization.md` §8 sets one Consistency plus Safety review per phase. A skim review replaces it only on the operator's instruction (memory `review-granularity`: "when the operator says so"). Phase 0 changes no product code (a CI leg, a checker, a script, test gates and a test-support fix). Phase 1 changes product code (the socket wait, the file reader that refuses devices, SSH through the qualification boundary), and the other phases change more.
- Options: (a) the standard phase review for every phase, Phase 0 included; (b) a skim for Phase 0 only; (c) a skim for Phases 0 and 1.
- **Recommend (b).** The risk in Phase 0 is a wrong gate or a wrong CI claim, which one quick read finds; Phase 1 admits SSH on Windows and earns the full review. Until the operator answers, every phase review is the standard one.

## 9. What must be true on Windows for 1.1.0

A checklist; each line is a step or an existing plan item, so none is new scope.

- **CI.** The Windows leg is green on every push to main (0.1, 0.2). The conditional-compilation and Windows-parity checks are clean (0.3). The HTTPS, host and helper test modules run on the leg, with no skipped Unix-gated test (0.5, 1.8 and 4.11; at 5.1 Appendix B has no `unported` entry). `windows-matrix.yml` passes on the release commit (6.5; amendment 2 §3.12).
- **SSH on the transport.** Fetch, clone and push over SSH take the transport route on Windows with: an explicit key; URL password; Pageant first, else the OpenSSH agent pipe `SSH_AUTH_SOCK` names, else the default pipe; a missing agent a refusal (B07); `known_hosts` under libgit2's home order, with `HOME` unset working (B03); the key types and host-key algorithms libssh2 on Windows handles, with differences from 1.0.17 documented (3.8); dropped connections retried under today's `Retry` verdict (3.7); option A's background close and the idle watch active (1.5).
- **HTTPS on the transport.** Anonymous, default-credential (OD16, to any host), helper-identity and `gh` authentication; the machine proxy with the bypass list, the inverse precedence stated in the notes; 407, PAC and Digest as OQ10 and OQ11 settle (4.4 to 4.9).
- **Packaging.** The installed CLI and wheel work in paths with spaces and non-ASCII characters; the worker's provenance is checked (4.10).
- **Reviews.** TR1.8 GO with Surface (2.4); the per-step duals at 3.5, 4.1, 4.2 with 4.3, 4.4 and 4.8 (TR4.10's own Code and State review); Phase 3 and Phase 4 reviews; TR4.7's dual (5.6); gwz-sspi's activation review (5.5, after 5.6 and 6.3).
- **Publication.** `gwz-sspi 0.1.0` on crates.io before gwz-cli's release (5.5, which runs after 5.6, 6.2 and 6.3; §3.21).
- **Evidence.** TR8.4's rows pass or the operator has taken OD17 (6.1, 6.2); S5.6's Windows column has an evidence ID or an amended-out cell for every row (6.3); S7.3's rows pass on the Windows consumer builds (6.4), as do S6.3's dabeest rows for the wheel (5.3); the post-release check passes on the installed 1.1.0 (6.5). No row is claimed from a mock: a fake Pageant or pipe agent is a test of the product's code, and parity claims rest on real Pageant 0.83 and 1.0.17 runs.
- **Not required.** Windows ARM64, the server's Windows primitives (named-pipe ACLs, AppContainer refusals: Phase 7, 1.2.0), the Windows logon-session must-match row (1.2.0, §3.18).

## 10. Risks to the 1.1.0 schedule

1. **TR1.8 is the critical path and is stuck on approvals.** Its freeze needs rows that need a trust import, an account and a service the operator has not approved (OQ9). Mitigation: ask now; run Phase 1 meanwhile (OQ1). The policy-free steps of later phases (1.7, 3.2a, 3.4, 3.7) also run meanwhile, on accepted text rather than on OQ1's answer.
2. **dabeest is the only Windows host and is shared.** Phase 2's evidence alone is days of host time, and TR8.4 wants the host idle. Mitigation: push everything portable to `windows-2022` (section 7.2), and book the host in blocks.
3. **No Windows SSH server fixture exists** (OQ6). Without one, Phase 1's end-to-end proofs and every Phase 3 SSH row have nothing to talk to. This is why step 1.1 is first.
4. **Pageant is the least-proved piece.** Receiver quiescence, HWND reuse (unexecuted), and cross-SID behaviour are open (P01, P02), and a broker, if the review demands one, adds a secret owner and a step.
5. **WinCNG.** The transport inherits libssh2's Windows limits, including no ed25519, and the preference code was written assuming OpenSSL-class support. A user whose `known_hosts` has only an ed25519 line is the likely first complaint (X2, 3.8).
6. **Qualification switch removal spans three repositories** (41 sites in core, 13 files in the CLI and Python) and must merge in order (core, then CLI, then Python), while a lane that touches `transport_host` is in flight. Hot-spot discipline in section 4 matters.
7. **TR8.1 itself is not closed** on the other platforms (`CurrentProgramCheckpoint.md:14`, option A in a lane; OD17 pending). TR8.4 measures against it; a Windows number cannot settle before the Unix numbers do.
8. **gwz-sspi's activation** is a release-gate review nobody has scheduled (O3), and gwz-cli and gwz-py cannot publish without 0.1.0 (O2). Step 5.5 sits after 5.6, 6.2 and 6.3, so any slip in the Windows evidence is a slip in the last link before release step 6; ask for O1's confirmation now, since it has no gate.
9. **Unproved SSPI behaviour under a blocked provider** (P05) and Digest (P07) may force a broker or a scope cut late.
10. **Warnings and an unconfirmed leg.** The red tail is cleared in the tree (the 10 `retry_tests` in `1cdb9557`; the 115 merge failures were the fake-Git default), but the lib build's 152 warnings (143 before the 2026-10-08 check) and a hosted-runner confirmation remain: a leg that is not required, or whose warning count nobody reads, can hide a real regression if step 0.2 is skipped.

## 11. Out of scope

- Windows ARM64, Intel macOS and Linux ARM64 (unsupported in 1.1.0, V110 §2).
- 1.2.0's session host, reuse, server and its Windows named-pipe primitives.
- Any native-route fallback (OD15). The off switch stays the user's choice and is unchanged.
- Changing TR8.1's targets, the adaptive designs, or option A. This plan only proves them on Windows.
- The byte-comparing generator and Python protocol checks that `transport-candidate.yml` leaves to the Linux legs because the Windows checkouts of gwz-transport and taut are CRLF (step 0.2, exclusion 1): they test platform-neutral artifacts.
- WH1's disclosed, non-waived REDs: strict core Clippy RED45, the generator owner-IR pin mismatch, and six candidate-leg failures (`GwzWindowsHttpsIntegrationImplementationAcceptance.md:31-37`). They are not Windows-parity work; both candidate legs and CI are green at the tuple (`CurrentProgramCheckpoint.md:12`), and step 5.6 re-reads the settled tree's lint and pin state.
- WH1's remaining NO-GO gates (`GwzWindowsHttpsIntegrationImplementationAcceptance.md:28`) and where this plan closes them: platform in 6.2, 6.3 and 6.5; performance in 6.1; package in 4.10, 6.4 and 6.5; aggregate in 5.6 and S7.5. "Selected-source" is named there without a definition; this plan reads it as the release review's check that the tested source is the settled tuple (5.6 and S7.5). If the lane owner reads it differently, it needs a step of its own, and this plan does not invent one.
- Bare `#[cfg]` attributes on declarations in code outside the transport that this plan does not touch (`git/gitbackend/preservation*.rs`, `commit_tag_characterization.rs`; Table B.4). Root `AGENTS.md` asks that existing-code migrations be recorded, not claimed; this is that record. The two bare `#[cfg(unix)]` attributes inside the transport (`https_worker.rs:60, 98`) become `cfg_if` arms in 4.4.
- Splitting large files that this plan touches: `ssh_network.rs` is 471 lines, `ssh_setup.rs` 603, `ssh_pool.rs` 442. None is over the 1,000-line alarm; new Windows code goes in new files of at most 500 lines (U-table, steps 1.2, 1.3, 3.3, 3.5, 3.6).

## Appendix A: baseline rows still needed, and who runs them

| Row | Status today | Needed by | Step |
|---|---|---|---|
| B06 Pageant and OpenSSH both | unexecuted | OQ3 (note), 3.6 | 2.2 |
| B08 selected pipe, native service | partial (pipe absent) | OQ4, 3.3 | 2.2 (owned pipe), OQ9(3) for the service |
| B04 dispositions | characterized | OQ7, 3.1 | 2.1, 2.2 (X10) |
| B09 with P04: proxy grammar, loopback bypass, immutable snapshot | partial | OQ11, 4.6 | 2.3, 4.6 |
| B11 to B13 helper precedence | unexecuted | 4.4 | 2.3 |
| B14 Digest | unexecuted | OQ10, 4.8 | 2.3 |
| B15 zones and redirects | partial | 4.9 | 2.3, 4.9 |
| B16 EPA | unexecuted | 4.9 | 2.3 |
| B17 macOS and Linux, 1.0.17 against the loopback `Negotiate` fixture | partial | amendment 2 §3.5; 6.4's notes | 2.3b (OQ9(4)) |
| B18 POST challenge | unexecuted | 4.9 | 2.3 |
| P01 cross-SID Pageant | partial | 3.5 | 2.3 (OQ9(2)); if declined, the claim is removed (2.1) |
| P02 numeric HWND reuse | unexecuted, fixed cap | OQ5 | 2.1 (disposition: stays UNEXECUTED, claim revised) |
| P03 native service identity | partial | OQ4, 3.3 | 2.3 (OQ9(3)); if declined, the rule is removed or narrowed (2.1) |
| P05, P07 blocked provider, Digest | partial | 4.8 | 2.3 |
| P06 TLS and EPA | partial | 4.8, 4.9 | 2.1 (MD5 and lifetime dispositions), 2.3 (B16's EPA controls), 4.9 (lifetime) |
| X1 to X10 | new, this plan | 3.1, 3.3 to 3.8, 4.5 | 2.2 |

## Appendix B: the complete Unix-gate inventory at gwz-core `0e21bdde`

Step 0.3 seeds its checker from this appendix, and every gate here has an owner step that retires it. Section 3's `U1` to `U27` list the OS calls; this appendix lists every gate and OS-call line, which is more.

**How it was made.** `grep -rnE 'cfg\(.*(unix|windows)|cfg_attr|target_os|target_family|std::os::unix|std::os::windows|os::fd|libc::|use libc|os::unix|OsStrExt|OsStringExt|PermissionsExt|OpenOptionsExt'` over `src/git/endpoint`, `src/transport_host`, `src/transport_setting.rs`, `src/transport_setting`, `src/git/gitbackend` and `src/git/gitbackend.rs`, dropping the lines that name `gwz_windows_https_qualification`. Multi-line `cfg!(` forms were searched separately and are all qualification sites. The result: **183 lines in 81 files; 161 lines in 76 files are in scope (rows G1 to G76, Table B.1); 22 lines in 5 files are outside it (Table B.4).** The 41 lines in `src/` that name the qualification cfg, with `build.rs` and the boundary test, are step 5.1's. Kinds: P production code, T test module or test-only accessor, F fixture. End states: `ungate` (the gate goes), `pair` (a Windows arm joins the Unix one in the same `cfg_if`), `platform` (permanent, with a reason). Rows name the lines where the gate or OS call is; a module-level gate hides every line of the module it gates, which the module tables (B.2) list.

### Table B.1: gates by file

| # | File (gwz-core `src/`) | Gate lines | U-row | What is gated | End state | Owner step |
|---|---|---|---|---|---|---|
| G1 | `git/endpoint/agent_auth.rs` | 4, 216, 277 | U11 | P: the module; `libc::malloc` for the signature buffer; the test re-export | ungate | 3.4 |
| G2 | `git/endpoint/agent_job.rs` | 194 | U13 | P: `start_setup` | ungate | 1.4 |
| G3 | `git/endpoint/agent_socket.rs` | 3, 7, 16, 19, 21, 47, 51 | U10 | P: `AF_UNIX` connect, `libc::poll`, `EISCONN`, `EINPROGRESS` | platform (Unix arm; the Windows channels are `agent_pipe.rs` and `agent_pageant.rs`) | 3.3 and 3.6 add the Windows siblings; the entry stays, kind `platform` |
| G4 | `git/endpoint/helper_script.rs` | 23, 78 | U26 | F: `PermissionsExt`; Linux-only warm-up | pair | 4.3 |
| G5 | `git/endpoint/https_auth.rs` | 10, 110, 123, 128 | U20-U23 | P+T: `:10` imports, `:110` the helper module list (`executable`, `owner`, `lookup`, `runner`, `view`, `file_worker`); `:123`, `:128` test imports and `runner_tests` | ungate | 4.2 and 4.3 (their modules leave the `:110` block as each compiles), 4.4 (`:10` and the remainder, as U23), 4.11 (`:123`, `:128`) |
| G6 | `git/endpoint/https_auth/executable.rs` | 30, 32 | U21 | P: `mode & 0o111` | pair | 4.3 |
| G7 | `git/endpoint/https_auth/file_worker.rs` | 5, 68 | U22 | P: `OsStrExt`; `O_NONBLOCK` (`:68`) | pair | 4.3 (uses 1.3's reader) |
| G8 | `git/endpoint/https_auth/lookup.rs` | 165 | U20 | P: `process_group(0)` | pair | 4.2 |
| G9 | `git/endpoint/https_auth/owner.rs` | 245, 250 | U20 | P: `killpg(SIGKILL)` | pair | 4.2 |
| G10 | `git/endpoint/https_auth/runner.rs` | 3 | U22 | P: `OsStrExt` | pair | 4.3 |
| G11 | `git/endpoint/https_auth/runner/tests.rs` | 151 | U20 | T: `libc::kill(-pgid)` in a test guard | pair | 4.2 |
| G12 | `git/endpoint/https_auth/view.rs` | 3 | U22 | P: `OsStrExt` | pair | 4.3 |
| G13 | `git/endpoint/https_auth/view/framing.rs` | 4 | U22 | P: `OsStrExt` | pair | 4.3 |
| G14 | `git/endpoint/https_auth/view/tests.rs` | 3, 218, 255 | U22 | T: `OsStrExt`; `mkfifo` rows (`:218`, `:255`) | pair (the FIFO rows get 1.3's device-file twin, or stay `platform` with a reason) | 4.3 |
| G15 | `git/endpoint/https_auth_integration_tests.rs` | 2 | U26 | T: module gate; uses `helper_script` | ungate | 4.11 |
| G16 | `git/endpoint/https_budget_tests.rs` | 16 | U26 | T: module gate; uses `helper_script` | ungate | 4.11 |
| G17 | `git/endpoint/https_lifecycle_tests.rs` | 140 | U26 | T: nested gate; uses `helper_script` | ungate | 4.11 |
| G18 | `git/endpoint/https_opening_tests.rs` | 169, 321 | U26 | T: nested gates; uses `helper_script` | ungate | 4.11 |
| G19 | `git/endpoint/https_pool.rs` | 362 | - | T: `idle_budget_tests`, `idle_tests` | ungate | 0.5 |
| G20 | `git/endpoint/https_worker.rs` | 60, 98, 113, 118, 135, 166, 245, 252, 417-419 | U23 | P+T: `:60`, `:98` bare `#[cfg(unix)]`; `:113`, `:118`, `:135`, `:245`, `:252` helper-owner wiring; `:166` a test accessor; `:417-419` test modules | pair | 4.4 (production; it rewrites the bare attributes into `cfg_if`), 0.5 (`:166`; `setup_slot_tests`), 4.11 (`https_worker_tests`, `budget_tests`, `retry_tests`, `helper_budget_tests`, `credential_tests`) |
| G21 | `git/endpoint/https_worker/credentials.rs` | 19 | U23 | P: helper owner wiring | pair | 4.4 |
| G22 | `git/endpoint/https_worker/helper_budget_tests.rs` | 92 | U26 | T: nested gate; uses `helper_script` | ungate | 4.11 |
| G23 | `git/endpoint/https_worker/native.rs` | 406, 412 | - | T: `:406` test accessor, `:412` `mod tests` | ungate | 0.5 |
| G24 | `git/endpoint/https_worker/native/tests.rs` | 6, 63 | - | T: `:6` the module gate; `:63` `CommandExt` in the real SSPI request validator, a test that spawns `cargo` and needs an external `CARGO_TARGET_DIR` (two of WH1's disclosed failures) | ungate; `:63` pair | 0.5 (the leg sets `CARGO_TARGET_DIR`) |
| G25 | `git/endpoint/https_worker/native/tests/exchange.rs` | 4 | U26 | T: module gate; uses `helper_script` | ungate | 4.11 |
| G26 | `git/endpoint/https_worker/native/tests/protocol.rs` | 4 | - | T: module gate | ungate | 0.5 |
| G27 | `git/endpoint/https_worker/native/tests/retention.rs` | 4 | - | T: module gate | ungate | 0.5 |
| G28 | `git/endpoint/https_worker_tests.rs` | 34, 62 | U26 | T: nested gates; uses `helper_script` | ungate | 4.11 |
| G29 | `git/endpoint/mod.rs` | 21, 47 | U1, U2 | P+T: `:21` `idle_watch`, `ssh_password`, `ssh_setup` (`ssh_setup.rs:4` imports `idle_watch`); `:47` the test block of Table B.2 | ungate | 1.4 (all three modules compile together; 1.5 owns `idle_watch`'s behaviour); the `:47` block per Table B.2 |
| G30 | `git/endpoint/placement_endpoint.rs` | 284 | - | T: `check_tests` (`placement_endpoint_tests.rs`) | ungate | 1.8 |
| G31 | `git/endpoint/placement_endpoint_tests.rs` | 105, 109, 129 | - | T: `mkfifo`, `OpenOptionsExt`, `O_NONBLOCK`, `/tmp` literals | pair (the FIFO row gets 1.3's device-file twin) | 1.8 |
| G32 | `git/endpoint/ssh_key_auth.rs` | 19 | U12 | P: module gate | ungate | 1.4 |
| G33 | `git/endpoint/ssh_key_snapshot.rs` | 152-153, 155 | U8 | P: key file read with `O_NONBLOCK`; Windows arm `Unsupported` | pair | 1.3 |
| G34 | `git/endpoint/ssh_local.rs` | 4 | U12 | P: module gate | ungate | 1.4 |
| G35 | `git/endpoint/ssh_network.rs` | 5, 16-17, 137, 215-216, 238, 240, 243, 252, 321-324, 329, 334 | U3-U7 | P: the module-level arm; `AsRawFd`, `OpenOptionsExt`, `O_NONBLOCK` (`:137`), `EINPROGRESS`, `libc::poll` | pair | 1.2 (all but `:137`), 1.3 (`:137`, the `known_hosts` reader) |
| G36 | `git/endpoint/ssh_password_helpers.rs` | 30 | U14 | P: the password-only helper `lookup` | pair | 4.5 |
| G37 | `git/endpoint/ssh_tests/agent_auth.rs` | 2, 159, 188 | U11 | T: module gate; a `UnixStream` test `Channel` | pair | 3.4 (with 1.7's pipe `Channel`) |
| G38 | `git/endpoint/ssh_tests/agent_client.rs` | 277, 280, 331 | U10 | T: `:277` real-socket codec tests; `UnixListener`; `EINPROGRESS` | pair | 3.3 |
| G39 | `git/endpoint/ssh_tests/agent_fixture.rs` | 6 | U10, U25 | F: `:6` is an unconditional `use std::os::unix::net` (an OS-call line, not a gate); `ssh_tests/mod.rs:17` declares the module unconditionally | pair (1.4 adds a `cfg_if` around `mod agent_fixture`; the Unix file stays, `platform`) | 1.4 (the `cfg_if`), 1.7 (the Windows twin) |
| G40 | `git/endpoint/ssh_tests/agent_wait.rs` | 3, 7 | U10 | T: `mod unix` over a real socket | pair | 3.3 |
| G41 | `git/endpoint/ssh_tests/host_case.rs` | 8 | U12 | T: module gate | ungate | 1.4 |
| G42 | `git/endpoint/ssh_tests/key_container.rs` | 2 | U12 | T: module gate | ungate | 1.4 |
| G43 | `git/endpoint/ssh_tests/key_files.rs` | 7 | U12 | T: module gate; uses `key_fixture` | ungate | 3.8 (needs 1.7) |
| G44 | `git/endpoint/ssh_tests/key_fixture.rs` | 90, 187, 196-197, 199, 204 | U25 | F: `UnixStream`, `libc::kill`, `waitpid`, `ESRCH`, `ECHILD` (the key agent) | platform (Unix file stays; Windows twin is new) | 1.7 adds the twin |
| G45 | `git/endpoint/ssh_tests/key_types.rs` | 8 | U12 | T: module gate; uses `key_fixture` | ungate | 3.8 |
| G46 | `git/endpoint/ssh_tests/local_endpoint.rs` | 4 | U12 | T: module gate; uses `agent_fixture`, `key_fixture` | ungate | 3.6 (needs 1.7) |
| G47 | `git/endpoint/ssh_tests/max_startups.rs` | 22, 107 | U16 | T: module gate; `target_os = "macos"` branch (`:107`) | ungate | 3.7 |
| G48 | `git/endpoint/ssh_tests/mod.rs` | 29, 50 | U2 | T: `:29` `key_fixture` and `:50` `password_helpers` (both already wrapped). Not grep lines, but declarations that 1.4 guards: `:17` `mod agent_fixture`, and `mod idle_loss` and `mod idle_loss_budget`; the other declarations are in Table B.2 | pair | 1.4 (the two new guards), 1.7 (`key_fixture`, `agent_fixture`), 1.5 (the `idle_loss` guard comes off), 4.5 (`password_helpers`) |
| G49 | `git/endpoint/ssh_tests/network.rs` | 2 | U3-U6 | T: module gate; rows that need no `sshd`, the rest, and agent rows | ungate | 1.2 (rows that need no `sshd`), 1.4 (the rest), 3.4 (agent rows) |
| G50 | `git/endpoint/ssh_tests/password_helpers.rs` | 177 | U14 | T: module gate; `libc::kill(pid, 0)` | pair | 4.5 |
| G51 | `git/endpoint/ssh_tests/retry.rs` | 7 | - | T: module gate | ungate | 1.8 |
| G52 | `git/endpoint/ssh_tests/rsa_sha1.rs` | 7 | U12 | T: module gate; uses `key_fixture` | ungate | 3.8 |
| G53 | `git/endpoint/ssh_tests/selected_key.rs` | 2, 276, 280 | U12 | T: module gate; `symlink` and `mkfifo` rows (`:276-280`) | ungate; the FIFO rows pair with 1.3's device-file twin | 1.4 |
| G54 | `git/endpoint/ssh_tests/selected_pool.rs` | 3 | U12 | T: module gate | ungate | 1.4 |
| G55 | `git/endpoint/ssh_tests/supervised.rs` | 87, 204 | U12 | T: `:87`, `:204`; uses `agent_fixture` | ungate | 3.6 (needs 1.7) |
| G56 | `git/endpoint/ssh_worker/endpoint.rs` | 135, 138, 143 | U9 | P: identity-file check with `O_NONBLOCK`; Windows arm `Unsupported` | pair | 1.3 |
| G57 | `git/gitbackend.rs` | 45 | - | T: `:45` `transport_candidate_tests` under `all(test, unix, gwz_transport_candidate)` | ungate | 1.8 |
| G58 | `git/gitbackend/transport_binding.rs` | 306 | - | T: `:306` `https_transport_binding_tests` | ungate | 0.5 |
| G59 | `git/gitbackend/transport_candidate_tests/drivers.rs` | 254 | U26 | T: `PermissionsExt` (a script writer) | pair | 4.11 |
| G60 | `git/gitbackend/transport_observations.rs` | 197 | - | T: `:197` `https_tests` | ungate | 0.5 |
| G61 | `git/gitbackend/transport_support/identity.rs` | 204-206 | U7-U9 | P: the selected identity opened with `O_NONBLOCK` on Unix only (`:204-206`) | pair | 1.3 (a caller of the regular-file reader) |
| G62 | `transport_host/ca_bundle_tests.rs` | 118 | - | T: `target_os = "linux"` (`:118`), the OpenSSL default-paths test | platform (an OpenSSL-only trust branch) | 0.5 records the reason |
| G63 | `transport_host/cleanup_tests.rs` | 181 | - | T: `:181` a test that builds the real Unix SSH endpoint | ungate | 1.8 |
| G64 | `transport_host/endpoint_environment.rs` | 20, 59, 75, 115, 122, 177 | U17, U24 | P: `:20` imports; `:59` SSH settings; `:75` helper `auth` config; `:115`, `:122` environment proxy; `:177` `platform::ssh_home`, `agent` | pair | 1.6 (`:20`, `:59` interim), 3.1 (`:59`, `:177` home), 3.2b (`:177` agent), 4.3 (`:75`), 4.6 and 4.7 (`:115`, `:122`) |
| G65 | `transport_host/https_endpoint.rs` | 93, 517, 562 | - | T: `:93` a test accessor; `:517` a test clock; `:562` test modules | ungate | 0.5 |
| G66 | `transport_host/https_route_scale_tests.rs` | 30 | - | T: `std::os::unix::fs::symlink` (`:30`); builds the runtime with an SSH configuration | pair (a directory junction or a copy) | 1.8 |
| G67 | `transport_host/mod.rs` | 15, 28, 43 | U18 | P+T: `:15` the test block of Table B.2; `:28` the three projection modules; `:43` the `ssh_local` import | ungate | 1.6 (`:43`), 4.11 (`:28`); `:15` per Table B.2 |
| G68 | `transport_host/qualification_tests.rs` | 272 | - | T: `:272` the Unix constructor tests (the `else if unix` arm) | pair | 1.6 (SSH admitted), 5.1 (switch retired) |
| G69 | `transport_host/request.rs` | 446 | - | T: `:446` `https_budget_gate_tests` (it builds a `TransportRuntime` with an SSH configuration) | ungate | 1.8 |
| G70 | `transport_host/session.rs` | 37, 259, 417 | U19 | P+T: `:37` `wake_tests`; `:259` test accessors; `:417` SSH engine construction | ungate | 1.6 (`:417`), 0.5 (`:37`, `:259`) |
| G71 | `transport_setting.rs` | 283 | - | T: `:283` `mod tests` under `all(test, unix)` | ungate | 3.1 |
| G72 | `transport_setting/tests/files.rs` | 6, 394 | - | T: `PermissionsExt`, `symlink` | pair | 3.1 |
| G73 | `transport_setting/tests/fixture.rs` | 7, 53, 116 | - | T: `OsStrExt`, `mkfifo`, `libgit2_sys` home-dir option | pair | 3.1 |
| G74 | `transport_setting/tests/precedence.rs` | 4-5 | - | T: `OsStrExt`, `PermissionsExt` | pair | 3.1 |
| G75 | `transport_setting/tests/scan.rs` | 4 | - | T: `symlink` | pair | 3.1 |
| G76 | `transport_setting/tests/text.rs` | 8 | - | T: `OsStrExt` | pair | 3.1 |

### Table B.2: the modules behind the block gates

**`git/endpoint/mod.rs:47-62`** (`all(test, unix)`, 13 modules):

| Module | What it needs | Owner |
|---|---|---|
| `budget_wait_tests`, `git_turns_tests`, `ssh_pump_clock_tests`, `ssh_destination_tests` | nothing OS-specific (`/tmp` literals at `budget_wait_tests.rs:187`, `ssh_destination_tests.rs:213`) | 1.8 |
| `job_budget_wait_tests` | `ssh_setup::SetupConnector` (`:21`, 1.4's) and `https_fixture` (0.5's); `/tmp` home at `:350` | 1.8 |
| `cut_proxy`, `https_fixture`, `https_local`, `https_opening` | the HTTPS fixtures | 0.5 |
| `helper_script` | `PermissionsExt` fake scripts (U26) | 4.3 |
| `ssh_fixture`, `ssh_password_fixture` | `sshd`, `ssh-keygen`, `kill -STOP`, `ps` (U25) | 1.1 |
| `ssh_tests` | the SSH server; its 33 files besides `mod.rs` are listed in the next table | 1.4 takes the module out of the block |

**`ssh_tests/mod.rs`** (33 files besides `mod.rs`, at `0e21bdde`). When 1.4 takes `mod ssh_tests` out of the block above, every file below compiles on Windows unless it has its own gate or 1.4 adds one. The imports of each file were grepped for anything owned by a later step.

| Files | Gate today | What they need | Owner |
|---|---|---|---|
| `agent_capacity`, `agent_keys` (imports `agent_client`, portable), `attachment`, `cleanup_capacity`, `pool_host`, `pooled`, `pump`, `remote_bridge` | none | nothing OS-specific | 1.4 (they run from the step) |
| `channel`, `pooled_remote`, `regression`, `worker` | none | `ssh_fixture` (1.1) | 1.4 |
| `placement_endpoint` | none | `ssh_setup` (1.4) | 1.4 |
| `idle_loss`, `idle_loss_budget` | none; 1.4 adds a paired `cfg_if` in `mod.rs` | `ssh_setup` (1.4), `cut_proxy` (0.5); they run 1.5's behaviour | guard 1.4, removed by 1.5 (needs 0.5's `cut_proxy`) |
| `agent_fixture` | none; `:6` imports `std::os::unix::net` unconditionally; 1.4 adds a paired `cfg_if` around `mod agent_fixture` | `ssh_fixture` (1.1) | guard 1.4, Windows twin 1.7 |
| `host_case`, `key_container`, `selected_key`, `selected_pool` | file-level `cfg(unix)` | `ssh_fixture`, `ssh_network` | 1.4 |
| `network` | file-level `cfg(unix)` | `ssh_network`; agent rows need `agent_auth`, `agent_socket`, `agent_fixture` | 1.2 (rows needing no `sshd`), 1.4 (the rest), 3.4 (agent rows) |
| `retry` | file-level `cfg(unix)` | `ssh_local` (1.4), `ssh_fixture` | 1.8 |
| `agent_auth` | file-level `cfg(unix)` | `agent_auth` (3.4), the fixture twin (1.7) | 3.4 |
| `agent_wait` | file-level `cfg(unix)` | real-socket agent channel | 3.3 |
| `agent_client` | partly gated: one `cfg(unix)` region, `:276-355` (Table B.1 G38) | real-socket agent channel inside the region; the rest is portable | 3.3 (the region); 1.4 (the ungated tests, which run from that step; on Windows `:3`'s `agent_socket` import is unused until 3.3) |
| `key_files`, `key_types`, `rsa_sha1` | file-level `cfg(unix)` | `key_fixture` (1.7), `ssh_password_fixture` | 3.8 |
| `local_endpoint` | file-level `cfg(unix)` | `agent_fixture`, `key_fixture` (1.7) | 3.6 |
| `supervised` | partly gated: two `cfg(unix)` regions, from `:86` and `:203` (Table B.1 G55) | `agent_fixture`, `key_fixture` (1.7) inside the regions; the rest is portable | 3.6 (the regions); 1.4 (the ungated tests, which run from that step; on Windows `:5`'s `agent_auth, agent_socket` imports are unused until 3.6) |
| `max_startups` | file-level `cfg(unix)` | `ssh_setup` (1.4) | 3.7 |
| `key_fixture`, `password_helpers` | wrapped in `cfg(unix)` at `mod.rs:29`, `:50` | the key agent (Unix); the helper machinery | 1.7; 4.5 |

That is 16 files with no gate, 15 with their own gate (13 file-level, 2 partial: `agent_client` and `supervised`) and 2 wrapped in `mod.rs`.

**`transport_host/mod.rs:15-40`** (`all(test, unix)`, 20 modules; the three projection modules also sit under a nested `cfg(unix)` at `:28`):

| Modules | Why | Owner |
|---|---|---|
| `fetch_preflight_tests`, `cancellable_https_tests`, `ca_bundle_tests` | no runtime built, no helper script | 0.5 |
| `tests`, `fault_tests`, `throughput_tests`, `message_embedding_tests`, `https_route_scale_tests`, `https_compat_tests`, `endpoint_environment_tests`, `retry_tests` | build a `TransportRuntime` with an SSH configuration, no helper script | 1.8 |
| `driver_tests`, `close_tests`, `command_tests`, `cancellable_tests`, `https_tests`, `https_policy_tests`, `https_helper_projection_tests`, `ssh_helper_projection_tests`, `https_negotiate_projection_tests` | use `helper_script` (Table B.3) | 4.11 |

The assignments follow the tuple's greps (`SshEndpointConfig::fixture`, `TransportRuntime::new`, `helper_script`). A step's inventory pass may move a module, and records the move in step 0.3's inventory.

**`https_worker.rs:417-419`:** `https_worker_tests`, `budget_tests` (`https_budget_tests.rs`), `retry_tests`, `helper_budget_tests`, `credential_tests` are 4.11's; `setup_slot_tests` is 0.5's. **Others:** `https_pool.rs:362` (`idle_budget_tests`, `idle_tests`), `https_endpoint.rs:562` (`cancellation_tests`, `https_cancel_mux_tests`, `retry_tests`, `stale_action_tests`, `wake_tests`), `session.rs:37` (`wake_tests`) and `https_worker/native.rs:412` (`tests`, with `protocol` and `retention`) are 0.5's; `native/tests/exchange.rs` is 4.11's.

### Table B.3: the helper boundary (28 files that mention `helper_script` at the tuple)

These need 4.3's Windows fake-helper form (U26), and most need 4.2's Job owner or 4.4's admission. Step 0.5 leaves them; step 4.11 owns the ones not already owned by 4.2 to 4.5.

`git/endpoint/helper_script.rs`, `git/endpoint/https_auth/runner/tests.rs`, `git/endpoint/https_auth/runner_tests.rs`, `git/endpoint/https_auth/test_support.rs`, `git/endpoint/https_auth/view/tests.rs`, `git/endpoint/https_auth_integration_tests.rs`, `git/endpoint/https_budget_tests.rs`, `git/endpoint/https_lifecycle_tests.rs`, `git/endpoint/https_opening_tests.rs`, `git/endpoint/https_worker/credential_tests.rs`, `git/endpoint/https_worker/helper_budget_tests.rs`, `git/endpoint/https_worker/native/tests/exchange.rs`, `git/endpoint/https_worker/retry_tests.rs`, `git/endpoint/https_worker_tests.rs`, `git/endpoint/https_worker_tests/configured_helpers.rs`, `git/endpoint/mod.rs`, `git/endpoint/ssh_tests/password_helpers.rs`, `transport_host/cancellable_tests.rs`, `transport_host/close_tests.rs`, `transport_host/command_tests.rs`, `transport_host/driver_tests.rs`, `transport_host/https_helper_projection_tests.rs`, `transport_host/https_negotiate_projection_tests.rs`, `transport_host/https_policy_tests.rs`, `transport_host/https_policy_tests/cancellation.rs`, `transport_host/https_tests.rs`, `transport_host/ssh_helper_projection_tests.rs`, `transport_host/ssh_helper_projection_tests/enablement.rs`.
### Table B.4: found by the wider `src/git/gitbackend` grep, outside 0.3's scope

These are not transport code (`src/git/gitbackend/transport_*` is the scope). The preservation files already carry paired Windows arms; the characterization test is a Unix-only test of Git hooks. They are listed so the grep's remainder is accounted for. They use bare `#[cfg]` on declarations, which root `AGENTS.md` forbids in new code; migrating them is recorded here (section 11), not claimed.

| File (`src/`) | Lines | What |
|---|---|---|
| `git/gitbackend/preservation.rs` | 616, 618, 620, 622, 624, 626 | fault-boundary names, paired `cfg(unix)` / `cfg(windows)` variants |
| `git/gitbackend/commit_tag_characterization.rs` | 78, 81 | a Unix-only characterization test of Git hooks (`mod unix`, `PermissionsExt`) |
| `git/gitbackend/preservation_image.rs` | 551, 553 | `cfg(unix)` / `cfg(not(unix))` pair for the executable bit |
| `git/gitbackend/preservation_root/parent.rs` | 102, 244, 249, 259, 268, 273 | paired `cfg(unix)` / `cfg(windows)` directory-sync barriers |
| `git/gitbackend/preservation_root/files.rs` | 105, 107, 111, 118, 120, 124 | paired `cfg(unix)` / `cfg(not(unix))` raw-path conversions |

### Table B.5: literal Unix paths in files that this plan ungates or ports

`grep -rln '"/tmp\|/usr/\|"/bin/\|/dev/null'` over the three directories finds the files below. Each file's ungating step also makes these paths portable (the class of defect `1cdb9557` fixed in `retry_tests`, where `/tmp` has no drive on Windows and `PlacementEndpoint::new` refused it).

`git/endpoint/budget_wait_tests.rs`, `git/endpoint/helper_script.rs`, `git/endpoint/https_auth/runner.rs`, `git/endpoint/https_auth/runner/tests.rs`, `git/endpoint/https_auth/runner_tests.rs`, `git/endpoint/https_auth/test_support.rs`, `git/endpoint/https_auth/view/tests.rs`, `git/endpoint/https_budget_tests.rs`, `git/endpoint/https_lifecycle_tests.rs`, `git/endpoint/https_opening_tests.rs`, `git/endpoint/https_worker/helper_budget_tests.rs`, `git/endpoint/https_worker/native/tests.rs`, `git/endpoint/https_worker_tests.rs`, `git/endpoint/job_budget_wait_tests.rs`, `git/endpoint/placement_endpoint_tests.rs`, `git/endpoint/ssh_destination_tests.rs`, `git/endpoint/ssh_fixture.rs`, `git/endpoint/ssh_tests/mod.rs`, `git/endpoint/ssh_tests/network.rs`, `git/endpoint/ssh_tests/password_helpers.rs`, `git/endpoint/ssh_tests/retry.rs`, `transport_host/cancellable_tests.rs`, `transport_host/driver_tests.rs`, `transport_host/https_helper_projection_tests.rs`, `transport_host/https_tests.rs`.
## Changelog

- **2026-10-08, operator decisions.** Every OQ of section 8 decided as recommended; recorded at the head of section 8.
- **2026-10-08, revision 3 filed.** Consistency round 3 (`-ReviewConsistency-3.md`) GO; its one P3, N3-2, applied as a text edit at filing as it allowed: Table B.2 marks `agent_client` and `supervised` as partly gated, with their ungated tests running from 1.4.
- **2026-10-08, revision 3.** Resolves [GwzTransportWindowsParityPlan-ReviewConsistency-2.md] (revision 2 NO-GO on N2-1, with N3-1; all of round 1 confirmed closed). N2-1: `idle_watch` compiles with `ssh_setup` in 1.4 (1.5 keeps the behaviour); `ssh_tests/mod.rs` has its own rows in Table B.2 (16 gate-less files with owners); 1.4 adds a paired `cfg_if` around `mod agent_fixture` (owner 1.7) and around `mod idle_loss` and `mod idle_loss_budget` (owner 1.5, which needs 0.5's `cut_proxy`), and Table B.1's agent_fixture row no longer calls `:6` a gate; 1.4 ── 1.7 is an edge, which 3.4 inherits; `job_budget_wait_tests` moves to 1.8. N3-1: the `transport_host` hot-spot order follows dependency (policy-free 3.2a and 4.3 merge when ready; the gated steps follow in order). Each moved or guarded module's imports were re-grepped at `0e21bdde`.
- **2026-10-08, revision 2.** Resolves [GwzTransportWindowsParityPlan-ReviewConsistency.md] (revision 1, NO-GO: P2-1 to P2-5 and twelve P3s); `GwzTransportWindowsParityPlan-RemPlan.md` maps each finding to its fix. Main changes: 5.5 now follows 5.6 and Phase 6's 6.2 and 6.3 (P2-1); 3.2 splits into the policy-free 3.2a and the TR1.8-gated 3.2b, and 4.2 and 4.3 implement 4.1's contract (P2-2); the named-pipe agent fixture is its own early step 1.7 (P2-3); B17 is step 2.3b, B09/P04, P02, P04 and P06 close as 2.1 and 2.3 say, and no clause is left provisional (P2-4); Appendix B is the complete gate inventory, with new steps 1.8 and 4.11, a split 0.5, 1.3's extra caller, and 0.3 seeded from it (P2-5); the header tuple, State, citations, hot spots and review tiers are corrected, with new OQ16 and a new row X10 (the P3s).
- **Revision 1 (2026-10-08).** The lane owner's additions to draft 0: the State paragraphs of 0.1 and 0.2, and step 0.5. Reviewed NO-GO.
- **Draft 0 (2026-10-08).** `GwzTransportWindowsParityPlan.draft0.md`.
