# TR2.22 helper context amendment — SAFETY-AXIS REVIEW

**Review object:** Committed `gwz-core/dev-docs/GwzTransportCredentialHelperTimingAmendment.md` and caller note `GwzTransportCredentialHelpersSurface.md` at core `0adb093a99f4eb8e9b8818b767514a842533a193`. DRAFT, not implementation acceptance; dated 2026-10-03.

**Baseline:** root `bb44a7214eac779095352224e8940df268fd3f96`; gwz-core `0adb093a99f4eb8e9b8818b767514a842533a193`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Documents were read using `git show <pinned SHA>:<path>`. Accepted producer sources were read at core `26922a1cfd823a4894e09be9aeb05f7f73d21414` and the pinned transport SHA. Working source edits were excluded.

**Date:** 2026-10-03

**Axis:** Safety — degraded paths, truthful failure provenance, credential disclosure, account isolation, and bounds. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — one P2 finding blocks. No P0, P1 or P3 findings. I pre-commit to GO on a revision that resolves P2-1 as specified.

---

## 0. Evidence base

Inspected committed material:

- Root `AGENTS_GWZ.md` and core `AGENTS.md`.
- Root `dev-docs/AgentProcessRules.md`, particularly L1-08/09, L1-17–20 and the Surface amendment; `GwzProcessOptimization.md` §§1–4 and §8; `CurrentProgramCheckpoint.md`’s helper-context and accepted-wire entries.
- `GwzTransportCredentialHelperTimingAmendment.md`, lines 1–224.
- `GwzTransportCredentialHelpersSurface.md`, lines 1–121.
- `GwzTransportCredentialHelpersDesign.md`, revision 4: accepted decisions, §§2–7, amended clauses in §9, regression inventory in §10, messages and outcomes in §11.
- `GwzTransportReleasePlanAmendment-2.md`, revision 6: §§3.19–3.20.
- Accepted core sources:
  - `https_worker/budget.rs`, lines 1–72.
  - `https_worker/prepare.rs`, lines 1–230.
  - `https_worker.rs`, lines 1–200.
  - `https_destination.rs`, lines 1–308.
  - `https_policy.rs`, lines 1–109.
  - `transport_host/https_endpoint.rs`, lines 1–310 and 510–618.
  - `transport_host/https_endpoint/retry.rs`, lines 225–311.
  - `transport_host/session/driver/opening.rs`, lines 1–100 and 160–305.
  - `transport_host/session.rs` and `session/requests.rs`, relevant ownership and cancellation declarations.
- Accepted transport sources:
  - `protocol/transport.taut.py`, lines 1–96.
  - `codec/validate.rs`, lines 1–274.
  - `codec/failure_detail.rs`, lines 1–86.
  - `pool/mod.rs`, lines 1–112.

The three mandated `rev-parse HEAD` commands matched the exact tuple at both the beginning and end of review.

No files were written; no tests, builds or history mutations were run. Private evidence, secret values and current-round peer reports were not inspected. This is source-and-contract analysis, not executed regression evidence.

## 1. Findings

### [P2-1] Zero retained allocation is mislabeled as eight busy helpers

**Location:** Timing amendment lines 56–60, 81–87 and 163–166; caller Surface note lines 66–85.

**Violated invariant:** M10 must truthfully describe the helper-admission failure using the retained allowance. Its numeric provenance does not establish that all eight helpers were busy.

**Reproduction/state sequence:**

1. An authenticated continuation reaches helper admission with its retained allocation allowance already exhausted. The amendment explicitly admits this state: allocation `helper_budget_ms = 0` is valid, and “a budget already exhausted is represented by zero and starts no helper.”
2. All endpoint permits and host helper slots are available. No helper is running.
3. The endpoint reports `Timeout`, effect `None`, setup cause `Allocation`, and `helper_budget_ms = 0`, exactly as the amendment permits.
4. The renderer selects M10. The amendment changes only its first clause and preserves the rest, so the resulting message says that “the eight helpers gwz runs at once were all busy, waiting for sign-ins or stuck.”

That statement is false in this admitted state. The caller note reinforces the diagnosis with its “All helper slots remained busy” heading.

