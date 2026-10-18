# GWZ 1.1.0 S5.6: the Design section 11 sign-off cell table

Status: **DRAFT, 2026-10-17 (lane `s56`).** It is a table of what the record shows, not a verdict. It decides nothing, accepts no amendment, and authorizes no run, commit, tag or publish. Sections 2 and 3 list the cells that need the operator. Columns marked PENDING wait on work that is running today (WPP 6.3, D30, D31, D32, D33).

Controlling texts: `GwzV110Plan.md` S5.6 (:263-274), `GwzV110RemainingPlan.md` (S5.6 row, B8, B9), `GwzTransportReleasePlanAmendment-2.md` section 3.10 (line 406's sign-off for 1.1.0: "TR2.8's agent key types and signature algorithms, TR1.6's credential helpers, and a cell for each behaviour TR1.8 designs. The server and reuse cells are 1.2.0's"), sections 3.2, 3.11 and 3.12, and decisions D1 to D33 in `GwzTransportTR18-OperatorDecisions.md`. The cells are the sixteen rows of `GwzRemoteTransportDesign.md` section 11 ("Acceptance matrix", :870-891), split where the evidence differs, plus the A2 additions and the TR1.8 behaviours (section 6).

Rule being applied (S5.6): every cell on each of the three 1.1.0 platforms holds an evidence ID or an unsupported mark. An unsupported mark on a cell the release's section 2 puts "in this release" fails S5.6 and blocks S7.5 and Phase 8 unless an accepted amendment first removes it. Linux ARM64, macOS x86-64 and exact-agent-until-proven may be unsupported without an amendment. "Python" has no cells (`GwzV110PlanAmendment.md:122`: S5.6's matrix has no Python rows).

## 1. How to read it

Platforms: **M** = macOS ARM64. **L** = Linux x86-64. **W** = Windows x86-64 (dabeest).

**On L (the Linux question).** V110:49 lists Linux x86-64 as supported and the Pi 5 (aarch64) is Linux ARM64, which is unsupported. D20 makes WSL 2 Ubuntu on dabeest (x86_64) the measurement host and the hosted `ubuntu-24.04` runner the CI host. Every L cell below cites x86-64 evidence only: the hosted-runner CI jobs, the WSL runs (`2026-10-15-s51`, `-s52`, `-s53`, `-r19stall`, `2026-09-22-q6-a`). The Pi runs (`2026-10-06-tr8-1-linux`, `2026-10-13-tr8-1-rerun` Pi rows, `2026-10-14-https-fixed-cost-fix*`, `2026-10-13-https-fixed-cost-pi`, `2026-10-08-ca-trust-parity` Linux rows, `2026-10-10-tr18-*` Linux rows) are aarch64 and are **not** cited as L evidence. Where the Pi was the only Linux number, the L cell says so. L speed evidence exists for no-op fetch, push, large packs, coalescing and memory (WSL, S5.1 to S5.3); the L functional evidence is the hosted CI job `Candidate build suites`.

Cell states (the first token of every platform cell):

| Code | Meaning |
|---|---|
| `E` | Evidenced. The ID names a run in `gwz-core-evidence/campaigns/transport-qualification/runs/` or a named test and the CI job that runs it. The README verdict was read. A measurement that missed its target is still `E`; the miss is in the note. |
| `ES` | Evidenced on an older tree or a smaller suite than today's. Counts as evidenced; a re-run is recommended (section 6). |
| `EP` | Evidenced today, and a pending step re-measures or re-confirms it. The step is named. |
| `P` | No evidence yet. Waits on the named running step. |
| `U` | Unsupported or out of the 1.1.0 scope, citing the decision or text that removes it. |
| `N` | The plan gives this platform no cell (a Windows-only behaviour, or a row the plan asks of one platform). |
| `G` | **Gap: neither evidenced nor ruled unsupported.** These block S7.5 and Phase 8 unless the operator rules or an amendment removes the cell. |

## 2. Counts and the blockers

82 cells (rows) times 3 platforms = 246 platform cells. `N` cells have no plan text asking for them, so 201 cells are in play.

| State | M (macOS ARM64) | L (Linux x86-64) | W (Windows x86-64) | Total |
|---|---|---|---|---|
| `E` evidenced | 49 | 49 | 46 | 144 |
| `ES` evidenced, older tree (re-run recommended) | 1 | 0 | 10 | 11 |
| `EP` evidenced, a pending step re-measures it | 1 | 1 | 5 | 7 |
| `P` PENDING, no evidence yet | 1 | 1 | 9 | 11 |
| `U` unsupported or out of the 1.1.0 scope, with its authority | 5 | 5 | 6 | 16 |
| `N` no cell on this platform | 21 | 22 | 2 | 45 |
| **`G` gap: neither evidenced nor ruled unsupported** | **4** | **4** | **4** | **12** |

**The `G` cells are 12 platform cells in 6 rows, and they are the S7.5 and Phase 8 blockers** unless the operator rules or an accepted amendment removes the cell (questions 3.1 to 3.5, 3.8 and 3.9 below; 3.6 and 3.7 are confirmations). The `P` cells (11) and `EP` cells (7) become blockers only if the named step does not deliver. By pending step:

| Step | `P` (no evidence yet) | `EP` (evidence exists, step re-measures) |
|---|---|---|
| WPP 6.3 (S5.5 repeat on dabeest) | W: 11.2, 11.3, 17.7, 17.8, 17.9, 17.10 | W: 17.1, 17.2, 17.3 |
| D30 (lane `cap72`) | W: 9.7 | |
| D49 (lane `win92`) | W: 9.2 | |
| D31 (gwz-sspi 0.1.2, lane `sspi012`) | | W: 18.5 |
| D32 (lane `d14two`) | | W: 17.3, 18.16 |
| D33 (HTTPS runtime, re-time on all three) | | M, L, W: 17.2 |
| lane `blobpush` | M, L, W: 3.3 | |

A cell that waits on two steps appears under both (17.2 and 17.3 do).

**Which Linux cells have x86-64 evidence.** All 82 L cells were assigned from x86-64 sources only. L is `E` on 49 cells. Functional cells rest on CI-L (hosted `ubuntu-24.04`, green at `965d884a`) and CI-T; speed, push, large-pack, coalescing and memory cells rest on the WSL runs S51, S52, S53 and R19S. L has **no** x86-64 evidence for the four `G` rows 10.2, 10.7, 12.1b and 17.11 (9.4 is `U` by D41), and the Pi cannot stand in (it is aarch64): where the only Linux number was the Pi's (HTTPS speed after the fixed-cost fix, CA-trust parity case rows, the 1.0.17 baselines RP and B17), the L cell cites CI-L or S51 instead. `D20` limit: WSL shares dabeest's CPU, so L timings are paired and booked, but are a VM's.

## 3. Questions for the operator (from the gaps and the near-gaps)

Each item is a cell the record cannot sign. Items 3.1 to 3.5, 3.8 and 3.9 are `G` cells; 3.6 and 3.7 are confirmations the table needs.

