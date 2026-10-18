# GwzTransportCredentialHelperConfigurationView — SAFETY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportCredentialHelperConfigurationViewAmendment.md` and `GwzTransportCredentialHelperConfigurationView-Spike.md` at core `63abcb8afa89750eb19b93bf1614fd36fc2d292e`; DRAFT mechanism proposal, dated 2026-10-03. Implementation is excluded.

**Baseline:** `/Volumes/projects/limbo/gwz-dev-tr2-22`: root `d13bdebdbf0ce9291e99510a1d5a36750371755c`; core `63abcb8afa89750eb19b93bf1614fd36fc2d292e`; transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`; private evidence `9ef2a6adbc19cfb8a038e7b841d954c0c9d3b248`. Documents and frozen feasibility source were read with `git show EXACT_SHA:path`, with `nl`, `sed` and `rg` for inspection. All four HEADs matched at the beginning and end.

**Date:** 2026-10-03

**Axis:** Safety — what the text permits to go wrong under scope confusion, interruption, disclosure and retained cleanup. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — two P2 findings block. No P0 or P1 finding. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.

---

## 0. Evidence base

The review inspected these committed inputs:

- Root and core standing instructions: `AGENTS.md`, root `AGENTS_GWZ.md`, and core `AGENTS.md`.
- Root `dev-docs/AgentProcessRules.md`: authority and amendment rules; L1-13/L1-15 at lines 319–355; independent review, severity and closure rules at lines 369–461.
- Root `dev-docs/GwzProcessOptimization.md`: preserved ownership/recovery requirements, two-track physical feasibility policy, review rules and the 2026-09-28 ruling.
- Root `dev-docs/CurrentProgramCheckpoint.md`, lines 1–39: this proposal’s settled tuple, exclusion of unfinished implementation, and separation from the accepted helper-context contract.
- Core `GwzTransportCredentialHelperConfigurationViewAmendment.md`, lines 1–254: the complete controlling review object.
- Core `GwzTransportCredentialHelperConfigurationView-Spike.md`, lines 1–31: the complete feasibility receipt.
- Core `GwzTransportCredentialHelpersDesign.md`, revision 4: status and decisions; §3 environment, command, bounds and secrets; §§4–8 outcomes, credential ownership, pooling, retry and parity; §§9–11 supersessions, regression inventory and recovery messages.
- Core `GwzTransportCredentialHelperTimingAmendment.md`: accepted status; captured timing and admission rules; scope-aware recovery; bounded diagnostics and exact supersessions.
- Core `GwzTransportReleasePlanAmendment-2.md`, revision 6: status, §3.19 helper authority, §3.20 TR2.22 and relevant integration/release boundaries.
- Core `GWZDesign.md` and `GWZRequirements.md`: accepted helper-context clauses, session/environment ownership, credential privacy and operation isolation.
- Private evidence directory `campaigns/https-integration/runs/2026-10-03-tr222-config-view/`: `README.md`, `FOLLOWUP.md`, `receipt.json`, `followup-receipt.json`, complete frozen `src/main-v3.rs`, and the v3 raw stdout/stderr.

The frozen v3 receipt reports exit status 0. Its stdout records a successful native-Git discovery → bounded private-copy parsing → unconditional walk → flattened view → credential-fill chain, including repeated SYSTEM/GLOBAL occurrences. Its source supports the stated representative feasibility assertions.

No test, build, experiment, trust action, write or Git mutation was performed. The counterexamples below are specification-level state sequences, not newly executed evidence. Moving working sources and current peer reports were not inspected.

The final tuple check returned the same four exact SHAs as the initial check.

## 1. Findings

### [P2-1] Captured HOME expansion lacks a fixed interpretation for relative or empty values

**Location:** Configuration-view amendment, lines 99–107; contrast the explicit discovery-root anchoring at lines 62–67 and command-scope anchoring at line 104. The frozen feasibility runner’s `walk` at `src/main-v3.rs:49–51` uses `home.join(rest)`; its fixture HOME is absolute at lines 109–112.

**Violated invariant:** Unconditional includes must preserve the configuration selected under the captured environment and Git’s fixed cwd `/`. Core’s worker must not introduce the parent process’s working directory as another configuration source. The proposal promises no different helper selection for unsupported path forms.

**Credible sequence:**

1. Run the parent from `/workspace`, with captured `HOME=tmp/config-home` and an explicit absolute `GIT_CONFIG_GLOBAL=/tmp/root-config`.
2. The root contains `include.path=~/selection`.
3. `/tmp/config-home/selection` selects synthetic helper A. `/workspace/tmp/config-home/selection` selects synthetic helper B.
4. Git running with cwd `/` resolves its relative HOME expansion in that fixed directory.
5. The draft requires expansion “only against captured HOME,” but neither requires HOME to be absolute nor specifies how its relative or empty result is anchored. A worker following the frozen prototype’s `home.join(rest)` obtains a relative path and opens it from the parent’s cwd, selecting B.
6. Applying the source-relative rule after expansion produces another interpretation. The contract does not select between these outcomes or require refusal.

The existing absolute-root rule does not resolve this: it governs origins returned by discovery, while this path is newly constructed during the unconditional include walk.

**Impact:** A legitimate unconditional include can select a different helper, or unexpectedly read a file beneath the parent’s workspace. This breaks the fixed-directory configuration boundary. The absolute-HOME feasibility case does not close it.

**Required correction:** Specify the HOME domain and expansion algorithm. Either resolve supported relative HOME values against discovery cwd `/`, including an explicit empty-HOME rule, or fail closed as M2 when `~/` requires an unsupported HOME form. Every resulting source path must have a fixed anchor before worker I/O; parent cwd must never participate. Preserve byte paths and the existing absent-HOME refusal.

**Closure/regression test:** Use an absolute root config, a parent cwd other than `/`, and distinct sentinel include files at the child-cwd, parent-cwd and source-directory interpretations. Cover absolute, relative, empty and absent HOME, nested includes and command-scope includes. Assert the specified selection or M2 refusal, and assert that neither unintended file is opened. Compare synthetic answers without printing them.

### [P2-2] Persistent sensitive scratch has no recovery owner after process termination

**Location:** Configuration-view amendment, lines 189–200, 223–230 and 243–245. Process authority: `AgentProcessRules.md` L1-13/L1-15, lines 319–355, preserved by `GwzProcessOptimization.md` §1.

**Violated invariant:** A new filesystem mutation must have an owner and defined interruption/recovery observations. The proposed private-copy exception creates persistent sensitive files, but its cleanup authority exists only in the live context. Pending cleanup must remain owned rather than become an unclassified orphan.

**Credible sequence:**

1. A lookup writes a 0600 source copy or flattened view inside its 0700 scratch directory. A Git configuration can contain a synthetic secret in a helper command or another configuration value.
2. Kill the owning process after the write and before disposal, or terminate it after scratch removal failed and cleanup became pending.
3. No destructor or context cleanup executes. A spawned child may also remain alive, so immediate deletion by another process cannot simply be assumed safe.
4. The next runtime creates a new unique directory. The draft defines no durable ownership marker, orphan classifier, cleanup handoff, restart sweep, retention bound or refusal rule for the previous directory.

All stated live-process cancellation and timeout rules can hold while this sequence leaves sensitive copies indefinitely. Repeating it also accumulates scratch beyond the per-lookup ceilings.

**Impact:** The mechanism introduces persistent readable copies that outlive the operation and its owner, with no defined recovery path. Restrictive modes reduce access; they do not complete the lifecycle. This finding does not allege cross-user disclosure or demand physical secure erasure.

**Required correction:** Complete the scratch lifecycle across process death. Use an OS-lifetime mechanism that prevents named sensitive leftovers, or specify persisted ownership and safe recovery under the runtime scratch parent. Recovery must distinguish live child ownership, owned orphans and foreign/ambiguous paths; must not follow substituted paths; and must bound unresolved retention or refuse further preparation. Explicitly define the interrupted-copy exception and recovery rule rather than relying on best-effort Drop cleanup.

**Closure/regression test:** Fault a disposable product runner after directory creation, during source/view writing, during final fill and after an injected removal failure. Terminate the process without running destructors, then exercise recovery. Assert that owned orphan files are removed or remain explicitly bounded and refused; live child files and foreign/substituted paths are preserved; no secret/path content enters diagnostics; and repeated interruptions cannot accumulate unlimited unowned copies. Keep secure-erasure claims excluded.

## 2. Invariant analysis

The following attacks did not produce additional findings:

- **Conditional include reactivation:** The walker ignores every normalized `includeif.<condition>.path`, removes include directives, and retains unconditional includes at their directive position. Command-scope conditions are included in the required regression inventory. Final helpers inherit the flattened source.
- **Repeated roots and overlay ordering:** Root occurrences retain scope/origin runs without physical or lexical deduplication. The frozen v3 source and receipt demonstrate two visits when SYSTEM and GLOBAL name the same file. Git decodes PARAMETERS/COUNT overlays; their original environment forms are removed before fill.
- **Normalization and serialization drift:** Git parses source grammar. The writer must preserve ordered normalized names and optional byte values, then obtain exact ordered equality through another Git parse before fill. Unsupported or unrepresentable cases fail closed. This is stronger than relying on representative writer examples alone.
- **Ambient source resurrection:** The final environment suppresses original system/XDG/global sources and overlays. Controlled discovery must report only the private global origin, or no entries for an empty view. Unexpected installation or other origins refuse the lookup.
- **Admission, deadline reset and worker abandonment:** Both admissions precede preparation. One interaction deadline covers every stage, and only one child is active at a time. An unfinished worker or unreaped child retains both quotas; a 500-ms miss is pending cleanup rather than permission to reuse slots. These rules defeat the live-context abandonment attack.
- **Special files and executor blocking:** The proposal requires nonblocking opens, opened-handle regular-file checks, bounded reads and a context-owned worker. Depth and visit ceilings bound cycles. An uninterruptible read retains its permits instead of blocking the Tokio executor or falsely acknowledging disposal.
- **Disclosure through diagnostics:** Configuration paths, fields, values, stderr and helper output are excluded from Failure, events and logs. M2 preparation refusal does not misuse M8’s credential-answer limit cause.
- **Degraded Git and mixed-version behavior:** Missing discovery support refuses rather than falling back to broader scope. No new wire field or public API is introduced. Existing native transport remains an escape hatch.
- **Environment versus file snapshot:** The draft freezes the captured environment and prepared final view. It does not promise an atomic snapshot of all original configuration files. Ordinary changes between discovery and source reads therefore were not treated as a violation of an unstated atomicity guarantee.
- **Authority and deferred scope:** The proposal remains DRAFT, identifies its bounded supersessions and keeps implementation, Windows mechanisms, release qualification and platform/performance work separate. The feasibility receipt explicitly declines to qualify cancellation, quotas, retained cleanup or additional platforms.

Initial native discovery’s original-file reads and internal allocation remain uncapped. The draft discloses that limitation rather than claiming the later private-copy bounds cover it. I found no separate invariant violation from that disclosed limitation within this object.

## 3. Risks and next action

Native-Git parsing and OS/library copies retain ordinary lifetimes. Private file modes and best-effort overwrite do not establish physical secure erasure. The feasibility evidence is macOS-only and representative; product ownership, cancellation and secret-lifetime qualification remain future gates, as the receipt states.

The next action is one bounded documentation remediation addressing P2-1 and P2-2, followed by same-reviewer closure against a newly settled tuple. Configuration-view implementation remains gated on acceptance.
