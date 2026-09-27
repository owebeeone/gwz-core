# GwzTransportReleasePlanAmendment — SAFETY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportReleasePlanAmendment.md`, SHA-256 `9ef88e44a7307a6d6b56c10719b26d3bdcf65a5b85942a575e23233be008fb9e`, 312 lines, untracked and uncommitted in gwz-core's working tree, status "draft; not implementation authority". Draft-stage review of the text. 2026-09-27.
**Baseline:** root `3480a761b0f91d5dfd731aa93da96261f7c155e7`; gwz-core `4bd92285e90ddc64571b2abaa78fa7b58d80be21`; gwz-cli `ebbea9025632ba8181df7ddb0bb57ac7b09f862e`; gwz-py `0b535dc5815748fdd01d31bad6b2b9738f3f13b2`; gwz-transport `a7a36aec0ec6d31e38647b61567166d612f5d2c5`. The amended plan `GwzTransportReleasePlan.md` hashes `48cccf5841962ff08993cd0ceb9a2ebe6bf9077a9837623aa3a054c17d99faca` (589 lines); the object's line numbers refer to it and were checked against it. Sources were read from the working trees at those HEADs with `cat`, `sed -n`, `rg`, `grep` and `git rev-parse`; third-party sources from `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/native-tls-0.2.18/`, `libssh2-sys-0.3.3/libssh2/` and `openssl-src-300.6.1+3.6.3/openssl/`, and the vendored libgit2 under `git2-rs/libgit2-sys/libgit2/src/` (whose uncommitted working-tree edits are manifests, not the C files read). The tuple was verified at the start and at the end; the object, the plan and all five HEADs were unchanged. During the review an untracked `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-ReviewConsistency.md` appeared; it is not part of the tuple and was not opened.
**Date:** 2026-09-27
**Axis:** SAFETY — what the text permits to go wrong: degraded and mixed-version paths, irreversible steps, disclosure scale, stuck states, "no worse than the status quo" under concrete interleavings, and blast radius. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 3 P2, 4 P3. I pre-commit to GO on a revision that resolves P2-1, P2-2 and P2-3 as specified below; each is a bounded text correction to a requirement the amendment places on TR1.3.

---

## 0. Evidence base

**The object**, all 312 lines, read twice; every plan line number it cites (23, 28, 48, 50, 61, 190, 227–239, 300–305, 376, 414–420, 432–437, 459, 468, 484, 493, 531, 556, 560) was located in the plan and matches the clause the object describes.

**Controlling documents:**
- `gwz-core/dev-docs/GwzTransportReleasePlan.md`, all 589 lines; `GwzTransportReleasePlan-Verdict-1.md`, all 52 lines.
- `dev-docs/GwzCoreServerDesign.md`, all 421 lines (§3 addresses, §4 trust boundary, §5 must-match table at 138–157, §6, §7, §9, §11, §12, §14).
- `dev-docs/GwzConnectionReuseDesign.md`, all 365 lines (§3 configuration, §4 revalidation, §12, §16.2).
- `dev-docs/GwzCoreSessionDesign.md`: §1 (12–55), §3 (87–117), §5.6–§5.8 (278–372), §8 (405–423), §9–§10 (424–460), §12 (504–510).
- `dev-docs/GwzClientCoreTransportProposals.md`: §2 fixed requirements (22–39, G1, G8, G10), P2 and P3 (146–171).
- `gwz-core/dev-docs/GwzRemoteTransportRetryPlan.md`: §4 retriable and non-retriable failures (136–200), the help pins (390–396).
- `gwz-core/dev-docs/GwzV110Plan.md`: S7.2–S7.5 (317–350) and the Phase 8 preamble.
- `gwz-core/dev-docs/GwzRemoteTransportHttpsDesign.md` (CA and root clauses at 220–224), `GwzRemoteTransportDesign.md` (§7.1 at 487, §11 at 797–840), `GwzRemoteTransportSshAgentDesign.md` (§6, 163–167), `GwzRemoteTransportSshSelectedIdentityDesign.md` (125–140, 255–265).
- `dev-docs/AgentProcessRules.md`: L1-08 and L1-09 (255–280), §7.1–§7.3 (990–1070); `dev-docs/GwzProcessOptimization.md` §2 and §4 (29–128).

