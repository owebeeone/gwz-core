# GWZ 1.1.0 amendment: three claims leave the S5.6 column and are named unsupported in the 1.1.0 notes

Date: 2026-10-18; revisions 1 and 2 the same day. Status: **DRAFT revision 2, not accepted.** It changes no code and authorizes no run, commit, tag or publish. It takes effect only when the lane owner accepts it and applies the edits of section 2. **The file name:** this file stays `GwzV110AmendmentS56Scope-DRAFT.md` until acceptance and is then renamed `GwzV110AmendmentS56Scope.md` (the DRAFT strike, as the route ledger's finalization does). Every citation in the replacement text below already uses the final name, so the edits are applied once.

Authority: **D41** in `GwzTransportTR18-OperatorDecisions.md` (line 84; operator, 2026-10-18): "Remove from 1.1.0's column, named unsupported in the 1.1.0 notes: the manual hardware security-key row (9.4), Kerberos (18.19), and Pageant- or native-agent-held certificates (L3). Text: `GwzV110AmendmentS56Scope-DRAFT.md`. It takes its own Consistency and Safety review now, not folded into S7.5." D41 answers question 3.5 of `GwzV110S56CellTable.md`. Revision 1 answered round 1 of that review (revision 2, below, answers round 2): `GwzV110AmendmentS56Scope-ReviewConsistency.md` (NO-GO: P2 5, P3 4), `GwzV110AmendmentS56Scope-ReviewSafety.md` (NO-GO: P2 3, P3 3) and `GwzV110AmendmentS56Scope-RemPlan.md`. The route is D41's dual review, at most two rounds; no skim. **D48 and D49 are cited from main** (gwz-core `24fe01f1`, `GwzTransportTR18-OperatorDecisions.md` lines 91 and 92); this lane's copy of that file stops at D44.

**A second ruling, new since the reviews (operator, 2026-10-18; D49):** Windows row 9.2 (security keys through a software authenticator, and DSA, from a pipe agent on Windows) gets ported rows in lane `win92`. This amendment therefore does **not** remove 9.2 on Windows. It becomes evidenced, or stays pending on that lane (section 4).

## Revision 1: where each finding is closed

| Finding | Closed in |
|---|---|
| Consistency F1: the superseded V110 table was edited and the controlling "in this release" rows were left | Section 2: E2 (A2's Windows scope row), E3 (A2's `Release` table row for amendment 1's §4), E4 (A2's sign-off), E5 (OD11's list), E6 (A2's tests sentence), E7 (amendment 1's §3.2 row as A2 rewrote it), E8 (amendment 1's §4 bullet). E1 turns V110 §2 into a cross-reference and states that V110 §1 and §2 name none of the three and need no row change |
| Consistency F2 and Safety S3 (first part): Windows 9.2 had no owner, and notes item 1 claimed coverage on Windows | Preamble (the ruling); section 3 item 1; section 4; E15 (row 9.2); section 6 item 2 |
| Consistency F3 and Safety S3 (second part): certificate behaviour wrong by platform, and row 3 cited the 1.0.17 baseline alone | Section 1 row 3 (re-cited to `key_types.rs` and run `2026-10-18-gapsb`); section 3 item 3; E12c; E25 |
| Consistency F4 and Safety S5: `Releases.md:154` and the Windows guide contradict the change | E22 (Releases.md line 154), E23 (Known limits), E25, E26, E27 (guide lines 335 to 338, 583 and 612 to 613) |
| Consistency F5: edits described, not given as text, and two targets did not exist as named | Section 2: every edit E1 to E29 gives exact old and new text. Releases.md has no "Not supported" list; E22 and E23 use its real places. WPD: E11 (§13 preamble), E12 (L1 to L3), E13 (§14 item 10, split). WPP: E9, E10. Cell table: E14 to E18. Ledger: E19 to E21. gwz-py: E24. The "S7.2 help" row is dropped: no help text names the three (section 6 item 5) |
| Safety S1: "Kerberos is not supported" misleads | Section 3 item 2; E20 (ledger L6); E26 |
| Safety S2: Kerberos failures fail closed with misleading or bare words | Section 5 bullet 2 (the actual texts, with their places in the code); section 3 item 2; section 6 item 3 (the wording fix is a separate finding) |
| Consistency F6 and Safety S6: authority and review text predate D41 | Preamble; section 5 bullet 4; the old open item 1 (a D-number) is closed by D41 |
| Consistency F7: absolute counts | Section 4 (deltas only) |
| Consistency F8 and Safety S4: the touch wait, and the declined-key fallback | Section 3 item 1 |
| Consistency F9: tie to D38 and D39 | Section 5 bullet 3; section 3 item 3 (last sentence); section 6 item 4 |

## Revision 2: where each round-2 finding is closed

Round 2: Consistency GO (`GwzV110AmendmentS56Scope-ReviewConsistency-2.md`, P3 N1 and N2); Safety NO-GO on one non-architectural P2 (`GwzV110AmendmentS56Scope-ReviewSafety-2.md`, N1, with P3 N2 and N3). This revision changes wording only, plus the cell-table edits E15, E28 and E29.

| Finding | Closed in |
|---|---|
| Safety N1 (P2): item 2 and E23 fixed in place a message that D48 replaces, and called every post-token failure the bare `Protocol` text | Section 3 item 2 and E23: the message sentence is rewritten as the reviewer suggested, with D48 marked *pending its landing*; E24 points at it. Section 5 bullet 2 and section 6 item 3 cite D48 as pending lane `words2`, and state the 401 rejection words (`https_failure.rs:237-249`) and the redirect words (`:319-325`) |
| Safety N2 (P3): "two ways" under-counted the first-step failures | Section 5 bullet 2: "at least four ways", adding `session.rs:29` and `conversation.rs:137-139` |
| Safety N3 (P3) and Consistency N1 (P3): item 3 read as covering every platform, and listed security-key certificates without a qualifier | Section 3 item 3 and E23: the RSA sentence starts "On Windows" and gives the libssh2 cause; the security-key types read "(through the software authenticator of item 1)" |
| Consistency N2 (P3): the 9.2 mark and its authority did not match section 4 | E15: the Windows mark becomes `P: lane win92` and cites D49; new E28 and E29 add `win92` to the cell table's "By pending step" list and to its section 8 list; section 4 bullet 3 says the same, with the delta; the preamble and section 6 item 2 cite D49 |
| Cosmetic: the sub-items of section 5 bullet 2 did not nest | Section 5 bullet 2: sub-items indented one level |

## 1. What this amendment changes

Three claims stop being 1.1.0 claims. Each is removed from the S5.6 sign-off (the cell it names) and from every controlling text that put it "in this release" (section 2), and each is named in the 1.1.0 notes as unsupported (section 3).

| # | Claim removed | Cell table row and platforms | Where the claim came from | Evidence that exists, and why it is not enough |
|---|---|---|---|---|
| 1 | **A real hardware security-key signature**: a physical `sk-ssh-ed25519@openssh.com` or `sk-ecdsa-sha2-nistp256@openssh.com` key signing through the transport, including the wait for its touch, as the manual row would run it | 9.4, first half, on macOS, Linux and Windows (design limitation L2) | `GwzTransportReleasePlanAmendment.md` §4 (line 361: "the security-key case, by a named manual row that S7.2 runs, with a hardware key"; "needs the operator's go"); `GwzTransportReleasePlanAmendment-2.md` §3.19 (line 489) and its §3.3 table (line 138) | None. No hardware key was ever run. On macOS and Linux a software authenticator answers the agent's sign request (`key_types.rs::security_keys_authenticate_through_the_software_authenticator`, a `cfg(unix)` row). On Windows there is no security-key row at all (the arm says they "stay with the Unix rows above"): that is 9.2, owned by `win92` |
| 2 | **Kerberos** as a mechanism of Windows HTTPS `Negotiate` | 18.19, Windows (L1) | `GwzTransportWindowsParityPlan.md` OQ10 (line 751) and step 6.3 (line 632); OQ10 (a) recommended removing it from the "in this release" column "by amendment" | None. Every successful `Negotiate` exchange in the record selected NTLM (`GwzTransportWindowsParityDesign.md` §7, line 203); dabeest has no domain. The code does **not** exclude Kerberos (section 3 item 2) |
| 3 | **A certificate held by Pageant, or by the Windows OpenSSH Authentication Agent service**, and naming such a certificate with `--identity <cert>.pub` | 9.4, second half, Windows only (L3) | `GwzTransportWindowsParityDesign.md` rows B08 (line 273) and X4 (line 289) | None for those two agents. The owned pipe agent is a different case and is evidenced (run `2026-10-18-gapsb`, `key_types.rs` Windows arm): Ed25519 and ECDSA P-256, P-384 and P-521 certificates authenticate from it, and an RSA certificate does not. Certificates held by a Unix SSH agent authenticate for every listed type (`key_types.rs::certificates_of_each_type_authenticate_against_a_trusted_user_ca`, cell 9.3 on macOS and Linux) |

Claim 3 is Windows-only (Pageant and the Windows service). On macOS and Linux row 9.4 is therefore removed for the hardware key alone; certificates held by a Unix agent are evidenced by 9.3 and stay in.

Nothing else changes. This amendment does not touch: rows 9.1 and 9.3; **9.2 on Windows (owned by `win92`)**; the real OpenSSH service for plain keys (18.4); exact agent-key selection and its capability answer (D38, D39, D30); Digest (D3, TD2); the evidenced Pageant rows. The exact-identity design's rows C1 to C4 on the owned pipe fixture are outside D41 (section 5).

## 2. The edits

Each edit gives the old text exactly as the file has it, with its line, and the new text. The old text of every edit was checked against its file at gwz-core `53dca026` (its target files are those of `c8097a8c`), gwz-cli `6b4fd2e` and gwz-py `7da569e`; E15, E28 and E29 were added or rewritten in revision 2 and checked the same way: each `Old` block occurs exactly once in its file, at the cited line (checked by a script that is not part of the repository). The name `GwzV110AmendmentS56Scope.md` is the accepted name (see the status line). Nothing is applied by this draft.

Two groups of line numbers in the Consistency review are off by 15 against the files at HEAD: amendment 2's OD11 list is lines 484 and 485 (cited as 487 and 488), and its rewritten §3.2 row is line 492 (cited as 507). The edits use the lines found.

#### E1. `GwzV110Plan.md`, after line 53: a cross-reference, not the home of the exclusion

File `gwz-core/dev-docs/GwzV110Plan.md`. Old (line 53):

> | New crates for this libgit2, the forked git2 bindings, `gwz-transport`, and `gwz-git` | Publishing those forks as `libgit2-sys` or `git2` |

New:

> | New crates for this libgit2, the forked git2 bindings, `gwz-transport`, and `gwz-git` | Publishing those forks as `libgit2-sys` or `git2` |
>
> Not in this table, and not in §1: a real hardware security-key signature, Kerberos in Windows `Negotiate`, and a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service. Neither §1 nor this table names them, so neither needs a row change. The rows that put them in the release are `GwzTransportReleasePlanAmendment-2.md` §3.2, §3.10 and §3.19, which `GwzV110AmendmentS56Scope.md` (D41) amends: it removes the three from S5.6's column and names them unsupported in the 1.1.0 notes.

#### E2. `GwzTransportReleasePlanAmendment-2.md` §3.2, the Windows scope row (line 104)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 104):

> and libgit2's SSH home order (OD15, TR1.8, TR4.8–TR4.10) | — |

New:

> and libgit2's SSH home order (OD15, TR1.8, TR4.8–TR4.10) | Kerberos selected inside Negotiate; a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service (`GwzV110AmendmentS56Scope.md`, D41) |

#### E3. `GwzTransportReleasePlanAmendment-2.md` §3.3, the table row for amendment 1's §4 (line 138)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 138):

> | §4: Phase 2's tests, S5.6's cell, S7.2's notes, and the manual security-key row, run on the transport (§3.19) | 1.1.0 |

New:

> | §4: Phase 2's tests, S5.6's cell and S7.2's notes run on the transport (§3.19); the manual security-key row does not run in 1.1.0 and the real-key claim is unsupported (`GwzV110AmendmentS56Scope.md`, D41) | 1.1.0 |

#### E4. `GwzTransportReleasePlanAmendment-2.md` §3.10, the sign-off (line 269)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 269):

> covers TR2.8's agent key types and signature algorithms, TR1.6's credential helpers, and a cell for each behaviour TR1.8 designs. The server and reuse cells are 1.2.0's.

New:

> covers TR2.8's agent key types and signature algorithms, TR1.6's credential helpers, and a cell for each behaviour TR1.8 designs, except the three claims `GwzV110AmendmentS56Scope.md` (D41) removes, which are marked unsupported: a real hardware security-key signature; Kerberos in Windows `Negotiate`; a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service. The server and reuse cells are 1.2.0's.

#### E5a. `GwzTransportReleasePlanAmendment-2.md` §3.19, OD11's list: security keys (line 484)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 484):

>     - security keys (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`), whose touch the agent asks for;

New:

>     - security keys (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`), whose touch the agent asks for (a real hardware key's signature is unsupported in 1.1.0: `GwzV110AmendmentS56Scope.md`, D41);

#### E5b. the same list: certificates (line 485)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 485):

>     - OpenSSH certificates of those types;

New:

>     - OpenSSH certificates of those types (a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service is unsupported in 1.1.0: `GwzV110AmendmentS56Scope.md`, D41);

#### E6. `GwzTransportReleasePlanAmendment-2.md` §3.19, the tests bullet's last sentence (line 489)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 489):

> Amendment 1's §4 manual security-key row runs on the transport, with a hardware key, and needs the operator's go.

New:

> Amendment 1's §4 manual security-key row, which would run on the transport with a hardware key, is not run in 1.1.0 (`GwzV110AmendmentS56Scope.md`, D41); TR2.8's security-key rows run through a software authenticator, on macOS and Linux.

#### E7. `GwzTransportReleasePlanAmendment-2.md` §3.19, amendment 1's §3.2 row as rewritten (line 492)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md`. Old (line 492):

>     - §3.2's row reads, in this release, "SSH through an agent with any key type 1.0.17 uses (TR2.8)", and as unsupported "—";

New:

>     - §3.2's row reads, in this release, "SSH through an agent with any key type 1.0.17 uses (TR2.8)", and as unsupported "a real hardware security-key signature; on Windows, a certificate held by Pageant or by the Windows OpenSSH Authentication Agent service (GwzV110AmendmentS56Scope.md, D41)";

#### E8. `GwzTransportReleasePlanAmendment.md` §4, the security-key bullet (line 361)

File `gwz-core/dev-docs/GwzTransportReleasePlanAmendment.md`. Old (line 361):

>   - the security-key case, by a named manual row that S7.2 runs, with a hardware key, before its notes are written. A manual row against a live account needs the operator's go (the plan's §2).

New:

>   - the security-key case, by a named manual row that S7.2 runs, with a hardware key, before its notes are written. A manual row against a live account needs the operator's go (the plan's §2). *Removed from 1.1.0 by `GwzV110AmendmentS56Scope.md` (D41): the row is not run in 1.1.0.*

