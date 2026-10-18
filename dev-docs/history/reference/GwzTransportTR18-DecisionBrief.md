# TR1.8 decisions TD1 to TD14: a brief for the operator

Date: 2026-10-10. Status: **a reading aid; superseded for the answers.** The operator recorded all fourteen decisions on 2026-10-10 in the decision list's discussion record, which controls; in particular TD12 there takes the baseline pipe policy (B) and also approves the bounded real-service run (A), and TD1, TD6, TD7, TD8 and TD9 carry qualifications this brief does not. It explains the fourteen questions in
[`GwzTransportTR18-OperatorDecisions.md`](GwzTransportTR18-OperatorDecisions.md) (the decision list, which keeps the row-level evidence),
with enough background to answer each one without the rest of the program's documents. The answer sheet is at the end.
The list's recommendations are unchanged here; where this brief adds a note of its own it says so.

## 1. Background

### What release this is for

1.1.0 ships the new transport inside the `gwz` CLI on macOS, Linux and Windows. "The transport" is gwz's own code for talking to
Git servers over SSH and HTTPS. It replaces the path 1.0.17 uses, which hands every network operation to libgit2.
On Windows, 1.0.17's network code is libgit2 on top of two Windows libraries: **WinHTTP** for HTTPS and **libssh2 built on WinCNG**
(Windows' own crypto library) for SSH.

### The rule these decisions apply

The standing rule (OD13 and OD15, 2026-10-01) is **parity with 1.0.17**: on each platform, the transport does what 1.0.17 does. Where it deliberately
does something different, that difference is written down for the migration notes. Parity means the observed behaviour, not what our own
documents claim 1.0.17 does.

### What TR1.8 is, and why it needs these answers

TR1.8 is the Windows parity design: how the transport finds SSH keys and agents, which home directory it uses, which HTTPS authentication
schemes it answers and with whose credentials, proxies, and redirects. Its draft (2026-10-03) was not accepted, because several of its claims
about 1.0.17 had never been measured.

Phase 2 of the Windows parity plan measures them. On 2026-10-10 lane D ran released 1.0.17 on the Windows test host (dabeest) and on macOS and Linux:

- **90 SSH rows**, against Windows' own OpenSSH server, a Python server, Pageant and a named-pipe agent;
- **35 HTTP authentication and proxy rows**, against a server that checks real Windows logins;
- **a redirect comparison** on all three platforms;
- **row B17** on the Mac and on Linux.

The runs are in the private gwz-core-evidence repository. [`GwzTransportWindowsParityDesign-Delta.md`](GwzTransportWindowsParityDesign-Delta.md)
maps them onto the design.

The measurements disagree with our reviewed text in four places, show platform limits wider than we assumed, and leave some rows that can only
run if you approve a change on the host. Step 2.4 freezes TR1.8 with a dual review, and it cannot start until these are answered.

### One limit on the evidence

Every Windows authentication row ran over **plain HTTP**. On Windows, 1.0.17 ignores `http.sslVerify=false` and `GIT_SSL_NO_VERIFY`, and it ignores
CA files named in the environment. So HTTPS against a test server needs that server's certificate in Windows' own trust store, which is TD9.
Where a recommendation rests on an HTTP row, the decision list says so.

### Terms used below

| Term | Meaning |
|---|---|
| Negotiate, NTLM | Windows' integrated login schemes. Through SSPI, the client answers with the identity of the logged-in user, with no password typed. |
| Logon session | The Windows login the process runs under. Answering Negotiate or NTLM "with the logon session" means using that user's credentials automatically. |
| Credential helper | Git's `credential.helper` (for example `gh` or Git Credential Manager): a program that supplies a username and password or token. |
| Basic, Digest | Two password-based HTTP schemes. Basic sends the password over TLS; Digest sends a hash of it. |
| SSPI | Windows' security API, which performs Negotiate and NTLM. |
| EPA, channel binding | Extended Protection for Authentication: a server ties the Windows login to the TLS connection. AD FS, Exchange and many IIS setups require it. |
| WinCNG | Windows' crypto library. libssh2 built on it supports fewer key types than libssh2 built on OpenSSL. |
| Pageant | PuTTY's SSH agent, common on Windows. |
| OpenSSH agent pipe | Windows' own `ssh-agent`, reached through the named pipe `\\.\pipe\openssh-ssh-agent`. |
| `known_hosts` | The file of trusted server keys that SSH checks before logging in. |
| WinHTTP machine proxy | The proxy set with `netsh winhttp set proxy`. It is machine-wide, and the only proxy 1.0.17 uses on Windows. |
| Row | One measured case in the evidence, for example B11 or X3. B rows are baseline rows; X rows are extra cases. |
| Migration notes | What the 1.1.0 release notes tell users about behaviour that changed from 1.0.17. |
| Amendment 2 | `GwzTransportReleasePlanAmendment-2.md`, the release-plan amendment that states several of the Windows rules now in question. |

## 2. How the decisions fit together

- **Group 1, TD1 to TD4: the evidence contradicts our reviewed text.** Choose between matching 1.0.17 and keeping the text as a deliberate change.
- **Group 2, TD5 to TD8: platform limits.** Decide what 1.1.0 promises on Windows.
- **Group 3, TD9 to TD12: host approvals that would let blocked rows run.** Saying no is allowed. The design then drops the claim the row would have proved.
- **Group 4, TD13 and TD14: process.** TD13 confirms the list of deliberate differences that your other answers produce. TD14 decides how the release-plan amendment gets corrected.

Links between them:
- TD1 A makes TD11 nearly moot.
- TD9 is also how TD1 and TD3 get confirmed over HTTPS.
- TD13's list follows from TD1 to TD6, so answer it last.

**Two of these touch approvals you have already given** (this brief's note):
- **TD9 A** is the same one-certificate trust transaction you approved under OQ9(1) on 2026-10-08. Answering A confirms that approval.
- **TD12 B recommends not using an approval you gave.** OQ9(3) approved starting the OpenSSH agent service for row P03. The list now recommends declining it, because 1.0.17 checks nothing about the pipe's server, so there is no rule to prove. Choosing TD12 A keeps your earlier approval.

## 3. The decisions

### TD1. Who answers a Negotiate or NTLM challenge on Windows?

**The question.** When an HTTPS server offers Windows login (Negotiate or NTLM) and the user has a credential helper configured, does the
transport log in as the Windows user, or ask the helper?

**What our text says.** Amendment 2 §3.5 and the draft design say the helper goes first: a configured helper answers, even when the server offers Negotiate.

**What 1.0.17 does.**
- The helper is never asked: zero calls in every case.
- The Windows logon session answers, for Negotiate alone, NTLM alone, Negotiate with Basic, and NTLM with Basic.
- The helper is asked only when Basic is the only scheme offered.
- When the server rejects the logon identity, 1.0.17 retries in a loop, up to 15 times.

**Options.**
- **A. Match 1.0.17.** Windows login when Negotiate or NTLM is offered; the helper only for Basic-only; stop after one rejection with a clear error instead of looping.
- **B. Keep helper-first** as a deliberate change. A user with `gh` configured would then log in as the `gh` identity, not as their Windows user, at a server that offers NTLM.
- **C. Windows login first, then the helper once after a rejection.** We don't have the evidence to define this: no row completed a helper-identity NTLM login.

**Recommended: A.** It is parity, it sends fewer credentials than B, and it removes the helper path for Windows schemes, which shrinks step 4.4.
The scheme is chosen before TLS, so HTTPS should behave the same, but that is only confirmed once TD9 runs.

### TD2. Digest

**The question.** What does the transport do when a server offers Digest?

**Background.** OQ10 (2026-10-08) said refuse Digest, once 1.0.17's Digest row had been tried. It has now been tried.

**What 1.0.17 does.** It never answers Digest: it sends the same unauthenticated request 15 times, asks the helper each time, then fails.
- It fails this way even when Basic is offered alongside Digest.
- Digest offered together with Negotiate works, because 1.0.17 uses Negotiate.

**Options.**
- **A. Refuse Digest-only at once,** with a message naming Digest. When a scheme we support is offered alongside, ignore Digest and use that scheme. This differs from 1.0.17 only for Digest plus Basic, which 1.0.17 fails and A succeeds.
- **A'. Exact outcome parity:** Digest plus Basic fails, as on 1.0.17.
- **B. Implement Digest.** There is nothing in 1.0.17 to match.

**Recommended: A.** It is OQ10 plus one sentence, and it removes Digest from step 4.8.

### TD3. Redirects on Windows

**The question.** Does the transport follow an HTTP redirect (301, 302, 307 or 308) during discovery on Windows?

**What 1.0.17 does.**
- On macOS and Linux it follows all of them, same server or another server: 16 of 16.
- On Windows it follows none. It repeats the original request 15 times, then fails with "too many redirects or authentication replays".
- Git for Windows follows the same server's redirect, so the test server is not at fault. This is very likely a libgit2/WinHTTP defect.

**Options.**
- **A. Follow redirects on Windows,** as on macOS and Linux and as the design says. Credentials never follow a redirect to a different server.
- **B. Mirror 1.0.17:** a redirect is an error on Windows only.
- **C. Decide after re-running the redirect rows over HTTPS** (needs TD9).

**Recommended: A, with C's HTTPS check when TD9 runs.** The migration notes tell Windows users whose remote redirects that 1.1.0 fixes it.

### TD4. A login challenge on a push or fetch POST (confirm)

**The question.** The server accepts the first request of a clone (discovery) without a login, then demands one on the POST that carries the data. What happens?

**What 1.0.17 does.** It fails in every case.
- With Basic, it retries the POST with an empty body.
- With NTLM or Negotiate, it stops with "The request must be resent".

**Options.**
- **A. Refuse with a plain message,** and never resend the request body.
- **B. Resend the body after logging in.** This would be a fix, but it risks repeating an effect.

**Recommended: A.** The outcome is the same as 1.0.17 (a failure), with a readable message. The case is rare.

### TD5. Windows SSH supports RSA keys only

**The question.** What does 1.1.0 promise about SSH key types on Windows?

**Background.** OQ8 chose to match WinCNG's limits, assuming only ed25519 was missing. The evidence shows more is missing.

**What 1.0.17 does.**
- **Host keys in `known_hosts`:** if the entry for a server is only ecdsa or ed25519, with no rsa entry, it fails before connecting with "failed to set hostkey preference: The requested method(s) are not currently supported". Windows' own OpenSSH usually writes ecdsa or ed25519 entries, so users hit this today.
- **Key files (`--identity`):** only an RSA key in PEM format works. The default `ssh-keygen -t rsa` output, ECDSA, ed25519 and passphrase-protected keys are not offered, and the error is an empty "failed to authenticate SSH session:".
- **Keys in an agent** (Pageant or a pipe agent): RSA, ECDSA and ed25519 all work. RSA keys sign only as `rsa-sha2-512`, so a server that accepts only `ssh-rsa` or `rsa-sha2-256` refuses.

**Options.**
- **A. Match 1.0.17,** and document the limits.
- **B. A, plus clear error messages** that name the cause and the fix: "add an rsa entry with `ssh-keyscan -t rsa`", or "convert the key with `ssh-keygen -p -m PEM`".
- **C. Build libssh2 on OpenSSL for Windows,** to get every key type. This is a separate build-and-packaging project.

**Recommended: A and B.** Parity, with better messages. Step 3.8 can then start.

### TD6. Home directories: non-ASCII paths, and `~/`

**The question.** Which home directory does Windows SSH use, and do non-ASCII paths work?

**What 1.0.17 does.**
- **Non-ASCII HOME:** it picks a HOME whose path contains non-ASCII characters, then fails to open `known_hosts` inside it. A key file under a non-ASCII directory also fails.
- **Two homes:** `~/` in `--identity` expands from `USERPROFILE`, while `known_hosts` is read from HOME first. When those differ, 1.0.17 uses two different homes.
- **HOME edge cases:** a relative HOME is resolved against the current directory, and an empty HOME is refused.

**Options.**
- **A. One captured home** (HOME, then HOMEDRIVE plus HOMEPATH, then USERPROFILE) for both `known_hosts` and `~/` keys, with non-ASCII paths working. This extends OQ7, which already fixes HOME.
- **B. Exact parity,** including both failures.
- **C. A for non-ASCII paths, B for `~/`.**

**Recommended: A.** Copying a failure that comes from a narrow file-open call helps no user. Both changes go on the TD13 list.

### TD7. Row B17 on the Mac, HTTPS with a trusted CA

**The question.** How do we close row B17, which checks what 1.0.17 does on macOS and Linux when a server offers Negotiate?

**What happened.**
- On macOS and Linux, 1.0.17 refuses Negotiate with "'Negotiate' authentication is not supported" and sends no credentials.
- Linux was measured over HTTPS and over HTTP, with the same result. The Mac was measured over HTTP.
- The Mac HTTPS run with a trusted test CA was blocked: adding the CA to a temporary keychain raised an interactive macOS authorization dialog. Nothing was left behind.

**Options.**
- **A. An attended re-run:** you answer the one dialog, which takes a few minutes.
- **B. Close B17 on the existing rows.** The refusal is decided before TLS matters.

**Recommended: B.** A trusted-HTTPS Mac row would add no information about authentication.

### TD8. Hardware security keys, and certificates held in Pageant

**The question.** Does 1.1.0 claim Windows support for hardware security keys (`sk-ssh-ed25519`) and for certificates loaded into Pageant?

**What was measured.**
- An agent identity that declines to sign is skipped and the next one is used.
- An ed25519 certificate from a pipe agent works.
- Not run: a real hardware token (none available), and a certificate in Pageant (that needs PuTTYgen's GUI).

**Options.**
- **A. Supply a token and a GUI setup,** and run both rows.
- **B. Declare both not qualified on Windows** in the notes.
- **C. Remove security keys from the Windows support list.**

**Recommended: B.** The rules forbid claiming support without evidence, and B claims none.

### TD9. Trust one test CA in Windows' certificate store (host approval)

**What it unblocks.**
- Every HTTPS authentication row on Windows.
- Row B16: a server that requires EPA (channel binding). Amendment 2 requires that this works. Servers like AD FS, Exchange and many IIS sites require EPA, so without the row we cannot show that the transport works where 1.0.17 works.
- The HTTPS confirmation of TD1 and TD3.

**Options.**
- **A. One attended transaction.** One disposable test certificate, valid for a day and pinned by thumbprint, goes into the current user's Root store (`certutil -user -addstore Root`). The HTTPS rows run in a booked window of about 45 minutes. The certificate is then removed (`certutil -user -delstore Root <thumbprint>`) and its absence checked. No machine store, no service, no policy is touched.
- **B. Unattended, with an external supervisor process.** This needs new engineering first.
- **C. Decline.** The EPA claim leaves the design.

**Recommended: A.** About half a day of host time in total. You approved this transaction under OQ9(1), so A confirms that approval.

### TD10. The machine-wide WinHTTP proxy (host approval)

**What it unblocks.** Everything the design says about a machine proxy beyond "no proxy":
- named proxies, `<local>`, wildcards, ports and IPv6;
- a 407 (proxy login) through the machine proxy.

The no-proxy half is already measured: on Windows, 1.0.17 ignores `http.proxy` and the proxy environment variables entirely, and only `netsh winhttp` settings reach it.

**Options.**
- **A. Authorize serialized changes to the machine proxy setting** on dabeest, as the 2026-10-03 run did. The host is booked, the exact prior setting is saved, a deadline guard is armed before each change, and the setting is restored and checked after each row. While it runs, every WinHTTP program on dabeest sees the proxy, so it needs a quiet window of about a day.
- **B. Narrow the promise:** prove only a plain `host:port` proxy, and refuse every other form before any connection.
- **C. Decline:** only "no proxy" is admitted. Any other machine proxy is refused with a message, and the release plan needs an amendment, because the transport would not match 1.0.17 behind a proxy.

**Recommended: A.** Proxies are common in Windows shops. This is a new host approval.

### TD11. A second Windows account on dabeest (host approval)

**What it unblocks.** Only one claim in the draft design: a Pageant run by another user or another login session is refused.

**Options.**
- **A. Approve a disposable local account** for the run, removed afterwards.
- **B. Decline,** and remove the claim.

**Recommended: B.** 1.0.17 and libssh2 do no such check. A rule stricter than the released client needs its own evidence, and an account is more change than the claim is worth.
Under TD1 A, no HTTP row needs a second account either. If step 2.4's Safety review insists on the refusal, A can be approved then, for that row alone.

### TD12. Start Windows' OpenSSH agent service (host approval)

**What it unblocks.** Row P03 and the native-service half of row B08, which are the draft design's rule that the agent pipe's server must run as the same user and login session.

**What 1.0.17 does.**
- It connects to whatever pipe `SSH_AUTH_SOCK` names, or to the default pipe name, and checks nothing about who serves it.
- A network (UNC) pipe name is not used.

**Options.**
- **A. A bounded service start:** set the service to manual, start it, record the pipe server's identity, run the rows, stop it, restore its start type.
- **B. Decline, and check nothing,** exactly like 1.0.17 and libssh2. Local pipe names only.

**Recommended: B.** It is exact parity, and it removes a rule that has no evidence behind it. Your earlier OQ9(3) approval for this start can be used at step 6.3 instead, if you want the real service in the final evidence.

### TD13. The list of deliberate differences from 1.0.17

Your answers to TD1 to TD6 produce this list of places where Windows 1.1.0 behaves differently from Windows 1.0.17. It goes into the migration notes.
Confirm it, or strike items; a struck item must then match 1.0.17 exactly.

1. Non-ASCII HOME and key paths work (TD6).
2. Digest beside a supported scheme uses that scheme. Digest alone fails at once, not after 15 tries (TD2).
3. A rejected Windows login ends with an error, with no retry loop (TD1).
4. Redirects during discovery are followed (TD3).
5. A login challenge on a POST is refused with a plain message (TD4).
6. `~/` keys use the same home as `known_hosts` (TD6).
7. An SSH connection dropped before login is retried under the retry plan. 1.0.17 tries once.
8. Clear messages replace "failed to set hostkey preference", the empty "failed to authenticate SSH session:", and the shared no-agent text (TD5).

Not differences, because they are kept as in 1.0.17: environment and git-config proxies are ignored on Windows; the Windows login is sent to any server that asks for it; RSA-only host and file keys; RSA agent signatures as `rsa-sha2-512` only.

**Recommended: accept the list.**

### TD14. How amendment 2 gets corrected

**The question.** TD1 to TD3 make three passages of amendment 2 §3.5 wrong. One of them is the brief for the tests in step TR4.10. When are they fixed?

**Options.**
- **A. A short revision 8 of §3.5 now,** with a skim review, so step 2.4 reviews consistent text.
- **B. Fix them inside the TR1.8 design revision,** and leave the amendment stale until its next revision.
- **C. Wait for step 2.4.**

**Recommended: A.** Steps 4.4, 4.8 and 4.9 read the amendment first.

## 4. What happens after you answer

1. The approved host work runs on dabeest, each piece in a booked window: TD9 about half a day, TD10 about a day.
2. One design revision is written from the delta and your answers. Claims whose rows stay unexecuted are removed.
3. Amendment 2 revision 8 is written (TD14 A).
4. Step 2.4: the dual Consistency and Safety review, plus a Surface review, then GO. Phase 3 (Windows SSH parity) is unblocked.

Steps 3.8 (key types), 3.5 (Pageant's exchange), 3.7 (the drop messages) and 4.5 can start now, whatever the answers.

## 5. Answer sheet

To accept every recommendation, reply with this line:

> TD1 A, TD2 A, TD3 A, TD4 A, TD5 A+B, TD6 A, TD7 B, TD8 B, TD9 A, TD10 A, TD11 B, TD12 B, TD13 accept, TD14 A

Otherwise, fill in this table:

| # | Short question | Options | Recommended | Your answer |
|---|---|---|---|---|
| TD1 | Who answers Negotiate or NTLM | A match 1.0.17 · B helper first · C login then helper | A | |
| TD2 | Digest | A refuse alone, ignore beside others · A' exact parity · B implement | A | |
| TD3 | Redirects on Windows | A follow · B error like 1.0.17 · C decide after HTTPS | A (+C check) | |
| TD4 | Challenge on POST | A refuse plainly · B resend body | A | |
| TD5 | RSA-only SSH | A match · B messages · C OpenSSL build | A+B | |
| TD6 | Home directories | A one home, non-ASCII works · B exact parity · C mixed | A | |
| TD7 | B17 Mac HTTPS | A attended re-run · B close on existing rows | B | |
| TD8 | Security keys, Pageant certificates | A run with hardware · B not qualified · C remove from list | B | |
| TD9 | Trust one test CA (host) | A attended · B supervisor · C decline | A | |
| TD10 | Machine proxy (host) | A serialized changes · B `host:port` only · C decline | A | |
| TD11 | Second account (host) | A approve · B decline | B | |
| TD12 | Agent service start (host) | A bounded start · B decline | B | |
| TD13 | Differences list | accept · strike items | accept | |
| TD14 | Amendment 2 | A revision 8 now · B in the design · C wait | A | |
