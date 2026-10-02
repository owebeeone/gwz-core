# GwzTransportCredentialHelperConfigurationView — CONSISTENCY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportCredentialHelperConfigurationViewAmendment.md` and `GwzTransportCredentialHelperConfigurationView-Spike.md` at core `63abcb8afa89750eb19b93bf1614fd36fc2d292e`. DRAFT mechanism proposal and physical-feasibility receipt, dated 2026-10-03; neither product implementation nor release acceptance.

**Baseline:** Repositories under `/Volumes/projects/limbo/gwz-dev-tr2-22`:

| Repository | Exact SHA |
|---|---|
| Root | `d13bdebdbf0ce9291e99510a1d5a36750371755c` |
| Core | `63abcb8afa89750eb19b93bf1614fd36fc2d292e` |
| Transport | `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` |
| Private evidence | `9ef2a6adbc19cfb8a038e7b841d954c0c9d3b248` |

Documents and frozen feasibility sources were read using `git show EXACT_SHA:path`. All four HEADs matched at the beginning and end.

**Date:** 2026-10-03

**Axis:** Consistency against the controlling document graph, precise supersessions, internal invariants and evidence claims. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — zero P0, P1, P2 or P3 findings. This verdict covers the proposed mechanism’s documented consistency, not its implementation, platform qualification or release fitness.

---

## 0. Evidence base

Read the canonical `PromptConsistency.md`, workspace `AGENTS.md`, `AGENTS_GWZ.md`, core `AGENTS.md`, and these committed authorities:

- Root `dev-docs/AgentProcessRules.md`: authority, amendment, exact-baseline, independent-review, severity and evidence rules, including L1-01, L1-08/09 and L1-16–22.
- Root `dev-docs/GwzProcessOptimization.md`, lines 1–206, including physical feasibility before freeze and the current review-granularity ruling.
- Root `dev-docs/CurrentProgramCheckpoint.md`, lines 1–120: exact review object, accepted helper-context contract, excluded working implementation and outstanding product regression.
- Core `GwzTransportCredentialHelpersDesign.md`, revision 4, lines 1–433: lookup scope, command, environment, clocks, ownership, secrets, outcomes, supersessions, tests and recovery.
- Core `GwzTransportCredentialHelperTimingAmendment.md`, particularly lines 38–142 and 203–304: captured allowances, unchanged timing policy, scope-aware recovery and precise representation changes.
- Core `GwzTransportReleasePlanAmendment-2.md`, revision 6, particularly §§3.15, 3.17, 3.19 and 3.20: runtime snapshots, configured-helper authority and TR2.22’s boundary.
- Core `GWZDesign.md`, lines 1–92, and relevant `GWZRequirements.md` clauses, including session/environment ownership, credential storage and REQ-120–124.
- Core `GwzTransportCredentialHelpersSurface.md`, lines 18–166: accepted timing explanations, helper-chain recovery, repair/undo and native-transport escape hatch.

Read both review objects completely: amendment lines 1–254 and spike receipt lines 1–31.

Inspected committed private evidence under `campaigns/https-integration/runs/2026-10-03-tr222-config-view/`:

- `README.md`, `FOLLOWUP.md`, `followup-receipt.json` and `initial-attempt-receipts.txt`.
- Frozen `src/main.rs` and authoritative follow-up `src/main-v3.rs`.
- The v3 raw stdout receipt reporting the successful feasibility assertions.

These distinguish the initial unavailable full logs/source snapshots from the retained repeated-root RED and corrected GREEN. The frozen prototype confirms the representative assertions attributed to it; it does not prove the product lifecycle matrix.

The lane owner expressly authorized one additional isolated, read-only native-Git inspection to resolve a suspected framing ambiguity. A Python subprocess invoked:

```text
/usr/bin/git config --no-includes --null --show-origin --show-scope --list
```

Its cwd was `/`; its complete environment was:

```text
PATH=/usr/bin:/bin
GIT_CONFIG_NOSYSTEM=1
GIT_CONFIG_GLOBAL=/dev/null
GIT_CONFIG_COUNT=1
GIT_CONFIG_KEY_0=credential.helper<LF>synthetic.ignored
GIT_CONFIG_VALUE_0=synthetic-value
```

`<LF>` denotes one actual newline. The inspection returned exit **128**, produced no expected command-scope record, and did not manufacture a helper key. Only structural booleans and the exit status were printed; stderr content was discarded. No helper or credential lookup ran.

No files were written, builds or product tests run, trust changed, or Git mutations performed. Moving product implementation and current peer reports were not read. The recorded per-commit and archive-gate passes were treated as supplied checkpoint evidence, not rerun.

## 1. Findings

None.

## 2. Invariant analysis