#### E9. `GwzTransportWindowsParityPlan.md` step 6.3 (line 632)

File `gwz-core/dev-docs/GwzTransportWindowsParityPlan.md`. Old (line 632):

> OQ10 names the likely ones (Digest, Kerberos).

New:

> OQ10 named the likely ones: Digest, which is refused (D3, TD2) and has evidence, and Kerberos, which `GwzV110AmendmentS56Scope.md` (D41) removes from the "in this release" column and marks unsupported.

#### E10. `GwzTransportWindowsParityPlan.md` OQ10, a status bullet after line 755

File `gwz-core/dev-docs/GwzTransportWindowsParityPlan.md`. Old (line 755):

> - **Recommend (a)**, decided only after step 2.3 has tried the 1.0.17 Digest row with an accepted identity if OQ9 allows it. Find out: B14 with a disposable local account.

New:

> - **Recommend (a)**, decided only after step 2.3 has tried the 1.0.17 Digest row with an accepted identity if OQ9 allows it. Find out: B14 with a disposable local account.
> - **Kerberos, decided (operator, 2026-10-18, D41):** removed from the "in this release" column by `GwzV110AmendmentS56Scope.md` and named unsupported in the 1.1.0 notes. It is not marked "works through the OS provider": the notes say that Windows may choose Kerberos inside Negotiate, that gwz does not prevent it, and that no claim is made. Digest stays as D3 and TD2 decided.

