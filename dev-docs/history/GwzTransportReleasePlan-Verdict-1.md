# GWZ transport release plan — revision 1 verdict

Date: 2026-09-27. Status: **accepted at SHA-256 `4ec6ba33da5311921edfe15e5e7f8c9b7cd245fec5eb7e9465875997c0ba996d` after [Consistency-1](GwzTransportReleasePlan-ReviewConsistency-1.md) and [Safety-1](GwzTransportReleasePlan-ReviewSafety-1.md) reported GO; this accepts the plan text only**.
- It authorizes no implementation, commit, tag, push or publish.
- The operator decisions OD1–OD10 in the plan's §7 remain open.

The same two reviewers re-verdicted revision 1 with their context intact.
- Each verified the object and the controlling documents by SHA-256, at the start and the end.
- Neither read the other's round-2 report.
- Both ran only inspection commands and `diff`.

| Axis | Verdict | Round-1 findings | New |
| --- | --- | --- | --- |
| Consistency | GO | All 18 closed (P2-1, P2-2, P3-1 to P3-16) | P3-17 to P3-21 |
| Safety | GO | All 12 closed (P1-1, P2-1 to P2-5, P3-1 to P3-6) | none; three residual notes |

Both reviewers confirmed that every hunk of revision 1 maps to a disposition in the [remediation plan](GwzTransportReleasePlan-RemPlan.md). Neither classified anything as architectural, so the two-round cap is not reached. Both cleared their new items to land without a further round: Consistency named the five P3s bounded text corrections, and Safety its three residual notes.

## Corrections applied after the GO

Each was applied as its reviewer specified. The plan then carries the accepted status line and hashes `8277f9ec06342b034041ffb781118308d4b2860fd07f0808158ccf48c215094d`.

1. **Consistency P3-17: TR1.4a's scope and CS3.7's gate.**
   - TR1.4a now revises every session-plan section and step that does not depend on the runtime model, in any phase.
   - TR1.4b takes CS3.7, the session plan's Phase 4, the dependency rewrite for CS3.7 and CS4.1, the panic-model note, and the moved obligations that land there. Verdict-2's second and third carried items move with them.
   - Phase 5 now gives each step one gate.
2. **Consistency P3-18 and Safety residual 3.** §8's closed recovery lists every reuse clause of §1, §2 and Phases 6–10 that it would touch.
3. **Consistency P3-19.** TR1.4a marks each session-plan step that changes ordinary-build behaviour outside the switch; the first to merge to a main triggers §6(b). The retry plan's Phases 1 and 2, already on main, do not trigger it; Phase 2's rule for main governs any release that carries them.
4. **Consistency P3-20.** Phase 2's exception names the retry-plan amendment it needs for the S2.2 and S3.4 help pins.
5. **Consistency P3-21.** TR1.5 resolves the switch's three forms into one value before `open`. That value travels in `SessionOpen`, and the `auto` key and the must-match set use it. Phase 7's exit row runs both the flag form and the environment form.
6. **Safety residual 1.** §6(a) says that `SocketCoreBridge` and gwz-py's `server` command fail closed until Phase 9, because the switch gates the extension's socket host and channel.
7. **Safety residual 2.** TR1.3's `/tmp` fallback: a missing directory is not a refusal. The auto-started host creates it with mode 0700; only an existing directory with another owner or mode refuses.
8. **Consistency residuals:**
   - The step-prefix rule is reworded: only S3 IDs collide.
   - §4's Phase 8 row adds "with Phase 10's replacements".
   - The S6.3 cost row measures each operation's own part of the transport, as TR1.2 names it.
   - TR3.2 is a Phase 10 prerequisite in the text and the sketch.
   - The sketch draws TR8.1's retune loop.
   - §10's Surface list includes TR1.2's conditional Surface.
   - TR1.3 and TR1.5 state which matches which.

## Application

The plan's §10 lists the status-only edits made on this GO, each with a changelog entry:
- **The 1.1.0 plan's and its amendment's status lines:** the supersession, in AgentProcessRules §7.2's pattern.
- **`gwz-py/dev-docs/GwzPyTransportDesign.md`:** its S1.1 sentence and NO-GO closing condition re-pointed at the plan's §4.
- **`dev-docs/GwzCoreSessionPlan.md` and `dev-docs/GwzCoreServerDesign.md`:** a sentence that the transport release carries them.
- **The program checkpoint:** its entry waits until another lane's uncommitted edits to `dev-docs/CurrentProgramCheckpoint.md` land.

## Next action

The operator decides OD1–OD10. The steps that can start now are listed in the plan's §6. Of them, TR1.2's reuse design is already in draft.