**Code facts in the object's §2, each checked at gwz-core `4bd92285`:**
- `src/git/endpoint/agent_auth.rs`, all 297 lines: the method admission at 187, the shape check at 223–247, the error return at 114–116, `begin_wait` around the sign request at 179–190. All as stated.
- `src/git/endpoint/agent_client.rs:63–66` (ECDSA sign flags), `ssh_network.rs:25–27` (ECDSA host keys), `src/git/gitbackend/transport_support.rs:255` (`Cred::ssh_key_from_agent`) and 214–231 (selected identity through `Cred::ssh_key`), `src/transport_host/local_command.rs:83–96` (`tls_config`, 1 MiB bound), `src/git/endpoint/https_connection.rs:214–218` (one `Certificate::from_pem`, mapped to `InvalidInput`), `tests/transport_backend/prepare.py:41` (`native-tls = "=0.2.18"`). All as stated.
- `src/git/endpoint/ssh_key_container.rs:269` (`Label::Rsa | Label::Dsa | Label::Ec => Ok(())`) and `ssh_key_snapshot.rs:120–175`: explicit EC key containers are admitted to libssh2's own signing.
- `rg 'env::var|var_os|home_dir|GIT_SSL_CAINFO|SSL_CERT_FILE|set_ssl_cert'` over `gwz-core/src`: the native path sets no libgit2 certificate location; `transport_binding.rs:39–41` and `transport_host/mod.rs:48–52` read `HOME` and `SSH_AUTH_SOCK`; `identity.rs:175` uses `home_dir()`.
- `rg -i proxy` over `gwz-core/src/git/gitbackend`: no proxy option is set; git2-rs `src/proxy_options.rs:1–30` derives `Default`, whose `git_proxy_t` is `GIT_PROXY_NONE`.
- gwz-cli: `globalargs/parser.rs:54–57` (`--identity`, `--remote-identity`, ungated by `gwz_transport_candidate`); `globalargs/dispatch.rs:315–330` (hook and log commands run CLI-side); `pager.rs:8, 55–70, 151–157`; `hook/setup.rs:895–915` and `hook/env.rs:240–260` (CLI-side local writes).
- gwz-py: `src/gwz/bridge.py` (`CoreBridge`, `NativeCoreBridge`; no `SocketCoreBridge` or `StreamCoreBridge` exists yet), `src/gwz/cli.py` exists.

**Third-party sources:**
- native-tls 0.2.18: `src/imp/openssl.rs:202–210` (`from_pem` → `X509::from_pem`, first certificate; `stack_from_pem` exists), `src/imp/security_framework.rs:212–235` (`from_pem` requires exactly one certificate, else `errSecParam`; `stack_from_pem` exists), `src/imp/schannel.rs:160–172`.
- libssh2 1.11.1_DEV (`include/libssh2.h:51`): `src/userauth.c:1516–1897` (`_libssh2_userauth_publickey` probes, waits for `SSH_MSG_USERAUTH_PK_OK` at state `sent`, calls `sign_callback` at 1748, returns `PUBLICKEY_UNVERIFIED` on callback error at 1760–1771; handles `sk-ecdsa-sha2-nistp256@openssh.com` and `sk-ssh-ed25519@openssh.com` at 1809–1812); `src/agent.c:180` and `src/agent_win.c:136` (`getenv("SSH_AUTH_SOCK")`).
- libgit2 (vendored): `streams/openssl.c:130–160` (`SSL_CTX_set_default_verify_paths` at init, 148); `remote.c:1147–1160` (proxy environment, read only under `GIT_PROXY_AUTO`); `sysdir.c:357, 402–405` (`HOME`, `XDG_CONFIG_HOME`); `repository.c` reads gated on `GIT_REPOSITORY_OPEN_FROM_ENV`; `util/fs_path.c:1984, 2081` (`SUDO_UID`, `PATH`).
- OpenSSL 3.6.3: `crypto/x509/by_file.c:56` and `by_dir.c:91` read `X509_get_default_cert_file_env()` and `_dir_env()` — `SSL_CERT_FILE` and `SSL_CERT_DIR` — when default paths are set.

**Commands:** `shasum -a 256` (start and end), `git rev-parse HEAD` per repository (start and end), `git -C gwz-core status --short`, `wc -l`, `cat -n`, `sed -n`, `rg`, `grep`, `ls`, `find`. No build, test, cargo, network, write or git mutation.

## 1. Findings

