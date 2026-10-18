# GwzV110PlanAmendment — SAFETY-AXIS REVIEW (focused re-verdict, round 2)

**Review object:** `gwz-core/dev-docs/GwzV110PlanAmendment.md`, third draft, uncommitted (untracked in gwz-core), 178 lines, SHA-256 `cb4ae1664a595231bde73cdb83809fa4b2316f574ca6365fb878255df4be1585`. Status line: "draft; not implementation authority. Third draft, after the first and second reviews." Hash verified identical at start (23:48 AEST) and end (23:51 AEST). Prior objects: first draft `774bb164…`, second draft `1606f967…`, both replaced in place; the second draft is held in this reviewer's context.
**Baseline:** root `9a65306544e19ee4fbb88757930764950c7494d2`; gwz-core `b13bbadb22c0238f1bc3f88f41c26a79a0669e0c`; gwz-py `4ad2b077ac473c079a62cdd7d5b7317a78fb5f1f`; gwz-cli `ebbea9025632ba8181df7ddb0bb57ac7b09f862e`; gwz-transport `a7a36aec0ec6d31e38647b61567166d612f5d2c5`; all unchanged start to end. Controlling plan read with `git -C gwz-core show b141e26d:dev-docs/GwzV110Plan.md` (SHA-256 `583dd308…`); `gwz-py/dev-docs/GwzPyTransportDesign.md` at gwz-py `5f938a0` (`2cb09134…`); contract at root `5a5d6cf` (`80e1c06a…`). Inputs now legitimate and read: `GwzV110PlanAmendment-Verdict-1.md` (`a562fc52…`), `GwzV110PlanAmendment-RemPlan-1.md` (`e98a2573…`). Inspection only: `shasum`, `git show/rev-parse/status`, `grep`, `sed`, `cat`. No build, test, edit or git mutation. `dev-docs/GwzCoreSessionPlan.md`, the contract re-verdict records, RemPlan-2 and other working-tree files were not opened.
**Date:** 2026-09-26
**Axis:** SAFETY — re-run P2-2's interleaving on the corrected S7.1/S7.3/step 7 text and its closure test; close or keep P3-8, P3-9, P3-10; then what the edits newly permit. Independent, adversarial, read-only. The other axis re-verdicts in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — 0×P0, 0×P1, 0×P2, 1×P3 (new: P3-11) open. P2-2, P3-8, P3-9 and P3-10 closed. No new blocking finding; no architectural root cause in any round. P3-11 is nonblocking, one sentence, and may be carried into the status-only application or S1.1's revision.

---

## Prior-finding closure table

