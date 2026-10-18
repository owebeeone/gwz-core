# Credential implementation remediation round 1 — CODE-AXIS REVIEW

**Review object:** Committed credential-helper correction in `/Volumes/projects/limbo/gwz-dev-tr2-22`, frozen on 2026-10-03. Controlling DRAFT: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan.md` at core `64ec039089b6e217625d7784b106d917452963cb`.

**Baseline:**

| Repository | Reviewed HEAD |
| --- | --- |
| root | `1de9e7fcbd17e5159ebb71ef3b683c2656a1cf8a` |
| gwz-core | `64ec039089b6e217625d7784b106d917452963cb` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Source changes were inspected through committed range diffs and targeted numbered reads. Read-only hashing compared the 39 remediation-owned files against `git show HEAD:<path>` bytes.

**Date:** 2026-10-03

**Axis:** Code — architecture, interfaces, ownership, call graphs, compatibility and error paths. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on their current reports. Filed verbatim by the lane owner.

**Verdict: GO** — zero open P0, P1, P2 or P3 findings on this axis. All six distinct original blocking causes are closed by source retracing and the inspected source-bound regression evidence. No new architectural root cause was identified.

---

## 0. Evidence base

The complete canonical `PromptCode-1.txt` was read first. All six HEADs matched the required tuple at the beginning and end. No tracked modifications were reported. The excluded root drafts, core private drafts and old evidence directory remained unchanged and their contents were not used.

Authority inspected included workspace and member instructions, the current credential checkpoint, process rules and optimization amendment, core design/requirements credential sections, the controlling RemPlan, credential design revision 4 and its timing/configuration-view/SSH-clock amendments, retry eligibility and library-boundary/release authority. Original Code, State and Surface reports were legitimate prior-round inputs. No current-round peer report was read.

Fresh source inspection concentrated on:

| Area | Sources inspected |
| --- | --- |
| Atomic authority | Transport `pool/setup_clock.rs:1–303`, `state.rs`, `transitions.rs`, and the atomic-admission regression |
| Detailed publication | Core `ssh_setup_context.rs:1–232`, `publication.rs:1–59`, publication tests and immediate-observer test |
| Result admission and secrets | `https_auth/runner.rs:1–219`, `lookup.rs:1–145`, `secret.rs:1–116`, runner tests and username regression |
| Operation policy and pooling | SSH handoff, local selection, Opening, worker runner/completion, key admission/token generation, transport binding and session opening |
| Carried failure/classification | Transport `stream/incoming.rs:1–169`, retained accessors, close-handshake regressions; core `https_remote.rs`, RPC tests, challenge projection and private-materialization regression |
| Retained package contracts | Configuration view/framing/file-worker/executable owners; route credentials; typed SSH/HTTPS renderers; first TransportAttempt error and clone/fetch/push consumers; retry classifier; configured budget carriage |
| Source composition | CLI help-only diff; Python README/shared-help diff and additive generated code75/drift sources |

Unchanged package evidence from the original reports was reused where the correction did not alter its proof surface. This was not represented as a fresh execution or whole-package reread.

The final receipt’s SHA-256 matched the mandated value:

`ceed1a3740aa76ab3609c1758392bea9d3cabe6d04200e462147addd2ed523d`

All **39 owned source hashes matched committed bytes**. All eleven final gate-log hashes checked against the receipt matched. The recorded logs contain:

- Core affected union: **455 passed, 0 failed, 4 ignored**, including all thirteen new core regressions.
- Transport: recorded full suite/doctest success, including the two close regressions and atomic admission regression.
- Strict transport library Clippy: exit 0.
- Core Clippy: exit 0 with **49 retained warnings**; the opening arity expansion remains explicitly unwaived.
- CLI build and three help/retry tests, Python 28 parser tests/drift, and source guards: recorded success.

The disclosed default-concurrency physical SSH failure remains historical evidence; bounded-concurrency success is not a claim that it never occurred. Earlier whole ordinary/candidate suites remain pre-correction.

No builds, tests, probes, writes or Git mutations were performed. Closure below means independent source retracing, supported by inspected recorded executions.

## 2. Invariant analysis

**Terminal admission and association.** `terminate_if_alive` executes inside the existing `change` mutex path, after time sampling and expiry settlement. `Ok` identifies this invocation’s new admission; `Err` identifies the existing terminal. Equal scalar resource causes therefore cannot authorize an unrelated detailed failure. The old `terminate` contract remains intact.

The core Publication reserves association before calling the authority. Readers and competing publishers release the context mutex while waiting on the Condvar. Commit holds no context lock. Drop associates detail only for `Ok`, clears publication, releases the mutex, notifies readers and then delivers clock notifications. Reentrant wake consumers consequently see the completed association. Precommit abandonment and postcommit unwinding resolve the reservation. Earlier expiry/cancellation and independent equal-scalar terminals cannot acquire unadmitted helper detail.

Actual capture sites were checked for outer locks across this wait. OpenRequest now clones `setup_slot` in a completed statement before invoking capture. The publication owner requires neither pool Runtime nor cleanup ownership to finish association.

**Late successful answers.** Completed output is checked before finishing; final parsing checks before and after parsing; the runner checks again before returning success. The same deadline is retained. Already-ready cancellation, equality at the deadline and parsing across the deadline refuse success. Refused output/Secret values drop through wiping owners. Parsing occurs while HelperJob owns its child/group and admission references; Runner also retains admissions through completion.

**Secret representation.** The sole production Secret constructor reserves username length plus one before copying bytes. `ssh_parts` appends once without growth, and repeated calls do not append again. HTTPS Basic construction excludes the SSH terminator. The regression observes unchanged pointer/capacity across conversion, avoiding freed-memory inspection.

**Private helper enablement.** Backend policy reaches HostRoute, private request opening, Session and UrlExtras. Disabled operations receive an opaque pool identity distinct from enabled operations while Opening retains their original authentication selection. Generated selected-key tokens (`selected-N`) and URL-password tokens cannot collide with the disabled namespace. Key authentication returns under the isolated requested identity; ambient agent selection remains available. The helper branch checks enablement before password-only discovery or lookup. Both operation orders and selected-key reuse have recorded regressions.

**Failure carriers and consumers.** `Closed` validates the envelope before moving its Failure into retention. Its separate facts remain retained, and terminal receive guards prevent duplicates replacing either. RPC enriches only a derived callback value when Failure-owned facts are absent; it preserves existing Failure facts otherwise. The raw retained Failure remains immutable.

Negotiate is prioritized within the existing four-token projection. Later mixed-case occurrences survive encoding and both classification consumers. Basic detection still scans beyond diagnostic capacity. The original fifth-field private-clone sequence now reaches loud `GitCommandFailed` without helper work or realm leakage.

**Retained package boundaries.** Typed helper timing remains necessary for code75; ordinary timeout text does not acquire helper provenance. First TransportAttempt error ownership continues through clone/fetch/push. Authored optional fields, fixed MalformedOutput cause8, bounded carrier validation and generated code75 remain additive.

The configuration path retains native scoped discovery, ordered unconditional includes, byte/null/empty preservation, explicit HOME anchors, controlled-empty stdin parsing and final native verification. Controlled E2BIG remains configuration refusal without a missing-Git latch. The initial native-discovery FIFO limitation remains disclosed. Route/account ownership, method precedence, password-only SSH selection and configured narrower budget carriage are preserved.

## 3. Risks and next action

This GO covers the frozen committed source correction. It does not establish Windows, provider/trust, performance, packaging, selected-source or release outcomes, nor supplied-carrier/iroh or 1.2 session acceptance.

Recorded evidence remains narrower than fresh whole-suite integration. Existing warnings and the disclosed physical SSH timing failure remain unwaived limitations.

The next action is lane-owner acceptance after every required axis reports GO, followed by the authorized GWZ integration and fresh combined CLI/core/Python gates.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
| --- | --- | --- | --- |
| Code P2-1 | Associate exact winning detailed terminal before fallback caching | Immediate wake and independent pool observer wait for association; atomic admission distinguishes equal competing causes; zero allowance retains code75/M10 | Closed |
| Code P2-2 | Preserve Negotiate within four tokens | Original fifth/later challenge retains Negotiate through codec and typed/private-clone classification; Basic beyond capacity still detected | Closed |
| State P2-1 | Correct detailed publication ordering | Original publication/observer sequence cannot cache scalar fallback; unwind and prior-terminal controls preserve first cause | Closed |
| State P2-2 | Size username before copying secrets | Production parser reserves terminator capacity; conversion/repeated conversion cannot reallocate; Basic bytes remain equivalent | Closed |
| State P2-3 | Recheck final successful result admission | Ready work plus timeout/cancel and parsing across deadline are refused under the unchanged deadline; owned values wipe and cleanup remains owned | Closed |
| State P2-4 | Carry private helper-disabled policy and isolate reuse | Policy reaches endpoint branch; disabled namespace prevents enabled pooled reuse; original key selection retained; both orders covered | Closed |
| State P2-5 | Retain admitted Closed Failure and separate facts | Real handshake retains owned exact Failure; invalid detail retains nothing; duplicates ignored; RPC callback receives derived facts without changing raw retention | Closed |
| Surface P3-1 | Explicit conventional-file set/remove recipes | README and shared help source use paired `--file "$HOME/.gitconfig"` commands and explain alternate `GIT_CONFIG_GLOBAL` | Source correction verified; rendered-help gate belongs to Surface |
| Surface P3-2 | Qualify network timing examples | CLI shared help source identifies network-clock examples, separate helper phases and code75 recovery | Source correction verified; rendered-help gate belongs to Surface |

## Changed-range analysis

The correction adds one neutral atomic terminal API and one bounded core publication owner; neither changes wire fields, timing policy, helper policy ownership or transport secret boundaries. The Condvar is confined to logical association, with wake delivery outside locks.

Other production changes are bounded to the stated dispositions: final result checks and parsing ownership, username capacity/Basic equivalence, Negotiate projection priority, private operation enablement and pool partition, and Closed/RPC retention. New test leaves exercise these seams. Existing fixture calls receive the explicit default-enabled argument.

CLI changes are help-only. Python round1 changes are README/shared help atop the composed accepted configuration implementation and generated code75 ancestor; no new Python credential mechanism is introduced.

No change outside the dispositions established a defect. **NEW ARCHITECTURAL root causes: zero.** The six original blocking causes remain non-architectural and are closed for this tuple.
