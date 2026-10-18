# GwzV110PlanAmendment — CONSISTENCY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzV110PlanAmendment.md`, uncommitted (`??`) in the gwz-core working tree, SHA-256 `774bb164c1c33122738b6864cabcc81203404bd25372bebe32b3984c1cdd7acf`. Status line: "draft; not implementation authority". Dated 2026-09-26. Hash verified identical at start and end of review.
**Baseline:** root `9a65306544e19ee4fbb88757930764950c7494d2`; gwz-core `b13bbadb22c0238f1bc3f88f41c26a79a0669e0c` (plan `dev-docs/GwzV110Plan.md` unchanged since `b141e26d`; hash `583dd3089df9bc1e972e174a386ed80e9e69597f2b0a94a89df833742d03659c` at `b141e26d`, HEAD and working tree); gwz-py `4ad2b077ac473c079a62cdd7d5b7317a78fb5f1f` (dev-docs clean). Controlling documents read with `git show` at the cited commits: plan at `b141e26d`; contract at root `5a5d6cf` (revision 3) and `58ea74b` (revision 2); Verdict-2, proposals, AgentProcessRules §7/§8.5/L1-06/L1-08/L2-03, GwzProcessOptimization, the plan's Consistency-2 and Safety-2 reviews. Code inspected read-only with `rg` to verify placement claims. No builds, tests, writes or git mutations.
**Date:** 2026-09-26
**Axis:** CONSISTENCY — the amendment against the plan, the contract, the proposals, the verdicts and the process rules. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 2 P2, 5 P3. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified below; the P3s are bounded text corrections that may land in the same revision.

---

## 0. Evidence base

| Source | Revision read | Role |
|---|---|---|
| `gwz-core/dev-docs/GwzV110PlanAmendment.md` | working tree, `774bb164…` | object |
| `gwz-core/dev-docs/GwzV110Plan.md` | `b141e26d` (= HEAD = working tree, `583dd308…`) | controlling; line numbers as cited |
| `dev-docs/GwzCoreSessionDesign.md` | `5a5d6cf` (rev 3) and `58ea74b` (rev 2) | adopted design; section numbering verified identical across the two revisions |
| `dev-docs/GwzCoreSessionDesign-Verdict-2.md` | HEAD | acceptance of rev 2; two-round cap accounting |
| `dev-docs/GwzClientCoreTransportProposals.md` | HEAD | §1, §2 G1–G11, §8, §9 |
| `dev-docs/AgentProcessRules.md` §7.1, §7.2, §8.5, L1-06, L1-08, L2-03; `dev-docs/GwzProcessOptimization.md` §4.1, §4.2 | HEAD | process authority |
| `gwz-core/dev-docs/GwzV110Plan-ReviewConsistency-2.md`, `-ReviewSafety-2.md` | HEAD | the plan's review rule (dual Consistency/Safety on the plan's SHA-256) |
| `gwz-py/dev-docs/GwzPyTransportDesign.md` | gwz-py HEAD `4ad2b07` | the object the plan's S1.1 (line 78) named; not cited by the amendment |
| `dev-docs/CurrentProgramCheckpoint.md` | root HEAD | 2026-09-23 acceptance record; working-tree copy is modified and was not relied on |
| `dev-docs/history/GwzPyTransportConcurrencyNoGo.md` | HEAD | origin of the "Phase 6/7 NO-GO" |
| `gwz-core/src/lib.rs`, `gwz-core/src/git/gitbackend/transport_binding.rs`, `gwz-core/src/transport_host/{local_command,request}.rs`, `gwz-cli/src/globalargs/dispatch.rs` | HEAD | placement of `with_local_transport`, `TransportRequest`, the "binding construction", `execute_invocation` |

