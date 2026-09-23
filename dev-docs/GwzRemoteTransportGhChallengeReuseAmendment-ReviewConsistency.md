# GwzRemoteTransportGhChallengeReuseAmendment — CONSISTENCY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzRemoteTransportGhChallengeReuseAmendment.md` at core `51e28002e3a9f78f5a8daa873e02b083e2b2341f`, SHA-256 `0103055f7ad5a6dd01d7ae9cb662cf61d624f89c954db983020a8a4b892e5fa3`; draft amendment dated 2026-09-23  
**Baseline:** root `b567fe46ea0112872772f9e532dad7c498cc69b4`; core `51e28002e3a9f78f5a8daa873e02b083e2b2341f`; transport `14f0d09cbca7ab54c06d36e8354799981584a7f9`; CLI `1e52b274ed242d81349b6d18c107680152ecfecf`; Python `d484e367d1df47eff5caf3421864e69ec85744f5`. Committed documents were read with `git show`; private evidence was read without modification.  
**Date:** 2026-09-23  
**Axis:** Consistency against the HTTPS, transport-lifecycle, and persistent-session contracts, including exact supersession and satisfiable causal evidence. Independent, adversarial, read-only. The Safety axis ran in parallel; nothing here relies on its current report. Filed verbatim by the lane owner.

**Verdict: NO-GO** — one P2 and one P3 finding. I pre-commit to GO on a revision that resolves **P2-1** and includes **P3-1** in the same bounded document correction.

---

## 0. Evidence base

The five repository HEADs and amendment hash matched the required tuple at both review boundaries. The authorized untracked files matched the prompt’s exclusions.

Reviewed the amendment, HTTPS Design §§4, 6, 7, and 10; relevant Remote Transport Design pooling/deadline clauses; Python Transport Design persistent-runtime clauses; Requirements C3; Placement Design’s replay rule; the H1 checkpoint; and the named private README and failing native-extension trace. No builds, tests, writes, product implementation, peer report, or release evidence were used.

## 1. Findings

### [P2-1] The amendment leaves HTTPS Design §7’s unconditional discard rule in force

**Root cause and location:** The amendment’s status says it changes only HTTPS Design §§4 and 6, and its proposed correction names only §6’s “no terminal failure” reuse condition. HTTPS Design §7 separately states: “All terminal non-success responses discard the connection.” An anonymous discovery 401/404 is a terminal non-success response.

**Violated invariant:** Every controlling clause contradicted by an amendment must be named and given an unambiguous replacement.

**Counterexample:** An implementer following the amendment drains a clean anonymous 401, observes a ready sender, and releases the lease reusable for the Gh GET. An implementer following the still-controlling §7 sentence discards the same connection. Both can claim conformance to the current authority graph.

**Impact:** Physical disposition, connection counts, and the central regression outcome remain authority-dependent; the amendment cannot serve as implementation authority.

**Required correction:** Explicitly supersede §7’s blanket discard sentence for exactly a fully drained, correctly framed anonymous discovery 401/404 that is immediately taking §4’s once-only Gh transition. Preserve discard for every other terminal non-success and for every failed drain/readiness/budget/cancellation condition. State that the bounded 64 KiB drain is the sole exception to §7’s error-body handling and grants no Git success.

**Closure test:** A normalized clause audit finds both §6 and §7 conditions in the supersession list, and one disposition table proves every status/cleanup combination has exactly one result.

### [P3-1] The cited red evidence and planned regression do not causally identify challenge-socket reuse

**Root cause and location:** “Observed gap” and “Verification and compatibility.” The raw fixture records `hits` and `connections` only after Authorization succeeds; anonymous 401 requests return before either collection is updated. The planned assertion that the second fetch opens no new connection likewise does not require the anonymous challenge and its immediate Gh GET to use the same physical connection.

**Counterexample:** With multiple idle connections, an implementation can discard the challenged socket, lease another existing idle socket for Gh, and satisfy “no new TCP/TLS connection” while violating the amendment’s intended handoff.

**Impact:** The regression can go green without proving the corrected disposition, and the archived trace supports distinct authenticated-request ports but not the stated anonymous-to-Gh socket sequence.

**Required correction:** Record every request before the authorization branch, including connection identity, anonymous/Gh policy, order, and gh invocation. Require each qualifying anonymous 401/404 and its immediate Gh GET to share the same connection; adverse cases must show discard and a different subsequent connection or no retry.

**Closure test:** A causal red/green fixture asserts the complete ordered `(policy, status, connection_id)` sequence for clone and both fetches plus fresh gh lookup counts.

## 2. Invariant analysis

The proposed mechanics otherwise agree with the controlling design:

- Authentication replay remains one anonymous discovery GET followed by at most one Gh GET, with no POST or network-failure replay.
- The first receipt stays private; only the final attempt becomes the operation result.
- Cleanup, allocation, connection, interaction, and network allowances remain cumulative.
- Bounded body drain, validated framing, `Connection: close`, sender readiness, cancellation, and peer-close handling preserve the pool’s disposal rules.
- Sender readiness remains only a hint; a reuse race surfaces as a typed failure without another retry.
- TLS reuse carries no account authority. Every Gh request performs a fresh helper lookup and supplies Authorization only on that request.
- Sequential operations retain their own anonymous challenge and once-only transition.
- Persistent Python runtime ownership is compatible with cross-operation connection reuse.
- No schema, public API, placement, release-order, or physical-carrier change is implied.

## 3. Risks and next action

Real-account GitHub behavior, implementation, activation, and release remain properly deferred. Apply one document-only correction naming the §7 supersession and strengthening the causal fixture, settle a new tuple, and run a focused consistency re-verdict.