#### E11. `GwzTransportWindowsParityDesign.md` §13, the preamble (line 320)

File `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`. Old (line 320):

> A separate list from the register. Each is a combination or claim that this design does not qualify; none is declared unsupported unless it says so.

New:

> A separate list from the register. Each is a combination or claim that this design does not qualify; none is declared unsupported unless it says so. **L1, L2 and L3 say so for 1.1.0** (`GwzV110AmendmentS56Scope.md`, D41).

#### E12a. `GwzTransportWindowsParityDesign.md` §13, row L1 (line 324)

File `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`. Old (line 324):

> the OS provider may work when domain-joined, unverified here | 7 |

New:

> the OS provider may work when domain-joined, unverified here; **unsupported in 1.1.0** (D41, `GwzV110AmendmentS56Scope.md`): Windows may choose Kerberos inside Negotiate and gwz does not prevent it, but no claim is made | 7 |

#### E12b. row L2 (line 325)

File `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`. Old (line 325):

> no hardware; the emulated declining-identity skip is executed | 4 |

New:

> no hardware; the emulated declining-identity skip is executed; **unsupported in 1.1.0** (D41, `GwzV110AmendmentS56Scope.md`) | 4 |

#### E12c. row L3 (line 326)

File `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`. Old (line 326):

> no certificate was loaded into the native service | 4 |

