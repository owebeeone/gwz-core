# TR1.8 refreshed against MAIN: the delta (plan step 2.1)

Date: 2026-10-10. Status: **a delta document, not a design revision.** It lists what MAIN has changed under `GwzTransportWindowsParityDesign.md`
(the 2026-10-03 DRAFT, NO-GO), what the new 1.0.17 evidence (steps 2.2, 2.3 and 2.3b, run 2026-10-10) confirms, revises or contradicts, and the
disposition each baseline row now has. It changes no clause of the design, the baseline, the user guide or the amendments, and it decides nothing:
most of the revisions it proposes wait on the operator's answers in [`GwzTransportTR18-OperatorDecisions.md`](GwzTransportTR18-OperatorDecisions.md)
(TD1 to TD14, cited below; the numbering is final). After those answers, one revision of the design is written from this delta and goes to step 2.4. Nothing here authorizes
implementation, a commit, a push or a release.

Read-only sources: workspace root `40090345`; gwz-core `eb924c0e`; gwz-transport `77f89cdf`; gwz-cli `21d62310`; gwz-py `5950ba38`; gwz-sspi `364ccc77`;
git2-rs `d13951f7`; gwz-core-evidence `d7ca6a38` plus this lane's uncommitted runs. `file:line` references are to that tuple.

Evidence runs, all in the private `gwz-core-evidence/campaigns/transport-qualification/runs/` (private access required; the public record is this document and the baseline):

| Short name | Run | Content |
|---|---|---|
| **RS** | `2026-10-10-tr18-windows-ssh-rows` | step 2.2: 90 rows on released Windows 1.0.17 (`cbab5e9b...`): B05 to B08, X1 to X10 |
| **RA** | `2026-10-10-tr18-windows-auth-rows` | step 2.3: 35 plain-HTTP rows: B09 (DIRECT machine), B11 to B15, B18; spikes on HTTPS trust |
| **RR** | `2026-10-10-tr18-redirect-parity` | the redirect question on macOS, Linux and Windows 1.0.17 |
| **RM** | `2026-10-10-tr18-b17-macos-linux` | step 2.3b: B17 on macOS and Linux |
| 2026-10-03 runs | `...-tr1-8-windows`, `-primitives`, `-residual-primitives`, `-digest-workers`, `-tls-adapter` | the baseline's earlier rows, unchanged |

## 1. What MAIN has changed under the design

