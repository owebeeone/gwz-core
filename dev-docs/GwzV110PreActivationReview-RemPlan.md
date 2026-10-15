# 1.1.0 pre-activation review (D19) — remediation plan, round 1

Status: **DRAFT, 2026-10-16.** The object reviewed is the D19 package's tuple (root `8fd99553`, gwz-core `d6cb37e0`, gwz-transport `a27817b9`, gwz-cli `bebcc198`, gwz-py `37e4c151`, gwz-sspi `d3038882`). The reports are filed verbatim:
- [Code](GwzV110PreActivationReview-ReviewCode.md): NO-GO, 2 P2 and 2 P3. Objects 1, 2, 3, 5 GO; object 4 NO-GO. Pre-committed to GO on P2-1 and P2-2 (self-provision, or a recorded decision).
- [State](GwzV110PreActivationReview-ReviewState.md): NO-GO, 2 P2 and 5 P3. Objects 2, 3, 5 GO; objects 1 and 4 NO-GO. Pre-committed to GO on P2-1 and P2-2.

**Architectural root causes (reviewer-classified):** Code P2-2 (a Cargo-built Windows CLI cannot provision its sign-in worker) and State P3-5 (the Python exit hook cannot bound async `call()`). Code P2-2 is answered by operator decision D23 (self-provision). State P3-5 is a P3 and goes to the operator (below).

**No blind convergence on a single defect.** Both axes found object 4 (SSPI/native HTTPS authentication) NO-GO, by different root causes: Code on response classification and provisioning, State on the helper's clock. Both also flagged the eight-round retry claim (Code P3-2, State follow-up 1) — the same root cause from two sides.

## Dispositions (one merged revision; lanes by area)