New:

> no certificate was loaded into the native service; **unsupported in 1.1.0** (D41, `GwzV110AmendmentS56Scope.md`) for a certificate held by Pageant or by the native service. A certificate from the owned pipe agent is a different case: Ed25519 and ECDSA certificates authenticate there and an RSA certificate does not (run `2026-10-18-gapsb`) | 4 |

#### E13. `GwzTransportWindowsParityDesign.md` §14 item 10, split (line 358)

File `gwz-core/dev-docs/GwzTransportWindowsParityDesign.md`. Old (line 358):

> 10. No claim is made for Kerberos (L1), a real security key (L2), a Pageant-held or native-agent certificate (L3), a Mac trusted-HTTPS result (L5),

New:

> 10. Unsupported in 1.1.0, not only unqualified (D41, `GwzV110AmendmentS56Scope.md`): Kerberos (L1), a real security key (L2), and a Pageant-held or native-agent certificate (L3). No claim is made for a Mac trusted-HTTPS result (L5),

#### E14. `GwzV110S56CellTable.md` §3, question 3.5 (line 68)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 68):

> Options: run the hardware row; or one short amendment removing these three from the 1.1.0 column.

New:

> Options: run the hardware row; or one short amendment removing these three from the 1.1.0 column. **Ruled 2026-10-18 (D41): the amendment, `GwzV110AmendmentS56Scope.md`.**

#### E15. `GwzV110S56CellTable.md`, row 9.2, its Windows cell and note column (line 192)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 192):

> | G: the Windows arm of `key_types.rs` says security keys, certificates and DSA "stay with the Unix rows"; only the type-listing logic (`agent_keys.rs`) runs on Windows, no end-to-end authentication. P38 expected Windows twins "when 3.4 lands"; none found | Blocker 3.4 |

New:

> | P: lane `win92` (D49). The Windows arm of `key_types.rs` says security keys, certificates and DSA "stay with the Unix rows"; only the type-listing logic (`agent_keys.rs`) runs on Windows, no end-to-end authentication. P38 expected Windows twins "when 3.4 lands"; none found | Owner: lane `win92` (D49, 2026-10-18: Windows twins of the security-key and DSA rows through the pipe-agent fixture, with a 1.0.17 comparison where 1.0.17 can express the case). D41 does not remove this cell |

#### E16. `GwzV110S56CellTable.md`, row 9.4 (line 194)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 194):

> | 9.4 | Manual hardware security-key row (amendment 1 section 4, "needs the operator's go"), and a Pageant-held or native-agent certificate (L2, L3) | G: no hardware run; TD8 B left it "not qualified" | G: same | G: same | Blocker 3.5 |

New:

> | 9.4 | Manual hardware security-key row (amendment 1 section 4, "needs the operator's go"), and a Pageant-held or native-agent certificate (L2, L3) | U: D41, `GwzV110AmendmentS56Scope.md` (a real hardware-key signature) | U: same | U: D41, `GwzV110AmendmentS56Scope.md` (a real hardware-key signature, and a certificate held by Pageant or by the Windows OpenSSH agent service) | Facts kept: TD8 B, L2, L3. Certificates from a Unix agent are row 9.3; from the owned pipe agent on Windows, row 9.3 |

#### E17. `GwzV110S56CellTable.md`, row 18.19 (line 275)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 275):

> | 18.19 | Kerberos | N | N | G: L1, "no claim is made" (design section 14 item 10); OQ10 (a) wanted it removed from the "in this release" column by amendment and no filed amendment text names it | Blocker 3.5 |

New:

> | 18.19 | Kerberos | N | N | U: D41, `GwzV110AmendmentS56Scope.md` (L1): Windows may choose Kerberos inside Negotiate and gwz does not prevent it; no claim is made | Facts kept: L1 |

#### E18. `GwzV110S56CellTable.md`, §7 register (line 288)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 288):

> | Kerberos; hardware security key; Pageant-held or native-agent certificate | 18.19, 9.4 | `G`: not qualified, never declared unsupported (design section 13: "none is declared unsupported unless it says so") | TD8 B, L1 to L3 |

New:

