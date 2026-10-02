# Credential helper implementation — remediation round 1

2026-10-03. Status: **NO-GO** at reviewed tuple root
`dc2b4af36e6ad6eca4a781611fcf76f6d9e76226`, core
`f97ff21fa56c2fe4abec6a73e90871dbe2d9d185`, transport
`41a16b2713b302c3675f081584d392afeae26ad5`, Python
`947ed579abec292a23db3db7394923e70ee4363e`, CLI
`1543a3bec00cda913a02c266e89d841eeeafb55b`, evidence
`662d89828b478a2acce8c0308834db7d17c872f7`.

Code NO-GO (two P2), State NO-GO (five P2), Surface GO (two P3).
Both Code and State independently identify the same first-failure publication
race. This is the strongest convergence in this implementation gate. Reviewers
classify every blocking cause as non-architectural: six distinct causes, zero
architectural causes. Reports are filed verbatim beside this document.
Earlier design/amendment GO does not accept these source defects.

## One combined correction

One drafter produces one settled correction, no Git mutations. Root owns report
filing, settlement, exact-commit gates, reviewer continuation, acceptance and GWZ
integration. No merge until all three required axes report GO. Two remediation
rounds are available for this implementation object; the original mechanism's
completed review remains separate. Implementer does not self-close findings.

| Findings | Disposition | Required closure evidence |
| --- | --- | --- |
| Code P2-1; State P2-1 | Fix the association of sanitized detailed failure with its exact winning clock record before any observer can cache a fallback. Preserve earlier expiry/cancellation, immutable first cause, no nested authority/context locks, callbacks outside locks. Postponing notification alone is insufficient. | Deterministic immediate observer at publication, zero allocation and fixed helper cause; earliest and later failure equal; M10/code75 exact zero; no child/permit; unadmitted detail cannot replace earlier network/cancel terminal. |
| Code P2-2 | Preserve classifier-critical Negotiate within bounded four-token projection, including carried/encoded failures; no schema or host-only workaround. | Negotiate fifth/later, mixed case/duplicates/reordering; Basic beyond diagnostic capacity still works; loud private clone GitCommandFailed, no helper or realm leakage, carrier round-trip. |
| State P2-2 | Size SSH username representation before copying secrets. Never release a populated credential allocation through growth; preserve HTTPS semantics and repeated conversion. | Tight nonempty username allocation observation/deallocation-safe seam; demonstrate no unwiped old allocation; repeated SSH conversion and HTTPS Basic unchanged. No unsafe test mechanism solely to inspect freed memory. |
| State P2-3 | Recheck cancellation and the same unchanged interaction deadline after child completion and after final parsing, before successful HTTPS answer admission. Preserve wiping and physical cleanup/permit ownership. | Hold polling until work and timer/cancel both ready; equality and parsing across deadline; no cached credential/Authorization; cleanup. Biased select alone is insufficient. |
| State P2-4 | Carry existing backend operation helper enablement through private local SSH opening and enforce it before helper work. Isolate enabled/disabled operations sharing endpoint/pool; no global mutation or new user policy. | Helper-enabled endpoint plus disabled backend: no lookup/password offer; enabled operation still succeeds; both operation orders and pooled reuse; existing key/agent outcome retained. |
| State P2-5 | Retain admitted Closed.failure by ownership as first terminal Failure, preserving Closed.facts, validation-before-retention and late/duplicate immutability. | Real StreamMachine close handshake with helper detail/retry; exact retained failure and facts/error; invalid retains nothing; late duplicates cannot replace; RPC typed consumer callback. |
| Surface P3-1 | Fold explicit conventional global-file set/unset recipes into Python README and all shared help surfaces, explain GIT_CONFIG_GLOBAL mismatch. Runtime selection unchanged. | Regenerated seven help artifacts and source provenance; original alternate-GIT_CONFIG_GLOBAL textual walkthrough uses effective file for both set/remove. |
| Surface P3-2 | Qualify six CLI help examples as network-clock time, name admitted helper allocation/interaction as separate local phases and point to code75 recovery. Runtime timeout/defaults unchanged. | Regenerated six network help artifacts with candidate availability; explain allowed helper wait beyond network example and locate recovery guidance. |

TDD for product defects: record original RED evidence first, then corrected
GREEN focused gates. Refresh the affected union, transport/doctest and necessary
source/generator guards, record exact argv/cwd/toolchain/env and source/log hashes.
Keep ordinary/core full candidate predecessor results labeled pre-correction;
combined MAIN validation follows accepted merge. No fictitious test probes or
passes. Rust1.95, retained debug/incremental profiles. Explicit conditional and
control scopes apply to modified code including disabled branches. Follow split
skill where new/modified cohesive owners become crowded; no unrelated refactor.

If a correction would require a new shared protocol/architecture, policy owner,
compatibility rule or platform assumption beyond this plan, stop and report
before drafting it. Reviewers determine whether the changed range invalidates
prior proof and requires fresh reviewers. No Windows trust/provider changes,
publication, push, tag, alpha installation, supplied-carrier expansion or
platform/performance/selected-source release claim is authorized here.

## Next verdict

The exact-winner regression exposed a final observe/terminate race when an
independent resource terminal has identical scalar fields. Root approves one
transport-neutral atomic `terminate_if_alive` result through the existing
clock mutex: Ok(newly admitted) versus Err(existing), preserving the old
terminate API and all expiry/cancellation/phase/lock policy. No new wire field
or helper policy enters transport. The core publication owner associates detail
only with an admitted result and resolves its pending association on unwind.

This additive shared API changes the proof surface. Review-loop §5.5 requires:
"Start a new numbered round with fresh reviewers instead, when remediation
changed a shared interface". Therefore the corrected tuple receives fresh
Code/State reviewers for remediation round1, with all original reports and this
merged plan. They inspect the changed clock/consumer seam and re-trace original
counterexamples with closure tables. Original reporters may cheaply verify their
own sequences where needed. Surface continues its original reviewer, unchanged
mechanism, and verifies its two help corrections. This is not another design
object or a reset of the implementation remediation cap.

Before settlement, root composed accepted MAIN Python a077 with generated75
947ed through GWZ member merge `merge_op_20095_1790977756851_0001`, producing
Py34398b6ace772a5d85191273086a17363a404a4d. The prior help WIP was preserved
in coordinated stash_unix_ms_1790977722088 and hashed external backup; root
reapplied only the P3 README/shared-help recipe correction. Accepted parser,
settings and driver sources remain unchanged. Final review/receipts use this
combined source, not an old lane help approximation.

Settle the revised exact tuple, re-trace original counterexamples, provide
closure tables and classify any new architectural cause.
GO from every required axis permits the already-authorized GWZ member merge,
then fresh combined CLI/core/Python integration gates. Source acceptance and
macOS integration do not imply Windows or release GO.