Hash-claim verification: the amendment's `9d49af85…` is the plan's own accepted hash (matches the plan's status line and both `-2` reviews; it corresponds to no commit because the status sentence was added after GO, as the plan says). `583dd308…` is the plan at `b141e26d`, HEAD and working tree. The `b141e26d` diff replaced the candidate S6.3 clarification with the closing note, as §1 states.

## 1. Findings

### [P2-1] The plan's original S1.1 object, its accepted Phase 1/6 amendment and its live NO-GO are outside the amendment's controlled graph, so the precedence trail L1-08 requires does not exist

- **Location:** amendment §1 (documents controlled), §2 item 5, §3.2, §3.4, §3.5, §3.7, §5 "On GO".
- **Root cause:** the amendment does not name `gwz-py/dev-docs/GwzPyTransportDesign.md`, the file the plan's S1.1 (line 78) designates as the design, whose committed status (gwz-py HEAD, lines 3–12) records three things the amendment contradicts or drops:
  1. "Status: **S1.1/S1.2 design accepted for implementation, 2026-09-23**. Consistency, Safety and Surface report GO at Python `259f73cc…` … Operator authorized implementation." The program checkpoint at root HEAD records the same acceptance. So the plan's Phase 1 gate was met once, on that object. Amendment §2 item 5 states the opposite: "The plan never names the contract. Until it does, S1.2's gate … and Phase 6's 'Starts only after the Phase 1 GO' have no object that can meet them." The plan's S1.1 named an object, and it met the gate.
  2. "This accepts the bounded package-boundary amendment to the [1.1.0 plan] §3 Phase 1/6". Its §1 (lines 26–35) re-decided S1.1's binary choice, S6.1/S6.2's binding-crate edge and Phase 8's steps ("no fifth crate or extra Phase 8 release step is needed. `gwz-transport` still publishes at Phase 8 step 2, core links it at step 5, and `gwz-py` links registry-pinned core at step 7"). That accepted amendment was never applied to the plan's text or status. The amendment under review supersedes the same clauses a second time without naming the first.
  3. "Current release-gate status (2026-09-24): **NO-GO for Phase 6 completion and Phase 7 activation** … The historical design GO below remains the verdict on the earlier review object; it does not close the NO-GO." The plan's closing note (line 446, written in the retirement commit `b141e26d`) keeps it open: "S6.3 and the Phase 6/7 NO-GO remain open." Amendment §2 item 5 calls it "the retired train's 'Phase 6/7 NO-GO'" — the NO-GO document predates and caused the train (history/GwzPyTransportConcurrencyNoGo.md, dated 2026-09-23, "operator-directed post-design finding") — and §3.7's replacement removes it from the plan with no closing condition, while its original closing condition (the retired remediation plan's gates) no longer exists.
- **Violated invariant:** AgentProcessRules L1-08 ("both old and new documents contain an unambiguous precedence trail"), §7.2 ("a reader can identify one current authority"), §7.1 amendment content "documents controlled … problem evidence".
- **Reproduction:** after the §5 status edit, read Phase 1/6 authority from the tuple. The plan says the design is the core session contract, S6.1 is in gwz-core, and Phase 6 waits on S1.2. gwz-py's design says S1.1/S1.2 are accepted for implementation on the long-lived `TransportSession` design, "S6.1 and S6.2 can be one `gwz-py` implementation lane after S1.2 GO", and the operator authorized implementation — and per the checkpoint that implementation is in working source. Both carry current status language; neither points at the other.
- **Impact:** two live, contradictory authorities for Phase 1 and Phase 6; the problem-evidence section of a §7.1 amendment is factually wrong on the plan's own history; the Phase 6/7 gate the operator kept open at `b141e26d` disappears from the plan without a step that closes it.
- **Required correction:** (a) add `gwz-py/dev-docs/GwzPyTransportDesign.md` to §1 as a document affected; (b) rewrite §2 item 5 to state that Phase 1 closed on 2026-09-23 on that design, that the 2026-09-23 NO-GO reopened Phase 6/7, and that the train's retirement left the NO-GO without a closing condition; (c) in §3, state that the design's §1 package-boundary amendment to Phase 1/6 and its "accepted for implementation" status no longer close S1.2 for this plan; (d) in §3.7's replacement, name what closes the NO-GO (the S6.3 overlapping-operations row passing on both bridges, or an explicit operator lift) instead of deleting it; (e) in §5 "On GO", add the paired status line for the Python design (Verdict-2 next item 1 already flips its pointers; the plan-facing status must flip with this amendment).
- **Closure test:** on the amended tuple, `rg -n "accepted for implementation|package-boundary amendment|Phase 6/7 NO-GO" gwz-py/dev-docs/GwzPyTransportDesign.md gwz-core/dev-docs/GwzV110Plan.md` returns only lines that name the amendment as controlling or a closing step; the amended plan names exactly one Phase 1 object and one NO-GO closing condition.

### [P2-2] S6.1 is declared gwz-core-only and gwz-cli is placed outside 1.1.0, but the contract S6.1 implements moves gwz-cli code into core and changes the core APIs gwz-cli calls; no step owns the gwz-cli adaptation

- **Location:** amendment §3.4 ("S6.1 belongs to gwz-core"; S6.1 text), §3.6 second bullet, §4 ("S3, S4, S5 and S7 are unaffected"), §5 ("this amendment changes no command, option or public API"), §6.
- **Root cause:** the amendment's own scope decision (§3.6/§6, in scope per the review brief) accounts only for "moving gwz-cli onto the session host" (contract §11, §15 item 14). It does not account for two other contract clauses S6.1 implements:
  - §5.2: "The worker runs the shared dispatch. That is gwz-cli's execution path (`execute_invocation`), moved into gwz-core as the shared dispatch … The shared dispatch also absorbs gwz-cli's diff, log and hook paths". gwz-core's paired GWZDesign paragraph says the same ("moved into core as a shared dispatch"). `execute_invocation` is defined at `gwz-cli/src/globalargs/dispatch.rs:4` and called from `gwz-cli/src/lib.rs:266` and five test files.
  - §16 "Core API changes": the fetch and push handlers "must take the host context's member lock through the gate"; handler contexts gain the token; a session variant of `with_local_transport` is added. gwz-cli calls `gwz_core::transport_host::with_local_transport` at `dispatch.rs:8` and the handlers through its own dispatch.
- **Violated invariant:** L1-06 (one coherent owner; "a discovered cross-package edit … stop for handoff"); L2-03 cross-driver parity; §8.5's "Interface, wire, ownership, and platform effects" content, absent from the amendment.
- **Reproduction:** implement S6.1 as written. Either the implementer moves `execute_invocation` into gwz-core and re-points gwz-cli (an unowned gwz-cli edit in a step that "belongs to gwz-core"), or leaves gwz-cli's copy in place (two dispatch tables, contradicting "shared dispatch", proposals §8.1 and G2), and either way the fetch/push handlers must keep a no-gate entry for gwz-cli's legacy path that the contract does not specify. S7.3 then builds a CLI whose adaptation no step authorized; Phase 8 step 6 tags it v1.1.0.
- **Impact:** the amended plan cannot be followed for gwz-cli without an out-of-plan decision; §4's "S7 unaffected" is wrong (S7.3 builds a CLI whose core dependency changed shape); §5's "no … public API" is wrong for gwz-core's published crate API (§16), which is the basis for waiving Surface; drivers diverge in 1.1.0 on `cancelled` (73) semantics and cleanup reporting (Python on the session host, CLI on the legacy path), an L2-03 parity exposure the amendment does not record.
- **Required correction:** state which path gwz-cli runs in 1.1.0 (legacy `with_local_transport` with the deprecated `cancellation_handle()`, per contract §5.2/§16) and that the contract's legacy exceptions therefore stay until the later phase; add to S6.1, or as a gwz-cli step before S7.3, the gwz-cli change the dispatch move and the §16 API changes force, with owner and files; correct §4 and §5 accordingly and record the driver parity consequence. If the intent is that gwz-cli keeps its own dispatch until the later phase, say so explicitly, because it contradicts the contract's "moved" and GWZDesign's paired paragraph.
- **Closure test:** every gwz-cli file the dispatch move or §16 touches is named in an owned step; S7.3's CLI build is reachable from that step, not from an unowned edit; `rg -n "execute_invocation" gwz-cli/src` is reconciled with the amended S6.1 text.

### [P3-1] §2 line 50's "one pool" survives in the in-release column unclassified

- **Location:** plan line 50 (`| SSH and gh-only HTTPS, both placements, one pool | … |`); amendment §3.1, §3.8.
- **Root cause:** the amendment supersedes line 52 and S1.1's "One pool, owned by `gwz-transport`, shared with the Rust host", and states (§2 item 4) that "connection reuse across operations is out of the contract's scope", but neither supersedes nor lists in §3.8 the "one pool" cell at line 50, which carried the same meaning.
- **Violated invariant:** the amendment's own completeness rule (§1: "Only the clauses in §3 change"); S5.6's rule that an unsupported mark on a §2 in-release cell blocks S7.5 and Phase 8 "unless an accepted amendment first removes that cell".
- **Reproduction:** read the amended §2. Line 50 promises "one pool" in-release; line 52 (replaced) promises "each operation's `gwz-transport` runtime". Under per-operation runtimes a Python process running eight operations has eight pools.
- **Impact:** an in-release cell that 1.1.0 no longer delivers under the original reading, with S5.6's blocking rule attached to it.
- **Required correction:** supersede line 50's "one pool" (for example, "gwz-transport's pool, one per operation runtime") or add it to §3.8 with the reading intended.
- **Closure test:** no §2 in-release cell describes cross-operation pooling.

### [P3-2] S1.2's closure rule is a floating reference with no named record, pinned by date to revision 3 and to two files that do not exist, while restating a remediation cap the contract has already consumed

- **Location:** amendment §3.2, S1.2 replacement.
- **Root cause:** "S1.2 closes when both axes report GO on the revision that implementation follows. On this amendment's date that is revision 3, whose focused re-verdict files `…-3.md`." Nothing names where "the revision that implementation follows" is recorded. The `-3` files do not exist in root dev-docs (only `-1`, `-2` and unsuffixed). "remediation capped at two rounds" is restated although Verdict-2 says "The object used both remediation rounds the cap allows and closed on the second", so as written S1.2 admits no remediation of revision 3 and states no branch for a NO-GO on it (implementation follows the accepted revision 2 with the P3s carried into the implementation plan per Verdict-2 next item 2, or the lane stops).
- **Violated invariant:** L1-07 (define freeze words precisely); §7.2 (one current authority identifiable without external context); L1-08 (a later revision must not be silently reinterpreted into the plan).
- **Reproduction:** revision 3's re-verdict returns a bounded NO-GO. The amended plan's S1.2 then names a revision that cannot be remediated under its own cap, and the sentence "that is revision 3" is false rather than stale.
- **Impact:** the gate is well defined only on the day the amendment was written; on any other outcome the plan text needs re-amendment or silent reinterpretation.
- **Required correction:** define closure as "both axes GO on the contract revision whose status line records acceptance and which the program checkpoint names for implementation; a later revision reopens S1.2 until its own dual GO"; mark the revision-3 sentence as a snapshot or drop it; state the NO-GO branch; drop the restated cap or cite GwzProcessOptimization §4.1's permitted non-architectural third round.
- **Closure test:** the rule identifies one revision from the tuple and the checkpoint alone.

### [P3-3] §4 misquotes the contract's §14 wording while attributing it verbatim

- **Location:** amendment §4 first bullet and §3.4 S6.3.
- **Root cause:** the amendment writes "two overlapping Python network operations on one Client complete independently, each on its own runtime", "the contract's §14 wording". Contract §14 (rev 3 line 549, rev 2 line 544): "two overlapping Python operations complete independently on one Client, each on its own runtime". "network" is inserted and "on one Client" moved.
- **Violated invariant:** exactness of quotation from a controlling document (§7.1 "exact superseded clauses"; L1-08 "names the superseded text").
- **Impact:** the plan and the contract carry two different sentences for the same test under a false attribution; "network" narrows the row relative to the contract (whose §15.4 also covers local overlaps).
- **Required correction:** quote the contract verbatim, or say "after the contract's §14" without quotation marks.
- **Closure test:** the quoted string appears identically in both documents.

### [P3-4] S6.3's ownership statement does not match its content

- **Location:** amendment §3.4 ("S6.3 belongs to gwz-py, plus the host binary that gwz-core ships as a Cargo example target"; S6.3 = "the verification list of §15 except item 14"; S6.1 = "the channel and its in-process adapter").
- **Root cause:** §15 contains gwz-core rows (§15.4 "A core test asserts…", §15.5 two "Core test:" rows, §15.6's fault-injected finish panic, §15.8's `check_process_globals.py` over gwz-core and gwz-transport), and §12's host binary needs the byte-stream adapter of §3, which S6.1 omits by naming only the in-process adapter. That gwz-core library and test work has no gwz-core-owned step.
- **Violated invariant:** L1-06; the sketch's order S6.1 → S6.2 → S6.3 places core's own tests after gwz-py's bridge.
- **Impact:** core work lands in a step declared to belong to gwz-py, or in no step.
- **Required correction:** assign §15's core rows and the byte-stream adapter to S6.1 (or extend S6.3's ownership with the gwz-core files) and record it in §4.
- **Closure test:** every §15 row S6.3 adopts has an owner repository matching where its test lives.