3.1 **Authenticated HTTPS against a real account (row 10.2; M, L, W).** V110 S5.6 says gh-authenticated HTTPS is advertised "only after a real-account `gh` operation. A fixture is not that proof"; A2 widened gh-only to the user's configured helpers (TR1.6) and kept the cell in the 1.1.0 column. No archived run authenticates over HTTPS against a real service: S33's HTTPS rows are anonymous and the alpha, HH and PR runs use dummy fixture credentials. Release plan RP:67 requires the operator's go for any live-account run, and D28 sent no key or agent to dabeest. Options: (a) one authenticated fetch per platform with a configured helper or `gh` against a real repository, and a decision on how a credential reaches WSL and dabeest; (b) an amendment narrowing the rule to what TR1.6 now claims.

3.2 **Environment-proxy parity with 1.0.17 on macOS and Linux (row 10.7; M, L).** The transport honours `HTTPS_PROXY`/`NO_PROXY` through a CONNECT tunnel and its own tests pass. Every 1.0.17 proxy baseline in the archive is a Windows run (RP, TD10, R13), and A2 S7.2 requires the notes to state the proxy policy "against 1.0.17's on every platform". With no 1.0.17 environment-proxy row on a Unix host the notes cannot state it. Options: one 1.0.17 against candidate row through a loopback CONNECT proxy on the Mac and in WSL; or mark Unix environment proxies unsupported in 1.1.0 (S5.6 allows "or mark them unsupported" only with an amendment if the cell is in the release column).

3.3 **Control latency (row 17.11; M, L, W).** Design section 11's performance row lists "control latency", and no plan step (S5.1 to S5.3, S5.5) or run measures it. Only the property that control messages progress under bulk traffic is tested (row 3.1). Options: add one measurement to the S5.3 harness (time to a cancel taking effect under a saturated stream) on M and L and the 6.3 booking on W; or amend the row to the contract test.

3.4 **Windows agent security keys, certificates and DSA end to end (rows 9.2, 9.3; W).** `ssh_tests/key_types.rs` runs the security-key, certificate and DSA rows on `cfg(unix)` only, and its Windows arm says they stay with the Unix rows; the Windows pipe-agent fixture (`key_fixture_pipe`) signs RSA, ECDSA and Ed25519 only. P38 expected Windows twins "when 3.4 lands". The type-listing logic (`agent_keys.rs`) does run on Windows. 1.0.17 on Windows (WSSH baseline): an Ed25519 certificate from an owned pipe agent authenticates, an RSA certificate does not, a declining security key is skipped. Options: extend the pipe-agent fixture and add the Windows twins before 6.3; or rule these types unsupported on Windows as parity with 1.0.17's list, which A2 says the transport must at least match.

3.5 **Claims the design leaves "not qualified" (rows 9.4, 18.19).** The manual hardware security-key row (A2 line 138; "needs the operator's go"), Kerberos (L1) and Pageant-held or native-agent certificates (L3) are neither evidenced nor declared unsupported (TD8 B kept them "not qualified"). WPP OQ10 (a) wanted Kerberos removed from the "in this release" column by amendment; no filed amendment text names it. S5.6 treats an unsupported in-scope cell as a blocker without that amendment. Options: run the hardware row; or one short amendment removing these three from the 1.1.0 column. **Ruled 2026-10-18 (D41): the amendment, `GwzV110AmendmentS56Scope.md`.**

3.6 **OD17 on x86-64 (rows 17.1, 17.2).** OD17 ("ship 1.1.0 and state the Linux HTTPS gap") was taken on Pi data. S51 on Linux x86-64 shows the same shape: five of the six no-op rows miss by the TR8.1 rule (SSH by 0.003, 0.033, 0.005 s at 32, 16, 1; HTTPS by 0.017 and 0.009 s at 32 and 16, met at 1), and D33 will re-time HTTPS. Confirm that OD17 and D29's reasoning cover x86-64 SSH and HTTPS, or say otherwise.

3.7 **The capability answer (row 9.7).** At `965d884a` the static handler (`src/protocol/transport_capabilities.rs:15`) answers `file_identity: true` and `exact_agent_identity: false` on every platform, and `TransportRuntime::capabilities` (session-only) answers both `!cfg!(windows)`. `GwzV110RemainingPlan.md` C5 says the handler answers both true on Windows. The table follows the code. D30 (Windows GO, subject to the exact-selection row) is `P`; macOS and Linux are `U` under S5.6's "exact-agent-until-proven". Should D30's reasoning extend to macOS and Linux, they need a named fixture row each.

3.8 **The candidate against the real running OpenSSH Authentication Agent service (row 18.4; W).** TD12 ran the real service (elevated, manual start type restored) on 1.0.17 only; the candidate's pipe rows (PR, SA) use an owned pipe agent or a stopped service. Options: one more TD12-style booking with the candidate, or a ruling that L4 ("beyond the cells run") stands as a recorded limitation.

3.9 **The binding package on macOS and Linux (row 12.1b).** `tests/transport_native` (foreign registered transport coexists; lossless native errors) has no CI job. Its only archived run is Windows (Q6A, 9 tests, 2026-09-22); the macOS claim is a sentence in `GwzRemoteTransportNativeBinding.md:95`. One `cargo test` per host closes it. It needs no decision beyond permission to run on WSL and dabeest.

Also for the operator, not `G` cells:

- **What S5.5's "applicable rows" include on Windows.** Rows 17.9 (coalescing) and 17.10 (memory) are `P: 6.3`. S5.5 says a row the host cannot run is recorded unsupported, not passed. Say whether 6.3 runs them or marks them unsupported.
- **Windows large-pack HTTPS (rows 11.2, 17.8).** The one unported Windows parity row (`throughput_tests::an_https_clone_of_a_large_pack_keeps_pace_with_libgit2`) timed out on dabeest on 2026-10-11 against a native baseline that cloned; no run has re-tried it. D33's runtime change may bear on it. Treat it as a risk for 6.3, not only a missing row.
- **CI reds today.** Transport candidate failed once on the Windows job (`272ad3d7`, library suites) and once on the Linux session leg (`4f4b7e01`); each was green at the next commit. The reasons were not read. If the cause is a product flake, the CI-based cells are weaker than they look.
- **Lane `blobpush` platforms.** Row 3.3 is `P` on all three; confirm which platforms the lane runs.

## 4. Evidence key

### 4.1 CI jobs

macOS has **no** CI job that builds the transport. `platform-matrix.yml` (macos-14, ubuntu-24.04-arm) and `windows-matrix.yml` build the ordinary shape, which carries no transport. They are not cited. CI status below is from `gh run list` on 2026-10-17 (read-only).

| ID | Job | Platform | What it runs | Status read |
|---|---|---|---|---|
| CI-T | gwz-transport `Transport contracts` / `owner-contracts` (`contracts.yml`) | ubuntu-24.04, x86-64 | generator check, rustfmt 1.96.0, `cargo +1.95.0 test --locked` and with `--features unstable-sequenced`, `cargo package` | green at `a27817b9` (2026-10-12), the pin |
| CI-L | gwz-core `Transport candidate` / `Candidate build suites` (`transport-candidate.yml`), two legs (`transport`; `transport-and-session`) | ubuntu-24.04, x86-64 | `scripts/run_tests.py` candidate library suites, the integration targets (`protocol`, `transport_placement_guide`, `native_libgit2`, `rename`, `diff_render_spike`, `publish_workflow`), generator checks, the consumer archive proof, the source checks | green at `965d884a` (2026-10-17 12:07Z). Red at `4f4b7e01` (the session leg) and green at the next commit; reason not read |
| CI-W | gwz-core `Transport candidate` / `Candidate build on Windows` | windows-2022 | the library checks in both shapes, the host-level `qualification_tests::` rows (7 on Windows), the candidate library suites with a portable OpenSSH server, the integration targets | green at `965d884a`. Red once at `272ad3d7` ("Run the candidate build's library suites"), green at the next commit; reason not read |
| CI-S | gwz-sspi `Standalone SSPI package`, plus `Opt-in production Windows fixtures` | ubuntu-24.04, macos-14, windows-2025 | the standalone crate's suites, codec, supervision, worker checks; native fixtures on Windows | green at `75c1b032` |
| CI-P | gwz-py `Transport candidate` / `Candidate suite` (two legs) | ubuntu-24.04 | the whole gwz-py suite on the candidate extension | green at `8e8446ba` |
| CI-C | gwz-cli `CI` / `Build and test on Windows` | windows-2022 | gwz-cli build and tests with the siblings beside it | green at `279bf85f` |

