# GWZ 1.1.0 plan amendment — third draft verdict

Date: 2026-09-26. Status: **accepted at SHA-256 `cb4ae1664a595231bde73cdb83809fa4b2316f574ca6365fb878255df4be1585` after [Consistency-2](GwzV110PlanAmendment-ReviewConsistency-2.md) and [Safety-2](GwzV110PlanAmendment-ReviewSafety-2.md) reported GO; this accepts the plan text only**. It authorizes no implementation, commit, tag, push or publish. S1.1's revision of gwz-py's transport design, and its S1.2 review, remain the gate before Phase 6.

The same two reviewers re-verdicted the third draft with their context intact.
- They verified its hash at the start and end.
- They did not read each other's current-round report.
- They ran inspection commands only.

| Axis | Verdict | Prior findings | New |
| --- | --- | --- | --- |
| Consistency | GO | P2-3, P3-6, P3-7, P3-8 closed | P3-9 |
| Safety | GO | P2-2, P3-8, P3-9, P3-10 closed | P3-11 |

The shared blocking root B1 is closed on both axes: gwz-py's normal build is now switched to the transport and proven before the tag. So is every finding from the three rounds. No reviewer classified any finding in any round as architectural.

## Corrections applied after the GO

Both reviewers cleared their new P3 findings to land without a further round. Consistency cited L1-22. Safety said the fix adds a phase to an existing rule and changes no step. Each correction below was applied as its reviewer specified. Two further wording fixes come from Consistency's residual risks. The amended file then carries the accepted status line and hashes `35763597cdf56b3ac20dfc30b711dee74691640589906d4902372719674da1a2`.

1. **Consistency P3-9.** S7.1's completeness check could never pass as written, because the string appears in dev-docs. It also missed gwz-cli's switch sites and the harness test. S7.1's added text now names:
   - gwz-cli's sites (`src/globalargs/dispatch.rs`);
   - all three crates' `check-cfg` declarations;
   - `test_prepare.py`, whose protocol-boundary assertion is retired or inverted, and the harness `README.md`.

   The `rg` check is scoped to source, tests, scripts and manifests of gwz-core, gwz-cli and gwz-py, excluding `dev-docs`.
2. **Safety P3-11.** The redaction rule's replacement reads "Phases 4, 5, 6, 7, and 8 redact", so S7.3's new route evidence is covered.
3. **Consistency residual.** S7.2's Python ledger rows are entered at S7.2 on S6.3's assertions and confirmed by S7.3 before S7.5.
4. **Consistency residual.** S6.3's close row no longer gives interpreter exit a report. `Client.close()` counts the running operation in its report. Exit cancels or joins it. Neither leaves a helper process.

## Carried, below the finding bar

These go to S1.1's revision and its S1.2 review:
- whether a waiting operation holds a native thread, and a bound on the waiting set;
- the interpreter-exit bound when `configure_transport_runtime` disables deadlines;
- the per-operation environment read's race with `os.environ` mutation, the same exposure as the CLI's entry;
- which reading of "paths" S7.2's notes use for the native branch on unsupported platforms;
- the fixtures that S7.3's Linux run needs on the CI host.

## Application

The amendment's §5 lists the status-only edits made on this GO:
- **`GwzV110Plan.md`'s status line** gains the "Amended … by `GwzV110PlanAmendment.md`" sentence. Its superseded text stays readable.
- **`gwz-py/dev-docs/GwzPyTransportDesign.md`'s status** records the supersession and the NO-GO's closing condition.
- **The program checkpoint** records the acceptance. `dev-docs/CurrentProgramCheckpoint.md` carries another lane's uncommitted edits, so that entry waits until those land.
