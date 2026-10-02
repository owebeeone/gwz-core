# GWZ transport credential helpers design (TR1.6) — remediation plan 2

Date: 2026-10-02.

**Object.** Revision 2, SHA-256 `db8009e8318ebcb1ca4ff88ea7f464c63d35217d76385f267547d2b15815d3e1`. A frozen copy is at `GwzTransportCredentialHelpersDesign-rev2.md`.

**Round-2 reviews.** Every round-1 finding is closed on all three axes.

| Review | Verdict | New P2 | New P3 |
|---|---|---|---|
| [Consistency-2](GwzTransportCredentialHelpersDesign-ReviewConsistency-2.md) | NO-GO | P2-1 | P3-1 to P3-3 |
| [Safety-2](GwzTransportCredentialHelpersDesign-ReviewSafety-2.md) | GO | 0 | P3-6 |
| [Surface-2](GwzTransportCredentialHelpersDesign-ReviewSurface-2.md) | GO, conditional on OQ5 (a) | 0 | P3-11 to P3-14 |

Consistency pre-commits to GO on a revision that resolves P2-1 as specified.

**This is the second and last remediation round** under the two-round cap. No reviewer labelled a new architectural root cause.

## Convergence

Consistency-2 P2-1, Surface-2 P3-12 and Surface-2 P3-13 all concern which code each outcome carries, and so what clone does with it. They pull in different directions:
- Consistency wants 1.0.17's `remote_rejected` kept for M6 and M8;
- Surface wants M3 not to be hidden by clone's quiet skip, and `unsupported_operation` not to be stretched past "not built yet".

**The operator's parity rule settles both** (F1).

## Dispositions

**F1. Each outcome's codes and clone behaviour are 1.0.17's** (Consistency-2 P2-1, Surface-2 P3-12 and P3-13).
- **Parity first.** For every outcome M1–M11, establish 1.0.17's behaviour for the same case: its member code on fetch and push (`handle_fetch.rs:430-435`, `push_member.rs:518-527`), and its clone treatment, a quiet skip or a failed member, which 1.0.17 decides by `clone_error`'s code and message recognition (`transport.rs:737-763`). Make revision 3 match it.
  - M6 and M8 take `remote_rejected`, and clone skips a `private: true` member quietly for them, as 1.0.17 did.
  - M3, the rejected credential, fails the member loudly on clone, as 1.0.17 did, since its replay-limit message is not recognized. That answers Surface P3-12 by parity.
  - M5's code is 1.0.17's code for the same case, under each OQ4 option.
- **The exceptions.** An outcome 1.0.17 never had keeps a code chosen by the catalog's rule:
  - M4 and M10, the bounds. 1.0.17 had no bound. They take OQ5's allocation.
  - M1, the missing `git`. 1.0.17 did not need git. It keeps `external_tool_missing`.
- **Where an option changes the outcome,** for example OQ4 (c)'s refusal, the open question's text states the code and clone behaviour under that option.
- **The principle.** §11's sentence reads "Codes, and clone's skip or failure, are 1.0.17's wherever 1.0.17 had the case". D19, N14 and the codes table follow.
- **Wire codes** (Consistency-2 P3-1). §4's wire-failure column and HTTPS §7 :315 agree. If M6's wire failure stays `UnsupportedOperation`, §9.1 amends :315 as the finding gives it; otherwise M6's wire failure is `Authentication`.
- **The catalog** (Surface-2 P3-13). If any message still carries `unsupported_operation` after F1, ErrorCatalog row 22 and line 94's "exactly 'not built yet'" sentence are amended in §11's page list.
- **Tests.** T10(b), T17 and T8(c) assert the parity codes and clone outcomes, with T8(c) gaining the `Bearer`-only `private: true` member case.

**F2. M10's bound** (Consistency-2 P3-2).
- `<m>` is the allocation budget the Open had left when the wait began: at most 30 seconds, or the caller's own deadline when nearer.
- §3.3's "30 s, fixed" reads "up to 30 s, less what the anonymous attempt used (HTTPS §4)".
- T15 gains the sub-row with a nonzero anonymous allocation wait.

**F3. The slot-wait deadline is an own choice that can break a setup** (Consistency-2 P3-3). §8 moves it into the list of choices that can break a working setup. §12 gains OQ6:
- (a) await a slot within the allocation deadline, as revision 2 does;
- (b) await it within the interaction allowance, 120 s, as REUSE:147 does, at the cost Safety P3-1 named.

State the recommendation and the hazard of each. §1's sentence then holds.

**F4. Server text is attributed and delimited** (Safety-2 P3-6).
- M9 shows the reason phrase as `the server's reason: "<phrase>"`, or drops it, since M9 already points to `git ls-remote`.
- M6 shows the schemes as `the schemes the server named: "<token>", …`.
- The length cuts stay.
- Line 293's rule names exactly these two attributed fragments as its exceptions.
- T8(b) and T10(b) gain instruction-shaped server text, and assert it appears only inside the delimiters.

**F5. Where the member's URL is** (Surface-2 P3-11). The Troubleshooting section, and one clause in every message that says "with this member's URL", state where it is:
- for a cloned member: `git -C <member path> remote get-url origin`;
- for one not yet cloned: the manifest's URL in the form gwz uses, which is `https://…` when the workspace uses `--url-scheme https`. `gwz --verbose materialize --lock` prints it as `manifest-url -> effective-url`.

**F6. includeIf** (Surface-2 P3-14). N3 reads "under any `includeIf`, whether `gitdir:` or `hasconfig:remote.*.url:`". M2 matches.

**F7. Safety-2's residual notes, applied as text:**
- **R-A** (under OQ1 (b)): name the Open field that carries the username, define it as percent-encoded, and require the endpoint's re-validation to apply D18's decoded check.
- **R-B:** say whether stderr's read end closes when the lookup completes.
- **R-C:** covered by F2.
- **R-D:** under `--verbose`, a quiet clone skip lists the member and the reason class. This keeps the documented quiet skip quiet, and makes it traceable.
- **R-E:** D18's decoded-control check lives in the shared destination validation, so a redirect's `Location` is covered too.

**Also:** C3's gwz-py citation becomes `:134` at `2e0509f` (Consistency-2's drift note). TR1.5's `<hint>` text has stayed `--transport native` / `GWZ_TRANSPORT=native` through TR1.5's revision 2.

## Re-checks after revision 3

- **Consistency re-checks** P2-1's counterexamples: the `Bearer`-only private member on clone, and the fetch code. It also re-checks its P3-1 to P3-3.
- **Surface confirms** its P3-11 to P3-14 and the codes. Its round-2 GO assumed OQ5 (a). F1 keeps OQ5 for M4 and M10 only.
- **Safety's P3-6** closes on re-read, and Safety re-checks only if F1 changes a hazard.

The operator then answers OQ1 to OQ6.
