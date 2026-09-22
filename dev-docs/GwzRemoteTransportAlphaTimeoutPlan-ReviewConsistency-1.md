# GwzRemoteTransportAlphaTimeoutPlan — CONSISTENCY-AXIS REVIEW

**Review object:** untracked working-tree draft `gwz-core/dev-docs/GwzRemoteTransportAlphaTimeoutPlan.md`, SHA-256 `cfdf028fb18557960da18a4682cb10f3e9e638ff197c4784efb76ac9a526984b` (was `773639e1b545eeba776c3faeac1b0785aef50ee33faa0ba1e984b5ec214cac4a`; verified at review start and end; not in gwz-core HEAD).
**Baseline:** gwz-dev `7b3f1bc723d6c846bd7d5a4f009602afd90a1593`; gwz-core `7bd9381f0593f759333c7d8d070f4962d4092df7`; gwz-transport `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Diagnosis draft SHA-256 `977d7884877aad024261c8bf366918f4a1903a884aab6422f30422e7648ad5f6` (unchanged). Design matches gwz-core HEAD (`git -C gwz-core diff --stat -- dev-docs/GwzRemoteTransportDesign.md` empty). Round-2 remplan read as `GwzRemoteTransportAlphaTimeoutPlan-RemPlan.md` (dispositions only; not treated as proof).
**Date:** 2026-09-22
**Axis:** Consistency: the plan against its controlling design, diagnosis, and the code it cites. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — prior blocking findings closed; 0×P0, 0×P1, 0×P2, 0×P3 open on this revision.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1 | Accept: `begin_wait` / `complete_wait`; idle slices not completions; S3.3 wires production path + production-graph regression | Original sequence (Control has no wait-boundary API → only scripted four-wait test → live DNS/TCP/handshake/agent never demarcates → Phase 5 can pass without per-attempt stall) fails: §2 defines completion; S3.1 names both APIs; S3.3 requires production wiring and production-graph tests; S5.2 refuses close without S3.3 stall proof | CLOSED |
| P2-2 | Accept: S3.1 deterministic clock seam; S3.2 drives it; no wall sleep | Original sequence (S3.2 forbids sleep while `Control` only compares `Instant::now()` with no test seam) fails: S3.1 requires a deterministic clock used inside `check` / `quantum`; S3.2 advances that clock for stall expiry with aggregate still ahead | CLOSED |
| P2-3 | Accept: S4.1 depends on S2.2 and S3.1; late-result uses uncoupled aggregate | Original sequence (S4.1 under S3 only → can “finish” while Open still couples `connect_ms` to 3 s I/O) fails: S4.1 text and §4 both require S2.2 + S3.1; late-result Instant named as post-uncoupling pool aggregate | CLOSED |
| P3-1 | Accept: S1.1 quotes superseded and unchanged design sentences | Original gap (topical paraphrase only) fails: S1.1 lists superseded quotes for §10 connect/auth cell, §10.2 disable identification, and §10.2 joint listing, plus unchanged §10.1 Open-shorten, §10.2 tighten, and §10.2 disposal; quotes match design HEAD prose (markup/wrapping only) | CLOSED |

---

## Changed-range analysis

Relative to draft `773639e1…`, the replacement adds: §2 completion / non-completion rule; interaction pause of both clocks; distinct `stall` / `aggregate` reasons; S1.1 supersession quote lists; S2.1 stall argument + identity-check default; Phase 2 HTTPS aggregate bound + stuck-connect fixture wording; S3.1 `begin_wait` / `complete_wait` + deterministic clock + interaction forward; S3.2 clock-driven cases; **new S3.3** production wait boundaries; S4.1 explicit S2.2+S3.1 dependency and late-result tests; S4.2 dual-reason fixtures; S5.1/S5.2 requiring S3.3; rewritten §4 graph; §5 HTTPS/reason clarifications.

These changes fall inside the remplan dispositions for Consistency P2-1, P2-2, P2-3, and P3-1 (and the shared production-wiring root). No change outside those dispositions introduced a new architectural root cause on this axis. Remplan-driven Safety items present in the same text (HTTPS bound, reasons, interaction, late-result kind) were checked only for consistency-graph breakage; none reopen a Consistency P0–P2.

---

## 0. Evidence base

End tuple: plan hash `cfdf028f…`, diagnosis `977d7884…`, three HEADs, and clean design diff — unchanged from start of this re-verdict.

Prior Consistency report counterexamples re-applied to the new plan text and to the cited live call sites (`wait_socket` / `wait_session` exist in `ssh_network.rs`; agent `EAGAIN` path exists in `agent_auth.rs`). Design L663, L705–707, L762–764, L770–771, L774–776, L783–784 compared to S1.1 quote lists.

---

## 1. Findings

None open.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| Prior P2-1 / P2-2 / P2-3 / P3-1 counterexamples | Closed (table above) |
| Internal §§1–5 / phase / §4 agreement | Holds: S3.3 before S5.2; S4.1 on S2.2+S3.1; Phase 2 HTTPS expiry fixture deferred to S3.1 clock via S3.2 as §4 states |
| Design §10 / §10.1 / §10.2 target agreement (zero disables both; positive stall does not move 10 s aggregate; Open-shorten on io) | Holds under S1.1’s listed replacements; unchanged Open-shorten / tighten / disposal sentences preserved by checklist |
| Cited current-code coupling claims | Still accurate; plan still describes the defect before the steps fix it |
| S3.1 / S3.2 / S3.3 / S4.1 satisfiable as written | Holds on paper: clock seam, completion API, production wiring, and aggregate dependency are all named |

---

## 3. Risks and next action

Residual execution risk is implementation fidelity (S3.3 must actually mark the §2 completions; S5.2 must refuse unwired stall), not a remaining plan-consistency hole.

**Next action:** accept this Consistency GO for draft `cfdf028f…`; lane owner merges with the parallel axis. No tree mutations in this review.