### [P3-5] S6.1 gains an unsketched prerequisite while §3.8 declares the normative sketch unchanged

- **Location:** amendment §3.4 ("The contract's implementation plan … is written and reviewed before S6.1 starts"); §3.8 first bullet; plan lines 305–306 ("The dependency sketch is normative").
- **Root cause:** the implementation plan is a new gate before S6.1 with no step ID, owner, review tier (GwzProcessOptimization §4.2 records tiers per checkpoint) or edge in the sketch.
- **Violated invariant:** the sketch's normative status; L1-10 (budget before implementation) is delegated to a document no step produces.
- **Impact:** an implementer following the normative sketch starts S6.1 on S1.2 GO without the implementation plan.
- **Required correction:** add a step (for example S1.3, 0 product lines, with its review tier) and the edge `S1.2 ── S1.3 ── S6.1`, or remove the sentence.
- **Closure test:** sketch and prose list the same predecessors for S6.1.

## 2. Invariant analysis

**Exactness of superseded clauses (all verified verbatim against `b141e26d`):** §3.1 line 52 — exact. §3.2 lines 72–100 — heading 72, paragraph 74–76, S1.1 78–91, S1.2 93–97, trailing sentence 99–100; ranges exact. §3.3 lines 125–127 — exact across the line breaks. §3.4 lines 276–301 — Phase 6 spans exactly those lines. §3.5 step 2 lines 377–378, step 3 lines 380–383, step 7 lines 401–402 — quotations exact. §3.6 "after line 442" — line 442 is "A Python implementation of the pool." §3.7 line 446 — the second sentence is quoted exactly; the note has two sentences as stated. §2 item 1's two quotations (lines 80–81, 299) and item 5's three (lines 96–97, 278, 446) — exact.