| # | Change on MAIN | Where | Design clause it touches | Effect on the design |
|---|---|---|---|---|
| M1 | The helper timing amendment (TR2.22) is accepted: the interaction allowance is named, local helper work is excluded from network clocks, an explicit HTTPS username carrier, messages M4 and M10 | `GwzTransportCredentialHelperTimingAmendment.md` (status: accepted contract, implementation pending) | section 7 (helper timeout), section 10 | Section 10's helper paragraphs and the user guide's "120-second bound" must cite the amendment's allowance and its M4/M10 wording; section 7's "a helper timeout or cancel ends the request" stays. Section 5's "no independent 120-second agent allowance" is about Pageant and stays. |
| M2 | The configuration-view mechanism is accepted: helper lookup reads a flattened, unconditional view of Git's configuration that Git itself parses, held for the process; its text says "No Windows mechanism is designed" (`...ConfigurationViewAmendment.md:142`) | that amendment | section 10 ("git owns configured helper parsing"), section 3 (the global-config lookup) | **Section 10 gains a Windows mechanism or states that the view is built the same way on Windows.** Git for Windows' system/global/XDG discovery, `env_clear` on Windows (which variables a Git child needs), captured HOME and relative includes are the open pieces; step 4.1's WH2 contract owns them. |
| M3 | The SSH helper clock is accepted and landed in the transport: a connection-scoped `SetupClock` (`gwz-transport/src/pool/setup_clock.rs`) with network, local-admission, local-interaction and terminal phases; a connect clock pauses while a connection waits on a local budget (`9eef731`) | amendment `...SshHelperClockAmendment.md`; gwz-transport `77f89cdf` | section 3 last sentence ("the Windows network arm uses begin_slice/end_slice") and section 5 ("remaining Control allowance") | Say the Windows closure only supplies waits and `SetupClock` owns the aggregate, stall and local phases. `Control::begin_slice` still exists (`transport_host/session/driver.rs:196-221`); the wording moves from slices to the clock. |
| M4 | The SSPI-only design is accepted for its bounded gate (`GwzSspiDesign.md`, `GwzSspiAcceptance.md`) | the design's own supersession list (header) | sections 2, 8 (last paragraph), 11 | **Fold the list into the text** (it is a list of replacements today): SSPI runs in an owned fresh process under the existing `Control` deadline; the kill-on-close Job is attached at creation; forced exit guarantees reaping, not secret erasure; pending calls keep their slots until process, Job and local I/O are confirmed. This removes the in-process cancellation language of section 8's last paragraph and closes P05's design need (see section 4: P05). |
| M5 | Option A, the background close, is on MAIN (`3e0b5690`, 2026-10-09); its State review is filed | `GwzTransportSshBackgroundCloseDesign.md` | section 9 (connection handling) | No clause changes. Add one sentence: a discarded SSH connection is ended by `shutdown(Both)` (`ssh_connection.rs:44`), which is portable; step 1.5 proves it on Windows. |
| M6 | Idle loss: `idle_watch.rs` exists and is Unix-only (`endpoint/mod.rs:21`) | `GwzTransportIdleLossDesign.md` section 9 | none in TR1.8 | Only the plan's step 1.5 (a Windows run under tokio's Windows driver). |
| M7 | Adaptive concurrency Phase 1, the limit machine wired non-adaptively (`fb5ad0b5`, `4c7f9f7d`), the per-host `Supervisor` (`agent_job/supervisor.rs:57`) | `GwzTransportAdaptiveConcurrencyDesign.md` F3, section 12 | none | F3 asks what kind a Windows `MaxStartups` drop gets. RS/x7-* gives 1.0.17's text for the three drop shapes (section 3, row X7); the kind the transport sees is a step 3.7 test. |
| M8 | The HTTPS fixed-cost port (`a8941cab`): one `native_tls::TlsConnector` per endpoint (`SharedTls`, `https_tls.rs`), wake-ups instead of polls; Linux CA trust follows OpenSSL's default paths | `GwzTransportCredentialHelpers*`, run `2026-10-08-ca-trust-parity` | section 8 ("capture ... in `https_connection::connect`, before the stream is wrapped for Hyper") | The capture point must be re-read: with one connector per endpoint, the binding query is still per physical connection (it asks the stream's peer, not the connector), so the clause stands, but step 4.8's test must hold with the shared connector. The CA-file variables (`SSL_CERT_FILE`, `GIT_SSL_CAINFO`) are TR2.7's, not TR1.8's; the 2026-10-08 run left them as operator decisions on macOS and Windows and the decisions document does not repeat them. |
| M9 | WH1's accepted shape: the qualification cfg `gwz_windows_https_qualification` (41 sites in core), `Owner::send_if` (`transport_host/https_endpoint.rs:355`), WinHTTP machine proxy admitted only when verified DIRECT (`endpoint_environment.rs:203-282`) | `GwzWindowsHttpsIntegrationImplementationAcceptance.md` | section 6 | Section 6 describes the machine proxy as the thing to implement; MAIN today admits DIRECT only and refuses any other machine proxy. Steps 4.6 and 4.7 widen it; until TD10 is answered the widening has no executed grammar rows behind it. |
| M10 | Windows parity Phase 0 and steps 1.1 to 1.3 are on MAIN: the Windows CI leg, the ratchet (`check_windows_parity.py`: 151 unported, 21 paired, 26 platform entries at `eb924c0e`), a Windows SSH test server on `sshd.exe` (`fixture_host.rs`), `socket_wait` (select on Windows), `git::regular_file::open` | plan; commits `b9a6595f`, `a3ba9ea0`, `caf1e4c8` | sections 3 and 4 | Section 3's "FIFO and device" and "Windows network arm" sentences now describe code, not intent. Two facts from those steps matter to the design: `getpeername` answers for a connect still in flight on Windows (the connect outcome comes from the wait, not from `peer_addr`), and the host's OpenSSH default shell is a POSIX `bash` on dabeest (RS used it). `AgentSource` (plan 3.2a) has not landed: `grep AgentSource src` finds nothing, so section 4's "owned enum" is still a proposal. |
| M11 | The plan's operator decisions of 2026-10-08 (OQ1 to OQ16 as recommended) | `GwzTransportWindowsParityPlan.md` section 8 | sections 3, 4, 5, 6, 8 | OQ2 (reuse the architecture), OQ5 (Pageant in process, bounded, no broker), OQ7 (adopt the HOME dispositions), OQ8 (match WinCNG's limits), OQ10 (Digest refused, Kerberos "not qualified"), OQ11 (refuse unsupported proxy forms and every 407) are now text the design must carry. OQ4's (b) and OQ9's (2), (3) are conditional on rows that this lane could not run (TD11, TD12). |

