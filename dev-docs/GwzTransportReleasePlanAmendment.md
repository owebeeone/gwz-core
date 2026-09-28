# GWZ transport release plan — amendment: SSH key types, CA bundles, server addresses and a stdio mode

Date: 2026-09-27. Status: **accepted at SHA-256 `213a164b79b7e46ce3855c699b491dcbd059f479e28d1a393b18e8a1ccb6514c` after [Consistency-1](GwzTransportReleasePlanAmendment-ReviewConsistency-1.md) and [Safety-1](GwzTransportReleasePlanAmendment-ReviewSafety-1.md) reported GO; this accepts the amendment text only**.
- This status sentence was added after that GO.
- So were the corrections the reviewers cleared without a further round. [Verdict-1](GwzTransportReleasePlanAmendment-Verdict-1.md) records them.
- Revision 1 applied the [first remediation plan](GwzTransportReleasePlanAmendment-RemPlan.md) after the [first verdict](GwzTransportReleasePlanAmendment-Verdict.md).
- On 2026-09-28 the operator decided OD12: yes. §3.11's "If yes" edits apply, as the changelog records.
- Amended 2026-09-28 by [`GwzCoreServerDesign.md`](../../dev-docs/GwzCoreServerDesign.md). This document remains authoritative only as amended for §3.4's Linux probe, macOS probe and `/net` sentences, and §3.6's macOS rows. The operator signed off the four clauses on 2026-09-28.
- Acceptance authorizes no implementation, commit, tag, push or publish.

This amendment controls the [transport release plan](GwzTransportReleasePlan.md), and one sentence of the [SSH agent design](GwzRemoteTransportSshAgentDesign.md).
- It records one operator decision, OD11, and opens another, OD12.
- It adds two Phase 2 corrections, TR2.7 and TR2.8.
- It adds five changes to TR1.3's revision of the server design:
  - strict address parsing;
  - a sandbox check on the listener;
  - native routes through a server;
  - a stdio mode;
  - the SSH remote form, as a separable design question.

It retires no step and moves no phase. It extends Phase 8's sign-off and the adopted S7.2, S7.3 and S7.5 only by the items §3.7 and §3.8 list. On 2026-09-27 the operator asked for it, on the recommendations it records.

## 1. Documents controlled

- `gwz-core/dev-docs/GwzTransportReleasePlan.md`, accepted at SHA-256 `4ec6ba33da5311921edfe15e5e7f8c9b7cd245fec5eb7e9465875997c0ba996d`. It was committed at gwz-core `23d8ed9b` with the post-GO corrections that [Verdict-1](GwzTransportReleasePlan-Verdict-1.md) records and the record of §7's decisions, and now hashes to `48cccf5841962ff08993cd0ceb9a2ebe6bf9077a9837623aa3a054c17d99faca`. Line numbers below are that file's.
- `gwz-core/dev-docs/GwzRemoteTransportSshAgentDesign.md`, accepted: its §5 sentence "Ed25519 and RSA modern-signature fixtures are required; unsupported algorithms fail explicitly." (lines 158–159), which TR2.8 replaces (§3.5).

Only the clauses in §3 change. The rest of both documents stays authoritative as written. Through the plan's TR1.2 and TR1.3, the requirements added here reach two unreviewed drafts: the [reuse design](../../dev-docs/GwzConnectionReuseDesign.md) and the [server design](../../dev-docs/GwzCoreServerDesign.md). Through TR1.3 they also reach the contract's §5.8 disclosure, which the server design already amends (its §8).

## 2. Problem evidence

Code facts are read at gwz-core `4bd92285`.