| Finding | Disposition | Lane | Closure test |
|---|---|---|---|
| Code P2-1 (Windows classifies every non-200 answer as a credential rejection) | `answer_basic` marks `credential.rejected` and `authenticated = Some(false)` only for a 401; every other final response goes to `prepare`'s classification (`classify`, `tell_governor`) as on the Unix `gh` path; the 3xx return stays. The SSPI exchange keeps its 3xx and "401 with no continuation token" refusals and hands 403, 404, 429 and 5xx to the same classification with the SSPI facts. | auth | For `WindowsConfigured` and `WindowsDefault`, fixture rows answering the credentialed request with 403, 404, 429 (Retry-After) and 503: code and words equal the `Gh` policy's rows; `authenticated` not `Some(false)`; no "(HTTP 401)" text. |
| State P2-2 (a configured helper answering Basic is charged to the 30 s network deadline D) | The Basic lookup runs on the helper's allocation and interaction allowance (as `credentials.rs`), with D paused for the lookup; an interaction expiry reports `Interaction` plus `helper_budget_ms` (M4). | auth | The pinned `native/tests/exchange.rs` row becomes two: a helper outlasting the old D but inside the allowance authenticates; one outlasting the allowance fails `Timeout` with `setup_cause = Interaction` and `helper_budget_ms`; `port.starts == 0` in both. |
| Code P3-1 (an SSPI provider refusal worded as a replaced worker) | `ProviderRejected` gets its own code or detail, keeps `native_status` as hex, and is worded "Windows refused the sign-in (SSPI status 0x…)", apart from the provenance refusals. | auth | The fake port returns `ProviderRejected(Some(0x8009030E))` on the first step: the text names the refusal and the status, without "Reinstall gwz". |
| Code P3-2, State follow-up 1 (the eight-round retry claim is proven only on a path production never takes) | Correct the comment and the test to the real invariant (a retry happens only before any exchange), driven through `HttpsEndpoint`'s retry machinery; or carry `native_rounds` and the logical deadline into `Held`. | auth | An endpoint-level row: an exchange then a forced retry — at most 8 steps across both attempts, or no retry admitted after an exchange. |
| Code P2-2, ARCHITECTURAL (a Cargo-built Windows CLI cannot sign in with Negotiate/NTLM) | **D23: self-provision.** gwz-cli `build.rs` generates a per-build random fingerprint when no packaging metadata is given and compiles it in; gwz-py's wheel keeps its producer. | provision | A gwz-cli test: a build without `GWZ_SSPI_PACKAGING_TARGETS` yields `Ok` from `executable()` and the worker's Hello matches; on dabeest a `cargo install --path` build authenticates against the real-SSPI fixture. |
| State P2-1 (the per-open backstop fires on held opens and cancels the whole request) | The backstop ends only its own stream (complete its reply, cancel its own stream, disconnect its peer; never `owner.cancel(request)`), and does not run while the endpoint holds the open (re-armed when an attempt starts, or held time excluded). | backstop | Candidate harness, stepped clock: one open held past its backstop while a second stream of the same request is mid-transfer; the second completes, the request is not sealed, the held open starts or fails alone. Red before. |
| State P3-4 (gwz-py notes attach to the interpreter after the host closed at exit) | The note sink drops notes once the host has closed at exit (`Ran.exiting`). | py | A note emitted after `close(true)` returns makes no interpreter call. |
| State P3-3 (on Windows the helper's config view, which can carry credentials, is copied into unwiped allocations) | Build the environment block and the copies in wiping owners on both platforms; or add them to OQ-C's accepted copy list. Fix preferred. | py (secrets hygiene, gwz-core `https_auth`) | An audit-probe row on the environment block owner. |

## Investigation item from the S5.3 evidence (not a reviewer finding)

- **A reader pause longer than `--ssh-timeout` (3 s native default) kills an SSH stream** with `TimedOut … StreamStall` (run `2026-10-15-s53`, both platforms; no CLI trigger found). If the pause is the gwz side's own consumer (disk, antivirus), the stall clock is charging local backpressure to the network, against the timeout plan §2. Lane `stall` establishes whether a real CLI path can trigger it and whether it is a defect; it reports before changing product code.

## For the operator (P3s that change scope or a published crate)

- **State P3-1:** gwz-sspi creates the worker's pipes inheritable; a concurrent std spawn can inherit them (bounded by D, same-user only). The fix is in gwz-sspi, which is published at 0.1.0, so it means a 0.1.1 and new pins. Fix for 1.1.0, or defer to 1.2.0 with the risk recorded.
- **State P3-2:** the WH2 spawn gate the contract tied to step 4.4 is not built, and C9 omits libgit2's helper spawn. Build the gate before S7.1, or record acceptance of the open exposure (with the libgit2 site added to C9).
- **State P3-5, ARCHITECTURAL:** Python's exit joins the executor threads that carry async `call()` before gwz's exit hook runs, so interpreter exit is bounded only by those operations. Run the close before the join, carry `call()` on gwz's own threads, or state the limitation in S7.2's notes.

## Follow-ups recorded, not in this round

Code §3 (cfg-boundary checker coverage of gwz-transport and gwz-sspi; Kerberos evidence; WPP 4.10's stale "Open" text; gwz-py's supervisor in every build; the gwz-py CI pin; the non-cancellable entry's cleanup). State §3 (WPP 4.2's permit text; the unbounded password-helper cleanup loop; gwz-sspi's ISC aliasing — to confirm; the per-request SSH cleanup count; gwz-py's late success after close; the not-in-job comment).

## Re-verdict

One merged revision of the lanes goes back to the same two reviewers with this plan and the diff since the D19 tuple. Each reviewer re-checks its own counterexamples and fills a prior-finding closure table. Round 1 of the two-round cap.

## Outcome of round 1 (2026-10-16)

Re-verdicts: [Code-2](GwzV110PreActivationReview-ReviewCode-2.md) **GO** (P2-1, P2-2, P3-2 closed; P3-1 deferred, accepted; new P3-3, P3-4, P3-5); [State-2](GwzV110PreActivationReview-ReviewState-2.md) **NO-GO** on a new P2-3 (present at the round-1 tuple, missed then; the round-1 P2s closed), with new P3-6 and P3-7, and P3-1, P3-2, P3-5 with the operator. No new architectural root cause. **Deferred and recorded:** Code P3-1 (the SSPI status needs a gwz-transport `FailureDetail` field — 1.2.0 wire list beside D16; the "Reinstall gwz" wording for a policy refusal is a known 1.1.0 limitation for S7.2's notes); the backstop's Cli-placement residue (no holds reported across processes — a wire field, 1.2.0). Round 2: `GwzV110PreActivationReview-RemPlan-2.md`.