## 2. Clause by clause

"Stands" means the new evidence agrees. "Revise" means the clause changes whichever way the open decisions go. "Pending TDn" means the revision depends on that decision.

| Section | Verdict | What changes and why |
|---|---|---|
| 1 Authority | Revise | Replace the 2026-10-03 tuple and the "read baseline" paragraph with the tuple above; record the 2026-10-08 decisions; the SSPI-only list becomes text (M4). |
| 2 Runtime ownership | Stands | The capture-once rule holds. Add that `SetupClock` and the per-host `Supervisor` are runtime-owned, not global. `IdSource` for mapping names stands. |
| 3 SSH home and trust | **Revise** | (a) The order HOME, HOMEDRIVE+HOMEPATH, USERPROFILE stands (RS/b04-home-unset-homedrive-homepath, b04-userprofile-only). (b) "A selected relative path is refused" is **contradicted**: a relative HOME that exists under the process directory is used (RS/b04-relative-home-exists-under-cwd) and a nonexistent one is skipped (b04-relative-home-nonexistent-good-fallback); OQ7(a)'s "resolve against the captured cwd" matches 1.0.17 for the existing case and needs a closure test for the cwd change after capture. (c) An empty HOME refuses even when a later candidate is good (b04-empty-home-good-fallback), and the first existing home without a matching entry refuses without trying the next (b04-first-existing-home-*): both keep. (d) **Unicode:** 1.0.17 selects the Unicode home as existing and then fails when `known_hosts` is opened (RS/x10-existence-probe-*, x10-ansi-range-e-acute, x10-outside-ansi-*; an ASCII junction to the same directory works, x10-junction-ascii-name-for-unicode-dir). Not a bug in home search; the transport, which reads `known_hosts` through wide APIs, will accept what 1.0.17 refuses (OQ7(a)); list it in the notes. The same narrow effect hits a non-ASCII key path (RS/x3-path-unicode-*): extend the fix, TD6. (e) **`~/` in `--identity` is expanded from `USERPROFILE`, not from `HOME`** (RS/x3-path-tilde-key-only-under-userprofile authenticates; ...-only-under-home is refused), while `known_hosts` comes from libgit2's HOME-first order. Section 3's "one captured home serves known_hosts and `~/` identities" is therefore **a change from 1.0.17** whenever HOME and USERPROFILE differ: TD6. (f) **Host-key algorithms, a new paragraph:** on Windows 1.0.17 (libssh2 on WinCNG) supports RSA only. A `known_hosts` entry whose key type is ed25519 or ecdsa, and no rsa entry, fails before any connection (`failed to set hostkey preference: The requested method(s) are not currently supported`, RS/x2-a, x2-b, x2-d); an rsa line beside such lines works (x2-f, x2-h); a server offering no rsa key against an rsa entry fails in key exchange (x2-e). File keys: only an RSA key in PEM is offered (RS/x3-type-*); a new-format RSA key, ECDSA, ed25519 and a passphrase-protected key offer no key at all. The transport's host-key preference list names `ssh-ed25519` (`ssh_network.rs:24`), which the same libssh2 strips: step 3.8 and TD5. (g) CRLF `known_hosts` lines are accepted, CR-only are not (RS/x1-*). (h) `known_hosts` is mandatory before authentication: stands. |
| 4 Agent source and pipe | Revise | **Stands, now executed:** Pageant wins when both run and the pipe sees no connection (RS/b06-pageant-and-pipe-both-*); a Pageant window with no usable key refuses and never falls through to a working pipe (RS/x9-*, b06-...-only-pipe-key-authorized); the default pipe name is used when `SSH_AUTH_SOCK` is unset (RS/b08-default-pipe-name-served-by-owned-agent); a UNC pipe name is not connected even when the pipe exists locally (RS/x8-ssh-auth-sock-unc-localhost-pipe-served); a MinGW Unix path, a file path and a UNC name all give the no-agent text (x8-*), so a specific message naming `SSH_AUTH_SOCK` (the design's) is an improvement in wording only. **New facts for the agent client:** RSA keys are signed at flag 4 (`rsa-sha2-512`) only, with no fall back; a server accepting only `ssh-rsa` or `rsa-sha2-256` is refused before any signature request (RS/x4-pipe-rsa-server-*); ECDSA and ed25519 agent keys sign at flag 0 and authenticate through Pageant and the pipe (RS/x4-pageant-ecdsa, x4-pageant-ed25519, x4-pipe-ecdsa, x4-pipe-ed25519); an ed25519 certificate from the agent authenticates, an RSA certificate does not (x4-pipe-*-certificate); an identity the agent declines to sign is skipped (x4-pipe-security-key-then-rsa). **Removed unless the operator approves rows:** "a found Pageant owned by another SID refuses" (P01, TD11) and "the pipe server's token has the caller's SID and logon id" (P03, TD12). |
| 5 Pageant exchange | Stands, with P02 revised | P02's numeric HWND reuse stays UNEXECUTED; the claim is revised as step 2.1 and ProofDispositions section 3 say (the identity check before a send is not atomic with the send; no claim that reuse is prevented). OQ5(a) in process, bounded by the remaining allowance, never reuse the name, never read after a timeout, never resend. RS gives the plan a reusable Pageant inspector (`runner/pageant_client.py`) and an unencrypted PPK v3 writer for RSA, ECDSA and ed25519 (`runner/ppklib.py`; the ed25519 private blob is the seed as a big-endian mpint or as a string, the little-endian mpint loads and signs wrongly). |
| 6 WinHTTP machine proxy | Revise | The DIRECT-machine half is executed: `http.proxy` in the global git config, `HTTP_PROXY`, `http_proxy` and `HTTPS_PROXY` (with and without `NO_PROXY`) are not consulted on Windows 1.0.17 (a configured proxy saw 0 requests; RA/b09-*), a numeric loopback origin connects directly (b09-gitconfig-http-proxy-numeric-loopback-origin). So "the machine setting wins and the environment is ignored" is stated by a measurement for DIRECT. **The machine-setting half is unexecuted** (grammar of NAMED_PROXY, `<local>`, wildcards, trailing dots, ports, IPv6, the implicit numeric-loopback bypass, 407 through a configured machine proxy): TD10. B10's 407 refusal (2026-10-03) stands for the machine proxy; whether a proxy set through `http.proxy` is also refused is moot (never contacted). |
| 7 Origin authentication choice | **Revise (large)** | **The helper-first precedence is contradicted by 1.0.17.** With a helper that can answer, 1.0.17 asks no helper when the challenge offers Negotiate or NTLM (alone or with Basic) and uses the logon session (helper calls 0 in RA/b11-*, b12-ntlm-only-helper-configured, b13-negotiate-and-basic-helper-configured, b13-ntlm-and-basic-helper-configured, b14-digest-and-negotiate-no-helper). The helper is asked only when Basic is the only workable scheme (b13-basic-only-helper-configured). After the logon identity is rejected, Negotiate-only retries it to the cap; NTLM-only and Negotiate+Basic consult the helper on each replay, but the helper identity's exchange never completes on this fixture (b11/b12/b13-...-logon-rejected-*). The section's order (Negotiate, NTLM, Digest, Basic with a helper first), its "a Negotiate-only offer asks no helper" (stands), its "once a helper identity was sent and rejected never try the logon identity" (no evidence either way) and the user guide's matching paragraph need a decision: TD1. OD16 has no zone test: **observed** (NTLM and Negotiate with the logon session authenticate to an Intranet name, an Internet-zone dotted name and a numeric loopback address, RA/b15-direct-*), over HTTP. **Discovery redirects:** Windows 1.0.17 follows none (RR; RA/b15-redirect-*), so "redirects change U before lookup" describes no 1.0.17 behaviour on Windows: TD3. |
| 8 SSPI and Digest | **Revise** | **Digest is never answered by 1.0.17**: the unauthenticated request is repeated until the cap, the helper is asked each time, no `Authorization` header is sent, Digest alone or beside Basic (RA/b14-digest-only-helper-configured, b14-digest-and-basic-helper-configured). OQ10(a) (refuse Digest) is now backed by a measurement and the Digest paragraph is deleted (TD2). Negotiate and NTLM with the logon session: SPNEGO carries NTLM; the connection is authenticated once and the POST follows on it with no further `Authorization` header (RA/b11-negotiate-only-no-helper). Target-name and flags clauses: the SPN is not captured for the logon exchange in the HTTP rows (`ntlm_spn` is null in the records), so those two clauses still rest on the 2026-10-03 SSPI rows. **CBT/EPA is unexecuted** (B16 is HTTPS): TD9. The binding API primitive stands (`...-tls-adapter` run). The last paragraph (SSPI blocking) is replaced by M4. |
| 9 Connections, retries, POST | Stands, evidence added | One connection per NTLM or Negotiate exchange: observed. **POST challenge (B18):** 1.0.17 sends the 129-byte body, receives 401, and then either re-sends with a 0-byte body (Basic, the clone fails: `could not read from remote repository`) or stops (`The request must be resent`, NTLM and Negotiate): RA/b18-*. The design's "the transport refuses it rather than replaying effects" matches the outcome (a failure); the message is the transport's own (TD4, a confirmation). A pre-authentication SSH drop is one attempt in 1.0.17 (RS/x7-*); the transport's TR2.1 retry is a deliberate difference and unchanged. |
| 10 Helper executable and seams | Revise | M1 and M2 apply. **Observed on 1.0.17:** shell-form helpers and `git-credential-<name>` scripts both run for an SSH password and authenticate when Git is on `PATH`; with Git not on `PATH` the helper does not run and the attempt is refused (RS/x6-*). A URL password is offered and accepted, even when the server also lists `publickey` and there is no agent (RS/x5-*). This is the parity target for step 4.5 and closes the TR2.18/TR2.23 gap. Job Object and path claims rest on P08 (executed 2026-10-03). TR1.5's three forms stand unchanged. |
| 11 Freeze and sequence | Revise | The GO rule is unchanged. The table in section 4 below is the "disposition for every row" that step 2.1 owes, and section 5 is the list of claims that leave the design if their rows stay unexecuted. |
| 12 References | Revise | Add: libssh2 on WinCNG supports RSA only (`wincng.h` defines no ed25519; the evidence adds ECDSA); the OpenSSH backend of `agent_win.c` does not connect to a UNC pipe name here; libssh2's RSA agent signature uses flag 4 only. |

## 3. The new rows

| Row | Run and rows | Result in one line |
|---|---|---|
| X1 CRLF `known_hosts` | RS/x1-* (7) | CRLF accepted in every placement tried; CR-only refused. |
| X2 ed25519-only entry | RS/x2-* (8) | Fails before connect for ed25519 or ecdsa entries without an rsa entry; ecdsa fails too. |
| X3 file keys by type and path | RS/x3-* (14) | Only RSA PEM. Spaces, forward slashes, relative path work; `~/` from USERPROFILE; non-ASCII directory fails. |
| X4 agent keys by type | RS/x4-* (17) | RSA (flag 4 only), ECDSA, ed25519 sign; ed25519 certificate works, RSA certificate does not; declining identity skipped. Real security key and a Pageant-held certificate not run. |
| X5 URL password | RS/x5-* (4) | Offered and accepted; wrong password: one attempt. |
| X6 password server with a helper | RS/x6-* (3) | Shell-form and named helpers both work with Git on PATH. |
| X7 pre-authentication drop | RS/x7-* (5) | One attempt; texts `Failed getting banner`, `Failed sending banner`, `Unable to exchange encryption keys`; `sshd.exe` implements MaxStartups. |
| X8 `SSH_AUTH_SOCK` forms | RS/x8-* (4) | All unsupported forms give the no-agent text; UNC not connected. |
| X9 empty Pageant beside a working pipe | RS/x9-* (2) | Refused, no fall through. |
| X10 Unicode HOME cause | RS/x10-* (9) | `known_hosts` open fails (narrow path), not home selection; ASCII junction works. |

## 4. Row dispositions

Status key: **E** executed, **P** partial, **B** blocked (exact need in the decisions document), **D** dispositioned without execution.

| Row | Status | Evidence | Disposition for the design |
|---|---|---|---|
| B01, B02 | E | 2026-10-03 | unchanged |
| B03 | E | 2026-10-03; RS/b04-home-unset-homedrive-homepath, b04-userprofile-only | stands |
| B04 | E | 2026-10-03; RS/b04-*, x10-* | OQ7(a): adopt ProofDispositions section 1; Unicode cause established (known_hosts open); relative-existing case now measured |
| B05 | E | 2026-10-03; RS/b05-pageant-rsa-sha2 | stands (RSA at flag 4) |
| B06 | E | RS/b06-* (3 rows) | Pageant wins, owned pipe sees no connection; clause stands |
| B07 | E | 2026-10-03; RS/b07-neither-agent-control | stands |
| B08 | P | RS/b08-* (owned pipe, default pipe name, nonexistent pipe) | owned-pipe selection executed; the native service is B (TD12) |
| B09 | P | 2026-10-03; RA/b09-* (DIRECT machine) | env and git-config proxies ignored on DIRECT; machine-setting grammar B (TD10) |
| B10 | E | 2026-10-03 | machine-proxy 407 refused with zero offers; stands |
| B11 | E (HTTP) / B (HTTPS) | RA/b11-* | logon session wins; helper not asked; contradicts amendment 2 (TD1) |
| B12 | E (HTTP) | RA/b12-* | logon session authenticates; helper identity not used; its exchange after a rejection not completed |
| B13 | E (HTTP) | RA/b13-* | as B12 |
| B14 | E (HTTP) | RA/b14-* | Digest never answered (TD2) |
| B15 | P | 2026-10-03 (zones); RA/b15-*, RR | no zone bound (HTTP); redirects not followed on Windows (TD3); HTTPS B (TD9) |
| B16 | B | RA/spikes/spike4 | needs native trust (TD9) |
| B17 | E (Linux, Mac HTTP) / B (Mac HTTPS trusted) | RM | Negotiate refused ("'Negotiate' authentication is not supported"), no `Authorization` header, on both; NTLM-only: "could not acquire credentials"; Mac HTTPS-trusted needs an interactive authorization |
| B18 | E (HTTP) | RA/b18-* | POST challenge fails on 1.0.17; the transport refuses (TD4) |
| P01 | B | needs a second local account | claim removed unless TD11 approves |
| P02 | D | 2026-10-03; ProofDispositions section 3 | stays UNEXECUTED; claim revised (plan step 2.1) |
| P03 | B | needs the `ssh-agent` service started | rule removed or replaced unless TD12 approves |
| P04 | P | 2026-10-03; RA/b09-* | capture executed; grammar B (TD10); the immutable-snapshot clause stands as 4.6's product test |
| P05 | D | 2026-10-03; M4 | in-process cancellation is no longer a design need; SSPI runs in the owned process (accepted); no clause rests on a thread being cancelled |
| P06 | P | 2026-10-03; ProofDispositions section 4 | MD5: native TLS refusal kept, RFC 5929 section 4.1 rule is a DER/OID test; EPA positive/missing/wrong B (TD9); adapter lifetime is 4.9's product test |
| P07 | D | RA/b14-* | closed at product level: 1.0.17 never offers Digest, so no WDigest identity is needed |
| P08 | E | 2026-10-03 | stands |
| X1 to X10 | E (X4 P) | RS | section 3 |

Counts, of the 26 rows B01 to B18 and P01 to P08: 10 executed in full (B01 to B07, B10, B18, P08); 5 executed on the plain-HTTP or platform variant with the HTTPS variant blocked (B11 to B14, B17); 5 partial (B08, B09, B15, P04, P06); 3 blocked (B16, P01, P03); 3 dispositioned without execution (P02, P05, P07). Of the ten new rows X1 to X10, nine are complete and X4 is partial.

## 5. Claims that leave the design if their rows stay unexecuted (for step 2.4's reviewers)

1. Section 4: "a found Pageant owned by another SID or logon session refuses" (P01).
2. Section 4: "the pipe server process token has the caller's SID and logon AuthenticationId" (P03). Replacement options are in TD12.
3. Section 6: the NAMED_PROXY grammar beyond a bare `host[:port]` and a scheme mapping with one applicable HTTPS entry, `<local>` semantics, wildcard, trailing-dot, port and IPv6 bypass rules, the implicit numeric-loopback bypass (B09/P04, TD10). A form with no executed row is refused before any open, which is the plan's own fallback (OQ11).
4. Section 8: "a server that requires Extended Protection authenticates" and the binding-prefix layout as a wire claim (B16/P06, TD9). Removing it means the transport cannot claim EPA parity.
5. Sections 7 and 8: the helper-first precedence, the Digest paragraph and the redirect rule are not "removed for lack of a row": they are **contradicted** and change by decision (TD1 to TD3).

## 6. Companion text to change at the refresh

| Document | Statements the evidence contradicts or leaves unsupported |
|---|---|
| `GwzTransportReleasePlanAmendment-2.md` section 3.5 | Line 166 (precedence of the helpers: helper first over Negotiate, NTLM, Digest, Basic); line 178 (the `Negotiate, Basic` row with a helper authenticates with the helper's identity over Negotiate and no default-credential offer; the `NTLM`-only and `Digest` rows authenticate with the helper's identity; a redirect from one name to the other authenticates at the second; a channel-binding server authenticates); line 226 (the same rows as 4.10's tests). Line 171 (macOS and Linux) is now answered by RM: Negotiate is refused, as the line expects. The amendment is revision 7 of a DRAFT for other sections; a revision is the operator's call (TD1 to TD3). |
| `GwzTransportWindowsUserGuide-DRAFT.md` lines 76 to 82 | "your configured git helper is asked first"; "When the server also offers Negotiate, a returned helper identity uses Negotiate"; Digest named as answerable; the redirect sentences at lines 94 to 96. Line 57 onward (SSH home) needs the `~/`/USERPROFILE and the RSA-only host-key facts; the proxy paragraph can state the DIRECT-machine facts. |
| `GwzTransportWindowsBaseline.md` | Section 3 rows B06, B08, B09, B11 to B15, B17, B18 and section 4 rows P01, P03 to P07 change status as in section 4 above; add the four new runs to the run list; the remaining-rows table (lines 342 to 359) is superseded by TD9 to TD12. |
| `GwzTransportWindowsCheckpoint.md` | One entry for this lane; the "Mac-only approval requested" entry is answered (approved 2026-10-08, OQ9(4)) and the execution outcome is RM: the user-domain trust step needs an interactive authorization. |
| `GwzTransportWindowsParityPlan.md` | Appendix A: rows move as in section 4; steps 3.8, 4.5, 4.8 and 4.9 change as in section 7. |

## 7. Steps the evidence changes

| Step | Change |
|---|---|
| 3.1 | HOME resolver: add the relative-existing case, the Unicode account (fixed), the USERPROFILE `~/` split (TD6). |
| 3.2b, 3.3, 3.6 | Agent selection tests: use B06, X8, X9 as the 1.0.17 targets; no fall through from an empty Pageant. |
| 3.4 | The signature buffer's allocator question is unchanged; ECDSA and ed25519 agent keys are in scope of the matrix (`agent_auth`). |
| 3.5 | Pageant fixture: reuse `pageant_client.py` and `ppklib.py` from RS. |
| 3.7 | Windows drop texts are in X7; map a pre-banner drop to the `Io` retriable class. |
| 3.8 | Can start: the key-type and host-key matrix is measured (RSA only; TD5). |
| 4.4 | Waits on TD1. |
| 4.5 | Parity target measured (X5, X6). |
| 4.8 | Smaller if TD2 is (a): no Digest. |
| 4.9 | Redirect rows change with TD3; the POST rows assert a refusal (TD4). |
| 4.6, 4.7 | Wait on TD10 for any grammar beyond DIRECT. |
| 2.4 | Cannot start until TD1 to TD3 and TD9 to TD12 are answered (or their rows declared removed). |

## 8. What this delta does not do

It does not edit the design, the baseline, the user guide, the checkpoint or the amendment; it does not run step 2.4; it does not assert that an HTTP result holds for HTTPS (the HTTPS rows are blocked, and the decisions document says so wherever a recommendation leans on one); it does not touch product code; and the macOS native-trust row it was approved to run did not complete (RM: the user-domain trust step needed an interactive authorization and was not escalated).
