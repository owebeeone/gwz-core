# GWZ transport release plan amendment 2 — remediation plan 1

Date: 2026-10-01. Status: **remediation plan for revision 1; not implementation authority**.

## 1. Inputs

- **Object:** `GwzTransportReleasePlanAmendment-2.md`, revision 0, SHA-256 `57ce8e46e42e6b800ef5eaeda2912b83a41fd35c927b937c80ab38204c79f477`, untracked draft.
- **Tuple:** root `d6f6db59`, gwz-core `0ddc513c`, gwz-cli `5ebb001`, gwz-py `950064d`, gwz-transport `a24e70a`. Both reviewers verified it at start and end.
- **Reviews:**
  - [Consistency](GwzTransportReleasePlanAmendment-2-ReviewConsistency.md): NO-GO, 6 P2 and 12 P3. It pre-commits to GO on a revision that resolves P2-1 to P2-6 as specified.
  - [Safety](GwzTransportReleasePlanAmendment-2-ReviewSafety.md): NO-GO, 1 P1, 5 P2 and 8 P3. It pre-commits to GO on a revision that resolves P1-1 and P2-1 to P2-5 as specified.
- **Blind convergence.** The axes found these root causes independently:
  - Consistency P2-1 and Safety P1-1: without a host context, SSH takes the transport's lazy endpoint, so S7.1 (1.1.0) puts gwz-py on the transport;
  - Consistency P2-2 and Safety P2-2: the adopted Python transport-route sentences stay in 1.1.0's S7.2, S7.3, step 7 and post-release check;
  - Consistency P2-4 and Safety P3-4: S5.4, S5.5 and TR8.4 wait on each other;
  - Consistency P3-1 and Safety P2-3: rule (e) names `gwz_session_candidate` before rule (a) creates it;
  - Consistency P3-4 and Safety P1-1's last sentence: gwz-py's `TransportSession` was removed on 2026-09-28;
  - Consistency P3-11 and Safety §0: the cap-64 build's 12.3 s is its `--max-per-host 32` median, not its defaults median.

## 2. Decisions this revision asks the operator for

- **OD16. Windows HTTPS that relies on the logon session's default credentials** (Safety P2-1). Its only signal is the server's 401 challenge, so it cannot be routed before an open.
  - **Recommended, shape (b):** a bounded exception to "no fallback after an open". The transport's first request carries no credential and asks only for the ref advertisement. If the answer is a 401 whose `WWW-Authenticate` offers `NTLM` or `Negotiate`, the operation takes the native route. No credential has been offered, and the request has no effect on the server. TR1.8 designs the trigger exactly, under its own Safety review, against a loopback fixture that answers `WWW-Authenticate: Negotiate`.
    - It applies on Windows. It applies on macOS and Linux only if TR1.8's 1.0.17 rows show 1.0.17 authenticating such a server there.
    - Revision 1's text is written for (b).
  - **Alternative, shape (a):** such servers are unsupported on the transport in 1.1.0. The operation is refused with a message that names the off switch, the migration notes list it, and OD13's parity is bounded to behaviours decidable before an open. That is a narrowing the operator signs.
- **OD17. A TR8.1 (1.1.0) miss** (Safety P3-7). Decided when the measurement is in, not now: ship with the measured gap stated in the notes, hold 1.1.0, or bring in OD8's channels with its re-run list. S7.1 (1.1.0) waits on TR8.1 being met or on OD17's answer.
- OD14 and OD15 stay recommended and open, as revision 0 has them.

## 3. Dispositions of the blocking findings

