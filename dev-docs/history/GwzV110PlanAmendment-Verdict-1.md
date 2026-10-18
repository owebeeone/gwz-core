# GWZ 1.1.0 plan amendment — second draft verdict

Date: 2026-09-26. Status: **NO-GO on the second draft at SHA-256 `1606f9672a1a08dd2e53c92ef497bb5911bc0de53893ca67af7ff56e9cd56740`: [Consistency](GwzV110PlanAmendment-ReviewConsistency-1.md) and [Safety](GwzV110PlanAmendment-ReviewSafety-1.md) both reported NO-GO on one shared blocking root, and both pre-committed to GO on a revision that resolves it as specified. The plan is unchanged**.

The same two reviewers re-verdicted with their context intact.
- They verified the draft's hash at the start and end, and read the controlling documents from commits.
- They did not read each other's current-round report.
- They ran inspection commands only.

The second draft applied the operator's 2026-09-26 decision and the [remediation plan](GwzV110PlanAmendment-RemPlan.md). Under that decision, gwz-py reaches the transport through gwz-cli's entry, one runtime per operation, and keeps its public API.

| Axis | Verdict | Prior findings | New blocking | New nonblocking |
| --- | --- | --- | --- | --- |
| Consistency | NO-GO | P2-1, P2-2, P3-1 to P3-5 closed | P2-3 | P3-6 to P3-8 |
| Safety | NO-GO | P2-1, P3-1 to P3-6 closed | P2-2 | P3-8 to P3-10 |

Every first-round finding is closed, and both axes found the operator's decision stated consistently. Both numbered new findings independently, so P3-8 names two different findings; this verdict cites every ID with its axis. Neither reviewer classified any finding as architectural. This is the amendment's second review round.

## Blocking root

### B1 — gwz-py's normal build is never switched to the transport, and the first proof comes after publishing (Consistency P2-3 and Safety P2-2, blind convergence)

gwz-py has its own `gwz_transport_candidate` sites: in `native/src/lib.rs`, `native/src/dispatch/mod.rs` and `native/src/dispatch/merge.rs`, plus its `check-cfg` declaration in `Cargo.toml`. Their fallback branch is the native path.

The draft places S6.2's new arms under that switch, and leaves S7.1 scoped to "the sites Phase 4 already opened to Windows", which are gwz-core's. No step removes gwz-py's switch. S7.3 only builds the extension, and the draft's only route assertion on a normal build is Phase 8's post-release check. That check runs after the wheel is on PyPI, where the plan's stop rule makes a failure irreversible.

The result could be a published gwz-py 1.1.0 that silently keeps native SSH and HTTPS on every platform. Both reviewers give the same three-part correction:
1. S7.1 removes the switch from gwz-py's sites and from S6.1's variant.
2. S7.3 asserts the route before the tag.
3. Phase 8 names that assertion as the pre-publish proof.

## Nonblocking findings

- **Environment stability (Consistency P3-8 and Safety P3-9, blind convergence).** S1.1 lists "environment stability" as surviving. The gwz-py design defines it as stability for the life of a `Client`, which only the contract's session context provided. `with_local_transport` reads the environment on every call. The per-operation model gives capture at each operation's start instead, which is a documented-behaviour change for S1.2's Surface trigger.
- **Consistency P3-6.** S7.2's new Python ledger rows are "backed by S6.3", but S7.2's unchanged rule admits only cells covered by S5.6's table, which has no Python rows.
- **Consistency P3-7.** S6.3's dabeest row cannot pass before S4.5, but the sketch, declared unchanged, has no `S4.5 ── S6.3` edge. Safety adds that the re-run rule does nothing when S6.2 lands second, since S4.5's fixtures do not run through gwz-py.
- **Safety P3-8.** S6.3 has no row for behaviours the model introduces or changes:
  - close and interpreter exit with operations running;
  - the ninth operation waiting under the 8-per-`Client` bound;
  - cancel naming a wrong, foreign or completed operation among concurrent ones.
- **Safety P3-10.** "Exactly as `with_local_transport` does" imports the CLI entry's finish from `Drop` into a library. A panic during finish while unwinding would abort the host Python process. The removed `TransportSession` guarded that path, and the contract's §5.2 forbids it.

## Residual risks below the finding bar

- §6 should say that the authority for S6.1 is the amended plan and S1.1's revision, not the contract. It should also say that a later change to the contract's §5.2 does not amend S6.1.
- S1.1 should state that `configure_transport_runtime` stays process-wide. It should also restate the process-wide helper caps as the only bound across `Client`s.
- S6.3's stall row is a gwz-py-level test.
- The native branch stays for the platforms 1.1.0 does not support. Removing the switch must not remove that branch.

## Next action

A third draft applies [RemPlan-1](GwzV110PlanAmendment-RemPlan-1.md) as one patch, and the same reviewers give a focused re-verdict.
