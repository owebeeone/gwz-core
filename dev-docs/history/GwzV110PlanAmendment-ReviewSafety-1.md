# GwzV110PlanAmendment — SAFETY-AXIS REVIEW (focused re-verdict, round 1)

**Review object:** `gwz-core/dev-docs/GwzV110PlanAmendment.md`, second draft, uncommitted (untracked in gwz-core), 155 lines, SHA-256 `1606f9672a1a08dd2e53c92ef497bb5911bc0de53893ca67af7ff56e9cd56740`. Status line: "draft; not implementation authority. Second draft, after the first review." Hash verified identical at start (23:35 AEST) and end (23:43 AEST). Prior object: first draft `774bb164…`, replaced in place.
**Baseline:** root `9a65306544e19ee4fbb88757930764950c7494d2`; gwz-core `b13bbadb22c0238f1bc3f88f41c26a79a0669e0c`; gwz-py `4ad2b077ac473c079a62cdd7d5b7317a78fb5f1f`; gwz-cli `ebbea9025632ba8181df7ddb0bb57ac7b09f862e`; gwz-transport `a7a36aec0ec6d31e38647b61567166d612f5d2c5`; all unchanged start to end. Controlling plan read with `git -C gwz-core show b141e26d:dev-docs/GwzV110Plan.md` (SHA-256 `583dd308…`; the draft's §1 hash claims check out). `gwz-py/dev-docs/GwzPyTransportDesign.md` at gwz-py `5f938a0` (SHA-256 `2cb09134…`, equal to the working tree). Contract at root `5a5d6cf` (`80e1c06a…`). Inputs now legitimate and read: `GwzV110PlanAmendment-Verdict.md`, `GwzV110PlanAmendment-RemPlan.md` (SHA-256 `1d7e2c50…`). Inspection only: `shasum`, `git show/log/rev-parse/status`, `grep`, `sed`, `ls`. No build, test, edit or git mutation. `dev-docs/GwzCoreSessionPlan.md`, the contract revision 3 re-verdict records and RemPlan-2 were not opened.
**Date:** 2026-09-26
**Axis:** SAFETY — re-check A3 (Safety P2-1) and P3-1 to P3-6 on the corrected text, then what the per-operation model newly permits in 1.1.0. Independent, adversarial, read-only. The other axis re-verdicts in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0×P0, 0×P1, 1×P2 (new: P2-2), 3×P3 (new: P3-8 to P3-10) open. P2-1 and P3-1 to P3-6 closed. No new finding is an architectural root cause. I pre-commit to GO on a revision that resolves P2-2 as specified; the P3 findings are nonblocking and may be carried into that revision or into S1.1's design revision with their closure tests.

---

## Prior-finding closure table

| ID | Disposition claimed (RemPlan §2) | Verified on corrected text | Status |
|---|---|---|---|
| P2-1 (A3) | Every new transport arm written for Unix and Windows under the candidate switch; whichever of S4.5 and S6.x lands second re-runs S4.5's dabeest fixtures; S6.3 on all three platforms with route assertions through transport observations; S7.2 Python ledger rows and exit row mapped to S6.3; Phase 8 post-release route assertions; gwz-cli untouched | §3.4 lines 74–75, 88; §3.5 lines 99–100; §3.6 line 107. The original interleaving no longer reproduces: the dispatch-move leg is dissolved (line 74; §3.7 line 114), the `cfg(unix)` leg is closed by line 75, and the missing-arm case now fails S6.3's dabeest route assertion (`ResponseMeta.transport` exists in the generated protocol, so the assertion is expressible). The RemPlan's closure test ("a Windows build with the Python arm removed fails S6.3 instead of passing natively") holds for the candidate build. What the correction does not reach — the normal build after S7.1, and gwz-py's own candidate switch — is a different root and is filed as P2-2. | **Closed** |
| P3-1 | S1.2 closes on a filed verdict naming the revised design's gwz-py commit, both axes GO; a later revision needs its own GO; no floating reference to the contract's revision | §3.2 lines 53–58: object pinned to "an exact gwz-py commit"; closure "on a filed verdict that names that commit, with both axes GO"; "A later revision of the design needs its own GO before implementation follows it"; "Nothing in this plan implements the revision before S1.2 closes." The revision-2 reading cannot be constructed; the post-closure-revision state has a rule. | **Closed** |
| P3-2 | Dissolved: each Phase 6 step within 500 lines (removed code not counted); no separate implementation plan | §3.4 line 73. S6.1 (one variant plus unit tests), S6.2 (rewire, 8-bound, cancel wiring, pins; `transport_session.rs`'s 59 KB is removed, not counted) and S6.3 (tests) are credibly within the aspiration. The gate that had no review rule no longer exists; S1.1/S1.2 carry the design decisions under dual review. | **Closed (dissolved)** |
| P3-3 | §2's redaction rule extended to Phase 6; path-pinned runs are development evidence; release proof is Phase 8's post-release check | §3.1 line 34 ("Phases 4, 5, 6, and 8 redact"); §3.4 line 95. The two-bridge run and host binary are no longer in 1.1.0 (§3.7 line 114), so the attribution half of the finding is moot; the redaction half is closed. | **Closed** |
| P3-4 | Each `Client` bounded to 8 running network operations; S6.3 records construction cost and connection counts for 1, 2 and 8; S7.2's notes state the per-operation model and the bound; §1's pooling stated as within each operation | §3.2 line 50; §3.4 line 93; §3.5 line 99; §3.1 line 33 (reuse across Python operations recorded unsupported); §3.9 line 130. The bound exists, the numbers are recorded and disclosed. Whether the bound's *behaviour* (a ninth waits, is cancellable) is tested is a new gap, filed under P3-8, not a regression of this disposition. | **Closed** |
| P3-5 | §1 controls `GwzPyTransportDesign.md` (its acceptance, unapplied bounded amendment and NO-GO); §2 states the history; S1.1 revises that document; §3.8 closes the NO-GO; §5 status-only edit on GO | §1 line 10; §2 lines 16–19 (history checks against the design's status line at `5f938a0` and against `dev-docs/history/GwzPyTransportConcurrencyNoGo.md`, which exists); §3.2 line 40; §3.8 line 120; §5 line 144. One Phase 1 object, one NO-GO closing condition, one authority after the On-GO edit. | **Closed** |
| P3-6 | S6.3 tests the accepted default clocks without `configure_transport_runtime` and runs S3.3's stall regression through gwz-py's path | §3.4 line 91. Under the per-operation entry the clocks come from the same construction as the CLI's (`TransportRuntime::with_https` in `local_command.rs`), so parity is by construction and the rows verify it. | **Closed** |

---

## Changed-range analysis

**What changed since `774bb164…`.** The amendment's premise changed from "the Python design is the core session contract" to "gwz-py enters the transport through the CLI's entry, one runtime per operation", per the operator's 2026-09-26 decision. Concretely: §1 adds `GwzPyTransportDesign.md` as a controlled document; §2 rewrites the history (2026-09-23 acceptance and unapplied bounded amendment, the NO-GO, the contract, the decision, the stale text); §3.1 replaces plan lines 50 and 52 and extends the redaction rule at lines 62–64 to Phase 6; §3.2 qualifies line 75 ("Before S7.1") and replaces S1.1/S1.2 with a design *revision* of gwz-py's own document, dual-reviewed at a commit; §3.4 replaces Phase 6 with three small steps (S6.1 token-taking variant in gwz-core; S6.2 removal of `TransportSession` and rewiring onto the entry with an 8-per-`Client` bound and registry pins; S6.3 five test rows on three platforms with route assertions and recorded counts), plus a Windows-arm rule, an S4.5 re-run rule and evidence sentences; §3.5 is new (S7.2 sentences; exit row mapped to S6.3); §3.6 adds a post-release route assertion; §3.7 moves the rest of the contract out of 1.1.0; §3.8 gives the NO-GO a closing condition; §3.9's unchanged list now names Phases 3–5 and S7.1/S7.3–S7.5; §6 replaces the operator question with a statement of which two contract pieces 1.1.0 implements.

**Within the dispositions and the decision?** Yes. Every change maps to a RemPlan §2 row, a first-round residual (line 75; Python placement; scope), or the decision's stated shape (entry, one runtime per operation, public API kept, 8 per `Client`, the rest to a later plan). The S6.1 unit-test list and the S6.3 cancel row are new text but inside S6.1/S6.3's shape. Nothing was changed that the dispositions or the decision do not cover.

**Architectural root cause?** None. P2-2 is a missing activation/verification step for gwz-py's normal build (text-fixable in S7.1/S7.3); P3-8 to P3-10 are bounded coverage and specification gaps. GwzProcessOptimization §4.1's cap is not triggered.

---

## 0. Evidence base

| Source | Identity | Role |
|---|---|---|
| `gwz-core/dev-docs/GwzV110PlanAmendment.md` | SHA-256 `1606f967…` | The object |
| `gwz-core/dev-docs/GwzV110Plan.md` | gwz-core `b141e26d`, SHA-256 `583dd308…` | Controlling plan; all cited line numbers checked (50, 52, 62–64, 74–76, 78–100, 125–127, 276–301, 317–323, 352, 377–378, 380–383, 401–402, 409, 442, 446) |
| `gwz-py/dev-docs/GwzPyTransportDesign.md` | gwz-py `5f938a0`, SHA-256 `2cb09134…` | Controlled document; its status line, §2 (long-lived host) and §5 (tests leaving with it) |
| `dev-docs/GwzCoreSessionDesign.md` | root `5a5d6cf` | §5.2 (transport entry and its panic clause), §14 (survivals), §16 |
| `GwzV110PlanAmendment-Verdict.md`, `-RemPlan.md` | working tree; RemPlan SHA-256 `1d7e2c50…` | Dispositions re-checked |
| `gwz-core/src/transport_host/local_command.rs:12–72` | gwz-core HEAD | The entry the variant copies: per-call `environment_config()` (`std::env::vars_os()`, `SshEndpointConfig::from_environment()`), `impl Drop for Command` → `finish()` |
| `gwz-py/native/src/lib.rs:13–16, 195–197`, `dispatch/mod.rs:423, 483–500`, `merge.rs:95`, `gwz-py/Cargo.toml:15` | gwz-py HEAD | gwz-py's own `cfg(all(unix, gwz_transport_candidate))` gates and check-cfg; the `else { call(…) }` native fallback |
| `gwz-py/native/src/transport_session.rs:404, 666, 728–739` | gwz-py HEAD | The session being removed runs build, dispatch and finish under `catch_unwind` |
| `gwz-py/src/gwz/client.py:203–232, 265` | gwz-py HEAD | `close`, `close_report`, `cancel_operation`, `meta` exist as public API |
| `gwz-core/src/protocol/generated.rs:2741` | gwz-core HEAD | `ResponseMeta.transport: Option<Vec<TransportObservation>>` — the route assertion's field |
| `dev-docs/history/GwzPyTransportConcurrencyNoGo.md` | root HEAD | Exists; cited by §2 item 2 |

Facts the new findings rest on:
- gwz-py's transport dispatch is gated by gwz-py's own `gwz_transport_candidate` cfg, separate from gwz-core's. The draft keeps S6.2's arms "under the candidate switch" (line 75). S7.1, declared unchanged (§3.9 line 128), removes the switch "from the sites Phase 4 already opened to Windows" (plan 308–312), which are gwz-core sites; S7.3 builds the extension and asserts nothing (plan 325–329). No amended or unchanged step names gwz-py's switch.
- `with_local_transport` reads the process environment on every call and finishes its `Command` from `Drop`; today's `TransportSession`, which S6.2 removes, wraps finish in `catch_unwind` and reports `worker_panicked`.
- The old design's §5 tests that leave with the session include close-joins-in-flight-work, nonzero-pending-work close, wrong/foreign/expired cancel IDs, and the bounded completed-cancellation registry.

---

## 1. Findings

### P2-2 — No step activates the transport in gwz-py's normal build, and the only normal-build route proof for gwz-py is after the wheel is published

**Location.** §3.4 line 75 ("Every new transport arm is a `cfg_if` arm for Unix and Windows *under the candidate switch*"), §3.9 line 128 ("S7.1, S7.3, S7.4 and S7.5" unchanged), §3.4 line 95 ("The release proof is Phase 8's post-release check"), §3.6 line 107.

**Root cause.** The operator's shape leaves gwz-py's transport dispatch in gwz-py, behind gwz-py's own `gwz_transport_candidate` gate (`native/src/lib.rs:13`, `dispatch/mod.rs:483`, `Cargo.toml:15`). The plan's activation step, S7.1, is unchanged and scoped to "the sites Phase 4 already opened to Windows", which are gwz-core's; its "This step does not introduce Windows into those sites; S4.5 already did" confirms the scope. S6.1's variant is likewise opened by S6.1, not Phase 4. So under the amended text no step removes the candidate switch from gwz-py's arms or, on a literal reading, from S6.1's variant. S7.3 builds the extension from workspace pins and asserts nothing; the draft places the only normal-build route assertion in Phase 8's post-release check, which by construction runs after `scripts/release.py v1.1.0 --push` and the PyPI publish.

**Violated invariant.** Plan §1: the transport "as supported behavior of the normal … `gwz-py` builds"; §2 rows 1 and 4 as amended; Phase 8's stop rule ("If any step fails after a push, a GitHub Release, or a registry publish, stop … Never move or reuse a tag"), which makes a post-publish failure irreversible for the 1.1.0 wheel.

**Interleaving (permitted by the amended text).**
1. S6.2 lands: gwz-py's dispatch calls the S6.1 variant inside `cfg_if` unix|windows arms under `gwz_transport_candidate`, keeping today's `else { call(…) }` native branch for builds without the switch (which the plan requires anyway for the unsupported Linux ARM64 and macOS x86-64 wheels, plan 314–315).
2. S6.3 passes on the candidate build on all three platforms, with route assertions. The NO-GO closes (§3.8).
3. S7.1 removes the switch from gwz-core's Phase-4 sites. gwz-py's arms, and on the literal reading S6.1's variant, stay under the switch.
4. S7.3 builds the CLI and the Python extension on the three platforms from workspace pins: the extension compiles with the native branch. S7.4 rechecks observations through the CLI. S7.5 reviews the tree.
5. Phase 8 steps 5–7 tag and publish gwz-core, gwz-cli and gwz-py 1.1.0. gwz-py's release script runs `run_tests.py` at the tag, but nothing in the plan says S6.3's fixture-backed route tests run there rather than being skipped without their fixtures.
6. The post-release check's Python route assertion fails on every host. Under the stop rule the 1.1.0 wheel stays on PyPI with native SSH and HTTPS for every Python user; recovery is a 1.1.1.

**Impact.** A published gwz-py 1.1.0 that does not ship the transport at all — no gh-only HTTPS policy, no endpoint-owned credentials, no setup clocks or cancellation — detected only after the irreversible publish. Unlike A3, it is detected; unlike A3, it can no longer be prevented by the text before the tag.

**Required correction (text).**
(a) S7.1 (amend it in §3.5, or add the sentence to S6.2): the candidate switch is removed from every site under `gwz_transport_candidate` in gwz-core and gwz-py, including S6.1's variant and S6.2's arms, together with both crates' `check-cfg` declarations and `tests/transport_backend/prepare.py`; `rg gwz_transport_candidate` over both repos finds nothing after S7.1.
(b) S7.3: each consumer build on each platform runs one CLI and one Python network operation against the fixtures and asserts the transport route through the transport observations, before S7.5, i.e. the same assertion the draft adds to Phase 8, moved ahead of the tag as well. Including the overlapping pair in the Python run keeps the NO-GO's closure valid on the normal build.
(c) Phase 8 step 7: name that gwz-py's release gates run S6.3's route tests against the registry-pinned core, or say explicitly that S7.3's assertion is the pre-publish proof.

**Closure test.** With gwz-py's candidate gate deliberately left in place, S7.3's Python route assertion fails on every platform before any tag exists; with it removed, `rg gwz_transport_candidate` over gwz-core and gwz-py is empty and the assertion passes.

### P3-8 — S6.3's rows omit tests for the behaviours the amendment itself introduces or changes: close and interpreter exit with running operations, the 8-per-`Client` bound, and cancel targeting among concurrent operations

**Location.** §3.4 lines 88–93 (the S6.3 rows); §3.2 lines 42–50 (the meanings S1.1 must state; the survival list, which drops the contract §14's "close's retained report"); §4 line 135 (tests leaving with the session).

**Violated invariant.** Plan S6.3 is the plan's named test list for Phase 6; every behaviour §2 or Phase 6 promises needs a row (the plan's own "A row with no step is not waived" principle, line 342–343). The old design's §5 rows that leave with the session covered these behaviours for the session model; the per-operation model changes them and gets no replacement rows.

**Sequence.** (a) `Client.close()` or interpreter exit with two running per-operation runtimes: the old barrier test "close joins in-flight work … nonzero pending-work case" leaves; no row says what close does, so a revision that returns at once leaves live runtimes and their SSH helper processes behind at exit, and `close_report` counts nothing. (b) The 8-bound: "further ones wait" (line 50) has no row; the NO-GO's cause ("another network operation is active") can return as a refusal of the ninth, or as one native thread per waiting operation, with the NO-GO formally closed by the two-operation row. (c) Cancel targeting: the old "a wrong, foreign or expired ID must fail without cancelling the active operation" leaves; with up to 8 tokens registered per `Client`, `cancel_operation` naming the wrong ID, or an ID of a completed operation, has no row.

**Impact.** Orphaned runtimes and helpers at close or exit; the refusal the NO-GO was raised for reappearing at 9 operations untested; a cancel cancelling a sibling operation. All bounded to the Python product and to the behaviours S1.1 will define, but with no test the plan requires.

**Required correction.** Add S6.3 rows: `Client.close()` and interpreter exit with a running network operation cancel or join it, return a report that counts it, and leave no helper process; with 8 running, a ninth waits without refusal, runs when a slot frees, and can be cancelled while waiting (S1.1 states whether a waiting operation holds a native thread); `cancel_operation` with a wrong, foreign or completed ID fails without cancelling any running operation, and completed-cancellation state stays bounded.

**Closure test.** Those rows exist in S6.3 and pass on the three platforms.

### P3-9 — "Environment stability" is claimed as surviving, but the per-operation entry reads the environment on every operation and §6 excludes the mechanism that provided it

**Location.** §3.2 line 44 ("environment stability and its isolation from invalid proxy or CA settings" under "What survives"); §6 line 152 ("without its session-context environment and timeouts").

**Violated invariant.** The contract's §14 survival of "environment stability" was the session context's snapshot captured once at `open` (§5.6). The old design's own statement was per `Client`: "Once installed, the host and its endpoint environment are stable until close; changing process environment mid-session does not silently change its credentials or trust context" (gwz-py design §2). `with_local_transport` calls `environment_config()` per call (`local_command.rs:22, 35–36`).

**Sequence.** A host application sets `SSH_AUTH_SOCK`, `GIT_SSL_CAINFO` or `HTTPS_PROXY` between two `client.fetch()` calls, or while eight are in flight: each operation captures whatever the environment is at its start, so overlapping operations on one `Client` can run with different trust contexts, and the guarantee S1.2's reviewers are told survives does not hold at `Client` scope. Separately, per-operation `std::env::vars_os()` on operation threads runs concurrently with any `os.environ` mutation on the Python main thread, the classic `getenv`/`setenv` race, more often than the once-per-`Client` read it replaces.

**Impact.** A misleading survival claim entering S1.1/S1.2; divergent credentials across overlapping operations; a wider window for the environment race than the status quo candidate.

**Required correction.** S1.1 restates the guarantee as per-operation capture: captured at the operation's start, stable within it, changes between operations take effect at the next; S6.3 adds a row for it (a change made after an operation starts does not affect it; a change before the next does); S7.2's notes state it. If the operator wants `Client`-scope stability, the bridge captures once at `Client` construction and the variant takes the snapshot as an argument, which is the contract's §5.2 shape and stays within "the token-taking part".

**Closure test.** The revision's text and an S6.3 row match; the word "stability" in S1.1 is qualified by scope.

### P3-10 — S6.1 specifies the variant as behaving "exactly as `with_local_transport` does", importing the CLI entry's finish-from-`Drop` into a library and dropping the contract's panic clause

**Location.** §3.4 line 78 ("Otherwise it builds, runs, finishes and shuts down exactly as `with_local_transport` does"); §6 line 152.

**Violated invariant.** Contract §5.2 "Panics": "The transport entry never finishes from `Drop` while unwinding, so a second panic cannot abort the process." gwz-py is a library in a host process; today's `TransportSession` (removed by S6.2) runs build, dispatch and finish under `catch_unwind` (`transport_session.rs:404, 666, 728`). `with_local_transport`'s `Command` finishes from `Drop` (`local_command.rs:70–72`), which is adequate for a CLI process that dies on panic.

**Sequence.** A handler panics inside the variant on an operation thread; unwinding drops `Command`, whose `Drop` runs `finish()` (`block_on` over request finish and runtime shutdown); a fault in finish or shutdown panics while unwinding; the process aborts, taking the Python interpreter and the host application's state with it. gwz-py's outer `catch_unwind` (`dispatch/mod.rs:493`) is never reached.

**Impact.** Host-process abort under a double fault, a regression against the candidate the amendment removes ("never worse than the status quo" broken on this path). Bounded: two faults are required.

**Required correction.** S6.1 states the library-safety condition: the variant catches a handler panic first, runs finish and shutdown under their own `catch_unwind`, never finishes from `Drop` while unwinding, and reports failure with unconfirmed cleanup; its unit tests add a fault-injected finish panic after a handler panic with the process staying alive. The CLI's use of `with_local_transport` is unaffected.

**Closure test.** That S6.1 test exists and passes; the operation's cleanup report reads unconfirmed and the next operation on the same `Client` succeeds.

---

## 2. Invariant analysis

| Invariant | Source | Under the second draft | Finding |
|---|---|---|---|
| Phase 6 code starts only after a filed dual GO on a pinned design | S1.2 as amended | Holds: commit-pinned, verdict-named, later-revision rule | P3-1 closed |
| No silent native route on the candidate build; Windows arms exercised | §3.4 lines 75, 88; §3.5; §3.6 | Holds for the candidate build on three platforms | P2-1 closed |
| The normal gwz-py build ships the transport, proven before the tag | plan §1; S7.1; S7.3; Phase 8 stop rule | Broken: no step removes gwz-py's candidate switch; the only normal-build route proof is post-publish | **P2-2** |
| gwz-cli untouched by Phase 6; CLI evidence not invalidated by S6.x | §3.4 line 74; line 75 re-run rule | Holds; the re-run rule covers `with_local_transport` when S6.1 lands second | — |
| Filed evidence carries no secrets; path-pinned runs are not release proof | §3.1 line 34; §3.4 line 95 | Holds | P3-3 closed |
| Python fan-out measured, bounded and disclosed | §3.2 line 50; §3.4 line 93; §3.5 line 99 | Holds for the numbers and the disclosure; the bound's behaviour is untested | P3-4 closed; P3-8 |
| Clocks and defaults apply to gwz-py | §3.4 line 91 | Holds | P3-6 closed |
| One current authority for the Python design | §1; §5 line 144 | Holds after the On-GO edit | P3-5 closed |
| The NO-GO's closing condition matches its cause | §3.8 line 120 | Holds for the candidate build (two overlapping operations on three platforms); Phase 7 still waits on all of S6.3; the normal build's overlap is covered only if P2-2(b) includes the pair | P2-2 (b) |
| Behaviours the model changes have test rows | S6.3 | Broken for close/exit, the 8-bound and cancel targeting | P3-8 |
| Survival claims name guarantees the model can keep | §3.2 line 44 | Broken for environment stability at `Client` scope | P3-9 |
| Library never worse than the removed candidate on the panic path | contract §5.2; `transport_session.rs` | Broken by "exactly as `with_local_transport` does" | P3-10 |
| Release order and registry pins (Phase 8 steps 2, 3, 7; S6.2) | §3.6; RELEASE.md | Holds; unchanged from the first review | — |
| Scope: no new package; gwz-transport unchanged; public Python API kept | §3.2 line 51; §5 line 141 | Holds; S1.2 adds Surface if a documented behaviour changes | — |

Round-2 closures of the plan's accepted Safety review (S5.6 unsupported-cell block; S5.6 before S7.1; vendor SHA gate; mid-train stop; tag immutability) are not regressed.

---

## 3. Risks and next action

**Next action.** NO-GO on this text for P2-2 alone. Its remedy is three sentences (S7.1's site list across both repos; a pre-publish route assertion in S7.3; step 7's gate statement). On a revision that carries them, identified by hash, I pre-commit to GO on this axis. P3-8, P3-9 and P3-10 are nonblocking; P3-8 and P3-9 are naturally discharged by S1.1's revision and S6.3's rows, P3-10 by one sentence in S6.1 plus its test. The lane owner merges this with the other axis outside this report. GwzProcessOptimization §4.1 permits this third bounded round; no architectural root was found in any round.

**Residual risks below the finding bar.**
- S6.3's dabeest row cannot run before S4.5 (the Windows candidate contains the endpoint modules only after S4.5), so S6.3 in practice waits on S4.5. The normative sketch (§3.9 line 126) shows no `S4.5 ── S6.3` edge. S7.2's "backed by S6.3's route assertions" and S5.6's in-release block are backstops; adding the edge would make the order explicit.
- The re-run rule "re-runs S4.5's dabeest fixtures through the entries it touched" is vacuous when S6.2 lands second, since S4.5's fixtures are CLI fixtures that do not run through gwz-py; S6.3's dabeest row is the real guard there, and the sentence could say so.
- Two `Client`s in one process run up to 16 runtimes; the disclosure is per `Client`, which is honest, but the process-wide caps (64 SSH helpers, the HTTPS helper slots) are the only cross-`Client` bound and are not restated.
- `configure_transport_runtime` remains process-wide under the per-operation model (libgit2's server timeouts); S1.1's stated meaning should say so, since two `Client`s in one process now share it.
- The unsupported Linux ARM64 and macOS x86-64 wheels keep the native branch by design (plan 314–315); P2-2(a)'s "nothing under the switch" must not be read as removing that branch, only the switch.

**End-of-review tuple.** Object SHA-256 `1606f9672a1a08dd2e53c92ef497bb5911bc0de53893ca67af7ff56e9cd56740`; RemPlan `1d7e2c50…`; root `9a65306`, gwz-core `b13bbad`, gwz-py `4ad2b07`, gwz-cli `ebbea90`, gwz-transport `a7a36ae`; plan at b141e26d `583dd308…`; gwz-py design at 5f938a0 `2cb09134…`; contract revision 3 `80e1c06a…`; all unchanged from the start of the re-review.