> | Kerberos; a real hardware security-key signature; a Pageant-held or native-agent certificate | 18.19, 9.4 | `U` | D41, `GwzV110AmendmentS56Scope.md`; the facts: TD8 B, L1 to L3 |

#### E19. `GwzV110RouteLedger-DRAFT.md`, row L1 (line 40)

File `gwz-core/dev-docs/GwzV110RouteLedger-DRAFT.md`. Old (line 40):

> then the default OpenSSH agent pipe on Windows) | transport |

New:

> then the default OpenSSH agent pipe on Windows). Unsupported in 1.1.0 (`GwzV110AmendmentS56Scope.md`): a physical hardware security-key signature, and on Windows a certificate held by Pageant or by the OpenSSH agent service | transport |

#### E20. `GwzV110RouteLedger-DRAFT.md`, row L6 (line 45)

File `gwz-core/dev-docs/GwzV110RouteLedger-DRAFT.md`. Old (line 45):

> Negotiate or NTLM, answered by the Windows logon session | transport,

New:

> Negotiate or NTLM, answered by the Windows logon session. Tested only where Negotiate selects NTLM: Windows may select Kerberos on a domain-joined host and gwz does not prevent it, but Kerberos is unsupported in 1.1.0, no claim (`GwzV110AmendmentS56Scope.md`) | transport,

#### E21. `GwzV110RouteLedger-DRAFT.md` §5, a sixth item after line 97

File `gwz-core/dev-docs/GwzV110RouteLedger-DRAFT.md`. Old (line 97):

> 5. Unsupported: Linux ARM64, macOS x86-64, Bazel's `@gwz_core_crates` (D27) until the Phase 8 registry pins.

New:

> 5. Unsupported: Linux ARM64, macOS x86-64, Bazel's `@gwz_core_crates` (D27) until the Phase 8 registry pins.
> 6. Unsupported in 1.1.0 (D41): a physical hardware security-key signature; Kerberos inside Windows `Negotiate`; a certificate held by Pageant or by the Windows OpenSSH agent service. The wording is section 3 of `GwzV110AmendmentS56Scope.md`; `Releases.md` (Known limits) and gwz-py's `Migration-1.1.0-DRAFT.md` (section 5) carry it.

#### E22. `gwz-cli/docs/Releases.md`, the Windows paragraph (line 154)

File `gwz-cli/docs/Releases.md`. Old (line 154):

> Not promised: Kerberos, hardware security keys, certificates held by an agent, PAC scripts.

New:

> Not promised: PAC scripts. Not supported in 1.1.0: hardware security keys, Kerberos, and certificates held by Pageant or by the Windows OpenSSH agent service (see *Known limits of this release*).

#### E23. `gwz-cli/docs/Releases.md`, *Known limits of this release*, three bullets after line 169

File `gwz-cli/docs/Releases.md`. Old (line 169):