| ID | Disposition claimed (RemPlan-1) | Verified on corrected text | Status |
|---|---|---|---|
| P2-2 (B1) | S7.1 also removes the switch from gwz-py's three sites, S6.1's variant, S6.2's arms, both crates' `check-cfg` declarations and `prepare.py`; `rg gwz_transport_candidate` over gwz-core and gwz-py finds nothing; the native branch stays for unsupported paths. S7.3 runs one CLI and two overlapping Python network operations per platform with route assertions before S7.5, as the pre-publish proof. Step 7 names S7.3 as that proof. | §3.5 line 112 (S7.1), line 116 (S7.3), line 117 (exit row); §3.6 line 123 (step 7); §3.4 line 107; §4 line 158. The interleaving re-run: (1) S6.2 lands under the switch with the native `else`; (2) S6.3 passes on the candidate build; (3) S7.1 now removes gwz-py's switch and S6.1's variant's, with an `rg`-empty exit predicate; (4) S7.3 runs Python network operations on the normal consumer build on each platform and asserts `ResponseMeta.transport` (present in `generated.rs:2741`), before S7.5 and before any tag; a gate left in place sends the Python operations down the native `else`, which carries no transport observations, so the assertion fails pre-publish; (5) step 7 and the post-release check repeat it on the published wheel. The closure test as I specified holds on the text: left in place, the gate fails S7.3 before any tag; removed, the `rg` is empty and S7.3 passes. | **Closed** |
| P3-8 | S6.3 rows for wrong/foreign/completed cancel IDs, the ninth operation under the 8-bound, and close and interpreter exit with an operation running; S1.1 states whether a waiting operation holds a native thread and what close, exit and cancel do with several operations running. | §3.4 lines 101–103; §3.2 line 51. The three behaviours each have a row with typed outcomes (no refusal, runs when a slot frees, cancellable while waiting; report counts the running operation, no helper process left; no sibling cancelled, bounded completed-cancellation state). | **Closed** |
| P3-9 | "Environment stability" no longer listed as surviving; new "What changes in documented behaviour" bullet (capture at each operation's start, process-wide `configure_transport_runtime`, process-wide helper caps); S1.2 includes Surface; S6.3 capture row; S7.2 notes state the capture point. | §3.2 lines 43–50 and 55; §3.4 line 104; §3.5 line 114; §5 line 164. The survival claim is gone, the scope is stated as per operation and "no longer stable for the life of a `Client`", and the four places name the same capture point. The concurrent-mutation race I described is unchanged in kind from the CLI's entry and is a residual below. | **Closed** |
| P3-10 | S6.1 gains the library-safety rule (never finish from `Drop` while unwinding; catch the operation's panic first; guard finish and shutdown; report failure with cleanup unconfirmed) and the fault-injected double-panic unit test. | §3.4 line 82 and line 88. The rule matches the contract's §5.2 panic clause and the guard the removed `TransportSession` has (`transport_session.rs:728`, `catch_unwind(AssertUnwindSafe(finish))`), and the test names the process staying alive and the next operation succeeding, as my closure test asked. | **Closed** |

---

## Changed-range analysis

**What changed since `1606f967…`.** Status line (third draft). §3.2: S1.1's survival list drops environment stability; a "What changes in documented behaviour" bullet (per-operation capture, process-wide `configure_transport_runtime`, process-wide helper caps as the only cross-`Client` bound); "What is added" now requires the revision to say whether a waiting operation holds a native thread and what close, exit and cancel do with several operations running; S1.2 includes Surface for the capture-point change. §3.4: the Windows bullet says S7.1 later removes the switch from the new arms, restricts the re-run rule to S6.1 landing after S4.5 through `with_local_transport`, and makes S6.3's dabeest rows (waiting on S4.5) the guard for S6.2; S6.1 gains the library-safety rule and the double-panic test; S6.3 is labelled gwz-py-level, its dabeest rows wait on S4.5, and it gains five rows (cancel targeting, the ninth operation, close and exit, environment capture, plus the existing counts row); the evidence sentence names S7.3 as the pre-publish proof. §3.5 is new for S7.1 and S7.3 and extends S7.2 (Python ledger rows evidenced by S6.3 and S7.3, the capture point in the notes) and the exit row (S7.3 added). §3.6 step 7 names S7.3 as the pre-publish proof. §3.9 is new (sketch edge `S4.5 ── S6.3`; line 429 prose). §3.10 no longer lists the sketch, S7.1 or S7.3 as unchanged. §4 lists S7.3's assertions as release evidence. §5 explains why this amendment needs no Surface review and lists the newly amended clauses in the status text. §6 states the authority for S6.1/S6.2 is the amended plan and S1.1's revision, and that a later change to the contract's §5.2 does not amend S6.1.

**Within the dispositions and the decision?** Yes. Every change maps to a RemPlan-1 row (B1; the P3-8/P3-9 convergence; Consistency P3-6 and P3-7; Safety P3-8 and P3-10; the residuals). The added S7.3 overlapping pair is the form I asked for under P2-2(b). Nothing changes the operator's shape (CLI's entry, one runtime per operation, public API kept, 8 per `Client`, the rest to a later plan).

**Architectural root cause?** None. P3-11 is a one-clause omission in the evidence rule.

---

## 0. Evidence base

| Source | Identity | Role |
|---|---|---|
| `gwz-core/dev-docs/GwzV110PlanAmendment.md` | SHA-256 `cb4ae166…` | The object |
| `gwz-core/dev-docs/GwzV110Plan.md` | gwz-core `b141e26d`, SHA-256 `583dd308…` | Controlling plan; new cites checked: S7.1 308–315, S7.2 317–323, S7.3 325–329, sketch 415–425, prose line 429 ("S6 waits on S1.2.") |
| `gwz-py/dev-docs/GwzPyTransportDesign.md` | gwz-py `5f938a0`, SHA-256 `2cb09134…` | Controlled document |
| `dev-docs/GwzCoreSessionDesign.md` | root `5a5d6cf` | §5.2 panic clause; §14 |
| `GwzV110PlanAmendment-Verdict-1.md`, `-RemPlan-1.md` | working tree; `a562fc52…`, `e98a2573…` | Dispositions re-checked |
| `gwz-core/build.rs:8`, `gwz-py/Cargo.toml:15`, `gwz-cli/build.rs:4` | HEADs | The three `check-cfg` declarations of `gwz_transport_candidate` |
| `gwz-py/native/src/lib.rs:13–16, 195–197`, `dispatch/mod.rs:483–500`, `merge.rs:95` | gwz-py HEAD | gwz-py's switch sites and native `else` |
| `gwz-py/native/src/transport_session.rs:404, 666, 728` | gwz-py HEAD | The guard S6.1's rule restores |
| `gwz-core/src/protocol/generated.rs:2741` | gwz-core HEAD | `ResponseMeta.transport`, the route assertion's field |

Facts the remaining finding and residuals rest on:
- Plan §2 lines 62–64, as amended by §3.1, read "Phases 4, 5, 6, and 8 redact agent-socket paths, known_hosts bodies, and `gh` tokens or headers from retained evidence. A secret in filed evidence fails that step." Phase 7 is not listed. The third draft makes S7.3's fixture-backed route runs on three platforms release evidence (§4 line 158).
- gwz-cli carries the switch too (`build.rs:4`, `dispatch.rs:6, 353`). S7.1's original clause covers its sites as "sites Phase 4 already opened to Windows"; the new `rg` predicate names gwz-core and gwz-py only. S7.3's CLI route assertion per platform is a functional backstop.

---

## 1. Findings

### P3-11 — S7.3's new fixture-backed route runs become release evidence outside §2's redaction rule, which the amendment extends to Phase 6 but not Phase 7

**Location.** §3.5 line 116 (S7.3 runs one CLI and two Python network operations "against the disposable fixtures" on each platform); §4 line 158 ("New release evidence: S7.3's pre-publish route assertions on the normal builds"); §3.1 line 34 ("Phases 4, 5, 6, and 8 redact").

**Violated invariant.** Plan §2 lines 62–64: retained evidence carries no agent-socket paths, known_hosts bodies or `gh` tokens or headers, and a secret in filed evidence fails the step. The amendment's own second-round correction (P3-3) applied this rule to the evidence it promoted in Phase 6; the third draft promotes new evidence in Phase 7 without it.

**Sequence.** S7.3 runs the SSH and gh-only HTTPS fixtures on macOS, the Linux CI host and dabeest; its transport observations, command output and any failure diagnostics (agent socket path under `/e/gwz-tests` or `~/.ssh`, known_hosts lines, `gh` headers) are retained as the pre-publish proof; no step's rule fails it if a secret is present, because Phase 7 is not a named phase.

**Impact.** Host-identifying material or a fixture credential in filed release evidence with no failing gate. Bounded: the fixtures are disposable and Phases 6 and 8 already run the scan on either side of it.

**Required correction.** §3.1: "Phases 4, 5, 6, 7, and 8 redact", or name S7.3 and S7.4 beside Phase 6 in the same sentence.

**Closure test.** Filed S7.3 evidence passes the same secret scan Phases 4, 5, 6 and 8 use.

---

## 2. Invariant analysis

| Invariant | Source | Under the third draft | Finding |
|---|---|---|---|
| The normal gwz-py build ships the transport, proven before the tag | §3.5 lines 112, 116; §3.6 line 123 | Holds: switch removed from gwz-py and S6.1's variant with an `rg`-empty predicate; per-platform route assertion on the normal consumer builds before S7.5; step 7 cites it; post-release repeats it | P2-2 closed |
| No silent native route on any entry or platform | exit row (line 117); S6.3 line 96; S7.3; post-release line 124 | Holds on the candidate build (S6.3), the normal build (S7.3) and the published artifacts (Phase 8), for both entries | — |
| Behaviours the model changes have test rows | S6.3 lines 97–105 | Holds: cancel targeting, the ninth operation, close and exit, environment capture, clocks, cancellation, refusals, counts | P3-8 closed |
| Survival claims name guarantees the model can keep | §3.2 lines 43–50 | Holds; per-operation capture is stated as a documented change, with Surface at S1.2 | P3-9 closed |
| Library never worse than the removed candidate on the panic path | §3.4 lines 82, 88 | Holds | P3-10 closed |
| Every S6.3 platform row has its enabling step as a predecessor | §3.9; §3.4 lines 78, 96 | Holds: `S4.5 ── S6.3` and "the dabeest rows wait on S4.5"; the re-run rule is no longer vacuous for S6.2 | — |
| Filed evidence carries no secrets | §3.1 line 34 | Broken for the new Phase 7 evidence | P3-11 |
| The NO-GO's closing condition matches its cause on the shipped build | §3.8; S7.3's overlapping pair | Holds: closed on the candidate build by S6.3, confirmed on the normal build by S7.3 before S7.5 | — |
| S6.1's authority is the plan and the reviewed revision, not a draft contract | §6 line 178 | Holds | — |
| Release order and registry pins | §3.6 | Holds; unchanged since the first review | — |
| CLI untouched by Phase 6; changed only by S7.1's switch removal | §3.4 line 75; S7.1 | Holds; S7.1's original clause covers gwz-cli's sites and S7.3's CLI assertion backstops them | residual |

Round-2 closures of the plan's accepted Safety review are not regressed. First- and second-round closures (P2-1, P3-1 to P3-7 on this axis) are not regressed by the third draft's edits.

---

## 3. Risks and next action

**Next action.** GO on this axis for the third draft at `cb4ae166…`, as plan text only. P3-11 is carried; its one-clause fix can be applied in a later status-only round or folded into S1.1's revision without reopening this review, since it adds a phase to an existing rule and changes no step. The lane owner merges this with the other axis outside this report. Accepting this amendment authorizes no implementation, commit, tag, push or publish; S1.1's revision and its S1.2 dual-plus-Surface review remain the gate before Phase 6.

**Residual risks below the finding bar.**
- The `rg` predicate and the `check-cfg` clause in S7.1 name gwz-core and gwz-py; gwz-cli also declares the switch (`build.rs:4`) and holds sites (`dispatch.rs:6, 353`). S7.1's original clause covers those sites and S7.3's CLI route assertion catches a miss before the tag; naming gwz-cli in the predicate would remove the ambiguity.
- S7.3's Linux assertion runs on "the existing CI host" and needs the disposable SSH and gh-only HTTPS fixtures reachable there. If the host cannot run them, S7.3 cannot complete on Linux; the text does not permit a silent skip ("On each platform … Each asserts"), so this is an operational precondition, not a hole.
- "Only the switch goes: the native branch stays for the paths 1.1.0 does not support" uses the plan's pre-existing word "paths". If read as platforms rather than routes, the unsupported Linux ARM64 and macOS x86-64 wheels would be gated native by target; if read as routes, they compile the transport unqualified. Either is consistent with §2's "recorded as unsupported", but S7.2's notes should say which.
- The 8-bound limits running operations only; the revision must state whether a waiting operation holds a native thread. An unbounded waiting set with one native thread each would be a resource exposure for a host that submits thousands of operations; S1.2's Safety axis is the place to refuse that shape.
- Interpreter exit that joins a running operation waits up to the clocks; with `configure_transport_runtime(0)` (deadlines disabled), the revision should state the exit bound.
- Per-operation `std::env::vars_os()` on operation threads races any concurrent `os.environ` mutation on the Python main thread, the same exposure as the CLI's entry and today's candidate, now once per operation rather than once per `Client`. The revision's documented capture point makes the exposure visible; it does not remove it.

**End-of-review tuple.** Object SHA-256 `cb4ae1664a595231bde73cdb83809fa4b2316f574ca6365fb878255df4be1585`; RemPlan-1 `e98a2573…`; root `9a65306`, gwz-core `b13bbad`, gwz-py `4ad2b07`, gwz-cli `ebbea90`, gwz-transport `a7a36ae`; plan at b141e26d `583dd308…`; gwz-py design at 5f938a0 `2cb09134…`; contract revision 3 `80e1c06a…`; all unchanged from the start of the re-review.
