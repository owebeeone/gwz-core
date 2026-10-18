# GWZ transport: the rest of Windows parity for 1.1.0, a phased plan

Date: 2026-10-08. Status: **DRAFT, not reviewed.** Drafted for the lane owner, who reviews it and files it in `gwz-core/dev-docs`. It decides nothing: every choice that needs the operator is an OQ in section 8, with options and a recommendation. It authorizes no implementation, commit, push, tag or publish.

Read-only sources: workspace root `1b019d67`, gwz-core `2a12006f` (working tree), gwz-transport `9eef731`. Nothing was run on dabeest for this plan. Where a fact needs a Windows check, the plan has a step or an OQ for it.

**Labels.** Step numbers (`0.1`, `3.5`) are this document's own. Requirement IDs (TR1.8, TR4.6 to TR4.10, TR8.4, S4.1 to S4.5, OD13, OD15, OD16) are the release plan's and its amendments', cited as they stand. Baseline rows `B01` to `B18` and primitives `P01` to `P08` are `GwzTransportWindowsBaseline.md`'s. New baseline rows this plan proposes are `X1` to `X9` (step 2.2).

## 1. Why this plan, and what is already decided

- **OD13 (operator, 2026-10-01).** 1.1.0 is the transport used in process by the `gwz` CLI on macOS ARM64, Linux x86-64 and Windows x86-64, with Windows parity (`dev-docs/CurrentProgramCheckpoint.md:1411-1413`; amendment 2 header).
- **OD15 (operator, 2026-10-01).** Windows parity is built into the transport. Native routes are not the way to it (`CurrentProgramCheckpoint.md:1445`; memory `no-native-route-fallbacks`). TR1.8 designs it, TR4.8 to TR4.10 implement it (`GwzTransportReleasePlanAmendment-2.md` §3.5).
- **OD16 (operator, 2026-10-02).** No zone bound: the logon session's default credentials go to any host that answers `Negotiate` or `NTLM`, as 1.0.17 does (`CurrentProgramCheckpoint.md:1453` ff.).
- **What has happened since.** WH1 (Windows HTTPS integration) was accepted with limits on 2026-10-04. Every transport change after it (option A, idle loss, the adaptive concurrency design and its Phase 1, the HTTPS fixed-cost fix) was built and tested on macOS and Linux only. Windows has no SSH transport at all.
- **The 2026-10-08 check on dabeest.** The Windows HTTPS candidate at gwz-core `2a12006f` equals WH1 on every row (`gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-08-windows-candidate-check/README.md`, uncommitted at the time of writing). So Unix-only design has not yet broken what Windows has. It has also not moved Windows forward, and each new Unix-only piece makes the later port larger.
- **What this plan is for.** Schedule the remaining Windows work now, foundational first, so that several agents can take steps at once, and so that the next transport change cannot add a Unix-only dependency without it being counted (step 0.3).

## 2. Where Windows stands today

### 2.1 Build and CI

- **No push-triggered Windows job.** Amendment 2 §2: "No push-triggered CI job builds or tests gwz-core on Windows"; `windows-matrix.yml` and `platform-matrix.yml` run on dispatch only (`GwzTransportReleasePlanAmendment-2.md:63`). Another agent is adding a `windows-2022` leg to `transport-candidate.yml` now (TR4.6).
- **The Windows candidate is HTTPS-only.** `src/git/mod.rs:5` compiles `endpoint` on Windows only under `all(windows, gwz_transport_candidate, gwz_windows_https_qualification)`. The qualification cfg appears at 41 sites in gwz-core `src/` and in gwz-cli (7 files) and gwz-py (6 files).
- **Windows full-lib red tail (2026-10-08 check).** `cargo test --lib` on Windows: 2170 passed, 125 failed. 115 are `workspace_ops::merge::v1_lifecycle` tests that need the real-Git fixture (`windows-matrix.yml` sets `GWZ_TEST_GIT=real`); 10 are `git::endpoint::placement_endpoint::retry_tests`, which fail with `Kind(InvalidInput)` because `scripted_endpoint` passes `PathBuf::from("/tmp")` as the SSH home (`retry_tests.rs:134`), which is not absolute on Windows.
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
| Negotiate-only, NTLM-only, mixed, Digest, EPA, POST challenge, redirect across zones | B11 to B16, B18 | **unexecuted** |

What 1.0.17's SSH credential callback offers (`src/git/gitbackend/transport_support.rs:240-280`, and the selected-identity form at `:219-228`): the ssh-agent once (`Cred::ssh_key_from_agent`, `:260`), a username, the configured credential helper for user and password, and default credentials. It has **no keyboard-interactive branch** and passes `None` as the passphrase for a selected key file (`:228`, error text at `:224`). So parity needs neither keyboard-interactive nor passphrase prompts. A URL password is offered through libssh2 first when the server lists `password` (`ssh_password.rs:1-20`; platform-neutral).

