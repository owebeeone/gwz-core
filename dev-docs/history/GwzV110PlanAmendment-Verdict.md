# GWZ 1.1.0 plan amendment — review verdict

Date: 2026-09-26. Status: **NO-GO on the draft at SHA-256 `774bb164c1c33122738b6864cabcc81203404bd25372bebe32b3984c1cdd7acf`: [Consistency](GwzV110PlanAmendment-ReviewConsistency.md) and [Safety](GwzV110PlanAmendment-ReviewSafety.md) both reported NO-GO. The plan is unchanged**.

The object is [GwzV110PlanAmendment.md](GwzV110PlanAmendment.md), an uncommitted draft identified by its hash, as the plan itself was reviewed. It controls [GwzV110Plan.md](GwzV110Plan.md) as last changed at gwz-core `b141e26d`.

Two fresh peer-blind reviewers on a different model from the drafter reviewed it.
- Both verified the object's hash at the start and end, and read every controlling document from commits.
- Neither saw the other's report.
- Neither ran anything but inspection commands.

| Axis | Verdict | Blocking | Nonblocking |
| --- | --- | --- | --- |
| Consistency | NO-GO | P2-1, P2-2 | P3-1 to P3-5 |
| Safety | NO-GO | P2-1 | P3-1 to P3-6 |

Both axes numbered from P2-1 and P3-1, so every ID below carries its axis name. Each reviewer pre-committed to GO on a revision that resolves its P2 findings as specified. Neither classified any finding as architectural.

## Blocking roots

### A1 — the plan's earlier Phase 1 design is missing from the amendment (Consistency P2-1, blind convergence with Safety P3-5)

The plan's S1.1 named `gwz-py/dev-docs/GwzPyTransportDesign.md`. That design's status still reads "S1.1/S1.2 design accepted for implementation, 2026-09-23 … Operator authorized implementation". It also accepted a bounded package-boundary amendment to the plan's Phase 1 and Phase 6 that was never applied to the plan's text. Its long-lived `TransportSession` was implemented in working source. A later status paragraph records a NO-GO for Phase 6 completion and Phase 7 activation, which predates the retired train that tried to close it.

The draft gets this history wrong. It says the plan's gate had no object that could meet it, calls the NO-GO the retired train's, and removes it with no closing condition. It leaves gwz-py's design carrying a live implementation authority for the superseded design.

### A2 — the gwz-cli change the contract forces has no owner (Consistency P2-2)

The draft treats gwz-cli as untouched in 1.1.0, but the contract S6.1 implements says otherwise.
- **§5.2** moves gwz-cli's `execute_invocation` into gwz-core as the shared dispatch.
- **§16** changes the core APIs that gwz-cli calls.

Either gwz-cli is re-pointed in a step the draft declares gwz-core-only, or two dispatch tables remain, which contradicts the contract and G2. The draft's claims that S7 is unaffected and that no public API changes are wrong for the same reason.

### A3 — no step proves the transport route after S6.1, so Windows can ship native (Safety P2-1)

S6.1 edits both transport entries, the CLI's moved dispatch and Python's session variant. The draft lets that edit land after S4.5 and S5.5, the plan's only Windows functional evidence. Afterwards, nothing proves the transport route:
- S7.1 adds no Windows arm;
- S7.3 only builds;
- S6.3 names no platform;
- Phase 8's post-release check only runs the ledger's commands.

Both entries fall back silently to libgit2's native backend when their arm is missing. A Windows 1.1.0 build can therefore ship native SSH and HTTPS, without gh-only HTTPS, endpoint-owned credentials or the setup clocks, and pass every named test. A2 and A3 share a cause: the draft did not account for S6.1 moving the CLI's entry.

## Nonblocking findings

### Convergences

- **S1.2's closure rule** (Consistency P3-2, Safety P3-1). "The revision that implementation follows" names no SHA and no verdict document, and revision 2's existing GO could be read to satisfy it. It gives no path when a revision fails its review or appears mid-implementation. It restates a remediation cap the contract has already used.
- **The implementation-plan gate** (Consistency P3-5, Safety P3-2). A new prerequisite of S6.1 with no step ID, no place in the normative dependency sketch, and no review rule.

### Single-axis findings

- **Consistency P3-1.** §2's in-release cell "one pool" (plan line 50) still promises cross-operation pooling.
- **Consistency P3-3.** §4 quotes the contract's §14 test inexactly while attributing it.
- **Consistency P3-4.** S6.3 is assigned to gwz-py, but it adopts §15's core rows and needs the byte-stream adapter, which S6.1 omits.
- **Safety P3-3.** The two-bridge CI run becomes release evidence outside §2's redaction rule. It also has no rule for which run, against which core, is the evidence.
- **Safety P3-4.** Python's per-operation runtimes (8 × the pool ceiling per host by default) are unmeasured in Phase 5 and undisclosed in S7.2.
- **Safety P3-6.** Nothing shows that the session variant applies the accepted clocks and defaults, or runs S3.3's stall regression.

### Residual risks below the finding bar

- **Scope.** As amended, S7.1, the CLI's activation, and so all of 1.1.0, waits on the whole core session program: three milestones each above 500 lines, plus the two-bridge proof. The draft's §6 offered the operator only the enlarging option, S6.4. The other option is to activate the CLI's transport in 1.1.0 and gate §2's Python row separately. That is a scope decision.
- The plan's line 75, "Production core does not depend on `gwz-transport`", is false after S7.1.
- The contract's example host binary ships in the gwz-core crate source, though not installed. §5 could say it is test-only.
- §2's "both placements" should say "local placement" for Python.
- The S6.1 milestone will push `transport_host` past the workspace's file-size review threshold.

## Next action

The amendment needs a second draft that resolves A1–A3 and carries the P3 findings, then a focused re-verdict from the same two reviewers. [GwzV110PlanAmendment-RemPlan.md](GwzV110PlanAmendment-RemPlan.md) maps them. A2 and the scope residual need operator decisions first, because they decide what 1.1.0 is.