### [P2-1] The must-match floor for native routes through a server is anchored to a §5.8 disclosure that omits OpenSSL's trust-root environment, so the routes this amendment adds can silently use the server's TLS roots

**Location.** Object §3.4 "Native routes through a server" (lines 114–117), which replaces plan line 232 and adds the switch-off bullet for TR1.6's and OD11's routes; §3.11 (line 271), which relies on the `auto` key; §3.6's exit row (line 215), which tests `SSH_AUTH_SOCK` only. The floor is "at least `SSH_AUTH_SOCK`, and each read the contract's §5.8 lists". Contract §5.8 (GwzCoreSessionDesign.md:363–365) discloses only "the SSH agent socket when they connect, and the home directory that locates git's global configuration"; the server design's must-match row (GwzCoreServerDesign.md:147) lists `HOME`, `XDG_CONFIG_HOME`, `SSH_AUTH_SOCK` and the Windows home variables, and §5 says the list "is kept beside contract §5.8's disclosure, so the two cannot drift". The implemented list will therefore be §5.8's.

**Violated invariant.** Server design §5: "The server's value must never silently replace the client's" and "The server's own environment is used for nothing on the session path"; the amendment's own rule that "every process-wide read the native path makes joins the session's must-match set".

**Evidence.** On Linux, gwz's native HTTPS is libgit2's OpenSSL stream. `streams/openssl.c:148` calls `SSL_CTX_set_default_verify_paths` once at libgit2 initialisation; OpenSSL's `by_file.c:56` and `by_dir.c:91` then read `SSL_CERT_FILE` and `SSL_CERT_DIR` from the process environment. gwz-core's native path never sets a certificate location (no `set_ssl_cert_locations` in `gwz-core/src`). In a server that init happens in the server's process, from the server's environment, at server start. The transport path, by contrast, takes `GIT_SSL_CAINFO`/`SSL_CERT_FILE` from the session snapshot (`local_command.rs:85`). Neither §5.8 nor the server design names `SSL_CERT_FILE` or `SSL_CERT_DIR`, and §5.8 itself says gwz's checker "cannot see" these reads. (The proxy environment at `remote.c:1147–1160` is read only under `GIT_PROXY_AUTO`, which gwz's native path does not set, so it is not a native-path read today; it becomes one the moment TR1.6 or the native branch enables proxy detection.)

**State sequence.** Linux. A server is started from a shell where `SSL_CERT_FILE` is unset (or names a pinning bundle). A client whose `SSL_CERT_FILE` names a private CA runs a private HTTPS operation authenticated through a non-gh helper. TR1.6's native route is chosen before any connection opens; the amendment's check compares `SSH_AUTH_SOCK`, `HOME` and `XDG_CONFIG_HOME`, which match; nothing refuses. libgit2 verifies the remote against the server's roots: the operation either fails TLS where the in-process command succeeds, without a refusal that names the cause, or accepts a certificate the client's own bundle would not. Under `auto`, the key hashes only must-match values, so the same server is selected; §3.11's "TR1.3 decides whether the `auto` key avoids that" cannot avoid a value the list does not know. The same holds with TR1.5's switch on.

**Impact.** Silent substitution of the server's trust roots for the client's on exactly the routes this amendment sends through a server, and a parity break the server design promises will be refused instead.

**Required correction.** (1) The floor names `SSL_CERT_FILE` and `SSL_CERT_DIR` for every build whose native HTTPS stream is OpenSSL, beside `SSH_AUTH_SOCK`. (2) TR1.3 is required to enumerate, per platform and from the vendored sources, every environment read that libgit2, its TLS stream and libssh2 make on the native path, to record that list as an amendment to contract §5.8's disclosure (the server design keeps the two side by side), and to state that any proxy detection added to the native branch joins the list. (3) §3.11 drops or qualifies its reliance on the `auto` key.

**Closure test.** Phase 7 exit gains: a server and a client with different `SSL_CERT_FILE` values; the client's private HTTPS member takes TR1.6's native route; it is refused before any connection opens with `server_environment_mismatch` naming `SSL_CERT_FILE`, and the disposable TLS fixture records no handshake. A source-level test asserts the must-match list holds every name in TR1.3's enumerated list.

### [P2-2] The SSH remote form's launch rule refuses argv option injection but not the expansion of the address inside the user's SSH configuration