| Finding | Disposition | Closure test |
|---|---|---|
| C P2-1, S P1-1 | **Accept.** New step **TR2.11: no host context, no transport** (under 200 lines). `transport_binding::configure` installs the SSH transport only when the backend carries a host context, as it already does for HTTPS (`transport_binding.rs:184-186`). Without one, SSH takes the native route, chosen before any connection opens. The lazy endpoint, `Runtime::default()`'s environment factory and the `Route::reporting(runtime.endpoint())` branch, then has no production caller and is removed, with its `env` debt entry. That brings forward the part of CS7.24 that retires it. Candidate tests that drive a backend without a host context install one. S7.1 (1.1.0) states that `with_local_transport` is the only transport entry in 1.1.0's ordinary build. For 1.1.0 this decides the reuse design's open item on network operations without a binding: both schemes take the native route. Its recommendation to refuse binding-less SSH is decided at S7.1 (1.2.0). | In the candidate build, `Git2Backend::new()` against the disposable SSH fixture constructs no transport endpoint, and the same operation inside `with_local_transport` takes the transport. S7.3 (1.1.0) and the post-release check run one gwz-py SSH and one HTTPS operation per platform and assert the native route. All of these fail on today's candidate. |
| C P2-2, S P2-2 | **Accept.** §3.10 and §3.11 supersede these by name for 1.1.0's run, each reading "native (OD14)": the 1.1.0 amendment's §3.5 S7.1 gwz-py bullet, whose sites move as C P3-5 says, and its "S6.1's variant and S6.2's arms" bullet, which is moot; its §3.5 S7.2 Python row, whose evidence becomes S7.3 (1.1.0)'s native-route assertion; its §3.5 S7.3 Python sentence, where two overlapping Python operations assert the native route; its §3.6 step-7 sentence; and its §3.6 post-release Python sentence. Plan line 417's adoption of that S7.2 addition is 1.2.0's. | No 1.1.0 sentence requires a Python transport route. S7.3 (1.1.0)'s Python rows fail on a build with the leak of C P2-1. |
| C P2-3 | **Accept.** New step **TR3.4: gwz-py's release pins** (under 150 lines). It brings S6.2's release-pin obligation into 1.1.0, from CS5.3, with CS5.3's closure test. The session plan's §5.3 row and CS5.3's file list lose it (§3.14). | On gwz-py's 1.1.0 release branch, `publish.yml` refuses a git-tag pin and accepts `=1.1.0`. `release.py`'s unit test shows the registry form, `RELEASE.md` names every native pin, and `test_native_module_reports_compiled_core_provenance` accepts the registry form. |
| C P2-4, S P3-4 | **Accept.** TR8.4 becomes its own step, beside TR8.1 (1.1.0) and before S5.4 (1.1.0). S5.5 (1.1.0) repeats both steps' targets after S5.4. Line 405's defaults wait, for 1.1.0, on TR8.1, TR8.4 and S5.3. §3.12's sketch follows. | A topological sort of §3.12's Phase 8 edges, plan line 480 and S5.5's text succeeds. S5.6 carries the repeated rows' evidence IDs. |
| C P2-5 | **Accept.** §3.14 amends the session plan's G2: the bullet "No tag precedes any phase" becomes the two-release rule, and its merge-timing bullet (a) names both switches. G2 joins §1's list and the session plan's status sentence. | G2 and rule (e), read together, give one answer for a marked and for an unmarked step, before and after S7.1 (1.1.0). |
| C P2-6 | **Accept.** §3.9: S5.1 (1.1.0) reads without "long-lived reuse", which is TR8.2's (1.2.0). "Both placements" stays `local` only (plan line 110). S5.2 and S5.3 apply as written. | Each S5.1 row for 1.1.0 names an operation measurable on the in-process CLI. |
| S P2-1 | **Operator decision OD16** (§2). Revision 1 recommends shape (b), writes TR1.8 for it, and records (a). TR1.8 names its fixture. | The dabeest fixture row. Under (b), the operation succeeds on the native route; the transport's record shows one anonymous request and no credential offered; and the `--verbose` row names the route's cause. |
| S P2-3, C P3-1 | **Accept.** New step **TR2.12: the second switch** (under 100 lines), which can start now. It declares `gwz_session_candidate`: the three `check-cfg` declarations (`gwz-core/build.rs`, `gwz-cli/build.rs`, `gwz-py/Cargo.toml`), the conditional-compilation check's second name, and a candidate CI configuration that builds with both cfgs. Rule (a) becomes two named switches, each with an inventory of its sites in the checkpoint. Rule (e) binds from TR2.12's merge. Until then, a session step whose ordinary-build change has no accepting review waits. | A source test that every `gwz_session_candidate` site is in the checkpoint's inventory. The candidate job builds with both cfgs on every push. |
| S P2-4 | **Accept.** S7.1 (1.1.0) gains a positive inventory and an absence check. Before the step, the checkpoint lists every 1.2.0 site by file and symbol; after it, `rg gwz_session_candidate` equals that inventory. The ordinary build on all three platforms then asserts: `gwz server` is an unknown command; `--server` and `--no-server` are unknown options; `GWZ_SERVER` has no effect, since the `--verbose` row shows the in-process route; gwz-py exposes no `SocketCoreBridge` and no `server` entry; and `transport_capabilities` reports no session route. S7.5 (1.1.0)'s Surface list gains "the absence of the 1.2.0 surfaces". | Those absence assertions run in S7.3 (1.1.0) and in the post-release check. |
| S P2-5 | **Accept.** TR1.8 adopts libgit2's resolution order for the SSH home on Windows as the transport's rule: `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE` (`libgit2/src/libgit2/sysdir.c:329-330, 357`). The transport then reads the same `known_hosts` as the native path. 1.1.0 S4.4's Windows arm implements it. Every Windows evidence step includes at least one row with `HOME` unset, recorded as its own row: TR1.8's 1.0.17 rows, TR8.4, S7.3 (1.1.0) and the post-release check. | A dabeest row with `HOME` unset and `%USERPROFILE%\.ssh\known_hosts` holding the fixture's host key. The transport route succeeds, and the row records which home resolution applied. |