What libssh2 does on Windows (`libssh2-sys` 0.3.3, the version both 1.0.17's `libgit2-sys` feature `ssh` and the candidate use, `git2-rs/libgit2-sys/Cargo.toml:26,37`; candidate pin `tests/transport_backend/prepare.py:78`):

- **Agent order.** `supported_backends` is Pageant, then OpenSSH, and `libssh2_agent_connect` takes the first that connects (`agent.c:436-441, 816-826`). Pageant "connects" if `FindWindowA("Pageant","Pageant")` finds a window (`agent.c:343`), so a visible Pageant with no keys never falls through to the pipe. The OpenSSH backend uses `SSH_AUTH_SOCK`, else `\\.\pipe\openssh-ssh-agent` (`agent_win.c:124-139`), connects with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, and checks nothing about the pipe's server.
- **Pageant's wire shape.** A mapping named `PageantRequest%08x` (the thread ID) with no security attributes, 8192 bytes, `SendMessage` with no timeout (`agent.c:366-394`). TR1.8's design is deliberately stricter (section 2.4).
- **Crypto backend.** WinCNG unless the `openssl-on-win32` feature is on (`libssh2-sys` `build.rs:97-114`). WinCNG defines `LIBSSH2_ED25519 0` (`wincng.h:74`). So **ed25519 host keys and ed25519 file keys are not available in libssh2 on Windows**, in 1.0.17 and in the transport alike. The transport's host-key preference list names `ssh-ed25519` (`ssh_network.rs:24`, `preferences` at `:346`), and `libssh2_session_method_pref` strips unsupported names and fails if none are left (`kex.c:4192` ff.). What 1.0.17 does with an ed25519-only `known_hosts` entry on Windows is unknown (new row X2, step 2.2).

### 2.4 Windows design state (TR1.8)

`GwzTransportWindowsParityDesign.md` is a DRAFT and **NO-GO**. Its header: "No downstream Windows implementation may consume this as GO." It predates MAIN's accepted helper-timing, configuration-view and SSH-clock amendments and carries an SSPI-only supersession list; its own §11 says to refresh the controlling graph before settling. It proposes, among others: Pageant through `WM_COPYDATA` with a `Local\` mapping that has an ACL, a single `SendMessageTimeoutW`, and a pinned window identity (§4, §5); an owned `AgentSource` enum rather than a `PathBuf` (§4); the machine proxy winning over the environment, with 407 refused unless 1.0.17 authenticates (§6); helper-identity precedence over `Negotiate`, `NTLM`, `Digest`, `Basic` (§7); a channel binding taken from the final origin handshake (§8); HOME order `HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE` (§3).

Open physical rows before it can freeze (baseline §3, §4): B06, B08, B11 to B16, B18; P01 (cross-SID), P02 (HWND reuse, unexecuted), P03 (native service identity), P05 (blocked provider), P06 (MD5 peers, EPA), P07 (Digest). Blocked on operator approvals recorded in the baseline: Windows native trust ("held"), a distinct account, and a coordinated service or agent setup. `GwzTransportWindowsProofDispositions-DRAFT.md` proposes dispositions for HOME (B04), Pageant timeout ownership (P02), window identity and weak certificate hashes; none is accepted.

### 2.5 gwz-sspi

- Status line: caller values, private codec, parent supervision, the serial Windows Negotiate/NTLM worker and installed-host packaging "accepted within their bounded gates. HTTP/core composition, full Windows qualification and release remain gated"; Digest is refused (`gwz-sspi/README.md`; its body also says installed-host packaging "has its own pending review").
- Publication gate: `publish = false`; `release_checks.py` refuses preparation and publication until a reviewed activation "after implementation acceptance and Windows qualification" (`gwz-sspi/RELEASE.md`, "SSPI release gate").
- Registry: the placeholder `0.0.0-bootstrap.1` has been on crates.io since 2026-10-04; trusted publishing only; the publisher's environment name is unconfirmed (amendment 2 §3.21, O1). gwz-cli's crates.io publication of 1.1.0 waits on `gwz-sspi 0.1.0`, and gwz-py appears to (O2). The activation's review route and date are unset (O3). Release step 5a is the new prerequisite of step 6 (§3.21 part C).

## 3. Why SSH setup is Unix-only, and what maps to what

The SSH path is not process- or fd-heavy. It is **threads plus non-blocking sockets plus sliced waits**: setup runs as a supervised job on its own thread (`agent_job.rs:194` `start_setup`, `Job::start`), every wait goes through `Control::wait_step`, which hands a closure at most 20 ms (`agent_job/control.rs:153-190`), and the closure is the only place the OS appears. libssh2 does the protocol and the I/O on a `std::net::TcpStream` made non-blocking (`ssh_connection.rs:22-33`). The worker loop is a parked thread (`ssh_worker/runner.rs`). Idle sockets are watched by a small tokio reactor on a duplicated socket (`idle_watch.rs`). So the Unix-only pieces are a short list of OS calls, not an architecture. Each, with its Windows counterpart:

| # | Where | Unix dependency | Windows counterpart | Step |
|---|---|---|---|---|
| U1 | `git/endpoint/mod.rs:21` | `idle_watch`, `ssh_password`, `ssh_setup` compiled on Unix only | the same modules, once U3 to U9 are portable | 1.4, 1.5 |
| U2 | `git/endpoint/mod.rs:47-62` | SSH, HTTPS and password test fixtures compiled `all(test, unix)` | Windows fixtures (U25) | 1.1 |
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
| U15 | `idle_watch.rs` | `tokio::net::TcpStream::from_std` on a `try_clone` and `poll_peek` | the same code under tokio's Windows driver; `GwzTransportIdleLossDesign.md` §9 says it "needs its own run" | 1.5 |
| U16 | `ssh_setup.rs:437`; `ssh_tests/max_startups.rs` (`cfg(unix)` throughout) | a `MaxStartups` drop maps `ConnectionAborted` to `Cancelled`, which the retry machine returns to the member | pin what Windows reports (`WSAECONNABORTED`, `WSAECONNRESET`) and map a drop to Suspect (`GwzTransportAdaptiveConcurrencyDesign.md` F3, test 20) | 3.7 |
| U17 | `transport_host/endpoint_environment.rs:20-23, 59-64, 177-200` | SSH home is `HOME` when absolute; agent is `SSH_AUTH_SOCK`; Windows `platform` has only `direct()` (`:203-240`) | HOME order and an `AgentSource` (TR1.8 §3, §4) | 3.1, 3.2 |
| U18 | `transport_host/mod.rs:43, 87-88, 138-141, 188-190, 222-224, 280-282` | `ssh_local` import is `cfg(unix)`; `SshSettings.agent: Option<PathBuf>`; the qualification refusals | `AgentSource`; admit SSH | 1.6, 3.2 |
| U19 | `transport_host/session.rs:417-440, 459-462` | SSH engine construction is `cfg(unix)`; Windows schemes are `[Https]` | construct the SSH engine; schemes `[Ssh, Https]` | 1.6 |
| U20 | `https_auth/owner.rs:245-254`; `https_auth/lookup.rs:165-170` | helper tree killed with `killpg(SIGKILL)`; `process_group(0)` | Job Object with kill-on-close, assigned at creation (TR1.8 §10; P08 executed) | 4.2 |
| U21 | `https_auth/executable.rs:30-35` | executable means `mode & 0o111` | `git.exe` found on the captured absolute `PATH`, no implicit cwd or shell-script extension (TR1.8 §10) | 4.3 |
| U22 | `https_auth/runner.rs:3`, `view.rs:3`, `view/framing.rs:4`, `file_worker.rs:5, 68` | `OsStrExt` bytes; `O_NONBLOCK` | UTF-16 round trip without lossy conversion; the U7 reader | 4.3 |
| U23 | `https_auth.rs:10, 110, 128`; `https_worker.rs:60, 98, 113, 118, 135, 245, 252`; `https_worker/credentials.rs:19` | helper owner wiring behind `cfg(unix)` | admit `WindowsConfigured` and `Gh` | 4.4 |
| U24 | `endpoint_environment.rs:115, 122-170` | environment proxy | WinHTTP machine proxy (TR1.8 §6) | 4.6, 4.7 |
| U25 | `ssh_fixture.rs:73-74, 106-109, 256, 291, 306, 330` | test server is `/usr/sbin/sshd`; also `ssh-keygen`, `kill -STOP`, `ps -axo` | a Windows SSH server fixture and process controls (OQ6) | 1.1 |
| U26 | `helper_script.rs:23, 78` | `PermissionsExt`, Linux-only warm-up | `.cmd` or `.exe` fake helpers | 4.3 |
| U27 | `Cargo.toml:103-108`; `tests/transport_backend/prepare.py:87` | `windows-sys` features are `Win32_Foundation`, `Win32_Globalization`, `Win32_Storage_FileSystem`; the candidate adds only `Win32_Networking_WinHttp` | add `Win32_Networking_WinSock`, `Win32_System_Pipes`, `Win32_System_IO`, `Win32_System_Threading`, `Win32_Security`, `Win32_UI_WindowsAndMessaging`, `Win32_System_Memory`, `Win32_System_JobObjects` as each step needs them, in the candidate's extra list first | 1.2 |

**Conclusion for the readiness model (OQ2).** Nothing in the SSH path needs a different architecture on Windows. The same thread-per-setup-job model, the same 20 ms sliced waits and the same tokio idle reactor are portable. Recommend reusing them; the choice is open only for the wait primitive inside step 1.2.

### 3.1 How option A, idle loss and the adaptive machine reach Windows

They are designed in by being OS-free above the closure, then proved on the Windows leg, not retrofitted:

- **Option A (`GwzTransportSshBackgroundCloseDesign.md`).** A fetch completes at libssh2's close, a closing exchange is discarded by terminating the socket (D5, §5), and the worker's 1 ms park keeps running during a close (`GwzTransportSshBackgroundCloseDesign.md:76`). The only OS touches are `SshConnection::terminate` (`ssh_connection.rs:44`, `shutdown(Both)`) and the `SshChannel::poll_dispose` change. Step 1.5 runs option A's tests on Windows, including "a discarded connection is gone within one pass". TR8.4 measures it.
- **Idle loss (`GwzTransportIdleLossDesign.md`).** `IdleReactor` and `IdleSocket` (`idle_watch.rs`) are Unix-only today only because `ssh_setup` is (§9, decision 6). Step 1.5 runs the idle tests under tokio's Windows driver, as §9 asks.
- **Adaptive concurrency (`GwzTransportAdaptiveConcurrencyDesign.md`).** The window sets, filter, machine and retry machine are pure state machines (L1, L2) and OS-free. Windows owes the L3-S SSH row on its own leg (§12 test 20) and the pinned error kind for a `MaxStartups` drop (F3). Phase 1's removal of the 64-caps, the per-host `Supervisor` and the local-wait clock are portable. Step 3.7 pins the Windows error kinds. Windows connect failures under 32 to 64 simultaneous connects (ephemeral ports, `TIME_WAIT`) are a TR8.4 observation, not a design change.

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
- **Review tiers** (`GwzProcessOptimization.md` §8; memory `review-granularity`). **Phase review** is the one Consistency plus Safety review at the phase's end, a skim review where the operator allows it. **Dual** is a per-step dual review, used only for secrets, wire format or the release gate. **Surface** is added where a phase freezes something a Windows user sees.
- **dabeest rules** (V110 §2; memory `dabeest-windows-builds`): `ssh gianni@dabeest` with `ClearAllForwardings=yes`, MinGW bash, work under `/e/gwz-tests/<label>`, Rust `+1.95.0` MSVC, no ancestor Cargo config, never change accounts, trust, services, proxies, zones or policy, install nothing globally. Section 7.2 lists which steps need the host, because it is shared.
- **Hot spots** (files several steps touch; the first step to touch each lands the structure, later steps fill their own file): `transport_host/mod.rs`, `transport_host/endpoint_environment.rs`, `transport_host/session.rs` (steps 1.6, 3.1, 3.2); `git/endpoint/mod.rs` lines 21 and 47-62 (1.1, 1.4, 1.5); `tests/transport_backend/prepare.py:87` and `Cargo.toml:107-108` (the `windows-sys` feature list: step 1.2 turns the list into one constant in `prepare.py`, so later steps add a line, not a conflicting edit). Memory `candidate-prepare-symlinks`: the candidate tree links `src` and `tests` into the real gwz-core, so copy with `cp -RL` before editing in one.

## 5. Phase 0: Windows CI and compile gate (TR4.6)

**Milestone.** Every push and pull request builds and tests the Windows shapes, and a lane cannot add a Unix-only dependency to the transport unseen. This is the prerequisite for everything below: without it each Windows step is judged by a manual dabeest run.

### Step 0.1: the `windows-2022` leg (TR4.6; in progress, another agent)

- **Goal.** Land TR4.6: on every push to main and every pull request, a `windows-2022` job builds gwz-core's and gwz-cli's ordinary builds, runs their ordinary suites, and runs the conditional-compilation check; after S4.5, it also builds the candidate and runs every candidate test that needs no fixture (`GwzTransportReleasePlanAmendment-2.md` §3.5).
- **Files.** `gwz-core/.github/workflows/transport-candidate.yml` (the leg), possibly `gwz-cli`'s workflow.
- **Acceptance this plan adds** (so the leg is useful to every later step): it builds three shapes (ordinary; `--cfg gwz_transport_candidate`; and that plus `gwz_windows_https_qualification`), as the 2026-10-08 check did; it sets `GWZ_TEST_GIT=real` as `windows-matrix.yml` does; it runs under bash, not PowerShell, so a failed command fails the step (`CurrentProgramCheckpoint.md`, "CI repaired after the push"); it records the three shapes' warning counts.
- **Tests first.** The existing workflow-text tests (`scripts/checks/test_check_candidate_switches.py`) gain a row that the Windows leg exists and names the three shapes.
- **dabeest.** None.
- **Review.** None beyond Phase 0's.

### Step 0.2: make the Windows leg green enough to require

- **Goal.** Clear the red tail the 2026-10-08 check found, and make `prepare.py` run on a hosted Windows runner.
- **Files.** `src/git/endpoint/placement_endpoint/retry_tests.rs:134` (an absolute home that is absolute on every platform, built from `std::env::temp_dir()`); `tests/transport_backend/prepare.py` (it creates symlinks with `symlink_to`, which needs privilege on Windows: add a copy mode used on Windows); the workflow from 0.1.
- **Tests first.** The 10 `retry_tests` fail on Windows before; `tests/transport_backend/test_prepare.py` gains a Windows-copy-mode row (runs on all platforms by injecting the mode).
- **dabeest.** One cold run of the three shapes to confirm the count: the 10 `retry_tests` and the 115 `v1_lifecycle` failures are gone with `GWZ_TEST_GIT=real`. About 15 minutes of host time; can be CI-only if the leg is green first.
- **Review.** Phase.

### Step 0.3: the Windows-parity inventory and its ratchet

- **Goal.** A shrink-only inventory of every Unix-only gate in the transport, so "Unix-only design stops piling up" is enforced, and so progress is countable.
- **Files.** new `scripts/checks/check_windows_parity.py`, `scripts/checks/windows_parity_inventory.json`, `scripts/checks/test_check_windows_parity.py`; one line in the lane gate (`scripts/checks/check_lane_commits.sh`) and in `run_tests.py`'s checks.
- **Design.** Syntax-aware, as `check_cfg_boundaries.py` is (it already inspects disabled platform arms). It records each `cfg(unix)`, `cfg(not(windows))`, `target_os` and `std::os::unix` / `libc::` use under `src/git/endpoint`, `src/transport_host` and `src/git/gitbackend/transport_*`, keyed by file, gate text and target (not line), each with an owner step from this plan. It fails on a new gate with no owner, and on an entry whose owner step is recorded done. Seed it with U1 to U26.
- **Tests first.** Planted-defect rows in the style of `test_check_process_globals.py`: a new `cfg(unix)` import with no owner fails; a `libc::` use inside a Windows arm fails; a removed gate with a stale entry fails.
- **dabeest.** None.
- **Review.** Phase.

### Step 0.4: the Windows compile gate on every lane

- **Goal.** A lane that touches the transport cannot merge without a Windows compile of the three shapes.
- **Files.** new `scripts/windows_lane_check.py` (a `git archive` of the lane head, copied to a new `/e/gwz-tests/<label>` on dabeest, `cargo check` of the three shapes, receipt archived); a paragraph in gwz-core `AGENTS.md` naming the paths that trigger it (`src/git/endpoint/`, `src/transport_host/`, `Cargo.toml`, `tests/transport_backend/prepare.py`). The runners are the 2026-10-08 check's `wcc_*` runners, generalized (that run's `runner/`).
- **Design.** Two layers, both cheap. The static layer is step 0.3 and runs per commit in the lane gate. The compile layer is the CI leg for anything pushed, and `windows_lane_check.py` for a lane that has not been pushed. Cross-compiling from the Mac is not used (V110 S4.1: "Do not compile Windows on the Mac").
- **Tests first.** `windows_lane_check.py`'s argument handling and its refusal to reuse a label are unit-tested without a host; the host run is the proof.
- **dabeest.** One timed `cargo check` of the three shapes, warm cache, to record the cost the lane owner pays per lane.
- **Review.** Phase.

