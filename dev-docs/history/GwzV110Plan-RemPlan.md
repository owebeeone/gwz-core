# GwzV110Plan remediation 1

Round-1 reviews, both NO-GO, on plan SHA-256
`a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31`:

- [Consistency](GwzV110Plan-ReviewConsistency.md)
- [Safety](GwzV110Plan-ReviewSafety.md)

One patch to `GwzV110Plan.md`. No product code. Every finding below is
accepted.

Blind convergence: both axes found that S3.3 does not close timeout-plan
S5.2, that Phase 8 can proceed without the Phase 6 aggregate/activation
review, that the dependency sketch drops a required edge, and that the
gearu dependency instructions do not match a crates.io publish.

| ID | Disposition | Closure test |
|---|---|---|
| Consistency P1-1 | One `gearu release` of `git2-rs`. Its publish workflow publishes the sys crate, waits for the index, then publishes the API crate, from that single tag. No second tag and no same-repo `--dependency-tag`. | Phase 8 step 1 is one release; the workflow text states sys then API. |
| Consistency P1-2 | Phase 8 has a step for a separate binding package, used only when S1.1 chooses one. Bindings inside `gwz-transport` are published by the `gwz-transport` step and are not a second release. | Each S1.1 option names the Phase 8 step that publishes the binding before `gwz-py`. |
| Consistency P1-3 | Published Cargo.toml edges are registry `version` pins with no `git` key. Gearu verifies the upstream tag exists. It does not write a git pin. | Phase 2 and Phase 8 require `cargo publish --dry-run` to accept the release commit with no `git =` on those edges. |
| Consistency P1-4 | A new crate name is first published with an operator-held API token, matching `GwzCratesIoPlan.md`. The trusted publisher is configured after the name exists. The token is not stored in the repo and is not the path for later versions. | S2.3 sequences bootstrap, then trusted publisher, then later tokenless publishes. |
| Consistency P2-1 | Phase 4 expands the parent `unix` construction gates to `cfg_if` unix\|windows under `gwz_transport_candidate` so S4.5 can run. Phase 7 removes the candidate switch. It is not the first Windows compile of those sites. | S4.5 names the gates it expands; S7.1 names only the candidate-switch removal. |
| Consistency P2-2 / Safety P1-1 | S3.1 reviews timeout-plan S3.1, S3.3, and S4.1 against §2. The already-rebuilt alpha does not close S5.1; this plan amends only the "before rebuilding" timing. S3.3 imports S5.2's production-graph stall regression as a hard precondition. | S3.1 and S3.3 text match those closure predicates. |
| Consistency P2-3 | S5.3 sits on every path into S5.4. | Sketch and Phase 5 prose both block S5.4 without S5.3. |
| Consistency P2-4 / Safety P2-1 | The sketch is normative. S3.3 and S5.5 both precede S7.1. Prose, sketch, and the Phase 7 header agree. | One reading of the waits. |
| Consistency P2-5 / Safety P1-2 | S7.4 rechecks observation attribution. S7.5 is the dual activation/release review after S7.3 and before any Phase 8 push. | Phase 8 refuses to start without the S7.5 GO. |
| Safety P0-1 | Phase 8 step 1 refuses `--push` unless the vendored libgit2 tree hash equals `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. | The step states the command and the fail-closed rule. |
| Safety P1-3 | S5.6 maps every Design §11 cell on the three platforms to evidence or an explicit unsupported mark. S7.2 cannot advertise an unmarked cell. Real-account `gh` is required before advertising gh-authenticated HTTPS. Fixtures are not that proof. | Ledger rule names S5.6 as its source. |
| Safety P2-2 | Dependent releases name `--dependency-tag NAME=vX.Y.Z` only where a gearu dependency override exists, and the Cargo edge stays a registry version. | Phase 8 shows the `NAME=TAG` form. |
| Safety P2-3 | Phase 8 requires the registry install smoke on Linux x86-64 as well as macOS ARM64 and dabeest. | Post-job paragraph names all three hosts. |
| Safety P2-4 | After any failed push or publish, stop. Do not tag the later product repos. Do not claim v1.1.0 complete. Resume only with a new patch or RC. Never move a tag. | Phase 8 contains that stop rule. |
| Safety P2-5 | S7.2 keeps Windows exact-agent false until a named fixture GO. | S7.2 states the capability and ledger consequence. |
| Consistency P3-1 | On acceptance, amend `CurrentProgramCheckpoint.md` so it names this plan and stops describing the timeout work and Q6 as paused-only. | Landing action after dual GO, recorded in the plan. |
| Consistency P3-2 | S6.2 amends `gwz-py/RELEASE.md` so every native dependency pin is named. Binding crates use a registry version. No sibling path. `GwzCratesIoPlan.md` D7's git-tag-only core pin is not the 1.1.0 form. | S6.2 and Phase 8 step 7 say so. |
| Safety P3-1 | Phases 4, 5, and 8 redact agent-socket paths, known_hosts bodies, and `gh` tokens or headers from retained evidence. A secret in filed evidence fails the step. | One evidence rule covers those phases. |