## 4. Dispositions of the non-blocking findings

All are applied in revision 1.

| Finding | Disposition |
|---|---|
| C P3-2 | Rule (e) uses the session plan's marker. An unmarked step merges as G2 says. A step marked **Ordinary path** merges before 1.1.0's tag only if its review accepts the change for 1.1.0 and the checkpoint lists it under (d). |
| C P3-3 | S7.5 (1.1.0)'s Surface list gains "every ordinary-path change admitted under (e) that §6(c) names". |
| C P3-4 | Revision 1 states the fact: gwz-py `a342b95` removed `TransportSession` on 2026-09-28. The S7.1 clause and §4's item go. §3.14 says that the session plan's mentions of `native/src/transport_session.rs` read as done. |
| C P3-5 | S7.1 (1.1.0) applies in all three crates, as line 413 says. The candidate protocol's placement projection moves behind `gwz_session_candidate`: `TransportPlacement`, `TransportOptions.placement` and `.endpoint_path_base`, and the `transport_message` fields. So do the cfg sites that select the candidate protocol in gwz-core, and gwz-py's two schema arms (`dispatch/mod.rs:423`, `dispatch/merge.rs:95`). The ordinary build compiles the transport against the production schema. Any candidate field the 1.1.0 transport needs in the ordinary protocol is named by S7.1 (1.1.0), regenerated in gwz-py, and placed under S7.5 (1.1.0)'s Surface. |
| C P3-6 | §3.3 assigns every section of amendment 1, §3.1 to §3.14, §4 and §5, by name. |
| C P3-7 | §5's status sentences use AgentProcessRules §7.2's amendment pattern, and the plan's scope list adds Phase 3 and §4's adopted table. |
| C P3-8 | §3.6 replaces line 167's second sentence with the exceptions: TR2.5 waits on TR1.5, TR2.2's route on TR1.6, and 1.1.0 S4.3's agent forms and S4.5's routes on TR1.8. |
| C P3-9 | The operator's ruling of 2026-09-28 is kept. TR3.3 registers the thirteen names of the [crate map](../../dev-docs/GwzCoreSessionCrateMap.md) together, before 1.1.0's Phase 10. In 1.1.0, the four ordinary crates on main publish real versions; the other names stay placeholders until 1.2.0 publishes them. The crate map joins §1: its sentence that candidate crates are "published at activation with gwz-transport, before gwz-core 1.1.0" reads as 1.2.0's, and no candidate crate enters 1.1.0's ordinary build. |
| C P3-10 | §3.6 states that for 1.1.0 the retry plan's §3 item 7, §6 and S1.4, and the HTTPS design's §4 snapshot, apply as written. The reuse design's supersessions take effect with its steps in 1.2.0. TR2.10's "operation's per-host limit" is the retry plan §6's. |
| C P3-11 | §2 item 2 reads "12.6 s at its defaults (12.3 s at `--max-per-host 32`)". |
| C P3-12 | Line 18's replacement keeps the general rule: in text assigned to 1.2.0, "1.1.0" and "v1.1.0" read 1.2.0. §3.4 writes "S7.1 (1.1.0)". The new §8 sentence reads "Every Windows transport row that needs a fixture runs on dabeest". |
| S P3-1 | TR1.8's list gains the inverse proxy case: an environment proxy that 1.0.17 ignores and the transport refuses or uses. TR1.8 decides whether the native route fires on any environment proxy or only on one the transport refuses. S7.2 (1.1.0)'s notes state the transport's proxy policy against 1.0.17's, on every platform. |
| S P3-2 | TR8.1 (1.1.0) and TR8.4 add a native-route row at the default settings, with the off switch on as the simplest native route: 16 and 32 members, three rounds, no partial result, on macOS and dabeest. |
| S P3-3 | TR4.6's job also runs the ordinary suite on `windows-latest`. Phase 10 (1.1.0) adds a precondition: before step 5's gwz-core tag, `windows-matrix.yml` is dispatched on the release commit and passes, and the checkpoint records the run. |
| S P3-5 | TR3.3's registry steps precede TR3.2's removal of gwz-core's bootstrap workflow and the revocation of its token. TR3.3's exit is the thirteen names visible on crates.io, with trusted publishers configured for the workflows that publish them, recorded in the checkpoint before Phase 10 (1.1.0). |
| S P3-6 | TR2.9 names the git2-rs fork's vendored libgit2 as a possible site. A fix there moves Phase 10 step 1's libgit2 hash by amendment, before Phase 10. |
| S P3-7 | OD17 (§2). S7.1 (1.1.0) waits on it when TR8.1 (1.1.0) misses. |
| S P3-8 | TR1.8 names the WinHTTP proxy fixture: a loopback CONNECT proxy, the administrator step that sets it, and a restore step that ends every row (`netsh winhttp reset proxy`), all under line 64's rules. |

## 5. New steps and budgets

| Step | Repository | Budget |
|---|---|---|
| TR2.11: no host context, no transport | gwz-core | under 200 production lines |
| TR2.12: the second switch | gwz-core, gwz-cli, gwz-py | under 100 lines |
| TR3.4: gwz-py's release pins | gwz-py | under 150 lines |

TR8.4 changes from a part of S5.5 into its own evidence step. TR1.8's list grows by the home rule, the inverse proxy case, OD16's trigger and the proxy fixture.

## 6. Integration and re-review

- **One patch.** Revision 1 of the amendment applies every disposition above in one change. Nothing else changes.
- **Re-verdict.** The same two reviewers receive revision 1's SHA-256, this plan and the diff from revision 0. Each re-verdicts its own findings with a closure table, and checks the changed range for new root causes.
- **New round instead** if revision 1 changes a shared interface beyond these dispositions. It does not: TR2.11, TR2.12 and TR3.4 are steps the findings specify, not new architecture.
- **Cap.** This is remediation round 1 of at most 2.
