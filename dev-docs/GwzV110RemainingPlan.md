# GWZ 1.1.0: the remaining work to the release, a phased plan

Status: **DRAFT, 2026-10-15.** It decides nothing; the choices that need the operator are the OQs in section 4. It authorizes no implementation, commit, push, tag or publish beyond what the operator directs.

**Why this document.** The checkpoint's "remaining for 1.1.0" lists (2026-10-14) used the Windows parity plan's step numbers only, and folded most of the 1.1.0 plan's Phases 5, 7 and 8 into "the release train". A read-only audit of 2026-10-15 against the commits and the evidence runs found more open than those lists showed. This plan puts the two numberings side by side and lists every open step once.

**Controlling documents.** The release's steps are a stack: `GwzV110Plan.md` (S-steps), as superseded for scope by `GwzTransportReleasePlan.md` (RP:111-113 adopts S5, S7 and Phase 8 extended), as restricted by `GwzTransportReleasePlanAmendment-2.md` §3.10–§3.12 and §3.21 (A2). The Windows steps are `GwzTransportWindowsParityPlan.md` (WPP). Acceptance of A2 covers revision 2 only; revisions 3–6 were skim-reviewed and 7, 8 and §3.22 are unreviewed drafts (A2:3, A2:782).

## 1. The two numberings

| WPP step | Is the 1.1.0 plan's | State |
|---|---|---|
| 5.1–5.4 | S4.5 (the qualification switch retired, publish workflow) | done: gwz-core `79736469`, `e65c9b06`; gwz-cli `220501a`; gwz-py `28aff11` |
| 5.5 | gwz-sspi activation review (A2 §3.21 part C) | not started; waits on 5.6, 6.2, 6.3 |
| 5.6 | TR4.7 (the Windows implementation review) | **accepted** 2026-10-14 (`GwzTransportWindows-5.6-Review*-2.md`) |
| 6.1 | TR8.4 speed on Windows (A2:229-234) | not started |
| 6.2 | TR8.4 parity rows on Windows | not started |
| 6.3 | S5.5 repeat and the Windows column of S5.6 | not started |
| 6.4 | S7.3 and S7.5 rows on Windows, notes | not started |
| 6.5 | Phase 8 preconditions and the post-release check on Windows | not started |

## 2. State of every open step (audit 2026-10-15)

| Step | State | What remains |
|---|---|---|
| S5.1 aggregate operations | partly: TR8.1's no-op fetch rows (runs `2026-10-04-tr8-1-macos-parity` … `2026-10-13-tr8-1-rerun`, plus the fixed-cost fix's re-time) | push then reads; large packs; **a Linux x86-64 row** (all Linux evidence is the Pi 5, aarch64, which V110:49 lists as unsupported) |
| S5.2 coalescing | not started (only the `db0f8447` defect fix that set the endpoint window to 0) | immediate against coalescing windows including 100 ms, connect counts and wall time |
| S5.3 sustained memory | not started | bounded memory under sustained backpressure, macOS ARM64 and Linux x86-64 |
| S5.4 choose defaults | not started | the measure-then-select note and any construction-default change (A2:266-268: after TR8.1, TR8.4 and S5.3) |
| S5.5 | = WPP 6.1–6.3 | see Phase B |
| S5.6 Design §11 sign-off | not started | the whole cell table, three platforms, each cell an evidence ID or an unsupported mark; an unsupported in-scope cell blocks S7.5 and Phase 8 unless an amendment removes it |
| S7.1 the transport in the ordinary build | not started | about 47 switch sites (inventories: gwz-core 23, gwz-cli 19, gwz-py 5) and about 195 text lines in 72 files; the candidate manifest's dependencies and target tables into `Cargo.toml`; the protocol pair (`candidate_generated.rs` against `generated.rs`); CI's candidate leg becomes the ordinary job. Gates: S5.5, S5.6, S3.3, TR2.6 |
| S7.2 help, migration notes, route ledger | partly: gwz-cli `Releases.md` "Unreleased" entries; the Windows user guide draft | the route ledger, migration notes, helper-policy notes; help matching the shipped switches |
| S7.3 clean consumer builds | not started | the rows on macOS ARM64, Linux x86-64 and Windows (WPP 6.4) |
| S7.4 observation recheck | not started | the whole step, on the activated tree |
| S7.5 activation and release review | not started | dual review plus Surface |
| Phase 8 tags and publish | not started | gwz-transport and gwz-git still `publish = false`; tags; publish jobs; WPP 6.5 |
| S3.x, TR2.6 | **unconfirmed**: no closing record found | to be checked before S7.1 starts |

Also open: the real Pageant 0.83 console rows (operator session); the checkpoint's owed Phase 1, 3 and 4 reviews, which step 5.6's review was made the single review point for (whether that closes them is to be recorded).

## 3. Phases

Foundational first; steps within a phase are independent unless a dependency is named, so separate lanes can take them. dabeest steps share one host and are serialized by the booking lock.