> - **A Windows SSH server that refuses a signature it accepted the key for** (`PUBLICKEY_UNVERIFIED`, about 1 in 6,000 connections against Windows' `sshd.exe`) is not retried.

New:

> - **A Windows SSH server that refuses a signature it accepted the key for** (`PUBLICKEY_UNVERIFIED`, about 1 in 6,000 connections against Windows' `sshd.exe`) is not retried.
> - **Hardware security keys are not supported.** gwz offers an SSH agent's `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` keys, and on macOS and Linux they authenticate in gwz's tests through a software authenticator that answers the agent's signature request. Signing with a physical key, including the wait for its touch, has not been tried. gwz waits for the agent's answer under `--ssh-timeout` (9 seconds by default), so a touch that takes longer ends the attempt with a message that names the option; raise `--ssh-timeout`. A key that the agent declines to sign, such as a security key that is not plugged in, is skipped and the next key in the agent is tried; to offer only one key, give its public-key file to `--identity` (exact agent-key selection, decisions D38 and D39, *pending its landing*). On Windows no security-key row has run yet (*pending the Windows security-key rows*).
> - **Kerberos is not supported (Windows).** A server that offers `Negotiate` is answered with the logon session's default credentials, as in 1.0.17. On a domain-joined host Windows may choose Kerberos inside `Negotiate`; gwz does not prevent that, asks for neither delegation nor mutual authentication, and has tested the exchange only where NTLM was chosen, so no claim is made for Kerberos. If a `Negotiate` sign-in that worked with 1.0.17 fails, run the command with `--transport native`, which is 1.0.17's behaviour for that command. A sign-in that Windows refuses is reported with its own message naming Kerberos and NTLM policy and `--transport native` (decision D48, *pending its landing*). An exchange that ends without a verdict after a token was sent is reported as "HTTPS endpoint request failed: Protocol", which does not name Kerberos.
> - **Certificates held by Pageant or by the Windows OpenSSH agent service are not supported.** On macOS and Linux, certificates of Ed25519, ECDSA (P-256, P-384, P-521), RSA and the two security-key types (through the software authenticator of item 1) authenticate from an SSH agent against a server that trusts the signing CA; a DSA certificate is skipped. On Windows, Ed25519 (as with 1.0.17) and ECDSA (P-256, P-384, P-521; no 1.0.17 baseline) certificates authenticate from an agent on a pipe other than the OpenSSH service. On Windows, an RSA certificate did not authenticate in the tests, as with 1.0.17: libssh2 on Windows offers it only under the algorithm `ssh-rsa-cert-v01@openssh.com`, which the test servers' default list refuses, and a server that accepts it was not tried. A certificate held in Pageant or in the OpenSSH Authentication Agent service has not been tried, and naming one with `--identity <certificate>.pub` is equally unsupported.

#### E24. `gwz-py/docs/Migration-1.1.0-DRAFT.md` §5, a bullet after line 82

File `gwz-py/docs/Migration-1.1.0-DRAFT.md`. Old (line 82):

> - **HTTPS authentication.** HTTPS is authenticated through the user's configured git credential helpers (`gh` among them), asked in their configured order; an `https://` URL that carries a password, a query or a fragment is refused before anything is sent. On Windows a server that offers Negotiate or NTLM is answered with the logon session. Credential material does not appear in Python-visible errors.

New:

> - **HTTPS authentication.** HTTPS is authenticated through the user's configured git credential helpers (`gh` among them), asked in their configured order; an `https://` URL that carries a password, a query or a fragment is refused before anything is sent. On Windows a server that offers Negotiate or NTLM is answered with the logon session. Credential material does not appear in Python-visible errors.
> - **Unsupported in 1.1.0: hardware security keys, Kerberos, and certificates held by Pageant or by the Windows OpenSSH agent service.** They are unsupported on the same terms as in the Rust command, whose 1.1.0 entry (*Known limits of this release*) has the details and the messages (the Kerberos sign-in words follow decision D48, *pending its landing*). In Python, `GWZ_TRANSPORT=native` selects 1.0.17's route when a sign-in that worked with 1.0.17 fails, and `await client.configure_transport_timeout(seconds)` raises the clock that a hardware key's touch is waited under (9 seconds by default; section 3).

#### E25. `GwzTransportWindowsUserGuide-DRAFT.md`, the agent-keys paragraph (lines 335 to 338)

File `gwz-core/dev-docs/GwzTransportWindowsUserGuide-DRAFT.md`. Old (lines 335 to 338):

> An RSA **certificate** held by an agent does not authenticate; the
> message says so. Not tested: a hardware security key, and certificates held by
> Pageant or by the OpenSSH agent itself (an ed25519 certificate served by a test
> agent on a pipe did authenticate).

New:

> Certificates from an agent on a pipe other than the OpenSSH service: Ed25519 and
> ECDSA (P-256, P-384, P-521) certificates authenticated in the tests; an RSA
> certificate did not, as with 1.0.17 (the test server's default algorithm list
> refuses `ssh-rsa-cert-v01@openssh.com`; a server that accepts it was not tried).
> Unsupported in 1.1.0: a hardware security key, and a certificate held by Pageant
> or by the OpenSSH agent itself; naming one with `--identity <certificate>.pub` is
> equally unsupported.

#### E26. `GwzTransportWindowsUserGuide-DRAFT.md`, the Kerberos sentence (line 583)

File `gwz-core/dev-docs/GwzTransportWindowsUserGuide-DRAFT.md`. Old (line 583):

> Kerberos is not tested; the Windows sign-in tests used NTLM. A server that

New:

> Kerberos is unsupported in 1.1.0: the Windows sign-in tests used NTLM. On a
> domain-joined computer Windows may choose Kerberos inside Negotiate; GWZ does not
> prevent that, asks for no delegation and no mutual authentication, and makes no
> claim for it. If a Negotiate sign-in that worked with 1.0.17 fails, run with
> `--transport native`. A server that

#### E27. `GwzTransportWindowsUserGuide-DRAFT.md`, the not-promised list (lines 612 and 613)

File `gwz-core/dev-docs/GwzTransportWindowsUserGuide-DRAFT.md`. Old (lines 612 to 613):

> **Not promised:** Kerberos; a hardware security key; certificates held in Pageant
> or in the OpenSSH agent; Pageant and the OpenSSH agent together;

New:

> **Not supported in 1.1.0:** Kerberos; a hardware security key; certificates held in
> Pageant or in the OpenSSH agent. **Not promised:** Pageant and the OpenSSH agent
> together;

#### E28. `GwzV110S56CellTable.md` §2, the "By pending step" table: the lane that now owns 9.2 on Windows (line 46)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 46):

> | D30 (lane `cap72`) | W: 9.7 | |

New:

> | D30 (lane `cap72`) | W: 9.7 | |
> | D49 (lane `win92`) | W: 9.2 | |

#### E29. `GwzV110S56CellTable.md` §8, the pending-by-step list (line 296)

File `gwz-core/dev-docs/GwzV110S56CellTable.md`. Old (line 296):

> | D30 (lane `cap72`) | 9.7 Windows (the exact-selection row) |

New:

> | D30 (lane `cap72`) | 9.7 Windows (the exact-selection row) |
> | D49 (lane `win92`, Windows security-key and DSA rows) | 9.2 Windows |

## 3. The text of the 1.1.0 notes

"Unsupported" has V110 §2's meaning, "recorded as unsupported in 1.1.0": **no claim is made, it is not advertised, and no evidence stands behind it.** It does not say that the transport refuses the case, and this amendment adds no refusal. Each item below is the text of edit E23 (and, shortened, E24); the lane owner may reword it, but not its facts, each of which was read in the code or the run named.

1. **Hardware security keys are not supported.** gwz offers an SSH agent's `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com` keys, and on macOS and Linux they authenticate in gwz's tests through a software authenticator that answers the agent's signature request. Signing with a physical key, including the wait for its touch, has not been tried. gwz waits for the agent's answer under `--ssh-timeout` (9 seconds by default), so a touch that takes longer ends the attempt with a message that names the option; raise `--ssh-timeout`. A key that the agent declines to sign, such as a security key that is not plugged in, is skipped and the next key in the agent is tried; to offer only one key, give its public-key file to `--identity` (exact agent-key selection, decisions D38 and D39, *pending its landing*). On Windows no security-key row has run yet (*pending the Windows security-key rows*).
2. **Kerberos is not supported (Windows).** A server that offers `Negotiate` is answered with the logon session's default credentials, as in 1.0.17. On a domain-joined host Windows may choose Kerberos inside `Negotiate`; gwz does not prevent that, asks for neither delegation nor mutual authentication, and has tested the exchange only where NTLM was chosen, so no claim is made for Kerberos. If a `Negotiate` sign-in that worked with 1.0.17 fails, run the command with `--transport native`, which is 1.0.17's behaviour for that command. A sign-in that Windows refuses is reported with its own message naming Kerberos and NTLM policy and `--transport native` (decision D48, *pending its landing*). An exchange that ends without a verdict after a token was sent is reported as "HTTPS endpoint request failed: Protocol", which does not name Kerberos.
3. **Certificates held by Pageant or by the Windows OpenSSH agent service are not supported.** On macOS and Linux, certificates of Ed25519, ECDSA (P-256, P-384, P-521), RSA and the two security-key types (through the software authenticator of item 1) authenticate from an SSH agent against a server that trusts the signing CA; a DSA certificate is skipped. On Windows, Ed25519 (as with 1.0.17) and ECDSA (P-256, P-384, P-521; no 1.0.17 baseline) certificates authenticate from an agent on a pipe other than the OpenSSH service. On Windows, an RSA certificate did not authenticate in the tests, as with 1.0.17: libssh2 on Windows offers it only under the algorithm `ssh-rsa-cert-v01@openssh.com`, which the test servers' default list refuses, and a server that accepts it was not tried. A certificate held in Pageant or in the OpenSSH Authentication Agent service has not been tried, and naming one with `--identity <certificate>.pub` is equally unsupported.

What each sentence rests on:

- Item 1. The security-key types are offered on every platform: `agent_keys.rs` lists them in `TYPES` with no platform condition, and `agent_auth.rs` offers every listed type. The software-authenticator row is `key_types.rs`, `cfg(unix)`. The agent's sign request runs between `control.begin_wait()` and `control.complete_wait()` (`agent_auth.rs`, the `sign` callback; `agent_client.rs::sign`), a network wait under the stall clock; the default is 9 seconds (`gwz-cli/src/lib.rs:211`, `:220`), and the stall words are "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)" (`transport_host/session/driver.rs:155`, `:478`). A declined key is skipped: `agent_auth.rs` breaks to the next listed identity when the agent declined (`signer.refused`), pinned by `key_types.rs::an_agent_refusal_fails_that_key_only_and_a_later_key_is_tried` on Unix and on Windows. Windows has no security-key row (the Windows arm's comment).
- Item 2. The worker accepts either mechanism for `Negotiate` (`gwz-sspi/src/worker/session.rs:41-44`; `values.rs`: "Permit Kerberos or NTLM selection"), and gwz-core maps a Kerberos selection to `NativeMechanism::Kerberos` without refusal (`https_worker/native.rs:263-271`). The call asks for `ISC_REQ_CONNECTION | ISC_REQ_ALLOCATE_MEMORY` only (`conversation.rs:213-217`), so no delegation and no mutual authentication are requested; the target is `HTTP/<host>` and the identity is the logon session (`exchange.rs:165-168`). Release notes 1.1.0 already state that `--transport native` is 1.0.17's behaviour for the command (`Releases.md`, the paragraph "If something goes wrong after the upgrade"). The two messages are in section 5 bullet 2.
- Item 3. Unix: `key_types.rs` lines 100 to 135 (certificate types and logged names, RSA at flag 4, the two security-key certificate types) and `a_key_of_a_type_neither_supports_is_skipped_and_a_later_key_is_tried` (a DSA certificate is skipped). Windows: the Windows arm's `ed25519_and_ecdsa_certificates_authenticate_against_a_trusted_user_ca` and `an_rsa_certificate_does_not_authenticate_as_with_gwz_1_0_17` (run `2026-10-18-gapsb`: the RSA certificate's algorithm is refused by the server's default list, and a server that accepts it was not tried; the test's comment gives libssh2 on Windows offering the certificate only under `ssh-rsa-cert-v01@openssh.com`). On Unix the RSA certificate signs at `rsa-sha2-512`, flag 4, and authenticates (lines 100 to 135). The last sentence follows D39's `--identity <public-key file>` selection (`GwzExactAgentIdentityDesign-DRAFT.md`, OQ1) and §3.5 of that design.

