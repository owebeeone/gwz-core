# GwzTransportCredentialHelperConfigurationView — CONSISTENCY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportCredentialHelperConfigurationViewAmendment.md` and `GwzTransportCredentialHelperConfigurationView-Spike.md` at core `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`. Round-1 corrected DRAFT mechanism proposal and feasibility receipt, dated 2026-10-03; not implementation or release acceptance.

**Baseline:** Repositories under `/Volumes/projects/limbo/gwz-dev-tr2-22`:

| Repository | Exact SHA |
|---|---|
| Root | `054d1dddc345452a2285e74c31f5ea4c0b0d5360` |
| Core | `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19` |
| Transport | `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` |
| Private evidence | `84fef16e6b225cf6668d4bbd4fb8c4678b649723` |

Committed bytes were read with `git show EXACT_SHA:path`. All four HEADs matched at the beginning and end.

**Date:** 2026-10-03

**Axis:** Consistency against the controlling graph, original counterexamples and complete changed range. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on its current report. Filed verbatim by the lane owner.

**Verdict: GO** — both prior Safety P2 findings are closed for the corrected proposal; zero open P0/P1/P2 and one new nonblocking P3 documentation finding. No new architectural root cause was found.

---

## Prior-finding closure table

The initial Consistency review had no findings. The two findings below are from the legitimate prior-round Safety report and were accepted in the combined remediation plan.

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Safety P2-1 — relative/empty HOME anchoring | Require nonempty absolute captured HOME when `~/` is needed; otherwise M2 before include I/O. Give every other include an explicit absolute anchor. | Re-traced the original `/workspace` parent-cwd counterexample against amendment lines 100–110 and frozen v7 `resolve_include`, lines 63–70. Relative, empty and absent HOME return an error before the caller can open an include. Absolute HOME and source/command anchors are exercised at v7 lines 260–279, with distinct synthetic source, parent and child locations. The original unintended parent-cwd selection is no longer permitted. | **Closed for proposal.** Integrated no-unintended-read regressions remain owed. |
| Safety P2-2 — sensitive scratch orphaned after process death | Eliminate named preparation copies using Git stdin parsing and process-lifetime configuration parameters. | Re-traced termination after source/view preparation against amendment lines 87–98, 119–154 and 198–208. Neither stage creates a named source copy or view, so termination cannot orphan the former files. Frozen v7 source parsing uses stdin; controlled configuration uses environment parameters. Native preparation/fill group kill/reap and absence of named preparation copies are asserted at v7 lines 285–299. Fixture configuration files model existing inputs rather than product-owned preparation artifacts. | **Closed for proposal.** Product worker/child lifecycle qualification remains owed. |

Closure here is earned by retracing the corrected contract and inspecting its frozen physical evidence. No new experiment or product test was run.

## Changed-range analysis

Compared the complete committed review objects at core `63abcb8afa89750eb19b93bf1614fd36fc2d292e` and `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`.

The amendment changes:

- Lines 20–34 and 87–98 replace private-copy parsing with bounded stdin parsing.
- Lines 100–110 define the supported HOME domain and absolute include anchors.
- Lines 119–154 replace the native-file writer with ordered `GIT_CONFIG_PARAMETERS` encoding, `/dev/null` global suppression, exact command-origin verification and explicit environment-size refusal.
- Lines 158–208 retain the single deadline and worker/child ownership while replacing scratch cleanup with process-lifetime buffer ownership.
- Lines 212–226 update the mechanism owner and reduce the maximum sequential child count from 132 to 131.
- Lines 231–257 update supersessions and required regressions for the corrected carrier, HOME refusal, inheritance, process death and environment-size failure.

The spike receipt identifies v7 as the current primitive and adds its HOME, inheritance, E2BIG and kill/reap evidence. It still explicitly excludes product lifecycle and platform qualification.

All five named core controlling documents and both root process authorities have unchanged committed bytes relative to the initial review. The inspected original private artifacts—including initial and v3 sources, v3 logs, initial-attempt receipts and follow-up records—also remain unchanged.