**Exactness of contract claims (rev 3; numbering identical in rev 2):** §5.2 "builds the operation's own runtime" — line 222, exact. §1 out-of-scope "connection reuse across operations" — line 25. §14 "The plan text is amended when this contract is accepted." — line 549, exact. §9 host-context constructor plus four operations — lines 411–419. §10 thin client — line 432. §12 host binary as a gwz-core Cargo example target — line 491. §11 "In a later phase" — line 483. §15 item 14 is gwz-cli — line 639. §5.8 ordinary builds — lines 347–356. Session variant of `with_local_transport` — lines 221, 654. Paired paragraphs in gwz-core's GWZDesign/GWZRequirements and gwz-py pointers — present at HEAD as the contract's line 7 says. "No change to `gwz-transport`" — consistent: `TransportRequest` and `with_local_transport` live in `gwz-core/src/transport_host/`, not in gwz-transport; the contract's "transport API" changes are core's.

**Completeness sweep of the plan** (`binding`, `pool`, `S1.1`, `S6.`, `RELEASE.md`, `Phase 6`, `Python`, `gwz-py`, `publisher`): every hit is superseded (52, 78–100, 125–127, 276–301, 377–378, 380–383, 401–402), kept by §3.8 with a verified reading (line 218 "binding construction" — `gwz-core/src/git/gitbackend/transport_binding.rs` is a `cfg(all(unix, gwz_transport_candidate))` site, so the reading is credible; lines 64, 416, 429), or unaffected (5, 26, 74–76, 113 "four crate names", 140–147 trusted publisher, 228, 254, 326–327, 342–355 Transport Plan Phase 6 rows, 403, 406–408, 442) — except line 50 (P3-1). The plan's Phase 1 preamble (74–76) is kept although its rationale for a binding is now inverted; it remains true as a statement of the pre-S7.1 state, so I record it as a risk, not a finding.