**Location.** Object §3.4, SSH remote form, question 2 (lines 153–157): "a user or host that starts with `-` is refused, and the destination follows `--`. This is the injection class Git fixed as CVE-2017-1000117"; "the user's SSH configuration applies". OD12's "If yes" exit row (line 258) tests only the `-` prefix.

**Violated invariant.** The rule's own intent that no part of the address selects what ssh executes. OpenSSH expands `%h`, `%r`, `%p` and `%n` from the destination into `ProxyCommand`, `LocalCommand`, `Match exec` and `KnownHostsCommand` and runs the result through the user's shell. Clients before OpenSSH 9.6 performed no character validation of the hostname or user (CVE-2023-51385, the class Git closed for submodule URLs); 9.6 added `valid_hostname`/`valid_ruser`. Enterprise clients at 8.x remain common, and a `Host *` `ProxyCommand … %h %p` is a common corporate configuration.

**Reproduction.** With such a client and configuration, an address whose host part contains a backtick or `$(…)` — from `--server`, or from `GWZ_SERVER` if TR1.3 admits it — passes the amendment's two checks, reaches ssh after `--`, is expanded into the ProxyCommand and executed by the user's shell before any host-key check.

**Impact.** Code execution with the user's authority from an address string; §2 item 3 names the unread sources that produce such strings.

**Required correction.** Question 2 adds: after parsing, gwz itself limits `user` to the characters OpenSSH 9.6's `valid_ruser` admits and `host` to those `valid_hostname` admits (letters, digits, `.`, `-`, plus `_` for the user and a bracketed IPv6 literal for the host; neither starts with `-`), independent of the installed ssh; the port is a decimal in 1–65535; the path is absolute; and no address component reaches ssh through a `-o` option.

**Closure test.** The "If yes" exit rows gain: an address whose user or host holds any character outside the set, including one case with a backtick and one with `$(`, is refused before `ssh` runs, with a spawn double recording zero spawns.

### [P2-3] The remote form does not say how `ssh` is found; the gwz-py CLI's spawn would search the caller's directory on Windows

**Location.** Object §3.4, SSH remote form, question 2 ("The client runs `ssh` directly, never through a shell") and question 8 (the Windows OpenSSH client on dabeest). Contrast the local client form (lines 141–143), which fixes the executable as the CLI's own.

**Violated invariant.** Nothing a workspace or repository contains chooses a program the client runs — the invariant the amendment applies when it fixes the remote command as text and refuses `-`-prefixed parts.

**Mechanism.** Python's `subprocess` on Windows calls `CreateProcessW` with a null application name; the documented search order puts the parent's current directory before `PATH` unless `NoDefaultCurrentDirectoryInExePath` is set. gwz-cli's Rust `Command::new("ssh")` has not searched the current directory since Rust 1.58, but gwz-py's CLI is Python, and both CLIs gain the form. By design the caller's directory is inside the workspace or a member repository (server design §7: core finds the workspace from the caller's directory).

**Reproduction.** A member repository holds `ssh.exe` at its root; on Windows the user runs `gwz-py … --server ssh://myhost/…` from inside it. The repository's `ssh.exe` runs with the user's authority and receives the address and every frame the client sends.

**Impact.** Code execution from repository contents, and disclosure of request bodies to the planted program.

**Required correction.** Question 2 states that both CLIs resolve `ssh` to an absolute path through `PATH` only — never the current directory, the workspace or a repository — before spawning, and names the mechanism (an explicit `PATH` search with `PATHEXT` limited to `.exe` on Windows; `PATH` with empty elements refused on POSIX).

**Closure test.** An "If yes" exit row on dabeest: with an `ssh.exe` placed in the caller's directory, the remote form spawns the `PATH` ssh (a fixture that records its own path) and never the planted one; the same on Linux and macOS with an `ssh` in the caller's directory and `.` absent from `PATH`.

### [P3-1] POSIX automount paths are acknowledged but left to "whether the parser refuses them", against the amendment's own Windows precedent

**Location.** Object §3.4, addresses bullet, last sub-bullet (line 130), against lines 124–128 (Windows forms: must refuse) and §2 item 3 (line 55: the realistic source is an unread `GWZ_SERVER`).

**Violated invariant.** "Everything else is refused with a named error code before any file-system, pipe or network call takes the address."

