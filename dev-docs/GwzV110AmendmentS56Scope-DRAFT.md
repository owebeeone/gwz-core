# GWZ 1.1.0 amendment: three claims leave the S5.6 column and are named unsupported in the 1.1.0 notes

Date: 2026-10-18. Status: **DRAFT (lane `gapsa`), not accepted, not reviewed.** It changes no code and authorizes no run, commit, tag or publish. It takes effect only when the lane owner accepts it. It exists because S5.6 says that an unsupported mark on a cell the release puts "in this release" fails the step and blocks S7.5 and Phase 8 "unless an accepted amendment first removes that cell" (`GwzV110Plan.md` S5.6). This is that amendment, for three claims.

Authority: the operator's ruling of 2026-10-18 on `GwzV110S56CellTable.md` section 3, as relayed to this lane: **accept every recommendation**. The recommendation for question 3.5 is "one short amendment removing these three from the 1.1.0 column". The ruling has no D-number yet: D34 to D38 in `GwzTransportTR18-OperatorDecisions.md` do not include it, and the lane owner should file it there (open item 1, section 6).

## 1. What this amendment changes

Three claims stop being 1.1.0 claims. Each is removed from the S5.6 sign-off (the `GwzV110S56CellTable.md` cell it names) and from every place that put it "in this release", and each is named in the 1.1.0 notes as unsupported.

| # | Claim removed | Cell table row | Where the claim came from | Evidence that exists, and why it is not enough |
|---|---|---|---|---|
| 1 | **A manual hardware security-key row**: a real `sk-ssh-ed25519@openssh.com` or `sk-ecdsa-sha2-nistp256@openssh.com` signature (touch) through the transport, run by hand with a hardware key | 9.4 (first half; macOS, Linux, Windows), design limitation L2 | `GwzTransportReleasePlanAmendment.md` §4 (line 361: "the security-key case, by a named manual row that S7.2 runs, with a hardware key, before its notes are written"; "needs the operator's go"); `GwzTransportReleasePlanAmendment-2.md` §3.19 (TR2.8's tests, line 489) and its §4 table (line 138: "the manual security-key row, run on the transport (§3.19) \| 1.1.0") | None. No hardware key was ever run. TD8 B left the row "not qualified". What does run is a software authenticator (rows 9.1 to 9.3 on macOS and Linux) and, on Windows, the emulated declining-identity skip (`x4-pipe-security-key-then-rsa`) |
| 2 | **Kerberos** as an authentication mechanism of HTTPS `Negotiate` on Windows | 18.19 (Windows), design limitation L1 | `GwzTransportWindowsParityPlan.md` OQ10 (line 751) and step 6.3 (line 632: "OQ10 names the likely ones (Digest, Kerberos)"); OQ10 (a) recommended removing it from the "in this release" column "by amendment"; no filed amendment names it | None. Every successful `Negotiate` exchange in the record selected NTLM (`GwzTransportWindowsParityDesign.md` §7, line 203: "NTLM and Kerberos are distinct protocols"); dabeest has no domain. 1.0.17 was not run against a domain either |
| 3 | **A certificate held by Pageant, or by the Windows OpenSSH Authentication Agent service** (design limitation L3) | 9.4 (second half; Windows) | the design's `X4` row (line 289: "X4's real security key and Pageant-held certificate are not qualified (TD8)") and B08 (line 273: "Pageant with the pipe, hardware keys and native-agent certificates not covered") | An Ed25519 certificate from an agent pipe the test owns authenticates, and an RSA certificate does not, on 1.0.17 and on the candidate alike (design §4, line 104, cited `RS/x4-pipe-*-certificate`; the candidate's `agent_keys.rs` type listing runs on Windows). No certificate was loaded into the real service or into Pageant (design §4, line 100; L3: it needs puttygen's GUI) |

Nothing else changes. In particular this amendment does not touch: the software-authenticator rows 9.1 to 9.3 on macOS and Linux (they stay evidenced); the Windows twins of the security-key, certificate and DSA rows (question 3.4, ruled separately); exact agent-key selection (D38); Digest (refused by D3 and TD2); the real Authentication Agent service for ordinary RSA, ECDSA and Ed25519 keys (row 18.4, question 3.8).

## 2. The text this amendment replaces

Each replacement is a removal. When the amendment is accepted, the lane owner applies them; this draft edits none of these files.

| File | Place | Change |
|---|---|---|
| `GwzV110Plan.md` | §2, "In this release" column | Add a sentence below the table: "Not in this release, and named unsupported in the 1.1.0 notes: a real hardware security-key signature; Kerberos; a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service. (`GwzV110AmendmentS56Scope.md`.)" The three items were never in the table's cells, which say only "SSH and gh-only HTTPS"; the amendment writes down what that sentence now excludes, because A2 widened it (below) |
| `GwzTransportReleasePlanAmendment-2.md` | §3.19 / TR2.8, the last test bullet (line 489): "Amendment 1's §4 manual security-key row runs on the transport, with a hardware key, and needs the operator's go." | Replace with: "Amendment 1's §4 manual security-key row is not run in 1.1.0 and the real-key claim is unsupported (`GwzV110AmendmentS56Scope.md`). TR2.8's security-key rows run through a software authenticator." |
| `GwzTransportReleasePlanAmendment-2.md` | §4 table (line 138) | The row's "the manual security-key row" leaves the 1.1.0 column: "§4: Phase 2's tests, S5.6's cell and S7.2's notes run on the transport (§3.19); the manual security-key row is unsupported in 1.1.0 (`GwzV110AmendmentS56Scope.md`) \| 1.1.0" |
| `GwzTransportReleasePlanAmendment.md` | §4 (line 361), the security-key bullet | Annotate, as amendment 2 annotates other lines: "Removed from 1.1.0 by `GwzV110AmendmentS56Scope.md`." |
| `GwzTransportWindowsParityPlan.md` | OQ10 (line 751) and step 6.3 (line 632) | OQ10 (a)'s "both removed from the 'in this release' column by amendment" is satisfied for Kerberos by this amendment (for Digest by D3, whose refusal rows are evidenced). Step 6.3's "OQ10 names the likely ones (Digest, Kerberos)" no longer blocks S5.6 for Kerberos |
| `GwzTransportWindowsParityDesign.md` | §13 L1, L2, L3 (lines 324 to 326) and §14 item 10 (line 358) | Change the limitations' status from "no claim is made" to "unsupported in 1.1.0 (`GwzV110AmendmentS56Scope.md`)"; the facts in the rows stay |
| `GwzV110S56CellTable.md` | rows 9.4 and 18.19; §2 counts; §3 question 3.5; §7 register | 9.4: `G` becomes `U` on macOS, Linux and Windows, citing this amendment; 18.19: `G` becomes `U` on Windows. §2: `G` falls from 5 to 4 on macOS, 5 to 4 on Linux and 7 to 5 on Windows (17 to 13 cells, 9 rows to 7); `U` rises from 4 to 5, 4 to 5 and 4 to 6. The §7 register's "Kerberos; hardware security key; Pageant-held or native-agent certificate" line changes its mark from `G` to `U` and its authority to this amendment |
| `GwzV110RouteLedger-DRAFT.md`, the S7.2 help and migration notes (C5), gwz-cli `Releases.md` "Unreleased" | the notes' "not supported" list | Add the three entries of section 3. The ledger's rule applies: an unsupported cell is not advertised, and no help text may promise any of the three |

## 3. The text of the 1.1.0 notes

Proposed wording for the notes' "Not supported in 1.1.0" list (the lane owner may reword; the meaning is the ruling's). "Unsupported" here means what V110 §2 means by "recorded as unsupported in 1.1.0": **no claim is made, it is not advertised, and no evidence stands behind it.** It does not say the transport refuses the case, and this amendment adds no refusal.