**Coherence of the amended plan:** dependency sketch and prose agree (S6 waits on S1.2; S7.1 waits on S6) except for P3-5. Phase 7's dependence on Phase 6 unchanged and consistent. §2's redaction rule for S6.3 still binds. Phase 8 keeps step numbers 4–7 after step 3 is voided; step 7's replacement dependency is sound (gwz-core 1.1.0 carries the transport transitively). §1's "connection pooling" holds within an operation; "normal … gwz-py builds" holds after S7.1 because the candidate switch is in core. S6.1/S4.5 coordination is consistent with line 228 ("S4 does not wait on the Python design") and with `transport_host` being the `cfg(all(unix, gwz_transport_candidate))` module in `gwz-core/src/lib.rs:50`.

**Agreement with the contract's plan-facing statements:** §14's S6.3 bullet — adopted, but misquoted (P3-3). §11/§15 item 14 — the amendment's "later phase" placement agrees with the contract's text; the amendment's release-scope decision is its own and is examined in P2-2.

**AgentProcessRules §7.1/§7.2:** naming `<ControllingSubject>Amendment.md` — conforms. Required content (documents controlled; exact superseded clauses; problem evidence; replacement decisions; affected tests; mandatory review and hash) — present as sections; the "documents controlled" and "problem evidence" content is defective per P2-1. Draft status line — exact §7.2 pattern. Proposed "Amended <date> by … remains authoritative only as amended for <scope>" — exact §7.2 pattern; the date is hard-wired to the draft date rather than the GO date (risk). §8.5's template sections "Interface, wire, ownership, and platform effects" and "Migration or no-migration rationale" are absent; their absence is where P2-2 would have been caught.

