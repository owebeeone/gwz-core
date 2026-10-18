# Consistency re-review, round 2: the S5.6 scope amendment (D41)

**Object:** `gwz-core/dev-docs/GwzV110AmendmentS56Scope-DRAFT.md`, revision 1, at gwz-core `53dca0265b1056cf019fc73460b06468ab240717` in lane `/Volumes/projects/limbo/gwz-dev-amend41` (410 lines). I read it together with `GwzV110AmendmentS56Scope-RemPlan.md`.

**Date:** 2026-10-18

**Axis:** Consistency, read-only. I modified nothing.

**Verdict:** **GO.** P0 0, P1 0, P2 0, P3 2. All nine round-1 findings are closed, and the two new P3s are bounded.

## §0 Evidence base

**Commits, checked at the start and again at the end; nothing changed:**
- gwz-core `53dca026`
- gwz-cli `6b4fd2e`
- gwz-py `7da569e`
- gwz-sspi `75c1b03`

The only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`, which is outside scope.

**What changed since round 1:**
- `53dca026` changes only the amendment, the RemPlan and the two round-1 review files.
- The drafter checked the old blocks at `c8097a8c`, which is the parent of `53dca026`. The target files are therefore the same as at HEAD.
- `7d52638c` (ambient-agent failure wording) touches `driver.rs`. I re-checked every `driver.rs` line the amendment cites at HEAD.

**Old-block check.** A scratch script (`scratchpad/chk.py`, `python3 -I`) pulled out each `Old` block of E1 to E27 (30 blocks, counting E5a/b and E12a/b/c) and counted it in its file at HEAD. **Every block occurs exactly once, at the line it cites.** That covers 30 of 30 blocks:
- E1 to E18: gwz-core design and plan documents.
- E19 to E21: the route ledger.
- E22 and E23: `gwz-cli/docs/Releases.md`.
- E24: `gwz-py/docs/Migration-1.1.0-DRAFT.md`.
- E25 to E27: the Windows user guide.

**Code claims I re-read at HEAD.** Each one matches the text that cites it.
- **Agent signing.**
  - `agent_keys.rs:24-32`: `TYPES` lists the security-key (sk) types with no platform condition.
  - `agent_auth.rs`: the `begin_wait`/`complete_wait` pairs at :89-91, :113-140 and :208-220, and the declined key going on to the next at :151-160.
  - `driver.rs:155` and `:478`: the stall message "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)".
  - `gwz-cli/src/lib.rs:211` and `:220`: the 9-second default.
- **Windows sign-in (SSPI).**
  - `gwz-sspi/src/worker/windows/conversation.rs:211-217`: the flags are `ISC_REQ_CONNECTION | ISC_REQ_ALLOCATE_MEMORY`, so no delegation and no mutual authentication.
  - `worker/session.rs:41-44, 52, 58` and `values.rs:86`: Negotiate allows Kerberos or NTLM.
  - `native.rs:263-271` and `:331-343`: a Kerberos selection is accepted, and an unlisted error kind becomes `Protocol`.
  - `exchange.rs:99`, `:165-168` and `:215-219`: fresh `History` facts, the `HTTP/<host>` target and the logon identity, and the early return.
  - `https_failure.rs:313-345`: the "began" test and the `Protocol` text at :343.
  - `https_failure.rs:785`: the bare "HTTPS endpoint request failed: Protocol".
- **Certificate rows in `key_types.rs`.**
  - `:100-135`: on Unix, seven certificate types authenticate, including RSA-CERT at flag 4 and the two sk-CERT types.
  - The Windows arm: the gapsb rows.
- **gwz-py.**
  - `client.py:422`: `configure_transport_timeout`.
  - `cli_shared.py:19-24` and Migration §3:52: `GWZ_TRANSPORT`.
- **Notes.** `Releases.md:135`: the paragraph "If something goes wrong after the upgrade".

**Correction to my round-1 report.** The drafter is right that my line references for amendment 2 were off by 15. OD11's list is at :484-485 and the rewritten §3.2 row is at :492. My report said 487-488 and 507. The edits use the correct lines.

## Closure table, F1 to F9

| Finding | Round 1 | Closed in | Status |
|---|---|---|---|
| F1: superseded V110 table edited, controlling scope rows left untouched | P2 | E2: A2 §3.2's Windows row; its unsupported column now names Kerberos and the Pageant- or service-held certificate. E7: A1 §3.2 as A2 §3.19 rewrites it; hardware key, and the Windows certificate. E4: A2 §3.10 sign-off. E5a/E5b: OD11's list. E3, E6, E8. E1 states that V110 §1 and §2 name none of the three, and points to the controlling rows | **Closed.** Every in-release home I named in round 1 is now edited with exact text |
| F2: Windows 9.2 had no owner, and notes item 1 claimed Windows coverage | P2 | New operator ruling: lane `win92`. §4 bullets 2 to 4: 9.3 W is evidenced and not `U`, 9.2 W is not removed, and gapsb's pointer is to be corrected. E15. Item 1 and E6 now say "on macOS and Linux", with Windows "pending" | **Closed.** One P3 residue, N2 |
| F3: certificate notes wrong by platform | P2 | §1 row 3 now cites `key_types.rs` and gapsb. Item 3 (E23) is split by platform: on Unix all listed types, RSA included, and DSA skipped; on Windows Ed25519 and ECDSA, with RSA refused by the test server's algorithm list. E12c, E25 | **Closed.** One P3 residue, N1 |
| F4: `Releases.md:154` and the Windows guide contradict the change | P2 | E22 narrows "certificates held by an agent" to Pageant and the Windows service and keeps PAC scripts under "not promised". E25 to E27 cover guide lines 335-338, 583 and 612-613 | **Closed** |
| F5: edits described rather than given; targets that did not exist | P2 | E1 to E27 now each give exact old and new text, and every old block was verified (§0). E11, E12 and E13 split the design preamble and §14 item 10. E19 and E20 qualify ledger rows L1 and L6. E21 adds ledger §5 item 6. E22 and E23 use the real places in Releases.md. E24 covers gwz-py. §6 item 5 drops the S7.2 help row, with the search that justifies dropping it | **Closed** |
| F6: authority and review text predated D41 | P3 | The preamble quotes D41. §5 bullet 4 records the dual review, at most two rounds. The status line settles the file name | **Closed** |
| F7: absolute counts in a shared table | P3 | §4 now gives deltas only (−4 `G`, +4 `U`) and recomputes the totals at apply time | **Closed** |
| F8: the touch wait was not stated | P3 | Item 1 says the touch counts against `--ssh-timeout`, gives the message and the fix, and adds the declined-key fallback. §5 bullet 2 gives the texts, which match the code | **Closed** |
| F9: no tie to D38 and D39 | P3 | §5 bullet 3 cites EAI §3.5 and places C1 to C4 on the owned pipe fixture, outside D41. Item 3's last sentence covers `--identity <cert>.pub`. §5 bullet 1 and §6 item 4 add the rule for putting a cell back after a run against the real service | **Closed** |

## New findings (round 2)

### N1 (P3): Item 3 lists security-key certificates as authenticating, with no qualifier

- **Root cause.** Item 3 is worded from the type list of the Unix row, which was produced by a software authenticator.
- **Location.** §3 item 3 (line 375), repeated word for word in E23's third bullet (line 310): "On macOS and Linux, certificates of Ed25519, ECDSA (P-256, P-384, P-521), RSA and the two security-key types authenticate from an SSH agent".
- **Controlling text.** In `key_types.rs:109-125`, the sk-CERT rows sign through `Sign::Sk` with an emulated key. Item 1 and D41 make a real hardware-key signature unsupported.
- **Consequence.** A reader can take item 3 to mean that a certificate on a physical security key works, which item 1 contradicts. The effect is bounded, because item 1 says plainly that hardware keys are unsupported.
- **Correction.** Change the phrase to "and the two security-key types (through the software authenticator of item 1)", in both places.

### N2 (P3): The 9.2 W mark and its authority do not match what §4 says

- **Root cause.** E15 changes only the note column of row 9.2. The win92 ruling is cited as "operator, 2026-10-18", with no D-number.
- **Location.**
  - E15 (line 226): the W mark stays `G` and the note becomes "Owner: lane `win92`".
  - §4 bullet 3 (line 389) says the cell "is pending on it".
  - `GwzTransportTR18-OperatorDecisions.md` at `53dca026` has no `win92` entry.
- **Controlling text.**
  - The cell table §1 legend: `P` means "waits on the named running step", and `G` means "neither evidenced nor ruled".
  - The table's §2 "By pending step" table and its §8 list every running lane.
  - Round-1 F6 established that rulings are cited by their D-number.
- **Consequence.** The table and the amendment describe the same cell in two different states, and the lane `win92` does not appear in the pending-step lists. This does not change scope, because 9.2 W blocks S5.6 under either mark.
- **Correction.** Do one of the following:
  - Make E15 change the W mark to `P: lane win92` and add `win92` to the table's §2 and §8. Ask the lane owner to file the 9.2 ruling and cite its D-number.
  - Or reword §4 bullet 3 to say "stays `G` with an owner".

**No new architectural root cause.** The revision keeps D41's three removals as its only scope change. E15 records an owner; it removes nothing. The 30 edits all trace either to the three removed claims or to texts that stated them.

## Attacks that failed

- **Exactness.** `U` goes only to 9.4 on macOS, Linux and Windows, and to 18.19 on Windows. Line 35 states that on macOS and Linux, 9.4 is removed for the hardware key alone, and Unix agent certificates stay in, under 9.3.
- **Precise scope.**
  - Every certificate edit (E2, E5b, E7, E12c, E16, E19, E22, E25, E27, and item 3) names Pageant or the Windows OpenSSH Authentication Agent service. None of them reaches the owned pipe agent (gapsb) or Unix agents.
  - I checked whether Pageant serving a named pipe could be "an agent on a pipe other than the OpenSSH service". When a Pageant window is running, gwz chooses the window as its source (EAI §4.2), so the two statements cannot both apply to the same agent.
- **Cells that now have evidence.** gapsb still says certificates in the native agent and hardware keys were "not covered". 9.3 W is explicitly not `U` (§4 bullet 2).
- **D38 and D39.**
  - EAI §3.5's "unqualified" reads as "unsupported", and the amendment correctly makes no edit to the EAI design.
  - EAI's K4 rule refuses a DSA certificate at admission, while the ambient path skips one (item 3). Both are true, on different paths.
- **Notes against the code.** Item 1 (the stall wait, the message and the declined-key skip), item 2 (the flags, the accepted mechanism and both failure texts, which §5 marks as code readings that were not reproduced) and item 3 (the type list and the reason RSA is refused) all match the code I read.
- **Grammar of the split.** After E13, item 10 reads correctly ("… No claim is made for a Mac trusted-HTTPS result (L5), a helper-identity exchange …").
- **gwz-py (E24).** `configure_transport_timeout` exists, and so does the `GWZ_TRANSPORT=native` route. Migration §3 is the right reference.

**Cosmetic, outside any finding.** The sub-items of §5 bullet 2 (lines 397-400) are written at the same list level as their parent, so they do not render nested.

## Next action

GO on the Consistency axis. The lane owner may fold N1 and N2 into the text when applying it, with no re-review needed. Applying section 2 also waits for the Safety reviewer's re-verdict, as D41 requires.