### 4.2 Gate evidence where no CI job exists

| ID | What | Read |
|---|---|---|
| GM | macOS ARM64 local gates, the lane owner's: ordinary suite, candidate leg 1 (`--cfg gwz_transport_candidate`) and leg 2 (plus `gwz_session_candidate`), archived as logs. Latest: `2026-10-16-spawngate` `addendum/mac-ordinary.log` 2,500 passed, `mac-leg1.log` and `mac-leg2.log` 3,953 passed each, 0 failed. Earlier equivalents in `2026-10-14-https-fixed-cost-fix-merged` (3,865), `2026-10-16-sspi011` (3,933), `2026-10-16-c3a` | read from the README lines |
| GW | dabeest Windows receipts, `scripts/windows_lane_check.py --worktree --tests --hosted-like`. Latest: `spawngate-20261016-d` (`2026-10-16-spawngate/windows/`) 3,970 passed, 7 failed, all 7 named host gaps (`gwz-py` and `taut-proto` absent on the host); ordinary and transport shapes pass. Hosted CI installs both, so those 7 run in CI-W | read from the README line |

### 4.3 Archived runs (`runs/<id>`), with the verdict read

Tags are used in the tables. "1.0.17 baseline" means the run measured released gwz 1.0.17 only and is the reference the candidate rows compare to, not candidate evidence.