1. **Hardware security keys.** gwz 1.1.0 qualifies SSH keys of the types `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` only through a software authenticator. Signing with a physical key, including waiting for its touch, has not been tried on the transport and is not supported in 1.1.0.
2. **Kerberos.** On Windows, an HTTPS server that offers `Negotiate` is answered by the logon session's default credentials, as gwz 1.0.17 does. gwz 1.1.0 has tested that exchange only where it selects NTLM. Kerberos (a domain-joined host with a ticket) has not been tested and is not supported in 1.1.0.
3. **Certificates held by Pageant or by the Windows OpenSSH Authentication Agent service.** SSH certificates are qualified from an agent the test owns. A certificate loaded into Pageant or into the Windows service's agent has not been tried and is not supported in 1.1.0. As in gwz 1.0.17, an Ed25519 certificate from an agent authenticates in the tests and an RSA certificate does not.

## 4. Effect on S5.6, S7.5 and Phase 8

- The four cells (9.4 on three platforms, 18.19 on Windows) stop being `G`. They carry `U` and cite this amendment, so S5.6 is not failed by them.
- The remaining `G` cells are those of questions 3.1 to 3.4, 3.8 and 3.9 (rows 9.2, 9.3, 10.2, 10.7, 12.1b, 17.11, 18.4). Questions 3.2, 3.3 and 3.9 have evidence runs (`2026-10-17-gapsa`); 3.1, 3.4 and 3.8 are in other lanes.
- This amendment is a precondition for Phase 8 only in that S5.6 cannot close without it. It adds no step and no evidence requirement.

## 5. What this amendment does not decide

- It does not say the three cases are broken, only that no evidence supports them. If a later release qualifies one, that is an amendment putting the cell back, as V110 §2 says of Linux ARM64 and macOS x86-64.
- It adds no refusal, no help text and no flag. If a physical key, a Kerberos ticket or an agent-held certificate fails, the user sees whatever the transport reports today; the notes' wording above promises nothing about it.
- It does not alter `exact_agent_identity` (D38), the Windows capability answer (D30), or any Pageant row that is evidenced (`pageant-rsa-key`, `pageant-beats-pipe`, `pageant-empty-no-fallthrough`).
- It names no review. Under the program's loop (`AgentProcessRules.md`) an amendment is a change to a plan text; the lane owner chooses the route (a skim, as amendment 2's revisions 3 to 8 had, or the dual review).

## 6. Open items for the lane owner

1. **File the ruling.** The operator's 2026-10-18 ruling on the cell table has no D-number. Add it to `GwzTransportTR18-OperatorDecisions.md` and cite the number here.
2. **The word "in this release".** V110 §2's table does not list hardware keys, Kerberos or agent-held certificates; they entered the release through A2 §3.19 (key types) and TR1.8 (Windows authentication). Section 2's first row writes the exclusion beside the table so it has one home. If the lane owner prefers to leave §2 untouched, the same sentence can live in the S7.2 notes alone; S5.6's rule is satisfied either way, because the cells become `U` with an accepted amendment.
3. **Keep the facts when the status changes.** The L1 to L3 rows state what was run. Changing their status must not delete those facts (the Ed25519 certificate from an owned pipe agent; NTLM selected by `Negotiate`).
4. **Question 3.4 is separate.** If the operator rules that Windows security keys, certificates and DSA are unsupported rather than twinned, that is another amendment, and its cells (9.2 and 9.3 on Windows) would be `U`; this document does not cover it.
