# TR1.8 freeze review (Consistency)

**Tuple, checked at start and end:** gwz-core `e5e8e804` and gwz-core-evidence `46b770f`. The only untracked items are the two out-of-scope ones. All nine SHA-256 values in section 1 of the review package match the files exactly. The package's own tuple is stale: it names lane `gwz-dev-tr18freeze`, core `2f27c05f` and evidence `12b46d90` (see P3-2).

**Method.** I checked the six amendment sites verbatim against revision 7 (`c37f0a5a…`) and revision 8 (`5d462c69…`). I spot-checked more than 40 cited rows against the raw tables and row JSON of RS, RA, RR, RM, RH, RP and RN, and the 2026-10-03 runs. That includes the four B16 authenticating rows, the 129-byte and 0-byte POST bodies, `0x80090346`, helper counts of 15 and 14, the `p407` helper count of 0, P03's SID and logon session, and the RS, RA, RP and RH row counts.

## Findings

**P2-1: The design contradicts itself on HTTPS through a machine proxy. Architectural.**
- **Where:** `GwzTransportWindowsParityDesign.md:122`, against `:143` and `:320`.
- **What conflicts:** Line 122 says an https URL "selects its proxy entry and bypass by the same WinHTTP rules, and this design takes that reading." Line 143 puts "the selection of the `https=` entry and the CONNECT leg" in the class "refused before any open". Line 320 says the https-origin selection and the CONNECT leg are "refused before any open (OQ11)".
- **Why it matters:** If that reading stands, every HTTPS remote behind a named machine proxy is refused. That breaks parity with 1.0.17 and is missing from the TD13 register. It also goes against TD10's rationale and OD15. Annex S.3 and guide lines 117–120 say instead that it is expected to work.
- **Fix:** Decide R13. Either admit it by the WinHTTP rule and delete "refused" at `:143` and `:320`, or refuse it and add a register difference plus a TD10/OD15 amendment.

**P2-2: The redirect rule ignores the accepted HTTPS bound. Not architectural.**
- **Where:** `:151`, `:159` and `:271` (register item 4).
- **What conflicts:** The design says "The transport follows all of them" and "every redirect is followed". The difference from 1.0.17 is said to be "only for a relative `Location` and for plain HTTP".
- **What the accepted text says:**
  - `GwzRemoteTransportHttpsDesign.md` §5 (`:185-200`) allows at most five GET hops and only 301/302/303/307/308. It admits HTTPS remotes only and refuses a downgrade.
  - TR1.6 §2.2 (`GwzTransportCredentialHelpersDesign.md:51`) and its C2 (`:234`) say "one validated discovery redirect" means that whole sequence.
- **Why it matters:** Amendment `:165` and `:470` still say "one". So the bound is stated differently in two places and settled in neither text. The plain-HTTP half is moot, because the transport admits no `http://` remote.
- **Fix:** In §7, cite HTTPS §5 and TR1.6 C2: five hops, HTTPS only, downgrade refused, 303 included but unmeasured. In register item 4, drop "and for plain HTTP" or say plain-HTTP remotes are not served.

**P2-3: Explicit-identity SSPI and Digest are "removed" while accepted documents and code still carry them. Architectural.**
- **Where:** `:180`, `:198` and `:311` (§14 item 6), and R9.
- **What still carries them:**
  - The accepted SSPI design: `dev-docs/GwzSspiDesign.md:93` ("`Identity::Explicit` owns …") and `:159` ("Digest requires Explicit identity").
  - gwz-sspi code: `gwz-sspi/src/worker/windows/conversation.rs:11-33,119` builds `SEC_WINNT_AUTH_IDENTITY_W`.
  - Plan step 4.4 (`GwzTransportWindowsParityPlan.md:446,448`) still says "pick the highest offered scheme (`Negotiate`, `NTLM`, `Digest`, `Basic`)". Its tests still expect "helper identity over `Negotiate`, no default-credential offer".
  - Plan step 4.8 (`:478-482`) still builds explicit-identity SSPI and offers WDigest.
