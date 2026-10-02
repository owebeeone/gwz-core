# TR2.22 helper context amendment — CONSISTENCY-AXIS REVIEW

**Review object:** Committed DRAFT `gwz-core/dev-docs/GwzTransportCredentialHelperTimingAmendment.md` and caller note `GwzTransportCredentialHelpersSurface.md`, at core `0adb093a99f4eb8e9b8818b767514a842533a193`, dated 2026-10-03. This is document review, not implementation acceptance.

**Baseline:** Root `bb44a7214eac779095352224e8940df268fd3f96`; gwz-core `0adb093a99f4eb8e9b8818b767514a842533a193`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Documents were read with `git show` at these commits. Accepted producer code was read at core `26922a1cfd823a4894e09be9aeb05f7f73d21414`. Working-tree changes were excluded.

**Date:** 2026-10-03

**Axis:** Consistency with the controlling contracts, exact supersessions, producer semantics and satisfiability of the stated regressions. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — one P2 finding blocks; one P3 finding remains. I pre-commit to GO on a revision that resolves P2-1 and P3-1 as specified.

---

## 0. Evidence base

Read and compared:

- Helper context amendment, lines 1–224, including the two wire additions, malformed-output cause, exact supersessions and regression rows.
- Caller Surface note, lines 1–121.
- `GwzTransportCredentialHelpersDesign.md`, revision 4: accepted decisions and authority, §§2–7, §§9.1–9.4, §10’s relevant fixture rows, §11’s messages, and OQ1(b), OQ5(a), OQ6(a), OQ7(1). Particular comparisons were lines 79–87, 100–143, 208–220, 247–254, 258–284, 317–338 and 394–426.
- `GwzTransportReleasePlanAmendment-2.md`, revision 6: status/authority, §3.19’s helper mechanism and §3.20’s TR2.22 boundary, lines 1–36, 454–476 and 511–543.
- `GwzRemoteTransportHttpsDesign.md`, destination and redirect contract, lines 180–215.
- Root `AGENTS_GWZ.md`; `AgentProcessRules.md`’s amendment/lifecycle and document/review templates; `GwzProcessOptimization.md`; and `dev-docs/CurrentProgramCheckpoint.md`, lines 1–39.

Accepted source comparisons:

- Transport authored schema, `protocol/transport.taut.py`, lines 1–96: existing enum values, Destination tag 5, four-field FailureDetail and all Failure carriers.
- Transport `src/codec/validate.rs`, lines 1–274; `src/codec/failure_detail.rs`, lines 1–86; `src/codec.rs`, lines 1–96.
- Transport `src/pool/mod.rs`, lines 27–71 and 169–211: defaults, validated configuration ceiling and caller shortening.
- Transport `scripts/regen.py`, lines 63–122: existing fail-closed Rust projection and reproducible generation.
- Core `https_worker/budget.rs`, lines 1–72; `https_worker/prepare.rs`, lines 30–149; `https_worker.rs`, budget and challenge-carriage declarations.
- Core `session/driver/opening.rs`, lines 53–93 and 139–194.
- Core `https_endpoint.rs`, retained Retry admission/publication and `retry_key`, including lines 229–254, 383–401 and 562–567; `https_endpoint/retry.rs`, retained budget shortening and reconstruction, lines 195–264 and 284–300.
- Core `https_destination.rs`, lines 1–202; `https_policy.rs`, lines 54–108; `https_opening.rs`, lines 39–63 and 170–179.
- Core `https_auth.rs`, current failure mapping and parser, lines 213–227 and 506–547.

The three prescribed `git rev-parse` commands returned the exact pinned tuple both at review start and review end. No builds, tests, file writes, Git mutations, secret inspection or private evidence inspection were performed.

## 1. Findings

### [P2-1] An already-exhausted budget is classified as busy helper slots without evidence of slot contention

**Location:** Amendment lines 43 and 56–60, together with lines 81–84; caller Surface note lines 68–81.

**Violated invariant:** Helper timing provenance must select a truthful diagnostic. The amendment changes only M10’s first clause and explicitly preserves its remainder: “the eight helpers gwz runs at once were all busy, waiting for sign-ins or stuck.” Accepted TR1.6 §3.3 assigns M10 to a helper permit/slot wait that runs out, rather than every exhausted allocation budget.

**Reproduction/state sequence:**

1. Anonymous work or earlier resource admission consumes the retained allocation allowance.
2. At helper admission, the retained allowance is zero. Both helper semaphores have capacity; no helper is waiting for a sign-in.
3. Amendment line 60 directs that this already-exhausted budget be represented by zero under the M10 capture rule, and no helper starts.
4. `Timeout` + `Allocation` + `helper_budget_ms = 0` identifies M10. The mandated unchanged remainder asserts that all eight helpers were busy.

This needs no failing helper. The accepted source already distinguishes exhaustion before helper work: `https_worker/prepare.rs:43–51` returns a generic timeout for an exhausted retained domain. Allocation is shortened independently of helper-slot occupancy at `:54–55`, `:149`, and `https_endpoint/retry.rs:249–251`.