**State sequence.** macOS's default `/etc/auto_master` maps `/net -hosts`. `GWZ_SERVER=/net/10.9.9.9/x/sock` from a direnv file or CI variable; `gwz status`; the listener check's `lstat`, or `connect`, looks up `/net/10.9.9.9`; automountd contacts 10.9.9.9 (portmapper and mountd, AUTH_UNIX carrying uid, gid and hostname) and the command blocks for the mount timeouts, in a kernel lookup that Ctrl-C may not interrupt. Linux autofs `-hosts` maps behave the same where configured.

**Impact.** A stuck command and a network contact from an address string — the class the amendment refuses on Windows — with a smaller payload (no credential leaves; uid, gid and hostname do).

**Required correction.** Replace "and whether the parser refuses them" with a requirement: the client refuses an address under an autofs or network filesystem, checked on the longest existing ancestor that does not itself trigger a mount (macOS `statfs` `f_fstypename`, Linux `f_type`), before any lookup below that ancestor; TR1.3 lists the filesystem types.

**Closure test.** Phase 7's refused-address row on macOS: an address under `/net` is refused, with a double recording no lookup below `/net`; on Linux, an address under an autofs mount point is refused.

### [P3-2] The handshake marker "uses the remote process's own environment" is not restricted to the stdio host

**Location.** Object §3.4, SSH remote form, question 3 (lines 158–160): "The handshake says so explicitly … an append-only schema addition"; the stdio "Environment" bullet (line 139) and "Trust" bullet (line 137).

**Violated invariant.** Server design §5: "The server's own environment is used for nothing on the session path"; "A command run through a server must behave as the same command run in the client's own process, or be refused".

**State sequence.** A socket client sends `SessionOpen` with the marker and no snapshot. A host that honours the marker wherever it arrives serves that session from the server's snapshot, and the must-match comparison has no client values to compare. The peer is the same user, unsandboxed, so this is not an OS-level escalation — but a CI job's session then runs under whatever token started the server, and the exit row "no environment entry leaves the client" passes while the rule it protects is inverted.

**Required correction.** State that only the stdio host accepts the marker; a socket host refuses a `SessionOpen` without a snapshot with `invalid_request` before `SessionOpened`; the local stdio form never sends it.

**Closure test.** A server-design §12 row: the socket host refuses the marker; the stdio host accepts it; the Phase 7 remote-form row also asserts the marker is present in the client's `SessionOpen`.

### [P3-3] Question 1 leaves `GWZ_SERVER` carrying the SSH remote form open, against the amendment's own source evidence, and the question list has no rule for a run without a terminal

**Location.** Object §3.4, SSH remote form, question 1 (lines 149–152), question 7 (171–174); §3.11 (272–275); OD12's "If yes" rows (253–259).

**Reasoning attacked.** §2 item 3: "Neither form would be typed on purpose. The realistic source is a `GWZ_SERVER` set by something the user does not read closely". For the Windows remote pipe this evidence yields a refusal; for the SSH form it yields "say whether". If TR1.3 admits it: a cloned repository's `.envrc` or a CI variable makes every command open ssh to the named host — the user's public keys are offered (identity, linkable to a GitHub account), the user's `ForwardAgent` configuration applies, ssh's password prompt appears under the attacker's banner, every request body goes to the host, and the reply sets the exit code. The harm is bounded by ssh's own trust checks and equals a hostile `git` remote, which is why this is P3, but the amendment's own precedent is refusal. Separately, question 7 asks only "how ssh's own prompts reach the user"; with no terminal (CI, gwz-py, an agent), a host-key or password prompt either fails or hangs, and "the address adds no ssh option" leaves `BatchMode` undecided.

**Required correction.** Question 1 becomes: only `--server` carries the form; `GWZ_SERVER` and the library never do, and any other answer must show, for each source §2 item 3 names, what stops the connection. Question 7 adds the no-terminal case: whether ssh is given a tty, that a prompt with no terminal fails within a bound with a named error, and whether `BatchMode` is set. The "If yes" rows gain: `GWZ_SERVER` holding the form is refused before ssh runs; a no-terminal run against an unknown host key fails within a bound and leaves no ssh process.

**Closure test.** Those rows.

### [P3-4] TR2.7's additive roots are wider than git's semantics for the same variables, and the migration notes are not required to say so

**Location.** Object §3.5 TR2.7 (lines 185–192), §3.13 "CA roots" (line 290), §3.7's S7.2 list (225–228).

