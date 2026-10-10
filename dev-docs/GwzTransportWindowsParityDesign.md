# Windows transport parity (TR1.8)

Date: 2026-10-10 (replaces the 2026-10-03 text). Status: **DRAFT for plan step 2.4 (settle, review, GO); not frozen, accepted, implemented or release-qualified.** This is the one revision plan step 2.1 owes, written from [the delta](GwzTransportWindowsParityDesign-Delta.md) and the operator's decisions TD1 to TD14 of 2026-10-10 ([the decision list](GwzTransportTR18-OperatorDecisions.md); its "Operator discussion record" controls over its per-item narrative). No downstream Windows implementation may consume this as GO. The current design work changes no product source and preserves `endpoint_environment`'s `compile_error!`.

**Pending-row markers.** Three host transactions were approved on 2026-10-10 and their rows have not yet reported: TD9 (one-certificate trust, which unblocks Extended Protection and every HTTPS row), TD10 (the machine-proxy grammar) and TD12 (the bounded native-agent-service run). A clause that depends on such a row is written as decided and carries a marker of the form `[PENDING-ROW <id> (run 2026-10-10-tr18-*)]`. The marker is the upper-case word PENDING, a hyphen and the word ROW; searching for that string finds every one. The lane owner replaces each marker with the executed row citation when the evidence lands. A claim whose marker has not been replaced with an executed result when step 2.4 starts is **removed** and listed in section 14, never kept as a provisional clause (plan step 2.1; section 11). Section 14 also indexes where each pending row is used, so that filling a row updates every place it touches.

**What this revision does.**

