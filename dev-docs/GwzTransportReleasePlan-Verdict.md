# GWZ transport release plan — first review verdict

Date: 2026-09-27. Status: **NO-GO at SHA-256 `90fbd213fa1cde21bb6b5d47aa471488c28b9eeed5dc10da4d43ee8e16a5cceb`: [Consistency](GwzTransportReleasePlan-ReviewConsistency.md) and [Safety](GwzTransportReleasePlan-ReviewSafety.md) both reported NO-GO. Both reviewers committed in advance to GO on a revision that resolves their blocking findings as specified.** This verdict accepts nothing.

The object was the uncommitted draft `gwz-core/dev-docs/GwzTransportReleasePlan.md`, identified by its SHA-256, against the repository HEADs its reviews list.
- Two fresh reviewers ran in parallel. Neither saw the other's report.
- Both verified the object and eight uncommitted controlling documents by SHA-256, at the start and the end.
- Both ran only inspection commands. Each also ran one read-only crates.io query.

| Axis | Verdict | P0 | P1 | P2 | P3 |
| --- | --- | --- | --- | --- | --- |
| Consistency | NO-GO | 0 | 0 | 2 | 16 |
| Safety | NO-GO | 0 | 1 | 5 | 6 |

## Blocking findings

| ID | Axis | Finding |
| --- | --- | --- |
| C-P2-1 | Consistency | §2 puts the in-process CLI's `cli` placement in the release. No production path installs a `cli` endpoint, no step builds one, and the contract excludes client placement. As written, S5.6 and S7.2 block the plan's own Phase 9. |
| C-P2-2 | Consistency | The 1.1.0 plan's Phase 8 stop-on-failure rule and its product-repository ordering and pin sentence are neither adopted nor retired, so superseding that plan drops them. |
| S-P1-1 | Safety | Under the server design, the client sends its secret-bearing environment as its first frame to whatever answers at the address. It never verifies the listener's owner, so a squatted `/tmp/gwz-<uid>/` socket or a pre-created Windows pipe receives it. |
| S-P2-1 | Safety | §6's "lands switched off until Phase 9" names no switch, and the session plan's ordinary-build steps cannot be switched off. A 1.0.x patch cut from main would ship them before the activation review. |
| S-P2-2 | Safety | Phase 2's rule for main ends at TR2.1. Until S7.1, the ordinary build's `--ssh-timeout` help and `--max-retries` would describe behaviour that exists only in the transport build. |
| S-P2-3 | Safety | TR1.5 leaves the off switch's scope open. A workspace- or repository-level key would let a workspace silently force every operation onto the native route. |
| S-P2-4 | Safety | TR1.5 requires the off switch to work through a server. In a transport-build server, native SSH authenticates with the server's agent, because the must-match list omits `SSH_AUTH_SOCK` in transport builds. |
| S-P2-5 | Safety | TR1.2's questions allow a shared runtime to keep today's construction-time endpoint configuration: agent, home, TLS roots, proxies and the `gh` environment. A server would then serve every session with the constructing environment's identity. |

No reviewer classified any finding as architectural. This was the first round; the two-round cap has one round left.

## Blind convergence

Both axes, reviewing blind, landed on the same defects:
1. **§6's merge rule.**
   - Consistency P3-14: the rule names no switch, and S7.1's check sees only the candidate cfg.
   - Safety P2-1: the rule cannot be enforced for the session host.
   - Both reviewers also noted that TR2.3's change to the `errors` contract would reach a 1.0.x patch.
2. **Pageant.**
   - Consistency flagged it as a residual: agent-channel code placed in Phase 7 rather than in S4.3's file.
   - Safety P3-3: scheduling it as a server primitive contradicts S4.3's rule against falling back to another key store.
3. **The bootstrap placeholders and S2.3.**
   - Consistency P3-6: the first-publish token clause is adopted although every name's first publish has already happened.
   - Safety P3-5: token-bearing publish paths outside the release sequence remain live.
4. **Surface for TR2.3.**
   - Consistency P3-13 flagged it.
   - Safety noted as a residual that the documented `errors` contract changes with Code and State review only.
5. **TR2.4's feature gate.**
   - Consistency P3-12: `binding.rs` has two version-3 call sites into the module.
   - Safety P3-4: a public feature would expose the unreviewed kernel in the published crate.
6. **TR1.2's list of questions is incomplete.** The two axes found different sides of it:
   - Consistency P3-1 and P3-9: contract and retry-plan clauses the design must name.
   - Safety P2-5 and P3-6: configuration ownership and agent identity.

## Next action

[GwzTransportReleasePlan-RemPlan.md](GwzTransportReleasePlan-RemPlan.md) maps each finding to one disposition and one closure test. The whole mapping is applied as one revision. The same two reviewers then give a focused re-verdict on the new SHA-256, with the plan and the diff.
