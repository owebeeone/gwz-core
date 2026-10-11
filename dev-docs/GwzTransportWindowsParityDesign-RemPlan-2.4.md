# TR1.8 freeze (step 2.4): remediation plan, round 1

**Object.** gwz-core `e5e8e804`, gwz-core-evidence `46b770f`, as pinned by `GwzTransportWindowsParityDesign-ReviewPackage-2.4.md`. The paperwork lane has since updated the plan, amendment 2 and the pinned hashes on main (`abf76ab7`).

**Reviews.** Consistency, Safety and Surface (Opus, blind): all three NO-GO on 2026-10-11. This plan is the lane owner's condensed record of their findings, with IDs as each reviewer gave them: Consistency P2-x and P3-x, Safety S1–S10, Surface (here **U1–U20**, renumbered from the reviewer's S1–S20 to avoid clashing with Safety's).

**Blind convergence:**
- **R13** (HTTPS through a named machine proxy) was found by all three reviewers: Consistency P2-1, Safety S3, Surface U1.
- **The plain-HTTP boundary** was found by all three: Safety S1 (P0), Consistency P2-2, Surface U9.

## Operator decisions (2026-10-11), controlling

- **D1, R13: measure, then admit.**
  - Run the HTTPS-origin machine-proxy rows on released 1.0.17 under the TD10 approval, with the same guarded transactions: booking lock, the exact prior state saved, a SYSTEM deadline guard armed before every mutation, and the restore verified after every row.
  - Then admit, by WinHTTP's rule, a bare proxy entry or an explicit `https=` entry for an HTTPS URL.
  - Refuse a mapping that has no entry applying to https (for example `http=` only) instead of going direct. This is Safety's preference. It is a recorded difference from 1.0.17, in the register and the guide.
- **D2, plain `http://` remotes on Windows: native route plus a warning.** The transport serves `https://` only and refuses a redirect from HTTPS to HTTP. An `http://` remote goes by the native route, with 1.0.17's behaviour: Basic in clear, and NTLM with no channel binding. State this plainly in §7, the register, §13 and the guide.
- **D3, explicit-identity SSPI and Digest: keep the code, revise the 1.1.0 plan.** TD1 and TD2 scope 1.1.0 only. 1.2.0 is a separate set of work, so nothing meant for a later release is removed.
  - Plan steps 4.4, 4.8 and 6.2 must not build helper-identity SSPI or offer Digest in 1.1.0.
  - The design and the plan record that gwz-sspi's `Identity::Explicit` and Digest stay on purpose, for use after 1.1.0, with no 1.1.0 caller.
  - The SSPI design is annotated, not amended to remove them.
- **D4, Windows CA trust: keep and document.** TR2.7 as accepted stands. The transport adds `GIT_SSL_CAINFO` and `SSL_CERT_FILE` roots beside Schannel's. 1.0.17 on Windows ignores both (run `2026-10-08-ca-trust-parity`). State the trust sources in §7, add the difference to the TD13 register, and warn in the guide: a CA file in the environment is trusted, so a corporate interception CA there will be trusted too.

## Dispositions

### Design text (`GwzTransportWindowsParityDesign.md`)