- It carries the SSPI-only supersession list of 2026-10-03 into the text (sections 2, 8, 11) and retires the list.
- It carries MAIN's accepted amendments: helper timing, configuration view and SSH clock (sections 3, 5, 10); the `SetupClock` and the per-host `Supervisor`; WH1's accepted shape (the qualification cfg, `Owner::send_if`, the machine proxy admitted only when verified DIRECT).
- It records the plan's operator decisions of 2026-10-08 (OQ1 to OQ16, as recommended) as text, and the dispositions of [the proof-disposition note](GwzTransportWindowsProofDispositions-DRAFT.md) that the evidence supports (HOME, Pageant timeout ownership, window identity, weak certificate hashes).
- It changes the clauses that the 2026-10-10 evidence confirms, revises or contradicts (the delta's sections 2 and 7), by the decisions: the logon session answers `Negotiate` and `NTLM` and a helper answers `Basic` only; Digest is refused; discovery redirects are followed; a challenged POST is refused; native paths; the RSA-only limits with plain diagnostics; no cross-user refusal; the baseline pipe policy.
- It removes the claims whose rows stay unexecuted (section 14), and adds the migration-difference register (section 12) and a separate limitations list (section 13).

## 1. Authority and object

The transport release plan, amendment 2 (revision 8 DRAFT: §§3.5, 3.18 and 3.19 as reconciled by the skim package of revision 8), and V110 §2 and S4.1 control. OD15 places every parity mechanism inside the transport; OD16 permits default credentials to any host. TR1.6 revision 4's accepted helper contract and operator answers, and TR1.5's accepted setting contract, control their seams. The accepted [helper timing](GwzTransportCredentialHelperTimingAmendment.md), [configuration view](GwzTransportCredentialHelperConfigurationViewAmendment.md) and [SSH clock](GwzTransportSshHelperClockAmendment.md) amendments control their domains. The accepted [SSPI mechanism design](../../dev-docs/GwzSspiDesign.md) and its [acceptance](../../dev-docs/GwzSspiAcceptance.md) control the SSPI boundary (sections 2 and 8). The plan (`GwzTransportWindowsParityPlan.md`) section 8, decided 2026-10-08, and the decisions TD1 to TD14 above control this revision's choices. This document cannot reverse those decisions. Process: root AgentProcessRules, GwzProcessOptimization §§3.1, 4 and 8; review-loop's canonical prompts. Evidence locations follow root EVIDENCE.md, with V110 §2's explicit E: override for this campaign only.

**Evidence tuple.** The delta's read-only sources: workspace root `40090345`; gwz-core `eb924c0e`; gwz-transport `77f89cdf`; gwz-cli `21d62310`; gwz-py `5950ba38`; gwz-sspi `364ccc77`; git2-rs `d13951f7`; gwz-core-evidence `d7ca6a38` plus the uncommitted runs below. The root owner records the settled, committed tuple at step 2.4.

**Evidence runs** (private, `gwz-core-evidence/campaigns/transport-qualification/runs/`; access required; the public record is this document, the delta, the decision list and the [baseline](GwzTransportWindowsBaseline.md)). Row names are cited as `RS/x2-b-all-server-ed-kh`.

| Short name | Run | Content |
|---|---|---|
| **RS** | `2026-10-10-tr18-windows-ssh-rows` | 90 SSH rows on released Windows 1.0.17: B05 to B08, X1 to X10 |
| **RA** | `2026-10-10-tr18-windows-auth-rows` | 35 plain-HTTP rows: B09 (DIRECT machine), B11 to B15, B18; spikes on HTTPS trust |
| **RR** | `2026-10-10-tr18-redirect-parity` | the discovery-redirect question on macOS, Linux and Windows 1.0.17 |
| **RM** | `2026-10-10-tr18-b17-macos-linux` | B17 on macOS and Linux |
| 2026-10-03 runs | `...-tr1-8-windows`, `-primitives`, `-residual-primitives`, `-digest-workers`, `-tls-adapter` | the baseline's earlier rows, unchanged |

Released Windows 1.0.17 is `gwz.exe` SHA-256 `cbab5e9b...`.

**What the evidence is not.** Every Windows authentication row ran as **plain HTTP**, because 1.0.17 on Windows ignores `http.sslVerify=false` and `GIT_SSL_NO_VERIFY`, and trusting a fixture CA needs a trust change (TD9). A clause that leans on an HTTP row says so. The SSH rows used Windows' own `sshd.exe` 9.5 and Paramiko, not GitHub. The macOS binary is a local source build of 1.0.17 (`cf97fa15...`), not the release asset, and the Linux run is the released artifact on **aarch64** (the release target is x86-64).

Owned outputs: this design, its concise baseline record and proposed user guide (revised separately at step 2.1). No shared interface implementation, main/sibling edits or Git/GWZ mutation are owned here. S4.2 to S4.5, TR4.6 to TR4.10 and TR8.4 remain later work. TR4.10 receives its separate mandatory secret-handling dual gate; the Windows phase receives its aggregate review.

## 2. Runtime ownership

One runtime captures the environment, machine proxy configuration, local agent source and caller token identity before its first Open. The CLI command and the Python per-operation entry each take this capture at their existing snapshot point. Workers receive immutable owned values; they never reread process environment, WinHTTP configuration or agent selection during retry. No process-global mutable cache, thread-local state or credential cache is introduced. Unique mapping names and credential scopes use the runtime's `IdSource`. The connection-scoped `SetupClock` (`gwz-transport/src/pool/setup_clock.rs`) and the per-host `Supervisor` (`agent_job/supervisor.rs`) are runtime-owned, not global. Platform code lives in enclosing Windows modules or `cfg_if` blocks.

Capture owns configuration only. Network, `known_hosts` reads, agent request exchange and SSPI work execute in supervised endpoint jobs under Control. The machine proxy's WinHTTP-owned strings are copied into validated owned values and `GlobalFree`'d on every exit, partial failure included. Capture failure refuses the operation before an endpoint opens.

**SSPI runs in an owned fresh process** supervised under the existing Control deadline; configuration capture remains in core. No in-process native thread is used as a cancellation guarantee. The kill-on-close Job is attached at process creation. Forced exit guarantees contained resource ownership and reaping, not secret erasure or external-provider cancellation. A pending native call keeps its slot until the process, the Job and local I/O are confirmed gone. (This folds in the 2026-10-03 SSPI-only list for sections 2 and 8; the accepted design owns the mechanism.)

The future 1.2 server compares the client's token logon AuthenticationId at SessionOpen, per amendment §3.18. 1.1 does not implement server admission. Local Pageant and SSPI use the current caller's logon session, never an impersonated remote user. A default credential identity is not a password or an environment value.

Option A's background close (section 3), idle loss (`idle_watch.rs`, Unix-only today) and the adaptive-concurrency Phase 1 change no clause of this design; plan steps 1.5 and 3.7 carry their Windows parts.

## 3. SSH home and trust

**Home order.** Resolve from the captured Windows environment in libgit2's order: `HOME`, `HOMEDRIVE` concatenated with `HOMEPATH`, then `USERPROFILE` (RS/b04-home-unset-homedrive-homepath, b04-userprofile-only). An unset variable, or a candidate directory that does not exist, is skipped. The first existing directory is the home. Executed on 1.0.17 and kept:

- an **explicitly empty** `HOME` refuses, with a specific home diagnostic, even when a later candidate is good (RS/b04-empty-home-good-fallback). It is not treated as unset, which remains distinct and succeeds;
- an existing first home without a matching `known_hosts` entry refuses without trying the next home (RS/b04-first-existing-home-*);
- a **relative** `HOME` that exists under the process directory is used, and a nonexistent one is skipped (RS/b04-relative-home-exists-under-cwd, b04-relative-home-nonexistent-good-fallback). The transport resolves it against the **captured** current directory (OQ7(a)), so a later change of the process directory cannot redirect trust. Closure test: change the process directory after capture and the worker still reads the original fixture's `known_hosts`;
- a malformed path is refused with a specific home error before agent or network I/O.

**Native paths (TD6, applies on every platform).** The transport retains native paths. On Windows it uses the Unicode file APIs and does not guess the filesystem or terminal encoding, nor pass a Unicode filename through a narrow C file-opening interface. On Unix it retains native filename bytes through file operations rather than requiring valid UTF-8 or converting lossily. Display text and file-content encoding are separate from the path used to open a file. The demonstrated defect is Windows only: 1.0.17 selects a non-ASCII home as existing and then fails when `known_hosts` is opened, a narrow path inside libssh2 (RS/x10-existence-probe-*, x10-ansi-range-e-acute, x10-outside-ansi-*, x10-unicode-and-spaces; an ASCII junction to the same directory works, x10-junction-ascii-name-for-unicode-dir). The same effect refuses a key file under a non-ASCII directory (RS/x3-path-unicode-*). The transport accepts what 1.0.17 refuses (register item 1). That Linux and macOS conversions are already correct has **not** been audited: the SSH home and identity path conversions on those platforms are reviewed as part of the implementation steps, and the broader audit stays on the limitations list (section 13).

**One captured home.** `known_hosts` and `~/` identity paths resolve under that captured home (TD6 A). 1.0.17 expands `~/` in `--identity` from `USERPROFILE` while reading `known_hosts` from the `HOME`-first order (RS/x3-path-tilde-key-only-under-userprofile authenticates; ...-only-under-home is refused), so the transport differs whenever `HOME` and `USERPROFILE` differ (register item 6). Explicit absolute identities retain their existing behaviour. The same resolver must cover `SshEndpointConfig::from_environment` or retire that duplicate factory. The transport-setting global-config lookup (section 10) shares this home contract.

**Host-key algorithms and key files (TD5 A and B).** On Windows, 1.0.17 (libssh2 on WinCNG) supports RSA only. The transport uses the same libssh2 and WinCNG and keeps the same limits; no OpenSSL backend change is selected.

- A `known_hosts` entry of ed25519 or ecdsa type with no rsa entry fails before any connection on 1.0.17 (`failed to set hostkey preference: The requested method(s) are not currently supported`, RS/x2-a, x2-b, x2-d); an rsa line beside such lines works (x2-f, x2-h); a server offering no rsa key against an rsa entry fails in key exchange (x2-e). The transport's host-key preference list names `ssh-ed25519` (`ssh_network.rs:24`), which the same libssh2 strips; the preference list is filtered by what the session supports, or the attempt is refused naming the algorithm (step 3.8).
- File keys: only an unencrypted RSA key in **PEM** is offered; a new-format RSA key (the default `ssh-keygen -t rsa` output), ECDSA, ed25519 and a passphrase-protected key offer no key, and 1.0.17's error is the empty `failed to authenticate SSH session:` (RS/x3-type-*, x3-encrypted-rsa-pem).
- The transport replaces both opaque texts with plain ones that **name the cause and the fix**. For an ecdsa or ed25519-only `known_hosts` entry: add an rsa entry with `ssh-keyscan -t rsa`. For a key file that is not RSA PEM: the message says so and, for an RSA key, names `ssh-keygen -p -m PEM`; for other key types it names an agent as the way to use them, since agents sign all three types (section 4). Messages only; behaviour is unchanged (register item 8).
- CRLF `known_hosts` lines are accepted, CR-only lines are not (RS/x1-*).

Known-host validation remains mandatory before any authentication; no trust failure invokes native transport or another agent. The Windows network arm supplies waits to the connection's `SetupClock`, which owns the aggregate, stall and local phases (disabled stall with an enforced aggregate deadline included). `Control::begin_slice` still exists in the host driver, and the wording here is the clock's, not slices'. A discarded SSH connection is ended by `shutdown(Both)` (`ssh_connection.rs:44`), which is portable; plan step 1.5 proves it on Windows.

## 4. Agent source and local pipe

Select once: `FindWindowW(L"Pageant", L"Pageant")` in the host's desktop/logon session, as libssh2's Pageant-first backend; otherwise `SSH_AUTH_SOCK` from the snapshot; otherwise `\\.\pipe\openssh-ssh-agent`. "Visible" means the window discoverable by this protocol, not `IsWindowVisible`: Pageant's protocol window can be hidden. Both sources running selects Pageant. A Pageant pipe named in `SSH_AUTH_SOCK` is an ordinary selected pipe.

**Executed on 1.0.17 and kept:** Pageant wins when both run, and the pipe sees no connection (RS/b06-pageant-and-pipe-both-*); a Pageant window with no usable key refuses and never falls through to a working pipe (RS/x9-*, b06-...-only-pipe-key-authorized); the default pipe name is used when `SSH_AUTH_SOCK` is unset (RS/b08-default-pipe-name-served-by-owned-agent); with neither agent, no key is offered (B07). A UNC pipe name is not connected even when the pipe exists locally (RS/x8-ssh-auth-sock-unc-localhost-pipe-served); a MinGW Unix path, a file path and a UNC name all give 1.0.17 the generic no-agent text (RS/x8-*).

The source is an owned `AgentSource` enum, not a `PathBuf` that sometimes means a window. (It is a proposal: `AgentSource`, plan step 3.2a, has not landed.) Pageant's identity is the window handle, a pinned process handle and the process creation identity. Before each request confirm the window still belongs to that pinned process. A vanished or replaced Pageant refuses; it never selects the pipe.

**No ownership refusal (TD11 B).** The transport does **not** refuse a Pageant owned by another SID or logon session, and does not promise any cross-user or cross-logon-session refusal. Neither 1.0.17 nor libssh2 does such a check, and no second local account is approved, so the row (P01's cross-user negative) stays unexecuted and the claim is removed (section 14). The retained process and window checks above are proven. The Safety review at step 2.4 must judge this policy explicitly.

**Pipe policy (TD12; OQ4(a)).** Admit local native pipe names only; reject UNC remote servers, traversal and unsupported MinGW Unix socket forms before connect, naming `SSH_AUTH_SOCK`. Connect with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, which is libssh2's own connect (`agent_win.c`); it limits what a server can do with the caller's token and is not a check of who the server is. The transport applies **no server-identity restriction**: no check of the pipe server's SID or logon id, as on 1.0.17. The native OpenSSH Authentication Agent service therefore works as on 1.0.17 because nothing is checked. No SMB authentication may occur. The 2026-10-03 same-SID server-token rule is withdrawn (section 14); starting the service does not imply acceptance of it. The native-service combination is **not qualified** until the bounded real-service run reports (capture the service state and start type, start it for the qualification, restore both): `[PENDING-ROW B08-native-service (run 2026-10-10-tr18-*)]`. The owned-pipe rows already cover the code path (RS/b08-owned-pipe-rsa, b08-default-pipe-name-served-by-owned-agent).

**Agent keys and signatures (executed).** RSA keys sign at flag 4 (`rsa-sha2-512`) only, with no fall back; a server accepting only `ssh-rsa` or `rsa-sha2-256` is refused before any signature request (RS/x4-pipe-rsa-server-*). RSA, ECDSA and ed25519 agent keys all authenticate through Pageant and through an owned pipe agent (RS/x4-pageant-*, x4-pipe-*); ECDSA and ed25519 sign at flag 0. An ed25519 certificate from the agent authenticates; an RSA certificate does not (x4-pipe-*-certificate). An identity the agent declines to sign is skipped and the next one is used (x4-pipe-security-key-then-rsa, emulated). A real security-key signature and a Pageant-held certificate are **not qualified** (TD8; section 13).

Pipe I/O is overlapped, one request owner per handle, completion polled under Control. `CancelIoEx` targets that owner's `OVERLAPPED`. A cancelled request retains its buffer, `OVERLAPPED` and event until completion is observed, then closes handles. `ERROR_NOT_FOUND` is not evidence of completed I/O. Cleanup unconfirmed stays owned in the runtime cleanup aggregate; no detached thread owns borrowed data. Source disappearance, malformed reply and failed trust poison that channel. `ERROR_PIPE_BUSY` waits inside Control, not libssh2's fixed second.

## 5. Pageant exchange

Use `WM_COPYDATA`, `dwData` `0x804e50ba`, with a NUL-terminated ASCII mapping name. Create an 8192-byte pagefile-backed Local mapping, exclusive creation with collision refusal, owned by the caller. The name is `Local\PageantRequest-<pid>-<runtime-id>-<request-id>`. The PuTTY 0.83 spike accepts the Local/unique name, the caller+SYSTEM ACL and an RSA SHA-256 signature, and the owned name-collision refusal executed (baseline P01, partial). Grant only the caller SID and SYSTEM read/write; no Everyone access. The cross-user negative did not run and no claim rests on it (section 14).

The four-byte big-endian frame plus payload must fit 8192 bytes, so payload is at most 8188. Reject oversized requests before dispatch. Validate reply length 1..8188, message type, exact payload exhaustion and every signature/key bound using the existing agent codec; never copy 8192 bytes starting after the header. Unsupported keys are skipped in order under TR2.8. An empty list or no usable key says `Pageant has no key this transport can use`; a declined signature permits the next key, while malformed I/O ends the channel.

Each request has its own mapping and serialized owner. No concurrent caller writes one mapping. Use `SendMessageTimeoutW` to the pinned window, never `HWND_BROADCAST`, with `SMTO_BLOCK | SMTO_ERRORONEXIT` and without `SMTO_NOTIMEOUTIFNOTHUNG`. Same-process or same-input-queue windows are refused because the documented timeout is otherwise ignored. There is one send, no resend after a short timeout: a sign request may still be active in the receiver.

**Bound.** The request bound is the remaining allowance of the connection's `SetupClock` (its aggregate and stall bounds); Pageant confirmation does not reset those clocks. No independent 120-second agent allowance is introduced. Cancel may return while the supervised request retires; cleanup must be observed within the existing cleanup bound or reported unconfirmed.

**Receiver boundary (OQ5 (a), ProofDispositions §2).** The request runs in process, bounded by the remaining allowance; there is no isolated broker (a broker bounds GWZ's lifetime but cannot retire an external receiver). `SendMessageTimeoutW` returning does not prove the receiver stopped. The actual Pageant fixture kept the mapping alive after sender timeout and close until it was reaped, and a synthetic receiver wrote after the sender timed out. So GWZ retires only its own ownership: retain input storage until send completion, never read a mapping after a timeout, never reuse its name, never resend the timed-out signing request, and wipe on confirmed quiescence before unmap/close. It reports that the external prompt may continue, and never terminates an operator's Pageant to satisfy cleanup. Broker-style reaping and receiver quiescence are separate assertions, and only the first is made.

**Window identity (P02, ProofDispositions §3).** No retired numeric `HWND` was reused in the fixed 100,000-attempt/15-second budget; the reuse assertion stays UNEXECUTED and is not reported as a pass. The identity check before `SendMessageTimeoutW` is not atomic with the send, so no clause claims that reuse is prevented. The post-send owner verification rejects a result; it does not undo what a replacement receiver could have obtained from the request. Section 14 lists this revision.

Mapping contents, agent comments, key bytes and signatures never reach logs, failure text or evidence. Counts, algorithm names, opaque source IDs and redacted result classes suffice. A Pageant request timeout names the bound and the confirmation/key action, without suggesting fallback happened. The RS run supplies a reusable Pageant inspector (`runner/pageant_client.py`) and an unencrypted PPK v3 writer for RSA, ECDSA and ed25519 (`runner/ppklib.py`).

## 6. WinHTTP machine proxy

Read `WinHttpGetDefaultProxyConfiguration` once beside capture, for both runtime entries. On Windows this machine setting wins. `HTTP_PROXY`, `http_proxy`, `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`, `all_proxy`, `NO_PROXY`, `no_proxy` and `http.proxy` in git configuration are ignored, **measured** on 1.0.17: a proxy configured by any of them saw zero requests, and only `netsh winhttp` settings reach WinHTTP (RA/b09-*); a numeric loopback origin connects directly (RA/b09-gitconfig-http-proxy-numeric-loopback-origin). The user guide states this inverse of Unix behaviour. Do not merge bypass lists from ignored variables. MAIN today admits the machine proxy only when it is verified DIRECT (`endpoint_environment.rs:203-282`) and refuses any other; plan steps 4.6 and 4.7 widen it as below.

A machine setting of WinHTTP access type `NO_PROXY` (DIRECT) means direct: executed (RA/b09-*). **The grammar below is decided (TD10 A; OQ11 (a)); its rows have not reported.**

- NAMED_PROXY admits a bare `host[:port]` (default port 80), or a semicolon-separated scheme mapping with a single applicable HTTPS entry. Bracketed IPv6 is parsed explicitly. Reject ambiguous multiple applicable proxies, malformed ports, userinfo, URL paths, PAC/automatic forms and unsupported schemes before any Open, naming the WinHTTP machine proxy. The default-proxy API does not stand for the user's browser or PAC settings. `[PENDING-ROW B09-P04-grammar (run 2026-10-10-tr18-*)]`
- Bypass is semicolon-delimited. `<local>` matches a hostname without a dot, not every RFC1918 address. Literal hosts and addresses compare without ASCII case; `*` is a glob within the complete host pattern, never URL/path matching. Matching uses the request's canonical destination host after discovery redirect; CONNECT uses the same destination and chosen proxy. No DNS lookup expands `<local>`. Reject bypass syntax the implementation cannot interpret; do not silently send direct on a parser failure. Wildcards, trailing dots, ports and IPv6 are specified by the executed counterexamples. `[PENDING-ROW B09-P04-bypass (run 2026-10-10-tr18-*)]`
- The released native fixture used a named machine proxy for a reserved `.invalid` origin even with a conflicting `HTTPS_PROXY`, and both tested numeric loopback origins bypassed it despite an empty configured bypass list (2026-10-03). The transport reproduces that implicit loopback bypass; the two addresses do not prove the whole native bypass grammar.
- The candidate snapshot is immutable once captured; the product test is plan step 4.6's.
- A form with no executed row when step 2.4 starts is refused before any open, naming the machine proxy (OQ11's fallback), and listed in section 14.

**407.** The transport refuses every 407 from a machine proxy before generating any token, naming the machine setting and the off switch, and offers neither a helper credential nor the login. 1.0.17 refuses it the same way: Negotiate and NTLM 407 reached the proxy once, offered no credentials and failed with the native string-conversion error (B10, 2026-10-03). No proxy credential mechanism is designed. Origin Authorization never becomes Proxy-Authorization. A proxy set through `http.proxy` is moot, since it is never contacted.

The machine-proxy rows run serialized in a quiet window: booking lock, the exact prior configuration saved, a deadline guard armed before each mutation, and restoration verified after every row (`reset proxy` alone restores only an initially direct machine).

## 7. Origin authentication choice

Initial discovery is anonymous. Retain its final challenged HTTPS repository base U and complete challenge set after the validated discovery redirects. Parse each field's first scheme token without case. No origin credential accompanies CONNECT. Certificate trust is validated before any origin authentication token is generated.

**Discovery redirects are followed on Windows (TD3 A)**, as on macOS and Linux (RR: 16 of 16). Windows 1.0.17 follows none: it repeats the original request 15 times and ends `too many redirects or authentication replays` (RR 0 of 8; RA/b15-redirect-* 0 of 5; Git for Windows follows the same server). This is a deliberate difference (register item 4), and a redirect row is a transport-only test. Credentials never follow a redirect to another origin (TR1.6 OQ4). The evidence is HTTP only: `[PENDING-ROW B15-https-redirect (run 2026-10-10-tr18-*)]`.

**Which identity answers (TD1 A, TD2 A).** The transport matches 1.0.17's measured choice, in this order:

1. If the challenge offers `Negotiate` or `NTLM`, alone or beside `Basic` or `Digest`, the **logon session** answers, `Negotiate` first and `NTLM` second, and no helper is asked even when one is configured and able to answer (RA/b11-negotiate-only-helper-configured, b12-ntlm-only-helper-configured, b13-negotiate-and-basic-helper-configured, b13-ntlm-and-basic-helper-configured, b14-digest-and-negotiate-no-helper: helper calls 0). This is not a Basic-only restriction.
2. Otherwise, if `Basic` is the only workable scheme, a configured helper (TR1.6) is asked once for U, and a usable helper identity answers over `Basic` (RA/b13-basic-only-helper-configured). GitHub token authentication stays this path.
3. **Digest is never answered**, as on 1.0.17 (it repeats the unauthenticated request to the cap, never sends an `Authorization` header, even beside Basic: RA/b14-digest-only-helper-configured, b14-digest-and-basic-helper-configured). A challenge whose only workable scheme is Digest is refused at once, naming Digest and the off switch. Digest is ignored when another supported scheme is offered, so `Digest, Basic` uses `Basic` through the helper (1.0.17 fails that case: register item 2).
4. No supported challenge yields the existing unsupported-authentication message, naming offered scheme names as untrusted quoted text and the native setting hint.

Disabled disables helpers only; default credentials remain allowed, as 1.0.17's callback. No credential, invalid helper output, missing git and unstartable git count as no helper identity (TR1.6 §4); with only `Basic` offered that ends in the message of step 4.

**Rejection is terminal (TD1).** If the server rejects the logon identity, the operation ends with an error naming the scheme. The transport does not replay that identity (1.0.17 retries a Negotiate-only logon identity to a cap of 15: RA/b11-negotiate-only-logon-rejected-helper-configured) and does not fall back to a helper credential (register item 3). A helper timeout or cancel ends the request; it never starts default credentials. A rejected Basic helper identity is terminal under TR1.6 §7; the transport tries no other identity, scheme or helper. Rejection means the server's 401 after the exchange's final token, not an intermediate round of a multi-round exchange.

**NTLM and Kerberos are distinct protocols.** `Negotiate` selects a mechanism. The successful fixture exchanges used NTLM, so they do not qualify Kerberos (section 13).

**OD16 has no zone test** (observed on HTTP): NTLM and Negotiate with the logon session authenticate to an Intranet name, a dotted Internet-zone name and a numeric loopback address (RA/b15-direct-*). Any host that challenges receives the logon session's response; this is an accepted hazard, not a finding that may reimpose a zone boundary. Redirects during the authenticated exchange end it with no credential sent to the new location; TR1.6 OQ4 (a) controls authenticated Basic redirects; SSPI's connection-bound exchange does not reuse an old context at a new URL.

**HTTPS.** The scheme choice happens before TLS in libgit2's WinHTTP code, so a different result over HTTPS is not expected, but it is not proven until the HTTPS variants run: `[PENDING-ROW B11-B15-https (run 2026-10-10-tr18-*)]`.

## 8. SSPI

`AcquireCredentialsHandleW` with `SECPKG_CRED_OUTBOUND` selects Negotiate or NTLM. The identity is always the **default**: NULL auth data, the current logon session only. No explicit identity (`SEC_WINNT_AUTH_IDENTITY_W`) enters SSPI under this design, since a helper identity answers `Basic` only (section 7). No password, token or OS error string is logged.

For Negotiate/NTLM the target is `HTTP/<canonical U host>` without port or path, brackets removed for IPv6; preserve U's host, not a reverse-DNS alias. Request `ISC_REQ_CONNECTION | ISC_REQ_ALLOCATE_MEMORY`, no delegation and no required mutual-auth flag that would remove NTLM parity. Check status and returned attributes instead of assuming requested attributes were granted. `SEC_I_CONTINUE_NEEDED` continues on the same lease. `SEC_I_COMPLETE_*` requires `CompleteAuthToken` before using output. Other status values fail explicitly. Bound every token by `QuerySecurityPackageInfo`'s maximum and the HTTP header bound; bound the exchange to eight request rounds and the existing setup clocks. SPNEGO carries NTLM, the connection is authenticated once and the POST follows on it with no further `Authorization` header (RA/b11-negotiate-only-no-helper). The SPN is not captured for the logon exchange in the HTTP rows (`ntlm_spn` is null), so the target-name and flags clauses rest on the 2026-10-03 SSPI rows.

**Channel binding.** Pass `SECBUFFER_CHANNEL_BINDINGS` from the actual verified TLS connection: `SEC_CHANNEL_BINDINGS` with application data `tls-server-end-point:` plus the RFC 5929 certificate digest (weak signature hashes upgraded to SHA-256). All offsets and lengths are checked. Do not use the configured CA certificate, the original redirect host's certificate or bytes from a prior connection. Capture the private binding outcome immediately after the final origin TLS handshake in `https_connection::connect`, before the concrete stream is wrapped for Hyper. The pinned `tokio-native-tls` 0.3.1 stream exposes the `native-tls` 0.2.18 stream through `get_ref()`; its `tls_server_end_point()` queries the actual peer through Schannel 0.1.29 and returns the digest without the prefix. Use this existing method, not a new certificate parser. Accept only the expected bounded 32/48/64-byte digest lengths; prepend the 21-byte binding prefix once when constructing the checked layout. Retain this owned outcome on that physical connection. It is not a hostname cache or a unique connection identifier. With MAIN's one `native_tls::TlsConnector` per endpoint (`SharedTls`, `https_tls.rs`), the query is still per physical connection (it asks the stream's peer, not the connector), and step 4.8's test must hold with the shared connector.

For a TLS proxy, capture from the final origin handshake after CONNECT, not from the preceding proxy TLS stream. A new connection after retry or discovery redirect captures a new outcome even if the peer digest happens to match. Native `None` or error refuses an SSPI authentication attempt before any offer, and the operation ends with an error; never substitute an empty binding or silently omit it. An anonymous or Basic exchange does not need this binding and is not refused merely for its unavailability. Initial extraction is not a claim of lifetime peer immutability; peer change or renegotiation is resolved against the connection's binding invariant.

**Weak certificate hashes (P06).** Native TLS refusal of MD5-signed peers is kept (both native MD5 handshakes failed). The RFC 5929 §4.1 rule (MD5 and SHA-1 select SHA-256) is a pure DER/OID test, independent of any handshake, and is not a claim of a successful MD5 handshake. The bounded native Rust probe proves two distinct verified origin digests, untrusted or wrong-name refusal before binding query or application write, and distinct proxy and origin bindings through nested TLS and CONNECT, with connector-local roots only.

**Extended Protection (EPA) is a decided requirement and awaits its rows.** A server that requires Extended Protection authenticates with the binding above: `[PENDING-ROW B16 (run 2026-10-10-tr18-*)]`. The negative controls, a missing binding and a wrong binding, are refused: `[PENDING-ROW P06-EPA (run 2026-10-10-tr18-*)]`. The absence of baseline EPA evidence does not establish that the transport fails EPA: the [WH1 native refresh](../../dev-docs/GwzWindowsHttpsIntegrationImplementation-Verdict-3.md) already records integrated CLI and installed-wheel Git operations and wrong-CBT rejection under its limited qualification route. That does not complete B16 or authorize ordinary Windows activation.

**Digest** has no clause: it is never answered (section 7). WDigest, the Digest input buffers and the helper-identity Digest path are removed with it; the 2026-10-03 WDigest acquire refusals are irrelevant to the release.

Each context has one owner, no concurrent `InitializeSecurityContext` calls. `FreeContextBuffer` after copying/wiping each provider output; `DeleteSecurityContext` and `FreeCredentialsHandle` exactly once on all exits. Network waits follow Control. Normal completion disposes initialized native handles once. SSPI blocks in OS/provider work only inside the owned process of section 2, so no clause rests on cancelling a thread (P05 needs no execution).

## 9. Connections, retries and POST

The challenged connection is exclusively leased to the operation throughout Negotiate/NTLM; one connection per exchange (observed). Consume a bounded 401 body before sending the next request; connection close, framing error or redirect discards it and its context. Never continue an exchange on a new connection with an old context or token. No HTTP multiplexing or another route shares that lease.

A credential-bearing connection receives TR1.6's opaque route scope: same operation, original route/service and pinned U only. Default credentials also mint a route scope; matching a logon SID alone cannot allow cross-route reuse. Authenticated idle connections remain subject to pool caps/eviction and are discarded at route retirement. Anonymous connections never inherit the scope. Basic retains the same credential-route isolation.

Rejected authentication is terminal, as TR1.6 §7 and section 7. Actual network loss can use TR2.1's per-key setup retry only before effect and within its attempt bound; every new attempt creates a new SSPI context and remains on the same captured identity and source. Never retry a partial or complete POST to acquire credentials.

**A 401 answering a POST (TD4 A).** On 1.0.17 the discovery GET succeeds and the POST is challenged; with Basic the first POST carries its 129-byte body and the retried POST carries **0 bytes** (the clone fails: `could not read from remote repository`), and with NTLM or Negotiate the client stops with `failed to receive response: The request must be resent` (RA/b18-*). The transport **refuses a challenged POST** with a clear diagnostic and never replays its body (register item 5). The outcome is the same as 1.0.17's (a failure); the message is the transport's own. The case needs a server that challenges POST but not discovery and is rare. HTTPS: `[PENDING-ROW B18-https (run 2026-10-10-tr18-*)]`.

**SSH drops.** A pre-authentication SSH drop is one attempt in 1.0.17 (RS/x7-*: 1 connection per drop shape); the transport's TR2.1 retry is a deliberate difference (register item 7). Windows' texts (`Failed getting banner`, `Failed sending banner`, `Unable to exchange encryption keys`; `sshd.exe` implements MaxStartups) map a pre-banner drop to the retriable `Io` class (step 3.7).

## 10. Helper executable and setting seams

Windows git discovery searches captured absolute `PATH` entries for `git.exe`; do not implicitly search the current directory or append a shell script extension. Paths with spaces are passed as an executable path, not a shell command. git receives the captured environment and TR1.6's exact stdin and limits, including accepted GUI prompt policy. It owns configured helper parsing: do not implement credential helper shell forms a second time. TR2.2's direct `gh` executable and `!gh auth git-credential` forms remain test rows while that old path is retired. Under section 7 a helper answers `Basic` over HTTPS and an SSH password; it is never asked for a `Negotiate` or `NTLM` challenge.

**Timing (accepted amendment, TR2.22).** The helper interaction allowance is the amendment's named allowance; local helper work is excluded from network clocks; an explicit HTTPS username is carried as the amendment says; its messages M4 and M10 are the user-visible wording. This design's "a helper timeout or cancel ends the request" stands. The user guide's helper time bound cites the amendment, not a number of its own.

**Configuration view (accepted amendment).** Helper lookup reads a flattened, unconditional view of Git's configuration that Git itself parses, held for the process. The amendment's text says "No Windows mechanism is designed" (`...ConfigurationViewAmendment.md:142`), and **this design does not design one**. The open pieces, Git for Windows' system/global/XDG discovery, which environment variables a Git child needs once `env_clear` applies on Windows, the captured `HOME` and relative includes, belong to the WH2 contract of plan step 4.1, to which this section points once that contract is accepted. The global-config lookup that finds `gwz.transport` shares section 3's home contract.

**Observed on 1.0.17 (the parity target for plan step 4.5).** Shell-form helpers and `git-credential-<name>` scripts both run for an SSH password and authenticate when Git is on `PATH`; with Git not on `PATH` the helper does not run and the attempt is refused (RS/x6-*). A URL password is offered and accepted, even when the server also lists `publickey` and there is no agent; a wrong URL password is one attempt (RS/x5-*).

Start git suspended, assign it to a kill-on-close Job Object before resume, and on timeout/cancel kill and wait for the job, including child helpers. Assignment failure refuses before resume. Breakaway is not enabled. Pipe buffers stay owned until asynchronous I/O and process cleanup complete. The Job Object and path claims rest on P08 (executed 2026-10-03; the helper parser integration is separate). External/nested-job and GUI-helper behaviour are primitive proof rows, including Python's caller.

TR1.5's three forms work on Windows unchanged: flag, captured `GWZ_TRANSPORT`, user-global git `gwz.transport`; precedence flag > environment > global > default `gwz`. Each native form prevents runtime construction. Repository and local values remain ignored under TR1.5. Removal uses its lifecycle pairs, not a new Windows settings file.

## 11. Freeze and implementation sequence

The GO rule is unchanged: every baseline row B01 to B18 and primitive row P01 to P08 needs an executed result with exact inputs, toolchain and binary hashes, or a recorded disposition, and every provisional clause is replaced by one physically proved design. Inventory or a documented API is not executed evidence. **A row that cannot run discharges this rule by having its claim removed and listed** (section 14), never by leaving a provisional clause. The disposition for every row follows. Status key: **E** executed, **P** partial, **D** dispositioned without execution; a pending marker means the row was approved on 2026-10-10 and has not reported.

| Row | Status | Evidence | Disposition for this design |
|---|---|---|---|
| B01, B02 | E | 2026-10-03 | unchanged |
| B03 | E | 2026-10-03; RS/b04-home-unset-homedrive-homepath, b04-userprofile-only | stands (section 3) |
| B04 | E | 2026-10-03; RS/b04-*, x10-* | OQ7(a); Unicode cause is the `known_hosts` open; relative-existing case measured (section 3) |
| B05 | E | 2026-10-03; RS/b05-pageant-rsa-sha2 | stands (RSA at flag 4) |
| B06 | E | RS/b06-* | Pageant wins, the owned pipe sees no connection (section 4) |
| B07 | E | 2026-10-03; RS/b07-neither-agent-control | stands |
| B08 | P | RS/b08-* (owned pipe, default pipe name, nonexistent pipe) | owned-pipe selection executed; native service: `[PENDING-ROW B08-native-service (run 2026-10-10-tr18-*)]` |
| B09 | P | 2026-10-03; RA/b09-* (DIRECT machine) | env and git-config proxies ignored on DIRECT; machine grammar: `[PENDING-ROW B09-P04-grammar (run 2026-10-10-tr18-*)]` |
| B10 | E | 2026-10-03 | machine-proxy 407 refused with zero offers; stands |
| B11 to B14 | E (HTTP) | RA/b11-* to b14-* | logon session wins, helper only for Basic, Digest never answered; HTTPS: `[PENDING-ROW B11-B15-https (run 2026-10-10-tr18-*)]` |
| B15 | P | 2026-10-03 (zones); RA/b15-*, RR | no zone bound (HTTP); redirects not followed by 1.0.17 on Windows; HTTPS redirect: `[PENDING-ROW B15-https-redirect (run 2026-10-10-tr18-*)]` |
| B16 | pending | RA/spikes/spike4 (needs native trust) | `[PENDING-ROW B16 (run 2026-10-10-tr18-*)]` |
| B17 | E (Linux HTTPS and HTTP, Mac HTTP) | RM | Negotiate refused (`'Negotiate' authentication is not supported`), no `Authorization` header; NTLM-only `could not acquire credentials`; **the Mac trusted-HTTPS row stays unexecuted** (TD7 B) and the baseline question is disposed by inference (section 13) |
| B18 | E (HTTP) | RA/b18-* | POST challenge fails on 1.0.17; the transport refuses; HTTPS: `[PENDING-ROW B18-https (run 2026-10-10-tr18-*)]` |
| P01 | D | needs a second local account | claim removed (TD11 B; section 14) |
| P02 | D | 2026-10-03; ProofDispositions §3 | stays UNEXECUTED; claim revised (section 5) |
| P03 | P | needs the `ssh-agent` service started | server-token rule withdrawn (OQ4(a)); native service: `[PENDING-ROW B08-native-service (run 2026-10-10-tr18-*)]` |
| P04 | P | 2026-10-03; RA/b09-* | capture executed; grammar pending as B09; the immutable-snapshot clause is step 4.6's product test |
| P05 | D | 2026-10-03; sections 2 and 8 | in-process cancellation is no design need; SSPI runs in the owned process |
| P06 | P | 2026-10-03; ProofDispositions §4 | MD5 native refusal kept; RFC 5929 §4.1 rule is a DER/OID test; EPA controls: `[PENDING-ROW P06-EPA (run 2026-10-10-tr18-*)]`; adapter lifetime is step 4.9's product test |
| P07 | D | RA/b14-* | closed at product level: 1.0.17 never offers Digest |
| P08 | E | 2026-10-03 | stands |
| X1 to X10 | E (X4 P) | RS | sections 3, 4, 9, 10; X4's real security key and Pageant-held certificate are not qualified (TD8) |

Settle through the root owner; record the committed root/core/dependency tuple. Required independent Consistency and Safety reviewers receive the same exact document and controlling graph, canonical review prompts and report paths, plus **(a) section 14's list of removed claims** and **(b) this section's reading of the rule** (a row that cannot run discharges it by removal and listing). If the reviewers read the rule as requiring an executed result for every row, the fallback is an amendment of the rule by the lane owner or the operator, never a provisional clause. They also check that section 10's helper paragraphs and step 4.1's accepted contract agree, and the Safety review judges the no-ownership-refusal and no-server-identity policies of section 4 explicitly. Surface receives only the proposed user guide and existing help/docs, never this design or implementation. File reports verbatim; no self-GO, no dirty review. All P0/P1/P2 block. One merged remediation patch, same reviewers' original counterexamples, at most two architectural remediation rounds.

After design GO: S4.2 network/trust, S4.3 pipe, S4.4 home/assembly, TR4.8 Pageant and TR4.9 proxy, TR4.10 secrets with its dual gate, then S4.5 integrated candidate and ordinary Windows CI/review/parity batch. Dependencies come from the amendment, not this list's typography. Each cohesive source file stays under 500 lines; tests are reported separately. Disabled Windows/Unix branches must pass fast syntax-aware scope checks. Do not remove the deliberate platform `compile_error!` until its complete Windows implementation is ready.

## 12. Migration-difference register (TD13)

The settled choices approve these intended differences from released Windows 1.0.17, for the migration notes (S7.2 (1.1.0)). They are migration-note requirements, not claims that the changes are implemented or qualified.

| # | Difference | 1.0.17 | The transport | Decision, section |
|---|---|---|---|---|
| 1 | Non-ASCII `HOME` and key paths work | selects the home, then fails opening `known_hosts` or the key | native Unicode file APIs | TD6, 3 |
| 2 | Digest | Digest-only: 15 replays, then error; `Digest, Basic` fails the same way | Digest-only fails at once; Digest beside a supported scheme is ignored and the supported scheme is used | TD2, 7 |
| 3 | A rejected logon identity | Negotiate-only retries it to a cap of 15; NTLM-only and Negotiate+Basic consult the helper on each replay (the helper exchange never completed on the fixture) | the operation ends with an error naming the scheme: no replay loop and no helper consulted | TD1, 7 |
| 4 | Discovery redirects | none followed on Windows | followed, as on macOS and Linux; credentials never cross origins | TD3, 7 |
| 5 | A challenged POST | fails (empty-body replay, or `The request must be resent`) | refused with a plain message; body never replayed | TD4, 9 |
| 6 | `~/` in `--identity` | expanded from `USERPROFILE` | the same captured home as `known_hosts` | TD6, 3 |
| 7 | A pre-authentication SSH drop | one attempt | retried under the retry plan (TR2.1) | 9 |
| 8 | Messages | `failed to set hostkey preference`, an empty `failed to authenticate SSH session:`, the shared no-agent text for unsupported `SSH_AUTH_SOCK` forms | plain texts naming the cause and the fix, and naming `SSH_AUTH_SOCK` | TD5, 3, 4 |

**Unchanged by choice (not deviations).** Environment and git-config proxies are ignored on Windows. The logon session's credentials go to any host that challenges (OD16). RSA-only host keys and unencrypted RSA-PEM file keys. RSA agent signatures at `rsa-sha2-512` only. Agent-key support for ECDSA and Ed25519 is separate from these limits. A cross-user refusal is not added. The pipe server is not identity-checked.

## 13. Limitations (not qualified)

A separate list from the register. Each is a combination or claim that this design does not qualify; none is declared unsupported unless it says so.

| # | Not qualified or limited | Why | Section |
|---|---|---|---|
| L1 | Kerberos | the fixture exchanges used NTLM through Negotiate; no domain; the OS provider may work when domain-joined, unverified here | 7 |
| L2 | Real hardware security-key signing | no hardware; the emulated declining-identity skip is executed | 4 |
| L3 | Pageant-held certificates | needs puttygen's GUI; an ed25519 certificate from a pipe agent is executed, an RSA certificate does not authenticate | 4 |
| L4 | The native OpenSSH Authentication Agent service combination | the bounded real-service run has not reported (pending in section 4) | 4 |
| L5 | Mac trusted-HTTPS B17 | unexecuted by decision (TD7 B); disposed by inference from Mac HTTP, Linux trusted HTTPS and the untrusted-HTTPS control; the Mac binary is a local build and the Linux run is aarch64 | 11 |
| L6 | HTTPS variants of the authentication rows | all Windows rows ran on HTTP (pending in sections 7 and 9) | 7, 9 |
| L7 | Extended Protection / channel binding against a real EPA server | pending in section 8; the WH1 limited qualification is separate evidence | 8 |
| L8 | Machine-proxy forms beyond the executed rows | pending in section 6; a form without an executed row is refused | 6 |
| L9 | Cross-user and cross-logon-session refusal of a found Pageant | not promised (TD11 B); Safety judges the policy | 4 |
| L10 | Prevention of numeric `HWND` reuse | UNEXECUTED; no claim of prevention | 5 |
| L11 | A helper-identity exchange after a rejected logon identity | moot; the helper is not consulted | 7 |
| L12 | Host keys other than RSA, and file keys other than unencrypted RSA PEM, on Windows | WinCNG; unchanged by choice, with plain diagnostics | 3 |
| L13 | RSA agent signatures other than `rsa-sha2-512`, and RSA agent certificates | libssh2's flag-4 signing; unchanged by choice | 4 |
| L14 | Native-path conversions on Linux and macOS | not audited; the SSH home and identity conversions are reviewed in the implementation steps, the broader audit is outstanding | 3 |
| L15 | The target name (SPN) of the logon exchange | not captured in the HTTP rows | 8 |
| L16 | Proxy authentication | every 407 is refused | 6 |

## 14. Removed and revised claims, and the pending-row index

**Removed or revised claims (input (a) to step 2.4's reviewers).**

1. Section 4 (2026-10-03): "a found Pageant owned by another SID or logon session refuses" and section 5's "cross-user Pageant is refused before creation" (P01). Removed (TD11 B); the process and window checks remain.
2. Section 4 (2026-10-03): "the pipe server process token has the caller's SID and logon AuthenticationId" (P03). Withdrawn (TD12; OQ4(a)); replaced by the baseline local-pipe policy with no server-identity check.
3. Section 5: that the identity check prevents numeric `HWND` reuse (P02). Revised to the non-atomic check and no prevention claim.
4. Section 5: "an isolated request broker design is required before GO". Replaced by OQ5 (a) and the receiver boundary.
5. Section 7 (2026-10-03): helper-first precedence over `Negotiate`, `NTLM`, `Digest`, `Basic`; "once a usable helper identity was sent and rejected, never try the logon identity" (no evidence either way, and the helper no longer feeds SSPI). **Contradicted** by RA, changed by TD1.
6. Section 8 (2026-10-03): the Digest paragraph, WDigest input buffers, explicit-identity SSPI and its zeroizing identity owners. Removed (TD1, TD2). Plan step 4.8's explicit-identity residual therefore has no caller.
7. Section 7 (2026-10-03): "redirects change U before lookup" as 1.0.17 behaviour. 1.0.17 follows none on Windows; kept as a transport clause, a deliberate difference (TD3).
8. Section 8 (2026-10-03): the in-process cancellation language of the last paragraph. Replaced by the owned process (section 2).
9. Section 3 (2026-10-03): "a selected relative path is refused" and "empty or unavailable candidates are skipped". Contradicted (relative-existing is used; an explicit empty `HOME` refuses); revised.
10. No claim is made for Kerberos, a real security key, a Pageant-held certificate, a Mac trusted-HTTPS result, or a helper-identity exchange after a rejected logon identity (section 13).

**Claims kept, awaiting rows.** If a pending row does not report an executed result before step 2.4, its claim is removed here: native-service qualification (B08), the machine-proxy grammar beyond DIRECT (B09/P04), the HTTPS variants of B11 to B15 and B18, the HTTPS discovery redirect (B15), Extended Protection and its negative controls (B16, P06). A form with no grammar row is refused before any open (OQ11).

**Pending-row index** (where each row is used; update every place when the row reports):

| Row id | Sections |
|---|---|
| B08-native-service | 4, 11 (B08, P03), 13 (L4) |
| B09-P04-grammar | 6, 11 (B09, P04), 13 (L8) |
| B09-P04-bypass | 6, 13 (L8) |
| B11-B15-https | 7, 11 (B11 to B14), 13 (L6) |
| B15-https-redirect | 7, 11 (B15), 13 (L6) |
| B16 | 8, 11 (B16), 13 (L7) |
| P06-EPA | 8, 11 (P06), 13 (L7) |
| B18-https | 9, 11 (B18), 13 (L6) |

## 15. Primary API references

- [SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw): timeout flags and same-queue exception.
- [WinHttpGetDefaultProxyConfiguration](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpgetdefaultproxyconfiguration): machine configuration and allocation ownership.
- [SEC_CHANNEL_BINDINGS](https://learn.microsoft.com/en-us/windows/win32/api/sspi/ns-sspi-sec_channel_bindings): layout and endpoint-binding form.
- libssh2 1.11.1 `agent.c:336-440` (registry libssh2-sys 0.3.2) gives the Pageant 8192-byte window protocol and source order. libssh2 on WinCNG supports RSA only (`wincng.h` defines no ed25519; the evidence adds ECDSA); the OpenSSH backend of `agent_win.c` does not connect to a UNC pipe name here; its RSA agent signature uses flag 4 only. libgit2 at the baseline `winhttp.c:139-212, 618-640, 830` and `1243-1262` gives scheme, default and proxy behaviour, and `sysdir.c:328-330` gives home candidates. These are static references; the choice of identity in section 7 rests on the measured rows, not on the source reading.
