# GWZ transport credential helpers design (TR1.6) — verdict

Date: 2026-10-02.

**Accepted** at SHA-256 `760f7ad4911bba53d9b7fa019be5225084236f1ab3cfd1a225cc8fceeeaeb025` (revision 3), after [Consistency-3](GwzTransportCredentialHelpersDesign-ReviewConsistency-3.md), [Safety-3](GwzTransportCredentialHelpersDesign-ReviewSafety-3.md) and [Surface-3](GwzTransportCredentialHelpersDesign-ReviewSurface-3.md) reported GO on that revision. Surface's GO assumes OQ5 (a). This accepts the design text only.

The [design](GwzTransportCredentialHelpersDesign.md) as filed is revision 4 (`9b2db41e0c58205eb5c29a43b18bec6e380b0ee99f82125ed3b84f47e6040ed2`) with two additions: the acceptance status block, and two citations of the uncommitted working draft `GwzRemoteTransportBugReport.md` marked as such. It hashes `9aef40ff93b6692be1f18ed0f1f15bf71d93b83feceb6ce0e47452d9b8d30d94`. Revision 4 carries round 3's corrections, which the reviewers cleared without a further round. [Consistency-4](GwzTransportCredentialHelpersDesign-ReviewConsistency-4.md) confirmed the one form that differs from the reviewer's text: under OQ7 (1), only the case of `Negotiate` without `Basic` becomes a loud clone failure, because the transport cannot tell an `NTLM`-only member with an answering helper.

## Before review

Revision 0 was the first draft. Before any review, the lane owner sent it back once, under the operator's parity rule. Every difference from 1.0.17 was classified as covered by a decision already made, or as the design's own choice. The two own choices that can break a working setup became open questions: prompting helpers, with the 120 s bound and the process group (OQ3), and a redirect after the challenge (OQ4). The stale C5 was updated. The result was revision 1.

## Round 1: revision 1 (`49b1a194…`)

| Review | Verdict | Findings |
|---|---|---|
| [Consistency](GwzTransportCredentialHelpersDesign-ReviewConsistency.md) | GO | 7 P3 |
| [Safety](GwzTransportCredentialHelpersDesign-ReviewSafety.md) | NO-GO | 1 P2, 5 P3 |
| [Surface](GwzTransportCredentialHelpersDesign-ReviewSurface.md) | NO-GO | 4 P2, 10 P3 |

- **Safety's P2.** OQ1 (b), as worded, passed a URL's decoded username to `git credential fill`. A crafted username in a shared manifest could add a second `url=` line, make git look up another host's credential, and send it to the attacker's host. The fix keeps URL text only inside the percent-encoded `url=` line. The destination refuses any component that decodes to a control character.
- **Surface's P2s:**
  - `gh` users were told to fix the credential "with git";
  - the no-helper case landed on the opaque M8;
  - no codes for M2–M8;
  - the configuration scope was unstated.
- **Blind convergence:**
  - M4's text and bound, on all three axes;
  - the `Capacity` outcome, on two;
  - prompting and the terminal, on two;
  - M2's next action, within Surface.
- [Remediation plan 1](GwzTransportCredentialHelpersDesign-RemPlan.md), E1–E9, produced revision 2.

## Round 2: revision 2 (`db8009e8…`)

Every round-1 finding is closed.

| Review | Verdict | New findings |
|---|---|---|
| [Consistency-2](GwzTransportCredentialHelpersDesign-ReviewConsistency-2.md) | NO-GO | P2-1 and 3 P3 |
| [Safety-2](GwzTransportCredentialHelpersDesign-ReviewSafety-2.md) | GO | 1 P3 |
| [Surface-2](GwzTransportCredentialHelpersDesign-ReviewSurface-2.md) | GO, on condition that OQ5 (a) is taken | 4 P3 |

- **Consistency's P2-1.** M6 and M8 changed codes that 1.0.17 gave as `remote_rejected`, and with them clone's quiet skip.
- **Surface's P3-12** wanted the rejected credential (M3) not to be hidden by that skip.
- **Resolution.** [Remediation plan 2](GwzTransportCredentialHelpersDesign-RemPlan-2.md), F1, settled both by the parity rule: each outcome's codes and clone behaviour are 1.0.17's. It produced revision 3.

## Round 3: revision 3 (`760f7ad4…`)

| Review | Verdict | New findings |
|---|---|---|
| Consistency-3 | GO | P3-1 and P3-2: contingency text around C10 |
| Safety-3 | GO | P3-7: a `git` that cannot start takes M1's class; notes R-F to R-H |
| Surface-3 | GO, on condition that OQ5 (a) is taken | P3-15: M4's wording under OQ6 (b) |

Revision 4 applies them. C10, the wire `Failure`'s missing detail, becomes OQ7.

**Remediation rounds:** two, the cap. No reviewer labelled an architectural root cause.

## Open for the operator

- **OQ1, HTTPS URLs with userinfo.** Recommended (b): the username goes only inside the encoded `url=` line, and a password stays refused.
- **OQ2, Ctrl-C and the helper's process group.** Recommended (a): accept the cost now, and treat Ctrl-C as a cancellation in a later step.
- **OQ3, helpers that prompt.** Recommended (b): allow a sign-in window but not the terminal, with no timeout latch.
- **OQ4, a redirect after the challenge.** Recommended (a): follow it under discovery's rules, with a fresh lookup. (d) is the strict parity option: resend on the same origin, as 1.0.17 does.
- **OQ5, an error code for helper timeouts** (`credential_helper_timeout`, 75). Recommended (a): allocate it. Surface's GO depends on it.
- **OQ6, where the slot wait is charged.** Recommended (a): the allocation deadline.
- **OQ7 (C10), a failure detail on the wire.** Recommended (1): an optional field carrying fixed causes and bounded scheme tokens.

## Follow-ons

- **TR2.22** implements the design after these answers. TR2.2 reproduces defect 1 first.
- **TR2.23** is the SSH password-only parity gap that TR2.18 found. It reuses this design's lookup.
- **Amendment 2's next revision:**
  - C2: "one validated discovery redirect" is the whole redirect sequence;
  - C7: where TR2.2's fix lives;
  - TR2.23;
  - the texts that §9 and C3 amend.
- **The response-level list of quiet skips** (Safety-3 R-H) goes to the connection-diagnostics design.
