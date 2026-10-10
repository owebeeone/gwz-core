# Review package: Windows transport parity design (TR1.8), plan step 2.4

Date: 2026-10-11. Status: **a package for the step 2.4 dual review plus Surface; no review has run.** It decides nothing and authorizes no implementation, commit, tag, push or publication. The lane owner dispatches the reviewers; this package does not.

The object is [`GwzTransportWindowsParityDesign.md`](GwzTransportWindowsParityDesign.md), the DRAFT revision of 2026-10-11 (plan step 2.1's output, with the 2026-10-10 host rows filled in). The plan's step 2.4 says what the three reviews are: Consistency and Safety on the design with the evidence from steps 2.2, 2.3 and 2.3b, and Surface on the user guide alone. At most two architectural remediation rounds follow. All P0, P1 and P2 findings block.

## 1. The exact documents and the tuple

All paths are under `gwz-core/dev-docs/` of the lane `gwz-dev-tr18freeze`. SHA-256 values are of the files as they stand when this package was written; the lane owner recomputes them at the commit that settles the tuple and replaces them here.

| Role | Document | SHA-256 (first 16) |
|---|---|---|
| **Object** | `GwzTransportWindowsParityDesign.md` | `8d34c609196f3211` |
| Controlling record | `GwzTransportTR18-OperatorDecisions.md` ("Operator discussion record" controls over its per-item narrative) | `61e787a87c9e5a26` |
| Delta (what MAIN and the evidence changed) | `GwzTransportWindowsParityDesign-Delta.md` | `3a1596fdfa9a0c54` |
| Amendment 2, revision 8 DRAFT (skim review not run) | `GwzTransportReleasePlanAmendment-2.md` | `5d462c6902e7c26c` |
| Its skim package | `GwzTransportReleasePlanAmendment-2-SkimPackage-8.md` | `8d5b4fa8ebaa1172` |
| Plan, step 2.4 and section 8 (OQ1 to OQ16) | `GwzTransportWindowsParityPlan.md` | `91783076ef12a7d9` |
| Baseline and dispositions | `GwzTransportWindowsBaseline.md`, `GwzTransportWindowsProofDispositions-DRAFT.md` | `d3eb823109122df2`, `feb7e9916e9bf3d1` |
| **Surface object** | `GwzTransportWindowsUserGuide-DRAFT.md` | `6752211da4bdc1e1` |
| Accepted contracts the design must agree with | `GwzTransportCredentialHelperTimingAmendment.md`, `GwzTransportCredentialHelperConfigurationViewAmendment.md`, `GwzTransportSshHelperClockAmendment.md`; `../../dev-docs/GwzSspiDesign.md`, `GwzSspiAcceptance.md`; `../../dev-docs/GwzWindowsHttpsIntegrationImplementation-Verdict-3.md` | n/a (accepted) |

**Repository tuple of the lane** (read-only `git rev-parse` at writing): workspace root `72922e4c`; gwz-core `2f27c05f` (the design and amendment revision 8 are in commit `03438559`; this package and the filled design are uncommitted in the lane until the lane owner commits them); gwz-transport `6146bd58`; gwz-cli `21d62310`; gwz-py `5950ba38`; gwz-sspi `364ccc77`; git2-rs `d13951f7`; gwz-core-evidence `12b46d90`. The design's own "evidence tuple" paragraph still quotes the delta's older tuple (gwz-transport `77f89cdf`, evidence `d7ca6a38`); the root owner records the settled tuple.

**Evidence** (private, `gwz-core-evidence/campaigns/transport-qualification/runs/`, access required; the public record is the design, delta, decision list and baseline). Short names are the design's: RS `2026-10-10-tr18-windows-ssh-rows` (90 rows), RA `...-windows-auth-rows` (35, HTTP), RR `...-redirect-parity`, RM `...-b17-macos-linux`, **RH** `...-td9-user-root-trust` (52 HTTPS rows plus a 10-row redirect supplement), **RP** `...-td10-machine-proxy` (82 rows), **RN** `...-td12-agent-service` (first run partial; the run of record is `raw/native-run2-interactive`). Released Windows 1.0.17 is `gwz.exe` SHA-256 `cbab5e9b...`. RH and RP are committed in gwz-core-evidence `12b46d90`; **RN's second run is not in that commit** (it landed in the source workspace after the lane was cloned), so the evidence commit the reviewers receive must include it (section 8, item 6).

**Who receives what.**

- Consistency and Safety: the design plus the controlling graph in the table, the canonical review-loop prompts and report paths, **(a)** the design's section 14 list of removed and revised claims, **(b)** the design's section 11 reading of the GO rule (a row that cannot run discharges it by having its claim removed and listed; if the reviewers read the rule as requiring an executed result for every row, the fallback is an amendment of the rule by the lane owner or the operator, never a provisional clause), and **(c)** this package's section 4 questions.
- Surface: the user guide, existing help and docs, and **Annex S** below, and nothing else. The design's section 11 says Surface never receives the design or the implementation; the migration notes (S7.2 (1.1.0)) do not exist yet, so Annex S states the register of section 12 in a user's words with no design vocabulary. The lane owner should give Surface the guide and Annex S only, and cut everything else from this package.
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

**R10. Pageant confirmation and the `SetupClock` (Safety, Consistency).** Section 5 bounds a Pageant request by the remaining allowance of the connection's `SetupClock` and says no independent 120-second allowance exists, but does not say how the user's confirmation wait is classified among the clock's phases (network, local admission, local interaction, terminal). *Question:* does the omission leave a hang or a premature expiry, and which phase should it be?

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
7. **Plan.** Step 2.4's scope, and the steps the plan says consume TR1.8 (3.1, 3.2b, 3.3, 3.5, 3.6, 4.4, 4.6, 4.7, 4.8), are served by clauses that are specific enough to implement; step 4.8's explicit-identity residual is removed (R9).
8. **Counts.** Row counts, run sizes and the delta's tallies match the archives (R6, R7, R17).

### 5.2 Safety

Judge explicitly, each with a verdict and the counterexample you tried:

1. **No server-identity check on the agent pipe (TD12, OQ4(a)).** The native service's pipe server is `ssh-agent.exe` as LocalSystem in logon session `0x3e7`; the caller is another session, and the pipe grants Authenticated Users read and write. Is admitting any local pipe server, with only name admission (no UNC, no traversal, no MinGW Unix form) and an identification-level impersonation limit, acceptable? What can a local attacker who creates a pipe named by `SSH_AUTH_SOCK` or the default name before the service does?
2. **No cross-user or cross-logon-session refusal of a found Pageant (TD11 B).** The retained checks are the pinned window-to-process identity, creation identity, a unique per-request mapping with a caller+SYSTEM ACL, and one send with no resend. Is that enough for a Pageant owned by another user in the same desktop? (P01's negative did not run.)
3. **Default credentials to any host (OD16) and NTLM exposure.** The logon session answers any host that challenges Negotiate or NTLM, including an Internet host. Does the design bound the hazard it accepts (certificate trust first, no CONNECT credential, rejected identity terminal, no replay, eight rounds), and are the cross-origin redirect rules sufficient given they are unmeasured (R8)?
4. **Channel binding.** Capture from the actual final TLS handshake, per physical connection, for proxy and origin separately; native `None` or error refuses the SSPI attempt (R4); weak-hash upgrade; EPA satisfied and refused in the executed rows, loopback caveat (R15).
5. **Redirect and POST.** Credentials never cross origins; a challenged POST is refused and the body is never resent (1.0.17 resends an empty body); no credentialed retry of a partial or complete POST.
6. **Machine proxy.** Grammar admission and refusal (R12), the implicit bypass including the machine's own address (R14), the unmeasured https-origin selection and CONNECT (R13), an unresolvable proxy sends nothing direct, a parser failure never sends direct, every 407 refused with no credential offered, environment and git-configuration proxies ignored (and beaten by a named machine proxy).
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

The design's section 11 reads the GO rule as follows. Every row B01 to B18 and P01 to P08 needs an executed result or a recorded disposition, and every provisional clause is replaced by one physically proved design. **A row that cannot run discharges the rule by having its claim removed and listed in section 14.** After this fill every row approved on 2026-10-10 has executed; the rows with no execution are P01 (claim removed, TD11 B), P02 (stays UNEXECUTED, claim revised), P05 and P07 (dispositioned), the Mac trusted-HTTPS B17 (TD7 B), a real hardware key, Pageant-held certificates, Kerberos, and the https-origin machine-proxy selection and CONNECT leg. The reviewers say whether they accept this reading; if not, the fallback is an amendment of the rule by the lane owner or the operator.

## 7. Procedure

Use the review-loop skill's canonical prompts, with the paths in section 1. Do not start any review from this package; the lane owner dispatches. Findings are filed verbatim. The two architectural remediation rounds are a cap, not a target.

## 8. Open items for the lane owner before dispatch

These are outside this task's edits (documents it was not asked to change) or need a decision.

1. **Stale redirect sentences in amendment 2 revision 8: done (2026-10-11).** Amendment lines 179 and 227, its status bullet and a new changelog line, and the skim package (update note, table row, items 2 and 3, checklist question 5) now say that 1.0.17 follows an absolute `Location` over HTTPS and that the redirect row is transport-only only in its relative-`Location` and plain-HTTP forms. The hash of revision 8 changed (`a39f4764...` to the value in section 1), the skim package pins the new one, and this package pins both. The decision list's TD3 narrative (lines 103 to 114) still says "follows none"; its discussion record controls, so it needs no edit, but the delta's section 2 row 7 and section 4 B15 row are superseded for HTTPS.
2. **TD12's second run and the `HKCU` residue.** The service created empty `HKCU\Software\OpenSSH\Agent\...` keys on its first key add (zero subkeys; owned by SYSTEM). They were not deleted. The exact command is `reg delete HKCU\Software\OpenSSH /f` from an elevated session; the lane owner decides whether to run it.
3. **Companion documents the delta (section 6) says change at the refresh.** The baseline, the checkpoint, the plan's Appendix A and the user guide were not part of this task. In particular the baseline still lists B06, B08, B09, B11 to B18, P01 and P03 to P07 at their 2026-10-04 statuses.
4. **User guide: filled where settled (2026-10-11).** The guide now covers the RSA-only host-key and key-file limits and their fix messages (TD5), the ignored `HTTP_PROXY`, `http.proxy` and the other proxy variables with the machine proxy winning, the accepted machine proxy forms, the implicit local bypass and bypass-list syntax, the 407 refusal, redirect behaviour, the non-ASCII and `HOME`/`USERPROFILE` points, the agent service needing to be running, and that Kerberos, hardware keys, agent certificates and remote-host EPA are not tested. Two items are left as visible `OPEN ITEM` placeholders because they are not settled: the Pageant confirmation bound (R10) and the https-origin machine proxy and own-address bypass (R13, R14). Other placeholders in the guide are the accepted helper-timing wording. Surface may still find more.
5. **Evidence archive to fix.** RN's first-run table and `summary.json` label the refused add row `added`; RH's README says five authenticating EPA rows and the list has four; the README of RN now describes run 2 as the run of record.
6. **The evidence commit must include `raw/native-run2-interactive`** and the README update; the lane's gwz-core-evidence `12b46d90` does not. The design cites rows from it.
7. **The tuple.** Commit the design, this package and any amendment edit, then recompute the hashes in section 1 and record the settled tuple. The proposed gwz-core commit message is in the preparing agent's report.
8. **Exclude** `GwzRemoteTransportBugReport.md` (R11).

---

# Annex S: what a Windows user sees (for the Surface reviewer)

This annex describes the behaviour the proposed Windows release (1.1.0) is meant to have, compared with gwz 1.0.17 on Windows. It is a statement of intended behaviour, not a statement that anything is implemented or released. Read it with the user guide. It uses no internal design vocabulary on purpose.

## S.1 How GWZ chooses who logs in

- GWZ first asks the server without any credentials. If the server offers **Negotiate or NTLM** (Windows sign-in), alone or beside Basic or Digest, GWZ signs in with **your Windows account** and does not ask your configured git credential helper.
- If the only scheme the server offers that GWZ can use is **Basic**, GWZ asks your configured git helper once and sends what it returns. A GitHub token held by a helper is sent this way.
- GWZ **never answers Digest**. A server that offers only Digest is refused at once with a message naming Digest and how to switch transports. If the server offers Digest and another scheme, Digest is ignored and the other scheme is used.
- If the server rejects your Windows account, the operation stops with an error naming the scheme. GWZ does not try it again and does not turn to a helper credential.
- A helper that times out or is cancelled ends the operation.
- If the server challenges the upload step of a fetch (after the first request succeeded), GWZ refuses with a message and does not send the data again.
- **Warning:** your Windows sign-in can be offered to any web host that asks for it, including a host on the Internet. This is what gwz 1.0.17 already does.

## S.2 Redirects

GWZ follows a redirect when it first contacts a repository, to another address or to another path. It does not forward a sign-in from the first address to a different address; a different address that asks for credentials is answered as if you had gone there directly. On gwz 1.0.17 on Windows, a redirect that gives a full address is followed over HTTPS and one that gives only a path is not, and no redirect is followed over plain HTTP.

## S.3 Proxy

- On Windows only the **machine** proxy (the one `netsh winhttp show proxy` shows) controls GWZ. `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` (and lowercase forms) and git's `http.proxy` have no effect, even when a machine proxy is set. This is the opposite of macOS and Linux.
- Requests to `localhost` (any case), any `127.x.x.x` address, `[::1]` and this computer's own address go direct without a bypass list. `localhost.` with a trailing dot does not. The token `<-loopback>` in a bypass list does nothing.
- A bypass list can be separated by semicolons, spaces or commas and may use `*` wildcards, `<local>` (names without a dot), a port, or a leading `http://`.
- If the machine proxy setting uses a form GWZ cannot interpret, GWZ refuses and names the machine proxy setting; it never guesses or goes direct.
- **If the proxy asks for a login (407), GWZ refuses.** Proxy sign-in is not supported.
- Not yet tested end to end: reaching an HTTPS repository through a machine proxy. The proxy rules above were measured with plain HTTP addresses and are expected to apply to HTTPS ones.

## S.4 SSH

- GWZ uses **Pageant** if it is running, otherwise the OpenSSH agent named by `SSH_AUTH_SOCK`, otherwise the default OpenSSH agent. It chooses once per operation and does not switch agent if one fails. If Pageant is running but has no usable key, GWZ reports that and does not try the OpenSSH agent.
- The OpenSSH Authentication Agent service must be **running** for GWZ to use it; it may be disabled on your machine (it was on the test machine).
- Remote machine pipes and Unix-style socket paths in `SSH_AUTH_SOCK` are refused, and the message names `SSH_AUTH_SOCK`.
- The SSH home is chosen from `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE`. It supplies both `known_hosts` and `~/` key paths, so **a key path starting with `~/` now uses the same home as `known_hosts`** (on gwz 1.0.17 it used `USERPROFILE`). An empty `HOME` is an error, not a missing one.
- **Folder and key names with non-ASCII characters now work** (on gwz 1.0.17 they fail).
- Host keys and key files are limited on Windows: **only RSA host keys and unencrypted RSA keys in PEM format** work. An `ecdsa` or `ed25519`-only `known_hosts` entry fails; the message tells you to add an RSA entry with `ssh-keyscan -t rsa`. A key file that is not RSA PEM fails; the message says so, and for an RSA key tells you to convert it with `ssh-keygen -p -m PEM`, and for other key types to load the key into an agent. Agents can hold RSA, ECDSA and ed25519 keys. RSA keys from an agent are signed with `rsa-sha2-512` only, so a server that accepts only `ssh-rsa` or `rsa-sha2-256` is refused.
- A dropped connection before login is retried; on gwz 1.0.17 it was a single attempt.
- Messages now say what failed, in place of `failed to set hostkey preference`, an empty `failed to authenticate SSH session:`, and the shared "no agent" text for unsupported `SSH_AUTH_SOCK` forms.

## S.5 What is not promised

Kerberos (the successful tests used NTLM); a hardware security key; certificates held in Pageant or in the OpenSSH agent; Pageant and the OpenSSH agent together; SSH host keys other than RSA; key files other than unencrypted RSA PEM; proxy sign-in; a server that requires channel binding on a remote machine (it has been tested on this computer only); the OpenSSH agent beyond RSA, ECDSA and ed25519 keys through its default pipe; filenames on macOS and Linux (not audited).

## S.6 Differences from gwz 1.0.17 on Windows, in one list

| # | Change | gwz 1.0.17 | The new transport |
|---|---|---|---|
| 1 | Non-ASCII folder and key names | fails | works |
| 2 | Digest | Digest-only: with a git helper configured, 15 retries then an error; with no helper it fails at once. Digest beside Basic fails the same way | Digest-only fails at once; Digest beside another scheme is ignored and the other scheme is used |
| 3 | Server rejects your Windows account | retried up to 15 times; may ask a helper each time | one error naming the scheme; no retry; no helper |
| 4 | Redirects | full-address redirects followed over HTTPS only; path-only redirects never; none over HTTP | all followed; credentials never cross to another address |
| 5 | Server challenges the upload step | fails (empty resend, or "request must be resent") | refused with a plain message; data never resent |
| 6 | `~/` in `--identity` | uses `USERPROFILE` | uses the same home as `known_hosts` |
| 7 | SSH drop before login | one attempt | retried |
| 8 | Error messages | opaque or shared text | text that names the cause and the fix |

Unchanged on purpose: environment and git proxies are ignored on Windows; your Windows account can be offered to any host that asks; RSA-only host keys and unencrypted RSA PEM keys; RSA agent signatures at `rsa-sha2-512` only; no refusal of a Pageant owned by another user; the agent pipe's server is not identity-checked.
