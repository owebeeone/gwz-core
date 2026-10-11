# TR1.8 freeze review, round 2 (Surface)

**Tuple.**
- gwz-core is at `17ec75f1` and gwz-core-evidence at `928f6d4`, as stated, at both start and end.
- Root is at `f16d9fb0`, not `b3e6de03`. It is one `gwz merge` after `b3e6de03`, which is its ancestor.
- That merge touches only `gwz.conf/gwz.lock.yml` (the `dirty: true` flags become `false` for gwz-core and gwz-core-evidence; commits unchanged), `markers/conf-integrity.yml` and one new marker file. No guide or annex input changed, so the review stands on the stated content. The coordinator should re-pin the tuple to `f16d9fb0`.

**Inputs:** `GwzTransportWindowsUserGuide-DRAFT.md` (cited as G:line), and Annex S of the 2.4 package (cited as S:line, lines 204–270). I used the remediation plan only for the ID map: U*n* = S*n*.

## 1. Closure of round-1 findings

| Mine | U | Status | Evidence |
|---|---|---|---|
| S1 R13, HTTPS through a proxy | U1 | **not closed** | G:224-226 and S:228 say the CONNECT tunnel "was measured with gwz 1.0.17", which is the native route, not GWZ's transport. The OPEN ITEM label is gone but the item itself is still open. S:250 also leaves out IP-address servers and redirects through a proxy; G:370-375 does not carry that over. |
| S2 corporate proxy dead ends | U2 | closed | G:206-215 (what is supported, netsh set/import, direct-failure error, PAC); G:249-252 (407 refused on both transports, measured). The new D1 case is reported separately as N1. |
| S3 Digest pointer to native | U3 | closed | G:267-270 |
| S4 Windows account rejected, no next step | U4 | **partially closed** | It is now a listed change (G:352), and G:273-278 says no setting changes it. The error still only names the scheme. A user who signed in with a token through the helper on 1.0.17 is told nothing to do. |
| S5 limits and defaults; R10 | U5 | closed | G:54-70 is a table of defaults and precedence. G:119-123 says the Pageant wait counts as interaction time. A P3 remains (N5). |
| S6 Pageant beats `SSH_AUTH_SOCK` | U6 | closed | G:77-81 |
| S7 key pin blocks the agent fix; agent setup steps | U7 | closed | G:90-117 |
| S8 `known_hosts` path, PowerShell encoding, fingerprint check | U8 | closed | G:147-162 |
| S9 redirect hazard | U9 | closed | G:332-340 (`--verbose` shows each redirect, at most five, never https→http, credentials stay with their origin). G:311-321 together with G:338-339 covers the Windows sign-in being answered at the new address. Wording nit in P3-c. |
| S10 helper queue | U10 | closed | G:295-306 |
| S11 process text and TR1.5 reference | U11 | closed | G:3-5 now holds only the DRAFT banner; G:47-52 quotes the "ignoring" note. |
| S12 change table and disclosures | U12 | closed | G:125-134, G:346-368 |
| S13 precedence and invalid values | U13 | closed | G:28-39 for the setting sources. The `http://` routing exception is reported separately as N3. |
| S14 misplaced sentence | U14 | **partially closed** | G:286-289 now sits inside the "Basic only, no helper" bullet, after its message block. It still points to the native transport with no evidence that native helps. P3. |
| S15 Python timeout | U15 | closed | G:68-70 |
| S16 rsa-sha2 and certificate messages | U16 | closed | G:194-201 |
| S17 PEM conversion in place | U17 | closed as written | G:177-192. A follow-on gap is reported as N2. |
| S18 no opt-out for Windows sign-in | U18 | closed | G:317-318 |
| S19 stopped agent and no-helper messages | U19 | closed | G:103-109 and G:279-284. Wording issues are in P3-a and P3-b. |
| S20 `--verbose` | U20 | closed | G:20-21 |

## 2. First-day walkthrough (guide only)

- **SSH clone with an ed25519 key: now completes.**
  - The key file is refused, and G:175 says to load it into an agent. G:94-109 gives the service commands and the stopped-service message.
  - G:115 covers a pinned key.
  - For an ed25519-only `known_hosts`, the message prints the file path (G:148-162), and G gives safe commands for both shells and a fingerprint check.
  - Snags (P3-a): the `ssh-add` example names `id_rsa`, and `-StartupType Manual` does not survive a reboot.
