# GwzV110PlanAmendment — SAFETY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzV110PlanAmendment.md`, uncommitted draft in the working tree (untracked in gwz-core), 110 lines, SHA-256 `774bb164c1c33122738b6864cabcc81203404bd25372bebe32b3984c1cdd7acf`. Status line: "draft; not implementation authority". Hash verified identical at start (22:47 AEST) and end (23:04 AEST) of the review.
**Baseline:** root `9a65306544e19ee4fbb88757930764950c7494d2`; gwz-core `b13bbadb22c0238f1bc3f88f41c26a79a0669e0c`; gwz-py `4ad2b077ac473c079a62cdd7d5b7317a78fb5f1f`; gwz-cli `ebbea9025632ba8181df7ddb0bb57ac7b09f862e`; gwz-transport `a7a36aec0ec6d31e38647b61567166d612f5d2c5`. All unchanged start to end. Controlling plan read with `git -C gwz-core show b141e26d:dev-docs/GwzV110Plan.md` (SHA-256 `583dd308…`, equal at b141e26d, gwz-core HEAD and the working tree; the amendment's §1 hash claims check out). Contract revision 3 read with `git show 5a5d6cf:…` (SHA-256 `80e1c06a…`, equal to root HEAD and working tree); revision 2 with `git show 58ea74b:…` (`23b251ea…`); the rev2→rev3 delta with `git diff 58ea74b 5a5d6cf`. Inspection only: `shasum`, `git show/diff/log/rev-parse/status`, `grep`, `sed`, `cat`. No build, test, edit or git mutation.
**Date:** 2026-09-26
**Axis:** SAFETY — what the amended plan text permits to go wrong: gates, ordering, mixed-version and degraded paths, release evidence, disclosure, blast radius. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0×P0, 0×P1, 1×P2, 6×P3 open. I pre-commit to GO on a revision that resolves P2-1 as specified below; the P3 findings are nonblocking and may be carried into that revision or into the contract's implementation plan with their closure tests.

---

## 0. Evidence base

| Source | Identity | Role |
|---|---|---|
| `gwz-core/dev-docs/GwzV110PlanAmendment.md` | SHA-256 `774bb164…` | The object |
| `gwz-core/dev-docs/GwzV110Plan.md` | gwz-core `b141e26d`, SHA-256 `583dd308…`; accepted text `9d49af85…` per its status line | Controlling document; line numbers cited are this file's |
| `dev-docs/GwzCoreSessionDesign.md` | root `5a5d6cf` (revision 3); `58ea74b` (revision 2) | The design the amendment adopts; content deferred, only the plan's use of it reviewed |
| `dev-docs/GwzCoreSessionDesign-Verdict-2.md` | root HEAD | Accepts revision 2 on both axes; carries ten P3 findings into "the phased plan" |
| `dev-docs/GwzClientCoreTransportProposals.md` §2, §8, §9 | root HEAD | G1–G11; recommended shape; phases 1–6 |
| `dev-docs/AgentProcessRules.md` §7.1, §7.2; `dev-docs/GwzProcessOptimization.md` §4 | root HEAD | Process authority: amendment content, status language, remediation cap, review tiers |
| `gwz-core/dev-docs/GwzV110Plan-ReviewSafety-2.md`, `-ReviewConsistency-2.md` | gwz-core HEAD | The plan's accepted reviews |
| `dev-docs/GwzRemoteTransportReleaseReadiness.md` items 2–5; `gwz-core/dev-docs/GwzRemoteTransportPlan.md` Phase 6 exit; `gwz-core/dev-docs/GwzRemoteTransportDesign.md` §11 | HEAD | Authorities the plan keeps in force; the exit rows the plan's Phase 7 table maps |
| `gwz-py/dev-docs/GwzPyTransportDesign.md`, `gwz-py/RELEASE.md`, `gwz-py/.github/workflows/*.yml`, `gwz-py/scripts/package_smoke.py` | gwz-py HEAD | The documents and jobs the amended S6.2/S6.3 rely on |
| `gwz-core/src/lib.rs:50`, `transport_host/mod.rs`, `transport_host/local_command.rs:12–45`, `transport_host/request.rs:299`, `gwz-cli/src/globalargs/dispatch.rs:4–19, 353–366`, `gwz-py/native/src/dispatch/mod.rs:483–500`, `gwz-core/scripts/checks/` | HEADs | The cfg sites, the entry, the fallback, and the existing check inventory the amended Phase 6 text names |

Facts established that the findings rest on:
- Both product entries fall back silently to libgit2's native backend when the transport cfg arm is absent: gwz-cli `dispatch.rs:6–19` (`cfg_if` block, then `execute_with_backend(…, Git2Backend::new(), …)`), gwz-py `dispatch/mod.rs:483–500` (`else { call(…) }`). Contract §5.2 keeps this shape: "Otherwise the handler runs with the default backend."
- `TransportRequest` and `with_local_transport` live in gwz-core, so the contract's new token-taking constructor is a gwz-core change; Phase 8 step 2 (gwz-transport) has no hidden dependency on Phase 6. Phase 8 as a whole still waits on S7.5, which waits on S6, so no crate can be published before S6.1 lands.
- `TransportCapabilitiesResponse` today has only `file_identity` and `exact_agent_identity`; revision 3 appends `cancellation` (§13), the only typed field from which "the session transport entry is live" can be read.
- gwz-py's CI matrix runs `run_tests.py` on windows-2022, but its smoke clone is `file://` and no test asserts a transport route. The plan's Windows authority is dabeest, not GitHub runners.
- `gwz-core/scripts/checks/` contains no syntax-aware cfg-boundary check (the workspace rule in AGENTS.md asks for one); the process-globals, filesystem-boundary and crate-version checks exist.
- The revision 3 re-verdict files the amendment names (`…-ReviewConsistency-3.md`, `…-ReviewSafety-3.md`) do not exist yet. Revision 3's §15 already contains the regression tests for the ten P3 findings that Verdict-2 carried forward.

---

## 1. Findings

### P2-1 — After the amended S6.1, either shipped transport entry can reach 1.1.0 on Windows x86-64 as a silent native route, and no step re-establishes per-platform route evidence

**Location.** Amendment §3.4, lines 55–65 (Phase 6 replacement, especially "S6.1's network half edits the transport entry that S4.5 splits into Unix and Windows arms. Whichever of the two lands second adapts to the first. Neither waits for the other."), together with §3.8 line 92 ("Phase 7 … unchanged") and §4 line 100 ("S3, S4, S5 and S7 are unaffected").

**Root cause.** The amendment makes S6.1 an editor of both transport-entry sites — it adds the session variant beside gwz-core's `with_local_transport` (contract §5.2, §16) and moves gwz-cli's `execute_invocation`, whose transport-scope branch is the CLI's entry, into gwz-core as the shared dispatch (contract §5.2) — and permits that edit after S4.5 and S5.5, which are the plan's only Windows functional evidence. It then declares Phase 7 unchanged, although S7.1 "does not introduce Windows into those sites; S4.5 already did" (plan 311–312), S7.3 is build-only (325–329), S7.4 and S7.5 name no platform, and Phase 8's post-release check requires only that "the route ledger's advertised commands work" (405–409), not that they took the transport route. S6.3's Python tests name no platform; the contract's only platform-naming item (§15.11) says Linux and macOS.

**Violated invariant.** Plan §1: the transport is "supported behavior of the normal gwz, gwz-core, and gwz-py builds" on the three §2 platforms. Transport Plan Phase 6 exit: "no advertised route silently remains native" (the plan maps this row to S7.2 only). Release Readiness item 5: gh-only HTTPS and credential locality "into the actual shipped CLI/core/Python paths … through real consumers". Plan S7.1: "Credential locality and gh-only authenticated HTTPS stay as designed."

**Interleaving (permitted by the amended text).**
1. S4.5 lands: the `cfg(all(unix, gwz_transport_candidate))` sites (`lib.rs:50`, `transport_binding.rs:9`, `gwz-cli/dispatch.rs:6,353`, …) become `cfg_if` unix|windows arms; the dabeest fixtures pass. S5.5 and S5.6 record Windows evidence on this tree.
2. S1.2 closes; S6.1 lands afterwards ("Neither waits for the other"). Its local half moves `execute_invocation` into gwz-core, re-creating the CLI's transport-scope branch there; its network half adds the session variant. Either lands inside the pre-existing unix arm, or a textual merge of the two lanes re-attaches a boundary (the `cfa14b8` failure mode the workspace rule exists for; no syntax-aware check guards it). Nothing in S6.1 runs on dabeest.
3. S6.3 passes on macOS and Linux. On gwz-py's windows-2022 job the smoke clone is `file://` and nothing asserts the route.
4. S7.1 removes `gwz_transport_candidate` from the conditions and adds no Windows arm. The session variant, and the moved CLI branch if step 2 lost its arm, are now `cfg(unix)`.
5. S7.3 builds the Windows CLI and extension on dabeest: both compile, because the fallback arm exists. S7.2's ledger cites S5.6's pre-S6.1 evidence. S7.5 reviews text.
6. Phase 8 publishes. The post-release check installs on dabeest and runs the ledger's commands, which succeed — natively.

**Impact.** A released 1.1.0 wheel, or CLI, on an in-release platform whose SSH and HTTPS operations run through libgit2/libssh2: gh-only HTTPS is not enforced (native HTTPS consults git credential helpers), agent and known_hosts handling is not endpoint-owned, the stall and aggregate clocks and cancellation are absent, and every test the plan names still passes. Detection would be by a user report.

**Required correction (text).**
(a) S6.1 exit: the S4.5 fixture pass (SSH and HTTPS clone/fetch on dabeest) is repeated on the S6.1 tree for the CLI entry, whichever of S4.5 and S6.1 lands second.
(b) S6.3: the overlapping-operations, gh-failure and unsupported-proxy tests run on all three §2 platforms including dabeest, with a typed assertion that the operation ran through the session transport entry (`transport_capabilities.cancellation == true` and the transport attribution in the result or observations), or an explicit unsupported mark that, under S5.6's rule, blocks Phase 8 unless §2 is amended.
(c) S7.2's route ledger gains a Python-entry row per platform, and the Phase 7 exit-row table maps "no silent native route" to S6.3 as well as S7.2.
(d) Phase 8's post-release check adds one CLI and one Python network operation per host that assert the route, not only that the command succeeds.
(e) The second-landing lane's edit of the cfg sites is verified before merge by a syntax-aware boundary check that inspects the disabled arm, or by (a)/(b) on Windows.

**Closure test.** On dabeest, with the S7.3 (later the released) Windows extension, a Python fetch against the SSH fixture reports the transport route and a gh failure on the HTTPS fixture refuses; a build with the session variant's Windows arm deliberately removed fails that test instead of passing natively. The same for the CLI on the S6.1 tree via `gwz fetch`.

### P3-1 — S1.1 names the contract by path; S1.2's closure predicate is satisfiable by revision 2 today and undefined for a revision produced after closure

**Location.** Amendment lines 39–43.

**Violated invariant.** AgentProcessRules §7.1 (an amendment names "documents controlled … mandatory review and hash") and §7.2 ("a reader can identify one current authority"); plan §4 "S6 waits on S1.2".

**Sequence.** Verdict-2 records both axes GO on revision 2 (root `58ea74b`). An implementer reads "S1.2 closes when both axes report GO on the revision that implementation follows", declares that implementation follows revision 2, and starts S6.1 while the revision 3 re-verdict is pending — on text without the ten corrections (lossy Windows environment capture, host context created "on first use", libgit2's credential helper run with the live environment, W collisions across sessions sharing a host context). "On this amendment's date that is revision 3" is descriptive, not a rule, and the plan never states which §15 list S6.3 follows. Conversely, if S1.2 closes on revision 3 and a revision 4 appears during S6.2, the predicate flips false with S6.x in progress; the text gives no rule for continuing, pausing or re-reviewing, and its restated "capped at two rounds" omits GwzProcessOptimization §4.1's permitted third bounded round, so a NO-GO on revision 3 has no stated path.

**Impact.** P3-class design defects can be implemented lawfully under the plan's letter; an auditor cannot decide from the plan whether Phase 6 started on the reviewed object.

**Required correction.** S1.1 names the revision by root SHA (`5a5d6cf`, revision 3) and says a later revision counts only once a filed verdict accepts it on both axes. S1.2 closes on a verdict document naming that SHA; a revision after closure is a contract amendment under its own dual review, S6.x continues on the last accepted revision meanwhile, and a draft revision carries no implementation authority. State the path on a NO-GO of the -3 re-verdict (a third bounded round under §4.1, or the lane stops and Python's row needs a scope amendment).

**Closure test.** The amended S1.2 cites a SHA, and the revision-2 reading above cannot be constructed from the text.

### P3-2 — The implementation-plan gate before S6.1 has no review rule and no verdict requirement

**Location.** Amendment line 57: "The contract's implementation plan divides each into steps under it, and is written and reviewed before S6.1 starts."

**Violated invariant.** GwzProcessOptimization §4.2: a checkpoint's review tier "is recorded … when its interface freezes — it is not chosen mid-lane by the implementer"; amendments and interface freezes are dual. Contract §12: "The implementation plan lists the assertions that change." Verdict-2 next action 2 directs the ten P3 regression tests into that plan.

**Sequence.** The implementation plan is written; a single informal read counts as "reviewed"; S6.1 starts. The plan rewrites existing gwz-py assertions (contract §12 names one: a foreign cancel now reports `operation_not_found`) and sets step budgets and ownership for three milestones each above 500 lines, with no verdict on record.

**Impact.** Behaviour changes to existing gwz-py tests, and the step split of the largest code change in 1.1.0, enter without a filed GO.

**Required correction.** Name the rule: dual peer-blind Consistency and Safety with GO before S6.1's first commit; the plan lists the rewritten assertions and maps the ten P3 regression tests to contract §15 items; the tier is recorded in the program checkpoint.

**Closure test.** The implementation plan's status line cites two GO reviews dated before S6.1's first commit.

### P3-3 — Phase 6's newly promoted release evidence sits outside the plan's evidence rules

**Location.** Amendment §4 line 98 ("The contract's §12 two-bridge CI run becomes release evidence for gwz-py"); §3.8 line 91 keeps only the Python-visible-errors clause.

**Violated invariant.** Plan §2 lines 62–64: "Phases 4, 5, and 8 redact agent-socket paths, known_hosts bodies, and gh tokens or headers from retained evidence. A secret in filed evidence fails that step." S7.3: "A path-only developer build is not the release proof." Phase 7 exit row "Attributable results at exact revisions".

**Sequence.** The two-bridge run executes on gwz-py `main` (path-pinned core, example binary from the sibling checkout) or in `publish.yml`. `StreamCoreBridge` starts the host binary with the bridge's full `os.environb` snapshot (CI runner tokens, the gh fixture's token). A failing run's captured stderr, backtrace or fixture log is retained as the release evidence. No redaction gate applies because Phase 6 is not a named phase, and the plan does not say which run, against which core, is the evidence.

**Impact.** Secrets or host-identifying material in filed release evidence with no failing step; or release evidence taken on a path-built core and cited as release proof.

**Required correction.** Extend §2's evidence rule to Phase 6 (S6.3's retained CI runs, fixture logs and the host binary's stderr). State that the release-evidence run is the one against the registry-pinned `gwz-core = "=1.1.0"` at the tag with the host binary built from the same tag, and that path-pinned runs are development evidence only.

**Closure test.** Filed S6.3 evidence passes the same secret scan Phases 4, 5 and 8 use, and its record names the gwz-core version and the example binary's source revision.

### P3-4 — "S5 and S7 are unaffected" leaves the shipped Python behaviour unmeasured and undisclosed

**Location.** Amendment line 100; §3.8 line 92.

**Violated invariant.** Contract §16: "Building a transport runtime per operation starts that runtime's threads for every operation. The cost must be measured" and "Eight overlapping operations may open up to 8×32 connections to one host." Plan §1 ("connection pooling, as supported behavior of … gwz-py builds"); S5.1's "long-lived reuse" row and Design §11's "multiple operations in a long-lived embedding"; S5.4's endpoint pool ceilings; Release Readiness item 2.

**Sequence.** S5.1–S5.4 measure the CLI (one runtime per process) and choose pool ceilings. A Python session runs the contract's default of 8 operations, each with its own runtime: 8×ceiling connections to one host. Nothing in the plan measures or bounds it. S7.2's notes are not required to say that gwz-py builds one runtime per operation, while gwz-py's own transport design still promises cross-call reuse (its §1–§2 are superseded only from the contract's §14).

**Impact.** Unmeasured connection fan-out for Python users, GitHub-side refusals discovered after release; S5.1's "long-lived reuse" evidence describes a configuration no shipped consumer has; users misled about pooling scope.

**Required correction.** Add a Python row to S5.1 or S6.3: runtime-construction cost and connection counts for 1, 2 and 8 overlapping operations against the fixture. S5.4 (or S6.2's session defaults) states the per-session bound (running limit × pool ceiling) the measurements support. Re-scope "long-lived reuse" to the CLI's in-command reuse or to that Python row. §1 and S7.2 state that gwz-py builds one runtime per operation with no reuse across calls.

**Closure test.** An evidence ID for the Python row appears in S5.6's table; the migration notes carry the per-operation statement.

### P3-5 — The 2026-09-23 bounded amendment to Phase 1/6, and gwz-py's live implementation authorization for the retired design, are not retired

**Location.** Amendment §3.2 and §3.4 (supersede only the plan's b141e26d text); §5 On-GO edit (plan status only); S1.1 line 39 names `GwzPyTransportDesign.md` as the contract's pointer host.

**Evidence.** `gwz-py/dev-docs/GwzPyTransportDesign.md` status line: "S1.1/S1.2 design accepted for implementation, 2026-09-23 … Operator authorized implementation. This accepts the bounded package-boundary amendment to the 1.1.0 plan §3 Phase 1/6", beneath a paragraph saying NO-GO and "pending review". Its §2 (single long-lived `TransportSession`, lazy host) and §5 (physical-session reuse and barrier tests) remain in the body.

**Violated invariant.** AgentProcessRules §7.2: one current authority; "never merely append a contradictory paragraph."

**Sequence.** A gwz-py lane reads its repository's design status and implements the long-lived `TransportSession` (the checkpoint section removed in the root working tree records that such code was in working source on 2026-09-23). S6.2 later has to remove it; contract §16's "legacy `TransportSession` entry points" grow instead of shrinking.

**Impact.** Two live implementation authorities for incompatible Python designs; a later auditor finds two amendments to Phase 1/6, one unnamed.

**Required correction.** §3 names the 2026-09-23 bounded amendment as superseded. §5's On-GO list adds a status-only edit to `GwzPyTransportDesign.md` marking its acceptance superseded for §1–§3 and §5's reuse and barrier tests by the contract's §14, and the matching pointer in `GwzPyDesign.md`.

**Closure test.** A reader of `GwzPyTransportDesign.md`'s status identifies the contract as the sole authority.

### P3-6 — No evidence that the session variant applies the accepted clocks and defaults

**Location.** Amended S6.1–S6.3, lines 59–63 ("Deadlines stay the Rust clocks"; the session variant; the S6.3 test list).

**Violated invariant.** Plan §2 row 3: the two setup clocks with 9 s stall and 30 s aggregate defaults, product-wide. S3.1: "Zero disables network deadlines." S3.3's production-graph stall regression, which "S7 and Phase 8 wait on". Contract §5.5–§5.6: workers use "the timeouts in the session context"; the contract states no default for them.

**Sequence.** The session context is created at `open` with timeouts from options or a default defined in the session code, a different source from the CLI's defaults. A Python caller never calls `configure_transport_runtime`. If the session default is unset or zero, Python network operations run with no deadlines; a stalled SSH setup hangs until cancel or the 60 s close bound detaches the worker. S3.3's regression exercises the legacy entry only, and S6.3 does not require it on the session variant.

**Impact.** The two shipped entries diverge on the clocks §2 promises; Python hangs-until-cancel on stalls; the §2 promise is unevidenced for gwz-py.

**Required correction.** S6.3 adds: a session opened without `configure_transport_runtime` reports, in a typed field, the accepted defaults; and S3.3's regression ("one idle stage expires with reason `stall` while the aggregate is still ahead") runs through the session variant on both bridges.

**Closure test.** Both tests exist and pass on the S6.3 tree.

---

## 2. Invariant analysis

| Invariant | Source | Under the amended text | Finding |
|---|---|---|---|
| Phase 6 code starts only after both axes GO on the adopted design | plan S1.2, §4, closing note | Holds in letter; the object is unpinned and the predicate is satisfiable by revision 2 or flipped by a later draft | P3-1 |
| Every shipped network route is the transport; no silent native route | plan §1, §2; Transport Plan Phase 6 exit | Broken for Windows after S6.1: both entries fall back silently and no post-S6.1 step asserts the route | P2-1 |
| gh-only HTTPS and credential locality in the shipped Python and CLI paths | S7.1; Readiness item 5 | Unevidenced on Windows after S6.1 | P2-1 |
| The two setup clocks and their defaults apply to gwz-py | §2 row 3; S3.1; S3.3 | Unevidenced for the session variant | P3-6 |
| Filed evidence carries no secrets | §2 lines 62–64 | Phase 6 evidence, newly release evidence, outside the rule | P3-3 |
| Release evidence attributable to the released tuple | Phase 7 exit rows; S7.3 | Two-bridge run's core source unspecified | P3-3 |
| Measurements cover shipped behaviour; defaults within caps | Phase 5; contract §16 | Python per-operation cost and fan-out unmeasured | P3-4 |
| One current authority per document | AgentProcessRules §7.2 | gwz-py's transport design still authorizes the retired design | P3-5 |
| Review tier recorded, not implementer-chosen | GwzProcessOptimization §4.2 | Implementation plan's review unspecified | P3-2 |
| Release order and registry pins (Phase 8 steps 2, 3, 7; S6.2) | plan Phase 8; gwz-py RELEASE.md | Holds: step 7 waits on gwz-core 1.1.0 on crates.io and on gwz-cli's tag (step 6); step 2 needs nothing from Phase 6 (`TransportRequest` is gwz-core's); no dangling reference to step 3 | — |
| No new package; gwz-transport unchanged; wheel contains no executable | S1.1, §5; G10 | Holds; the host binary is an example target, not installed | — |
| Redaction of Python-visible errors on both bridges | §2; S6.3; contract §12 | Holds at plan level: the same native-integration tests run through both bridges | — (residual below) |
| CLI never worse than status quo in 1.1.0 | §3.6 decision | Holds for the entry choice, but S6.1 moves the CLI's dispatch into core; covered by P2-1(a) | P2-1 |

Round-2 closures of the plan's accepted Safety review (S5.6 unsupported-cell block; S5.6 before S7.1; vendor SHA gate; mid-train stop; tag immutability) are not regressed by this amendment.

---

## 3. Risks and next action

**Next action.** NO-GO on this text. Resolve P2-1 by the five text corrections stated (a–e); they are bounded and touch only the amendment's Phase 6 replacement, its "S7 unchanged" declaration, and Phase 8's post-release check. On such a revision, identified by hash, I pre-commit to GO on this axis. P3-1 to P3-6 are nonblocking and carried with their closure tests; P3-1 and P3-5 belong in the amendment's revision, P3-2, P3-3, P3-4 and P3-6 may instead be discharged in the implementation plan if the amendment names them as its obligations. The lane owner merges this verdict with the other axis outside this report.

**Residual risks below the finding bar.**
- Blast radius: S7.1, the CLI's activation, now waits on the full session program (three milestones each above 500 lines, the two-bridge proof and the §15 list). The amendment's §6 offers the operator only the enlarging option (S6.4). The decoupling option, activating the CLI's entry in 1.1.0 and gating §2's Python row separately, is a scope decision the operator may want stated; it is not a defect of this text.
- In the stream-bridge run, the host binary's stderr is a Python-visible surface the plan does not name; the "any difference between the two runs is a defect" rule catches content differences, not a secret present on both.
- Plan line 75, "Production core does not depend on `gwz-transport`", is retained as "unchanged" though false after S7.1 and Phase 8 step 5; consistency territory.
- The amended §2 Python row says the host builds runtimes "through the entry the CLI uses" while S6.2 says "a session variant" of it; a reader may cite CLI evidence for the Python entry. P2-1's corrections make the evidence explicit either way.
- §2 row 2's "both placements" is product-agnostic; the contract leaves client placement out of scope for Python. If S5.6's cell table has a Python × carried cell, the §2 block in S5.6 trips unless the Python row says "local placement". Stating it costs one word.
- The contract's example host binary is shipped in the gwz-core crate package source (not installed). §2 records "a separate-process wire" as unsupported; a sentence in §5 that the example is test-only would close the reading.
- Rust file sizes: the S6.1 milestone will push `transport_host` and `local_command.rs` past the workspace's review threshold; the implementation plan should budget the split.

**End-of-review tuple.** Object SHA-256 `774bb164c1c33122738b6864cabcc81203404bd25372bebe32b3984c1cdd7acf`; root `9a65306`, gwz-core `b13bbad`, gwz-py `4ad2b07`, gwz-cli `ebbea90`, gwz-transport `a7a36ae`; plan at b141e26d `583dd308…`; contract revision 3 `80e1c06a…`; all unchanged from the start of the review.