1. **SSH keys the transport cannot sign with.**
   - The transport's agent-signing callback admits three methods: `if !matches!(method, "ssh-ed25519" | "rsa-sha2-256" | "rsa-sha2-512")` (`src/git/endpoint/agent_auth.rs:187`). Its shape check knows only those (`:223`). These methods fail there with `Unsupported`:
     - ECDSA keys (`ecdsa-sha2-nistp256`, `-nistp384`, `-nistp521`);
     - security keys (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`);
     - certificates (`*-cert-v01@openssh.com`).
   - libssh2 calls the callback only after the server has accepted the offered key: it probes first and waits for `SSH_MSG_USERAUTH_PK_OK`. The callback's error then ends the whole login (`:114`, `if let Some(error) = signer.error.take() { return Err(error); }`), so a usable key listed after it is never tried. The login therefore fails whenever the first agent key the server accepts is one of these.
   - The code already handles ECDSA elsewhere: in the agent client's sign flags (`agent_client.rs:63-66`) and in the host-key list (`ssh_network.rs:25-27`).
   - The native path signs through the agent with libgit2's `Cred::ssh_key_from_agent` (`src/git/gitbackend/transport_support.rs:255`), on libssh2 1.11.1, whose `userauth.c` handles the security-key methods.

   This is the upgrade break that OD10 addresses for HTTPS, here for SSH.
2. **CA bundles.**
   - `tls_config` (`src/transport_host/local_command.rs:83-96`) reads the file that `GIT_SSL_CAINFO` or `SSL_CERT_FILE` names, up to 1 MiB, into `ca_pem`.
   - `https_connection.rs:214-218` passes that file once to `native_tls::Certificate::from_pem`.
   - native-tls 0.2.18, which the candidate harness pins (`tests/transport_backend/prepare.py:41`), parses one certificate there:
     - on Linux (OpenSSL), `X509::from_pem` keeps the first certificate and ignores the rest;
     - on macOS (Security framework), a file with more than one certificate is an error (`errSecParam`). `https_connection.rs` maps it to `InvalidInput`, so every transport HTTPS connection fails while the variable names a bundle;
     - Windows has no transport build yet (Phase 4).

   A corporate bundle, typically the public roots with the company's CA added, therefore always fails on macOS, and fails on Linux unless the needed CA comes first.
3. **Server addresses.** The server design's §3 accepts `auto` or "an absolute address: a socket path on Linux and macOS, or a pipe name (`\\.\pipe\…`) on Windows", and refuses a relative path. It does not refuse two forms.
   - **A remote pipe name.** On Windows, opening `\\host\pipe\name` makes the SMB client connect to `host` and sign in as the user, usually with NTLM, inside the open call and before gwz can check anything.
     - A host the attacker controls gets a NetNTLMv2 response. It can crack the response offline, or relay it to a service that accepts NTLM without signing.
     - This is the forced-authentication class, exploited in the wild through Outlook's CVE-2023-23397, where a UNC path in a reminder did the same.
     - Windows also sends `//host/pipe/name`, `\\?\UNC\host\pipe\name` and WebDAV forms such as `\\host@SSL\…` to the network.
     - TR1.3's listener verification cannot help, because it runs after the open, when the response has already left.
     - Neither protection the design already has stops the sign-in: `PIPE_REJECT_REMOTE_CLIENTS` protects servers, and the client's `SECURITY_IDENTIFICATION` only limits impersonation.
   - **A Linux abstract socket name** (`@name`, a leading NUL). It has no file, owner or mode, so TR1.3's file checks cannot apply. Any process in the network namespace can bind it first.

   Neither form would be typed on purpose. The realistic source is a `GWZ_SERVER` set by something the user does not read closely: a direnv file in a cloned repository, a CI variable, a script, or an agent that builds commands from untrusted text.
4. **A same-user listener inside a sandbox.**
   - TR1.3's client-side verification checks the listener's user.
   - The server refuses sandboxed clients (server design §4), so that a sandboxed process cannot gain its user's authority through the server. The reverse case is open.
   - A sandboxed process of the same user that can write the per-user directory can bind an address before a server starts, or after one exits idle. On macOS, where `XDG_RUNTIME_DIR` is usually unset, that directory is under `$TMPDIR` (server design §4), which command sandboxes often leave writable.
   - The next client passes the user check and sends its `SessionOpen` into the sandbox. That is the environment snapshot and every secret in it, such as `GH_TOKEN`.
5. **Native routes through a server.**
   - TR1.3's must-match rule for the native path covers only the off switch: "With TR1.5's switch on, every process-wide read the native path makes joins the session's must-match set" (line 232). Its floor is "at least `SSH_AUTH_SOCK`, and each read the contract's §5.8 lists".
   - TR1.6's non-gh HTTPS route and OD11's SSH route also run the native path, per operation, with the switch off.
   - In a server that path reads the server's process-wide values:
     - `SSH_AUTH_SOCK`: "libssh2 reads the agent socket from the host's environment" (server design §5);
     - on Linux, `SSL_CERT_FILE` and `SSL_CERT_DIR`. The vendored libgit2 uses OpenSSL for HTTPS on Linux, SecureTransport on macOS and WinHTTP on Windows (`git2-rs/libgit2-sys/build.rs:256-268`). With OpenSSL it calls `SSL_CTX_set_default_verify_paths` once, at initialisation (`streams/openssl.c:148`), and OpenSSL reads both variables from the process environment there. Neither the contract's §5.8 disclosure nor the server design's must-match row names them.
   - A session whose agent or trust roots differ from the server's would then be served with the server's. That is the crossover TR1.3's rule exists to prevent.
6. **A stdio mode.** The pieces exist:
   - The contract's §12 already serves one session over standard input and output, from a test-only host binary, and the server design's §3 makes every byte stream use the handshake.
   - Proposals P2 draws the remote shape: "client ── stream bridge ── byte stream (for example SSH) ── core host on another machine ── core session host".
   - What is missing is a product mode, and the rules that change when the stream reaches another machine:
     - a socket server uses its client's environment (server design §5). Across machines the client's `HOME` and `SSH_AUTH_SOCK` mean nothing, and sending `GH_TOKEN` to another machine spreads credentials;
     - the client's working directory means nothing on the remote host;
     - `forall` runs its commands in the client's terminal (server design §7).
   - The server design excludes a server on another machine because "It would hold the client's credentials on another machine" (§1). A remote session that uses the remote account's own credentials holds none of the client's.

## 3. Superseded clauses and their replacements

### 3.1 The status block and §1 (lines 7, 23 and 28)

- Line 7 becomes: "On 2026-09-27 the operator adopted every recommendation in §7 (OD1–OD10), and OD11 by the [amendment](GwzTransportReleasePlanAmendment.md). OD12 is open."
- Line 23 becomes: "`gwz server` and `gwz-py server` from the server design, reached with `--server` and `SocketCoreBridge`, and the server's stdio mode (TR1.3);"
- After line 28, the decisions list gains: "2026-09-27, by the [amendment](GwzTransportReleasePlanAmendment.md): OD11; the server's stdio mode ships in this release; its SSH remote form is designed in TR1.3 and decided under OD12."

### 3.2 §2, scope (lines 48, 50 and 61)

- After line 48, this row, shown under the table's header:

  | In this release | Recorded as unsupported in this release |
  |---|---|
  | SSH through an agent whose keys the transport can sign with: ed25519, RSA with `rsa-sha2-256` or `rsa-sha2-512` and, after TR2.8, ECDSA | SSH through an agent that holds a key the transport cannot sign with, such as a security key or a certificate. Each SSH remote that authenticates through that agent takes the native route, chosen before any connection opens, and the migration notes list it (OD11, TR2.8) |

- Line 50 becomes this row, shown under the table's header:

  | In this release | Recorded as unsupported in this release |
  |---|---|
  | `gwz server` and `gwz-py server` over a Unix-domain socket or a local named pipe, for the same user on the same machine; the server's stdio mode, which serves one session to the process that starts it (TR1.3) | A server for another user; any TCP listener; a remote pipe name, an abstract socket name, or an address in an automounted or network file system; a server on another machine, unless OD12 brings in the SSH remote form |

- Line 61's redaction list gains "agent key comments and fingerprints".

### 3.3 TR1.2 (after line 190)

A bullet:
- **Key types.** Its answers to questions 3 and 4, eligibility and revalidation, cover every key type TR2.8 admits, ECDSA included.

### 3.4 TR1.3 (lines 227–239)

TR1.4b places the steps for these changes in Phase 7, as it does for the rest of TR1.3.

- **Listener verification.** After line 229, a bullet:
  - on all three platforms, the listener's process passes the server design's §4 sandbox refusal:
    - on Linux, the process that `SO_PEERCRED` names;
    - on macOS, the process that `LOCAL_PEERPID` names;
    - on Windows, the pipe server's process (`GetNamedPipeServerProcessId`), whose token must not be an AppContainer's or below medium integrity.

    A check that cannot be made refuses, as on the server. The refusal names its cause.
- **Native routes through a server.** Line 232 is replaced:
  - With TR1.5's switch on, every process-wide read the native path makes joins the session's must-match set. That is at least `SSH_AUTH_SOCK`; `SSL_CERT_FILE` and `SSL_CERT_DIR` where the native HTTPS stream is OpenSSL; and each read the contract's §5.8 lists. A mismatch refuses with `server_environment_mismatch`. The `auto` key covers those values when the switch is on.
  - With the switch off, an operation that takes TR1.6's or OD11's native route is checked against the same values when it routes, before any connection opens. A mismatch refuses that operation with `server_environment_mismatch`. The message names the route's cause, the variable but never its value, and `--no-server`.
  - **The list.** TR1.3 enumerates, per platform and from the vendored sources, every environment read that libgit2, its TLS stream and libssh2, with their crypto backends, make on the native path. That includes OpenSSL's own start-up reads, such as `OPENSSL_CONF`, `OPENSSL_MODULES` and `OPENSSL_ENGINES`. It records the list as an amendment to the contract's §5.8 disclosure, which the server design keeps beside its must-match list (its §5 and §8). Proxy detection, if the native path ever gains it, joins the list.
  - TR1.3 states whether the `auto` key also covers those values when the switch is off.
- **Addresses, parsed before any open.** After line 237, a bullet.
  - `--server`, `GWZ_SERVER` and `SocketCoreBridge(address)` accept these forms and no others:
    - `auto`;
    - on Linux and macOS, an absolute path: it starts with `/` and holds no NUL byte;
    - on Windows, a name that starts with the literal prefix `\\.\pipe\`. The rest is non-empty, and holds no `/`, no NUL and no `.` or `..` component;
    - in the two CLIs only: the stdio mode's local form, in `--server` and `GWZ_SERVER`; and, if OD12 brings it in, the SSH remote form, in `--server` alone.
  - Everything else is refused with a named error code before any file-system, pipe or network call takes the address. That includes:
    - a remote pipe name (`\\host\pipe\…`);
    - the other forms Windows sends to the network: `//host/pipe/…`, `\\?\UNC\…` and `\\host@SSL\…`;
    - `\\?\pipe\…`;
    - a Linux abstract name (`@name`, or a leading NUL).
  - **Automounts and network file systems.** On Linux and macOS, the client walks an absolute path from `/`, one component at a time. It refuses the path when a component is an automount trigger, an automounter's mount or a network file system, before looking up the next component.
    - **The probe.** Each probe neither follows a symbolic link nor triggers a mount.
      - On Linux: `statx` with `AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT`, refusing a component that carries `STATX_ATTR_AUTOMOUNT`, and `fstatfs` on an `O_PATH | O_NOFOLLOW` descriptor for the file-system type.
      - On macOS: `open` with `O_SYMLINK` or `O_NOFOLLOW`, and `fstatfs` on the descriptor. TR1.3 establishes how a direct-map trigger presents.
    - **Links.** A symbolic-link component is read, not followed. Its target, made absolute against the link's directory, is walked from `/` under the same rule, to a bounded link depth. Links are walked rather than refused, because `/var`, `/tmp` and `/etc` are links on macOS and `TMPDIR` lives under `/var/folders`.
    - macOS's default `/net` map is one such mount. TR1.3 lists the file-system types.
    - The same check applies to the path `auto` derives from `XDG_RUNTIME_DIR` or `TMPDIR`, which an unread environment can set as easily as `GWZ_SERVER`. The host applies it too, before it creates the per-user directory.
  - The connector takes only the parser's output type, so no code path opens an unparsed string.
- **The stdio mode.** A bullet follows. A mode of the `server` command in both CLIs, working name `server --stdio`, serves exactly one session over its standard input and output. TR1.3 fixes its spelling under Surface review.
  - **The stream.** It uses the contract's byte-stream adapter and the handshake (server design §3), binary on every platform, with no text-mode translation on Windows. Log lines go to standard error. Nothing else is written to standard output.
  - **Lifetime.** The process exits when its session ends: after `session.close`, at end of input, or when a write fails.
    - End of input is channel closure (contract §8), which cancels live work.
    - It opens no socket and takes no lock. `auto` never selects or starts it: the local client form, the SSH remote form or a caller that runs the command starts it.
    - The design lists which of the command's options combine with the mode, each with its default.
  - **Trust.** Its peer is the process that started it, and it runs with that process's user, authority and sandbox. The server design's §4 peer and sandbox checks therefore have nothing to check. The design says why the mode grants nothing its starter lacks.
  - **Descriptors and signals.** No child process inherits the stream. The design states how, for example by moving the channel to private, non-inheritable descriptors at start and pointing standard input and output at the null device. It states the same for the client's end of the stream. It also states how the child treats the terminal's interrupt, so that the client's interrupt protocol (server design §7) governs.
  - **Environment.** A local client sends its snapshot in `SessionOpen`, as over a socket, and the must-match check runs as for a socket. The child inherits its starter's environment, so a mismatch means the starter changed it for the child. The local client form never sends the SSH remote form's marker (question 3).
  - **Reuse.** The mode's host context serves one session. That session reuses connections across its own operations (TR1.2), never across processes.
  - **The local client form.** Both CLIs gain a `--server` form that runs the CLI itself in the stdio mode for one command:
    - gwz-cli runs its own executable;
    - gwz-py runs its own interpreter on `gwz.cli`, never the `gwz` executable (G10).

    gwz-py's library gains no stdio client in this release. The design may point the contract §12 `StreamCoreBridge` at `gwz-py server` in the stdio mode, in place of the Cargo example host. That amends contract §12, as the server design's §8 already does.
- **The SSH remote form (OD12).** A bullet follows. This is a `--server` form that runs the stdio mode on another machine through the system `ssh`.
  - TR1.3 designs it in a section of its own, which never holds up TR1.3's GO. If a blocking finding is confined to that section, the operator may accept the rest without it.
  - The section answers:
    1. **Address and sources.**
       - The form is `ssh://[user@]host[:port]/absolute/remote/path`, parsed by the address parser.
       - Only `--server` carries it. `GWZ_SERVER`, gwz-py's library and every configuration file never do, and the parser refuses the form from them before `ssh` runs. A design that proposes another source must show, for each source this amendment's §2 item 3 names, what stops the connection, and it amends this rule.
    2. **Launch.**
       - **The program.** Both CLIs resolve `ssh` to an absolute path through `PATH` alone, never the current directory, the workspace or a repository. On Windows that is an explicit `PATH` search limited to `.exe`. On every platform, empty and relative `PATH` elements are skipped. The client runs that program directly, never through a shell.
       - **Characters.** gwz itself admits only an allowlist, whatever `ssh` is installed.
         - The host holds ASCII letters, digits, `.`, `-` and `_`, or is a bracketed literal that parses as an IPv6 address, with no zone identifier.
         - The user holds ASCII letters, digits, `.`, `-` and `_`.
         - Neither starts with `-`. The port is a decimal from 1 to 65535.

         OpenSSH expands the host and user into `ProxyCommand`, `LocalCommand`, `Match exec` and `KnownHostsCommand`, which run through the user's shell. Clients before 9.6 check neither for shell characters (CVE-2023-51385).
       - **Options.** The destination follows `--`, which closes the option-injection class Git fixed as CVE-2017-1000117. The address sets no `ssh` option beyond its port, and the user's SSH configuration applies.
       - **The remote command** is fixed text, with nothing from the address in it. The remote directory travels in-band, as `InvocationContext.caller_cwd`.
    3. **Environment.**
       - The client sends no environment. The remote session uses the remote process's own, captured at its start as a driver's is.
       - The handshake says so with an explicit marker, so an empty snapshot is never mistaken for it. Only the stdio host accepts the marker. A socket host refuses a `SessionOpen` that carries the marker, or that carries no snapshot, with `invalid_request` before `SessionOpened`.
       - The design states the frame change as an append-only schema addition (server design §9).
    4. **Credentials.**
       - The remote core uses the remote account's agent, keys and `gh` login, or an agent the user's SSH configuration forwards. The design states the risk of forwarding.
       - Keeping credentials on the client while core runs remotely is client placement, which stays out (§9).
    5. **Paths and inputs.**
       - `--root` and relative operands resolve against the remote directory.
       - The design lists every input the CLI itself reads from a path, such as a message file, and says whether it reads the client's file or refuses.
       - `forall` is refused.
    6. **What the remote side can do to the client.**
       - The remote host is trusted as far as the user's SSH trust goes.
       - The design states the client's local effects in this form: rendering and the exit code. It shows that no reply makes the client read or write a local file or run a command.
    7. **Prompts, termination and errors.**
       - How `ssh`'s own prompts reach the user.
       - A run with no terminal, such as CI, gwz-py or an agent: whether `ssh` gets a terminal, whether `BatchMode` is set, and that a prompt with no terminal fails within a bound, with a named error.
       - Closing the client ends the remote session, through channel closure.
       - Which errors `ssh`'s failures map to.
    8. **Tests and cells.** These run with no live account:
       - against a disposable `sshd` on Linux and macOS;
       - with the Windows OpenSSH client on dabeest.
    9. **The off switch.** The form sends no environment, so TR1.5's snapshot-entry carrier is unavailable. The section says which side's value governs. Recommended: the client's resolved value, carried as a `ProcessAttributes` field, TR1.5's other carrier; the remote's environment and user configuration are not consulted for it. `--verbose` says which value applied.
- **Cells.** Line 238 becomes: "It lists the Design §11 cells (1.1.0 S5.6 rows) the server adds, the stdio mode's included, with their evidence kind."
- **Review.** Line 239 becomes: "**Review:** dual Consistency and Safety, plus Surface. The Surface review covers the `server` command family, `--server`, `GWZ_SERVER`, `--no-server` and `SocketCoreBridge`; the address grammar those three accept, and its refusals' error code; the stdio mode and its local client form; and the SSH remote form, unless the operator accepts TR1.3 without its section."

### 3.5 Phase 2 (after line 300, and lines 301 and 305)

After line 300, two steps.

- **TR2.7: CA bundles** *(under 200 lines)*.
  - **Every certificate.** Every certificate in the file that `GIT_SSL_CAINFO` or `SSL_CERT_FILE` names is added as a root, on every platform, not just the first.
    - Text outside the certificate blocks is ignored.
    - A malformed block, or a file with no certificate, is refused before any connection opens, as an unreadable file is today.
    - The 1 MiB bound stays.
  - **Roots.** The file's certificates add to the platform's built-in roots, as the single certificate does today. That can be wider than Git gives for the same variables, and S7.2's notes say so (§3.8). A test pins it. On Linux, with OpenSSL's default paths pointed at a temporary store that holds the fixture's CA, the connector accepts the fixture while `GIT_SSL_CAINFO` names a bundle without that CA. On macOS, whose keychain a test cannot seed without privilege, TR2.6's review of the connector's construction pins it.
  - **Tests,** with the disposable HTTPS fixture:
    - a two-certificate bundle whose second certificate issued the fixture's server certificate connects: on Linux and macOS now, and on dabeest after S4.5;
    - a bundle with one malformed block is refused, and no connection opens.
- **TR2.8: SSH keys the transport cannot sign with (OD11)** *(under 500 lines)*.
  - **ECDSA.** The agent-signing callback admits `ecdsa-sha2-nistp256`, `-nistp384` and `-nistp521`. Its shape check verifies, for each, the key's curve and the signature's two integers.
  - **No abort on a key it cannot use.** The transport never offers an agent key whose type it cannot sign with. It skips the key, so a later key is still tried.
  - **The route.** Before any connection opens or is reused, each SSH remote of an operation that authenticates through the agent lists the agent's keys.
    - If any key is of a type the transport cannot sign with, that remote takes the native route, as TR1.6 does for HTTPS. HTTPS remotes are unaffected.
    - There is no fallback after an open.
    - `--verbose` names the route and its cause, never the key. S7.2's route ledger gains the route (§3.8).
    - An agent that cannot be reached is reported as today.
  - **Through a server,** TR1.3's rule for native routes applies. Phase 7 tests it.
  - **The agent design.** Its §5 sentence "Ed25519 and RSA modern-signature fixtures are required; unsupported algorithms fail explicitly." becomes: "Ed25519, RSA SHA-2 and ECDSA (P-256, P-384 and P-521) modern-signature fixtures are required. A key of any other type is never offered, and a remote whose agent holds one takes the native route before any connection opens (OD11)."
  - **Tests,** against the disposable SSH fixture:
    - an ECDSA key of each curve authenticates;
    - an agent that lists a security key or a certificate before a usable key takes the native route, and the `--verbose` transport row names the route's cause, never the key;
    - an agent whose listing gains such a key between the route check and authentication authenticates with a later usable key, or fails as `Authentication`, and never offers it;
    - an agent that holds only ed25519 and RSA keys takes the transport, as today;
    - with a disposable `sshd` that trusts a test user CA, whether the native path authenticates with a certificate key through the agent is recorded.
  - **Evidence.** These tests evidence S5.6's cell for agent key types (§3.7).

Line 301 reads "on the settled tree after TR2.1–TR2.5, TR2.7 and TR2.8", and line 305 reads "TR2.2–TR2.5, TR2.7 and TR2.8."

### 3.6 Phase 7's exit (after line 376)

The exit gains:
- **Refused addresses.** Each address TR1.3's parser refuses is refused before any open, and a test double for the connector records no call:
  - on Windows, a remote pipe name and each other network form;
  - on Linux, an abstract name, an address under an autofs mount, and an address under an autofs direct-map trigger, which stays unmounted: the automount daemon records no mount request;
  - on macOS, an address under `/net`, with no lookup below `/net`, and an address whose component is a symbolic link, in a local directory, that points under `/net`, with no lookup below `/net`.
- **Sandboxed listeners.** A listener of the same user inside a sandbox, at the `auto` path and at an explicit address, receives zero bytes, on all three platforms.
- **Native routes through a server,** at an explicit address:
  - A server and a client have different `SSH_AUTH_SOCK` values, and the client's agent holds a key TR2.8 routes native. The client's SSH operation is refused before any connection opens, and the server's agent records zero signature requests.
  - On Linux, a server and a client have different `SSL_CERT_FILE` values, and the client's private HTTPS member takes TR1.6's native route. It is refused before any connection opens with `server_environment_mismatch` naming `SSL_CERT_FILE`, and the disposable TLS fixture records no handshake.
  - A source-level test asserts that the must-match list holds every name in TR1.3's enumerated list.
- **The handshake.** A socket host refuses a `SessionOpen` that carries no snapshot, with `invalid_request` before `SessionOpened`.
- **The stdio mode.**
  - gwz-cli's suite runs once through the local client form on Linux.
  - On all three platforms, both CLIs' local client forms pass the stdio rows TR1.3 lists. They include:
    - end of input cancels live work, and the process exits with no helper left;
    - a child that writes to its standard output or reads its standard input cannot touch the stream.

### 3.7 Phase 8's sign-off (line 402)

Line 402 becomes: "**Sign-off.** S5.6's table gains the server and reuse cells that TR1.2 and TR1.3 list, and a cell for agent key types (ed25519, RSA with SHA-2 and ECDSA) that TR2.8's tests evidence. It keeps its rule: nothing is advertised without evidence."

### 3.8 Phase 9 (lines 414–437)

- **S7.2's route ledger** gains a row for OD11's route (lines 414–419).
- **S7.2's help, docs pages and migration notes** (line 420) also cover:
  - the key types that take the native route, and whether the native path authenticates with each, from §4's records;
  - that the transport signs RSA only with SHA-2 (§3.14);
  - the stdio mode;
  - the agent-confirmation behaviour §3.14 records, and that `--ssh-timeout 0` waits for a confirmation without a clock;
  - that on the transport route a CA file's certificates add to the platform's roots, so a bundle cannot restrict trust below them (TR2.7).
- **S7.3's route checks** (lines 423–429) gain a fifth: one CLI network operation through the stdio mode's local client form.
- **S7.5's Surface list** (lines 432–437) gains:
  - the address grammar and its refusals;
  - the stdio mode and its local client form;
  - the SSH remote form, if OD12 brings it in.

### 3.9 Phase 10's post-release check (after line 459)

It gains: one command through the stdio mode's local client form.

### 3.10 §6 (lines 468, 484 and 493)

- Line 468 becomes: "TR2.3;  TR2.4;  TR1.5 ── TR2.5;  TR3.1 ── TR2.7, TR2.8;  TR2.1–TR2.5, TR2.7, TR2.8 ── TR2.6".
- Line 484's list after TR3.1 gains "TR2.7 and TR2.8".
- Line 493 becomes: "the `server` command and its options, the stdio mode included; and every client form of `--server` and `GWZ_SERVER`;".

### 3.11 §7 (line 508, then after line 531)

- Line 508 becomes: "**Decided 2026-09-27: the operator adopted every recommendation below from OD1 to OD11. OD12 is open.**" The two entries below follow line 531.
- **OD11. SSH keys the transport cannot sign with.** Decided 2026-09-27: the operator adopted the recommendation.
  - Today such a key fails the login, where the native path signs with it (the amendment's §2).
  - Recommended and adopted: the transport signs with ECDSA too (TR2.8). Each SSH remote whose agent holds any key the transport still cannot sign with, such as a security key or a certificate, takes the native route. The route is chosen before any connection opens, and the migration notes list it.
  - The alternative was to refuse such an operation with a message that names the off switch.
- **OD12. The SSH remote form in this release.** Open. TR1.3 designs the form. At TR1.3's GO, the operator decides, on the reviewed design and its test cost, whether it ships in this release.
  - **If yes,** these edits apply, recorded in this amendment's changelog:
    - line 23's replacement (§3.1) reads: "`gwz server` and `gwz-py server` from the server design, reached with `--server` and `SocketCoreBridge`, and the server's stdio mode and its SSH remote form (TR1.3);";
    - the decisions list gains "OD12: yes, <date>";
    - in the replacements of line 7 (§3.1) and line 508 (above), "OD12 is open." becomes "OD12: yes, <date>.";
    - TR1.3's list of contract amendments (the server design's §8) amends the contract's §1 exclusion "remote deployment" to read "remote deployment, other than the SSH remote form of the server's stdio mode";
    - §2's server row (§3.2) reads, in this release: "`gwz server` and `gwz-py server` over a Unix-domain socket or a local named pipe, for the same user on the same machine; the server's stdio mode, which serves one session to the process that starts it; and its SSH remote form, which runs that mode on another machine through the system `ssh` (TR1.3, OD12)". As unsupported it reads: "A server for another user; any TCP listener; a remote pipe name, an abstract socket name, or an address in an automounted or network file system; a server on another machine reached by any other form";
    - §9's first bullet (§3.13) reads: "A server for another user. A server on another machine reached by any form other than the SSH remote form. Client placement; only frame tags 16–31 stay reserved for it.";
    - §9's line-561 replacement (§3.13) applies as written;
    - Phase 7's exit gains the rows TR1.3 lists for the form. They include:
      - one network operation that uses the remote account's credentials;
      - no environment entry leaves the client, and its `SessionOpen` carries the marker;
      - a socket host refuses the marker;
      - `forall`, and each input the design refuses, are refused;
      - closing the client ends the remote session and leaves no helper;
      - a user or host that starts with `-`, or holds a character outside question 2's allowlist, including a backtick and `$(`, is refused before `ssh` runs, and a spawn double records zero spawns;
      - with a planted `ssh.exe` in the caller's directory on dabeest, and a planted `ssh` with `.` absent from `PATH` on Linux and macOS, the form runs the recording `PATH` program, never the planted one;
      - `GWZ_SERVER` holding the form is refused before `ssh` runs;
      - a run with no terminal against an unknown host key fails within a bound and leaves no `ssh` process;
    - S7.2's notes cover the form and the risk of forwarding an agent;
    - S7.3 runs one CLI network operation through the form against a disposable loopback `sshd` on Linux, and Phase 10's post-release check repeats it on one host with a disposable `sshd`.
  - **If no,** the clauses above stand as this amendment writes them, and so does the contract's §1 exclusion. The decisions list gains "OD12: no, <date>", "OD12 is open." in the replacements of lines 7 and 508 becomes "OD12: no, <date>.", and the design waits for a later release. The changelog records the answer.

### 3.12 §8, risks (after line 556)

- **The stdio stream.**
  - Anything written to the stdio mode's standard output corrupts its session.
  - A child that reads its standard input takes frames meant for core.
  - TR1.3's descriptor rule and Phase 7's test cover both.
- **Security-key users lose the transport.**
  - OD11 routes every SSH remote whose agent holds such a key to the native path, so those users get neither pooling nor reuse for SSH.
  - The ledger row makes that visible. Admitting such keys is a later design.
- **Native routes through a server.** A client whose agent or trust roots differ from the server's is refused for routed operations.
  - At an explicit address that refusal is expected. Under `auto`, whether it can arise depends on TR1.3's answer on the `auto` key (§3.4).
  - The rule is only as complete as TR1.3's enumeration, which Phase 7's source-level test pins.
- **The SSH remote form,** if OD12 brings it in.
  - A remote host the user's SSH trusts can answer every request.
  - A `--server` value in a script or an alias can point the client at such a host. `GWZ_SERVER` cannot.
  - TR1.3's answers 1, 2, 6 and 7 bound both.

### 3.13 §9, out of scope (lines 560 and 561, then new bullets)

- Line 560 becomes: "A server for another user. A server on another machine, unless OD12 brings in the SSH remote form, and any other remote form. Client placement; only frame tags 16–31 stay reserved for it."
- Line 561 becomes: "iroh, a physical carrier, or a separate-process wire other than the local server socket, the stdio mode's standard streams and, if OD12 brings it in, the SSH remote form's `ssh` channel."
- New bullets:
  - A gwz-py library client for the stdio mode or the SSH remote form.
  - Admitting security keys or certificates to the transport's own signing.

### 3.14 Unchanged on purpose

- **Explicit identities** (`--identity`, `--remote-identity`) never consult the agent. Their key handling is unchanged.
- **RSA with SHA-1.** The transport signs RSA only with `rsa-sha2-256` or `rsa-sha2-512` (`agent_auth.rs:187`), and never lets libssh2 fall back to `ssh-rsa` (the comment at `:220-221`). A server that accepts only SHA-1 RSA signatures therefore fails on the transport. It gets no native route, because nothing before the connection shows it. S7.2's notes say so.
- **Agent confirmation.** An agent's confirmation prompt (`ssh-add -c`) runs under the 9 s stall clock on the transport path: the sign request sits inside `begin_wait` (`agent_auth.rs:179-190`). The agent design's §6 requires it: "absence of bytes is not evidence of user interaction and cannot pause that clock".
  - The retry plan retries a stalled setup: "A setup stall fails one attempt, and that attempt is retried". A retry asks the agent again, up to four times at the defaults.
  - S7.2's migration notes state this, and that `--ssh-timeout 0` waits for a confirmation without a clock. Changing it amends the retry plan, the agent design's §6 and the timeout plan's interaction accounting.
- **CA roots.** The CA file's certificates add to the platform's built-in roots, as the single certificate does today. That can be wider than Git gives for the same variables, where the file takes the place of the default bundle, and S7.2's notes say so. TR2.7 does not decide whether they should replace the roots.
- **§3 and §10.** The plan's §3 is a record of 2026-09-27 and is not rewritten; this amendment's §2 adds to it. §10 stays the record of the plan's own review.

## 4. Affected tests and evidence

- **Phase 2:** TR2.7's and TR2.8's tests, inside their steps, and TR2.6's review of both.
- **Phase 7's exit:** the rows in §3.6.
- **Phase 8:** S5.6's cell for agent key types (§3.7).
- **Phase 9:** S7.2's ledger row and notes, S7.3's fifth route check, and S7.5's added Surface items.
- **Phase 10:** the stdio row of the post-release check.
- **The native path's key types.** Before S7.2's notes say the native route serves a routed key type, whether the native path authenticates with that type is recorded:
  - the certificate case, by TR2.8's fixture row;
  - the security-key case, by a named manual row that S7.2 runs, with a hardware key, before its notes are written. A manual row against a live account needs the operator's go (the plan's §2).
