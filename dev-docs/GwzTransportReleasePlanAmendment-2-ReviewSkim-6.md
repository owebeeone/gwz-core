# Skim second re-check of amendment 2 revision 5

**Objects:** (SHA-256 verified at start and at end; none changed; heads root `1292b0c`, gwz-core `fde46265`, gwz-cli `5ebb001`, gwz-py `b24204e`, gwz-transport `a24e70a` at both)
- `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2.md` — `316df0d8c979fbf6bfad369d47bfe31181ff34cbabf628886aa873c83bbc7a9a`
- `dev-docs/GwzCoreSessionPlan.md` — `a581e4b6d86972593fc06e40d50e0e952bb738c7947f5e352c3880c2f67d4082`
- `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2-Verdict.md` — `183a88c5ac2739bc3c2f724aaa31f39fb6a27013c142422354f3cc2c71e4be44`
- `dev-docs/CurrentProgramCheckpoint.md` — `fa3f47233bc711ded0c86ffa819b06fb94f682ad3b4aa6bfb84c2fe200b11fd5`
- Unchanged since the re-check: `GwzTransportReleasePlan.md` `7c8dab5b…`, `GwzCoreServerDesign.md` `e2d3ee2f…`, `GwzTransportReleasePlanAmendment.md` `bad2abd8…`, `GwzRemoteTransportSshAgentDesign.md` `2aed855a…`, `GwzConnectionReuseDesign.md` `36cd64f1…`
- Filed re-check: `gwz-core/dev-docs/GwzTransportReleasePlanAmendment-2-ReviewSkim-5.md` — `3e14854effa25ae99fa6560e63e646f70c0b4b9e6b4f56138616fdc2b985eb46`. The `rev5b/` copy of the amendment hashes `df94662d…`, as re-checked.

**Verdict: GO** — P0: 0, P1: 0, P2: 0, P3: 0. All four findings are closed; no new issues.

| ID | Fix claimed | Verified | Status |
| --- | --- | --- | --- |
| P2-6 | TR1.8's precedence: helpers answer first only for `NTLM`, `Basic` or `Digest`, over WinHTTP's order (Negotiate, NTLM, Digest, Basic; SSPI with the helper's identity for the first two); a `Negotiate`-only challenge takes the logon session, no helper asked; callback gating cited; OD16's bullet matches; fixture rows and TR4.10's tests follow | TR1.8 line 163 carries the specified text with `winhttp.c:139-147`, `:624-627` and `transport_support.rs:265-274` ("its helper branch runs only when a user and password are allowed"); OD16 line 364: "when the challenge offers `NTLM`, `Basic` or `Digest` … A challenge that offers only `Negotiate` takes the logon session's default credentials, and no helper is asked, as on 1.0.17"; fixture line 175: the `Negotiate, Basic` row authenticates "with the helper's identity over `Negotiate`, as WinHTTP picks it", and the new `Negotiate`-only row "asserts that the helper is not asked and the logon session authenticates"; TR4.10 line 223 has both rows. Consistent with `parse_unauthorized_response` (`winhttp.c:618-637`) and `apply_userpass_credentials` (`:139-147`) | Closed |
| P3-4 | The transport design's HTTPS cell reads "configured helpers, through `git credential fill` (TR1.6)" | §3.19 line 469 | Closed |
| P3-5 | Amendment 1's §3.8 notes item reads "that the transport signs RSA with SHA-1 exactly where libssh2 does, in TR2.8's three cases, as 1.0.17 does" | §3.19 line 492 | Closed |
| P3-6 | §3.15 gains the CS3.4 bullet; §1's session-plan entry, §5's status-edit text, the applied status line and a session plan changelog entry name CS3.4 | §3.15 line 388: "CS3.4's transport half, the `git credential fill` spawn for the transport's HTTPS, is TR1.6's in 1.1.0 (§3.19). CS3.4 keeps its native-callback change, its §15.8 rows and its tests, under rule (e)"; §1 line 29 "CS3.4's transport half (§3.15, §3.19)"; §5 line 525 and the applied session plan line 6 both read "CS8.3's routing rule and CS3.4's transport half"; session plan changelog line 1574 records it | Closed |

**New issues:** none at the finding bar. The amendment's changelog (lines 567–571), the verdict's revision 5 section and the checkpoint record the re-check, its one P2 and three P3s, and their application consistently; the verdict's "Next action" and the checkpoint's commit-state bullets agree (revision 4 committed at `1292b0c`/`fde46265`/`b24204e`; revision 5's edits uncommitted). One wording note below the bar: the checkpoint's "Not committed" bullet (line 70) says "its skim review" where two review files, Skim-4 and Skim-5, now wait with it.