### Phase A: settle the record (documents; no product code)

- **A1.** Confirm S3.1–S3.3 and TR2.6: closing commits and reviews, or the work left. Record whether 5.6's review closes the owed Phase 1, 3 and 4 reviews.
- **A2.** OQ-A answered: the Linux x86-64 evidence host.
- **A3.** Amendment 2's revisions 3–8: one review, or folded into S7.5 (OQ-D).
- **A4.** The checkpoint carries this plan's list.

### Phase B: measurements and Windows evidence (evidence; small product changes only from S5.4)

- **B1. S5.1 completion:** push then reads and large packs on macOS ARM64 and Linux x86-64; the Linux x86-64 no-op fetch rows.
- **B2. S5.2:** the coalescing comparison.
- **B3. S5.3:** sustained memory.
- **B4. WPP 6.1:** TR8.4 speed on dabeest against 1.0.17 (about a day of host time, host otherwise idle; OQ12 and the operator's go for live fetch).
- **B5. WPP 6.2:** the Windows parity rows.
- **B6. Pageant 0.83 console rows** with the operator.
- **B7. S5.4:** choose the defaults from B1–B4 (after them).
- **B8. WPP 6.3:** S5.5's repeat on dabeest and the Windows column of S5.6 (after B7, B5). Phase review.
- **B9. S5.6:** the full cell table (after B1–B8).

B1–B3 run on the Mac and the x86-64 host while B4–B6 hold dabeest.

### Phase C: activation (product code)

- **C1. S7.1a:** the candidate manifest's dependencies, `windows-sys` features and TLS target tables into gwz-core's `Cargo.toml`, still behind the switch (no behaviour change; the candidate and ordinary shapes build the same dependency set).
- **C2. S7.1b:** the protocol pair: name the candidate fields 1.1.0 needs, one generated module, gwz-py's protocol regenerated.
- **C3. S7.1c:** remove the switch from gwz-core's sites (the `cfg_if` arms stay; the native branch stays for unsupported paths and the off switch).
- **C4. S7.1d:** the same in gwz-cli and gwz-py; `check-cfg` declarations; `prepare.py` retired or inverted; CI's candidate leg becomes the ordinary job (the session leg stays, `gwz_session_candidate` is 1.2.0).
- **C5. S7.2:** help, the route ledger, migration notes (after C4 and B9).

C1 and C2 can run in parallel with Phase B; C3 and C4 wait on S7.1's gates (S5.5, S5.6, S3.3, TR2.6).

### Phase D: release

- **D1. S7.3 and WPP 6.4:** clean consumer builds on the three platforms.
- **D2. S7.4:** observation recheck.
- **D3. S7.5:** activation and release review, dual plus Surface.
- **D4. WPP 5.5:** gwz-sspi activation review (dual).
- **D5. WPP 6.5 and Phase 8:** the Windows matrix on the release commit, tags, publish jobs, the post-release check on each host; release-gate dual review.

## 4. Open questions for the operator

- **OQ-A. Linux x86-64 evidence.** There is no x86-64 Linux host. (a) GitHub-hosted runners (x86-64, noisy for timing); (b) a rented or borrowed x86-64 box; (c) amend the plan to accept the Pi as the Linux evidence platform for 1.1.0. Recommendation: (b) for speed and memory rows, (a) for functional rows.
- **OQ-B. S5.2 and S5.3.** Run as written (about a day each), or amend scope. Recommendation: run them; S5.4's defaults rest on them.
- **OQ-C. A refused signature on Windows** (`PUBLICKEY_UNVERIFIED`, about 1 in 6,000 dials against `sshd.exe`, run `2026-10-14-idleloss`): D14 retries only a failed key exchange. Recommendation: leave it for 1.1.0 and record it; measure 1.0.17 on the same rows to confirm parity.
- **OQ-D. Amendment 2's revisions 3–8.** One dual review before S7.5, or folded into S7.5. Recommendation: folded into S7.5.
- **Open from before:** UNC `PATH` entries for the helper `git.exe` lookup; yank gwz-sspi `0.0.0-bootstrap.1`; the required CI checks; optionally "max(TLS 1.2, policy)" for the Linux TLS floor.

## 5. Contradictions found by the audit (to be fixed in the documents they live in)

1. The 1.1.0 plan's "gh-only HTTPS" text (V110:47-48, 312-313; design §11) conflicts with A2's configured helpers (A2:97, §3.19). A2 controls; the older text is unedited.
2. The checkpoint (2026-10-09) called TR8.1 moot; the 2026-10-13 rerun found the Pi HTTPS miss and OD17 was taken. A2:266 wants an S5.4 retune before OD17; none is recorded.
3. A2:278 expects each repository's switch inventory to list the 1.2.0-gating sites before S7.1; all three list none, while the placement projection sits under `gwz_transport_candidate` (`src/protocol/mod.rs:5-9`).
4. V110:49 supports Linux x86-64; all Linux speed evidence is aarch64.