## 4. Effect on S5.6, S7.5 and Phase 8

As deltas, because other lanes change the same table at the same time (one moves 12.1b on macOS and Linux to `E`, one moves 9.3 and 18.4 on Windows, `win92` will move 9.2 on Windows). Apply them to whatever the table says when this amendment is applied.

- **Cells that leave `G` and become `U`:** 9.4 on macOS, Linux and Windows, and 18.19 on Windows. By platform: macOS −1 `G`, +1 `U`; Linux −1 `G`, +1 `U`; Windows −2 `G`, +2 `U`. Two rows leave the blocker list: 9.4 and 18.19. Total: −4 `G`, +4 `U`. The table's §2 counts and its "17 platform cells in 9 rows" sentence are recomputed from the rows after E14 to E18; the figures are not edited by hand here.
- **9.3 on Windows is not `U`.** It is evidenced by `2026-10-18-gapsb` (PASS, matches 1.0.17 for the types the baseline covered). The first draft's open item that would have marked 9.3 on Windows `U` is dropped.
- **9.2 on Windows is not removed.** D41 does not name it; D49 (operator, 2026-10-18) gives it ported rows in lane `win92`. E15 marks the Windows cell `P: lane win92` and cites D49, and E28 and E29 list the lane in the cell table's "By pending step" table and its section 8, so the table and this section describe one state. The delta of E15 is −1 `G`, +1 `P` on Windows (not `U`). S5.6 cannot close on the cell without that lane's result or a further ruling. The first draft's statement that the question was "in other lanes" is replaced by this one.
- **Run `2026-10-18-gapsb`'s README** says of Windows security keys and DSA "lane `gapsa`'s amendment covers it". It does not: this amendment does not cover 9.2. The lane owner should correct that pointer to `win92`.
- The remaining `G` cells outside this amendment (10.2, 10.7, 12.1b and 17.11 at the time of the cell table) are not touched. This amendment adds no step and no evidence requirement.