**Configuration scope and ordered discovery.** The amendment preserves the accepted exclusion of every conditional include while retaining legitimate unconditional includes. Discovery disables includes only to obtain root occurrences and command entries; the subsequent ordered walk restores unconditional inclusion at each directive position. Final credential fill receives the flattened view. This addresses the documented hasconfig counterexample without treating `--no-includes` as an equivalent final configuration.

Lines 77–80 explicitly preserve scope/origin occurrences rather than deduplicating paths. The authoritative frozen follow-up exercises identical SYSTEM/GLOBAL paths twice and verifies six fixture helper entries. Known local/worktree scopes fail closed. Leading Apple installation origins are admitted only within the bounded leading unknown/file run and are included in the same preparation limits.

**Environment overlays and snapshot boundaries.** Git parses PARAMETERS and COUNT overlays. Their effective entries are retained in order before their environment forms are removed from final fill. The explicit-file parse excludes those overlays, avoiding duplication. Captured HOME, original source directories and discovery cwd determine include resolution; ambient-home fallback and lossy path conversion are prohibited.

The final environment delta is concrete: controlled SYSTEM/GLOBAL/NOSYSTEM settings, removal of folded command overlays, and the existing prompt controls. Controlled discovery must establish that only the private global origin remains. Failure cannot silently restore the original broader configuration. This is a bounded amendment of the literal environment mechanism, with the runtime remaining the snapshot producer.

**Normalized byte round-trip.** Git remains the configuration parser. The writer must preserve ordered normalized names, exact subsection/value bytes, duplicate/reset order and valueless versus empty values. A same-Git reparse compares the complete entry sequence before fill. Unsupported representations and NUL fail closed, without rendering their contents.

I attacked the newline separator by proposing a command key whose subsection contained LF. The authorized isolated inspection rejected that key before producing a record. That candidate therefore does not establish a defect. The evidence supports representative escaping, non-UTF-8 values and subsection normalization; the draft correctly requires an every-input comparison beyond those examples.

**Clocks, admissions and retained ownership.** Both admissions retain the accepted allocation provenance. One interaction deadline starts after admission and covers discovery, source preparation, all parses, round-trip verification, controlled discovery and fill. No subprocess receives a fresh 120-second allowance.

Blocking filesystem work belongs to a bounded context-owned worker. Its permits remain owned through join, and an unfinished worker is retained alongside an unreaped child. A cleanup miss is explicitly neither a disposal acknowledgement nor authority to reuse its slots. Sequential children, per-source limits, cumulative preparation limits, entry count, depth and visit count make the new preparation footprint explicit. The 132-child maximum agrees with 128 source parses plus initial discovery, view verification, controlled-origin verification and fill.

**Sensitive copies and cleanup.** The proposal expressly introduces private scratch copies rather than silently extending the existing buffer rule. It specifies create-new ownership, 0700/0600 modes, zeroizing memory, retained file lifetime, best-effort overwrite/unlink and pending cleanup on failed removal. It does not claim physical secure erasure or zeroization of native Git/OS copies. This remains a mechanism requiring Safety acceptance and implementation evidence; the receipt does not claim those have been obtained.

**Refusal and recovery.** Preparation failures use Authentication/M2 rather than falsely identifying the final answer’s 16-KiB M8 limit. Unsupported discovery options fail the lookup without broad-scope fallback. Helper result/parser/retry policy remains the accepted policy. Source paths, configuration fields, values, helper output and stderr remain excluded from diagnostics.

The accepted recovery explanation identifies the actual unconditional helper chain, preserves legitimate includes and empty resets, qualifies native Git success, and supplies repair/undo and native-route choices. The mechanism does not introduce a conflicting recovery command or automatic fallback.

**Supersessions and evidence satisfiability.** The amendment identifies its environment change, preparation commands, lookup-start qualification, scratch-copy exception and spawn-inventory obligation. It preserves the accepted username/timing/fixed-cause amendment and adds no wire or public application field. The broader configured-helper and runtime-snapshot authorities remain compatible with this bounded correction.

Required integrated regressions are future obligations, not asserted executions. The spike receipt expressly excludes cancellation, quota retention, filesystem ceilings and other-platform qualification. Its incomplete initial receipts are labeled as incomplete. No physical-feasibility result is promoted to product acceptance.

## 3. Risks and next action

Initial native-Git discovery reads original roots before core can impose per-file input limits. The draft accurately limits its claim to child deadline/output bounds; native allocation and original-file reads remain a disclosed limitation. Product cancellation, worker/child retention, scratch-removal failure, byte/path handling and the full environment/include matrix still require implementation evidence. Windows and release qualification remain outside this review.

The next action is for root to record the mechanism decision after the required independent reviews. If adopted, implement this exact contract and execute its integrated regressions, including making the actual supervised-child hasconfig counterexample green.