- **Redaction.** The plan's §2 evidence rule covers the new rows, including agent key comments and fingerprints (§3.2).

## 5. Review and application

- **Review.** Dual peer-blind Consistency and Safety review of this draft's text, identified by its SHA-256.
  - There is no Surface review of this amendment, because it freezes no command, option or API.
  - TR1.3's review includes Surface (§3.4), and freezes what this amendment leaves open: the stdio mode's spelling and options, the client forms, the address grammar's refusals and their error code, and the SSH remote form.
- **On GO,** these edits follow under AgentProcessRules §7.2, each with a changelog entry:
  - **`GwzTransportReleasePlan.md`'s status gains:** "Amended <date of GO> by `GwzTransportReleasePlanAmendment.md`. This document remains authoritative only as amended for its status block's decision record, §1's outcome and decisions, §2's scope rows and redaction list, TR1.2, TR1.3, Phase 2's new steps and TR2.6, Phase 7's exit, Phase 8's sign-off, S7.2, S7.3, S7.5, Phase 10's post-release check, §6, §7, §8 and §9." Its new changelog entry records OD11 as adopted and OD12 as open. The 2026-09-27 entry stays as history.
  - **`GwzRemoteTransportSshAgentDesign.md`'s status gains:** "Amended <date of GO> by `GwzTransportReleasePlanAmendment.md`. This document remains authoritative only as amended for §5's algorithm sentence." `GwzRemoteTransportSshAgentA2.md`, whose clause is scoped to A2, gains a changelog note that TR2.8 admits ECDSA after it.
  - **`GwzCoreServerDesign.md`'s status sentence** adds the TR1.3 changes this amendment requires.
  - **`GwzConnectionReuseDesign.md`,** an unreviewed draft, gains §3.3's key-types rule before its review starts. This is a content edit, made by TR1.2's drafter.
  - **The program checkpoint** records the acceptance, once another lane's uncommitted edits to it have landed.
