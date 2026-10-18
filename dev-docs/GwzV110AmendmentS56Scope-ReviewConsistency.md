# Consistency review: GWZ 1.1.0 S5.6 scope amendment (D41)

**Object:** `gwz-core/dev-docs/GwzV110AmendmentS56Scope-DRAFT.md` at gwz-core `84e2570d4804d7452f5e2af70b25f0f2b1273988` (60 lines). The file is the same at main HEAD `31d5183b1c0d11c03155133aef816dbc0d8706aa`.
**Date:** 2026-10-18
**Axis:** Consistency, read-only. I modified nothing.
**Verdict:** **NO-GO.** P0 0, P1 0, P2 5, P3 4.

## §0 Evidence base

- **Commits.** I checked them at the start and again at the end, and nothing moved:
  - object `84e2570d`;
  - gwz-core HEAD `31d5183b`;
  - gwz-cli `6b4fd2e`;
  - gwz-core-evidence `767d3297`.

  The only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`, which is not in scope.
- **Requirement.** D41 in `GwzTransportTR18-OperatorDecisions.md:84`, filed in `accb1b04`, which came after the object. I also read TD8 (:226-232) and D30 to D40 (:69-83).
- **Cell table.** `GwzV110S56CellTable.md` at `50f1b16a`, the same at the object and at HEAD. I read §2 counts (:27-52), §3 items 3.4 and 3.5 (:66-68), rows 9.2 to 9.4 (:192-194), row 18.19 (:275) and the §7 register (:277-287).
- **Release plans.**
  - `GwzV110Plan.md`: the status line (:3), §1 (:32-44), §2 (:45-69), S5.6 (:263-274) and S7.2.
  - `GwzTransportReleasePlanAmendment-2.md`: §3.2 (:92-106), §3.3 table line 138, §3.19 (:456-511, including :489), and a check of §3.21 (:565-649).
  - `GwzTransportReleasePlanAmendment.md`: §3.2 (:93-109) and §4 (:361).
- **Windows parity.**
  - `GwzTransportWindowsParityPlan.md`: step 6.3 (:632) and OQ10 (:749-753).
  - `GwzTransportWindowsParityDesign.md`: lines 100, 104, 203, 273, 289, 324-326 and 358. Each line number the object cites is correct at both the object and HEAD.
- **Exact agent identity.** `GwzExactAgentIdentityDesign-DRAFT.md`, accepted: §2.4, §3.5 (:379-389), §4.2 (:444-461), §6.1 rows C1 to C4 (:555-558) and steps 4.1 and 4.2.
- **Runs.**
  - `2026-10-18-gapsb` README, whose test code landed in gwz-core `e976cbbe`, after the object.
  - `2026-10-17-gapsa` README, whose verdict table names this amendment.
- **Notes and ledger drafts.**
  - `gwz-cli/docs/Releases.md` at `6b4fd2e`: lines 137-171.
  - `GwzV110RouteLedger-DRAFT.md`: §3 rows L1 to L13 and §5.
  - `gwz-py/docs/Migration-1.1.0-DRAFT.md`.
  - `GwzTransportWindowsUserGuide-DRAFT.md`: lines 330-337, 583 and 612.
- **Code at HEAD.**
  - `src/git/endpoint/ssh_tests/key_types.rs`: the Unix RSA certificate row at :100-109, and the Windows arm at :191-282.
  - `src/git/endpoint/https_worker/native.rs:256-281`: the Kerberos mechanism is accepted.
  - `agent_client.rs:79-130` and `agent_socket.rs:15-30`: the agent's sign wait is bounded by `Control`'s stall and aggregate bounds.

## Findings

### F1 (P2): The amendment edits a superseded scope table and leaves the controlling "in this release" rows that carry the three claims

- **Root cause.** The amendment adds an exclusion sentence below V110 §2. That table never named the three items. The scope rows that actually put them in 1.1.0 are not edited.
- **Location.** §2, row 1 (line 25). §6 item 2 (line 58) admits the claims "entered the release through A2 §3.19 (key types) and TR1.8 (Windows authentication)", but no §2 row edits those texts.
- **Violated text.**
  - `GwzV110Plan.md` S5.6 says an amendment must remove the cell "from §1 and from the §2 'in this release' column".
  - V110's status line (:3) says V110 is superseded for release scope by `GwzTransportReleasePlan.md`.
  - The controlling rows are:
    - A2 §3.19 (:507): A1 §3.2's row now reads "SSH through an agent with any key type 1.0.17 uses (TR2.8)", with "—" in the unsupported column. Hardware keys and Pageant-held certificates are key types 1.0.17 uses.
    - A2 §3.2 (:104): the Windows row "the transport does what 1.0.17's native path does: Pageant, … the logon session's default credentials under OD16", also with "—". Kerberos falls under this row, and so do Pageant-held certificates.
    - A2 §3.19's OD11 list (:487-488) names "security keys … whose touch the agent asks for" and "OpenSSH certificates of those types".
    - The sign-off at A2 §3.10 (:406) covers "TR2.8's agent key types".
- **Consequence.** Once the amendment is accepted, the authoritative scope still lists all three as in this release, with nothing marked unsupported. So S5.6's condition for a `U` mark is not met in the controlling text, and S7.2 is left with two scope statements that contradict each other.
- **Correction.**
  - Add exact replacement rows to §2:
    - A1 §3.2 as A2 §3.19 rewrote it: put "a physical hardware security-key signature; a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service (GwzV110AmendmentS56Scope)" in the unsupported column instead of "—".
    - A2 §3.2's Windows row: replace "—" with "Kerberos selected by Negotiate".
  - Annotate OD11's list (:487-488) and A2 §3.10 line 406.
  - State that V110 §1 names none of the three and needs no change.
  - Keep the V110 §2 sentence, but present it as a cross-reference rather than the home of the exclusion.

### F2 (P2): Windows row 9.2 is left with no owner, and notes item 1 claims software-authenticator coverage on Windows

- **Root cause.** The amendment assumes question 3.4 is closed elsewhere. In fact gapsb hands Windows 9.2 back to this amendment.
- **Location.**
  - §4 bullet 2 (line 45): "3.1, 3.4 and 3.8 are in other lanes".
  - §6 item 4 (line 60): "its cells (9.2 and 9.3 on Windows) would be `U`".
  - §3 item 1 (line 38).
  - §2 row 2's replacement text (line 26): "TR2.8's security-key rows run through a software authenticator".
- **Violated text.**
  - gapsb's verdict table marks 9.2 W "Unchanged: 'not qualified' on Windows … (lane `gapsa`'s amendment covers it)". The object, at lines 17 and 60, explicitly does not cover it.
  - gapsb also moves 9.3 W to PASS, so 9.3 W must not become `U`.
  - Cell table row 9.2 W is `G`: there is no end-to-end Windows security-key or DSA row.
  - D41 does not name 9.2 W. So the amendment is right not to remove it, but it is wrong to describe it as handled.
- **Consequence.**
  - The S5.6 blocker 9.2 W has no lane, ruling or amendment, and each document points at the other.
  - Notes item 1 ("gwz 1.1.0 qualifies … `sk-*` only through a software authenticator") tells Windows users their security keys are qualified at the software level. No Windows row supports that.
  - §6 item 4 would `U`-mark a cell that now has evidence (9.3 W).
- **Correction.**
  - Restate §4 and §6 item 4: 9.3 W is evidenced by `2026-10-18-gapsb`. 9.2 W (security keys through a software authenticator, and DSA, on Windows) stays `G`, is not covered by D41, and needs its own operator ruling or rows.
  - Ask the lane owner to correct gapsb's pointer.
  - Notes item 1 should read "on macOS and Linux, … through a software authenticator". The Windows status follows that ruling.
  - Qualify §2 row 2's replacement text the same way: "on macOS and Linux".

### F3 (P2): Notes item 3 and §1 row 3 state certificate behaviour that is wrong on macOS and Linux and stale on Windows

- **Root cause.** The certificate sentence was written from the 1.0.17 Windows baseline alone, before gapsb, and it is presented as a statement about every platform.
- **Location.**
  - §3 item 3 (line 40): "SSH certificates are qualified from an agent the test owns … As in gwz 1.0.17, an Ed25519 certificate from an agent authenticates in the tests and an RSA certificate does not."
  - §1 row 3's evidence column (line 15): "on 1.0.17 and on the candidate alike (design §4, line 104, cited `RS/x4-pipe-*-certificate`…)".
- **Violated text.**
  - `key_types.rs:100-109`: on Unix an RSA certificate authenticates at `rsa-sha2-512` ("RSA-CERT", flag 4).
  - gapsb, 3.4 and F1: Windows ECDSA P-256, P-384 and P-521 certificates authenticate.
  - gapsb F2: the Windows RSA-certificate failure comes from the server's `PubkeyAcceptedAlgorithms` refusing `ssh-rsa-cert-v01@openssh.com`. It is not a property of gwz.
  - The design line 104 that the object cites rests on RS, which is a run of released 1.0.17 only. Cell table row 9.3 W stated that the candidate had no end-to-end certificate row when the object was written.
  - gapsb F4: a refused certificate gets the generic `ssh setup failed: Authentication` message.
- **Consequence.** The 1.1.0 notes would tell macOS and Linux users that RSA certificates do not work, which is false. They would leave out ECDSA certificates on Windows. And they would present a server-side algorithm refusal as a gwz limit. "An agent the test owns" also gives users nothing they can act on.
- **Correction.**
  - Re-cite §1 row 3 to `2026-10-18-gapsb`.
  - Reword item 3 by platform. For example: "macOS and Linux: certificates of each listed type authenticate from an SSH agent. Windows: Ed25519 and ECDSA certificates authenticate from an agent on a pipe other than the OpenSSH service; an RSA certificate fails where the server does not accept `ssh-rsa-cert-v01@openssh.com` (as 1.0.17). A certificate held in Pageant or in the Windows OpenSSH Authentication Agent service has not been tried and is not supported in 1.1.0."

### F4 (P2): The existing notes text "certificates held by an agent" is not superseded and covers evidenced cells

- **Root cause.** §2 tells the lane owner to add entries to the notes. It does not name the existing notes sentence that already covers the topic, and that sentence is broader than D41.
- **Location.** §2, last row (line 32).
- **Violated text.**
  - D41 removes only "Pageant- or native-agent-held certificates".
  - `gwz-cli/docs/Releases.md:154` reads "Not promised: Kerberos, hardware security keys, certificates held by an agent, PAC scripts."
  - Cell table 9.3 M and L are `E`, and 9.3 W is now evidenced by gapsb.
  - The Windows guide has parallel text at `GwzTransportWindowsUserGuide-DRAFT.md:335-337` and :612 ("Not promised … certificates held in Pageant or in the OpenSSH agent"). Releases.md:139 says this guide "moves into this documentation set at the release".
- **Consequence.** The published notes would hold both "not promised: certificates held by an agent", which disclaims the evidenced pipe-agent and Unix-agent certificate cells, and the new narrower "not supported" entry. They would also use two different terms, "not promised" and "not supported".
- **Correction.** Add exact replacement rows to §2:
  - For Releases.md:154, replace the "Not promised" clause with a pointer to the three D41 entries, worded as in F3. Keep PAC scripts in the sentence, since PAC scripts are not D41's.
  - Do the same for guide lines 335-337, 583 and 612.

### F5 (P2): The edits elsewhere are described, not given as exact text, and two targets do not exist as named

- **Root cause.** Several rows of the §2 table are instructions, not replacement text.
- **Location.** §2, rows 5 to 8 (lines 29-32).
- **Violated text.** The mandate and the review requirement call for exact text for each document that must change. The rows fall short as follows:
  - **WPD (line 30).** It says to change "no claim is made" in L1, L2 and L3. That phrase is not in rows :324-326. It appears in §14 item 10 (:358), which also lists L5, L7, L8, L11 and L15, and those must stay "no claim". The §13 preamble ("none is declared unsupported unless it says so") is not addressed.
  - **WPP (line 29).** It states an effect, not text for OQ10 (a) or step 6.3.
  - **Cell table (line 31).** "§3 question 3.5" has no change text.
  - **Ledger, notes and help (line 32).** No exact text is given.
    - Releases.md has no "Not supported in 1.1.0" list. Its sections are "Known limits of this release" (:158) and the Windows "Not promised" sentence.
    - The ledger's list is §5 item 5.
    - Ledger rows L1 (agent SSH, "Pageant, then … the default OpenSSH agent pipe") and L6 ("Negotiate or NTLM, answered by the Windows logon session") are advertised rows. They have no qualifier excluding Kerberos and agent-held certificates.
    - `gwz-py/docs/Migration-1.1.0-DRAFT.md`, which ledger §5 names as a notes target, is not listed.
    - "The S7.2 help" names no file.
- **Consequence.** Applying the amendment needs new drafting that has not been reviewed. Ledger L6 would still advertise Negotiate without saying it is NTLM only, which conflicts with the amendment's own rule (line 32) that no help text may promise any of the three.
- **Correction.**
  - For each of these rows, give the exact old and new text.
  - Split WPD §14 item 10 into an "unsupported in 1.1.0" sentence for L1 to L3 and a "no claim" sentence for the rest. Amend the §13 preamble for L1 to L3.
  - Add a qualifier to ledger L6 ("Negotiate selecting NTLM; Kerberos unsupported in 1.1.0") and to L1 (no certificate held by Pageant or the service).
  - Give exact wording for ledger §5 item 5.
  - Name the target sections in Releases.md and in gwz-py's Migration-1.1.0-DRAFT.md.

### F6 (P3): The authority and review text predates D41

- **Location.** Line 5 ("The ruling has no D-number yet: D34 to D38 … do not include it"), §6 item 1 (line 57), and §5 bullet 4 (line 53): "It names no review … the lane owner chooses the route (a skim … or the dual review)". Every new citation uses `GwzV110AmendmentS56Scope.md`, a path that does not exist, while D41 cites `-DRAFT`.
- **Violated text.** D41: "It takes its own Consistency and Safety review now".
- **Consequence.** The accepted text would contradict the decision that authorizes it. The impact is bounded, because the dual review is already running.
- **Correction.** Cite D41 at line 5. Close §6 item 1. Replace line 53 with "Reviewed Consistency and Safety under D41". Settle the accepted file name before applying the citations.

### F7 (P3): The counts in §2 are absolute but other lanes are changing the same table

- **Location.** Line 31: "`G` falls from 5 to 4 … 7 to 5 …; `U` rises …".
- **Violated text.** Cell table §2 is being changed in parallel:
  - gapsa moves 12.1b M and L to `E`.
  - gapsb moves 9.3 W and 18.4 W to PASS.
- **Consequence.** Whichever lane applies its change second will find these absolute figures wrong.
- **Correction.** State the change as deltas: −2 `G` and +2 `U` on W, −1 `G` and +1 `U` on M and on L, one row (18.19) leaving `G`, and 9.4 leaving `G` on all three platforms.

### F8 (P3): The notes do not say what 1.1.0 does when a user waits on a hardware key's touch

- **Location.** §3 item 1 (line 38) and §5 bullet 2 (line 51): "the user sees whatever the transport reports today".
- **Violated text.** In the code, the agent's sign request waits under `Control` (`agent_client.rs:79-130`, `agent_socket.rs:15-30`). So a touch slower than `--ssh-timeout` (9 s by default) ends the attempt on the stall clock. Releases.md:151 states exactly this for Pageant's confirmation wait (D5 b, cell 18.2).
- **Consequence.** Users of physical keys get no warning about the one behaviour that can be predicted from the code.
- **Correction.** Add a sentence to item 1: "the agent's wait for a touch counts against `--ssh-timeout`; raise it if the touch takes longer." For Kerberos (item 2), the code accepts a Kerberos mechanism when the system's security provider (SSPI) selects it (`native.rs:269`), so "has not been tested" is accurate as written.

### F9 (P3): Agreement with D38 and D39 is asserted but not tied to the exact-identity design's text

- **Location.** §1 (line 17) and §5 bullet 3 (line 52): "does not alter `exact_agent_identity` (D38)".
- **Violated text.**
  - EAI §3.5 (:388-389): "Real security-key and Pageant-held-certificate signatures remain unqualified … Exact selection does not widen that claim."
  - EAI §2.4 help, and its file-classification error text (K4, :338), admit certificate files with no Windows-agent caveat.
  - EAI step 4.2 runs C1 to C4 "against the OpenSSH pipe agent on dabeest". Read with §4.2 (:444), that means the owned `agent_fixture_pipe`, not the service.
- **Consequence.** With no cross-reference, a reader could take D41's removal of native-agent certificates to cover D38's Windows certificate rows. Or a later step 4.2 run against the real service would evidence a cell this amendment marks `U`, with no rule for putting it back.
- **Correction.**
  - Cite EAI §3.5 as the agreeing text.
  - State that C1 to C4 on the owned pipe fixture are outside D41.
  - Add to notes item 3: "selecting such a certificate with `--identity <cert>.pub` is equally unsupported".
  - Note that a step 4.2 row against the real service would be evidence for a later amendment that puts the cell back, as §5 bullet 1 already says.

## Attacks that failed

- **Exactness of removal.** The cells that become `U` are exactly 9.4 on M, L and W and 18.19 on W, which is what D41 names. Lines 17 and 52 keep out rows 9.1 to 9.3, 9.2 and 9.3 W, 18.4, Digest, D30, D38 and the evidenced Pageant rows. Nothing beyond D41 is removed.
- **No cell with evidence is removed.** gapsb's README says what it did not cover: "certificates held by the native agent, a hardware key". The real-service run (4 plain keys) and the pipe certificate rows do not touch 9.4 or 18.19. No archived run selects Kerberos (design :203).
- **Definition of "native agent".** The object defines it as the Windows OpenSSH Authentication Agent service at lines 15, 25 and 40. Read literally, it does not cover the owned pipe agent (gapsb), the Unix agents (9.3 M and L), or D38's exact certificate selection by blob (D39 OQ8). The remaining risk is only F9's missing cross-reference.
- **Superseded clauses quoted exactly.**
  - A2 :489's last sentence is quoted verbatim.
  - A2 line 138's row is quoted faithfully.
  - A1 :361 matches.
  - Line numbers cited in WPD (100, 104, 203, 273, 289, 324-326, 358) and WPP (632, 751) are correct at the object and at HEAD.
- **Count arithmetic against the table at `50f1b16a`.** 17 cells in 9 rows become 13 cells in 7 rows. `U` goes 4→5, 4→5 and 4→6. This is correct. F7 concerns only how the counts are written.
- **A2 §3.21.** It says nothing about security keys, Kerberos or certificates, so it does not conflict.
- **Kerberos wording against TD1 and the code.** "Answered by the logon session's default credentials, as gwz 1.0.17 does" holds. The code tracks a Kerberos selection and adds no refusal, which matches "adds no refusal" (line 36).
- **TD8 B.** The amendment upgrades "not qualified" to "unsupported" and keeps the executed facts (§6 item 3). This is consistent with TD8's recommendation that nothing is advertised without evidence.

## Next action

The drafter remediates F1 to F5 in one patch (one round):
- exact replacement rows for the A1 and A2 scope rows and the sign-off;
- the explicit 9.2 W hand-off, and correction of gapsb's pointer, through the lane owner;
- notes items 1 and 3 reworded by platform from the gapsb and `key_types.rs` facts;
- exact old and new text for Releases.md:154, the Windows guide, WPD §13 and §14 item 10, WPP OQ10 and 6.3, cell table §3.5, ledger §5 item 5 and rows L1 and L6, and gwz-py's migration notes.

The P3s F6 to F9 go in the same patch. Then both D41 reviewers re-review it against these counterexamples.

**Outside the object, recorded and not blocking:** `GwzTransportWindowsUserGuide-DRAFT.md:335` says an RSA certificate's failure message "says so". gapsb F4 shows the candidate prints only `ssh setup failed: Authentication`.