**Fact.** git, through curl, uses `GIT_SSL_CAINFO`/`SSL_CERT_FILE` instead of the default store; OpenSSL's default-path init uses `SSL_CERT_FILE` instead of the default file. The transport adds the file's certificates to the platform roots, and TR2.7 makes that apply to every certificate in the file. A bundle assembled to restrict trust — a corporate CA only, to force traffic through an inspecting proxy or to refuse public CAs — therefore gives less protection on the transport route than with git. §3.13 records the additive choice but not that it is wider; §3.7's list of what S7.2's notes must cover has no CA item.

**Impact.** A user relying on a restricting bundle gets public roots trusted without a note that says so.

**Required correction.** §3.13 states the widening; §3.7's S7.2 list gains "the CA file's certificates add to the platform roots on every route that reads the file; a bundle cannot restrict trust below the platform roots", or the decision that replaces them.

**Closure test.** The notes' text, and a TR2.7 test that a server certificate chaining to a platform root is accepted while `GIT_SSL_CAINFO` names a bundle without that root — documenting the semantics whichever way §3.13 finally decides.

## 2. Invariant analysis

**Strict address parsing closes the Windows forced-authentication hole.** Attacked with `\\host\pipe\x`, `//host/pipe/x`, `\\?\UNC\host\…`, `\\host@SSL\…`, `\\localhost\pipe\x`, `\\?\pipe\x`, `\\.\GLOBALROOT\…`, `\\.\UNC\…`, `\\.\pipe\..\..\GLOBALROOT\…`, `\\.\PIPE\x`, forward slashes after the prefix, embedded backslashes, a trailing newline, and trailing dots or spaces. Every network-reaching form fails the literal-prefix rule or the `/`, `.` and `..` component rules; the only Win32 normalisation the grammar leaves open, trimming of trailing dots and spaces in the last component, can at most resolve one level up, to the NPFS root, which is local. A per-session DosDevices `pipe` redirection needs the same user's own act. The `auto` pipe is under `\\.\pipe\`. Held. The POSIX grammar excludes the abstract namespace and NUL; the automount gap is P3-1.

**The listener sandbox check fails closed.** Linux `SO_PEERCRED` names the process that called `listen()`; reading its `/proc/<pid>/ns/*` needs same-uid dumpable access, which a normal server has; a process that listened and then sandboxed itself keeps its PID and is caught at connect time. Passing a listening socket to a sandboxed process needs an unsandboxed accomplice, which already has the user's authority. Rust and socket2 open sockets `SOCK_CLOEXEC`, so git, gh, hooks and helpers do not inherit a listener. A dead listener's PID yields "cannot be made", which refuses. The one hole found — a listening socket inherited across a Python `fork()` in a `gwz-py server` host, with the parent gone and its PID recycled — ends in a connect into an unread backlog, a residual below the bar (the client's handshake timeout is the server design's content). Windows `GetNamedPipeServerProcessId` plus a token check has the same accomplice-only race. Held.

**Native routes through a server.** `SSH_AUTH_SOCK` is in the must-match set in every build and therefore in the `auto` key; the exit row at line 215 covers it. `HOME`/`XDG_CONFIG_HOME` cover libgit2's global configuration. The credential-helper spawn is closed by contract §5.8's `git credential fill` rule with the snapshot and by the `env`/`process` release gate. The proxy environment is not a native-path read today (`GIT_PROXY_NONE`). The gap is TLS roots (P2-1). Refusals name the variable, never the value, and the routed refusal names the cause and `--no-server`: diagnosable without disclosure.

**TR2.8 / OD11.** The pre-connect listing's TOCTOU is closed by the second bullet: the transport never offers a key type it cannot sign with, so a key that appears after the route check is skipped, and the test row at line 205 pins it. Disclosure: the ledger and `--verbose` name the route and cause, never the key; the redaction list gains comments and fingerprints. Identity parity with 1.0.17 holds whenever the listing is stable; only the listing race can authenticate with a later key than libssh2's agent order would, which the same row records. libssh2's agent path handles `sk-*` methods (`userauth.c:1809–1812`) and §4 requires recorded evidence before the notes claim the native route serves a routed type. The ECDSA shape statement (curve and two integers, per curve) is sufficient at plan level. §3.13's "explicit identities unchanged" hides no gap: EC containers are admitted to libssh2's own signing. Held.