- **Why it matters:** Both plan steps consume TR1.8 after GO, so they would implement the text TD1 and TD2 contradicted. Package §8 item 3 schedules only the plan's Appendix A. By the remove-dead-code rule, `Identity::Explicit` and the Digest package lose their only caller.
- **Fix:** As a GO condition, revise plan steps 4.4, 4.8 and 6.2. Either amend the SSPI design to retire `Identity::Explicit` and Digest from gwz-sspi (before 0.1.0, step 5.5), or record why they stay with no caller.

**P2-4: "Measured" overclaims the proxy environment rows. Not architectural.**
- **Where:** `:120`.
- **What it says:** All of `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`, `all_proxy` and `NO_PROXY` are ignored, "**measured** … (RA/b09-*)".
- **What was run:**
  - RA's runner (`auth_rows.py:157-158`) set only `HTTP_PROXY`, `http_proxy`, `HTTPS_PROXY`, `NO_PROXY` and `no_proxy`, all against an http origin.
  - RP's e01–e04 rows set `HTTP_PROXY`, `http_proxy`, `NO_PROXY`, `no_proxy` and `http.proxy` (`td10_proxy.py:289-292`).
  - `ALL_PROXY`, `all_proxy` and lowercase `https_proxy` were never set, and `HTTPS_PROXY` was never tried against an https URL.
- **Why it matters:** Annex S.3 and guide `:92` repeat this as fact.
- **Fix:** Narrow "measured" to what was set. Mark the rest as design (WinHTTP reads no environment). List them in L8 and §14.

**P3-1.** `:18` says revision 8 reconciled "§§3.5, 3.18 and 3.19". It changed §§3.5, 3.11, 3.14 and 3.19 (amendment `:11`; skim package items 1–6).

**P3-2.** `:20` says RH, RP and RN "are committed in gwz-core-evidence `12b46d90`". RN's run of record (`raw/native-run2-interactive`) landed in `e07f516`. The package tuple (section 1) also differs from the review tuple.

**P3-3.** `:194` says "In every row the NTLM Type 3 message is 88 bytes". The raw `auth_header_bytes` values are 88 only for `localhost` and `127.0.0.1` NTLM. Dotted-name NTLM is 103 (b15s-direct-internet-zone-name-ntlm-logon, rd-https-302-other-origin-ntlm-logon, rd2-…-name-change). The Negotiate rows are 121 or 119, including b16-gwz-negotiate-epa-server-right-dotted-name. Restate as "88–103 bytes, too short for an NTLMv2 response". The `ntlm_channel_binding_av` false finding holds in every row.

**P3-4.** `:90` says "with four keys loaded and only the last authorized". In the raw row the authorized key is ed25519, the third added and the **first** the agent lists (sshd accepts ED25519 at the first publickey). The row does not show that each identity is tried, which the RN README asserts.

**P3-5.** §11 marks P01 as "D" (`:248`). Part of it executed (the Local name, the ACL and the name-collision refusal; `:104` says "baseline P01, partial"), and plan `:308` says "P01 stays PARTIAL". It should be "P", claim removed.

**P3-6.** Amendment `:167` still says "the HTTPS confirmation is owed". So do skim package `:123` ("rows have not run") and checklist Q4 (`:134`). RH confirmed TD1 over HTTPS.

**P3-7.** The TD13 register omits two differences the design introduces:
- refusing a value with more than one bare entry, where 1.0.17 uses the first (R12(b));
- the 407 message change.

TD13 asks for the list "reconciled with these settled choices".

**P3-8.** The removed-claims and limitations lists do not match. §14 item 10 and package §6 omit L3's native-agent certificates, L7 (remote NTLMv2 EPA), L15 (the SPN), the own-address rule, the redirect/bypass interplay, multi-hop and 303 redirects, and the variables in P2-4.

