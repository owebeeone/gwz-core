# TR1.8 (Windows parity): what the evidence leaves for the operator

Date: 2026-10-10. Status: **a decision list, for the operator.** It decides nothing and authorizes nothing: no implementation, commit, push, tag, publish,
host change or account. It is the product of plan steps 2.2, 2.3 and 2.3b (`GwzTransportWindowsParityPlan.md`, revision 3) and the refresh in
[`GwzTransportWindowsParityDesign-Delta.md`](GwzTransportWindowsParityDesign-Delta.md); step 2.4 (settle, review, GO) has not been run and waits on these answers.

Every question has options, a recommendation and the evidence behind it, so all fourteen can be answered in one message. Evidence runs are in the private
`gwz-core-evidence/campaigns/transport-qualification/runs/` (private access required): **RS** `2026-10-10-tr18-windows-ssh-rows` (90 SSH rows on released Windows 1.0.17),
**RA** `2026-10-10-tr18-windows-auth-rows` (35 plain-HTTP rows), **RR** `2026-10-10-tr18-redirect-parity`, **RM** `2026-10-10-tr18-b17-macos-linux`. Row names are cited as
`RS/x2-b-all-server-ed-kh`, and so on. Released Windows 1.0.17 is `gwz.exe` SHA-256 `cbab5e9b...`.

**What the evidence is not.** All Windows authentication rows ran as **plain HTTP**, because 1.0.17 on Windows ignores `http.sslVerify=false` and `GIT_SSL_NO_VERIFY` and trusting a
fixture CA needs a trust change (TD9). Where a recommendation leans on an HTTP row it says so. SSH rows used Windows' own `sshd.exe` 9.5 and Paramiko, not GitHub.

## The answers in one line each

| # | Question | Recommend | Needs an approval or a host change? |
|---|---|---|---|
| TD1 | Which identity answers Negotiate or NTLM on Windows: the helper (amendment 2) or the logon session (1.0.17)? | A: match 1.0.17, logon session, helper only for Basic | no (amends text) |
| TD2 | Digest | A: refuse; ignore it when another supported scheme is offered | no |
| TD3 | Redirects on Windows (1.0.17 follows none) | A: follow them, as on macOS and Linux; confirm on HTTPS after TD9 | no |
| TD4 | A 401 to a POST | A: refuse with a clear message (confirm) | no |
| TD5 | RSA-only host keys and file keys on Windows | A plus B: match 1.0.17, add diagnostics | no |
| TD6 | Home directories: non-ASCII paths, `~/` | A: one captured home; fix non-ASCII for HOME and key paths | no |
| TD7 | B17, Mac HTTPS with a trusted CA | B: close B17 on plain HTTP (Mac) and HTTPS (Linux) | an interactive keychain authorization if A |
| TD8 | Hardware security key and a Pageant-held certificate | B: declare not qualified; no hardware | hardware if A |
| TD9 | Windows native trust for one fixture CA (B16 EPA and every HTTPS row) | A: attended one-certificate transaction | **yes: a trust change** |
| TD10 | Machine WinHTTP proxy rows (B09/P04 grammar) | A: authorize the serialized transactions | **yes: a machine setting** |
| TD11 | A second local account (P01 cross-SID Pageant) | B: decline; drop the claim | an account if A |
| TD12 | Start the OpenSSH Authentication Agent service (P03) | B: decline; no server-identity check on the pipe | a service start if A |
| TD13 | The list of deliberate differences from 1.0.17 for the migration notes | accept the list | no |
| TD14 | How amendment 2's superseded sentences are changed | A: a short revision 8 before step 2.4 | no |

To accept every recommendation: "TD1 A, TD2 A, TD3 A, TD4 A, TD5 A+B, TD6 A, TD7 B, TD8 B, TD9 A, TD10 A, TD11 B, TD12 B, TD13 accept, TD14 A."
The two host approvals that cost the most (TD9, TD10) need a quiet window on the shared host (about half a day and about a day); under the recommendations nothing else needs the host.

---

## Group 1: where the evidence contradicts the reviewed text

### TD1. Which identity answers `Negotiate` or `NTLM` on Windows?