| Tag | Run | Platforms | Verdict as read |
|---|---|---|---|
| Q6A | `2026-09-22-q6-a` | M, L (WSL), W | gwz-transport suites native on all three: 140 passed, 2 ignored each (the crate has 237 passing today). Integrated candidate perf fixtures on M and L only; Windows integrated adapter was unimplemented. Oldest tree in the table |
| HC | `2026-10-12-helpercause` | M (W via receipt) | gwz-transport on the Mac: 237 passed, 255 with `unstable-sequenced`; Mac ordinary 2,446; candidate legs 3,726 each; helper wire causes D13 |
| FCM | `2026-10-14-https-fixed-cost-fix-merged`, `-fix`, `-fix-rem1` | M, W receipts; Pi (aarch64, not cited for L) | gates green on the merged tree: Mac ordinary 2,477, legs 3,865 or 3,866, Windows receipts `perf-20261014-c` and `-f` PASS (7 host gaps). Round-1 fixes closed |
| RR | `2026-10-13-tr8-1-rerun` | M; Pi rows not cited | TR8.1 no-op fetch, candidate against 1.0.17: Mac SSH met at 32, 16, 1 (-0.181, -0.215, -0.231 s); Mac HTTPS met (-0.034, -0.033, -0.041 s) |
| MP4 | `2026-10-04-tr8-1-macos-parity` | M | SSH no-op fetch against GitHub, native-route row (off switch) at 16 and 32 members: 0 failed members, 3 rounds. Older tree. Candidate then missed SSH by 0.22 s (superseded by RR) |
| S51 | `2026-10-15-s51` | M, L (WSL) | S5.1 completed: Linux x86-64 no-op rows (SSH misses by 0.003/0.033/0.005 s at 32/16/1; HTTPS by 0.017 and 0.009 s, met at 1), push plus post-push reads (candidate 16 connections against 32; +6% to +39% per command on L, parity on M), large packs (L: SSH clone +5%, HTTPS clone +17%, SSH push +6%, HTTPS push +3%; M parity), 1,964 Git-result comparisons, 0 mismatches. The Mac was loaded (indicative); WSL quiet |
| S52 | `2026-10-15-s52` | M, L (WSL) | S5.2 coalescing: the client end's window (0, 10, 50, 100 ms) makes no measurable difference; the SSH endpoint end at 100 ms costs +0.10 s per exchange (the defect `db0f8447` removed; the product default 0 is kept); connect counts equal in all nine configurations |
| S53 | `2026-10-15-s53` | M, L (WSL) | S5.3: client-process RSS flat (L 29.0 to 32.5 MiB, M 22.5 to 29.0 MiB) under slow readers and flat-out writers up to 1.6 GiB; one exception (reader pause longer than the stall limit) was investigated in R19S |
| R19S | `2026-10-16-r19stall` | M, L, W | the exception is a defect in the SSH pump (consumer backpressure and post-pack local work charged to the network stall clock), reproducible on all three; fixed (gwz-core `d9d6ad47`, `d94d3816`) with closure rows; State review GO |
| S33 | `2026-10-17-s33` | M | S3.3: all criteria pass: production-graph stall rows with a red negative control, 5 of 5 cold SSH fetches and 5 of 5 default fetches of 16 members against GitHub at default timeouts (the operator's agent, authorised), HTTPS anonymous rows pass |
| T84 | `2026-10-16-tr84speed` | W | WPP 6.1: every speed row misses the TR8.1 rule (SSH +0.008 s at 32, +0.015 at 16, +0.032 at 1; HTTPS +0.165, +0.074, +0.018 s); no partial result on the transport route at 16 and 32, SSH and HTTPS, 10 of 10 attempts each, where 1.0.17 failed 5 of 10 (SSH 32); connection counts equal 1.0.17's. D29 accepts SSH, HTTPS awaits D33 |
| H611 | `2026-10-17-https611` | W | cause of the HTTPS gap confirmed: certificate-chain checks serialized on the one `current_thread` runtime; a 4-worker scratch build closes it (+0.007/+0.018/-0.001 s). Basis of D33. No product change |
| PR | `2026-10-15-parityrows` | W | WPP 6.2: the parity rows PASS or differ as the register says; findings F1 (native route partial results, equal to 1.0.17, decided D22), F2 (NTLM at an Internet-zone name with a helper: transport authenticates, 1.0.17 does not), F3 (a Negotiate server that withholds its final token). Pageant rows were owed to the operator |
| PO | `2026-10-17-pageant-operator` | W | the operator ran real Pageant 0.83 rows: `pageant-rsa-key`, `pageant-beats-pipe`, `pageant-empty-no-fallthrough` PASS on 1.0.17 and the candidate. Binary predates 2026-10-16 merges (stall fix, spawn gate, C3a) |
| SG | `2026-10-16-spawngate` | M, W | see GM and GW; D25 spawn gate, State review GO |
| WF | `2026-10-16-winflakes` | W | the D14 residual measured over 77,058 establishes (0.75% first-try -8, 2 double -8); basis of D32 |
| IL | `2026-10-14-idleloss` | W (M test code) | the `idle_loss*` flakes were four test-support faults; dabeest 0 of 600 failures after, 9 of 120 before; no product change |
| GD | `2026-10-15-gitdaemon` | W | the failing `git://` clone is Git for Windows' daemon ending sessions with a reset; test support only |
| CAT | `2026-10-08-ca-trust-parity` | M, W; Pi (not cited for L) | CA-file trust parity case by case, rows pass on macOS and Windows; basis of the register's item 11 |
| PX | `2026-10-12-proxy` | W | steps 4.6/4.7: 33 of 33 capture rows and 16 of 16 tunnel rows against the real machine WinHTTP proxy, every restore verified |
| HH | `2026-10-12-httpshelp` | W (M gates) | steps 4.4/4.8/4.11: helper policies, real SSPI Negotiate/NTLM with channel binding through the production worker, wrong binding refused, helper test modules on Windows |
| WH3 | `2026-10-12-wh3`, `-wh3c` | W | 4.9/4.10: T5 refusal verbatim, redirects with `--verbose`, wrong channel binding/untrusted chain/wrong name send no token, installed CLI and wheel under paths with spaces and Unicode, worker provenance refused when wrong |
| SA | `2026-10-12-sshagent` | W | 3.2a/3.2b/3.3: agent-source rule and overlapped pipe channel; receipts PASS |
| SS | `2026-10-12-sshsmall` | W | 3.1 SSH home contract, 3.4 allocator proof, 3.8 differentials on Windows; a real Windows defect found and fixed |
| P38 | `2026-10-11-p38srv` | W | Windows arms of `key_types.rs` (4 rows) and `rsa_sha1.rs` (4 rows) green on dabeest; stuck-close server on every platform |
| WP3 | `2026-10-11-winp3a` | W | 3.8 (WinCNG host-key and key-file limits with TD5's words), 3.7 (error kinds, retry), 3.5 (Pageant exchange): all pass |
| PG | `2026-10-12-pageant` | W | 3.6 over a synthetic Pageant: 118 passed, 1 failed (a known `--test-threads=1` ordering row), 1 ignored; real Pageant rows deferred to PO |
| CNG | `2026-10-13-cngretry` | W | D14: the key-exchange retry, pinned with an injected -8 |
| KW | `2026-10-14-keywords` | W, M | D17 passphrase-protected key words, every platform |
| RT51, CL52, PY53 | `2026-10-13-retire51`, `-cli52`, `-py53` | W | qualification switch retired; SSH and HTTPS operations and two overlapping operations on the Windows wheel take the transport route |
| WSSH, WAUTH, TD9, TD10, TD12, R13, RP, B17 | `2026-10-10-tr18-windows-ssh-rows`, `-windows-auth-rows`, `-td9-user-root-trust`, `-td10-machine-proxy`, `-td12-agent-service`, `2026-10-11-tr18-r13-https-proxy`, `2026-10-10-tr18-redirect-parity`, `2026-10-10-tr18-b17-macos-linux` | W; RP and B17 also M and Pi | **1.0.17 baselines** for the TR1.8 design; the TD10/R13 rows restored the machine proxy byte-identically |

### 4.4 Other IDs

`D<n>` is a decision in `GwzTransportTR18-OperatorDecisions.md`. `L<n>` is an item of `GwzTransportWindowsParityDesign.md` section 13 (limitations); `Reg<n>` is an item of its section 12 (register). Test names are given as `file::test`; a file alone means its module.

## 5. The table

### 5.1 Contract machinery (Design section 11 rows 1 to 5, gwz-transport)

gwz-transport is portable Rust. Its only CI job runs on Linux x86-64 (CI-T). It has had no macOS run since HC (2026-10-12, 237 tests; the crate's last change is `a27817b9`, 2026-10-12, a wire field that touches no stream or pool code) and no Windows run since Q6A (2026-09-22, 140 tests, before the pool's limit, settle, tag and discard-idle additions). Section 9 asks for one run per host.

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 1.1 | **Byte/message adaptation**: chunking, binary/NUL, empty writes, ordering. `tests/stream.rs`, `mux.rs::randomized_chunks_reads_and_mux_routing_reassemble_exact_bytes`, `monte_carlo.rs`, `sequenced*.rs` | E: HC | E: CI-T | ES: Q6A | |
| 1.2 | Byte/message adaptation through the product's channels: SSH pump and HTTPS bridge carry binary packs byte-exact. `transport_host/message_embedding_tests::rust_messages_embed_live_git_exchange`, `throughput_tests` (TR2.13), Git results compared with git's own | E: GM; S51 (1,964 comparisons, 0 mismatches, M and L) | E: CI-L; S51 | E: CI-W; GW; RT51 (SSH and HTTPS clone and fetch pass on dabeest) | Windows `throughput_tests::an_https_clone_of_a_large_pack_keeps_pace_with_libgit2` is the one unported parity row, see 11.2 |
| 2.1 | **Batching**: full buffer sends at once, first-byte timer, trickle cannot postpone, flush and end-write bypass. `stream.rs::batching_timer_is_from_first_byte_and_flush_bypasses_it`, `read_makes_small_pending_request_immediately_eligible`, `io_clock*.rs`, `wake_amplification.rs` | E: HC | E: CI-T | ES: Q6A | |
| 3.1 | **Flow control**, contract: paused consumers and tiny messages within byte and message budgets, both directions progress, control cannot deadlock. `stream.rs::tiny_window_applies_backpressure_until_application_reads`, `mux.rs::data_queue_capacity_preserves_control_admission`, `cancellation_control_progresses_while_other_requests_keep_bulk_queue_busy`, `stream_limits.rs`, `pool_local_wait.rs` | E: HC | E: CI-T | ES: Q6A | |
| 3.2 | Flow control in the product under a real stalled or slow consumer; consumer backpressure no longer runs the stall clock. `ssh_pump_clock_tests` (11 rows), HTTPS `Backpressure` in `https_worker/serve.rs` | E: R19S; GM | E: R19S; CI-L | E: R19S; CI-W | The S5.3 exception was this defect; fixed and re-verified on all three |
| 3.3 | Flow control: a **push whose local pack producer pauses mid-pack** is charged as Network (stall-review Follow-up 1) | P: lane `blobpush` | P: lane `blobpush` | P: lane `blobpush` (platforms to be confirmed) | Operator decision with a 1 GiB-blob push row as the trigger test |
| 4.1 | **Lifecycle**, contract: all four half-states, final offsets, bounded drain with discard reporting, racing Cancel and final-handle drop. `stream.rs::half_close_preserves_reverse_direction_and_empty_read_is_not_eof`, `graceful_close_drains_unread_response_and_waits_for_request_sink`, `close_deadline_includes_time_waiting_for_credit`, `mux.rs::finish_retains_every_terminal_until_direct_handoff_with_facts`, `async_stream.rs`, `mux_async.rs` | E: HC | E: CI-T | ES: Q6A | |
| 4.2 | Lifecycle in the product: SSH close (including a stuck close) and HTTPS close are bounded. `transport_host/close_tests.rs`, `cleanup_tests.rs`, `https_worker/close_tests.rs`, `ssh_controlled_server` | E: GM | E: CI-L | E: CI-W; P38 (stuck-close server on Windows) | |
| 5.1 | **Carrier loss**, contract: disconnect fails all owned streams, stale session messages cannot reattach. `mux.rs::unknown_session_is_ignored_but_unregistered_request_cannot_open`, `retired_request_ids_and_stream_ids_cannot_reopen_work`, `dropping_port_with_a_pending_terminal_closes_generation_and_peer_on_propagation`, `failure_detail.rs` | E: HC | E: CI-T | ES: Q6A | |
| 5.2 | Carrier loss in the product: a cut connection during open, read, write, close and while idle. `transport_host/fault_tests.rs`, `ssh_tests/idle_loss*` (cut proxy), `https_worker_tests/pool.rs` | E: GM | E: CI-L | E: CI-W; IL | IL: the Windows rows' faults were test support, product unchanged |

### 5.2 Pool, idle and shutdown (rows 6 and 7)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 6.1 | **Pool**, contract: reuse, exclusive leases, both caps, incompatible idle eviction, no starvation or count leaks. `pool.rs`, `pool_random.rs`, `pool_regressions.rs`, `pool_limit.rs`, `pool_settle.rs`, `pool_fresh.rs`, `pool_tag.rs`, `pool_discard_idle.rs` | E: HC | E: CI-T | ES: Q6A | The limit, settle, tag and discard-idle tests postdate Q6A |
| 6.2 | Pool in the product, within one command: caps respected, one connection per member, a push reuses its advertisement connection (16 connections against 32). `ssh_pool`, `https_pool`, `ssh_tests/pooled.rs`, `agent_capacity.rs` | E: GM; S51 (connect counts) | E: CI-L; S51 | E: CI-W; T84 (counts 32/16/1, equal to 1.0.17's) | Windows push counts: see 17.7 |
| 6.3 | Pool reuse across operations, a long-lived embedding or a server | U: A2 section 3.2 and 3.10 (reuse is 1.2.0's, TR8.2) | U: same | U: same | S5.4 note: in 1.1.0 no connection outlives a command |
| 7.1 | **Idle and shutdown**, contract: 60-second expiry, checkout and timer races, a busy-but-quiet lease survives, shutdown wakes waiters. `pool_async.rs`, `pool.rs`, `pool_io_clock.rs`, `io_clock.rs`, `network_timeouts.rs` | E: HC | E: CI-T | ES: Q6A | The 60-second default stays (V110:439; `GwzV110S54Defaults.md` 3.2) |
| 7.2 | Idle and shutdown in the product: idle watch, shutdown, a session lost while idle. `idle_watch`, `shutdown_watch`, `https_worker_tests/pool.rs::idle_connection_is_reaped_and_shutdown_can_be_polled_again` | E: GM | E: CI-L | E: CI-W; IL | |

### 5.3 Identity and trust (row 8)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 8.1 | **Identity/trust**: same user and host with different explicit keys cannot cross; an unavailable or changed selected key refuses; endpoint-local path bases and known_hosts; no fallback. `ssh_key_*`, `ssh_home`, `ssh_host_key`, `host_key_tests.rs`, `agent_source_tests.rs`, `ssh_destination_tests.rs` | E: GM | E: CI-L | E: CI-W; SS (SSH home contract); PR (`home-unset-absolute-identity`, `home-unset-tilde-identity`) | |
| 8.2 | "Host not listed" and "key changed" worded apart (D12), refusals name the file and the home source | E: GM | E: CI-L | E: CI-W; WP3 | |
| 8.3 | SSH key files (`--identity`): ECDSA and Ed25519 containers authenticate; a passphrase-protected file is refused with words (D17). `key_files.rs::ecdsa_and_ed25519_key_files_in_each_container_authenticate` | E: GM; KW | E: CI-L; KW | E: CI-W; KW | |
| 8.4 | Windows host keys other than RSA and key files other than unencrypted RSA PEM (WinCNG) | N | N | U: D16, TD5, L12 (interim `UnsupportedOperation` carrier with plain words; WP3 rows pass) | Same limit as 1.0.17 |

### 5.4 SSH agents and key types (A2 TR2.8; row 15's agent fixtures)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 9.1 | **Agent key types and signatures**: Ed25519; ECDSA P-256, P-384, P-521; RSA at `rsa-sha2-512`, `rsa-sha2-256` and the three `ssh-rsa` cases. `ssh_tests/key_types.rs`, `rsa_sha1.rs`, `agent_auth.rs`, `agent_keys.rs` | E: GM | E: CI-L | E: CI-W (Windows arms: 4 + 4 rows, libssh2's own agent client against the pipe agent); P38; PR (`agent-pipe-named-rsa` flag 4, `-ed25519` flag 0) | The Unix arms are `cfg(unix)`, so M and L run all rows |
| 9.2 | Security keys (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`) through a software authenticator; DSA | E: GM (`key_types.rs`, `local_endpoint.rs::security_key_and_ecdsa_agent_keys_serve_an_exchange_through_the_endpoint`) | E: CI-L (same rows) | P: lane `win92` (D49). The Windows arm of `key_types.rs` says security keys, certificates and DSA "stay with the Unix rows"; only the type-listing logic (`agent_keys.rs`) runs on Windows, no end-to-end authentication. P38 expected Windows twins "when 3.4 lands"; none found | Owner: lane `win92` (D49, 2026-10-18: Windows twins of the security-key and DSA rows through the pipe-agent fixture, with a 1.0.17 comparison where 1.0.17 can express the case). D41 does not remove this cell |
| 9.3 | OpenSSH certificates of those types against a test user CA. `key_types.rs::certificates_of_each_type_authenticate_against_a_trusted_user_ca` | E: GM | E: CI-L | G: as 9.2. 1.0.17 baseline (WSSH): an Ed25519 certificate from an owned pipe agent authenticates, an RSA certificate does not | Blocker 3.4 |
| 9.4 | Manual hardware security-key row (amendment 1 section 4, "needs the operator's go"), and a Pageant-held or native-agent certificate (L2, L3) | U: D41, `GwzV110AmendmentS56Scope.md` (a real hardware-key signature) | U: same | U: D41, `GwzV110AmendmentS56Scope.md` (a real hardware-key signature, and a certificate held by Pageant or by the Windows OpenSSH agent service) | Facts kept: TD8 B, L2, L3. Certificates from a Unix agent are row 9.3; from the owned pipe agent on Windows, row 9.3 |
| 9.5 | Agent source: one agent source per session. Unix `SSH_AUTH_SOCK`; Windows rule is in 10.x | E: GM | E: CI-L | N (rows 18.1, 18.3) | |
| 9.6 | `file_identity` capability: a selected key file is honoured | E: the same rows as 8.1, 8.3; handler `file_identity: true` | E: same | E: D30 GO on PR `home-unset-absolute-identity`, `home-unset-tilde-identity` | |
| 9.7 | `exact_agent_identity` capability (exact-agent-until-proven) | U: V110 S5.6 allows unsupported; the 1.1.0 handler answers false (`src/protocol/transport_capabilities.rs:15`) and no decision turns it on for macOS | U: same | P: D30 (GO subject to one Windows exact-selection row; lane `cap72`); PO is the Pageant evidence | Tree reading conflicts with the C5 note, see 3.7 |

### 5.5 HTTPS (row 9)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 10.0 | **HTTPS anonymous access** (live, GitHub public) | E: S33 (HTTPS rows); RR (met at 1, 16, 32) | E: S51 (WSL, github.com) | E: T84 (24 rounds per row, 0 failed members) | |
| 10.1 | HTTPS authenticated through the user's **configured credential helpers**, `gh` and others (A2 TR1.6; replaces "other helpers rejected"): `git credential fill` asked only to answer a discovery 401, once per route. `https_worker_tests/configured_helpers.rs`, `https_worker/credential_tests.rs`, `https_auth_integration_tests`, `https_helper_projection_tests.rs` | E: GM; HC | E: CI-L | E: CI-W; HH; PR (`helper-basic-only`) | Fixture helpers |
| 10.2 | HTTPS authenticated against a **real account** (V110 S5.6: advertised "only after a real-account `gh` operation. A fixture is not that proof") | G | G | G | No run: S33's HTTPS rows are anonymous, the alpha and HH runs use dummy fixture credentials. A live account needs the operator's go (RP:67). Blocker 3.1 |
| 10.3 | Login failure: bad credentials, no helper, a helper that declines, a refused configuration (D13 causes, texts T4, T8, T9) | E: GM; HC | E: CI-L | E: CI-W; PR (`basic-only-no-helper`); HH | |
| 10.4 | Redirect-origin isolation: 301/302/307/308 on discovery followed, credentials never cross origins, https to http refused (T5). `https_worker/redirect_tests.rs`, `https_redirect_tests.rs` | E: GM | E: CI-L | E: WH3 (installed CLI: T5 verbatim, the redirecting server saw no credential); CI-W | 1.0.17 baseline RP: macOS and Linux follow 8 of 8 (HTTP), Windows 0 of 8 (Reg4) |
| 10.5 | Streaming POST: push over HTTPS, a rejected POST is not replayed. `https_worker_tests/failure.rs::early_post_rejection_wakes_blocked_writer_without_replay`; the 128 MiB push | E: S51 (candidate only; 1.0.17 cannot trust a fixture CA on macOS) | E: S51 (+3%) | E: PR (POST 200 under NTLM/Negotiate, chunked body); PY53 (HTTPS operation on the wheel); WH3 | Windows large-push speed: see 11.2 and 17.8 |
| 10.6 | **TLS parity**: system trust, a CA file from `GIT_SSL_CAINFO` (and `SSL_CERT_FILE` on OpenSSL), hostname check, TLS 1.2 floor, a malformed CA file refused before any connection. `ca_trust_tests.rs`, `ca_trust_clone_tests.rs`, `ca_bundle_tests.rs`, `tls_share_tests.rs`, `endpoint_environment_tests.rs`, `https_backend::openssl_stack` | E: GM (the three macOS CA-bundle tests); CAT | E: CI-L (the OpenSSL-only trust rows run on Linux only; they are `platform` rows of the Windows parity inventory, step 0.5) | E: CI-W; CAT; TD9 (1.0.17 HTTPS rows under an attended one-certificate user Root add) | Reg11 and D4: the transport adds roots beside the system's. L5: the macOS trusted-HTTPS row of 1.0.17 was unexecuted (TD7 B) |
| 10.7 | **Proxy parity**: macOS and Linux. The transport honours environment proxies with `NO_PROXY` (CONNECT, proxy credentials never in origin requests, unsupported forms refused before any open). `https_worker_tests/proxy.rs::connect_proxy_and_no_proxy_keep_proxy_credentials_out_of_origin_requests`, `endpoint_environment_tests.rs` | G: tests of the candidate exist and run (CI-L, GM), but no 1.0.17 environment-proxy row on macOS or Linux was found in the 99 runs (every proxy run is Windows). A2 S7.2 requires the notes to state the proxy policy "against 1.0.17's on every platform" | G: same | N | Blocker 3.2 |
| 10.8 | Proxy parity, Windows: the WinHTTP machine proxy (bare, `https=`, `http=`-only refused, `socks=`-only refused, grammar, bypass list, 407 refused, env proxies ignored beside it) | N | N | E: PX (33 + 16 rows, real WinHTTP, restored verbatim); PR proxy rows; CI-W (`machine_proxy/*_tests`); R13/TD10 are the 1.0.17 baselines | D1, D6, D7, D8 |
| 10.9 | Bounded cleanup of HTTPS requests: cancel, close, shutdown. `https_worker/close_tests.rs`, `cleanup_tests.rs`, `cancellable_https_tests.rs`, `https_cancel_mux_tests.rs::native_publication_*` | E: GM | E: CI-L | E: CI-W | D33 changes the endpoint's runtime; these rows rerun in CI on the D33 commit |

### 5.6 Git semantics, context isolation, compatibility, URL routing, ingress bounds (rows 10 to 14)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 11.1 | **Git semantics**: discovery-only reads, stateful SSH negotiation, multi-round HTTP fetch, no-op and updating fetch of 16 members at default timeouts | E: S33 (SSH, 5 of 5 cold and 5 of 5 default fetches of 16 members against GitHub); RR | E: S51 (WSL); CI-L | E: T84; RT51; CL52; PY53 | |
| 11.2 | Large clone (400 MiB) | E: S51 (Mac loaded: parity, SSH -4%) | E: S51 (SSH +5%, HTTPS +17% slower than 1.0.17, 5 of 5 rounds) | P: 6.3 | Windows HTTPS large pack timed out on dabeest on 2026-10-11 and its throughput row is the one unported parity row (`windows_parity/6.1.json`: "a speed finding for TR8.4"); T84 measured no-op fetches only |
| 11.3 | Large push (128 MiB), post-push reads, 16-member push | E: S51 (parity; candidate 16 connections against 32) | E: S51 (+9 to +34 ms per command; SSH push +6%, HTTPS +3%) | P: 6.3 | |
| 11.4 | Rejection and uncertain push; publication ordering. `https_worker_tests/failure.rs`, `https_cancel_mux_tests.rs::native_publication_*`, `retry_tests.rs` (the no-replay classification) | E: GM | E: CI-L | E: CI-W | Read by test name only |
| 12.1 | **Context isolation**, product level: per-remote callback lifetime, error and unwind; nested and concurrent routes, through the fork's `smart_transport` binding on the real route. `transport_host/driver_tests.rs`, `cancellation_tests.rs`, `https_route_scale_tests.rs`, `tests/native_libgit2.rs` | E: GM | E: CI-L | E: CI-W | |
| 12.1b | Context isolation, the binding package: a pre-existing foreign registered libgit2 transport coexists before, during and after; native `file` transport works throughout; lossless native error classes. `tests/transport_native/tests/{smart_transport,registry_coexistence}.rs` (an unpublished package; no CI job runs it) | G: only `GwzRemoteTransportNativeBinding.md:95` (2026-09-20, "seven integration tests" on this host), not an archived run | G: no run | ES: Q6A (9 tests, 2026-09-22) | git2-rs has moved since. Blocker 3.9 |
| 12.2 | No process-global registry write; no globals or thread-locals. `scripts/checks/check_process_globals.py` in `run_tests.py`'s preamble and in `run_boundary_job.py` | E: GM | E: CI-L | E: CI-W | |
| 13.1 | **Compatibility/reporting**: Bind/Bound version and scheme intersections, minimum limits, stale sessions, absent binding and an unsupported route with zero socket or helper calls. gwz-transport `binding.rs`, `protocol.rs`, `placement_v2.rs`, `https_selector.rs`; gwz-core `tests/protocol.rs`, `qualification_tests::` | E: HC; GM | E: CI-T; CI-L | ES: Q6A for gwz-transport; E: CI-W (`qualification_tests::`, `tests/protocol.rs`) | |
| 13.2 | Old local requests unchanged; the 1.1.0 additions (`OperationPolicy.max_retries`, four receipts) are `MISSING_OK`; merge refuses `max_retries` | E: GM | E: CI-L | E: CI-W | C2 dual review GO |
| 13.3 | Offered, authenticated and reused facts reported; private-member suppression. `ssh_helper_projection_tests`, `https_helper_projection_tests.rs`, `command_tests.rs` | E: GM | E: CI-L | E: CI-W | Two 4.11 refusal tests are `platform` rows (D10) |
| 14.1 | **URL routing**: SCP and every SSH scheme alias, canonical pool keys, local HTTP and git retained, unsupported nonlocal schemes refused before effects. `src/git/url_scheme.rs`, `ssh_destination_tests.rs`, `https_destination`, `transport_candidate_tests/url_parity.rs` | E: GM | E: CI-L | E: CI-W; SS (`url_parity`, `known_hosts_case` on Windows); GD | |
| 14.2 | URL credentials: userinfo, query and fragment sentinel refusals, including redirects, no secret in messages | E: GM | E: CI-L | E: CI-W | |
| 15.1 | **Ingress bounds**: oversized Data or metadata, a huge declared truncated frame, deep or large unknown collections, a tiny-message storm; storage and nesting bounded; typed failure wakes waiters and releases leases. `tests/protocol.rs`, `failure_detail.rs`, `stream_limits.rs`, `src/cbor.rs`, `src/codec/preflight.rs`, `wake_amplification.rs` | E: HC | E: CI-T | ES: Q6A | |

### 5.7 Platforms (row 15) and performance (row 16)

| ID | Cell | M | L | W | Note |
|---|---|---|---|---|---|
| 16.1 | **Platforms**: trust fixtures | E: GM; CAT | E: CI-L | E: CI-W; CAT; TD9 | |
| 16.2 | Platforms: agent fixtures (Unix agent; Windows pipe agent; real Pageant 0.83) | E: GM | E: CI-L | E: CI-W; PO; PR | The real running OpenSSH Authentication Agent service is row 18.4 |
| 17.1 | **Performance**: SSH no-op fetch of 32, 16, 1 members, candidate against 1.0.17 at `--max-per-host 32` (TR8.1) | E: RR (met at all three) | E: S51 (WSL): missed by 0.003, 0.033, 0.005 s; connections equal 1.0.17's | EP: 6.3. T84: missed by 0.008, 0.015, 0.032 s (a fixed 20 to 30 ms per command); D29 accepts it as parity | The x86-64 rows are not covered by OD17, taken on Pi data, see 3.6 |
| 17.2 | HTTPS no-op fetch of 32, 16, 1 members (TR8.1, OD17) | EP: D33. RR: met at all three | EP: D33. S51 (WSL): missed by 0.017 s at 32, 0.009 at 16, met at 1 | EP: D33 and 6.3. T84: missed by 0.165, 0.074, 0.018 s | D33 re-times all three. Pi (aarch64, not cited) missed by 0.06 s at 32 after the fixed-cost fix |
| 17.3 | No partial result in three rounds at 16 and 32 members (transport route, D22) | E: RR; S33; S51 (0 bad runs) | E: S51 (0 bad runs) | EP: D32 and 6.3. T84: 10 of 10 clean attempts each, SSH and HTTPS; 1.0.17 failed 5 of 10 at 32 | |
| 17.4 | Native route with the off switch: 16 and 32 members, three rounds, no partial result (A2 section 3.10; macOS) | ES: MP4 (0 failed, 3 rounds) | N: A2 asks macOS | E: PR `native-route-16-members`/`-32-members` and the 1,280-fetch rate row; F1: not clean (3 of 5 attempts), equal to 1.0.17's 3 of 5 and 1 of 5; D22 records parity | |
| 17.5 | Connection and channel counts recorded | E: S51; RR | E: S51 | E: T84 (fetch counts); push counts: see 17.7 | |
| 17.6 | Cold against warm SSH; reuse across commands | U: A2 section 3.10 (TR8.2 is 1.2.0's) | U | U | Within one command the push reuse is in 6.2 |
| 17.7 | Push plus post-push reads (aggregate operations, S5.1) | E: S51 | E: S51 | P: 6.3 | |
| 17.8 | Large-pack throughput | E: S51 | E: S51; CI-L (`throughput_tests`) | P: 6.3 (HTTPS row unported, see 11.2) | |
| 17.9 | Coalescing windows including 100 ms, connect counts and wall time (S5.2) | E: S52 | E: S52 (WSL) | P: 6.3, if S5.5's "applicable rows" include it | Whether the host can run it needs a ruling (S5.5: an unrunnable row is recorded unsupported, not passed) |
| 17.10 | Memory under sustained backpressure, the client's own RSS (S5.3) | E: S53; R19S | E: S53; R19S | P: 6.3, same ruling as 17.9. R19S has Windows stall rows only | |
| 17.11 | **Control latency** under backpressure | G: only the progress property is tested (`mux.rs::cancellation_control_progresses_while_other_requests_keep_bulk_queue_busy`, rows 3.1); no step S5.1 to S5.3 measures a latency | G | G | `control latency` appears in no plan step or run. Blocker 3.3 |
| 17.12 | Local and carried endpoints | U: A2 section 3.2 ("both placements" stays `local` only; carried is 1.2.0's) | U | U | |

## 6. Windows behaviours (A2 line 406: a cell for each behaviour TR1.8 designs)

These cells exist on Windows only. M and L are `N`. Evidence is the candidate's rows on dabeest (PR, PO, PX, HH, WH3, SA, SS, WP3, PG) against the 1.0.17 baselines (WSSH, WAUTH, TD9, TD10, TD12, R13, RP).

| ID | Behaviour | M | L | W | Note |
|---|---|---|---|---|---|
| 18.1 | Pageant as the agent source: holds the key; beats a working pipe; an empty Pageant refuses, no fallthrough | N | N | E: PO (real Pageant 0.83, operator session, PASS on 1.0.17 and candidate); PG (synthetic) | The binary predates the stall fix, spawn gate and C3a |
| 18.2 | Pageant confirmation wait outlasting the stall (D5 b): the stall clock keeps running; `--ssh-timeout` named in the message | N | N | E: PG (stall clock runs while a prompt is open); L20 states the cost | A real confirm-every-use Pageant was not run |
| 18.3 | OpenSSH agent pipe: named pipe, the default pipe with `SSH_AUTH_SOCK` unset, UNC refused (naming the variable), absent agent (words of the stopped service, case-insensitive `\\.\PIPE\` per D15) | N | N | E: PR (`agent-pipe-named-rsa`, `-named-ed25519`, `-default-name`, `-unc-name`, `agent-absent`); SA (stopped service); CI-W | Against an owned pipe agent |
| 18.4 | The candidate against the real running OpenSSH Authentication Agent service | N | N | G: TD12 ran the real service on 1.0.17 only (RN run of record, L4); the candidate's pipe rows used an owned agent and a stopped service | Blocker 3.8, or a ruling that L4 stands |
| 18.5 | Default credentials: `Negotiate` and `NTLM` answered by the logon session, helper never asked; Internet-zone name; intranet name; numeric address (OD16) | N | N | EP: D31. PR (6 + 1 rows PASS), HH (real SSPI through the production worker, channel binding) | gwz-sspi 0.1.2 launch guard changes the worker launch path; rows re-run on that build at 6.3 |
| 18.6 | A helper's `Basic` credential for a Basic-only challenge; helper not asked for `Negotiate`/`NTLM` (TD1) | N | N | E: PR (`helper-basic-only`, `helper-not-asked-*`) | |
| 18.7 | `Digest`-only refused at once; `Digest` beside a supported scheme ignored (TD2, Reg2) | N | N | E: PR (`digest-only-*`, `digest-and-negotiate`: PASS, differs as the register says) | D3: no Digest authentication and no helper-identity SSPI in 1.1.0 |
| 18.8 | A rejected logon identity is terminal; a challenged POST refused with a plain message and never replayed (TD1, TD4) | N | N | E: HH (4.4/4.8); CI-W | |
| 18.9 | SSPI channel binding of the real TLS connection, eight rounds, wrong binding refused | N | N | E: HH (`native/tests/real_sspi.rs`); WH3 | L7: remote NTLMv2 Extended Protection not measured (loopback only) |
| 18.10 | `http://` remote by the native route with warning T2 (D2) | N | N | E: PR (`http-remote-native-route-*`) | |
| 18.11 | Machine proxy | N | N | E: see 10.8 | |
| 18.12 | `HOME` unset: SSH home order, `USERPROFILE` fallback, non-ASCII home | N | N | E: PR (2 rows + `switch-network-global-home-unset`); SS; WH3 | |
| 18.13 | The off switch's three forms (flag, `GWZ_TRANSPORT`, user-global `gwz.transport`), precedence, repository-local ignored, with `HOME` unset | N | N | E: PR (12 dry-run combinations + 5 real rows) | |
| 18.14 | Windows trust roots plus a CA file named in the environment (D4, Reg11) | N | N | E: CAT; CI-W | |
| 18.15 | Installed paths with spaces and non-ASCII, worker provenance (4.10) | N | N | E: WH3 | |
| 18.16 | Key-exchange failure on WinCNG retried inside the setup attempt (D14), no partial result on the transport route | N | N | EP: D32 and 6.3. CNG, WF, T84 | D32 raises the retry to two |
| 18.17 | A pre-authentication SSH drop retried (Reg7) | N | N | E: WP3; CNG | |
| 18.18 | Redirect discovery on Windows: a relative `Location` followed within five hops, HTTPS only, no downgrade (TD3, Reg4) | N | N | E: WH3; CI-W | |
| 18.19 | Kerberos | N | N | U: D41, `GwzV110AmendmentS56Scope.md` (L1): Windows may choose Kerberos inside Negotiate and gwz does not prevent it; no claim is made | Facts kept: L1 |

## 7. Unsupported, out-of-scope and not-claimed register

| Item | Where it shows | Mark | Authority |
|---|---|---|---|
| Linux ARM64, macOS x86-64 | not columns | `U` | V110 section 2; A2 section 3.2. The Pi evidence is context |
| Reuse across operations, long-lived embedding, the server, carried placement | 6.3, 17.6, 17.12 | `U` | A2 section 3.2, 3.10 |
| Bazel `@gwz_core_crates` | build route, not a cell | `U` | D27 |
| `exact_agent_identity` on macOS and Linux | 9.7 | `U` | V110 S5.6 (exact-agent-until-proven). The operator may turn it on with named fixtures |
| WinCNG limits (RSA-only host keys, RSA PEM key files) | 8.4 | `U` | D16, TD5, L12 |
| Plain `http://` remotes on the transport | 18.10 | native route with a warning, `E` | D2 |
| Explicit-identity SSPI, Digest authentication | 18.7 | refused, `E` | D3, TD1, TD2 |
| Kerberos; a real hardware security-key signature; a Pageant-held or native-agent certificate | 18.19, 9.4 | `U` | D41, `GwzV110AmendmentS56Scope.md`; the facts: TD8 B, L1 to L3 |
| Remote NTLMv2 Extended Protection (L7), the logon exchange's target name (L15), repeated NTLM attempts per host (L19), Mac trusted-HTTPS against 1.0.17 (L5) | no cell | limitations of the design, not cells: no claim is made and nothing is advertised | design sections 13 and 14 item 10 |

## 8. What is pending, by step

| Step | Cells that wait on it |
|---|---|
| 6.3 (S5.5 repeat on dabeest) | 11.2, 11.3, 17.7, 17.8, 17.9, 17.10 (Windows, no evidence yet); 17.1, 17.2, 17.3 (Windows, evidence T84 to be re-measured) |
| D30 (lane `cap72`) | 9.7 Windows (the exact-selection row) |
| D49 (lane `win92`, Windows security-key and DSA rows) | 9.2 Windows |
| D31 (gwz-sspi 0.1.2, lane `sspi012`) | 18.5 |
| D32 (kex retry two, lane `d14two`) | 17.3 Windows, 18.16 |
| D33 (HTTPS multi-thread runtime, re-time on M, L, W) | 17.2 on all three platforms |
| lane `blobpush` | 3.3 |

## 9. Re-runs recommended (evidence that is old, not missing)

| Cell group | What is old | Cheap closure |
|---|---|---|
| gwz-transport contract suites on Windows (rows 1.1, 2.1, 3.1, 4.1, 5.1, 6.1, 7.1, 13.1, 15.1) | Q6A, 2026-09-22, 140 tests of 237 | `cargo +1.95.0 test --locked` (and `--features unstable-sequenced`) in gwz-transport on dabeest, one booking |
| the same on macOS | HC, 2026-10-12, 237 | none needed unless gwz-transport changes |
| Context isolation, the binding package (12.1b), on all three | Q6A, 2026-09-22 (Windows only); `tests/transport_native` has no CI job | `cargo test` of `tests/transport_native` on the Mac, in WSL and on dabeest |
| Windows candidate rows of 18.1, 18.5 | PO/PR binary predates the stall fix, spawn gate and C3a | rows re-run on the release candidate at 6.3 or S7.3 |

## 10. Method and what was read loosely

- Every ID in section 4 was found by listing `runs/` (99 directories) and reading the README verdict sections named in section 4.3. Raw logs and `manifest.json` were not re-checked; `verify_archive.py` was not run (this document changes no evidence).
- CI status is the conclusion of the latest run of each named job on `main` as listed by `gh run list` on 2026-10-17. Job steps were read from the workflow files.
- Test names in cells are file and function names found in the tree at gwz-core `965d884a`. Rows 11.4, 13.3 and 14.1 name modules and were not opened in full.
- Not read in full: `2026-10-03-*` (TR1.8 Windows primitive runs), `2026-10-04-windows-*`, `2026-10-08-windows-*`, `2026-10-09-ssh-syn-loss`, `2026-10-16-r19auth*`, `-r19provision`, `-c21`, `-r19backstop`, `-r19pyfix` (no README), `2026-10-14-ciflake*`, `-fix[ABC]`, `2026-10-13-cifix2`, `2026-10-12-cifix`. None is cited as the only evidence for a cell.
- The Pi runs are read as aarch64 and cited nowhere as L evidence.
