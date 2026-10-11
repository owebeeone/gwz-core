# TR1.8 freeze review (Surface)

**Inputs read:** `gwz-core/dev-docs/GwzTransportWindowsUserGuide-DRAFT.md` (cited as G:line), Annex S of `GwzTransportWindowsParityDesign-ReviewPackage-2.4.md` (cited as S:line), and `gwz help` output. The installed gwz is 1.0.17, so its help has no `--transport` and I could not check the 1.1.0 help surface. Its help does show `--identity` ("no agent fallback"), `gwz auth identity --set/--unset`, `--ssh-timeout` (default 3) and `--verbose`.

## First-day walkthrough

| Task | Where I get stuck |
|---|---|
| SSH clone with an ed25519 key | The key file is refused, and the message says "load into an agent". G does not say how to start the OpenSSH agent service, which was disabled on the test machine. If the key is pinned with `--identity` or `gwz auth identity`, loading it into an agent changes nothing (S7). My `known_hosts` is ed25519-only, so I am told to run `ssh-keyscan -t rsa`, but G does not say which file to write to, how to do it safely in PowerShell, or how to check the fingerprint (S8). If PuTTY's Pageant is running, my OpenSSH agent is never used (S6). |
| HTTPS clone behind a corporate proxy | With PAC or auto-detect, GWZ refuses. If WinHTTP is unset, GWZ goes direct and fails without mentioning a proxy. A proxy that asks for a login (407) is refused and I am pointed to native, which G never says will work. HTTPS through a proxy is untested (R13). I cannot finish the clone (S1, S2). |
| Clone from a redirecting server | The clone works. But I cannot "review the redirect" as G:150 tells me to, because G never says where a redirect is shown. My Windows identity is offered to the new host without my knowing (S9). |

## Findings

**P2**

- **S1 (G:117-121, S:198).** R13 is open: HTTPS through a machine proxy and the `https=` entry are untested. This is the main corporate-proxy case. **Impact:** 1.1.0 would ship the default transport unverified for the most common proxy setup. **Correction:** close R13 with the proxy product tests before freeze. Otherwise, state in G that HTTPS through a proxy is unsupported in 1.1.0 and that users should use `--transport native`, verified.
- **S2 (G:91-97, 104-105, 123-126).** Most corporate proxies are PAC/auto-detect, browser-only, or ask for a login (407). The user meets one of three outcomes: refused, a plain network error that does not mention the proxy, or refused with a pointer to native. G does not say what native does with PAC or 407. S:228 says environment and git proxies were already ignored on 1.0.17. The only fix G gives is an administrator command that inspects the setting. **Impact:** these users have no route that works. **Correction:**
  - Open the section with what is supported: one static WinHTTP proxy that does not ask for a login.
  - Give the administrator's fix: `netsh winhttp import proxy source=ie` or `netsh winhttp set proxy`.
  - State the measured native outcome for PAC and 407, and name native in the 407 message only if it works.
  - When a direct connection fails while the browser has a proxy and WinHTTP has none, say so in the error.
- **S3 (G:132-134, G:166, S:181, S:220).** A server that offers only Digest is refused, and the message names "how to switch transports". S.6 row 2 shows that 1.0.17, which is the native transport, also fails on Digest-only. **Impact:** the error sends the user to a fix that does not work. **Correction:** the message should say Digest is unsupported on both transports and suggest SSH or a change on the server side. Drop the native pointer for this case.
- **S4 (G:134-136, S:182, S:221).** If the server rejects your Windows account, the operation stops and no helper credential is tried. On 1.0.17 the helper could be asked. **Impact:** this regresses servers that offer Negotiate/NTLM alongside Basic where the user signs in with a token (PAT) through the helper. G does not mark it as a change and gives no next step. **Correction:**
  - List it as a change from 1.0.17.
  - Make the error name `--transport native`, or a documented way to prefer the helper.
  - Say whether any setting can make GWZ skip Windows sign-in for a host.
- **S5 (G:39-42, G:57-61).** "Adjust the existing setup limits" names no setting. G gives native's defaults but not the GWZ transport's:
  - the setup budget (only implied to be 30 s);
  - the connect and read timeouts;
  - the retry count.

  G also does not say how `--ssh-timeout` (help: default 3) and `--max-retries` relate to them. R10 is open. **Impact:** the error is unactionable and the defaults are missing. **Correction:** name each GWZ-transport limit, its default, its flag or config, and its precedence. Close R10, or delete the "adjust" advice and state the limitation.
- **S6 (G:47-51, G:57-58, S:202).** A running Pageant wins over an explicit `SSH_AUTH_SOCK`. A user with keys in the OpenSSH agent and an idle PuTTY Pageant is told to load the key into Pageant. **Impact:** a misleading fix. G documents no way to select the OpenSSH agent and does not say whether 1.0.17 behaved the same. **Correction:**
  - State "to use the OpenSSH agent, close Pageant".
  - Make the message name Pageant as the selected source and say that `SSH_AUTH_SOCK` was not consulted.
  - State the 1.0.17 behaviour.