**Why it is open.** Amendment 2 section 3.5 (line 166) and the draft design (section 7) say a configured credential helper that can answer goes first, over the scheme WinHTTP picks (Negotiate, NTLM, Digest, Basic),
and that a `Negotiate, Basic` offer authenticates with the helper's identity and no default-credential offer (lines 178 and 226 repeat it as tests). The amendment reasons from `winhttp.c:139-147` and `transport_support.rs:265-274`. 1.0.17 does not behave so.

**Evidence (RA, plain HTTP, SSPI-validated).** With a helper configured and able to answer, the helper is **not asked** (0 calls) and the **logon session authenticates** for: Negotiate only (`b11-negotiate-only-helper-configured`),
NTLM only (`b12-ntlm-only-helper-configured`), Negotiate+Basic (`b13-negotiate-and-basic-helper-configured`), NTLM+Basic (`b13-ntlm-and-basic-helper-configured`); Digest+Negotiate also takes the logon session (`b14-digest-and-negotiate-no-helper`, no helper configured).
The helper is asked once and answers only when **Basic is the only workable scheme** (`b13-basic-only-helper-configured`). If the logon identity is rejected, Negotiate-only retries the logon identity to a cap of 15 (`b11-negotiate-only-logon-rejected-helper-configured`);
NTLM-only and Negotiate+Basic consult the helper on each replay (15 and 14 calls) but the helper identity's exchange never completed on this fixture (`b12-...-logon-rejected-*`, `b13-...-logon-rejected-*`), so whether it would be used is not established.
The same logon-first behaviour held for a dotted Internet-zone name and for a numeric address (`b15-direct-*`), so OD16 is observed, on HTTP.

**Options.**
- **A. Match 1.0.17.** If the challenge offers Negotiate or NTLM: the logon session, the helper never asked. If Basic is the only workable scheme: the helper. After one rejection: stop with an error naming the scheme (no replay loop). Amendment 2 lines 166, 178, 226 and the user guide lines 76 to 82 change.
- **B. Keep the amendment's helper-first rule** as a deliberate change (a user with `gh` configured gets the helper identity, not the logon session, from a server offering NTLM). The migration notes say so; the Safety review covers "helper identity sent over SSPI to any host".
- **C. Logon first, helper once after a rejection** (what the replay calls suggest 1.0.17 attempts). Needs evidence the lane could not produce: a completed helper-identity NTLM exchange (a distinct account, or a better fixture).

**Recommendation: A.** OD13 and OD15 say the transport does what 1.0.17's native path does on Windows; the amendment's premise is measured false; A sends fewer credentials than B; and step 4.4 shrinks (no helper path for SSPI schemes).
**Caveat.** The scheme choice happens before TLS in libgit2's WinHTTP code, so a different HTTPS result is not expected, but it is unproven until TD9 runs B11 to B13 over HTTPS.
**Unblocks.** Steps 4.4 and 4.10; the Safety review's scope for 2.4.

### TD2. Digest

**Why it is open.** OQ10 (decided as recommended on 2026-10-08) refused Digest "decided only after step 2.3 has tried the 1.0.17 Digest row". It has been tried.

**Evidence.** RA `b14-digest-only-helper-configured` and `b14-digest-and-basic-helper-configured`: 1.0.17 repeats the unauthenticated request 15 times, asks the helper each time, **never sends an `Authorization` header**, and ends `too many redirects or authentication replays`,
even with Basic offered beside Digest. Digest with Negotiate works through Negotiate. No provider-accepted identity can change that: the client never offers Digest. (The 2026-10-03 WDigest acquire refusals are therefore irrelevant to the release.)

