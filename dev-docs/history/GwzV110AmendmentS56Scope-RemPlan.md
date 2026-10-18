# S5.6 scope amendment (D41): remediation plan, round 1

**Reviews:** `GwzV110AmendmentS56Scope-ReviewConsistency.md` (NO-GO: 5 P2, 4 P3) and `-ReviewSafety.md` (NO-GO: 3 P2, 3 P3). Both are peer-blind, dated 2026-10-18.

**How it is fixed:** one merged revision. Each reviewer then re-verdicts its own findings.

**Blind convergence** (each item found independently by both reviews):

| Topic | Consistency | Safety |
|---|---|---|
| Windows 9.2 has no owner | F2 | S3 |
| Certificate notes wrong by platform | F3 | S3 |
| `Releases.md:154` and the guide contradict the change | F4 | S5 |
| Authority predates D41 | F6 | S6 |
| The touch wait counts against `--ssh-timeout` | F8 | S4 |

**Operator ruling (2026-10-18), new since the reviews:** Windows 9.2 gets ported rows. Security keys through a software authenticator, and DSA, from a pipe agent on Windows, go in lane `win92`. So 9.2 W is not removed by this amendment. It becomes evidenced, or pending on that lane.

| Finding | Disposition | Closure |
|---|---|---|
| C F1: superseded table edited; controlling rows left | Accept | Exact replacement rows for: A1 §3.2 as rewritten by A2 §3.19 (the unsupported column); A2 §3.2's Windows row; OD11's list (A2 :487-488); A2 §3.10 :406. V110 §2 becomes a cross-reference. |
| C F2 / S3: 9.2 W ownerless | Accept, per the ruling | §4 and §6 item 4: 9.3 W is evidenced (gapsb); 9.2 W is owned by lane `win92`. Notes item 1 and §2 row 2 say "on macOS and Linux"; the Windows status follows win92's result. |
| C F3 / S3: certificate notes | Accept | Item 3 reworded by platform from `key_types.rs` and gapsb. §1 row 3 re-cited to `2026-10-18-gapsb`. |
| C F4 / S5: `Releases.md:154` and guide lines 335-337, 583, 612 | Accept | Exact old and new text that replaces the "not promised: certificates held by an agent" clause. |
| C F5: edits described, not given | Accept | Exact old and new text for: WPD §13 and §14 item 10 (split); WPP OQ10 and 6.3; cell table §3.5; ledger §5 item 5 and rows L1 and L6; gwz-py `Migration-1.1.0-DRAFT.md`; the named Releases.md sections. |
| S S1: "Kerberos unsupported" misleads | Accept | Item 2 says Windows may choose Kerberos inside Negotiate on a domain-joined host; gwz does not prevent it; no delegation; untested, no claim; `--transport native` as the fallback. Ledger L6 is qualified the same way. |
| S S2: Kerberos failures give misleading text | Accept (split) | §5 names the actual texts (`https_failure.rs:342-344` "reinstall", and the bare `Protocol`). Notes item 2 points to `--transport native`. The code wording fix is a separate finding the lane owner files. |
| C F6 / S6: authority stale | Accept | Cite D41 and the dual review; settle the accepted file name. |
| C F7: absolute counts | Accept | State them as deltas. |
| C F8 / S4: touch wait and declined-key fallback | Accept | Item 1 gains two sentences: the touch counts against `--ssh-timeout`, and a declined key is skipped to the next key, with D38 selection as the remedy. |
| C F9: tie to D38 | Accept | Cite EAI §3.5. C1 to C4 on the owned pipe are outside D41. "Selecting such a certificate with `--identity <cert>.pub` is equally unsupported". |