**Impact:** The new provenance can turn ordinary resource exhaustion into a false helper-capacity diagnosis. The caller is told to finish sign-ins or unlock a credential store when that condition did not occur. This defeats the amendment’s stated purpose of distinguishing helper admission from general allocation timeout.

**Required correction:** Define the exhausted-before-wait case explicitly. Preserve generic timeout rendering by omitting helper provenance for that case, or amend the zero-budget message and its exact supersession so it makes no unsupported claim about slot occupancy. Specify when a zero allocation detail is legitimate; validator acceptance of zero alone does not establish contention.

**Closure/regression test:** Enter helper admission with zero retained allocation and both semaphores free. Assert no spawn, no latch and no busy-slot diagnosis. Separately expire a positive allowance while a helper permit or host slot remains unavailable, and assert M10 retains its captured initial allowance and the intended capacity diagnosis.

### [P3-1] Exact decimal rendering leaves the controlling rounded-down queue assertion unsuperseded

**Location:** Amendment lines 76–80, 148–166 and regression row 2 at line 197; controlling TR1.6 revision 4 §10 T15(b), line 275.

**Violated invariant:** The exact supersession list must identify changed controlling assertions, and the combined test requirements must specify one observable result.

**Reproduction:** TR1.6 T15(b) states that M10 names `D − W`, “rounded down.” Choose retained allocation of 1,250 ms after the anonymous resource wait. The amendment requires exact rendering as `1.25`; its caller note repeats that fractions are shown exactly. The purported exhaustive supersession list changes M10’s first clause and universal ceiling, but never changes T15(b)’s rounding assertion.

**Impact:** The regression owner inherits an ambiguous controlling assertion about rounding in seconds alongside a new exact fractional assertion. An existing T15 expectation can conflict with the new regression, and acceptance depends on an unstated interpretation of “rounded down.”

**Required correction:** Add T15(b) to the exact supersession list and give its replacement assertion. Distinguish any truncation used to capture an integer millisecond timer from rendering that captured value in seconds.

**Closure/regression test:** T15(b) and the new regression must both require a captured 1,250 ms allowance to render as `1.25`, with the same captured millisecond value driving the timer and detail. Also state the treatment of sub-millisecond retained remainders.

## 2. Invariant analysis

The following attacks did not establish further blocking defects:

- **Allocation ceiling:** The amendment correctly distinguishes admitted positive i64 Open values from endpoint producer values. Pool configuration permits allocation durations through 86,400,000 ms; the endpoint initializes its budget from that validated configuration and subsequently shortens it. The 30,000 ms default is therefore unsuitable as a universal detail ceiling.
- **Interaction provenance:** The fixed 120-second helper cap, positive caller shortening and configured shortening can be represented by one bounded integer. Capturing after admission preserves the accepted separation between allocation waits and helper interaction.
- **Failure-carrier completeness:** The accepted envelope validator walks `BindRejected`, `OpenFailed`, `Failed`, `IdentityCheckFailed` and `Closed.failure`. Admission and encoded decode reach that validation seam. “Every Failure carrier” is implementable without a new interface.
- **Fixed malformed-output cause:** Enum value 8 extends values 1–7 without reassigning them. Its fixed phrase agrees with the caller note and represents duplicate/malformed recognized-field refusal without transporting helper output.
- **Username carriage:** Destination tag 6 is distinct from the existing SSH tag 5. Missing-or-null absence, explicit-empty refusal, scheme restriction, encoded preservation, decoded-control refusal and additional metadata limits are stated. The separate field can carry account selection through a process boundary without adding a public GWZ schema field.
- **Request and diagnostic isolation:** The contract explicitly removes userinfo before request serialization, excludes the selector from facts and Failure detail, and requires generated and local wrapper Debug checks. Existing reconstruction, route selectors and retry-carriage keys need implementation changes, but the proposed field is sufficient to make those changes; no further wire redesign was demonstrated.
- **Generation ownership:** The existing generator already has a fail-closed local Rust projection. Extending that reproducible mechanism for username Debug redaction is consistent with retaining the boxed FailureDetail projection and avoiding handwritten generated edits.
- **Scope:** The amendment remains a DRAFT and the checkpoint excludes runner working noise. No implementation, Windows authentication, release or performance conclusion is inferred.

The amendment’s line 168–169 citation is imprecise: TR1.6 §9.1’s OQ1(b) contingency changes HTTPS line 148’s helper-input prohibition, whereas the accepted remote-username decision is stated directly in OQ1(b) and reflected in §9.4 C3’s application-contract changes. I did not elevate that citation issue into another finding or infer permission to relax the separate redirect-Location restrictions.

## 3. Risks and next action

Producer implementation, generated redaction and local/carried route isolation remain unverified. In particular, implementation evidence should interleave account selectors on the same host/path so route and retained-challenge keys cannot accidentally collapse them. That is future implementation evidence, not a finding against unimplemented runner code.

The next action is one bounded document correction resolving P2-1 and P3-1, followed by a Consistency re-verdict on the new settled tuple. The reviewed tuple remained unchanged through the final check.
