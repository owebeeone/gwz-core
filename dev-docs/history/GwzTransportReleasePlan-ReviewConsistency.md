# GwzTransportReleasePlan — CONSISTENCY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzTransportReleasePlan.md`, uncommitted working-tree file, SHA-256 `90fbd213fa1cde21bb6b5d47aa471488c28b9eeed5dc10da4d43ee8e16a5cceb`. Status in the object: DRAFT plan, review required, no implementation, commit, tag, push or publish authority. Draft-stage review of the text. Date 2026-09-27.

**Baseline:** root `9a65306544e19ee4fbb88757930764950c7494d2`, gwz-core `b13bbadb22c0238f1bc3f88f41c26a79a0669e0c`, gwz-cli `ebbea9025632ba8181df7ddb0bb57ac7b09f862e`, gwz-py `4ad2b077ac473c079a62cdd7d5b7317a78fb5f1f`, gwz-transport `a7a36aec0ec6d31e38647b61567166d612f5d2c5`, git2-rs `08786281876889d42e73ad5a63111d07ef5be8b1`, gwz-git `b43fae2ede31f96a7d920093fe1c12a067eb42dd`. Committed documents and code were read from the working tree of those HEADs (per-repo `git status` confirmed each read file was unmodified, except the eight uncommitted controlling documents, whose SHA-256s matched the tuple). The tuple was verified by `shasum -a 256` and `git rev-parse HEAD` at 10:30:50 AEST and again at 10:46:50 AEST; nothing moved. Working-tree edits in git2-rs, gwz-git and gwz-transport `Cargo.toml` (another lane's rename work) were observed but not reviewed; §3's claims were checked at HEAD as the brief instructs.

**Date:** 2026-09-27

**Axis:** CONSISTENCY — the document against its controlling graph: internal contradictions, verbatim agreement with every cited contract, exactness of the superseded/adopted lists, satisfiability of its own evidence sections, and unstated impacts on documents it does not cite. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 2 P2, 16 P3. Both P2s are bounded text corrections. **I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.** The P3s do not block but every one has a one-paragraph remedy and should ride the same revision.

---

## 0. Evidence base

**Object read in full:** `gwz-core/dev-docs/GwzTransportReleasePlan.md`, lines 1–391.

**Controlling documents read in full:** `gwz-core/dev-docs/GwzV110Plan.md` (446 lines; working tree, whose only diff from HEAD is the "Amended 2026-09-26…" status sentence), `GwzV110PlanAmendment.md` (185), `GwzV110PlanAmendment-Verdict-2.md` (45), `GwzRemoteTransportRetryPlan.md` (467), `GwzRemoteTransportAlphaTimeoutPlan.md` (299), `GwzRemoteTransportPoolCapacity.md` (100), `GwzRemoteTransportBugReport.md` (99), `GwzTransportSequencedStreamDesign.md` (45), `GwzTransportInternalTautCompatibilityAmendment.md` (13), `GwzRemoteTransportQualification.md` (119); `dev-docs/GwzCoreSessionDesign.md` (659; revision 3), `-Verdict-2.md` (94), `-Verdict-3.md` (77), `-RemPlan-2.md` (72), `GwzCoreSessionPlan.md` (700), `GwzCoreServerDesign.md` (417), `GwzClientCoreTransportProposals.md` (256), `GwzRemoteTransportReleaseReadiness.md` (125); `gwz-py/dev-docs/GwzPyTransportDesign.md` (282; working tree, diff is the supersession status).

**Read by section:** `GwzRemoteTransportPlacementDesign.md` §1 header, operator scope clarification and §2 (verbatim "Healthy pooling survives operations within a runtime, not its teardown"); `GwzRemoteTransportHttpsDesign.md` header and §4 (verbatim "If disabled, return the original refusal and never invoke gh. No fallback to another provider."); `GwzRemoteTransportPlan.md` Phase 6 (lines 346–379, exit-evidence paragraph); `GwzRemoteTransportDesign.md` §11–§12 (lines 797–850); `dev-docs/AgentProcessRules.md` §7.2 (1016–1048), §7.3, L1-22 (465–471); `dev-docs/GwzProcessOptimization.md` §4.

**Tree facts checked (commands: `rg`, `grep`, `sed -n`, `git -C … show/log/merge-base/diff/branch -r`, `find`, `curl -s` to crates.io):**
- `rg gwz_transport_candidate` over gwz-core, gwz-cli, gwz-py excluding dev-docs: 35 `cfg(all(unix, gwz_transport_candidate))` sites; two non-unix sites (`gwz-core/src/lib.rs:13`, a `cbor` re-export; `src/protocol/mod.rs:5`, the candidate generated protocol); `transport_host` (`lib.rs:50`) and `git::endpoint` (`git/mod.rs:5`) are unix-gated.
- `DEFAULT_JOBS = 100`, `DEFAULT_MAX_PER_HOST = 32`; `gwz-cli/src/lib.rs:177` `unwrap_or(9)`; `gwz-transport/src/pool/mod.rs` defaults `per_user_host: 32, per_host: 32, total: 256, max_requests: 1024, idle 60_000, connect_timeout_ms: 30_000`; `has_non_idle_lease` refusal in `gwz-core/src/transport_host/session.rs:150,499,614` (retry S1.4 landed); no `max_retries`, backoff, jitter or key-state machine anywhere in gwz-core or gwz-cli source (retry Phase 3 absent); `gwz-cli/src/globalargs/parser.rs:232–233` help text names `--max-retries` and "stalled setup is retried".
- gwz-core `26b30ca6` ("Enable causal HTTPS challenge reuse…", 2026-09-23) and gwz-transport `c4b632d` ("Add candidate sequenced transport message kernel") are ancestors of the tuple HEADs; `gwz-transport/src/lib.rs:20` `pub mod sequenced;`; `gwz-transport/src/binding.rs:183–186, 241–244` call `crate::sequenced::valid_limits` inside `if version == 3`.
- crates.io: `gwz-transport`, `gwz-git`, `gwz-git2`, `gwz-libgit2-sys` each have exactly one version, `0.0.0-bootstrap.1`.
- gwz-core `Cargo.toml` at HEAD: `git2 = { package = "gwz-git2", … }`, `libgit2-sys = { package = "gwz-libgit2-sys", … }`; git2-rs `Cargo.toml` at HEAD: `name = "git2"`, `libgit2-sys/Cargo.toml` `name = "libgit2-sys"`; `gwz-core/tests/transport_backend/prepare.py:47` patch table `("git2", …), ("libgit2-sys", …)`; no `gearu.toml` in the workspace.
- `ssh_network.rs`, `agent_socket.rs`, `ssh_key_auth.rs`, `ssh_local.rs`: `cfg_if! { if #[cfg(unix)] {` with no `windows` arm.
- `gwz-transport/src/pool/mod.rs:85` `Key { scheme, username, host, port }`; `:153` `Owner { session, operation }`.
- `gwz-py/native/src/transport_session.rs` exists; `gwz-py/scripts/process_globals_allowlist.json` has 8 entries, all `debt`, including `CURRENT_SESSION`; no `reconciled_commit` in `process_globals_allowlist_gwz_transport.json`; no `SocketCoreBridge` anywhere; gwz-py console script is `gwz-py`.
- gwz-transport `origin/main` is `46e65a9a888fbd4a5bbeace946996581dcf23333`; local is four commits ahead.
- `GwzRemoteTransportGhChallengeReuseAmendment.md` status: accepted as a design, document only. No Code/State review document exists for the retry Phases 1–2 code, the typed causes or the sequenced kernel; no S5.1 review note exists beside the timeout plan.
- `dev-docs/GwzCoreSessionPlan.md`: 51 distinct `CS` step headings.
- Private evidence member has `transport-qualification/runs/2026-09-23-timeout-clock-baseline` and four other 2026-09-23 runs.
- gwz-cli production source has no reference to `TransportPlacement`, `install_cli` or a client endpoint; `gwz-cli/src/globalargs/dispatch.rs:4–19` reaches the transport only through `with_local_transport`; core's `transport_host/mod.rs:135 pub fn install_cli` has no driver caller.

## 1. Findings

### [P2-1] §2 puts the in-process CLI's `cli` placement in this release, but no production path exists, no step builds one, and the session contract the same release ships excludes client placement

- **Location.** Object line 38 (§2 row 2, left cell: "The in-process CLI supports both placements, `local` and `cli`"); line 12 (§1 outcome); adopted S5.1 ("both placements", 1.1.0 plan 236–240) and S5.6 (263–274); adopted Transport Plan Phase 6 exit rows ("The network-entry ledger is complete for SSH and HTTPS in both placements", `GwzRemoteTransportPlan.md` 366–367); §9 line 372.
- **Violated invariant.** Every in-scope row has an owning step and agrees with the controlling contract. Contract §1 (line 24): "client placement, for which only a transport lane is reserved" is out of scope; O6 (line 76): "Nothing but frames crosses"; §3 (line 94): tags 16–31 "unused by this contract"; session plan §1.3 (line 38) repeats the exclusion; the object's own §9 puts client placement out of scope.
- **Reproduction.** (1) Today `cli` placement is reachable only by tests: core exposes `install_cli` (transport_host/mod.rs:135) but gwz-cli's production path (dispatch.rs:4–19) never installs an endpoint, and the proposals §3 record that client placement "exists only in tests". (2) Phase 5 moves gwz-cli onto the session host; from then on the CLI reaches core through the channel, and nothing but frames may cross. (3) No step of Phases 1–9 designs or wires a `cli`-placed endpoint through the session host (TR1.2 amends the placement design only "where needed" for the pool). (4) S5.6, adopted unchanged: "A mark of unsupported on a cell that §2 lists as in this release fails S5.6 and blocks S7.5 and Phase 8, unless an accepted amendment first removes that cell"; S7.2, adopted: "An unsupported cell, a native route, or a cell with no S5.6 row fails this step."
- **Impact.** As written the plan blocks its own Phase 9 at S5.6/S7.2, or forces an unplanned contract amendment (§1, §3, O6) plus an endpoint-installation design late in Phase 8/9. It also contradicts the object's own §9.
- **Required correction.** Either (a) move `cli` placement to the recorded-unsupported column ("client placement in any process, including an in-process `cli` endpoint; tags 16–31 stay reserved"), restate adopted S5.1's "both placements" and the Transport Plan Phase 6 ledger sentence as `local` only for this release (this plan being the "accepted amendment" that exit row demands), and state that `transport_capabilities.placements` advertises only `local`; or (b) add a Phase 1 design step that gives the `cli` placement a path through the session host (amending contract §1, §3 and O6 and placement design §2) with an owning implementation step and an S5.6 cell.
- **Closure test.** A text check that every left-cell capability of §2 names an owning step; at S7.2 the route ledger has no `cli` row marked as a transport route unless (b) was taken and its evidence exists.

### [P2-2] The 1.1.0 plan's Phase 8 stop-on-failure rule and product-repository ordering paragraph are neither adopted nor retired, and are dropped when the 1.1.0 plan is superseded

- **Location.** Object §4 table (line 90: "Phase 8 steps 1–7 and the post-release check, as the amendment's §3.6 left them"); Phase 10 (296–304); §10 status text (387: "…and Phase 8 as its §4 lists").
- **Violated invariant.** "Nothing the old documents require is silently dropped." The object adopts "by their IDs" (line 5); §10 marks the 1.1.0 plan "Superseded as the release plan", and under AgentProcessRules §7.2 superseded text "is not current".
- **Reproduction.** 1.1.0 plan 362–365: "If any step fails after a push, a GitHub Release, or a registry publish, stop. Do not run a later product tag. Do not claim v1.1.0 complete. Record the published artifact IDs. Resume only with a new patch or release-candidate version. Never move or reuse a tag." and 391–393: "Product repositories, each with the existing release script, tag v1.1.0. Each waits until the previous product crate is visible. Pins are registry versions, not git pins and not sibling paths." Neither has a step ID. The object restates only "depends on S7.5 and S2.3" and "The operator runs it" (Phase 10, §9). §4 and §10 disagree on the scope ("steps 1–7 and the post-release check" vs "Phase 8").
- **Impact.** Recovery after a half-failed release train (for example gwz-core published, gwz-cli's publish job fails) is unspecified in current authority, as is the rule that each product tag waits for the previous crate's visibility and that product pins are registry versions. The 1.0.11 release history shows this class of failure is real.
- **Required correction.** Phase 10 adopts, by quotation, the Phase 8 preamble (stop rule; "the operator runs the commands"; the product-repository order and pin sentence) alongside steps 1–7 and the post-release check; §10's status text names the same scope as §4.
- **Closure test.** From the object alone, a reader can answer "step 5's push succeeded and step 6's publish job failed; what now?" with the stop rule, and "what does gwz-py's release branch pin gwz-core to?" with "the registry version".

### [P3-1] TR1.2's contract-amendment list omits every section whose text rests on the per-operation runtime, and the closing condition cites §14's test whose wording contradicts the release outcome

- **Location.** Object 128 (TR1.2 amends "§1's exclusion of reuse, §5.2's per-operation runtime, §5.6's host context, §15 and §16"); 113 (closing condition: "the contract's §14 overlapping-operations test"); 175 (TR1.7: "§9–§10 as TR1.2 amends them").
- **Violated invariant.** An amendment names every clause it supersedes exactly (AgentProcessRules §7.2; the object's own §4 standard).
- **Evidence.** Contract line 60 (§2 Worker row: "its transport runtime"); 173 (§4.2: "The session host never produces `TransportCapacityConflict`, since each operation has its own runtime"); 522 (§14: "per-operation transport runtimes"); 549 (§14: S6.3's test "each on its own runtime", "The plan text is amended when this contract is accepted"); 551 (§14: the placement guide's reuse premise "needs amending when client placement is scheduled"). None is in TR1.2's list. TR1.7 cites §9–§10, which TR1.2 does not amend. TR1.2 has no closure statement, while TR1.1 "closes on a filed Verdict-4"; G0 of the session plan requires a filed verdict accepting "the contract revision the step implements".
- **Impact.** The post-TR1.2 contract would say both "one runtime per host context" and "each operation has its own runtime"; §4.2's rationale for never mapping `TransportCapacityConflict` becomes false exactly when a shared pool makes capacity conflicts across sessions possible (TR1.2 Q5); the NO-GO closing condition names a test whose text ("each on its own runtime") is false after Phase 6.
- **Required correction.** Extend TR1.2's list with §2, §4.2 (stating whether `TransportCapacityConflict` re-enters and how Q5's caps map to it) and §14; state that the closing condition uses §4's wording; fix TR1.7's citation; give TR1.2 a closure sentence ("closes on a filed verdict accepting the reuse design and the contract's revision N text, with the contract's status edited under §7.2").
- **Closure test.** `grep -n "own runtime\|per-operation\|per operation\|TransportCapacityConflict"` over the revised contract finds no live statement contradicting §5.6's host-context runtime.

### [P3-2] §4's restatement of the moved S6.1–S6.3 obligations drops four clauses of the amendment's §3.4

- **Location.** Object 96–108 versus amendment 80–107.
- **Dropped.** (a) S6.2 (94): "Amend `RELEASE.md` **and the publish workflow** so every native dependency pin is named" and "`GwzCratesIoPlan.md` D7's git-tag-only core pin is not the 1.1.0 form" — the object names `RELEASE.md` only. (b) S6.3's preamble (96): the three platforms with dabeest rows waiting on S4.5, and "Each network test asserts, through the result's transport observations, that the operation took the transport route." (c) S6.3's ninth row (105): "Runtime construction cost and connection counts are recorded for 1, 2 and 8 overlapping operations, for S7.2's notes." (d) S6.1 (82, 84–88): "reports failure with cleanup unconfirmed", and the cancel-before-start and cancel-while-running unit tests.
- **Impact.** TR1.4 is told to map "§4's moved obligations"; what §4 omits is not mapped. (b)–(d) have a landing spot in the contract or session plan; (a)'s workflow half has none (CS4.8 touches `RELEASE.md`, nothing names the wheel workflow's pins). A wheel built from a path or git pin while `RELEASE.md` names the registry pin is caught only by Phase 10's post-release check, after publish.
- **Required correction.** Restate the four clauses in §4, workflow included.
- **Closure test.** A line-by-line table from amendment §3.4 to §4 with no unmapped clause.

### [P3-3] Verdict-2's five carried below-the-bar items were routed to S1.1's revision and S1.2's review, which §4 retires without re-homing them

- **Location.** Object 95; `GwzV110PlanAmendment-Verdict-2.md` 31–38; session plan §5.3 line 599 ("their carried items go to 1.1.0's S1.1 revision").
- **Items.** Whether a waiting operation holds a native thread and its bound; the interpreter-exit bound when `configure_transport_runtime` disables deadlines; the per-operation environment read's race with `os.environ`; which reading of "paths" S7.2's notes use for the native branch; the fixtures S7.3's Linux run needs on the CI host.
- **Impact.** Both routes point at a retired step. The last two apply to adopted and extended steps (S7.3 now runs a server on the CI host); the exit-bound one applies to contract §10's finalizer.
- **Required correction.** §4 names a home for each (TR1.4/contract for the first three; Phase 9's S7.2 and S7.3 for the last two) or records each as moot with the reason.
- **Closure test.** Each of the five appears in the object or TR1.4's revision with an owner.

### [P3-4] No rule for phase numbers in adopted text: "Phase 7" and "Phase 8" there name 1.1.0's activation and release, which are this plan's Phases 9 and 10, while its own Phases 7 and 8 are the server and measurements

- **Location.** Object §1 gives a rule only for "1.1.0". Adopted text: S2.1 (117: "Phase 8 release commit"), S3.3 (182: "S7 and Phase 8 wait on this step"), S4.5 (221: "Phase 7 removes the candidate switch"), S5.6 (271: "blocks S7.5 and Phase 8"), S7.3 (329: "Phase 8 repeats the consumer build"), S7.5 (339: "Phase 8 does not start without this GO"), exit-row table (350: "S7.3, Phase 8"), amendment §3.6 step 7.
- **Impact.** Read under this plan, S5.6 "blocks … Phase 8" blocks the phase that contains S5.6; S4.5's "Phase 7 removes the candidate switch" points at the server phase.
- **Required correction.** Add beside the version rule: "In adopted text, 'Phase 7' means this plan's Phase 9, 'Phase 8' means Phase 10, and 'S7' means S7.1–S7.5."
- **Closure test.** Every "Phase N" in adopted text resolves to the intended TR phase under the rule.

### [P3-5] Adopted amendment text still names retired S6.x steps without restatement, and the amendment's S7.2 addition is neither adopted nor retired as a whole

- **Location.** Object 89 (adopts "the amendment's S7.1 site list and check, its first S7.3 addition, and its exit-row change" — the S7.2 addition is absent); 110 (retires one S7.2 sentence only); 292–294; 298. Amendment 115 ("S6.1's variant and S6.2's arms"), 121 ("Its evidence is S6.3's and S7.3's route assertions … entered here on S6.3's assertions"), 124 ("S7.2, S6.3, S7.3"); 1.1.0 plan 403 (step 7: "Wheels use the pins S6.2 wrote into `RELEASE.md`"). The amendment has one S7.3 addition, not a "first".
- **Impact.** The adopted Python-ledger evidence rule and exit row cite a step that no longer exists; the site list omits the session entry's sites; the rest of the S7.2 addition has undefined standing.
- **Required correction.** §4 adopts the amendment's S7.2 addition explicitly, retiring its S6.3 references and restating the Python rows' evidence as Phase 5's route tests and S7.3; Phase 9's exit-row mapping replaces S6.3 rather than adding beside it; S7.1's site list gains "the session entry and its arms (TR1.4's steps)"; Phase 10 step 7 points at the moved pin obligation; "first S7.3 addition" becomes "S7.3 addition".
- **Closure test.** `grep -n "S6\.[123]"` over the object's adopted-text references finds only §4's retirement list.

### [P3-6] S2.3's and Phase 8 step 1's "first publish uses an operator-held token" are adopted unchanged, although §3 and Phase 3 record that every name's first publish already happened

- **Location.** Object 59, 210 ("each name has had its first publish, and the trusted publisher can be configured before the first real version"); 1.1.0 plan 143–147 (S2.3) and 373–374 (step 1: "The first publish of each new name uses the operator-held token from S2.3"), both adopted without change.
- **Impact.** Phase 10 step 1 tells the operator to token-publish a first version that is not a first publish; S2.3's exit does not state that the trusted publisher must be configured before Phase 10, so the tokenless path the plan relies on is nobody's step.
- **Required correction.** State that S2.3's token clause is satisfied by the bootstrap publish, that S2.3's exit includes a configured trusted publisher per name, and that step 1's token sentence is replaced.
- **Closure test.** Phase 10 step 1 has no token sentence; Phase 3's exit lists the four trusted publishers.

### [P3-7] §10's status texts do not follow AgentProcessRules §7.2's exact supersession pattern, do not match §4's scope, and leave `GwzPyTransportDesign.md` pointing at retired steps until TR1.4

- **Location.** Object 386–390; AgentProcessRules 1016–1048 ("Use these exact patterns": "Status: **superseded for <exact scope> by `<NewDocument.md>` as of <date>. Historical evidence and already-completed gates remain valid only where the new document says they do**"); object 175 (TR1.7 defers the gwz-py design's edit to TR1.4's acceptance); gwz-py design status (lines 3–5) cites S1.1, S1.2 and S6.3.
- **Impact.** The amendment's proposed sentence names only S1.1/S1.2/S6.1–S6.3 while §4 also retires its §3.1 rows, §3.7–§3.9, §6 and the S7.2 sentence; the 1.1.0 plan's sentence says "Phase 8" where §4 says "steps 1–7 and the post-release check"; the gwz-py design's NO-GO closing condition and "S1.1 revises this document" sentence are stale from this plan's GO, not from TR1.4.
- **Required correction.** Use the §7.2 pattern for both superseded documents with scopes equal to §4; add a GO-time status edit for `GwzPyTransportDesign.md` that re-points its S1.1/S1.2 sentence and NO-GO condition at this plan's §4.
- **Closure test.** The three status lines contain the §7.2 pattern verbatim and their scope lists equal §4's.

### [P3-8] §2 row 5 lets TR1.2 change the 60 s idle default while adopted S5.4 and two in-force authorities forbid it, and no supersession is named

- **Location.** Object 41 ("a changed 60 s idle default, unless TR1.2 amends this row"), 145 (Q9), 368; 1.1.0 plan 254 (S5.4, adopted unchanged: "The 60-second idle default stays"); Transport Plan Phase 6 (in force per §1): "Runtime defaults cannot override the agreed 60-second idle default"; release readiness item 2: "Preserve the agreed 60-second idle default"; transport design §12 "Tuning | 60 s idle".
- **Impact.** If TR1.2 amends the default, S5.4's evidence note would contradict the step's own adopted text and two authorities the plan keeps in force, with no clause named as superseded.
- **Required correction.** Q9 states that changing the idle default also supersedes S5.4's sentence and the named Transport Plan/design/readiness sentences, by quotation.
- **Closure test.** Every "60" idle sentence in the in-force set is either unchanged or named in TR1.2's supersession list.

### [P3-9] §1 keeps the retry plan in force unqualified while TR1.2 Q5 must replace its S1.4/§3.7/§6 refusal rule, which the retry plan's own closing note fixes "as written"

- **Location.** Object 27, 141; retry plan 115–133 (§3.7 replacement: "While a lease is non-idle, a request cannot raise them; that operation is refused"), 336–339 (§6), 415 (S1.4), 467 ("§3 item 7, §6 and S1.4 stand as written"); live code `gwz-core/src/transport_host/session.rs:614` (`has_non_idle_lease` refusal).
- **Impact.** The refusal is committed code. In a shared host-context pool the second session's first operation is refused whenever another session holds a non-idle lease; the plan says TR1.2 "cannot" keep the rule but keeps the document that fixes it, and TR2.1 implements retry Phase 3 "as written" on a per-operation Closed state that Q6 may widen.
- **Required correction.** §1's retry-plan line reads "as TR1.2 amends its S1.4, §3.7 and §6"; TR1.2 Q5 names those clauses as superseded by quotation and states what replaces the refusal.
- **Closure test.** TR1.2's supersession list quotes the three retry-plan clauses; a Phase 6 exit test has two sessions' operations admitted while a lease is non-idle.

### [P3-10] TR1.4's revision list omits session-plan text the plan's own decisions invalidate, and its G2 instruction contradicts Phase 5's prerequisites

- **Location.** Object 157–163; session plan §1.1 (11–18, "What 1.1.0 already does"), §1.3 (37, 39, 41: S6.x out of scope; the server "a follow-on, not a phase"; "Any change to gwz-transport … or the public Python API"), §2.4 (76), CS3.1 (287), CS3.7 (326: "Depends on … 1.1.0 S6.1"), CS4.1 (374: "Depends on G2 (the 1.1.0 tag)"), R1 (655), §5.3 (599).
- **Reproduction.** TR1.4 says "It rewrites G2, since there are no 1.1.0 steps to build on", but the object's Phase 5 says "the transport tests in the session plan's Phase 3 need TR3.1" and Phase 7 says "Windows follows Phase 4", and CS3.11's dabeest route proof needs S4.5's Windows arms. Phase 6 needs "every gwz-transport change TR1.2 names" (session plan §1.3 forbids gwz-transport changes); §2 row 6 adds `SocketCoreBridge` to the public API (§1.3 forbids public-API changes).
- **Impact.** A TR1.4 that applies only the five listed bullets leaves the session plan describing S6.x as done, the server as a follow-on, gwz-transport as frozen, CS4.1 waiting on a tag that no longer precedes it, and a G2 that omits its real prerequisites.
- **Required correction.** TR1.4 enumerates §1.1, §1.3, §2.4, CS3.1/CS3.7/CS4.1's 1.1.0 dependencies, R1 and §5.3; G2 becomes "TR3.1 (candidate build) and S4.5 (Windows arms) for transport rows; no tag precedes any phase".
- **Closure test.** `grep -n "1\.1\.0 S6\|1\.1\.0 tag\|follow-on"` over the revised session plan returns nothing unrevised.

### [P3-11] §6's "What can start now" lists steps whose failing tests cannot be written until TR3.1, which §3 and §8 say blocks every transport build

- **Location.** Object 327–329 (TR2.1, TR2.2's reproduction, S4.2–S4.4 "can start now"); 62 and 367 ("breaks the candidate build", "blocks transport builds until TR3.1"); 179; `gwz-core/src/git/mod.rs:5` and `src/lib.rs:50` (endpoint and transport_host compile only under the candidate cfg); gwz-core `AGENTS.md` (TDD-first).
- **Impact.** TR2.1's retry machine lives in the endpoint, TR2.2 reproduces "on the current tree", and S4.2–S4.4 compile Windows arms "by the same tests the Unix module already has"; none can start its first failing test before TR3.1 lands. The sketch's "no step starts before its gate" is violated by its own prose.
- **Required correction.** "What can start now" reads "TR3.1; then TR2.1, TR2.2's reproduction, S4.2–S4.4; TR2.3 and TR2.4 need no transport build." (TR2.3 is gwz-core's result type; TR2.4 is gwz-transport.)
- **Closure test.** Each start-now step's first named test builds at the tuple plus only the steps listed before it.

### [P3-12] TR2.4 omits `binding.rs`'s two profile-3 call sites of `sequenced::valid_limits`, so "the module behind a non-default feature" as written breaks the default build or leaves profile-3 Bind half-enabled

- **Location.** Object 187–190; `gwz-transport/src/binding.rs:183–186, 241–244` (`if version == 3 { crate::sequenced::valid_limits(...) }`); `sequenced.rs:59` (`pub(crate) fn valid_limits`).
- **Impact.** Feature-gating `sequenced` without gating those branches fails to compile; gating them without a refusal rule leaves a `Bind.versions=[3]` offer negotiable with the reassembly kernel absent, contrary to the sequenced design §1 ("An older endpoint rejects UnsupportedVersion before Git, credential or pool effects; there is no silent downgrade").
- **Required correction.** TR2.4 names `binding.rs`, puts the two branches in the same `cfg_if` boundary, and states that version 3 is refused before Git, credential or pool effects when the feature is off; the "under 100 lines" budget stands.
- **Closure test.** Default build compiles; a test that Bind 3 is refused without the feature and accepted with it.

### [P3-13] Surface coverage is enumerated inconsistently across §2, §5 and §10

- **Location.** Object 385 (TR1.6 carries Surface "where it changes behaviour") vs 174 (TR1.6's review: Consistency and Safety only; OD10 says either option changes behaviour on upgrade); 186 (TR2.3 changes `gwz-cli/docs/MachineOutput.md`, a documented machine contract, with no Surface named); 233 (Phase 6 exit adds physical-connection and channel counts to `--verbose` rows; S7.5's enumerated scope at 291 omits them); 175 (TR1.7 lists two "changed documented behaviour" items for gwz-py) vs session plan R8 (662: cancel codes, `transport_session_full`/`operation_expired`, the D4 pre-gate, interpreter exit waiting up to the close bound, `git` on `PATH`) and CS4.5 (405: `NativeCoreBridge`'s host-context parameter, "the one Python-visible addition") vs object 42 ("unchanged apart from the new `SocketCoreBridge`").
- **Impact.** A user-visible change with no named Surface owner is the class of gap S7.5 exists to close; §2's API claim is false by the session plan's own count.
- **Required correction.** TR1.6's review line adds Surface; TR2.3 names Surface for the `errors` contract or routes it into S7.5's scope; S7.5's list adds the `--verbose` row fields; TR1.7 cites R8 and CS4.5 instead of listing two items; §2 row 6 says "apart from `SocketCoreBridge` and `NativeCoreBridge`'s optional host context".
- **Closure test.** Every user-visible change named in the object, the server design and session plan R8 appears in exactly one Surface scope.

### [P3-14] §6's merge rule ("lands switched off until Phase 9") names no switch, and S7.1's completeness check covers only `gwz_transport_candidate`

- **Location.** Object 332; 270–272; amendment 119 (`rg gwz_transport_candidate`).
- **Impact.** The server command, `--server`, `GWZ_SERVER`, `--no-server`, `SocketCoreBridge`, `--max-retries` and the off switch are not transport-cfg code by nature. A step that lands one behind a different gate either leaks before Phase 9 or leaves a gate S7.1's check never sees.
- **Required correction.** Name the switch (the candidate cfg extended to those sites, or a named alternative) and add it to S7.1's `rg` pattern and site list.
- **Closure test.** S7.1's check pattern matches the switch every Phase 2–7 step used; a grep for the alternative finds nothing after S7.1.

### [P3-15] Phase 8's sign-off relies on "the server and reuse cells that TR1.2 and TR1.3 name", but neither step is required to name Design §11 cells

- **Location.** Object 264; 132–148 (TR1.2's twelve answers; Q12 names contract §15 rows only); 150–156 (TR1.3's change list); 1.1.0 plan 263–274 (S5.6 is a cell table over Design §11).
- **Impact.** S5.6 cannot gain cells nobody produces; without them the server and reuse ship without a sign-off row, which S5.6's rule ("nothing is advertised without evidence") forbids.
- **Required correction.** TR1.2 Q12 and TR1.3 each add "the Design §11 cells (S5.6 rows) this design adds, with their evidence kind".
- **Closure test.** S5.6's table at Phase 8 has a server row and a reuse row traceable to TR1.2/TR1.3 text.

### [P3-16] TR1.6's recommended native route for non-gh helpers contradicts Design §11's HTTPS cell and adopted S7.1, and §2 does not record the route

- **Location.** Object 173, 355 (OD10), 38 (§2 row 2 right cell omits it); Design §11 HTTPS row: "`gh` auth; **other helpers rejected**"; 1.1.0 S7.1 (313–315, adopted): "Credential locality and gh-only authenticated HTTPS stay as designed"; Transport Plan Phase 6: "Do not silently relax that policy".
- **Impact.** Under the recommended option S5.6's HTTPS cell expects a rejection the product no longer performs; S7.1's "stay as designed" is false; S7.2 fails on "a native route" unless the route is listed as unsupported, which §2 does not do.
- **Required correction.** TR1.6's amendment scope adds Design §11's HTTPS cell and S7.1's sentence; §2's right cell records "HTTPS through a credential helper other than gh (native route, listed in the migration notes)" under OD10's recommended option.
- **Closure test.** S5.6's HTTPS cell and S7.2's ledger row for non-gh helpers agree with TR1.6's accepted text.

## 2. Invariant analysis

Attacks that **failed** (the invariant held):

- **§3 starting-point facts against the tree.** Every checkable claim held at the tuple HEADs: 100/32 defaults, the 9 s stall and 30 s aggregate, the S1.4 cap install, the absence of retry Phase 3 and of `--max-retries`, the help text naming it, commits `26b30ca6` and `c4b632d` as ancestors, `pub mod sequenced`, the four `0.0.0-bootstrap.1` placeholders, gwz-core's new-name dependencies, git2-rs's `git2`/`libgit2-sys` manifests, `prepare.py`'s old-name patch table, no `gearu.toml`, no Windows arm in the four SSH modules, `pool::Owner{session, operation}` with a `Key` lacking endpoint configuration, the typed `SetupFailureCause`, `transport_session.rs` and the `CURRENT_SESSION` debt entry, the 2026-09-23 timeout evidence, the gh-challenge design's document-only GO, the sequenced design's contract-only GO, the absence of Code/State reviews and of an S5.1 note, the three bug-report defects and the 6.9 s / 3.1 s pool numbers, and 51 session-plan steps. The "on Unix" qualifier is accurate for the transport proper; two non-unix candidate sites (`cbor` re-export, candidate generated protocol) are covered by S7.1's "every site".
- **Verbatim quotations.** The placement design §2 quote, the HTTPS §4 refusal sentence, OD9's G1 wording (server design §8), TR8.1's targets against the pool brief, defect 3's five-fetch check against the bug report, and the contract's §14 test sentence are exact.
- **The "1.1.0 means the release version" rule.** Applied to every adopted sentence that carries the token (S2.2's tag sentence, S3.1's tag sentence, S5.5, S5.6, S7.1, Phase 8 steps 5–7 and the post-release check, S6.2's moved pin), none breaks; the plan's own step prefixes ("1.1.0 S4.2") are outside adopted text.
- **1.1.0 §5 to §9.** All seven bullets and the amendment's §3.7 additions map, with the idle default deliberately made conditional (P3-8 concerns the unnamed supersession, not the mapping) and the session-contract bullet correctly brought into scope.
- **The Windows and redaction paragraphs.** Restated in §2; the redaction rule's phase set is a strict superset of the amended 1.1.0 rule (Phase 2's live fetches are now covered).
- **Internal IDs.** Every TR, OD, CS, S and § reference resolves; OD1–OD10 each have a consuming step; no dangling section number.
- **Dependency sketch.** Acyclic; Phase 1's chain, Phase 2's TR2.2/TR2.5/TR2.6 edges, Phase 3's TR3.1 edges, Phase 5's gates, Phase 6's and Phase 9's dependencies, and Phase 10's S7.5+S2.3 edge all match the prose.
- **§2 rows 3, 4 and 7** agree with the contract, server design §1/§3/§4 and the crate-identity steps. OD2's recommendation matches server design §7's existing opt-in default.
- **Phase 7's exit** matches server design §12 and its §5 release gate; Phase 10's post-release lifecycle matches §6's command table; `gwz-py server` matches the actual console-script name.
- **§4's S6.3 rows** that were restated are correctly altered for the session model (environment at open; waits under session limits; close/exit without a helper process).

Attacks that **succeeded** are the findings above. Their pattern: the plan is exact where it adopts by ID and loose where the old documents' obligations had no ID (P2-2, P3-2, P3-3), where adopted text carries a cross-reference the plan invalidated (P3-4, P3-5, P3-6), and where one of its own decisions reaches into a document it does not list as amended (P2-1, P3-1, P3-8, P3-9, P3-10, P3-16).

## 3. Risks and next action

Residual, below the finding bar:
- Two S3.3s exist (1.1.0 and retry); §4 line 102 and Phase 2 line 199 use "S3.3" unprefixed. A prefix costs nothing.
- Adopted S3.3's live cold fetches run in the same phase as TR2.1; a retried stall no longer fails a member, so the two-clock evidence must come from the production-graph regression (which S3.3 already makes the gate) or from runs at `--max-retries 0`. Say which.
- OD3 says the `reconciled_commit` pin "moves when gwz-transport is pushed"; RemPlan-2's bump rule moves it only in a gwz-core commit that changes the allowlist.
- §1 says the timeout plan is closed by "1.1.0 S3.1–S3.3"; S3.2 is the Q6 review, not a timeout-plan step.
- Pageant is placed in Phase 7 although it is agent-channel code in S4.3's file, after TR2.6's transport review has closed.
- §2 row 3 omits the in-process CLI's host context (reuse within one command), which TR1.2 Q1 names.
- The §4 retirement of the amendment's §6 has no named replacement in the four-item list at line 109.
- git2-rs and gwz-git carry uncommitted rename edits in another lane; §3's "half landed" is true at HEAD and TR3.1's ownership sentence covers it, but "at the tuple HEADs" would remove the ambiguity.
- A 1.0.x patch cut from main after TR2.3 lands would change the machine-output `errors` contract in a patch release; §6's "Defect fixes need not" land switched off should say so.

**Next action:** one revision of the object that (1) resolves P2-1 by choosing remedy (a) or (b) and P2-2 by adopting the Phase 8 preamble by quotation, and (2) applies the sixteen P3 text corrections, followed by a focused Consistency re-check of §1, §2, §4, §5 Phase 1, §6 and §10. On (1) alone I pre-commit to GO.