**TR2.7.** Parser exposure is bounded by the 1 MiB read and TR1.2 §3's reduction to `CERTIFICATE` blocks; refusing a malformed block matches git; a private-key block in the file is filtered before it can enter an instance. The widening is P3-4.

**The stdio mode.** The trust claim holds: a starter that hands the pipes to a sandboxed process spends its own authority, as it could by running any command for it. The descriptor rule, the null-device stdio and stderr logging make stray writes harmless; Rust's and Python's spawn defaults keep the client's pipe ends from `forall` children. End of input is contract §8 closure; a write failure coincides with it in every realistic interleaving. Must-match through the local form is trivially satisfied by inheritance; `env` debt cannot cross sessions in a one-session process, so the debt gate's silence on `--stdio` is safe. Under an untrusted `GWZ_SERVER` the local form runs the CLI's own executable with the same authority. Mixed versions: the local form is the same binary; the remote form is bounded by the handshake's version check and the 64 MiB frame cap when a foreign command answers. Held; residuals below.

**The SSH remote form's question list.** Questions 3, 4 and 6 bound credential spread and reply-driven effects: no environment leaves the client, the remote uses its own credentials, `forall` is refused, and the design must show no reply reads or writes a local file or runs a command. CLI-side commands (`hook`, the clone probe, the pager) act locally from the client's own inputs, not from replies. Command-line option injection is closed by `--` and the `-` refusal. The gaps are the configuration-expansion class (P2-2), executable resolution (P2-3), the source question and the no-terminal case (P3-3). OD12's pre-reviewed conditional edit is a safe widening mechanism: the rows and notes are listed now, the design itself carries dual review plus Surface, a NO-GO confined to the section withholds the form, and §6(a)'s line 493 edit already gates "every client form of `--server` and `GWZ_SERVER`".

**§3.13's agent confirmation.** Acceptable as recorded: a confirmation answered after the 9 s stall fails the attempt, a retry re-prompts, and `--ssh-timeout 0` restores the unbounded wait 1.0.17 had. No data is lost; a stale prompt approved late signs for a closed connection.

**Scope and blast radius.** TR2.7 and TR2.8 are transport code behind the candidate cfg; the stdio mode is a mode of the gated `server` command; line 493's edit gates every client form; Phase 2's rule for main is unchanged. Nothing widens what a release cut from main before Phase 9 ships. The Surface claim is defensible: the address grammar is effected as a freeze under TR1.3's Surface review, which covers `--server`, `GWZ_SERVER` and `SocketCoreBridge` wholesale, and under S7.5's.

## 3. Risks and next action

**Residual risks below the finding bar.**
- OD11's listing race can authenticate with a different agent key than 1.0.17 would; the test row records it, and the notes should say so.
- RSA keys against a server that offers only `ssh-rsa` (SHA-1) fail on the transport by the deliberate downgrade refusal at `agent_auth.rs:220–222`, with no native route; §3.2's row "ed25519, RSA and, after TR2.8, ECDSA" should read "RSA with `rsa-sha2-*`", and the notes should name it.
- A hardened server under a seccomp filter or in another mount namespace is refused by every client, fail closed; the client's refusal should name the cause.
- The stdio child in the local form shares the terminal's process group or console: TR1.3 should state its SIGINT and Ctrl-C disposition so the client's interrupt protocol governs; the descriptor rule should be stated for the client's end too.
- Retried setups re-prompt a confirm key up to four times within 44 s; the notes should state `--ssh-timeout 0` as the way to wait on a confirmation and name the prompt-fatigue effect.
- OD11's user-visible route change carries no Surface item where TR1.6's does; the S7.2 ledger row and notes are its only user-facing record. This is the Consistency axis's matter.
- The native branch ignores `GIT_SSL_CAINFO` on every platform and proxies entirely, while the transport honours both; S7.2's "paths" reading must cover it. Pre-existing.
- A client connecting to a listener that passes every check but never answers has no stated handshake bound; server-design content, deferred.

**Next action.** The drafter revises the amendment to resolve P2-1, P2-2 and P2-3 (and P3-1 to P3-4 at their discretion), identified by a new SHA-256; the same two axes re-verdict the revision. On my pre-commit, a revision that resolves the three P2s as specified is GO on this axis without a further round.