**Phase 0 review.** One skim Consistency plus Safety review of 0.1 to 0.4 (they change no product code).

## 6. Phases 1 to 6

### Phase 1: portable SSH setup (S4.2, S4.4 in part, S4.5's SSH half)

**Milestone.** SSH setup, trust, URL-password and explicit-key authentication, option A's background close and the idle watch run on Windows through the same worker, pool and supervisor as Unix. Agents are not yet reachable (a missing agent is a refusal, as S4.3 says). Nothing in this phase fixes a Windows policy that TR1.8 has not frozen: it ports Unix behaviour, which the amendment's sketch allows before TR1.8's GO (S4.3's agent forms and TR4.8 to TR4.10 wait on it, `GwzTransportReleasePlanAmendment-2.md` §3.13 sketch). OQ1 asks the operator to confirm that reading.

#### Step 1.1: a Windows SSH test server

- **Goal.** `SshdFixture` on Windows: a loopback server on a high port with temporary host and client keys, a `known_hosts` that trusts only its key, and a bare repository, as the Unix fixture does (`ssh_fixture.rs:1-7`).
- **Files.** `src/git/endpoint/ssh_fixture.rs` (a Windows arm; the Unix arm uses `/usr/sbin/sshd`, `ssh-keygen`, `kill -STOP` and `ps`, `:73-74, 106-109, 256, 291, 306, 330`), a process-control helper for stop and kill (Job Object, as step 4.2's primitive; here a small test-only one), `git/endpoint/mod.rs:47-62`.
- **Tests first.** The fixture's own readiness test (the server answers a libssh2 handshake and serves `git-upload-pack` for the bare repo) fails on Windows before the arm exists.
- **dabeest.** A spike first (OQ6 picks the server). Existing baseline runs used a Paramiko server with `diffie-hellman-group14-sha1` and `strict_kex` off, and say it "is not universal KEX coverage" (`2026-10-03-tr1-8-windows/README.md`). Whether Windows' OpenSSH server is installed on dabeest is unknown (only the agent pipe was found absent, error 2), and the host rules forbid enabling a service.
- **Review.** Phase.

#### Step 1.2: portable socket readiness and connect (S4.2)

- **Goal.** Replace the `libc::poll` calls and the `EINPROGRESS` tests in `ssh_network.rs` with one portable readiness helper that has a Unix and a Windows arm, keeping `Control::wait_step`'s contract (a closure bounded by the slice, true when ready).
- **Files.** new `src/git/endpoint/socket_wait.rs` (about 150 lines: `wait_readable`, `wait_writable`, `connect_wait`); `ssh_network.rs` (U3 to U6; drop the module-level `cfg(unix)`); `Cargo.toml` and `prepare.py:87` (the `Win32_Networking_WinSock` feature, via the constant described in section 4).
- **Primitive.** Prefer `WSAPoll`, whose shape matches `poll`. WSAPoll reportedly does not report a failed non-blocking `connect` on older Windows 10 builds; `select` with an except set does. Decide by test: the failed-connect row below must pass on dabeest and on `windows-2022` with the primitive chosen. Unverified here; step 1.2 is where it is found out.
- **Tests first.** Portable rows over a loopback pair, on all three platforms: readable after a write, writable, EOF, timeout within the slice, **connect to a closed port reports failure inside the `Control` bound**, connect to a listener succeeds, a cancelled `Control` ends a wait inside one slice. The Unix-only `ssh_tests/network.rs` rows that need no `sshd` move out of `cfg(unix)`.
- **dabeest.** The failed-connect row on the host (Windows 11 build 10.0.26200) and the result on `windows-2022` recorded side by side; both bounds.
- **Review.** Phase.

#### Step 1.3: opening regular files without blocking on special files

- **Goal.** One reader that opens a `known_hosts`, key or identity file, refuses anything that is not a regular file, and cannot block on a pipe or device, with Unix and Windows arms.
- **Files.** new `src/git/endpoint/regular_file.rs` (about 200 lines); callers `ssh_network.rs:133-157` (`read_regular`), `ssh_key_snapshot.rs:152-172`, `ssh_worker/endpoint.rs:135-157`. The Windows arm refuses `\\.\...` and `\\?\...` device paths, reserved names (`CON`, `NUL`, `COMn`) and, after opening, anything but `FILE_TYPE_DISK`. It is the same module step 4.3 uses for `https_auth/file_worker.rs:68`, so write it once.
- **Tests first.** Cross-platform: a regular file reads; a directory refuses; a file over the cap refuses; paths with spaces and non-ASCII characters read. Unix: a FIFO refuses without blocking (the existing rows). Windows: `CON`, `NUL`, `\\.\pipe\x` with no server, and a path to a pipe created by the test refuse without blocking.
- **dabeest.** The named-pipe and device rows, which hosted runners can also run; record both. WH1's design (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §5) wants "reject pipe/device/unsupported path classes before blocking opens" proved, not assumed.
- **Review.** Phase.

#### Step 1.4: ungate the setup chain

- **Goal.** Compile and run `ssh_key_auth`, `ssh_password`, `ssh_setup`, `ssh_local` and `agent_job::start_setup` on Windows with no agent source, by removing their `cfg(unix)` (U1, U12, U13).
- **Files.** `git/endpoint/mod.rs:21`; `ssh_key_auth.rs:19`; `ssh_local.rs:4`; `agent_job.rs:194`; `ssh_password_helpers.rs:30` (Windows arm returns the existing `Unsupported`, with a message naming WH2, until 4.5). `agent_auth` and `agent_socket` stay Unix until 3.3 and 3.4; `ssh_local` takes `agent: None` on Windows, and an open that needs an agent is refused with the existing "no agent" refusal (S4.3).
- **Tests first.** The existing `ssh_tests` rows for explicit key, URL password, trust refusal, `known_hosts` case folding (TR2.18) and setup cancellation, un-gated for Windows and failing on a Windows build before this step because the modules do not exist there.
- **dabeest.** Explicit-key and URL-password fetch against the step 1.1 server; `known_hosts` mismatch refusal before any key offer.
- **Review.** Phase.

#### Step 1.5: option A and the idle watch on Windows

- **Goal.** Run the background-close and idle-loss behaviour on Windows, and fix whatever tokio's Windows driver does differently.
- **Files.** `idle_watch.rs` (expected: no change; possible: how a `try_clone`d socket is registered), `ssh_connection.rs` (`terminate`, `watch_socket`), `ssh_channel.rs` (`poll_dispose`), `ssh_setup.rs:401-417`; test modules `ssh_tests/idle_loss.rs`, `idle_loss_budget.rs`, `pooled.rs` and `ssh_worker` tests un-gated.
- **Tests first.** The idle-loss rows: the server closes an idle connection and the host reports `idle_closed` without a lease; a reused dead connection is replaced by one fresh retry; a reset is lost as EOF is; a registration failure marks the connection lost (`GwzTransportIdleLossDesign.md` §5.1, §8). Option A's rows: a fetch completes at libssh2's close; a push waits for the client's EOF to reach the server; a discarded connection is gone within one pass; shutdown and discard terminate a closing exchange (`GwzTransportSshBackgroundCloseDesign.md` §5, §10). Only the option A rows that are in the tree when this step runs apply; if option A has not landed, this step covers idle loss and adds a line to option A's lane owner (it is in lane `gwz-dev-bgclose` today, `CurrentProgramCheckpoint.md:7`).
- **dabeest.** The idle and close rows, plus a socket-count observation (one connection per member, the connection dies within a pass) with `netstat` on the host; this is the Windows half of TR8.1's "physical connection counts recorded".
- **Review.** Phase.

#### Step 1.6: SSH through the Windows qualification boundary (S4.5, SSH half)

- **Goal.** Admit `Scheme::Ssh` on Windows in the candidate: construct the SSH engine, list `Ssh` in the endpoint's schemes, stop refusing SSH-only runtimes, and give `SshSettings` a Windows `home`. The qualification cfg keeps its current name until step 5.1 retires it (renaming it now would touch 41 sites for a switch with weeks to live; OQ15).
- **Files.** `transport_host/mod.rs:87-88, 138-141, 188-190, 222-224, 280-282`; `transport_host/session.rs:417-440, 459-462`; `endpoint_environment.rs:59-64` (Windows arm builds `SshSettings` with `HOME` absolute as an **interim** that step 3.1 replaces; the inventory marks it); `gitbackend/transport_binding.rs:227` and `transport_support.rs`, `transport_observations.rs` sites that gate SSH.
- **Tests first.** `transport_host/qualification_tests.rs` gains rows: SSH is offered on Windows in the candidate; an SSH-only runtime builds; the capability projection lists `Ssh`. Existing rows asserting SSH refusal flip.
- **dabeest.** gwz-cli (built as in the 2026-10-08 check) clones, fetches and pushes over SSH with an explicit key against the 1.1 server, `--max-per-host 1`, with the `--verbose` row asserting the transport route.
- **Review.** Phase.

**Phase 1 review.** One skim Consistency plus Safety review of 1.1 to 1.6.

### Phase 2: TR1.8 design freeze (design and evidence; no product code)

**Milestone.** TR1.8 has GO. This is the critical path: steps 3.1, 3.3 and 3.5 onward, and all of Phase 4's policy choices, consume it.

#### Step 2.1: refresh TR1.8 against MAIN

- **Goal.** One revision of `GwzTransportWindowsParityDesign.md` that carries the accepted helper-timing, configuration-view and SSH-clock amendments, the SSPI-only supersession list folded into the text, the `AgentSource` enum, and every disposition in `GwzTransportWindowsProofDispositions-DRAFT.md`, with the changes since 2026-10-03 reconciled: option A (§3.1), idle loss, the adaptive Phase 1, the per-host `Supervisor`, WH1's accepted shape (the qualification cfg, `Owner::send_if`).
- **Files.** `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`, `...Baseline.md`, `...UserGuide-DRAFT.md`, `...Checkpoint.md`.
- **Tests first.** n/a (document). Its check: every clause cites a baseline row or marks itself provisional.
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
  - **X9** an agent that exists but holds no key, with Pageant visible and a working pipe agent behind it (libssh2 does not fall through, `agent.c:343, 816-826`).
- **Files.** `gwz-core-evidence/campaigns/transport-qualification/runs/<new label>/`; runner scripts reuse the `ssh_baseline_v5.py` shape.
- **Tests first.** The runner's own expectations are written down before each run and recorded as predictions, as the baseline's HOME rows were ("the first runner's prediction that empty HOME would authenticate failed and is retained").
- **dabeest.** All of it. About 1 day of host time. Needs OQ6 (server), OQ9 (service, only if the native pipe row is wanted).
- **Review.** None (evidence); step 2.4 reviews it.

#### Step 2.3: the remaining HTTPS and authentication rows on 1.0.17

- **Goal.** Execute B11 to B16 and B18, P05 (provider blocked inside a call), P07 (Digest with a provider-accepted identity), P01 (cross-SID) and P03 (native service identity) to the extent OQ9 allows.
- **Files.** evidence run as above.
- **Dependency.** These need the approvals the baseline lists: a one-certificate Windows trust transaction (the guarded proposal exists, `NATIVE_TRUST_TRANSACTION_v2/v3.md` in the 2026-10-03 run), a distinct account for helper-identity and Digest rows, and a coordinated service for the native OpenSSH agent. Any of the three may be denied. Rows that cannot run are recorded **unexecuted**, and the design marks the clause provisional or removes the claim; S5.6 forbids advertising a cell without evidence.
- **Tests first.** As step 2.2.
- **dabeest.** About 1 to 2 days, in the proxy-serialized form the baseline §5 requires (exact prior proxy state saved, restoration guard armed before each mutation).
- **Review.** None (evidence).

#### Step 2.4: settle, review, GO

- **Goal.** Root-settled tuple, the canonical dual Consistency and Safety review of the revision from 2.1 with the evidence from 2.2 and 2.3, and Surface on the user guide alone (`GwzTransportWindowsBaseline.md` §7). At most two architectural remediation rounds.
- **Files.** the reports beside the design: `GwzTransportWindowsParity-ReviewConsistency.md`, `-ReviewSafety.md`, `-ReviewSurface.md`; the verdict.
- **dabeest.** None.
- **Review.** **Dual** (TR1.8's own: it designs Pageant's window and shared memory, the machine proxy's bypass rule, and the default-credential exchange; amendment 2 §3.5), plus Surface.

### Phase 3: Windows SSH parity (agents, home, algorithms, errors)

**Milestone.** On Windows, SSH authenticates through a visible Pageant first, else the OpenSSH agent pipe `SSH_AUTH_SOCK` names or the default pipe, resolves its home and `known_hosts` as libgit2 does, signs with the key types libssh2 on Windows can handle, and reports a dropped connection the way the retry machine expects. TR4.8 and S4.3 are inside it.

#### Step 3.1: the SSH home resolver (S4.4; TR1.8 §3)

- **Goal.** Resolve the SSH home on Windows as libgit2 does (`HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE`; first existing directory) from the captured environment, with TR1.8's accepted dispositions for empty, relative, Unicode and spaces (OQ7). Replace step 1.6's interim.
- **Files.** new `transport_host/ssh_home.rs` (about 200 lines, a pure function of the snapshot and a filesystem probe port); `endpoint_environment.rs:177-200` (the Windows `platform::ssh_home`); `transport_host/mod.rs:85-100` (`SshEndpointConfig::from_environment`, which must use the same resolver or be retired, TR1.8 §3); `known_hosts` and `~/` identity resolution.
- **Tests first.** A table of B03/B04 rows reproduced as pure tests (candidates in order, skip empty, existence probe, no fallthrough to another home's trust file when the first has no good `known_hosts`, relative resolved against the captured cwd, UTF-16 preserved), then the same rows through the fixture.
- **dabeest.** B03 and B04 against the transport, side by side with 1.0.17; one row with `HOME` unset (TR8.4 asks for it).
- **Review.** Phase.

#### Step 3.2: the `AgentSource` seam

- **Goal.** Replace `agent: Option<PathBuf>` by an owned enum chosen once per runtime, before any connection: `Pageant`, a named pipe, a Unix socket path, or none. On Unix nothing changes.
- **Files.** `transport_host/mod.rs:138-150` (`SshSettings`, `SshEndpointConfig`); `endpoint_environment.rs:177-200`; `ssh_local.rs` (`connect_with_helpers`'s `agent_socket` parameter); `ssh_setup.rs` (where the agent path is used); `agent_socket.rs`.
- **Tests first.** Pure selection tests with an injected "Pageant window visible" probe and a snapshot: Pageant beats the snapshot's pipe; the pipe beats the default; the default is `\\.\pipe\openssh-ssh-agent`; a MinGW socket path and a UNC pipe path are refused before connect, naming `SSH_AUTH_SOCK` (TR1.8 §4); the Unix selection rows pass unchanged.
- **dabeest.** None (pure logic). The probe's real implementation is step 3.6.
- **Review.** Phase.

#### Step 3.3: the OpenSSH agent pipe (S4.3)

- **Goal.** `agent_pipe.rs`: a `Channel` over a local named pipe, overlapped, one request owner per handle, bounded by `Control`.
- **Files.** new `src/git/endpoint/agent_pipe.rs` (about 350 lines); `agent_client.rs` (no change expected); `agent_socket.rs` stays Unix.
- **Design points** (TR1.8 §4; `agent_win.c:124-139` is the 1.0.17 behaviour to match): connect with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`; `ERROR_PIPE_BUSY` waits inside `Control`, not libssh2's fixed 1 s; local pipe names only (reject UNC, traversal); `CancelIoEx` on the owner's `OVERLAPPED`, and the buffer, `OVERLAPPED` and event stay owned until completion is observed (P03 executed); the server-identity rule is OQ4's.
- **Tests first.** Agent-codec tests over an in-process named-pipe agent fixture (the Windows twin of `ssh_tests/agent_fixture.rs`): identities, a sign, a partial reply delivered 1 byte then 2 (P03), a cancel mid-read that completes before the storage is released, a wrong-name refusal, a server that never answers (stall bound), a pipe that vanishes mid-request poisons the channel.
- **dabeest.** B08 and B06 with the owned pipe fixture against the transport and against 1.0.17; the native service row only under OQ9.
- **Review.** Phase.

#### Step 3.4: `agent_auth` on Windows, and the allocator question

- **Goal.** Ungate `agent_auth` for Windows, and prove the `libc::malloc` that hands libssh2 its signature buffer is the allocator libssh2 frees with (U11).
- **Files.** `agent_auth.rs:4, 213-220, 275-277`.
- **Tests first.** A sign through the pipe fixture on Windows completes, and a debug-CRT run (MSVC debug heap checks) of one thousand signs shows no heap mismatch or leak. If `libc::malloc` and libssh2 differ (for instance a static-CRT build), the step's fallback is to allocate through the session's allocator via a small accessor, not to copy a Rust `Vec` into native ownership (the existing comment forbids that).
- **dabeest.** The debug-CRT run, and a `+crt-static` release build, since cargo-dist may build Windows that way (check `gwz-cli`'s dist config as the first task).
- **Review.** Phase.

#### Step 3.5: Pageant's exchange primitive (TR4.8, part 1)

- **Goal.** `pageant_exchange.rs`: given a window handle, send one agent message and read one reply through a mapping, with TR1.8 §5's rules: `WM_COPYDATA` with `dwData 0x804e50ba`, an 8192-byte `Local\` mapping with a unique name from the runtime's `IdSource`, exclusive creation, an ACL that grants the caller and SYSTEM only, payload at most 8188 bytes, reply length checked 1..8188, one `SendMessageTimeoutW` with `SMTO_BLOCK | SMTO_ERRORONEXIT`, no resend, no read after a timeout, no name reuse.
- **Files.** new `src/git/endpoint/pageant_exchange.rs` (about 350 lines); Cargo features `Win32_UI_WindowsAndMessaging`, `Win32_System_Memory`, `Win32_Security_Authorization` (candidate extras).
- **Tests first.** Against a synthetic receiver (a test window class on a thread): a round trip; a timeout; the late write after timeout is never read; collision on the mapping name refuses without writing (error 183); an oversized request refuses before dispatch; a reply with a bad length or a bad message type ends the channel; same-queue window refused. Against actual Pageant 0.83: the P01/P02 rows the baseline marks executed, re-run through the product code.
- **dabeest.** Pageant 0.83 (pinned, official SHA-256 manifest `putty-0.83-sha256sums.txt`), disposable key; the encrypted-key prompt row (the 2037 ms timeout, mapping alive until reap).
- **Review.** Phase.

#### Step 3.6: Pageant as the session's agent source (TR4.8, part 2)

- **Goal.** `agent_pageant.rs`: the `Channel` adapter over 3.5 (a written frame is buffered; the first read performs the exchange and serves the reply bytes), the real window probe for 3.2 (`FindWindowW("Pageant","Pageant")`; "visible" means discoverable by this protocol, not `IsWindowVisible`, TR1.8 §4), the window identity pin (HWND, process handle, creation identity, SID, logon `AuthenticationId`; checked before each request; a vanished or replaced window refuses and never selects the pipe), and the refusal texts: "Pageant has no key this transport can use", a timeout naming the bound and the confirmation or key action.
- **Files.** new `src/git/endpoint/agent_pageant.rs` (about 300 lines); `ssh_local.rs` (open the channel for the selected source).
- **Tests first.** Selection and failure rows over the synthetic receiver: Pageant visible with keys signs, and the pipe fixture records **no** request (B06); Pageant visible with no keys refuses and the pipe is not tried (X9); a replaced owner refuses; the confirmation timeout error names no fallback; neither agent present refuses before any connection opens (B07).
- **dabeest.** B05, B06, B07, X9 and X4's Pageant column with the transport, side by side with 1.0.17.
- **Review.** Phase. OQ3 and OQ5 apply.

#### Step 3.7: Windows error kinds and the retry machine

- **Goal.** Pin what Windows reports for a `MaxStartups` drop, a refused connect, a reset and an abort, and map a drop to Suspect, as the adaptive design requires where a platform reports it as `ConnectionAborted` (`GwzTransportAdaptiveConcurrencyDesign.md` F3, test 20).
- **Files.** `ssh_setup.rs:437` (the `ConnectionAborted` mapping), `ssh_tests/max_startups.rs` (un-gated, with a Windows branch like its macOS one at `:107`), `setup_retry/` classification tests.
- **Tests first.** The L3-S row for `MaxStartups` on the Windows leg: a server that drops the Nth unauthenticated connection yields a retried setup, not a failed member; a unit row for each Windows error kind (the Windows-numbered ones go in a table keyed by `raw_os_error`).
- **dabeest.** X7 on 1.0.17 first (what it reports), then the transport. If the step 1.1 server has no `MaxStartups`, the row uses a TCP shim that resets the Nth connection.
- **Review.** Phase.

#### Step 3.8: host-key preferences and the key-type matrix under WinCNG

- **Goal.** Make `preferences()` and `HOSTKEYS` (`ssh_network.rs:24-30, 346`) safe on a libssh2 that lacks ed25519, and run TR2.8's matrix on Windows against what X2 to X4 found 1.0.17 doing.
- **Files.** `ssh_network.rs` (the preference list filtered by what the session supports, or a refusal that names the algorithm), `ssh_tests/key_types.rs`, `rsa_sha1.rs`, `key_files.rs`, `selected_key.rs`, `host_case.rs` (un-gated).
- **Tests first.** An ed25519-only `known_hosts` entry yields the outcome X2 recorded for 1.0.17 (this step is blocked on that row); mixed entries select the supported algorithm; each key type outcome equals 1.0.17's.
- **dabeest.** The matrix side by side with 1.0.17.
- **Review.** Phase.

**Phase 3 review.** One Consistency plus Safety review of 3.1 to 3.8, with Pageant's window and shared memory and the pipe's server identity on the Safety finding list. Surface covers the Windows messages for Pageant and agents (S7.5 (1.1.0) names "TR1.8's Windows messages for Pageant and the machine proxy").

### Phase 4: HTTPS parity residuals (WH2, TR4.9, TR4.10, WH3)

**Milestone.** On Windows, HTTPS does what 1.0.17 does: configured credential helpers, the WinHTTP machine proxy, helper-identity and default-credential authentication, and the integrated adversity and installed-path rows are qualified. Phase 4 does not wait on Phases 1 and 3; it waits on Phase 2 for its policy choices only.

#### Step 4.1: the WH2 contract

- **Goal.** The WH2 spikes and accepted contract (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §3 "WH2", §5): Job attached at process creation before the child runs, allowlisted inherited pipe handles, descendant survival, cancellation and reaping with capacity retained, Git-for-Windows origin and UTF-8 versus UTF-16 rules, quoting, the null config source, the executable extension rule, regular-file refusal, non-ASCII paths and case-varied environment keys.
- **Files.** a WH2 design and baseline section; evidence run.
- **Tests first.** n/a (design); the spikes are its rows. P08 (helper Job Object primitive) is already executed, so this is composition.
- **dabeest.** The helper spikes, with Git for Windows' `git.exe` and a fake helper that hangs, spawns a descendant, or writes without reading.
- **Review.** Dual Consistency plus Safety (the contract handles credentials).

#### Step 4.2: the helper process owner

- **Goal.** Replace `killpg` and `process_group(0)` with a Job Object owner: start suspended, assign to a kill-on-close Job before resume, no breakaway, kill and wait for the whole job on timeout or cancel, pipe buffers owned until I/O and reaping complete.
- **Files.** `https_auth/owner.rs:245-254`, `https_auth/lookup.rs:165-170`, `https_auth/runner.rs`; Cargo feature `Win32_System_JobObjects`.
- **Tests first.** The Unix process-group rows, generalized: the helper's descendant dies on cancel; capacity is retained until reaping; a drop mid-I/O does not free a buffer a pending read still uses. Windows variants: direct, spaces in the path, shell, GUI, nested Job (P08's four shapes).
- **dabeest.** The P08 shapes through product code.
- **Review.** Phase.

#### Step 4.3: helper discovery, environment and paths, framing

- **Goal.** `git.exe` found on the captured absolute `PATH` (no implicit cwd, no shell-script extension); the environment and arguments passed as UTF-16 without lossy conversion; the helper protocol framing and the config "view" carried as wide paths; the U7 reader for the config files.
- **Files.** `https_auth/executable.rs:30-35`, `runner.rs:3`, `view.rs:3`, `view/framing.rs:4`, `file_worker.rs:5, 68`, `endpoint_environment.rs:75-83` (the Windows `auth` config), `helper_script.rs:23, 78` (Windows fakes).
- **Tests first.** Non-ASCII and spaced paths and case-varied keys survive a round trip into the helper; a path that cannot convert refuses with `ConfigurationRefused` and never substitutes a replacement character; a missing `git` reports the setting that names it (TR1.8 §10).
- **dabeest.** The WH2 path rows with Git for Windows.
- **Review.** **Dual** if the step carries credential bytes, otherwise Phase. It sits next to 4.4; the lane owner may merge their reviews.

#### Step 4.4: admit `WindowsConfigured` and `Gh`, and the precedence rule

- **Goal.** Offer the configured-helper policies on Windows, with TR1.8 §7's rules: ask the helper once for the URL the credential goes to, after a redirect; pick the highest offered scheme (`Negotiate`, `NTLM`, `Digest`, `Basic`); a `Negotiate`-only challenge asks no helper; no helper identity falls to the logon session; a helper timeout or cancel ends the request; a rejected helper identity is never replaced by another.
- **Files.** `https_worker.rs:60, 98, 113, 118, 135, 245, 252`, `https_worker/credentials.rs:19`, `https_auth.rs:10, 110, 128`, `transport_host/session/driver/opening.rs`, `https_worker/prepare.rs`, `gitbackend/transport_binding.rs:227` capability projection, the WH1 refusals that keep these policies off.
- **Tests first.** The B11 to B13 matrix as fixtures: Negotiate-only with a helper credential present (helper not asked), NTLM-only with and without a helper, `Negotiate, Basic` with a fake `gh` then a non-gh helper (helper identity over `Negotiate`, no default-credential offer). A forged `WindowsConfigured` Open still refuses where the policy is not offered.
- **dabeest.** The same matrix over loopback HTTPS (needs OQ9's trust approval for 1.0.17 side-by-side).
- **Review.** **Dual** (secrets).

#### Step 4.5: SSH password-only servers with a helper

- **Goal.** `ssh_password_helpers::lookup` on Windows (U14), so a server that lists no `publickey` uses the configured helper, as the accepted ambient password-only route does (`ssh_password.rs:15-17`).
- **Files.** `ssh_password_helpers.rs:30, 142`.
- **Tests first.** The Unix `ssh_tests/password_helpers.rs` rows on Windows; X6 expected result.
- **dabeest.** X6 side by side with 1.0.17.
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
- **Review.** **Dual** (TR4.10's own review; secrets). Safety's list names the forced-authentication hazard OD16 accepted, and SSPI secret ownership.

#### Step 4.9: WH3, integrated adversity and the HTTPS rows

- **Goal.** The WH3 matrix (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` §6) at product level, on the final route: original deadline D across reuse, 401 and native rounds; cancel during HTTP and native IPC; cleanup retained before replacement admission; pool reuse within an opaque scope and no cross-operation credential reuse; receive-pack POST failure never retried; EPA matched and mismatched (B16); a discovery redirect between names (B15); the Windows POST challenge (B18).
- **Files.** tests under `src/transport_host/` and `src/git/endpoint/https_worker_tests/` un-gated for Windows; fixtures in `src/` test modules.
- **Tests first.** Each case is a row; the cases that already pass on the WH1 route (the 2026-10-08 check) are kept as regression rows.
- **dabeest.** The full set, in the form the 2026-10-08 check ran.
- **Review.** Phase.

#### Step 4.10: WH3, installed paths and worker provenance

- **Goal.** The installed CLI and the Python wheel in a path with spaces and a non-ASCII path, the worker's provenance refusal and its missing and mismatched cases, and wheel concurrency.
- **Files.** gwz-cli and gwz-py install rows; `gwz-sspi/docs/HostPackaging.md` rows.
- **Tests first.** Packaged-artifact rows (the worker found at its packaged path, no `PATH` discovery for SSPI).
- **dabeest.** Both artifacts installed under `E:/gwz-tests/<label>/with spaces/...` and a Unicode directory.
- **Review.** Phase.

**Phase 4 review.** One Consistency plus Safety review of 4.1 to 4.10 (4.4 and 4.8 already had their dual reviews; this one reads their composition). It is TR4.7's input.

### Phase 5: Windows activation, gwz-sspi publication, the Windows review

**Milestone.** The Windows build ships the transport on SSH and HTTPS with no qualification switch, gwz-sspi 0.1.0 is on crates.io, and TR4.7 has GO.

#### Step 5.1: retire the qualification switch in gwz-core (S4.5)

- **Goal.** Turn every `all(windows, gwz_transport_candidate, gwz_windows_https_qualification)` site (41 in `src/`) into the plain `cfg_if` unix or windows arm under `gwz_transport_candidate` that S4.5 specifies (`GwzV110Plan.md` S4.5), and delete the refusals that WH1 added (SSH-only runtime unavailable, HTTPS-only schemes, verified-DIRECT only).
- **Files.** gwz-core `src/lib.rs`; `src/transport_host/{mod,session,endpoint_environment,qualification_tests}.rs` and `session/driver/opening.rs`; `src/git/{mod,gitbackend}.rs` and `gitbackend/{backend,transport_support,transport_observations,transport_binding}.rs`, `transport_support/identity.rs`; `src/git/endpoint/https_worker.rs` and `https_worker/prepare.rs`; `build.rs`; `tests/transport_backend/test_windows_qualification_boundary.py` (17 files).
- **Tests first.** `test_windows_qualification_boundary.py` flips from "the boundary exists" to "the cfg name is gone"; the Windows leg builds only the two remaining shapes.
- **dabeest.** The 2026-10-08 rows plus SSH.
- **Review.** Phase.

#### Step 5.2: gwz-cli

- **Goal.** gwz-cli's seven files (`build.rs`, `src/globalargs.rs`, `src/lib.rs`, `src/globalargs/{dispatch,transport,transport_help}.rs`, `src/tests/g09.rs`) drop the switch; help and `--verbose` rows describe Windows SSH, Pageant and the machine proxy.
- **Tests first.** The existing help-text and `--verbose` tests gain Windows rows.
- **dabeest.** The installed CLI runs one SSH and one HTTPS command and shows the transport route.
- **Review.** Phase.

#### Step 5.3: gwz-py

- **Goal.** gwz-py's route and client-host sites (`native/src/{route,client_host,lib}.rs`, `dispatch/{mod,merge}.rs`, `Cargo.toml`) drop the switch, and S6.3's dabeest rows (which "wait on S4.5") run.
- **Tests first.** The S6.3 rows for the Windows wheel: an SSH and an HTTPS operation and two overlapping operations, each asserting the transport route.
- **dabeest.** The wheel, built as the 2026-10-08 check did (`CARGO_INCREMENTAL=0`).
- **Review.** Phase.

#### Step 5.4: the candidate leg on Windows CI (TR4.6, second part)

- **Goal.** After 5.1, the Windows leg builds the candidate and runs every candidate test that needs no fixture, and the SSH fixture rows that 1.1 made runnable on a hosted runner run too if the server is installable there (OQ6).
- **Files.** the workflow; `scripts/run_tests.py` Windows mode if needed.
- **Tests first.** The workflow-text test from 0.1 gains the candidate-on-Windows row.
- **dabeest.** None.
- **Review.** Phase.

#### Step 5.5: gwz-sspi activation and 0.1.0 (release step 5a)

- **Goal.** Lift `publish = false` in a reviewed activation change, confirm the trusted publisher's environment (O1), publish `0.1.0` through `release.yml`, and unblock gwz-cli and gwz-py's pins (O2). This is amendment 2 §3.21 part C's step 5a, scheduled here so it is not found at release time.
- **Files.** `gwz-sspi/Cargo.toml`, `scripts/release_checks.py`, `RELEASE.md`, `README.md`; gwz-cli and gwz-py CI checkouts of gwz-sspi (the checkpoint lists them as open follow-ups).
- **Tests first.** `release_checks.py`'s unit tests for the lifted guard (the checks refuse while `publish = false`, accept after).
- **dabeest.** gwz-sspi's native opt-in fixtures (`tests/native/`) on the final tuple, plus the packaged worker qualification (its `RELEASE.md` says "qualify the packaged library and Windows worker").
- **Review.** **Dual** (release gate; OQ14).

#### Step 5.6: TR4.7, the Windows implementation review

- **Goal.** The plan's Phase 4 exit: a dual peer-blind Code and State review on the settled tree after S4.2 to S4.5, TR4.6, TR4.8, TR4.9 and TR4.10 with its own GO (`GwzTransportReleasePlanAmendment-2.md` §3.5).
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

- **Goal.** One row per behaviour TR1.8 designs, each running on the transport and succeeding as on 1.0.17: Pageant, the OpenSSH agent pipe, the machine proxy, default credentials (an Internet-zone `NTLM` challenge, OD16), a helper identity; at least one row with `HOME` unset; the native-route row with the off switch on, at 16 and 32 members, three rounds, no partial result.
- **Files.** evidence run.
- **dabeest.** The rows; proxy rows use the serialized protocol.
- **Review.** None (evidence).

#### Step 6.3: S5.5 and the Windows cells of S5.6

- **Goal.** Repeat S5.5's rows on dabeest after S5.4 (the defaults), and fill the Windows column of S5.6's table with an evidence ID or an explicit unsupported mark. A cell the release's scope puts "in this release" and marks unsupported blocks S5.6 unless an accepted amendment removes it (`GwzV110Plan.md` S5.6); OQ10 names the likely ones (Digest, Kerberos).
- **Files.** the S5.6 table (evidence document).
- **dabeest.** S5.5's repeat.
- **Review.** Phase.

#### Step 6.4: S7.3 and S7.5 on Windows, notes

- **Goal.** The Windows rows of S7.3 (1.1.0): one operation per behaviour TR1.8 designs asserting the transport route, one Internet-zone `NTLM` challenge asserting that the transport authenticates (OD16), one SSH operation with `HOME` unset; the Windows messages Surface reviews (Pageant, the machine proxy); migration notes that state the inverse proxy precedence, the OD16 hazard, WinCNG's missing ed25519, the unsupported cells.
- **Files.** gwz-cli docs pages, help text, `docs/` migration notes; S7.3's tests.
- **dabeest.** The rows.
- **Review.** Surface (S7.5).

#### Step 6.5: the release preconditions

- **Goal.** Before step 5's gwz-core tag, `windows-matrix.yml` is dispatched on the release commit and passes, and the checkpoint records the run (amendment 2 §3.12); `gwz-sspi 0.1.0` is visible on crates.io before step 6 (§3.21); after release, on dabeest, one behaviour TR1.8 designs on the transport and one SSH command with `HOME` unset (§3.12 post-release check).
- **Files.** the checkpoint; the release scripts' checks.
- **dabeest.** The post-release rows on the installed 1.1.0.
- **Review.** **Dual** (release gate; the Phase 10 review).

## 7. Dependencies, parallel lanes and host load

### 7.1 Sketch

```text
Phase 0   0.1 (in progress) ── 0.2 ── (everything else's Windows leg)
          0.3, 0.4 independent
Phase 1   1.1 ┐
          1.2 ├── 1.4 ── 1.5 ── 1.6
          1.3 ┘
Phase 2   2.1 ┐
          2.2 ├── 2.4  (TR1.8 GO)        2.2 can start now (1.0.17 + owned fixtures)
          2.3 ┘                          2.3 waits on OQ9
Phase 3   1.4 ── 3.2;  1.2 ── 3.4;  2.4 ── 3.1, 3.3, 3.5;  3.2 + 3.3 + 3.4 + 3.5 ── 3.6
          1.6 + 1.1 ── 3.7;  2.2 + 1.6 ── 3.8
Phase 4   4.1 ── 4.2, 4.3 ── 4.4 (also 2.4) ── 4.5 (also 1.4, 1.1);  2.4 ── 4.6 ── 4.7;  2.4 + 4.4 ── 4.8;  4.4 + 4.7 + 4.8 ── 4.9 ── 4.10
Phase 5   3.* + 4.* ── 5.1 ── 5.2, 5.3, 5.4;  5.5 any time after 4.8;  5.1..5.5 ── 5.6
Phase 6   5.1 + TR2.1/2.9/2.10 + option A ── 6.1, 6.2;  6.1 ── 6.3 ── 6.4 ── 6.5;  5.5, 5.6 ── 6.5
```

Steps that can start today, in parallel: 0.2, 0.3, 0.4, 1.1, 1.2, 1.3, 2.1, 2.2, 4.1. That is nine independent lanes; dabeest can serve about two of them at a time (section 7.2). 3.4 joins as soon as 1.2 lands.

### 7.2 What needs dabeest

| Step | Host time | Notes |
|---|---|---|
| 0.2, 0.4 | 15 to 30 min each | CI can substitute once the leg is green |
| 1.1 | spike, about half a day | OQ6 |
| 1.2, 1.3 | short | also run on `windows-2022` |
| 1.4, 1.5, 1.6 | 1 to 2 h each | after 1.1 |
| 2.2, 2.3 | 1 to 2 days each | serialized proxy and trust transactions; the biggest user |
| 3.1 to 3.8 | short, per step | Pageant rows need a console session with Pageant |
| 4.1 to 4.10 | 4.6, 4.7, 4.8 need the serialized protocol | |
| 5.x, 6.x | 6.1 about a day, host otherwise idle | do not overlap another agent's build |

Hosted `windows-2022` can run: 1.2, 1.3 (device paths), 3.2, 3.3 (pipe fixture), 3.4, 3.5 (synthetic receiver only, and only if the hosted session can create windows), 4.2, 4.3, 4.6 grammar, 5.4. It cannot run: Pageant 0.83 (needs the pinned release and a desktop session), the machine proxy (shared state), the logon session's SSPI rows, and anything timed.

## 8. Open questions for the operator

Each gives 1.0.17's behaviour on Windows where known, how to find out where not, the options, and a recommendation.

**OQ1. May Phase 1 start before TR1.8 has GO?**
- 1.0.17: not applicable (a scheduling question).
- Context: TR1.8's header says no downstream implementation may consume it as GO. Amendment 2 §3.13's sketch gates only S4.3's agent forms and TR4.8 to TR4.10 on TR1.8. Phase 1 ports Unix behaviour and fixes no Windows policy.
- Options: (a) start Phase 1 now; (b) wait for 2.4.
- **Recommend (a).** The schedule is the point of this plan, and Phase 1 is a prerequisite of every Phase 3 test. The steps' Windows-specific choices (the wait primitive, the file reader) are recorded in the inventory, not in policy.

**OQ2. Does Windows SSH reuse the Unix worker architecture or need a different readiness model?**
- 1.0.17: libssh2 blocks or polls on its own; the comparison is behavioural.
- Options: (a) reuse the thread-per-job model, 20 ms sliced waits and the tokio idle reactor, changing only the wait primitive; (b) overlapped I/O or IOCP for sockets and a different supervisor.
- **Recommend (a).** Section 3: every OS call is inside a closure, the pool and worker are OS-free, and (b) would fork the architecture that the adaptive and idle-loss designs rely on. Revisit only if TR8.4 shows a latency the slice wait causes. Find out: TR8.4's connect-latency trace on dabeest.

**OQ3. Are both the OpenSSH-for-Windows agent and Pageant required for 1.1.0 parity?**
- 1.0.17: both. libssh2 tries Pageant, then the OpenSSH pipe (`agent.c:436-441`, `agent_win.c:124-139`). B05 shows Pageant authenticating; B08 (the pipe) has not run against 1.0.17 with a real service.
- Options: (a) both, as OD15 reads literally; (b) the pipe in 1.1.0 and Pageant refused with a message naming the off switch; (c) hold 1.1.0 for Pageant.
- **Recommend (a)**, ordered pipe first (step 3.3 is smaller and has no shared-memory risk), Pageant second. If Pageant (3.5, 3.6) slips because TR1.8 needs a broker (OQ5), the operator chooses between (b) and (c) then; (b) is a native-route-flavoured exception that OD15 rejects, so it needs an explicit amendment.

**OQ4. What must a pipe server's identity be?**
- 1.0.17: nothing. libssh2 connects with `SECURITY_IDENTIFICATION` and checks no server (`agent_win.c:142-153`).
- TR1.8 §4 proposes the pipe server's token has the caller's SID and logon `AuthenticationId`, and says that if the native OpenSSH service runs as SYSTEM "this proposed same-SID rule cannot be frozen unchanged".
- Options: (a) no check, as libssh2; (b) caller SID, or the well-known local SYSTEM SID for a service pipe; (c) caller SID only (breaks the native service).
- **Recommend (b)**, conditional on the P03 row: find out on dabeest whether the OpenSSH Authentication Agent service runs as SYSTEM, which needs the service started (OQ9). Until then 3.3 ships (b) with SYSTEM admitted, and a test pins it.

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
- **Recommend (a)**, with the Unicode and empty-`HOME` differences listed in the migration notes. Reproducing a likely bug as parity serves no user. Find out first: X-row for the cause of the Unicode refusal (UTF-8 narrow path in libgit2's `sysdir`, `sysdir.c:328-330`).

**OQ8. Key types and host-key algorithms: match 1.0.17's WinCNG limits or exceed them?**
- 1.0.17: libssh2 on WinCNG, `LIBSSH2_ED25519 0` (`wincng.h:74`); ed25519 host keys and file keys cannot be used. Agent-signed ed25519 may still work, because the agent signs and libssh2 forwards the blob (to be shown by X4).
- Options: (a) match: the transport uses the same `libssh2-sys` and gets the same limits; document them; (b) build libssh2 against OpenSSL on Windows (`openssl-on-win32`), a build and packaging change that makes the transport support more than 1.0.17 does.
- **Recommend (a).** Parity is the rule (OD13) and (b) is its own project (vcpkg, licensing, a divergence between the transport and the native route). Step 3.8 only has to avoid failing differently from 1.0.17 (the `method_pref` strip, section 2.3).

**OQ9. Which dabeest authorizations does the baseline need?**
- Context: the baseline lists three as unanswered or held: (1) one-certificate Windows trust transaction (guarded proposal in the 2026-10-03 run) for 1.0.17's native HTTPS rows; (2) a distinct local account for helper-identity and Digest rows (and cross-SID Pageant); (3) starting the OpenSSH Authentication Agent service for the native pipe row. Also: downloading the pinned Win32-OpenSSH release (OQ6).
- Options: approve each separately, or decline and mark the rows unexecuted (the design then drops or marks provisional the claims they support).
- **Recommend: approve (1) now**, because B11 to B16 and B18 cannot run without it and they decide the HTTPS precedence rules; approve (2) only if OQ10 keeps Digest in scope; (3) only if OQ4's SYSTEM question matters (it does for the native service). This is the largest schedule lever in Phase 2.

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
- Options: (a) required for the compile shapes from day one, the test run advisory until step 0.2 clears the red tail; (b) required entirely once green; (c) advisory.
- **Recommend (a).** The compile shapes are cheap and stop the `cfa14b8`-class break (the `#[cfg(not(windows))]` that attached to the next import and broke v1.0.5). Lanes are local clones that never trigger CI before merge: for those, either push a lane branch (the lane owner's call) or run `windows_lane_check.py` (step 0.4).

**OQ14. gwz-sspi 0.1.0's activation: one review, or two?**
- Context: TR4.10 has its own dual review (secrets); gwz-sspi's `RELEASE.md` wants "implementation acceptance and Windows qualification" before the activation; amendment 2 §3.21 O3 leaves its route unset; O2 says gwz-py also waits.
- Options: (a) step 4.8's dual review also accepts the implementation, and 5.5 is a short release-gate check; (b) a separate dual activation review at 5.5.
- **Recommend (b), scheduled early** (any time after 4.8), because its failure mode is a release that cannot publish gwz-cli, and it can run beside Phases 5 and 6. Also confirm O1 (the trusted publisher's environment) now, which needs the operator on crates.io.

**OQ15. Keep the name `gwz_windows_https_qualification` while SSH is admitted?**
- Options: (a) keep it until step 5.1 retires it; (b) rename to `gwz_windows_qualification` now.
- **Recommend (a).** The switch has 41 sites in core and 13 files in gwz-cli and gwz-py; renaming it costs a review and a three-repo change for a switch whose life is a few phases. Note the name misdescribes the state for the duration, in the inventory and the checkpoint.

## 9. What must be true on Windows for 1.1.0

A checklist; each line is a step or an existing plan item, so none is new scope.

- **CI.** The Windows leg is green on every push to main (0.1, 0.2). The conditional-compilation and Windows-parity checks are clean (0.3). `windows-matrix.yml` passes on the release commit (6.5; amendment 2 §3.12).
- **SSH on the transport.** Fetch, clone and push over SSH take the transport route on Windows with: an explicit key; URL password; Pageant first, else the OpenSSH agent pipe `SSH_AUTH_SOCK` names, else the default pipe; a missing agent a refusal (B07); `known_hosts` under libgit2's home order, with `HOME` unset working (B03); the key types and host-key algorithms libssh2 on Windows handles, with differences from 1.0.17 documented (3.8); dropped connections retried as Suspect (3.7); option A's background close and the idle watch active (1.5).
- **HTTPS on the transport.** Anonymous, default-credential (OD16, to any host), helper-identity and `gh` authentication; the machine proxy with the bypass list, the inverse precedence stated in the notes; 407, PAC and Digest as OQ10 and OQ11 settle (4.4 to 4.9).
- **Packaging.** The installed CLI and wheel work in paths with spaces and non-ASCII characters; the worker's provenance is checked (4.10).
- **Reviews.** TR1.8 GO with Surface (2.4); Phase 3 and Phase 4 reviews; TR4.10's dual (4.8); TR4.7's dual (5.6); gwz-sspi's activation review (5.5).
- **Publication.** `gwz-sspi 0.1.0` on crates.io before gwz-cli's release (5.5; §3.21).
- **Evidence.** TR8.4's rows pass or the operator has taken OD17 (6.1, 6.2); S5.6's Windows column has an evidence ID or an amended-out cell for every row (6.3); S7.3's Windows rows pass (6.4); the post-release dabeest check passes (6.5). No row is claimed from a mock: a fake Pageant or pipe agent is a test of the product's code, and parity claims rest on real Pageant 0.83 and 1.0.17 runs.
- **Not required.** Windows ARM64, the server's Windows primitives (named-pipe ACLs, AppContainer refusals: Phase 7, 1.2.0), the Windows logon-session must-match row (1.2.0, §3.18).

## 10. Risks to the 1.1.0 schedule

1. **TR1.8 is the critical path and is stuck on approvals.** Its freeze needs rows that need a trust import, an account and a service the operator has not approved (OQ9). Mitigation: ask now; run Phase 1 and the pure parts of Phase 3 meanwhile (OQ1).
2. **dabeest is the only Windows host and is shared.** Phase 2's evidence alone is days of host time, and TR8.4 wants the host idle. Mitigation: push everything portable to `windows-2022` (section 7.2), and book the host in blocks.
3. **No Windows SSH server fixture exists** (OQ6). Without one, Phase 1's end-to-end proofs and every Phase 3 SSH row have nothing to talk to. This is why step 1.1 is first.
4. **Pageant is the least-proved piece.** Receiver quiescence, HWND reuse (unexecuted), and cross-SID behaviour are open (P01, P02), and a broker, if the review demands one, adds a secret owner and a step.
5. **WinCNG.** The transport inherits libssh2's Windows limits, including no ed25519, and the preference code was written assuming OpenSSL-class support. A user whose `known_hosts` has only an ed25519 line is the likely first complaint (X2, 3.8).
6. **Qualification switch removal spans three repositories** (41 sites in core, 13 files in the CLI and Python) and must merge in order (core, then CLI, then Python), while a lane that touches `transport_host` is in flight. Hot-spot discipline in section 4 matters.
7. **TR8.1 itself is not closed** on the other platforms (`CurrentProgramCheckpoint.md:7`, option A in a lane; OD17 pending). TR8.4 measures against it; a Windows number cannot settle before the Unix numbers do.
8. **gwz-sspi's activation** is a release-gate review nobody has scheduled (O3), and gwz-cli and gwz-py cannot publish without 0.1.0 (O2).
9. **Unproved SSPI behaviour under a blocked provider** (P05) and Digest (P07) may force a broker or a scope cut late.
10. **Red tail and warnings on the Windows leg** (125 test failures, 152 warnings) can hide a real regression if step 0.2 is skipped.

## 11. Out of scope

- Windows ARM64, Intel macOS and Linux ARM64 (unsupported in 1.1.0, V110 §2).
- 1.2.0's session host, reuse, server and its Windows named-pipe primitives.
- Any native-route fallback (OD15). The off switch stays the user's choice and is unchanged.
- Changing TR8.1's targets, the adaptive designs, or option A. This plan only proves them on Windows.
- Splitting large files that this plan touches: `ssh_network.rs` is 471 lines, `ssh_setup.rs` 603, `ssh_pool.rs` 442. None is over the 1,000-line alarm; new Windows code goes in new files of at most 500 lines (U-table, steps 1.2, 1.3, 3.3, 3.5, 3.6).

## Appendix A: baseline rows still needed, and who runs them

| Row | Status today | Needed by | Step |
|---|---|---|---|
| B06 Pageant and OpenSSH both | unexecuted | OQ3, 3.6 | 2.2 |
| B08 selected pipe, native service | partial (pipe absent) | OQ4, 3.3 | 2.2 (owned pipe), OQ9 for the service |
| B04 dispositions | characterized | OQ7, 3.1 | 2.1, 2.2 |
| B09 proxy grammar and loopback bypass | partial | OQ11, 4.6 | 2.3, 4.6 |
| B11 to B13 helper precedence | unexecuted | 4.4 | 2.3 |
| B14 Digest | unexecuted | OQ10, 4.8 | 2.3 |
| B15 zones and redirects | partial | 4.9 | 2.3, 4.9 |
| B16 EPA | unexecuted | 4.9 | 2.3 |
| B18 POST challenge | unexecuted | 4.9 | 2.3 |
| P01, P03 cross-SID, native service | partial | OQ4, 3.5 | 2.3 |
| P02 HWND reuse | unexecuted, fixed cap | OQ5 | 2.3 |
| P05, P07 blocked provider, Digest | partial | 4.8 | 2.3 |
| X1 to X9 | new, this plan | 3.1, 3.3 to 3.8, 4.5 | 2.2 |
