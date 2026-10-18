# Configuration-view mechanism — remediation round 1

2026-10-03. Status: **NO-GO; one combined correction pending**.
Reviewed root `d13bdebdbf0ce9291e99510a1d5a36750371755c`, core
`63abcb8afa89750eb19b93bf1614fd36fc2d292e`. Consistency reports GO with
no findings; Safety reports NO-GO with two P2 findings. Both complete reports
are filed verbatim. No blind convergence was claimed. The implementer's
accepted helper-context work remains independent and does not adopt this view.

| Finding | Disposition | Required closure evidence |
|---|---|---|
| Safety P2-1: relative/empty HOME expansion | Accept. When an unconditional `~/` include requires HOME, support only a nonempty absolute captured byte path; absent, empty or relative HOME refuses as M2. Every supported include has an explicit absolute anchor before worker I/O. No parent cwd or ambient home participates. | Distinct synthetic include files for child cwd, parent cwd and source directory; absolute/relative/empty/absent HOME, nested and command-scope includes. Verify specified selection/refusal and no unintended file read. |
| Safety P2-2: orphaned sensitive scratch after process death | Accept. Prefer eliminating named persistent copies instead of adding a durable recovery subsystem. Prove a concrete process-lifetime mechanism before changing the draft: native Git parses source bytes through `--file -` stdin; investigate a bounded Git configuration environment for the flattened entries, preserving null/empty/byte values and exact order. This is a candidate correction, not an accepted mechanism. | Full real-Git discovery/walk/fill and normalization/overlay matrix; exact roundtrip; helper inheritance; process/environment size refusal; fail/drop/kill during preparation and fill, no core-owned named sensitive artifacts and no secret diagnostics. If the primitive cannot satisfy the original case, stop and propose a physically proved OS-lifetime alternative; do not silently retain unowned files. |

One patch must update the mechanism, exact supersessions and concise physical
receipt together. Explicitly name any new environment variables, buffer/native
copy lifetimes, admission/deadline/child ownership and OS limitations. Preserve
the unconditional-only policy, same one interaction deadline, bounded refusal,
accepted error mapping and absence of protected-dependency changes. The stdin/
environment direction must not become product code before mechanism acceptance.

No speculative feasibility claim closes P2-2. Retain prior failed/green evidence
unchanged; new numbered frozen sources and raw receipts belong in the authorized
private campaign. Product hasconfig remains RED until accepted implementation.

Root settles one corrected tuple and requests the original reviewers to verify
the counterexamples; any material mutation/interface change also receives the
fresh independent review required by review-loop. The reviewers classify new
architectural causes; the two-round cap remains. Implementation cannot self-GO.
