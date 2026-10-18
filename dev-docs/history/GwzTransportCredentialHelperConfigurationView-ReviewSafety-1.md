# GwzTransportCredentialHelperConfigurationView — SAFETY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportCredentialHelperConfigurationViewAmendment.md` and `GwzTransportCredentialHelperConfigurationView-Spike.md` at core `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`; round-1 corrected DRAFT mechanism proposal, dated 2026-10-03. Product implementation remains excluded.

**Baseline:** `/Volumes/projects/limbo/gwz-dev-tr2-22`: root `054d1dddc345452a2285e74c31f5ea4c0b0d5360`; core `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`; transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`; private evidence `84fef16e6b225cf6668d4bbd4fb8c4678b649723`. Committed bytes were inspected using `git show EXACT_SHA:path`. All four HEADs matched at the beginning and end.

**Date:** 2026-10-03

**Axis:** Safety — configuration scope, interruption, sensitive-copy lifetime, bounded ownership and refusal behavior. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — both original P2 findings are closed for this mechanism proposal; zero new P0, P1, P2 or P3 findings. This does not accept product implementation, platform qualification or release fitness.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1: captured HOME expansion lacks a fixed interpretation for relative or empty values | Require nonempty absolute captured HOME for `~/`; otherwise M2 before include I/O. Require absolute source and command-cwd anchors. | Re-traced the original `HOME=tmp/config-home`, parent cwd `/workspace`, absolute root and competing include-file sequence against amendment lines 100–110. The lookup now refuses before either unintended include open. Empty and absent HOME take the same refusal; absolute HOME remains supported. Frozen v7 resolver lines 63–69 places the guard before path return or I/O; assertions at lines 260–279 cover refusal and distinct source/HOME/command anchors. | **CLOSED — proposal** |
| P2-2: persistent sensitive scratch has no recovery owner after process termination | Eliminate named sensitive preparation artifacts: parse bounded source bytes through stdin and carry flattened entries in bounded process-lifetime parameters. | Re-traced termination after source preparation, during fill and after cleanup interruption. Amendment lines 87–98 and 198–208 remove the source-copy/view writes that created the orphan. Frozen v7 `parse`, `parameters` and `controlled` at lines 17–62 create pipes and environment bytes, not named config copies. Lines 285–299 exercise killed preparation/fill groups and assert absence of preparation-copy names. Existing fixture inputs are distinguishable from product-owned preparation artifacts. | **CLOSED — proposal** |

Verification here means independent inspection and re-tracing on the corrected tuple, supported by committed executed feasibility evidence. No new experiment or product test was run during this read-only re-review.

## Changed-range analysis

I compared the complete committed amendment and receipt at original core `63abcb8afa89750eb19b93bf1614fd36fc2d292e` with corrected core `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`.

The amendment changes are:

- Lines 3, 20–34 and 50 identify the corrected, process-lifetime mechanism.
- Lines 87–98 replace private source-copy parsing with `git config --no-includes --null --file - --list`, retaining bounded input and child/buffer ownership.
- Lines 100–110 freeze supported HOME forms and absolute include anchors.
- Lines 119–154 replace the disk writer/global view with ordered `GIT_CONFIG_PARAMETERS` encoding, `/dev/null` as the empty Unix global source, command-origin-only exact verification, inherited helper configuration, OS-size refusal and explicit inspection limitations.
- Lines 158–208 update preparation accounting and sensitive-buffer/native-copy lifetime without changing admissions, the single deadline or retained worker/child rules.
- Lines 212–257 update owners, supersessions, regression obligations and the maximum child count from 132 to 131.
- The receipt changes at lines 6–17 and 23–35 identify the v7 primitive, added HOME/inheritance/E2BIG/kill evidence and preserved qualification limits.

These changes fit the combined RemPlan’s two accepted dispositions. I found no unrelated expansion of authentication policy, public API, wire vocabulary, Windows mechanism, retry policy or protected dependencies.

**Classification:** Replacing named files with stdin and configuration parameters is a material mechanism, secret-lifetime and private child-environment contract change. It requires review of the complete corrected proposal rather than a clerical closure. This re-review performed that broader inspection. It resolves the existing P2-2 architectural concern; I found **no new architectural root cause** and no new public/shared application interface. There is no new finding that triggers the architectural-root-cause remediation cap.

## 0. Evidence base

Read the canonical `ReReviewSafety-1.md` and the following committed inputs:

- Root `AGENTS.md`, `AGENTS_GWZ.md`, and core `AGENTS.md`.
- Root `AgentProcessRules.md`: L1-08/09 at lines 255–277, durable ownership/recovery at lines 319–355, and independent review/severity/closure at lines 370–462.
- Root `GwzProcessOptimization.md`: preserved rules at lines 18–27; physical feasibility at lines 76–87; remediation cap and review tiers at lines 97–119; current review-granularity ruling at lines 178–206.
- Root `CurrentProgramCheckpoint.md`, lines 1–47: corrected tuple, pending original-reviewer closure and excluded working implementation.
- Complete corrected configuration-view amendment, lines 1–263, and spike receipt, lines 1–35.
- Complete original versions of both objects for changed-range comparison.
- Complete committed initial Consistency and Safety reports and combined `GwzTransportCredentialHelperConfigurationView-RemPlan.md`, lines 1–29. These were legitimate prior-round inputs.
- Core `GwzTransportCredentialHelpersDesign.md`, revision 4: status and decisions, §3 command/environment/bounds/secrets, §4 outcomes and §7 retry.
- Accepted `GwzTransportCredentialHelperTimingAmendment.md`: status, captured timing/admission rules, scope-aware recovery and exact supersessions.
- `GwzTransportReleasePlanAmendment-2.md`, revision 6: §3.19 helper authority and §3.20 TR2.22.
- `GWZDesign.md` and `GWZRequirements.md`: accepted helper context, session/environment ownership, operation isolation and diagnostic privacy.

Private committed evidence under `campaigns/https-integration/runs/2026-10-03-tr222-config-view/`:

- `REMEDIATION.md`, lines 1–29, and complete `remediation-receipt.json`.
- Complete authoritative frozen `src/main-v7.rs`, lines 1–308.
- Complete v7 raw stdout/stderr.

The remediation receipt records numbered v4–v7 executions with status 0 and separate source/output fingerprints. V7 stdout reports the successful stdin/parameter-view chain, HOME refusal and anchors, helper inheritance, E2BIG refusal, killed groups and absence of named preparation copies. V7 stderr contains compilation/run notices, without a warning or credential output.

Only inspection commands were executed. No tests, builds, experiments, writes, trust changes or Git mutations occurred. Current peer re-review reports and moving working sources were not inspected. Supplied per-commit/archive-gate passes were not independently rerun.

The final four `git rev-parse HEAD` results exactly matched the baseline above.

## 2. Invariant analysis

**Original HOME counterexample.** The corrected contract rejects relative, empty or absent HOME only when an unconditional `~/` include requires it. The check precedes include I/O. In the original sequence, neither parent-cwd helper B nor a source-relative alternative can be opened. Absolute HOME, source-directory relative paths and command-scope paths retain explicit anchors. This closes the missing policy decision without rejecting configurations that never require HOME expansion.

**Original crash/orphan counterexample.** Source bytes now enter Git through stdin. Flattened entries enter controlled children through configuration parameters. No named source copy, view or scratch directory is created by the mechanism. Killing the owner therefore cannot leave the core-owned filesystem copies identified in P2-2. The correction removes that durable edge rather than introducing an incomplete recovery classifier. OS/native memory is explicitly outside the physical-zeroization claim.

**Encoding, ordering and scope.** The new encoder distinguishes valueless `'key'` from empty `'key'=''`, preserves order and escapes apostrophes. Before fill, controlled Git discovery must equal the complete flattened name/optional-value sequence and report only command/`command line:` origins. Encoder discrepancies or resurrected file/installation/system/global origins refuse as M2. The frozen v7 checks preserve representative byte values, subsection normalization, resets and repeated SYSTEM/GLOBAL visits.

This comparison is a required per-input gate, not merely a conclusion drawn from examples. Final `-c core.askPass=` remains last, and terminal prompting remains disabled.

**Helper inheritance.** Removing original overlays and replacing PARAMETERS prevents a helper’s ordinary inherited Git invocation from restoring the original include directives. Frozen helper A queries the inherited private sentinel and refuses if an `includeif` entry remains; real fill then selects A while the original configuration selects B. No configuration bytes need to become command-line arguments.

**Environment-size failure.** The encoded allocation counts toward the preparation ceiling, while the full environment remains subject to potentially lower OS limits. E2BIG/`ArgumentListTooLong` during controlled verification or fill is expressly M2, without truncation, disk fallback, broader retry or a missing-Git latch. Frozen v7 records the OS refusal before Git executes. This prevents the new representation’s capacity limit from being mistaken for a broken executable or causing a different helper selection.

**Ownership and clocks.** Both admissions still precede preparation. One interaction deadline covers discovery, reads, stdin parses, encoding, controlled verification and fill. Only one child is active at a time. An unfinished worker or unreaped child retains both quotas; a cleanup miss does not authorize slot reuse. The corrected maximum—128 source parses, one discovery, one combined verification and one fill—is 131 sequential children.

**Disclosure scale.** Flattened configuration bytes now appear in child environment/pipe memory. The proposal names that exposure, prohibits their appearance in GWZ diagnostics or retained data, and distinguishes core-owned zeroizing allocations from unavoidable Command/OS/native copies. It does not claim protection from same-user or privileged OS inspection. I found no additional disclosure boundary violation under that explicit exception.

**Preserved failures and recovery.** Unsupported discovery, representation, source and size cases fail closed as M2 rather than invoking broader configuration. Accepted helper result parsing, route ownership, retry outcomes, scope-aware recovery and repair/undo rules remain controlling. The patch adds no free-form refusal data or new public error code.

## 3. Risks and next action

Initial native discovery still reads original roots without a core-enforced input/allocation ceiling. OS execution limits can refuse a view below the proposed 4-MiB preparation ceiling. Native/OS copies have ordinary lifetimes, and process death is not physical secure erasure. These limitations are explicit.

The v7 proof establishes this primitive on macOS. It does not qualify the product’s cancellation, quota retention, worker cleanup, complete filesystem ceilings or other platforms; the receipt preserves that distinction.

The next action is for root to record the corrected mechanism decision after the required independent reviews, then implement the adopted contract and run its integrated product regressions. This GO supplies Safety acceptance of the proposed mechanism only.