**Options.**
- **A. Refuse at once** when Digest is the only workable scheme, naming Digest and the off switch; **ignore Digest** when a supported scheme (Negotiate, NTLM, Basic) is also offered. Differs from 1.0.17 only for Digest+Basic, which 1.0.17 fails.
- **A'. Refuse any challenge that offers Digest unless Negotiate or NTLM is also offered** (exact outcome parity; Digest+Basic fails as on 1.0.17).
- **B. Implement Digest** (WDigest with the helper's identity). Nothing in 1.0.17 to match; no verified fixture.

**Recommendation: A.** It is OQ10(a) plus one sentence. Delete the Digest paragraph of design section 8 and the Digest rows of amendment 2 (lines 178, 226).
**Unblocks.** Step 4.8 loses its Digest half.

### TD3. Redirects on Windows

**Why it is open.** The draft design (section 7: "discovery redirects change U before lookup") and amendment 2 (line 178: "a row in which one name redirects discovery to the other authenticates at the second") assume a discovery redirect is followed. On Windows 1.0.17 it is not.

**Evidence.** RR: macOS and Linux 1.0.17 follow all of 301, 302, 307, 308, to a new path on the same origin and to another origin (16 of 16). Windows 1.0.17 follows **none** (0 of 8; RA `b15-redirect-*` 0 of 5, and 12 more code-by-form combinations in RA `raw/spikes/spike5.out`):
it repeats the original request 15 times, then `too many redirects or authentication replays`. Git for Windows follows the same server (`raw/spikes/spike6.out`), so the fixture is sound. HTTP only.

**Options.**
- **A. The transport follows discovery redirects on Windows**, as on macOS and Linux and as the design says; credentials never follow a redirect to another origin (TR1.6 OQ4). A deliberate difference from Windows 1.0.17, which already fails such remotes; the "cross-zone redirect" test becomes a transport-only test.
- **B. Mirror 1.0.17:** a 3xx on discovery is an error on Windows only. Reproduces what is probably a libgit2/WinHTTP defect and makes Windows differ from the other two platforms.
- **C. Decide after HTTPS:** run the redirect row over HTTPS after TD9; if HTTPS follows, there is no difference to decide.

**Recommendation: A**, with C's HTTPS check when TD9 runs. Record the 1.0.17 behaviour in the migration notes (S7.2) so a Windows user whose remote redirects knows the transport fixes it.

### TD4. A 401 in answer to a POST (confirm)

**Evidence.** RA `b18-*`: the discovery GET succeeds, the POST is challenged. With Basic the first POST carries its 129-byte body and the retried POST carries **0 bytes** (the clone fails: `could not read from remote repository`); with NTLM or Negotiate the client stops with `failed to receive response: The request must be resent`. 1.0.17 fails in all three.
**Options.** A. The transport refuses a challenged POST with a plain message and never replays the body (the draft design, section 9). B. Replay the body after authenticating (a fix; risks repeating an effect).
**Recommendation: A.** Same outcome as 1.0.17 (a failure), a readable message. The case needs a server that challenges POST but not discovery; it is rare.

---

## Group 2: limits of the platform that decide what to promise

### TD5. RSA-only host keys and file keys

**Why it is open.** OQ8 chose "match WinCNG's limits" on the premise that only ed25519 is missing. The evidence is wider and the failure texts are poor.

**Evidence.** RS: with a `known_hosts` entry of **ed25519 or ecdsa** type and no rsa entry, 1.0.17 fails before connecting with `failed to set hostkey preference: The requested method(s) are not currently supported` (`x2-a`, `x2-b`, `x2-d`);
an rsa line beside such a line works (`x2-f`, `x2-h`). **File keys:** only an RSA key in **PEM** is offered; a new-format RSA key (the default `ssh-keygen -t rsa` output), ECDSA, ed25519 and a passphrase-protected key offer **no key**, and the error is `failed to authenticate SSH session:` with no reason (`x3-type-*`, `x3-encrypted-rsa-pem`).
**Agent keys:** RSA, ECDSA and ed25519 all authenticate through Pageant and an owned pipe agent (`x4-pageant-*`, `x4-pipe-*`); RSA is signed at flag 4 (`rsa-sha2-512`) only, so a server accepting only `ssh-rsa` or `rsa-sha2-256` is refused before any signature request (`x4-pipe-rsa-server-*`); an ed25519 certificate works, an RSA certificate does not.
A Windows OpenSSH client's `known_hosts` typically holds ecdsa or ed25519 lines, so users meet this on 1.0.17 today.

**Options.**
- **A. Match 1.0.17** (the transport uses the same libssh2 and WinCNG and gets the same limits); document them in the notes and the user guide.
- **B. A plus diagnostics:** name the cause and the fix (an ecdsa or ed25519-only `known_hosts` entry: add an rsa entry with `ssh-keyscan -t rsa`; a key file that is not RSA PEM: `ssh-keygen -p -m PEM`), replacing 1.0.17's two opaque texts.
- **C. Build libssh2 against OpenSSL on Windows** (`openssl-on-win32`): ed25519 and ECDSA host and file keys. A build and packaging project (vcpkg, licensing) that makes the transport support more than 1.0.17.

**Recommendation: A and B together.** Parity is the rule; C is its own project; B changes messages, not behaviour. Step 3.8 can start now.
**Unblocks.** Step 3.8; the user guide's SSH section.

### TD6. Home directories: non-ASCII paths and `~/`

**Evidence.** RS: 1.0.17 selects a non-ASCII HOME as existing and then **fails when `known_hosts` is opened** (a narrow path inside libssh2): `x10-existence-probe-*`, `x10-ansi-range-e-acute`, `x10-outside-ansi-*`, `x10-unicode-and-spaces`; an ASCII junction to the same directory works (`x10-junction-ascii-name-for-unicode-dir`).
The same narrow effect refuses a key file under a non-ASCII directory (`x3-path-unicode-*`). A relative HOME that exists under the process directory is used (`b04-relative-home-exists-under-cwd`); empty HOME refuses even with a good fallback; the first existing home without a matching entry refuses without trying the next (`b04-*`).
And `~/` in `--identity` is expanded from **`USERPROFILE`**, while `known_hosts` is read from libgit2's HOME-first order (`x3-path-tilde-key-only-under-userprofile` authenticates; `...-only-under-home` is refused), so with `HOME` different from `USERPROFILE` 1.0.17 uses two homes.

**Options.**
- **A. One captured home** in the HOME, HOMEDRIVE+HOMEPATH, USERPROFILE order serves `known_hosts` and `~/` identities (the draft design); non-ASCII HOME and key paths work (OQ7(a) already fixes HOME; extend it to `--identity`); a relative HOME is resolved against the captured current directory.
- **B. Exact parity:** `~/` from USERPROFILE, `known_hosts` from the HOME-first order, non-ASCII refused as 1.0.17 does.
- **C. A for non-ASCII, B for `~/`.**

**Recommendation: A.** Reproducing a refusal that comes from a narrow `fopen` serves no user (the plan's OQ7 reasoning), and the `~/` split matters only when HOME differs from USERPROFILE. List both in the deviation register (TD13).

### TD7. B17, the Mac HTTPS row with a trusted CA

**Evidence.** RM: on macOS and Linux 1.0.17 refuses `401 Negotiate` (`'Negotiate' authentication is not supported`, code `GitCommandFailed`) and sends **no `Authorization` header**; NTLM-only gives `could not acquire credentials` (`RemoteRejected`). Linux was run over HTTPS (CA trusted through `SSL_CERT_FILE`) and over HTTP with identical results; the Mac over plain HTTP and, as a control, HTTPS with the CA **untrusted** (fails at certificate verification).
The approved Mac trust step (temporary keychain, `security add-trusted-cert -r trustRoot -k <temp keychain>`) **did not complete: macOS raised an interactive authorization dialog** and the lane did not escalate. Everything was restored (search list equal to the saved one, no entry for the CA, no keychain file). The dialog's helper process (`SecurityAgent`) outlived the run by several minutes and has since exited.
**Options.** A. An attended re-run in which the operator answers the one authorization (a few minutes). B. Close B17 as executed: the Negotiate refusal is decided by libgit2 before any TLS-specific step, the Linux HTTPS row is complete, and the Mac plain-HTTP and untrusted-HTTPS rows are consistent with it.
**Recommendation: B.** An HTTPS-trusted Mac row would add no information about the authentication layer; the amendment needs "refused with a message naming the off switch", which is the transport's job, not a 1.0.17 property.
**Note.** The Mac binary is a local source build of 1.0.17 (`cf97fa15...`), not the release asset; Linux is the released artifact on **aarch64** (the release target is x86-64).

### TD8. A hardware security key; a certificate held by Pageant

**Evidence.** RS: an agent identity that declines to sign is skipped and the next one used (`x4-pipe-security-key-then-rsa`, emulated); an ed25519 certificate served by a pipe agent authenticates (`x4-pipe-ed25519-certificate`). Not run: a real `sk-ssh-ed25519@openssh.com` key (needs a hardware token) and a certificate loaded into Pageant (needs puttygen's GUI).
**Options.** A. Provide a token and a certificate-capable Pageant setup and run both. B. Declare both **not qualified** on Windows in the notes (security-key signing and Pageant-held certificates), with the executed behaviours (skip a declining identity; certificates from a pipe agent for ed25519) as the support statement. C. Remove the TR2.8 list's security-key entry on Windows.
**Recommendation: B.** S5.6 forbids advertising a cell without evidence; B advertises none.

---

## Group 3: approvals that would finish rows (the schedule levers)

If none of TD9 to TD12 is answered yes, step 2.4's reviewers get the claims listed in the delta's section 5 as removed, which the plan allows (a row that cannot run discharges section 11 by removing its claim). The cost is what the claim protected.

### TD9. Windows native trust for one fixture CA (B16 EPA and every HTTPS row)

**What it blocks.** B16 (a server that requires channel binding, positive and negative controls), the HTTPS variants of B11 to B15 and B18, P06's EPA part, and the HTTPS confirmation of TD1 and TD3. Amendment 2 requires "a server that requires channel binding authenticates" (lines 178, 226).
**Evidence for "needs trust".** 1.0.17 ignores `http.sslVerify=false` and `GIT_SSL_NO_VERIFY` on Windows (`RA/raw/spikes/spike4.out`: three attempts, zero requests reached the server); environment CA files are ignored (2026-10-03 `process-ca-windows`, 2026-10-08 `ca-trust-parity`). The 2026-10-03 guarded transaction (`NATIVE_TRUST_TRANSACTION_v3.md`) was held because its supervisor could not prove it is outside every job object.
**Options.**
- **A. Attended one-certificate transaction.** On dabeest, one disposable certificate (a day's validity, pinned thumbprint) is added to the **current user's** Root store with `certutil -user -addstore Root <cert>` by the operator or in an attended session, the lane runs the HTTPS rows inside a booked interval of about 45 minutes, and the certificate is removed (`certutil -user -delstore Root <thumbprint>`) and its absence verified. No machine store, no service, no policy.
- **B. Unattended, with an external supervisor** that is proved to be outside every job (the 2026-10-03 design). New engineering before any row can run.
- **C. Decline.** B16 stays unexecuted and the EPA/CBT claim leaves the design; the transport could not claim EPA parity, so a server that requires Extended Protection (AD FS, Exchange and current IIS setups often do) works on 1.0.17 and fails on the transport with nothing in the evidence to show it.
**Recommendation: A.** It is the smallest change that makes the EPA claim evidence. About half a day of host time including the HTTPS variants.

### TD10. The machine WinHTTP proxy (B09/P04 grammar)

**What it blocks.** Everything the design says about the machine proxy beyond "DIRECT": the NAMED_PROXY grammar, `<local>`, wildcards, trailing dots, ports, IPv6, the implicit numeric-loopback bypass, and a 407 through a configured machine proxy. The DIRECT half is done: on Windows 1.0.17 `http.proxy`, `HTTP_PROXY`, `http_proxy` and `HTTPS_PROXY` are never consulted and a proxy so configured saw 0 requests (`RA/b09-*`); only `netsh winhttp` settings reach WinHTTP.
**Options.**
- **A. Authorize the serialized transactions** as the 2026-10-03 run did: booking lock, exact prior state saved, a deadline guard armed before each mutation, restore and verify after each row. A machine proxy change affects every WinHTTP client on the host for the interval, so it needs a quiet window. About a day.
- **B. Narrow the promise:** execute only a bare `host:port` machine proxy (a few rows) and refuse every other form before any open (OQ11's fallback).
- **C. Decline:** keep WH1's DIRECT-only admission (any other machine proxy is refused, with the off switch named). TR4.9 and OD15 then need an amendment, because the transport would not match 1.0.17 behind a proxy.
**Recommendation: A.** Proxies are common in Windows shops, and OQ11 already limits the grammar to refuse what cannot be proved.

### TD11. A second local account (P01, a Pageant owned by another SID)

**What it blocks.** Only the claim "a found Pageant owned by another SID or logon session refuses" (draft design, section 4). Under TD1 A, no helper-identity row needs an account either (B12's helper-identity exchange is moot).
**Options.** A. Approve a disposable local account for the run and its removal. B. Decline; remove the claim; keep the checks that need no second account (the pinned window-to-process check, creation identity).
**Recommendation: B.** 1.0.17 does no such check, libssh2 neither; a stricter rule than the released client needs its own evidence, and the account is a larger change than the claim is worth. If the Safety review in 2.4 insists on the refusal, approve A for P01 alone (the plan's own fallback).

### TD12. Starting the OpenSSH Authentication Agent service (P03)

**What it blocks.** The native-service half of B08 and the draft design's "pipe server token has the caller's SID and logon id" (section 4), which the design itself says "cannot be frozen unchanged" if the service runs as SYSTEM.
**Evidence.** The OpenSSH backend connects to whatever pipe `SSH_AUTH_SOCK` names, or `\\.\pipe\openssh-ssh-agent`, and checks no server (`RS/b08-owned-pipe-rsa`, `b08-default-pipe-name-served-by-owned-agent`); a UNC name is not connected even if the pipe exists (`x8-ssh-auth-sock-unc-localhost-pipe-served`).
**Options.**
- **A. Approve a bounded start** (set the service to manual, start, record the pipe server's identity and run B08, stop, restore the start type). Admin rights on the host.
- **B. Decline and adopt OQ4(a):** no identity check on the pipe server, as libssh2; keep the local-name admission (no UNC, no remote). The native service works as it does on 1.0.17 because nothing is checked.
- **C. Adopt OQ4(b) unmeasured:** not allowed by the design's own section 11.
**Recommendation: B.** It is exact parity and removes a rule that has no row behind it. The one thing B leaves is S5.6's evidence for the native-agent cell: the owned-pipe rows cover the code path; if the operator wants the real service in the evidence, take A at step 6.3.

---

## Group 4: process

### TD13. The deliberate differences from 1.0.17 (for the migration notes)

The recommendations above make the transport differ from released Windows 1.0.17 in these places. Confirm the list, or strike items (a struck item must be matched exactly):

1. Non-ASCII HOME and key paths work (TD6).
2. A challenge offering Digest beside a supported scheme uses the supported scheme; a Digest-only challenge fails at once, not after 15 replays (TD2).
3. A rejected logon identity ends the operation with an error; no replay loop (TD1).
4. Discovery redirects are followed on Windows (TD3).
5. A POST challenge is refused with a plain message (TD4).
6. `~/` identities use the same captured home as `known_hosts` (TD6).
7. An SSH drop before authentication is retried under the retry plan (TR2.1); 1.0.17 makes one attempt (RS/x7-*, 1 connection per drop shape).
8. Plain messages replace `failed to set hostkey preference`, an empty `failed to authenticate SSH session:`, and the shared no-agent text for unsupported `SSH_AUTH_SOCK` forms (TD5, and the design's naming of the variable).
9. Unchanged by choice (not deviations): environment and git-config proxies are ignored on Windows; the logon session's credentials go to any host that challenges (OD16); RSA-only host and file keys; RSA agent signatures at `rsa-sha2-512` only.

**Recommendation: accept the list.**

### TD14. How amendment 2's superseded sentences are changed

TD1 to TD3 make lines 166, 178 and 226 of `GwzTransportReleasePlanAmendment-2.md` wrong, and line 226 is the implementation brief for TR4.10's tests.
**Options.** A. A short revision 8 of section 3.5 now, with a skim review as revisions 3 to 6 had, so step 2.4 reviews consistent text. B. Fold the changes into the TR1.8 design revision and leave the amendment stale until its next revision. C. Wait for step 2.4.
**Recommendation: A.** Step 4.4, 4.8 and 4.9 read the amendment first.

---

## After the answers

1. Run the approved host work (TD9 about half a day, TD10 about a day, TD12 optional), each inside a booked interval; the rows to run are listed in the delta's section 4. Nothing else needs the host.
2. Write the one design revision from the delta and the answers; remove the claims whose rows stay unexecuted (delta section 5); revise the baseline, user guide and checkpoint (delta section 6).
3. Step 2.4: the dual Consistency and Safety review plus Surface, with the removed-claims list as part of the reviewers' input.
4. Steps that can start now regardless: 3.8 (the key-type matrix is measured), 3.5 (the Pageant inspector and PPK writer exist in RS), 3.7 (the drop texts), 4.5 (the helper and URL-password targets).

## What is complete and what is not

Executed on released 1.0.17: 90 Windows SSH rows (RS), 35 Windows HTTP authentication and proxy rows (RA), the redirect comparison on three platforms (RR), B17 on Linux and the Mac over HTTP (RM). Not executed, each with its exact need: B16 and every HTTPS row (TD9); the machine-proxy grammar (TD10); P01 (TD11); P03 and the native-service half of B08 (TD12);
a real security key and a Pageant-held certificate (TD8); the Mac HTTPS row with a trusted CA (TD7); P05, which no longer needs execution (the accepted SSPI-only design isolates SSPI in an owned process); B12's helper-identity exchange after a rejected logon identity (TD1, option C).