| ID | Disposition |
| --- | --- |
| S1 (P0), C P2-2, U9 | State in §7 that the transport serves `https://` only and refuses HTTPS→HTTP. Apply D2. Register item 4 becomes "relative `Location` over HTTPS", and plain HTTP is noted as the native route (D2). Fix §14 item 7, guide line 146 ("any HTTPS host") and Annex S row 4. |
| C P2-2, S8 | Cite the accepted redirect bound: `GwzRemoteTransportHttpsDesign.md` §5 and TR1.6 C2. That is at most five GET hops; only 301, 302, 303, 307 and 308; HTTPS only; no downgrade; no redirect after a native exchange (`https_worker/prepare.rs:501`). 303 is unmeasured on 1.0.17. Reconcile the amendment's "one validated discovery redirect" (amendment `:165`, `:470`) to this sequence. Name the product tests as proof: `production_configured_basic_redirect_requeries_without_forwarding_credentials` and `gh_redirect_looks_up_each_origin_and_never_reuses_auth_header`, plus a native redirect-after-exchange test. |
| C P2-1, S3, U1 | Apply D1. Remove "refused" at `:143` and `:320` for the cases D1 admits. Add the http-only-mapping refusal and its register row. Mark the rows `PENDING-ROW R13-https-proxy` until the measurement lands, then fill them in. |
| C P2-3 | Apply D3: revise plan steps 4.4 (no "Digest" in the scheme list; no helper-identity-over-Negotiate tests), 4.8 (no explicit-identity SSPI and no WDigest in 1.1.0) and 6.2. Record in the design and plan that the gwz-sspi code is retained for post-1.1.0. |
| C P2-4 | Narrow "measured" at `:120` to the variables actually set: `HTTP_PROXY`, `http_proxy`, `HTTPS_PROXY` (against an http origin), `NO_PROXY`, `no_proxy` and `http.proxy`. Mark `ALL_PROXY`, `all_proxy`, lowercase `https_proxy`, and `HTTPS_PROXY` against an https origin as design (WinHTTP reads no environment), and add them to L8 and §14. If the D1 run can set them cheaply against the https origin, do so and cite it. |
| S2 | Apply D4. |
| S4 | Classify the Pageant confirmation wait as the SSH clock amendment's `LocalInteraction`: the network stall and aggregate bounds pause, with a finite deadline at the amendment's interaction allowance. It is not a new 120 s figure. Close guide OPEN ITEM R10 accordingly. Answers R10 for both Consistency and Safety. |
| S5 | Add a limitation row and a guide sentence for the TD12 pipe-squatting hazard. A local account can create `\\.\pipe\openssh-ssh-agent` while the service is not running (the default), and so learn the user's identity, the sign-request metadata and the offered keys. It cannot obtain a signature. Pin the admission grammar to exactly `\\.\pipe\<name>`, and refuse any other server component. |
| S6 | State the retained cross-SID barrier: the per-request mapping DACL (caller SID and SYSTEM). Set the mapping's owner explicitly to the caller's user SID, not the token default, because an elevated process defaults to Administrators. Note the residual: same-SID, lower-integrity code. |
| S7 | Define one exchange's boundary. A mid-exchange 401 with no continuation token for the selected scheme, or a 3xx, is rejection. The eight rounds are per operation. No new exchange starts within the operation except TR2.1's retry on network loss before effect. |
| S9 | Name the own-address rule: an IP-literal URL host that equals an interface address enumerated once at capture goes direct. Names are never resolved (b04). Anything else goes through the proxy. Close guide R14. |
| S10 | State as a limitation: a rejection is terminal per operation, but several operations or members can each produce one rejected NTLM attempt against a host. No per-runtime memory is added in 1.1.0. |
| OD16 wording (Safety) | Use the amendment's terms at `:174`: a NetNTLMv2 response, crackable offline, and relayable to services that do not enforce EPA or signing. Mention that administrators can restrict outgoing NTLM by Windows policy. |
| C P3-1 | `:18`: revision 8 changed §§3.5, 3.11, 3.14 and 3.19. |
| C P3-2 | `:20` and the package tuple: RN's run of record landed in evidence `e07f516`. Restate the package tuple as current. |
| C P3-3 | `:194`: "88–103 bytes (Negotiate 119–121), too short for an NTLMv2 response". The `ntlm_channel_binding_av` false finding holds in every row. |
| C P3-4 | `:90`: the authorized key is the ed25519, which is the first one the agent lists. The row does not show that each identity is tried; fix the RN README claim too. |
| C P3-5 | §11: P01 is "P" (partial), not "D". The claim is removed. |
| C P3-6 | Amendment `:167`, skim package `:123` and checklist Q4: RH confirmed TD1 over HTTPS. |
| C P3-7 | TD13 register: add the refusal of a value with more than one bare entry (R12(b)), the 407 message change, D1's http-only refusal, D2 and D4. |
| C P3-8 | Reconcile §14 item 10 and package §6 with L3 (native-agent certificates), L7 (remote NTLMv2 EPA), L15 (SPN), the own-address rule, redirect and bypass interplay, multi-hop and 303, and the proxy variables from C P2-4. |
| C P3-9 | Add to the decision list's discussion record that the operator approved, in chat on 2026-10-11, the in-session TD12 run: an interactive-logon scheduled task in the operator's RDP session, the two extra service starts, and RDP left enabled. The HKCU residue was left in place by the operator's choice. |
| C P3-10 | State that TD12's adoption of OQ4(a) supersedes the plan's OQ4(b) of 2026-10-08. |
| C P3-11 | Name the step for TD6's Unix SSH-path review (3.1) and add it to the plan. |
| C P3-12 | R5's rule covers an encrypted RSA PEM key: the remedy is an agent, or `ssh-keygen -p -m PEM` with an empty passphrase on a copy. |
| C P3-13 | The guide points at the current amendment, not the retired SSPI-only one. Remove the guide's own "120-second bound" (see S4). |
| C P3-14 | `:155`: define "origin" as scheme, host and port, so that a host-name change is cross-origin. |

### User guide (`GwzTransportWindowsUserGuide-DRAFT.md`) and messages