**Classification:** This is a material change to the configuration carrier and filesystem mutation boundary, not an editorial correction. It removes the proposed persistent mutation and changes the child environment. The current full changed-range independent review is therefore necessary. It introduces no new transport field, public application schema or protected-library interface.

The environment-size outcome and its no-latch rule are consequential additions within the accepted remediation’s requested size-refusal disposition. Their incomplete supersession trail is the new **non-architectural** P3-1 below. No **NEW ARCHITECTURAL** root cause was identified.

## 0. Evidence base

Read:

- Canonical `ReReviewConsistency-1.md`.
- Corrected root and core standing instructions and root checkpoint lines 1–62.
- Committed combined `GwzTransportCredentialHelperConfigurationView-RemPlan.md`, lines 1–29.
- Both initial complete Consistency and Safety reports. No current peer re-review report was accessed.
- Corrected amendment, lines 1–263, and spike receipt, lines 1–35, completely.
- The complete old/new bytes of both review objects for changed-range comparison.
- Named controlling documents: helper design revision 4, accepted helper-context amendment, release-plan amendment 2 revision 6, `GWZDesign.md` and `GWZRequirements.md`. Relevant accepted outcome and surface clauses were reread directly, including helper-design lines 98–119 and 288–321.
- Root `AgentProcessRules.md` and `GwzProcessOptimization.md`, compared with their initial-review committed versions.

Inspected private evidence at `84fef16e6b225cf6668d4bbd4fb8c4678b649723` under `campaigns/https-integration/runs/2026-10-03-tr222-config-view/`:

- Complete authoritative frozen `src/main-v7.rs`, lines 1–308.
- `REMEDIATION.md`, `remediation-receipt.json`, v7 raw stdout/stderr, and v4–v6 raw stdout/stderr.
- Historical README and the previously reviewed artifacts for preservation checks.

The remediation receipt records all numbered runs as exit 0. V7’s raw stdout reports successful stdin parsing, ordered parameter round-trip, real fill selecting A, HOME refusal/anchors, helper inheritance, OS E2BIG refusal and killed preparation/fill groups. Its stderr records normal compilation/execution without the predecessor warning.

The frozen source supports those representative physical assertions. It does not implement the proposed production admission, concurrent pipe supervision, retained-worker or full lifecycle gates. The public receipt correctly says those remain unqualified.

No writes, builds, tests, experiments, trust changes or Git mutations were performed. The initial review’s narrowly authorized Git inspection was not repeated. Corrected boundary/archive gate passes were treated as supplied checkpoint evidence.

## 1. Findings

### [P3-1] The environment-size spawn exception is missing from the precise outcome supersessions

**Location:** Corrected amendment lines 46–50, 147–151 and 231–241. Controlling helper design revision 4: §4 lines 107 and 116; §11 lines 294–306, 313 and 318.

**Violated invariant:** An amendment must identify the accepted clauses it changes and keep its preservation claims consistent with those changes.

The corrected amendment expressly assigns `ArgumentListTooLong` during controlled verification or fill to **M2**, without a missing-Git latch. Its precise supersessions enumerate the environment delta, preparation commands, lookup clock and buffer exception, but do not identify the accepted M1 outcome exception.

The accepted §4 row covers “a `git` that is found but cannot start,” assigns `Unavailable`/M1 and latches it. Accepted §11 assigns M1 `external_tool_missing` and a failed clone member; M2 instead maps to `remote_rejected` and skips a `private: true` member. The amendment also claims to preserve “all helper result/parser/retry rules.”

**Credible sequence:**

1. Controlled verification or fill attempts to spawn a resolved Git executable.
2. The OS refuses execution with `ArgumentListTooLong`, as the retained physical probe demonstrates.
3. Amendment lines 149–150 require M2 and no missing-Git latch.
4. An implementer following the purportedly preserved §4 outcome table instead selects M1, latches subsequent lookups and reports `external_tool_missing`.
5. The classification also changes whether a private clone member is skipped or fails.