**P3-9.** On TD12's approval scope:
- The decision record (`:25`) approves one bounded start.
- RN's README says the interactive scheduled-task mechanism is "a mechanism the TD12 approval does not cover". The run of record used it, with two more service starts and RDP enabled.
- The HKCU residue remains.
- `:87`/`:94` qualify B08 on this run without recording any approval. Add an entry to the operator discussion record.

**P3-10.** The design adopts OQ4(a) through TD12 (`:85`) but never says this supersedes the plan's 2026-10-08 OQ4(b) (plan `:674`, "admitting any pipe server that runs as SYSTEM is not such a rule").

**P3-11.** TD6 asks for the Unix SSH-path review. `:62` sends it to unnamed "implementation steps". Name the step, for example 3.1, and add it to the plan.

**P3-12.** R5's message rule (`:70`, "a key file that is not RSA PEM") does not cover x3-encrypted-rsa-pem, which is RSA PEM, encrypted and refused. Package R5 says such a key gets the agent remedy. State the rule: an agent, or `ssh-keygen -p -m PEM` with an empty passphrase.

**P3-13.** Guide `:3-8` points to the retired SSPI-only amendment. Guide `:141` has its own "120-second bound", although design `:218` says the guide cites the amendment and not a number of its own.

**P3-14.** `:155` contrasts "another origin (a different port)" with "the same server under another host name". The cross-origin non-forwarding rule needs "origin" defined as scheme, host and port, so that a host-name change counts as cross-origin.

## R answers (Consistency axis)

- **R1:** Yes. All six superseded texts match revision 7 verbatim, and all six replacements are in revision 8. Extending to §§3.11, 3.14 and 3.19 is within TD14: it is the same contradicted claim and adds no mechanism. Fix P3-1 and P3-6.
- **R2:** Faithful. TD1 says "do not … downgrade to helper credentials". RH and RA show 15 and 14 helper calls.
- **R3:** Consistent with TD1. A 401 after the final token is what RH's rejected rows show.
- **R5:** Naming an agent is within TD5 B's intent, but see P3-12.
- **R6:** Yes. §6, §11 (B10), the delta and the decision record agree with `RP/p407-*` (1 request, no `Proxy-Authorization`, 0 helper calls).
- **R7:** The §11 statuses match the archives. The delta's §4 counts are superseded rather than restated (its B18 "in full" was inconsistent when written). Say so in §11.
- **R8:** The accepted HTTPS §5 and TR1.6 C2 already settle the cap: five hops, HTTPS only. The design must cite them (P2-2). Reading OD16 and OQ4 together matches TR1.6 §2.2. Unmeasured non-forwarding is correctly listed in §14 item 12 and L6.
- **R9:** Not confirmed (P2-3).
- **R10:** Under the SSH clock amendment, only the password route enters the local phases, so a Pageant wait is Network phase, under the aggregate and stall bounds. That gives no hang, but the stall bound can expire before the user confirms. Say this in §5 and drop the guide's OPEN ITEM.
- **R12:**
  - (a) and (c) match RP g03, g09 and b27–b29.
  - (b) is consistent with OQ11's "ambiguous" but is unregistered (P3-7).
  - (d) is parity with x18–x20, and the token has rows, so the refuse-unmeasured rule does not apply. The rest is for Safety.
- **R13:** The text contradicts itself (P2-1) and must be decided before GO.
- **R15:** "E (loopback)" is consistent with how B15 is labelled, with the caveat carried in L7, but the byte figure is inexact (P3-3).
- **R16:** Register item 4's "plain HTTP" half is moot (P2-2). The amended amendment row is correct.
- **R17:** The labels are honest except P01 (P3-5).
- **R18:** Confirmed. RA/b14-digest-only-no-helper and RH/b14s-digest-only-no-helper each show one GET, 0 helper calls and `GWZ could not acquire credentials`. Annex S.6 item 2 is not overstated.
- **Section 11 GO-rule reading:** It matches plan step 2.4 word for word.

Verdict: NO-GO