## 5. What this amendment does not decide

- **It does not say the three cases are broken**, only that no evidence supports them. If a later release qualifies one, an amendment puts the cell back, as V110 §2 says of Linux ARM64 and macOS x86-64. A row of the exact-identity design's step 4.2 run against the real OpenSSH service would be evidence for such an amendment, not for 1.1.0.
- **It adds no refusal, no help text and no flag.** What a user sees today, from the code:
    - A hardware key whose touch is slower than `--ssh-timeout`: "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)" (`session/driver.rs:155`). A key the agent declines is skipped silently and the next key is used.
    - A Windows `Negotiate` sign-in whose provider call fails **before any token was sent**: `https_failure.rs` `sign_in_words` (line 313), the `Protocol` case (line 343): "GWZ's Windows sign-in worker did not answer as the one installed with gwz does: the file may have been replaced by another program, or Windows refused to start the sign-in. No credential was sent. Reinstall gwz, or" followed by the `--transport native` advice (`native_hint.rs`). **Decision D48** (main, `GwzTransportTR18-OperatorDecisions.md` line 91; **pending lane `words2`**) replaces these words for a provider-refused sign-in with its own: the cause, Kerberos and NTLM policy, and `--transport native`; the "reinstall gwz" text stays for an actual worker mismatch. The code shows at least four ways to reach the provider error on the first step: Windows rejects the credential handle (`conversation.rs:137-139`); the `InitializeSecurityContextW` status is an error (`session.rs:29`); the mechanism Windows chose is not one the package allows (`session.rs:52`); a token is over the size limit (`session.rs:58`). `native.rs::native_code` maps that error kind, which it does not list, to `Protocol`, and `exchange.rs:215-219` returns it with the facts still at "not started". Which Windows conditions produce a provider error on a Kerberos attempt was not run.
    - After a token was sent, the words depend on how the exchange ends. A 401 rejection of the logon identity says "<host> rejected your Windows account (<scheme>). GWZ tries nothing else…" (`https_failure.rs:237-249`). A redirect has its own words (`:319-325`). Only an exchange that ends with the `Protocol` code reads "HTTPS endpoint request failed: Protocol" (pinned at `:785`).
    - The `Protocol` texts do not name Kerberos. Whether a Kerberos failure on a domain-joined host reaches the first is not reproduced; the sequences are readings of the code. The wording is decided (D48), not changed by this amendment (section 6 item 3).
- **It does not alter `exact_agent_identity` or the exact-identity design.** `GwzExactAgentIdentityDesign-DRAFT.md` §3.5 says "Real security-key and Pageant-held-certificate signatures remain unqualified" and "Exact selection does not widen that claim"; with D41 "unqualified" there reads as "unsupported in 1.1.0", and no edit is needed. Its rows C1 to C4 (a certificate does not match its plain key, and the reverse) run on the owned pipe agent fixture (`agent_fixture_pipe`, §4.2) and are outside D41. Selecting a certificate held by Pageant or by the Windows service with `--identity <cert>.pub` is equally unsupported (item 3).
- **Review.** Reviewed Consistency and Safety under D41; this is revision 2, the second and last round of at most two. The lane owner applies section 2 only after both reviewers re-verdict.

## 6. Open items for the lane owner

1. **The file name.** Rename this file to `GwzV110AmendmentS56Scope.md` when it is accepted; section 2's replacement text already uses that name. D41 cites the `-DRAFT` name, which keeps its meaning until then.
2. **Windows 9.2.** Lane `win92` owns it (D49). Correct run `2026-10-18-gapsb`'s README pointer (section 4). Notes item 1 says "on macOS and Linux" and "pending" for Windows; update it with `win92`'s result.
3. **The Kerberos wording is decided (D48) and pending lane `words2`.** This amendment makes no code change. When `words2` lands, check the first message sentence of notes item 2 (and E23 and E24) against its text; the "HTTPS endpoint request failed: Protocol" sentence for an exchange that ends without a verdict after a token stays as written. Cite D48 from main until this lane's copy of the decisions file is rebased.
4. **A later row against the real service.** A step 4.2 row of the exact-identity design against the OpenSSH Authentication Agent service would evidence cells this amendment marks `U`; if it runs and passes, put the cell back by amendment. The facts in L1 to L3 stay in the design (E12 keeps them).
5. **Help text.** No help text names any of the three: a search for "Kerberos", "hardware" and "security key" finds them only in `Releases.md:154` (E22), the Windows guide (E25 to E27) and the design documents; `GwzV110HelpAudit-DRAFT.md`, `gwz-cli/src`, `gwz-py/src` and the READMEs have none. So there is no help edit. The Windows guide's sentence that an RSA certificate's failure message "says so" is dropped by E25 (the first draft's reviewers report that the candidate prints only the generic setup-failure text; this amendment states no message for it).