**S1.2 closure rule:** consistent with the original S1.2's dual-axis requirement; not well defined across outcomes (P3-2).

## 3. Risks and next action

- **Working-source implementation of the retired design.** The root checkpoint records Python's long-lived `TransportSession` "in working source". The amendment's "Nothing else in this plan implements the session host … before that GO" does not address that code. Not a defect of the amendment text; the remediation of P2-1 should say what happens to it.
- **Review tier of revision 3.** GwzProcessOptimization §4.2 makes bounded-remediation re-reviews single-axis by default; the amended S1.2 requires both axes on the revision implementation follows. Satisfiable only if the lane owner runs both `-3` files; the amendment should say the dual tier is deliberate.
- **Status-line date and changelog.** §5's "Amended 2026-09-26" will be wrong if GO lands later; §7.2 also asks for a changelog entry, which a "status-only edit" omits (the plan has no changelog; its status line has served as one).
- **Phase 1 preamble.** Lines 74–76 are kept; "Production core does not depend on `gwz-transport`" is true only until S7.1 and now sits beside "core builds each operation's transport runtime". A one-line note that this describes the pre-S7.1 state would remove the apparent contradiction.
- **§5.7 debt.** S6.3 adopts §15.8, which passes with `debt` allowlist entries; the amended plan does not say whether 1.1.0 retires that debt. The contract's §16 discloses the residual interference.

**Next action:** remediate P2-1 and P2-2 as text (add the Python design to the controlled graph with its precedence and NO-GO disposition; own the gwz-cli change the contract forces or state explicitly that gwz-cli keeps its dispatch and legacy path through 1.1.0), fold in P3-1 to P3-5, and re-submit for the focused dual re-verdict. On that revision I pre-commit to GO if P2-1 and P2-2 are resolved as specified.

**Final tuple recheck:** object SHA-256 `774bb164c1c33122738b6864cabcc81203404bd25372bebe32b3984c1cdd7acf` unchanged; root `9a65306…`, gwz-core `b13bbad…`, gwz-py `4ad2b07…` unchanged; plan at `b141e26d` hashes `583dd308…` as at start.