| ID | Disposition |
| --- | --- |
| U1 | Covered by D1. The guide states what is supported. |
| U2 | Open the proxy section with what is supported: one static WinHTTP proxy that does not ask for a login. Give the administrator's fix (`netsh winhttp import proxy source=ie`, or `netsh winhttp set proxy`). State the measured native outcome for PAC and 407 (TD10 rows), and name native in the 407 message only if native works there. When a direct connection fails while WinHTTP has no proxy, say so in the error. |
| U3 | The Digest-only message says Digest is unsupported on both transports, and suggests SSH or a server-side change. No native pointer. |
| U4 | List as a change from 1.0.17: a rejected Windows sign-in does not fall back to helper credentials. Say no setting changes this in 1.1.0. Name `--transport native` only if native measurably consults the helper; RA shows it does for NTLM-only and Negotiate+Basic. |
| U5 | Name each GWZ-transport setup limit with its default, flag or config, and precedence: the setup budget, the connect and read timeouts, the retry count, and how `--ssh-timeout` and `--max-retries` relate. Or delete the "adjust" advice and state the limit. |
| U6 | "To use the OpenSSH agent, close Pageant." The message names Pageant as the selected source and says `SSH_AUTH_SOCK` was not consulted. State 1.0.17's behaviour (RS b06: Pageant wins). |
| U7 | A key that `--identity`, `--remote-identity` or `gwz auth identity` pins: remove the pin first (`gwz auth identity <remote> --unset`, or drop `--identity`). Give the agent steps: `Get-Service ssh-agent`, then as administrator `Set-Service ssh-agent -StartupType Manual` and `Start-Service ssh-agent`, then `ssh-add`; for Pageant, load the key. |
| U8 | `known_hosts`: the message prints the resolved path and the home source. Give exact commands: PowerShell `ssh-keyscan -t rsa <host> \| Add-Content -Encoding ascii <path>`, and Bash. Tell the user to compare the fingerprint with the provider's published one. |
| U9 | Covered by S1. Say where a redirect is reported (a warning line, or `--verbose`). |
| U10 | Quote the allocation-budget message. Give a warm-up step (`git ls-remote <url>`, or fetching one repository first) and say whether `--max-per-host 1` avoids it. |
| U11 | Remove process text: "MAIN import", "Full Windows NO-GO persists", the external SSPI link, "as TR1.5 specifies" (quote the message for an ignored setting instead), and "review package R10/R13/R14". |
| U12 | Add the S.6 changes table and the "unchanged on purpose" disclosures to the guide: Pageant ownership is not refused, and the agent pipe is not identity-checked (S5 and S6). |
| U13 | State that the environment beats global config, and what happens with an invalid `GWZ_TRANSPORT` or `gwz.transport` value. |
| U14 | Move "unsupported schemes name…" into the Windows authentication paragraph. |
| U15 | Name the Python package behind the 9-second timeout, or move that text to its docs. |
| U16 | Give the message and the fix for an `ssh-rsa`-only or `rsa-sha2-256`-only server, and for an RSA certificate from an agent. |
| U17 | `ssh-keygen -p -m PEM` rewrites the key in place. Convert a copy, prefer an agent, and note how to convert back. |
| U18 | Say plainly that no setting turns off offering Windows credentials, and whether native differs. |
| U19 | Quote the messages for a stopped OpenSSH agent service and for a Basic-only server with no helper configured. |
| U20 | Point to `--verbose` from the error paths. |

### Messages that are code

U3, U4, U6, U7, U8, U16 and U19 describe messages the transport prints. This round fixes the **text the design and guide promise**. The message code changes belong to the steps that own each message (3.2b, 3.3, 3.8 or 4.4). List each owed message change, with its owner step, in §14.

## Measurement for D1 (dabeest, TD10 approval)

Released 1.0.17 (`gwz.exe` `cbab5e9b…`) runs against a real public HTTPS origin, for example `https://github.com/owebeeone/gwz-sspi.git` with `ls-remote`. Windows already trusts it, so no CA change is needed. Each case runs through the TD10 fixture proxy, which records the CONNECT target, or confirms there was no CONNECT. Use the TD10 runner and guard (gwz-core-evidence `2026-10-10-tr18-td10-machine-proxy/runner/`), one guarded transaction per row.

The rows:
- **Proxy forms:** a bare `host:port`; `https=host:port`; `http=host:port` only; `http=a;https=b`; `socks=` only.
- **Bypass:** an entry that matches the https origin.
- **Environment:** `HTTPS_PROXY`, `ALL_PROXY` and lowercase `https_proxy` set against the https origin (C P2-4).

Archive the results as `2026-10-11-tr18-r13-https-proxy`. Fill the `PENDING-ROW R13-https-proxy` markers from it.

## After

Recompute every hash the package pins. Then the same three reviewers re-verdict their own findings against the revised tuple. This is round 2 of 2.
