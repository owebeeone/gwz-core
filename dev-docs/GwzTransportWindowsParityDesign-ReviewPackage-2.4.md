# Review package: Windows transport parity design (TR1.8), plan step 2.4

Date: 2026-10-11. Status: **the package for step 2.4's dual review, restated after round 2 of 2.** Round 1 ran on 2026-10-11 against the first tuple of this package: Consistency, Safety and Surface (blind) were all NO-GO. Round 2 (also 2026-10-11) was NO-GO from all three again; its remediation (section 2c) is documents only and is uncommitted in the lane `gwz-dev-tr18fix2`, whose working tree the hashes below describe. By the operator's two-reviewer rule of 2026-10-11 Surface is retired as a reviewer: its open findings are fixed here and Consistency verifies them. This package decides nothing and authorizes no implementation, commit, tag, push or publication. The lane owner dispatches the reviewers; this package does not.

The object is [`GwzTransportWindowsParityDesign.md`](GwzTransportWindowsParityDesign.md), the DRAFT revision of 2026-10-11 (plan step 2.1's output, with the 2026-10-10 host rows filled in). The plan's step 2.4 says what the three reviews are: Consistency and Safety on the design with the evidence from steps 2.2, 2.3 and 2.3b, and Surface on the user guide alone. At most two architectural remediation rounds follow. All P0, P1 and P2 findings block.

## 1. The exact documents and the tuple

All paths are under `gwz-core/dev-docs/` of the lane `gwz-dev-tr18fix2`. SHA-256 values are of the files as they stand in that lane's working tree after the round 2 remediation, recomputed on 2026-10-11 (the objects are uncommitted); the lane owner recomputes them again at the commit that settles the tuple and replaces them here.

| Role | Document | SHA-256 (first 16) |
|---|---|---|
| **Object** | `GwzTransportWindowsParityDesign.md` | `c24c7033f295c671` |
| Round 1's findings and the operator's decisions D1 to D4, with the dispositions | `GwzTransportWindowsParityDesign-RemPlan-2.4.md` | `03760d35dbf787fd` |
| Round 2's findings and the operator's decision D5, with the dispositions (new; every round 2 finding has one row) | `GwzTransportWindowsParityDesign-RemPlan-2.4-2.md` | `84de2944276b1959` |
| Controlling record | `GwzTransportTR18-OperatorDecisions.md` ("Operator discussion record" controls over its per-item narrative; it now also records the TD12 in-session run approval and D1 to D4) | `4254e3a00a7acf94` |
| Delta (what MAIN and the evidence changed) | `GwzTransportWindowsParityDesign-Delta.md` | `3a1596fdfa9a0c54` |
| Amendment 2, revision 8 DRAFT (skim review not run; three wording changes of 2026-10-11) | `GwzTransportReleasePlanAmendment-2.md` | `8784fb33f914d1dc` |
| Its skim package | `GwzTransportReleasePlanAmendment-2-SkimPackage-8.md` | `4fcb9a84df2a8c9c` |
| Plan, revision 4, step 2.4 and section 8 (OQ1 to OQ16), with the round 1 changes to steps 3.1, 4.4, 4.6 to 4.8 and 6.2 | `GwzTransportWindowsParityPlan.md` | `da5d7858787e4f27` |
| Baseline and dispositions | `GwzTransportWindowsBaseline.md`, `GwzTransportWindowsProofDispositions-DRAFT.md` | `d3eb823109122df2`, `feb7e9916e9bf3d1` |
| **Surface object** | `GwzTransportWindowsUserGuide-DRAFT.md` | `f5e993852aa084f9` |
| Accepted contracts the design must agree with | `GwzTransportCredentialHelperTimingAmendment.md`, `GwzTransportCredentialHelperConfigurationViewAmendment.md`, `GwzTransportSshHelperClockAmendment.md`; `../../dev-docs/GwzSspiDesign.md` (now carrying the D3 annotation of 2026-10-11, text otherwise unchanged), `GwzSspiAcceptance.md`; `../../dev-docs/GwzWindowsHttpsIntegrationImplementation-Verdict-3.md` | n/a (accepted) |

**Settled tuple** (the round 2 reviewers pinned gwz-core `17ec75f1` and found the other members unchanged at the start and the end of their reviews; their root had advanced by a gwz merge that touched only `gwz.conf/`): workspace root at the commit that records this package (its base is `cea80726`); gwz-core at the commit that carries this package (a document cannot name its own commit; the review prompt names both; its base is `ad9619ec`, which filed the round 1 and round 2 reports on top of the reviewed `17ec75f1`, documents only) with the round 2 remediation committed (this package, the design, the guide, the decision list, the plan and the new `GwzTransportWindowsParityDesign-RemPlan-2.4-2.md`); gwz-core-evidence `928f6d4` (includes run RX and the two README corrections); gwz-transport `6146bd58`; gwz-cli `21d62310`; gwz-py `5950ba38`; gwz-sspi `73de8c8e`; git2-rs `d13951f7`. The workspace root's `dev-docs/GwzSspiDesign.md` carries the D3 annotation (committed in `b3e6de03`, text otherwise unchanged). Round 1 pinned gwz-core `e5e8e804` and evidence `46b770f`; the design's own "evidence tuple" paragraph states the same settled tuple and says where each run landed.

**Evidence** (private, `gwz-core-evidence/campaigns/transport-qualification/runs/`, access required; the public record is the design, delta, decision list and baseline). Short names are the design's: RS `2026-10-10-tr18-windows-ssh-rows` (90 rows), RA `...-windows-auth-rows` (35, HTTP), RR `...-redirect-parity`, RM `...-b17-macos-linux`, **RH** `...-td9-user-root-trust` (52 HTTPS rows plus a 10-row redirect supplement), **RP** `...-td10-machine-proxy` (82 rows), **RN** `...-td12-agent-service` (first run partial; the run of record is `raw/native-run2-interactive`). Released Windows 1.0.17 is `gwz.exe` SHA-256 `cbab5e9b...`. RH and RP landed in gwz-core-evidence `12b46d9` and RN's run of record in `e07f516` (both are in the lane's history); **RX** `2026-10-11-tr18-r13-https-proxy` (40 rows plus a guard drill: the machine proxy against a public https origin, decision D1) is committed in gwz-core-evidence `928f6d4`.

**Who receives what.**

- Consistency and Safety: the design plus the controlling graph in the table, the canonical review-loop prompts and report paths, **(a)** the design's section 14 list of removed and revised claims, **(b)** the design's section 11 reading of the GO rule (a row that cannot run discharges it by having its claim removed and listed; if the reviewers read the rule as requiring an executed result for every row, the fallback is an amendment of the rule by the lane owner or the operator, never a provisional clause), and **(c)** this package's section 4 questions.
- Surface (retired as a reviewer by the operator's two-reviewer rule of 2026-10-11; its findings are fixed and Consistency verifies them, so Consistency's mandate gains the guide and Annex S): the user guide, existing help and docs, and **Annex S** below, and nothing else. The design's section 11 says Surface never receives the design or the implementation; the migration notes (S7.2 (1.1.0)) do not exist yet, so Annex S states the register of section 12 in a user's words with no design vocabulary. The lane owner should give Surface the guide and Annex S only, and cut everything else from this package.
- Reports are filed beside the design as `GwzTransportWindowsParity-ReviewConsistency.md`, `-ReviewSafety.md`, `-ReviewSurface.md`, then the verdict, verbatim, with no self-GO and no dirty review. One merged remediation patch; the same reviewers re-check their original counterexamples.

## 2. What changed since the 2026-10-03 DRAFT

The 2026-10-03 text was NO-GO and carried provisional clauses. The 2026-10-11 revision:

1. **Carries MAIN's accepted amendments**: helper timing, configuration view (no Windows mechanism is designed; step 4.1's WH2 contract owns it) and the SSH clock; the connection-scoped `SetupClock` and the per-host `Supervisor` as runtime-owned values; WH1's accepted shape (the qualification cfg, `Owner::send_if`, the machine proxy admitted only when verified DIRECT). The SSPI-only supersession list is folded into sections 2, 8 and 11 and retired: SSPI runs in an owned fresh process; no clause rests on cancelling a thread.
2. **Records the plan's decisions of 2026-10-08** (OQ1 to OQ16) and the proof dispositions the evidence supports (HOME, Pageant timeout ownership, window identity, weak certificate hashes).
3. **Changes the clauses the 2026-10-10 evidence contradicts**, by the operator's decisions:
   - which identity answers: the logon session answers Negotiate and NTLM, a helper answers Basic only, Digest is refused or ignored, a rejected logon identity ends the operation (section 7; the earlier helper-first precedence was contradicted by RA and RH);
   - discovery redirects are followed (section 7; see 5 below), and a challenged POST is refused (section 9);
   - native paths, one captured home, and a relative `HOME` that exists is used while an explicit empty `HOME` refuses (section 3); the RSA-only host-key and file-key limits with plain diagnostics (section 3);
   - no cross-user Pageant refusal (section 4), and the baseline pipe policy with no server-identity check (section 4);
   - the 407 rule is confirmed through a named machine proxy (section 6).
4. **Adds** the TD13 migration-difference register (section 12), a separate limitations list (section 13), and the removed-claims list with a row index (section 14).
5. **Fills the rows that waited on host transactions** (this fill, 2026-10-11), citing run and row, and rewrites four clauses the rows contradicted:
   - *TD3.* Windows 1.0.17 follows an **absolute-URL `Location` over HTTPS** (301, 302, 307, 308; another origin, another host name, a new path) and no relative `Location`; over plain HTTP it follows neither. The deliberate difference is only for a relative `Location` and for HTTP.
   - *TD10.* Bypass entries separate on `;`, space or `,` (not only `;`); the implicit bypass is `localhost` in any case, all of 127/8, `[::1]` and the machine's own address, but not `localhost.`; `<-loopback>` is not honoured; trailing dots compare literally; `*` spans labels and matches numeric text.
   - *B16 and EPA.* Channel binding is enforced and satisfied over Negotiate and NTLM, and the negative controls are refused, with the loopback NTLM caveat (88-byte Type 3, AV pair unreadable).
   - *Digest-only with no helper* already fails at once on 1.0.17; the register's Digest row is refined (15 replays only with a helper configured).
   - *B08 native service* is qualified for the cells run; the earlier same-SID server-token rule is shown unworkable by measurement (the server is LocalSystem).

## 2b. What changed after round 1 (2026-10-11), for round 2

**Round 1.** Consistency, Safety and Surface read the first tuple blind; all three were NO-GO. Two findings converged: the HTTPS origin through a named machine proxy (Consistency P2-1, Safety S3, Surface U1; reading R13) and the plain-HTTP boundary (Safety S1, rated P0, Consistency P2-2, Surface U9). The operator then decided D1 to D4. The findings and decisions are in `GwzTransportWindowsParityDesign-RemPlan-2.4.md` (IDs as each reviewer gave them; Surface's S1 to S20 are U1 to U20 there).

| Decision | What it did to the documents |
|---|---|
| D1 | Measured on released 1.0.17: run RX, 40 rows and a guard drill against a public https origin (every restore byte-identical). The design admits a bare or `https=` entry for an https URL by WinHTTP's rule and **refuses a mapping with no entry for https** (register item 9). The Annex S sentence "not yet tested end to end" is gone. |
| D2 | The transport serves `https://` only and refuses an HTTPS to HTTP redirect; an `http://` remote goes by the native route with a warning (register item 10; L18). Design section 7, the guide, amendment 2's redirect rows (relative-`Location` form over HTTPS only). |
| D3 | gwz-sspi's `Identity::Explicit` and Digest code is kept for after 1.1.0 (the SSPI design is annotated in the workspace root, not amended). Plan steps 4.4, 4.8 and 6.2 no longer build or test helper-identity SSPI or Digest. This answers reading R9 differently from the first package, which proposed removal. |
| D4 | The `GIT_SSL_CAINFO` and `SSL_CERT_FILE` roots stay beside Schannel's. Design section 7 states the trust sources; register item 11; the guide warns. |

**Documents changed in the lane (all documents; no code):** the design (sections 3 to 8, 11 to 14, with the owed code changes listed in section 14 item 13), the guide (rewritten against U1 to U20), the decision list (TD12 in-session approval; D1 to D4), the plan (steps 3.1, 4.4, 4.6, 4.7, 4.8 and 6.2, OQ4 note, section 9, 12.5), amendment 2 revision 8 and its skim package (three wording changes), the workspace root's `GwzSspiDesign.md` (annotation), and in gwz-core-evidence the new run RX plus two README corrections. The design's finding-by-finding dispositions are in the remediation plan; the unclosed items are the limitations L17 to L20 and the items marked as owed code.

**Readings after round 1** (the originals stand in section 4; this is where each moved):

| Reading | Round 1 disposition |
|---|---|
| R3 | Defined: a 401 with no continuation token for the selected scheme, or a 3xx, is rejection; the eight rounds are per operation; no new exchange except TR2.1's retry on network loss before effect (design section 7). Repeated NTLM attempts across operations is limitation L19. |
| R5 | A passphrase-protected key gets the agent remedy, or `ssh-keygen -p -m PEM` with an empty passphrase on a copy (design section 3, guide). |
| R8 | The redirect bound is the accepted five-hop sequence (HTTPS design section 5; TR1.6 C2), not "one"; origin means scheme, host and port; the non-forwarding rule is still unmeasured against 1.0.17, with two product tests named and a native redirect-after-exchange test owed to step 4.9. |
| R9 | Not removed: D3. The code stays for after 1.1.0 and has no 1.1.0 caller. |
| R10 | Round 1 classified the wait as the clock amendment's LocalInteraction phase; round 2 found that this needed an extension of an accepted contract. **Superseded by D5 (b), section 2c:** the wait stays in the Network phase, the stall and aggregate clocks keep running, and the cost is stated (L20, register item 14). |
| R12 (b) | Register item 12 records the refusal of more than one bare entry. (d), accept-and-ignore of `<-loopback>`, is unchanged. |
| R13 | Measured (RX) and decided (D1). The design no longer lists the https case as unmeasured except for the rows RX did not run (L8). |
| R14 | The transport's own rule is stated: an IP-literal host equal to an interface address enumerated once at capture goes direct; names are never resolved; anything else goes through the proxy. Still one observed address on 1.0.17. |
| R15 | Unchanged, with the header sizes corrected (88 to 103 bytes for NTLM, 119 to 121 for Negotiate). |
| R16 | Plain HTTP is moot for the transport under D2 (native route); register item 4 is now only the relative `Location` over HTTPS. |
| R17 | P01 is now labelled P (partial); the others stand. |

## 2c. What changed after round 2 (2026-10-11), for the verification

**Round 2.** Consistency, Safety and Surface read the round 1 tuple again and were NO-GO again (reports beside the design: `GwzTransportWindowsParityDesign-2.4-ReviewConsistency-2.md`, `...-ReviewSafety-2.md`, `...-ReviewSurface-2.md`). One finding was architectural, found by two reviewers: the Pageant confirmation wait was classified into the SSH clock's LocalInteraction phase, which the accepted clock amendment forbids for any route but the ambient password helper (Consistency N1, Safety R2-1). The operator decided it. Every other finding is wording, trace or evidence.

| Decision | What it did to the documents |
|---|---|
| D5 (b) | The Pageant confirmation wait stays in the SSH clock's Network phase in 1.1.0, and nothing in the accepted clock amendment changes. The cost is stated plainly in the design (section 5, L20, register item 14, section 14 items 13 and 14), the guide and the plan: a Pageant set to confirm every use, whose prompt outlasts the stall, fails that attempt (raise `--ssh-timeout`, or `--transport native`, which waits as 1.0.17 did), and a passphrase entered on first use succeeds on retry. The same holds for ssh-agent confirmation (`ssh-add -c`) on macOS and Linux today. Step 3.6 owes only the timeout message and a test that the stall clock is running. |

**The other fixes** are listed finding by finding, with file and line, in `GwzTransportWindowsParityDesign-RemPlan-2.4-2.md`. The ones the reviewers should look at first: the one rule for machine-proxy forms with no executed row (design section 6, last bullet, L8, the guide's proxy section, Annex S.3 and S.5); the guide's quoted refusal texts and next actions, with their owning steps in design section 14, item 13 (texts T1 to T7); the guide's key-file section, which now says that GWZ tries no key file unless one is named and shows `--identity` and `gwz auth identity` with the undo; the setting-order exception for `http://` remotes; the stale `GIT_SSL_CAINFO` and `SSL_CERT_FILE` refusals; and the plan's steps 3.3, 3.6, 4.4, 4.6, 4.7 and 4.9.

**Documents changed (all documents; no code, no evidence repository, no amendment):** the design, the guide, the plan, the decision list (D5), this package, and the new `GwzTransportWindowsParityDesign-RemPlan-2.4-2.md`. Amendment 2 and its skim package say nothing about the Pageant clock and are unchanged; section 1 re-verifies their hashes.

## 3. The operator decisions (2026-10-10), as the design carries them

| Decision | Answer | Where in the design |
|---|---|---|
| TD1 | A: logon session answers Negotiate/NTLM; helper only for Basic; stop after a rejected logon identity (no replay, no downgrade) | 7 |
| TD2 | A: Digest-only refused promptly; ignored beside a supported scheme | 7 |
| TD3 | A: follow discovery redirects; no credentials across origins; HTTPS confirmation owed (now executed, and it narrows the difference) | 7, 12 item 4 |
| TD4 | A: refuse a challenged POST; never replay its body | 9 |
| TD5 | A+B: keep the RSA-only backend limits, add plain diagnostics; agent keys of three types are separate | 3, 4 |
| TD6 | A with the native-path clarification: one captured home; Windows Unicode paths fixed; native paths on every platform; Unix audit outstanding | 3, 13 (L14) |
| TD7 | B: dispose of B17 by inference; the Mac trusted-HTTPS row stays unexecuted | 11, 13 (L5) |
| TD8 | B: hardware security key and Pageant-held certificates not qualified | 4, 13 (L2, L3) |
| TD9 | A: attended one-certificate transaction (done) | 7, 8, 9 |
| TD10 | A: serialized machine-proxy transactions (done) | 6 |
| TD11 | B: no cross-user refusal promise; Safety judges it | 4, 13 (L9) |
| TD12 | OQ4(a) baseline pipe policy plus the bounded service test (done) | 4 |
| TD13 | the migration-difference register | 12 |
| TD14 | A: amendment 2 revision 8 | amendment 2, skim package 8 |

## 4. The drafter's readings where the decisions are silent: questions for the reviewers

R1 to R11 are the eleven items of the tr18docs drafter's report (the drafter flagged ten readings and one housekeeping note; R11 is the housekeeping item and is addressed to the lane owner). R12 to R18 are readings introduced by the 2026-10-11 fill. Each question names the axis that judges it; any reviewer may comment on any. "Reading" is what the text now says; the question is whether to keep, change or remove it.

**R1. Scope of the amendment edits (Consistency).** The decision list cites amendment 2 section 3.5 lines 166, 178 and 226. The same claim stood in three more places, sections 3.11 (line 286), 3.14 (line 367) and 3.19 (line 471), and revision 8 changed them. *Question:* is the design consistent with all six sites, and is the extension to the extra three within TD14's "reconcile the contradicted text, no new mechanism"? (Items 4 to 6 of the skim package can be reverted without touching the rest.)

**R2. TD13 item 3 omitted a difference (Consistency).** The decision text says only "no replay loop" for a rejected logon identity. On 1.0.17, NTLM-only and Negotiate+Basic also consult the helper on each replay (15 and 14 calls; the helper exchange never completes), while TD1 says do not downgrade to helper credentials. The register (section 12, item 3) therefore says "no replay loop and no helper consulted". *Question:* is that a faithful statement of TD1, or an extension?

**R3. What "one rejection" means in a multi-round exchange (Safety, Consistency).** TD1 says stop after rejection of the logon identity. For NTLM, whose exchange has several rounds, the design defines rejection as the server's 401 after the exchange's final token, not an intermediate 401. *Question:* is this the right boundary, and could a hostile server abuse the intermediate rounds (the exchange is bounded by eight rounds and the setup clocks)?

**R4. Channel binding unavailable (Safety).** If the final TLS handshake's binding cannot be obtained, the design refuses the SSPI attempt and ends the operation with an error rather than falling to Basic or omitting the binding. TD1 forbids a downgrade only after logon rejection; this extension is the drafter's. *Question:* keep (fail closed), or allow a Basic fall-through for a server that offers it?

**R5. TD5's remedy for non-RSA keys (Surface, Consistency).** `ssh-keygen -p -m PEM` converts only a new-format RSA key. For ECDSA, ed25519 or passphrase-protected keys the diagnostic names an agent instead (agents sign all three types). TD5's wording ("name the cause and the fix") does not say this. *Question:* is naming an agent the right fix, and is the message wording acceptable cold?

**R6. TD10 listed "a 407 through a machine proxy" as blocked while the delta marks B10 executed (Consistency). Resolved by the fill.** RP/p407-* now execute it through a named machine proxy (one request, no `Proxy-Authorization`, helper asked 0 times). *Question:* does the design (section 6, 407) now say the same as the delta, the decision list and the register?

**R7. The delta counted B18 "executed in full" though its HTTPS variant was blocked (Consistency). Resolved by the fill.** RH/b18s-* execute it over HTTPS with identical outcomes. *Question:* are the section 11 statuses and the delta's counts now consistent, or must the delta's counts be restated?

**R8. TD3 beside OD16, and whether "one" redirect is a cap (Safety, Consistency).** "Credentials never follow a redirect to another origin" (TR1.6 OQ4) sits beside OD16's "any host that challenges receives the logon session's response". The design reads this as: a challenge issued by the redirect target itself is answered by the logon session (that is a first request to the target), but nothing from the first origin is forwarded. TD3 also does not say whether amendment 2's "one validated discovery redirect" is a cap on the number followed. The cross-origin non-forwarding rule is the transport's own and **no row measures it** (in RH's other-origin rows the first origin never challenged). *Questions:* is the reading right; is the redirect count capped (and at what value); is an unmeasured safety rule acceptable at GO if a product test is named for it?

**R9. Explicit-identity SSPI has no caller (Consistency).** Under TD1 the helper never feeds SSPI, so `SEC_WINNT_AUTH_IDENTITY_W` and its zeroizing identity owners are dead. The design removes them (section 14 item 6) and plan step 4.8's explicit-identity residual has no caller. *Question:* confirm removal, and that nothing else in the plan or amendment still depends on it (the remove-dead-code rule applies).

**R10. Pageant confirmation and the `SetupClock` (Safety, Consistency).** Section 5 bounds a Pageant request by the remaining allowance of the connection's `SetupClock` and says no independent 120-second allowance exists, but does not say how the user's confirmation wait is classified among the clock's phases (network, local admission, local interaction, terminal). *Question:* does the omission leave a hang or a premature expiry, and which phase should it be? (Disposed by operator decision D5 (b), section 2c: Network phase, with the premature expiry stated as the cost.)

**R11. Housekeeping (lane owner, not a reviewer question).** `gwz-core/dev-docs/GwzRemoteTransportBugReport.md` is an unrelated untracked file in the lane; exclude it from the commit.

**R12. What the transport admits of the machine-proxy grammar (Safety, Consistency).** The design admits exactly the forms RP executed (section 6) and refuses the rest before any Open. Four readings inside that: (a) the `http://` prefix on a proxy server is admitted (g03, g09), other proxy URL schemes are refused; (b) a value with more than one bare entry is refused as ambiguous, although 1.0.17 uses the first (it only meets this through a registry value `netsh` cannot write); (c) bypass entries separate on `;`, space or `,` (the earlier draft said semicolon only; 1.0.17 accepts all three); (d) the token `<-loopback>` is accepted and has **no effect** (the implicit loopback bypass stays), as on 1.0.17, rather than being refused. *Question for (d):* silently keeping a bypass the user tried to remove sends loopback traffic direct; is accept-and-ignore acceptable, or should the transport refuse the token naming the machine proxy (a difference from 1.0.17)?

**R13. HTTP-origin rows applied to https origins (Safety, Consistency).** Every RP row used an http origin through a forwarding fixture proxy, which cannot show a CONNECT tunnel. The design takes the proxy-entry selection, the bypass matching and the `https://host` entry for an https URL to follow the same WinHTTP rules, and lists the `https=` entry's selection and the CONNECT leg as unmeasured (L8; section 14 item 11), first executed by step 4.6's product test. *Question:* is "by WinHTTP's rule, unmeasured, product-tested at 4.6" enough for GO, or should the https-origin machine proxy be refused until measured (the OQ11 fallback applied to the whole class)?

**R14. "This machine's own address" (Safety).** The implicit bypass includes the machine's own non-loopback address (RP/b35, b36). One address was observed (redacted), and the rows do not say which addresses count (every interface address at capture? IPv6? the computer name?). The design leaves the rule to step 4.6 with a test on more than one address. *Question:* is that acceptable, or must the design name the rule now (for example, interface addresses enumerated once at capture, and the unresolved cases refused)?

**R15. Does a loopback EPA result discharge B16 and P06 (Safety, Consistency).** B16 shows 1.0.17 satisfies an enforcing server and is refused when the server expects other bytes, and the controls are refused. But every row ran on loopback, so the NTLM Type 3 is the 88-byte local-call form, the channel-bindings AV pair is unreadable, and a remote NTLMv2 exchange was not tested (L7). The design marks B16 and P06 executed with that caveat and treats the transport's own EPA behaviour as 4.8 and 4.9 product tests. *Question:* is "E (loopback)" the right status, or is B16 partial and L7 a GO condition?

**R16. The TD3 difference is narrower than the decision assumed, and HTTP may be moot (Surface, Consistency).** TD3 and the register's item 4 were written when "Windows follows none" was believed. The difference is now only a relative `Location` and plain HTTP. If the product does not admit plain `http://` remotes, the HTTP half is moot for users. *Questions:* is the register item 4 wording right; do the migration notes need the HTTP case; and is amendment 2's redirect row ("one name redirects discovery to the other", an absolute `Location` to another host name) still a "transport-only" row, given 1.0.17 follows it over HTTPS? (It is not; amendment 2 now says so, section 8, item 1.)

**R17. Status relabelling in section 11 (Consistency).** The fill moves B09 to "E (http origin)", B15 to "E (HTTP and HTTPS; loopback)", B16 to "E (HTTPS; loopback)", B08 to "E (cells run)", P03 to "E" and P06 to "E (EPA controls); D (MD5, adapter lifetime)". B15 includes the untested cross-origin non-forwarding; B08 is one Windows build, one run pair, keys added through an interactive-logon task token. *Question:* are these labels honest under the GO rule, or does any belong at "P"?

**R18. Digest-only with no helper (Consistency).** The register (section 12, item 2) said Digest-only is "15 replays, then error" on 1.0.17. With **no helper configured** 1.0.17 already fails at once (`GWZ could not acquire credentials`: RA/b14-digest-only-no-helper, RH/b14s-digest-only-no-helper); the 15 replays occur only with a helper configured. The register now says so. *Question:* confirm the refinement and that the user-facing statement (Annex S) is not overstated.

## 5. Reviewer checklists

### 5.1 Consistency

1. **Evidence to text.** Every clause cites an executed row or an accepted disposition. Spot-check at least 20 cited row names against the run archives (README plus `raw/results-table.md`), including three that carry a number or a quoted message (for example B16's four authenticating rows, the 129-byte POST, `0x80090346`). No clause is provisional; a search for the upper-case PENDING, hyphen, ROW string finds nothing.
2. **Contradictions kept honest.** The clauses the fill rewrote (section 2, item 5) say what the rows say and no more. The README of RH says "5 rows" for the authenticating EPA rows and the row list has four plus two no-binding baselines; the first RN run's table labels a refused `ssh-add` as `added`. Neither is hidden in the design.
3. **Decisions to design.** Each of TD1 to TD14 is carried as recorded (table in section 3), with its qualification (TD6's path clarification, TD7's unexecuted Mac row, TD11's explicit Safety judgement, TD12's combined choice). No clause reverses a decision or adds a mechanism the decisions did not approve (R1, R4, R5, R12 to R14).
4. **Design to amendment 2 revision 8.** Sections 3.5, 3.11, 3.14 and 3.19 agree with design sections 7, 8 and 9; no mechanism is stated twice with different bounds. The redirect row (amendment line 179 and 227) is checked against RH (R16, section 8 item 1).
5. **Design to accepted contracts.** Section 10's helper paragraphs agree with the helper timing and configuration-view amendments and with step 4.1's WH2 contract; section 5's bound agrees with the SSH clock amendment (R10); sections 2 and 8 agree with the SSPI design and acceptance.
6. **Internal.** Section 11's table, section 12's register, section 13's limitations and section 14's lists agree row by row (each removed claim has a row or a decision; each limitation points to a section; each register difference points to a decision). Cross-references resolve. The GO rule's reading in section 11 matches the plan's.
7. **Plan.** Step 2.4's scope, and the steps the plan says consume TR1.8 (3.1, 3.2b, 3.3, 3.5, 3.6, 4.4, 4.6, 4.7, 4.8), are served by clauses that are specific enough to implement; step 4.8 builds no explicit-identity SSPI and no Digest, and steps 4.4 and 6.2 test neither (D3); gwz-sspi keeps that code for after 1.1.0 with no 1.1.0 caller (R9).
8. **Counts.** Row counts, run sizes and the delta's tallies match the archives (R6, R7, R17).

### 5.2 Safety

Judge explicitly, each with a verdict and the counterexample you tried:

1. **No server-identity check on the agent pipe (TD12, OQ4(a)).** The native service's pipe server is `ssh-agent.exe` as LocalSystem in logon session `0x3e7`; the caller is another session, and the pipe grants Authenticated Users read and write. Is admitting any local pipe server, with only name admission (no UNC, no traversal, no MinGW Unix form) and an identification-level impersonation limit, acceptable? What can a local attacker who creates a pipe named by `SSH_AUTH_SOCK` or the default name before the service does?
2. **No cross-user or cross-logon-session refusal of a found Pageant (TD11 B).** The retained checks are the pinned window-to-process identity, creation identity, a unique per-request mapping with a caller+SYSTEM ACL, and one send with no resend. Is that enough for a Pageant owned by another user in the same desktop? (P01's negative did not run.)
3. **Default credentials to any host (OD16) and NTLM exposure.** The logon session answers any host that challenges Negotiate or NTLM, including an Internet host. Does the design bound the hazard it accepts (certificate trust first, no CONNECT credential, rejected identity terminal, no replay, eight rounds), and are the cross-origin redirect rules sufficient given they are unmeasured (R8)?
4. **Channel binding.** Capture from the actual final TLS handshake, per physical connection, for proxy and origin separately; native `None` or error refuses the SSPI attempt (R4); weak-hash upgrade; EPA satisfied and refused in the executed rows, loopback caveat (R15).
5. **Redirect and POST.** Credentials never cross origins; a challenged POST is refused and the body is never resent (1.0.17 resends an empty body); no credentialed retry of a partial or complete POST.
6. **Machine proxy.** Grammar admission and refusal (R12), the implicit bypass including the machine's own address (R14), the https-origin selection and CONNECT as RX measured them for one public origin, and the `http=`-only refusal (R13, D1), an unresolvable proxy sends nothing direct, a parser failure never sends direct, every 407 refused with no credential offered, environment and git-configuration proxies ignored (and beaten by a named machine proxy).
7. **Paths and trust files.** One captured home; a relative `HOME` resolved against the captured working directory so a later change cannot redirect `known_hosts`; an explicit empty `HOME` refuses; host-key checking mandatory before any authentication and no trust failure invokes another agent or the native transport; native paths on every platform.
8. **Secrets.** Mapping contents, agent comments, key bytes, signatures, tokens and OS error strings never reach logs or failure text; SSPI runs in an owned process whose forced exit guarantees reaping, not secret erasure (is that stated honestly?); wipe-before-unmap rules for the Pageant mapping.
9. **Process and handle ownership.** Job Object assignment before resume for git; Pageant receiver boundary (GWZ retires only its own ownership, never terminates an operator's Pageant); `CancelIoEx` and buffer retention until completion; no thread cancellation relied on.
10. **The removed-claims list and the GO-rule reading (section 14, section 11).** Is anything removed that Safety needs kept, and is any claim kept that only an unexecuted row supports?
11. **Limitations.** Is any limitation in section 13 (Kerberos, hardware keys, Pageant-held and native-agent certificates, remote NTLMv2 EPA, remote hosts, non-RSA keys) a hazard rather than a gap?

### 5.3 Surface (reads the user guide and Annex S cold)

1. **Can a Windows user tell what GWZ will do?** For each topic in Annex S: which identity answers a login challenge; what happens with a GitHub token; what GWZ never answers (Digest) and why; what happens when a server rejects the Windows logon identity; what happens when a server challenges the upload step; how the SSH agent and home are chosen; what a proxy setting does and does not do.
2. **Messages and remedies.** Is every refusal actionable: does it say what failed, which setting or switch is involved (`--transport native`, `GWZ_TRANSPORT`, `SSH_AUTH_SOCK`, the machine proxy) and what to do next? Judge the proposed texts for the RSA-only host-key and key-file cases (R5) and the proxy refusals.
3. **Honesty about what is not promised.** The guide and notes must not imply support for Kerberos, a hardware security key, certificates held by an agent, proxy authentication, non-RSA SSH host keys or file keys, or the native agent beyond the cells in Annex S. Does the Windows logon-credential warning read as a warning?
4. **Migration notes.** Does each difference in Annex S say what the user sees on gwz 1.0.17 and what changes, without design vocabulary? Is anything in it a surprise a user would reasonably resent (for example Digest beside Basic now working, a rejected logon identity no longer retried, a redirect now followed)?
5. **Cold-read defects.** Jargon (SSPI, WinHTTP, CONNECT, EPA, SPN) used without a gloss; instructions that assume knowledge (what `netsh winhttp show proxy` is; starting the OpenSSH agent service, which may be disabled); placeholders still in the text ("The final guide will name..."); statements that contradict Annex S or each other; commands that would not work as written in PowerShell or Git Bash.
6. **Known gaps the drafter did not close** are in section 8, item 4. Surface should report any it finds independently and say whether it blocks.

## 6. How to read the GO rule (input (b))

The design's section 11 reads the GO rule as follows. Every row B01 to B18 and P01 to P08 needs an executed result or a recorded disposition, and every provisional clause is replaced by one physically proved design. **A row that cannot run discharges the rule by having its claim removed and listed in section 14.** After this fill every row approved on 2026-10-10 has executed; the rows with no execution are P01 (claim removed, TD11 B), P02 (stays UNEXECUTED, claim revised), P05 and P07 (dispositioned), the Mac trusted-HTTPS B17 (TD7 B), a real hardware key, Pageant-held certificates, native-agent certificates (L3), Kerberos (L1), a remote NTLMv2 Extended Protection exchange (L7), the target name of the logon exchange (L15), the interface addresses that count as the machine's own beyond one (L8), a redirect through a proxy and its interplay with the bypass list (L8), a multi-hop redirect chain and a 303 on 1.0.17, and the proxy variables not set in RX or RP (L8); the https-origin machine-proxy selection and CONNECT leg is no longer in this list, because RX executed it for one public origin. The reviewers say whether they accept this reading; if not, the fallback is an amendment of the rule by the lane owner or the operator.

## 7. Procedure

Use the review-loop skill's canonical prompts, with the paths in section 1. Do not start any review from this package; the lane owner dispatches. Findings are filed verbatim. The two architectural remediation rounds are a cap, not a target.

## 8. Open items for the lane owner before dispatch

These are outside this task's edits (documents it was not asked to change) or need a decision.

1. **Stale redirect sentences in amendment 2 revision 8: done (2026-10-11).** Amendment lines 179 and 227, its status bullet and a new changelog line, and the skim package (update note, table row, items 2 and 3, checklist question 5) now say that 1.0.17 follows an absolute `Location` over HTTPS and that the redirect row is transport-only only in its relative-`Location` and plain-HTTP forms. The hash of revision 8 changed (`a39f4764...` to the value in section 1), the skim package pins the new one, and this package pins both. The decision list's TD3 narrative (lines 103 to 114) still says "follows none"; its discussion record controls, so it needs no edit, but the delta's section 2 row 7 and section 4 B15 row are superseded for HTTPS.
2. **TD12's second run and the `HKCU` residue.** The service created empty `HKCU\Software\OpenSSH\Agent\...` keys on its first key add (zero subkeys; owned by SYSTEM). They were not deleted. The exact command is `reg delete HKCU\Software\OpenSSH /f` from an elevated session; the lane owner decides whether to run it.
3. **Companion documents the delta (section 6) says change at the refresh.** The baseline, the checkpoint, the plan's Appendix A and the user guide were not part of this task. In particular the baseline still lists B06, B08, B09, B11 to B18, P01 and P03 to P07 at their 2026-10-04 statuses.
4. **User guide: rewritten for round 2 (2026-10-11).** The guide now answers Surface's U1 to U20: it opens the proxy section with what is supported, states the `http=`-only refusal, the https-origin measurement, the own-address rule, trust sources, the redirect bound, the setup limits with their options, the agent steps and commands, the pinned-key remedy, the `known_hosts` commands, the allocation-budget message, a changes table and the unchanged-on-purpose disclosures. The OPEN ITEM placeholders for R10, R13 and R14 are gone. Texts in quotation marks are proposals for messages that are owed code (design section 14, item 13). Round 2 (section 2c) added the D5 Pageant text, the quoted refusals and their next actions, the key-file steps with their undo, the `http://` exception to the setting order, the stale CA-variable refusals and the unmeasured-proxy list. Surface is retired; Consistency verifies the guide and Annex S.
5. **Evidence archive: fixed (2026-10-11).** RH's README now says four authenticating EPA rows (it said five), and RN's README no longer says each identity is tried (the authorized ed25519 key is the first the agent lists). RN's first-run results table and `summary.json` still label the refused `ssh-add` row `added`, left as raw output; the README and the design say the row was refused.
6. **The evidence commit holds run RX** (`2026-10-11-tr18-r13-https-proxy`) and the two README corrections of item 5, in gwz-core-evidence `928f6d4`; RN's run of record is in `e07f516`. Nothing in the evidence repository changes in round 2.
7. **The tuple.** Commit the remediation (the proposed messages are in the lane owner's report), then recompute the hashes in section 1 and record the settled tuple.
8. **Exclude** `GwzRemoteTransportBugReport.md` (R11): it is untracked in this lane's gwz-core; leave it out of the commit.
9. **Code owed, not done.** The messages and behaviours the design and guide promise are listed with their owner steps in design section 14, item 13. None is implemented, and no reviewer should read the guide's quoted texts (T1 to T7 of that item) as shipped.

---

# Annex S: what a Windows user sees (for the Surface reviewer)

This annex describes the behaviour the proposed Windows release (1.1.0) is meant to have, compared with gwz 1.0.17 on Windows. It is a statement of intended behaviour, not a statement that anything is implemented or released. Read it with the user guide. It uses no internal design vocabulary on purpose. It was revised on 2026-10-11 after each review round, to match the guide.

## S.1 How GWZ chooses who logs in

- GWZ first asks the server without any credentials. If the server offers **Negotiate or NTLM** (Windows sign-in), alone or beside Basic or Digest, GWZ signs in with **your Windows account** and does not ask your configured git credential helper.
- If the only scheme the server offers that GWZ can use is **Basic**, GWZ asks your configured git helper once and sends what it returns. A GitHub token held by a helper is sent this way. If no helper is configured, the message says so and how to configure one.
- GWZ **never answers Digest**. A server that offers only Digest is refused at once with a message that says Digest is not supported by either transport and suggests SSH or a change on the server. If the server offers Digest and another scheme, Digest is ignored and the other scheme is used.
- If the server rejects your Windows account, the operation stops with an error that names the host and the scheme (for example "<host> rejected your Windows account (NTLM). GWZ tries nothing else. Ask the server's administrator for access for this account, or use an SSH remote."). GWZ does not try it again and does not turn to a helper credential. No setting changes this.
- A helper that times out or is cancelled ends the operation.
- If the server challenges the upload step of a fetch (after the first request succeeded), GWZ refuses with a message and does not send the data again.
- **Warning:** your Windows sign-in can be offered to any web host that asks for it, including a host on the Internet. What the host receives can be cracked offline and relayed to services that do not require extra protections. This is what gwz 1.0.17 already does, no setting turns it off, and the native transport does the same.

## S.2 Redirects, plain `http://` and certificates

- GWZ follows a redirect when it first contacts a repository, to another address or to another path: at most five, only from `https://` to `https://` (a redirect to `http://` is refused), and none after a Windows sign-in exchange has started. It does not forward a sign-in from the first address to a different address (a different scheme, host or port); a different address that asks for credentials is answered as if you had gone there directly. With `--verbose` GWZ prints each redirect it follows.
- On gwz 1.0.17 on Windows, a redirect that gives a full address is followed over HTTPS and one that gives only a path is not.
- A plain `http://` remote is carried by the native transport under every setting (also with `--transport gwz`), with a warning printed before the first request, on every operation, that nothing turns off: a Basic password travels unencrypted and a Windows sign-in is answered without channel binding. For that remote the native transport's behavior applies: no pooling, no setup retry, no 30-second setup budget, and the `--ssh-timeout` value as libgit2's timeout (9 seconds by default). A redirect from `https://` to `http://` is refused, with a message that says what to do next.
- GWZ trusts the Windows certificate store **and** the certificates in the files named by `GIT_SSL_CAINFO` and `SSL_CERT_FILE`; gwz 1.0.17 on Windows ignores those two variables. A CA file named in your environment is trusted, including a corporate interception CA. A file named by either variable that is empty, has no certificate, is garbled or is in the `TRUSTED CERTIFICATE` format is refused before connecting, so a stale variable that did nothing on gwz 1.0.17 now fails (a missing or unreadable file is refused too, by the code; that case was not measured on Windows).

## S.3 Proxy

- What is supported: one static proxy in the machine's WinHTTP settings (`netsh winhttp show proxy`), and a proxy that does not ask for a login. PAC scripts are not supported. Only the **machine** proxy controls GWZ. `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` (and lowercase forms) and git's `http.proxy` have no effect, even when a machine proxy is set. This is the opposite of macOS and Linux.
- An `https://` repository goes through the machine proxy by asking it to open a tunnel. **This was measured with gwz 1.0.17 against one public HTTPS server; GWZ's own transport is not yet tested through a proxy until step 4.7 lands** (step 4.6 reads the setting). A bare `host:port` or an `https=` entry in a list applies; a list with no entry for https (for example `http=` only or `socks=` only) is **refused by GWZ**, naming the machine proxy setting and giving the next action: the administrator adds an `https=` entry (`netsh winhttp set proxy proxy-server="http=proxy:8080;https=proxy:8080"`), or `--transport native`, which connects directly as gwz 1.0.17 did, where that is acceptable (the exact text is design section 14, item 13, T1). Cases not measured even with gwz 1.0.17 (an IP-address HTTPS server, a redirect through a proxy, this computer's own address as an HTTPS server, a proxy entry with no port) follow one rule: the proxy is used or GWZ refuses, and the request does not go direct on a guess.
- Requests to `localhost` (any case), any `127.x.x.x` address, `[::1]` and this computer's own address written as an IP address go direct without a bypass list. `localhost.` with a trailing dot does not. The token `<-loopback>` in a bypass list does nothing.
- A bypass list can be separated by semicolons, spaces or commas and may use `*` wildcards, `<local>` (names without a dot), a port, or a leading `http://` or `https://` (for an `https://` repository, `https://host` matches and `http://host` does not).
- If the machine proxy setting uses a form GWZ cannot interpret (including more than one bare entry), GWZ refuses and names the machine proxy setting; it never guesses or goes direct.
- **If the proxy asks for a login (407), GWZ refuses** and says proxy sign-in is not supported. The message does not point to the native transport, which refuses a 407 too.

## S.4 SSH

- GWZ uses **Pageant** if it is running, otherwise the OpenSSH agent named by `SSH_AUTH_SOCK`, otherwise the default OpenSSH agent. It chooses once per operation and does not switch agent if one fails. If Pageant is running but has no usable key, GWZ reports that and does not try the OpenSSH agent. When Pageant is chosen over a set `SSH_AUTH_SOCK`, the message says so; to use the OpenSSH agent, close Pageant (gwz 1.0.17 also prefers Pageant).
- The OpenSSH Authentication Agent service must be **running** for GWZ to use it; it may be disabled on your machine (it was on the test machine). The message for a stopped service gives the commands to start it.
- Remote machine pipes and Unix-style socket paths in `SSH_AUTH_SOCK` are refused, and the message names `SSH_AUTH_SOCK`.
- While Pageant waits for your confirmation, the stall and setup-budget clocks keep running (9 and 30 seconds by default; `--ssh-timeout`). A Pageant set to confirm every use whose prompt stays open longer fails that attempt, with a message that names `--ssh-timeout`; raise it, or use `--transport native`, which waits as gwz 1.0.17 did. A passphrase entered on first use succeeds on retry. The same is true of an ssh-agent that confirms every use (`ssh-add -c`) on macOS and Linux today.
- A key pinned with `--identity`, `--remote-identity` or `gwz auth identity` is the only key offered; the message names the pin and how to remove it. When no key is pinned GWZ tries no key file at all, only the agent: converting a key file to PEM does nothing until it is pinned (`gwz auth identity <remote> --set <full path>`, undone with `--unset`).
- The SSH home is chosen from `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE`. It supplies both `known_hosts` and `~/` key paths, so **a key path starting with `~/` now uses the same home as `known_hosts`** (on gwz 1.0.17 it used `USERPROFILE`). An empty `HOME` is an error, not a missing one. A host missing from `known_hosts` gets the printed path and the commands to add an RSA entry.
- **Folder and key names with non-ASCII characters now work** (on gwz 1.0.17 they fail).
- Host keys and key files are limited on Windows: **only RSA host keys and unencrypted RSA keys in PEM format** work. An `ecdsa` or `ed25519`-only `known_hosts` entry fails; the message tells you to add an RSA entry with `ssh-keyscan -t rsa`. A key file that is not RSA PEM fails; the message says so, and for an RSA key tells you to convert **a copy** with `ssh-keygen -p -m PEM` (which removes a passphrase from the copy), and for other key types to load the key into an agent. Agents can hold RSA, ECDSA and ed25519 keys. RSA keys from an agent are signed with `rsa-sha2-512` only, so a server that accepts only `ssh-rsa` or `rsa-sha2-256` is refused, and the message says why.
- A dropped connection before login is retried; on gwz 1.0.17 it was a single attempt.
- Messages now say what failed, in place of `failed to set hostkey preference`, an empty `failed to authenticate SSH session:`, and the shared "no agent" text for unsupported `SSH_AUTH_SOCK` forms.
- GWZ does not check who created the agent pipe or who owns a Pageant. Another account on the same computer could create the default agent pipe while the service is stopped and see who you are, which keys it offers and what GWZ asks it to sign, but it cannot sign for you.

## S.5 What is not promised

Kerberos (the successful tests used NTLM); a hardware security key; certificates held in Pageant or in the OpenSSH agent; Pageant and the OpenSSH agent together; SSH host keys other than RSA; key files other than unencrypted RSA PEM; proxy sign-in; PAC scripts (and whether the native transport follows them); a server that requires channel binding on a remote machine (it has been tested on this computer only); GWZ's own transport through a machine proxy (only gwz 1.0.17 was measured, against one public HTTPS server, until step 4.7 lands); through a proxy, an IP-address HTTPS server, a redirect from one HTTPS server to another, this computer's own address as an HTTPS server and a proxy entry with no port; the OpenSSH agent beyond RSA, ECDSA and ed25519 keys through its default pipe; filenames on macOS and Linux (not audited).

## S.6 Differences from gwz 1.0.17 on Windows, in one list

| # | Change | gwz 1.0.17 | The new transport |
|---|---|---|---|
| 1 | Non-ASCII folder and key names | fails | works |
| 2 | Digest | Digest-only: with a git helper configured, 15 retries then an error; with no helper it fails at once. Digest beside Basic fails the same way | Digest-only fails at once; Digest beside another scheme is ignored and the other scheme is used |
| 3 | Server rejects your Windows account | retried up to 15 times; may ask a helper each time | one error naming the scheme; no retry; no helper; no setting changes this |
| 4 | Redirects | full-address redirects followed over HTTPS only; path-only redirects never; none over HTTP | path-only redirects are followed too (at most five); credentials never cross to another address |
| 5 | Server challenges the upload step | fails (empty resend, or "request must be resent") | refused with a plain message; data never resent |
| 6 | `~/` in `--identity` | uses `USERPROFILE` | uses the same home as `known_hosts` |
| 7 | SSH drop before login | one attempt | retried |
| 8 | Error messages | opaque or shared text | text that names the cause and the fix |
| 9 | Machine proxy list with no entry for https | connects directly, ignoring the proxy | refused, naming the machine proxy setting |
| 10 | Plain `http://` remote | carried like any remote | carried by the native transport, with a warning |
| 11 | Trusted certificates | the Windows store only | the Windows store plus the files named by `GIT_SSL_CAINFO` and `SSL_CERT_FILE` |
| 12 | Machine proxy with more than one bare entry | uses the first | refused as ambiguous |
| 13 | A proxy answering 407 | opaque error | one plain message: proxy sign-in is not supported |
| 14 | A Pageant prompt that stays open longer than `--ssh-timeout` | waits for the prompt | the attempt fails with a message naming `--ssh-timeout`; `--transport native` waits as 1.0.17 did |

Unchanged on purpose: environment and git proxies are ignored on Windows; your Windows account can be offered to any host that asks; RSA-only host keys and unencrypted RSA PEM keys; RSA agent signatures at `rsa-sha2-512` only; no refusal of a Pageant owned by another user; the agent pipe's creator is not checked.
