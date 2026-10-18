# Credential helpers — STATE-AXIS ROUND-2 CLOSURE REVIEW

**Review object:** Bounded credential remediation round 2 in `/Volumes/projects/limbo/gwz-dev-tr2-22`, frozen at the tuple below. Controlling document: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan-2.md`. Source acceptance remains pending.

**Baseline:**

| Repository | Reviewed HEAD |
|---|---|
| root | `b35887e579cbf8d42e42ae848038f408f51ddecb` |
| gwz-core | `ec43f585f7c768afa0fe71e23a7e503cfda9150c` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

All six heads matched at the beginning and end. No tracked dirt was present at the final check. Numbered working-source reads were bound to committed bytes through the receipt comparison.

**Date:** 2026-10-03

**Axis:** State machines, races, fail-closed admission, ownership and physical cleanup. Independent, adversarial, read-only continuation by the reviewer who raised round-1 State P2-1. No current-round peer report was read or used.

**Verdict: GO** — round-1 State **P2-1 is closed**. The corrected finalizer retains cleanup ownership through the last refusal decision, terminates the process group on refusal, and cannot manufacture a later ownerless refusal after successful retirement. No new P0, P1, P2 or P3 finding is established. The six original blocking root causes remain closed.

---

## 0. Evidence base

I read the complete canonical `PromptState-2.txt`, the controlling RemPlan-2, the newest credential checkpoint sections, my filed round-1 State report, and the round-2 remediation record. The round-1 authority analysis remains applicable: adopted design and requirements, credential-helper lifecycle and outcome requirements, timing, configuration-view and SSH-helper-clock amendments, and process review rules. The remediation record was treated as evidence rather than authority.

The bounded source review covered:

- The complete production delta in `https_auth/runner.rs`.
- The three new boundary regressions in the existing `runner/tests.rs`, with the retained completed-output and parsing regressions.
- The unchanged `HelperJob` completion, termination and Drop paths in `owner.rs`.
- Lookup admission, Runner construction, configuration-preparation callers and `run_secret` result flow.
- The changed remediation record and the absence of production changes to the previously closed publication, representation, policy and failure-carrier mechanisms.

No builds, tests, probes, source edits or other mutations were performed.

The final receipt is:

`/Volumes/projects/limbo/gwz-tr222-round2-final-source-receipt-20261003.json`

SHA-256:

`3d8d7ef0b898d5139a9529cf4a60410070423288c69741362b6cc5b7883084a1`

All three owned paths matched committed `git show HEAD:` bytes, sizes and recorded hashes:

| Owned path | SHA-256 |
|---|---|
| `src/git/endpoint/https_auth/runner.rs` | `8ba02ba6e72f44e78e78a452080c9969f57e13a655ee07ab7a39999d3a965532` |
| `src/git/endpoint/https_auth/runner/tests.rs` | `7449506d6e83f710a6ac1e568af0cada815f8a1f10b3db74c4f81ca15ae7bee6` |
| `dev-docs/GwzTransportCredentialHelpersRemediationRecord.md` | `4a09ec83d062b38e0583b8ef68b241dad5ce141ad78a2d398ebf72ec373d1030` |

All eight listed logs matched their hashes. The RED-source receipt, external gate runner, root boundary log and reused round-1 receipt also matched their recorded hashes. Precommit HEAD fields in receipts were not substituted for the reviewed tuple.

Recorded results inspected:

| Evidence | Result |
|---|---|
| Original-behavior boundary RED | 0 passed, 3 failed; both refused descendants continued writing |
| Corrected focused runner/helper tests | 17 passed, 0 failed |
| Corrected affected credential union | 458 passed, 0 failed, 4 existing ignores |
| Core Clippy | Exit 0, 49 retained warnings |
| Conditional-compilation guard | Exit 0; nothing new |
| Candidate-switch guard | Exit 0; inventory matched |
| Process-global guard | Exit 0; nothing new |
| Root checked-artifact boundary | Passed, unchanged checker |
| Exact core commit gate | `lane gate: ok at ec43f585f7c768afa0fe71e23a7e503cfda9150c` |

The focused command contains a nonmatching `ownership_tests` selector. Its actual 17 passing tests are identified in the log; retained ownership and reap regressions also pass in the affected union. No additional ownership coverage is inferred from the selector.

The first fixture compilation failure is retained separately and is not behavioral RED evidence. The valid RED run’s command was recorded retrospectively without a captured process envelope; the final gates have captured envelopes. These limitations do not replace the source proof or the physical heartbeat observations.

## 1. Findings

**None.**

Round-1 State P2-1 is closed by the bounded correction and regression evidence described below.

## 2. Invariant analysis

### Final refusal retains its cleanup owner

The original counterexample was:

1. A helper leader produced a valid answer and exited.
2. A live descendant in the same process group closed inherited output pipes and continued independent heartbeat writes.
3. Successful cleanup discarded the job’s child and process-group capability.
4. A later deadline or cancellation check refused the answer.
5. Drop had no remaining child, so the refused descendant survived without tracked cleanup ownership.

The corrected path in `runner.rs:152–188` removes step 4 after retirement. Completed output and parsing checks remain before the production call to `finish_job`. Within that finalizer, the last admission decision runs at line 181 while the `HelperJob` still owns its child, process-group identifier and permit reference.

A refusal takes `job.terminate().await` at line 185. The unchanged owner path first sends process-group SIGKILL, then kills/waits for the child. Successful child cleanup retires the job only afterward. Cleanup timeout or wait failure remains `CleanupPending`; the job retains its fields for the existing Drop and pending-reap ownership path.

An accepted result takes `complete_if_exited` and returns that admitted result directly. There is no deadline or cancellation check after retirement that can recreate the original ownerless refusal.

The success branch contains no asynchronous suspension between its admission decision and retirement. Cancellation or expiry after that decision does not retroactively refuse an already admitted completion. This is the explicit admission point required by RemPlan-2.

### Equality and cancellation exercise the production boundary

The fixture in `runner/tests.rs:162–280` recreates the specific adverse state: the leader has exited successfully, stdout and stderr have reached EOF, a valid answer has been parsed, and a same-group descendant has already written independently at least twice.

The fixture calls the same private `finish_job` used by production. It verifies that the Job’s permit reference is still present at the decision and that both host and endpoint admissions remain charged. The private clock seam supplies exact equality at the existing comparison position; production still samples `Instant::now` there. Cancellation uses the real token and unchanged `check`.

The refusal cases assert Timeout or Cancelled, no admitted answer, restored admission capacity after cleanup, no retained pending child, and cessation of independent heartbeat writes after the cleanup grace. The fixture’s emergency group guard remains alive until after those assertions, so its Drop cannot make the heartbeat assertion pass.

The valid RED log records continued heartbeat growth after refusal:

- Deadline equality: 29 to 34 bytes.
- Cancellation: 30 to 35 bytes.

Both cases pass in the corrected focused and affected runs. The normal-success regression also passes and proves that cancellation after final admission cannot turn accepted completion into a refusal. Its original-behavior RED failure exposed the already-retired Job reference at the final decision.

These regressions establish process-group termination and independent-write cessation. They do not claim native grandchild reaping.

### Existing result checks and retained cleanup remain intact

`run_finished` still checks before spawn, supervises completion against cancellation, timeout and output overflow, and checks successful output before parsing. `parse_answer` still checks before and after parsing. The finalizer supplies the remaining admission check while cleanup ownership exists.

The lookup still constructs one Runner with the captured interaction deadline and both admissions. Configuration preparation uses that Runner; the credential lookup continues through `run_secret` and its supervised parsing. No fresh deadline, allowance or helper-policy decision is introduced.

If the future is dropped before final admission or during refusal cleanup, the unchanged `HelperJob::drop` still owns the group and child. It kills the group and either releases completed ownership or transfers child and permits to the owner’s pending queue. The affected log includes passing abort/reap ownership regressions.

Secret output, parsing and wiping mechanisms are unchanged. Refused generic results are discarded through their existing owners; this correction adds no credential cache or Authorization construction.

## 3. Risks and next action

This GO permits the lane owner to record closure and proceed with the authorized integration and actual combined MAIN validation. It is not a merge result or release acceptance.

Earlier full suites remain historical. The unchanged transport, CLI, Python and help evidence is reused from round 1; it was not rerun or relabeled as new round-2 evidence. The 49 core warnings and four existing ignores remain disclosed and unwaived.

Windows, provider/trust, performance, selected-source, packaging, release and supplied-carrier outcomes remain deferred. Native Git/OS copies and native initial-discovery limitations remain unchanged.

## Prior-finding closure table

The original six blocking root causes were independently verified in round 1. Their proof surfaces are unchanged except for the helper finalizer, which was re-traced here.

| Finding | Round-2 verification | Status |
|---|---|---|
| Original Code P2-1 / State P2-1: detailed terminal publication race | Atomic winning-terminal admission and the core publication reservation are unchanged. No runner change affects association or observer ordering. | Closed |
| Original Code P2-2: Negotiate lost beyond diagnostic capacity | Bounded challenge retention, carriage and private-materialization classification are unchanged. | Closed |
| Original State P2-2: populated username allocation growth | Reserved terminator capacity, stable SSH conversion and Basic exclusion of the terminator are unchanged. | Closed |
| Original State P2-3: late completed or parsed answer admission | Completed-output and parsing checks remain intact. Final admission now also retains cleanup ownership. | Closed |
| Original State P2-4: disabled helper policy and pool reuse | Per-operation policy carriage, isolated pool identity and selected-key authentication handling are unchanged. | Closed |
| Original State P2-5: lost Closed Failure and close facts | Raw retained Failure ownership and separate derived close facts are unchanged. | Closed |
| Round-1 State P2-1: refusal after cleanup retirement | Final refusal precedes retirement; refusal terminates the group. Exact equality and cancellation regressions stop closed-pipe descendants’ independent writes. Accepted completion has no later refusal check. | Closed |
| Surface P3-1 and P3-2 | Conventional-file recipes and elapsed/helper-phase documentation are unaffected by this correction. Round-1 textual proof is retained. | Closed |

## Changed-range analysis

The reviewed core range is:

`64ec039089b6e217625d7784b106d917452963cb..ec43f585f7c768afa0fe71e23a7e503cfda9150c`

The only production change is in the existing runner: private clock sampling is factored without moving its comparison, and one private finalizer makes admission before Job retirement. The existing runner test file gains three focused boundary regressions. The remediation record and root-owned review/plan documents record the correction and evidence.

No public or shared API, protocol, policy, platform assumption, dependency or source-loading edge is added. The accepted cleanup owner and lifecycle suffice. The correction is **non-architectural** and remains within the bounded second remediation round.

There are **zero new architectural root causes**, **zero new non-architectural findings**, and **no open blocking findings** established by this State review. All six heads remained at the specified tuple through the final check.