The specific new paragraph supplies the intended rule, so this is a bounded documentation inconsistency rather than an unresolved mechanism decision.

**Impact:** The claimed exact supersession trail omits an observable code, latch and clone-treatment exception. A reader carrying forward the accepted generic spawn-failure mapping can implement the wrong outcome.

**Required correction:** Add a precise supersession for the §4 M1 row/latch and corresponding §11 M1 wording, narrowly excluding `ArgumentListTooLong` from controlled verification/fill. State its intended Authentication/M2, no-latch and existing M2 member treatment. Qualify the blanket result-rule preservation claim accordingly. Preserve ordinary missing/unexecutable-Git M1 behavior.

**Closure/regression test:** Trace the corrected clauses for the same E2BIG sequence. The integrated environment-size regression should assert M2/Authentication, no helper execution, no missing-Git latch on a subsequent lookup, and the existing M2 fetch/push and private-clone treatment. Keep a missing or unexecutable Git control selecting M1.

**Classification:** New non-architectural documentation root cause. No new field, error code or mechanism redesign is required.

## 2. Invariant analysis

**Ordered configuration and unconditional inclusion.** Discovery framing and repeated scope/origin occurrences remain unchanged. Includes are expanded at their directive positions; all conditional include paths are ignored and no include directive enters the final parameter view. Existing system/XDG/global and environment overlays retain their effective order before flattening. V7 retains the representative repeated-root and normalization assertions.

**Parameter encoding and verification.** The corrected encoder distinguishes a valueless key from an empty value, quotes key/value bytes separately and escapes apostrophes using Git’s syntax. Native-Git command-origin discovery must reproduce the complete ordered name/optional-value sequence. Unexpected sources, failed parsing or mismatched bytes refuse before fill. The v7 comparison at lines 243–254 verifies this representative mechanism rather than merely asserting that a child started.

**Source suppression and inheritance.** `/dev/null`, NOSYSTEM and replacement PARAMETERS suppress the original file roots while COUNT and numbered overlays are removed after folding. Controlled verification admits only command-line entries. Helper A queries an inherited synthetic configuration value and checks that conditional directives are absent. The proposal explicitly identifies the environment delta and same-user/privileged observation limitation.

**Original scope and ownership counterexamples.** HOME now has a defined refusal domain before worker I/O. Named sensitive preparation files no longer exist, eliminating the process-death orphan state rather than attempting to recover it through an unproved sweep. Original user inputs remain readable files; that does not recreate the removed product-owned scratch mutation.

**Clocks and bounded cleanup.** Both admissions still precede preparation. One interaction deadline covers discovery, stdin parses, parameter preparation, verification and fill. A retained worker or child holds both quotas; cleanup-pending does not authorize slot reuse. The new 131-child maximum matches 128 source parses, initial discovery, combined verification and fill.

**Diagnostics and evidence boundaries.** Environment/configuration bytes, source paths, stderr and answers remain excluded from diagnostics. OS-size refusal cannot truncate configuration, fall back to a named file or silently widen scope. Apart from P3-1’s supersession omission, the outcome is explicit. The receipt separates primitive kill/reap evidence from unexecuted product cancellation, quota and cleanup qualification.

## 3. Risks and next action

Initial discovery still has no cap on native Git’s original-file reads or internal allocation. Command/OS/native environment copies have ordinary lifetimes and can be observed through same-user or privileged facilities. The corrected documents disclose those limits without claiming physical zeroization or cross-platform qualification.

Product pipe concurrency, worker retention, cancellation, environment-size classification and the complete integrated matrix remain implementation obligations. The real product hasconfig regression remains RED, as recorded.

The next action is the bounded P3-1 supersession correction. Root may then record adoption only after the required independent review dispositions; this Consistency GO does not accept the working implementation or qualify release.