- **No authorization.** This amendment authorizes no implementation, commit, tag, push or publish.

## Changelog

- 2026-09-27: revision 0 at SHA-256 `9ef88e44…` reviewed. Both axes reported NO-GO ([Verdict](GwzTransportReleasePlanAmendment-Verdict.md)).
- 2026-09-27: revision 1 applied the [first remediation plan](GwzTransportReleasePlanAmendment-RemPlan.md), and was accepted at SHA-256 `213a164b…` ([Verdict-1](GwzTransportReleasePlanAmendment-Verdict-1.md)), with the corrections Verdict-1 records. OD11 is adopted. OD12 is open; its answer, and the list applied, will be recorded here.
- 2026-09-28: OD12: yes, decided by the operator at the GO of TR1.3's [server design](../../dev-docs/GwzCoreServerDesign.md) ([its Verdict-1](../../dev-docs/GwzCoreServerDesign-Verdict-1.md)). §3.11's "If yes" edits apply as written there:
  - §3.1's replacement for the plan's line 23 reads "`gwz server` and `gwz-py server` from the server design, reached with `--server` and `SocketCoreBridge`, and the server's stdio mode and its SSH remote form (TR1.3);";
  - the plan's decisions list gains "OD12: yes, 2026-09-28";
  - in §3.1's replacement of line 7 and §3.11's replacement of line 508, "OD12 is open." becomes "OD12: yes, 2026-09-28.";
  - the server design's §8 amends the contract's §1 exclusion "remote deployment" to read "remote deployment, other than the SSH remote form of the server's stdio mode";
  - §3.2's server row and §3.13's first bullet read as §3.11 gives them, and §3.13's line-561 replacement applies as written;
  - Phase 7's exit gains the rows the server design's §16 question 8 lists;
  - S7.2's notes cover the form and the risk of forwarding an agent;
  - S7.3 runs one CLI network operation through the form against a disposable loopback `sshd` on Linux, and Phase 10's post-release check repeats it on one host with a disposable `sshd`.
- 2026-09-28: amended by the [server design](../../dev-docs/GwzCoreServerDesign.md) §8, accepted at SHA-256 `9fc80261…`, with the operator's sign-off.
  - §3.4's Linux probe is corrected: `STATX_ATTR_AUTOMOUNT` never marks an autofs trigger, so the file-system type check stays beside it.
  - §3.4's macOS probe is corrected: an `open`, whatever its flags, mounts a direct-map trigger, so `getattrlistat` probes first.
  - §3.4's `/net` sentence is corrected: macOS has shipped `/net` disabled since autofs-281.0.3, and the default auto_home map remains.
  - §3.6's macOS rows use the auto_home map through `/home`, plus a disposable runner with `/net` and a direct map enabled.