This is not a hypothetical new clock policy. Accepted producer code subtracts prior resource waits from allocation (`https_worker/prepare.rs:53–55, 149`) and carries the resulting budget into the continuation. The draft itself expressly specifies the zero-allowance case.

**Impact:** A caller is directed to finish or repair supposedly busy helpers when the failure arose from allocation consumed before helper admission. Retrying or signing in cannot address the asserted condition because no helper was busy. The new provenance makes the duration accurate while preserving an unsupported causal diagnosis.

**Required correction:** Make M10’s common explanation valid for both an exhausted allowance at entry and an actual permit/slot timeout. For example, describe failure to obtain helper resources within the available allowance, and present busy or waiting helpers as a possible cause rather than an established fact. Update the caller heading and explanation consistently. Explicitly supersede the additional M10 wording needed for this correction; the current “remainder stands” restriction otherwise prevents it.

No additional wire field, allocation policy or retry change is necessary.

**Closure/regression test:** Specify a row with zero retained allocation and all helper permits/slots available. Assert:

- no helper starts and no latch is set;
- the valid allocation detail carries zero;
- rendering reports zero seconds without asserting that helpers were running, busy or stuck.

Retain the positive-allowance saturation row to verify that its message and recovery remain useful.

## 2. Invariant analysis

**Timing bounds and provenance otherwise withstand the attack.** The draft distinguishes helper timeouts by field presence and phase, rather than inferring them from generic `Timeout` or `Allocation`. Wrong code, effect, phase and mixed detail are explicitly refused on every Failure carrier. Absent detail remains generic. The allocation ceiling is supported by accepted configuration validation: `pool::Config` caps resource durations at 86,400,000 ms, and `budget_for_open` only shortens the configured allocation. An arbitrarily large admitted Open does not enlarge that endpoint allowance.

**The timer/detail rounding seam is closed in the drafted contract.** The producer must use the same captured millisecond value for the timer and detail. Exact decimal rendering avoids silently rounding a shorter allowance to zero or overstating it. Sequential endpoint-permit and host-slot waits share one allocation deadline; helper interaction begins after admission.

**The added parser cause does not enlarge disclosure.** `malformed_output = 8` is a fixed enum alternative with fixed wording. It preserves existing duplicate/malformed-field refusals without carrying helper output, stderr or credential values.

**The username carrier has explicit confinement and bounds.** The separate HTTPS field preserves SSH semantics. The contract requires encoded input, password refusal, raw delimiter/control restrictions, decoded-control refusal, driver validation before Open, endpoint validation before effects, redirect validation and a reconstructed total URL bound. Username data is confined to Git’s `url=` input and private route selection; request serialization removes userinfo.

**The obvious diagnostic leak paths are expressly prohibited.** Generated Debug must redact the field through a reproducible projection, local/generated wrapper checks are required, and copying it into error formatting is forbidden. Generic raw-envelope formatting is excluded as an authorized diagnostic path. Those requirements still need implementation verification, but the draft does not authorize the leak.

**Route isolation is implementable behind the proposed shape.** The new field carries enough information to distinguish accounts without placing account selectors in HTTP requests or diagnostics. The affected-owner list includes endpoint reconstruction and account route isolation. Implementers must preserve that distinction through both route ownership and carried-retry selection; the existing retry key currently contains service, host, port and path only.

**Scope remains bounded.** The draft changes internal representation and two precisely named diagnostic clauses; it preserves public application schemas, error-code allocation, clone outcomes, cleanup ownership and accepted authentication decisions. Mixed-version behavior is not claimed as release qualification. Old missing fields remain absent, and old generic timeouts are not relabeled.

## 3. Risks and next action

Implementation still owes the listed admission/decode, redaction, carried-endpoint and timer-equality regressions. Account isolation should be exercised with two concurrent account selectors for the same host/path, including their anonymous-to-authenticated continuations. These are implementation obligations, not findings against this draft.

The next action is a bounded text revision resolving P2-1 in both documents, followed by a closure re-check against its newly settled tuple. This verdict does not accept runner implementation, platform authentication or release qualification.
