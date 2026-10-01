# GWZ transport release plan amendment 2 — remediation plan 2

Date: 2026-10-01. Status: **remediation plan for revision 2; not implementation authority**.

## 1. Inputs

- **Object:** `GwzTransportReleasePlanAmendment-2.md`, revision 1, SHA-256 `42b91afdb7c148bf5149124d79c58306ab4e1ab3d52ac6dda9321dbced4680b0`, untracked draft. The tuple is unchanged from round 1.
- **Re-verdicts:**
  - [Consistency-1](GwzTransportReleasePlanAmendment-2-ReviewConsistency-1.md): **GO**. It closes all 18 round-1 findings, and files 6 new P3s that may be applied without a further round.
  - [Safety-1](GwzTransportReleasePlanAmendment-2-ReviewSafety-1.md): **NO-GO**. It closes all 14 round-1 findings, and files 1 new P2 and 1 new P3. It pre-commits to GO on a revision that resolves P2-6 as specified.
- **Classification.** Both reviewers classify every new finding as non-architectural. This is remediation round 2 of at most 2.
- **Related findings.** Consistency's P3-5 and P3-6 and Safety's P2-6 all concern OD16. Revision 2 rewrites OD16 once, to resolve all three.

## 2. Dispositions

| Finding | Disposition | Closure test |
|---|---|---|
| Safety P2-6 (blocking) | **Accept.** OD16's recommended shape gains a bound, and states its effect. The route fires only when the URL maps to the Local Machine, Intranet or Trusted zone (`IInternetSecurityManager::MapUrlToZone`), the bound libgit2's own fallback applies (`winhttp.c:230-282`). Any other `NTLM` or `Negotiate` challenge is refused, with a message naming the off switch. The migration notes list that refusal as OD16's one parity exception. OD16 and TR1.8 say that the route's native connection then offers the logon session's default credentials to the host, as 1.0.17 does, since gwz's callback answers a default-credential request before libgit2's zone check runs (`transport_support.rs:272-274`). TR1.8's Safety finding list names the forced-authentication hazard and the bound. OD16 also records the unbounded alternative and its hazard, so that choosing it is a decision. | TR1.8's `Negotiate` fixture runs twice: under a name that maps to the Internet zone, where 1.1.0 refuses, the transport records one anonymous request, the fixture records no second connection and no authentication exchange, and the message names the off switch; and under a name that maps to the Intranet zone, where the operation succeeds on the native route. |
| Safety P3-9 | **Accept.** TR2.12's CI configuration that builds with both cfgs runs beside the transport-only configuration. The transport-only configuration stays until S7.1 (1.1.0), and then becomes the ordinary build's job. Rule (a) names both shapes as the ones CI keeps green. | A workflow-text test that the candidate job has two legs until S7.1 (1.1.0): `--cfg gwz_transport_candidate` alone, and both cfgs. |
| Consistency P3-1 | **Accept.** §1's session-plan list gains CS7.24, §5.4's `transport_binding.rs` row, Phase 6's preamble sentence on the lazy endpoint's debt entry, and §3.0's marker definition. §3.15 reads the first three as TR2.11's. §3.0's "a build without `gwz_transport_candidate`" reads "a build without the candidate switch rule (a) names at the time". §5's session-plan status sentence follows. | Every session-plan sentence on the lazy endpoint, or on a build without the candidate switch, is named in §3.15. |
| Consistency P3-2 | **Accept.** TR3.3's third bullet: the other names stay placeholders until 1.2.0 publishes them, except gwz-transport, which is already bootstrapped and is released by Phase 10 (1.1.0) step 2. | Each of the thirteen names has exactly one first real publication in the amendment. |
| Consistency P3-3 | **Accept.** Each product repository keeps an inventory file for each switch's sites in that repository. Its source test reads the file in that repository's own CI, and the checkpoint records each file's digest. Rule (a), TR2.12 and S7.1 (1.1.0)'s equality check use the files. | The inventory test passes in each repository's own CI with no checkout of the root. |
| Consistency P3-4 | **Accept.** TR2.12 names the process-globals checker, whose definition of production names `gwz_transport_candidate` (`check_process_globals.py:63`) and gains `gwz_session_candidate`. The conditional-compilation check needs no change. | `check_process_globals.py` lists a `debt` entry planted under `cfg(gwz_session_candidate)`. |
| Consistency P3-5 | **Accept.** OD16's route applies on Windows only. On macOS and Linux, TR1.8 records with 1.0.17, against the loopback `Negotiate` fixture, whether 1.0.17 authenticates there. If it does, the migration notes list the case as unsupported on the transport, refused with a message naming the off switch. | TR1.8's evidence list names a 1.0.17 row on macOS and on Linux against the fixture. |
| Consistency P3-6 | **Accept.** OD16 states an edit list for each alternative, as amendment 1's OD12 did: "If (a)", refuse every such challenge; "If unbounded", parity with 1.0.17, with the hazard stated. | Each OD16 outcome maps to a complete, applicable edit list. |

## 3. Integration and re-verdict

- **One patch:** revision 2 applies every disposition above in one change, and nothing else.
- **Re-verdicts:** both axes re-verdict revision 2, so that GO rests on one revision. Safety re-checks P2-6 and P3-9 and the changed range. Consistency re-checks its round-2 P3s and the changed range.
- **The cap:** a further round, if any, is permitted only for non-architectural corrections. An architectural root cause in it stops the lane.