- **HTTPS clone behind a corporate proxy: stuck in two places.**
  - A static WinHTTP proxy is the documented path, but its evidence is 1.0.17 (S1).
  - With PAC or a 407 proxy, G says clearly there is no route. That is acceptable.
  - **D1, a mapping such as `http=proxy:8080` only:** G:228-231 says only "GWZ refuses and names the machine proxy setting". G gives no message text and no next action. I cannot tell whether to ask the administrator for an `https=` entry or to use `--transport native`, which per G:230 goes direct as 1.0.17 did (N1).
- **Clone from a redirecting server: completes.**
  - `--verbose` shows each redirect.
  - A redirect from https to http is refused, but G does not quote the message or say what to do next, such as using the `http://` URL directly or fixing the server (P3-d).
- **D2, a plain `http://` remote:** G:13-17 says "GWZ prints a warning" but does not quote it. G does not say:
  - whether the warning can be silenced;
  - that native's defaults (3-second timeout, no retry, `--max-retries` ignored) then apply to that member;
  - what happens when I asked for `--transport gwz` explicitly (N3).

## 3. New or remaining P2

- **N1 (G:228-231, S:228, S:264). The D1 refusal is not actionable.**
  - **What the user meets:** a refusal "naming the setting", with no quoted text and no fix. Such mappings worked on 1.0.17, so this case affects users who are upgrading.
  - **Correction:** quote the message and give both routes. The administrator can add an `https=` entry (`netsh winhttp set proxy proxy-server="http=p:8080;https=p:8080"`). If native's direct connection is acceptable, say so and give `--transport native`; if it is not wanted, say "no user-side route".
- **N2 (G:171-192). The PEM copy is never connected to GWZ.**
  - G does not say which key files GWZ looks for when no key is pinned. After making `id_rsa_pem`, retrying still offers the original file and fails the same way.
  - **Correction:** list the default key file names. After the conversion, add `gwz auth identity <remote> --set ~/.ssh/id_rsa_pem`, or `--identity`, with its `--unset` pair. Note that PowerShell's `$HOME` is the user profile, which can differ from GWZ's selected home (G:138-139).
- **N3 (G:10-17, G:28-30). The `http://` route overrides the stated order.**
  - G:28 says `--transport` comes first, and G:10-11 says GWZ never changes transports. Yet an `http://` remote silently uses native even under `--transport gwz`.
  - **Correction:**
    - State the exception in the order section.
    - Quote the warning, including that this remote uses the native transport.
    - Say whether the warning can be silenced, and state that native's timeout and retry defaults apply to that member.
- **S1 still open.** Either measure HTTPS through a machine proxy with GWZ's own transport before freeze, or say so plainly in G: "measured with gwz 1.0.17; not yet tested with GWZ's transport". Also carry S:250's gap for IP-address servers and redirects through a proxy into G:370-375.
- **S4 remainder (G:272-274).** Quote the rejection message, and give a next action, for example: "<host> rejected your Windows account (NTLM); ask its administrator for access, or use an SSH remote."

## 4. P3

- **P3-a (G:97, G:100, G:107):**
  - Suggest `-StartupType Automatic`, or say the service has to be started again after each restart.
  - The `ssh-add` example should not use `id_rsa` in what is otherwise a key-type-neutral passage.
- **P3-b (G:283).** The no-helper message is good, but `<name>` leaves the user guessing. Name `manager` (Git Credential Manager, which ships with Git for Windows) as the usual value.
- **P3-c (G:337-339).** Make the hazard explicit: the new address receives your Windows sign-in if it challenges with NTLM or Negotiate.
- **P3-d (G:334).** Quote the https→http refusal message and its next step.
- **N4 (G:61 vs G:65-66).** `--ssh-timeout` defaults to 9 on GWZ's transport and 3 on native, and on GWZ's transport it also governs HTTPS read stalls and the HTTPS setup budget. The name misleads HTTPS users. The stall error should name `--ssh-timeout`, and the help text should state both defaults.
- **N5 (G:119-123, G:299, G:307).** The fixed interaction limit and the queue wait `<m>` are referred to but their values are not stated. Round 1's text said 120 s. State the numbers.
- **N6 (G:323-328).** G does not say what happens when `GIT_SSL_CAINFO` or `SSL_CERT_FILE` names a missing or unreadable file. G:301's message should print the member's URL rather than saying "this member's URL".

## 5. Summary

- Of my 20 round-1 findings, 17 are closed, 2 are partially closed (S4, S14) and 1 is not closed (S1).
- The rewrite introduced three new P2s: N1 (the D1 refusal), N2 (the converted key copy) and N3 (the `http://` route versus the stated order).
- All of these are fixes to wording and evidence, plus one measurement (S1). None reopens an operator decision D1–D4.
- This is round 2 of 2 under the cap.

Verdict: NO-GO