- **S7 (G:77-81, gwz help `--identity`, `gwz auth identity`).** An ed25519 or encrypted key pinned with `--identity`, `--remote-identity` or a stored `gwz auth identity` is refused with "load the key into an agent". The pin disables agent use, so the retry fails the same way. G also gives no agent setup steps. **Impact:** a fix that cannot work, on day one. **Correction:**
  - The message and G must say to remove the pin first: `gwz auth identity <remote> --unset`, or drop `--identity`.
  - G should give the agent steps: check with `Get-Service ssh-agent`, have an admin run `Set-Service ssh-agent -StartupType Manual` and `Start-Service ssh-agent`, then `ssh-add`. For Pageant, load the key.
- **S8 (G:63-67, G:74-77, S:207).** An ed25519-only `known_hosts` entry, which is common after first connecting with Windows OpenSSH, sends the user to `ssh-keyscan -t rsa`. G does not give:
  - the target file, which is the selected home's `.ssh\known_hosts`. This can differ from OpenSSH's `%USERPROFILE%` when `HOMEDRIVE`/`HOMEPATH` point to a network home.
  - a safe append command. In Windows PowerShell 5.1, `>>` defaults to UTF-16.
  - a fingerprint check against the provider.

  G also does not say whether the error names the `known_hosts` path and home source it used. **Impact:** a corrupted or wrong `known_hosts` file, or an unverified host key accepted on trust. **Correction:**
  - Make the message print the resolved `known_hosts` path and the home source.
  - Give exact commands for PowerShell (`| Add-Content -Encoding ascii`) and for Bash.
  - Tell the user to compare the fingerprint with the provider's published one.
- **S9 (G:146-159, S:185, S:189).** G:156 promises that the first address's credentials never go to a different address. But the redirect target, which may now be another host or plain HTTP, can still receive the user's Windows sign-in if it challenges with NTLM. G:150 asks the user to review the redirect, but G never says where gwz shows it. Downgrade from HTTPS to HTTP and the hop limit are not stated. **Impact:** a misleading reassurance on a credential-exposure path. **Correction:**
  - State that the Windows identity is offered to a redirect target that challenges.
  - Say where the redirect is reported (a warning line, or `--verbose`).
  - State what happens on HTTPS→HTTP, the hop limit, and what a redirect after the first request produces.
- **S10 (G:140-143).** "A queued lookup can exhaust its allocation budget" uses internal vocabulary. "Sign in with the helper once" does not say how. G does not show the message the user sees. A first HTTPS clone of a workspace with more than 8 members reaches exactly this case. **Impact:** an unactionable failure on day one. **Correction:** quote the message and give a concrete warm-up step, for example `git ls-remote <url>` or a fetch of a single repository first. Say whether `--max-per-host 1` avoids it.

**P3**

- **S11 (G:3-8, 13-15, 37, 59-61, 117-121).** Process text that must not ship:
  - "MAIN import" and "Full Windows NO-GO persists";
  - an SSPI link that points outside the repo;
  - "as TR1.5 specifies", which leaves the user not knowing what a workspace-level `gwz.transport` prints;
  - "review package R10/R13/R14".

  Remove them, and quote the message for an ignored setting.
- **S12 (G overall vs S:215-228).** G has no consolidated list of changes from 1.0.17. It is missing S.6 rows 2, 3, 5 and 8 as changes. It also omits the "unchanged on purpose" disclosures in S:228: a Pageant owned by another user is not refused, and the agent pipe's server is not identity-checked. G:47 ("your logon session") reads as a stronger guarantee than that. Add the S.6 table and the disclosures to G.
- **S13 (G:28-37).** G does not say outright that the environment variable beats global config (only implied), or what happens with an invalid `GWZ_TRANSPORT` or `gwz.transport` value such as `Native` or a typo. State both.
- **S14 (G:165-166).** "Unsupported schemes name the offered scheme and the native transport option" sits in the macOS/Linux paragraph, so it is unclear which platforms it covers. Move it into the Windows authentication paragraph.
- **S15 (G:41-42).** The "Python … 9 seconds" timeout is unexplained to CLI users. Name the Python package or move it to that package's docs.
- **S16 (G:83-87).** For a server that accepts only `ssh-rsa`/`rsa-sha2-256`, and for an RSA certificate from an agent, G gives neither the message nor the fix (use an ed25519 key in an agent, or native, if verified). Add both.
- **S17 (G:79-80).** `ssh-keygen -p -m PEM` rewrites the key in place, and an encrypted PEM key is still refused, so in effect it means removing the passphrase. Tell the user to convert a copy, prefer an agent, and note how to convert back.
- **S18 (G:146-150).** Offering Windows credentials to any host has no opt-out. That is the decided behaviour, but G should say plainly that no setting turns it off, and whether native differs.
- **S19 (G:53-55, G:130-131).** G does not say what the user sees when the OpenSSH agent service is stopped, or on a Basic-only server with no helper configured. Quote both messages.
- **S20 (G overall).** `--verbose`, which shows per-attempt authentication diagnostics, is never mentioned. Point to it from the error paths.

## OPEN ITEMs

- **R10 (G:59-61):** must close, or the "adjust limits" advice must be removed (S5).
- **R13 (G:117-120):** must close before freeze (S1).
- **R14 (G:121):** acceptable to ship if reworded as a plain "not tested" limitation.

Verdict: NO-GO