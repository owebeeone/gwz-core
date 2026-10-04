# GWZ transport release plan — amendment 2: 1.1.0 ships the transport on three platforms; the session host, reuse and the server move to 1.2.0

Date: 2026-10-01; revisions 4–6, 2026-10-02; revision 7, 2026-10-05. Status: **accepted at SHA-256 `c5850e52227e9f27e7c989c417ea262d6f749e312593af3cbbc7baeeda468509` after [Consistency-2](GwzTransportReleasePlanAmendment-2-ReviewConsistency-2.md) and [Safety-2](GwzTransportReleasePlanAmendment-2-ReviewSafety-2.md) reported GO; this accepts the amendment text only**.
- This status sentence was added after that GO.
- So were the corrections the reviewers cleared without a further round: Consistency-2's P3-A, P3-B and P3-C, and Safety-2's P3-10 and P3-11. The [verdict](GwzTransportReleasePlanAmendment-2-Verdict.md) records them.
- Revision 1 applied [remediation plan 1](GwzTransportReleasePlanAmendment-2-RemPlan.md) after revision 0's reviews: [Consistency](GwzTransportReleasePlanAmendment-2-ReviewConsistency.md) and [Safety](GwzTransportReleasePlanAmendment-2-ReviewSafety.md), both NO-GO.
- Revision 2 applied [remediation plan 2](GwzTransportReleasePlanAmendment-2-RemPlan-2.md) after revision 1's re-verdicts: [Consistency-1](GwzTransportReleasePlanAmendment-2-ReviewConsistency-1.md) GO, and [Safety-1](GwzTransportReleasePlanAmendment-2-ReviewSafety-1.md) NO-GO on one new P2.
- On 2026-10-01 the operator decided OD14: its alternative, gwz-py's network operations on the per-operation transport entry in 1.1.0. Revision 3 applies it, with the 1.1.0 amendment's S6.1–S6.3 restored (§3.17) and the Python [design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md) they point to. On the operator's instruction it skips the review loop, and one skim review checks it ([skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md)).
- On 2026-10-01 the operator decided OD15 under OD13's parity: the transport itself does what 1.0.17's native path does on Windows, and no native route stands in for it. Revision 4 applies it, with OD16's shape following it (§3.14), three Windows steps (TR4.8–TR4.10), and the server design's and session plan's sentences it changes (§3.18). It is skim-reviewed only, as revision 3 was ([skim review 2](GwzTransportReleasePlanAmendment-2-ReviewSkim-2.md) and its [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-3.md)), because TR1.8's and TR4.10's own dual reviews carry the Windows designs.
- On 2026-10-02 the operator decided three more: OD16's unbounded alternative, so the transport offers the logon session's default credentials to any host that asks, as 1.0.17 does; and OD10 and OD11 reversed, so the transport itself runs the user's configured credential helpers and signs with every agent key type 1.0.17 uses. Revision 5 applies them (§3.19). It is skim-reviewed only, as revisions 3 and 4 were.
- Revision 6 records what Phase 2's implementation added (TR2.13–TR2.22 and the split of `placement_endpoint.rs`), the operator's decisions of 2026-10-02 on its follow-ups, and OD18, the cold start (§3.20). It is skim-reviewed only, as revisions 3–5 were.
- **Revision 7 is a DRAFT, not yet reviewed.** It records three changes the program checkpoint's entries of 2026-10-05 owe this amendment: TR3.3's thirteen crates.io names become fourteen with gwz-sspi, Phase 10 step 2's mechanism for gwz-transport (Gearu, published by Trusted Publishing), and gwz-sspi's place in the publication order (§3.21). The acceptance above covers revision 2's text only, as it did for revisions 3–6. Revision 7's review route is not chosen here. Its open items are listed in §3.21.
- OD17 is taken only if TR8.1 (1.1.0) misses.
- Acceptance authorizes no implementation, commit, tag, push or publish.

This amendment controls the [transport release plan](GwzTransportReleasePlan.md), as [amendment 1](GwzTransportReleasePlanAmendment.md), the [reuse design](../../dev-docs/GwzConnectionReuseDesign.md) and the [server design](../../dev-docs/GwzCoreServerDesign.md) left it, and named sections of the [session plan](../../dev-docs/GwzCoreSessionPlan.md) and the [crate map](../../dev-docs/GwzCoreSessionCrateMap.md).
- It records the operator's decision of 2026-10-01, OD13: the release ships as two minor releases, **1.1.0, the transport**, and **1.2.0, the session release**, and 1.1.0 has **Windows parity**.
- It records OD14, decided by the operator on 2026-10-01: gwz-py's network operations take the per-operation transport entry in 1.1.0 (§3.17). It records OD15, decided by the operator on 2026-10-01: Windows parity is built into the transport, not reached by native routes. OD16, how the transport answers a challenge for the logon session's default credentials, was decided on 2026-10-02, with OD10 and OD11 reversed onto the transport (§3.19). It names a fourth, OD17, to be taken if 1.1.0 misses its speed target. It records OD18, the cold start, decided by the operator on 2026-10-02 (§3.20).
- It adds thirteen steps: TR1.8, TR2.9 to TR2.12, TR3.3, TR3.4, TR4.6 to TR4.10, and TR8.4. Revision 6 adds TR2.13 to TR2.22 and the split of `placement_endpoint.rs` (§3.20). It restores the 1.1.0 amendment's S6.1–S6.3 for 1.1.0 (§3.17).
- It retires no accepted design and no step. It assigns each phase, or part of a phase, to one of the two releases.

## 1. Documents controlled

- `gwz-core/dev-docs/GwzTransportReleasePlan.md`, at SHA-256 `4e05b8e8a1fdffb7ff36fddcf7402c5662c95f441582a84ff65661ac993fbaac` (gwz-core `3f99e49c`). Line numbers below are that file's.
  - That file carries its earlier amendments only as status lines 8–11. Where amendment 1 replaced a clause, this amendment cites the clause as amendment 1 left it.
  - §3.19 changes its OD10 texts: line 52's row, TR1.6, TR2.2's route sentence (line 293), line 423's ledger item and OD10 (lines 533–534).
- `gwz-core/dev-docs/GwzTransportReleasePlanAmendment.md`: §3.3 assigns each of its sections to a release. §3.19 changes its OD11 texts: §3.2's key-type row, §3.4's routed operations, TR2.8, §3.6's routed exit rows, §3.7's cell, §3.8's notes, §3.11's OD11, §3.12's risk, §3.13's out-of-scope bullet, §3.14's RSA sentence and §4's key-type rows.
- `gwz-core/dev-docs/GwzRemoteTransportSshAgentDesign.md`: its §5 fixture sentence, as amendment 1 left it (§3.19).
- `gwz-core/dev-docs/GwzV110PlanAmendment.md`, adopted through the plan's §4: in 1.1.0's run, its §3.4 (Phase 6, S6.1–S6.3) applies as §3.17 restores it, and its §3.5 and §3.6 sentences on gwz-py apply as written (§3.11, §3.12).
- `gwz-core/dev-docs/GwzRemoteTransportRetryPlan.md`: its §4 sentence on what Closed stops, its §5 Cold state and two S3.1 test sentences (§3.20, OD18).
- `gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md`: the Python design that the 1.1.0 amendment's S6.2 and S6.3 call "S1.1's revision" (§3.17).
- `dev-docs/GwzCoreSessionPlan.md`: §1.1, §2.3's G2, §2.4's candidate-build sentence, §3.0's definition of an **Ordinary path** step, §5.3's row for S6.2's release pins, CS5.3's files, CS6.5's two sentences on the lazy endpoint's read, CS7.24, §5.4's `transport_binding.rs` row, Phase 6's preamble sentence on the lazy endpoint's debt entry, and its mentions of gwz-py's `native/src/transport_session.rs` (§3.15); CS8.1's and CS8.18's sentences on the logon session, CS8.18's logon-session test row, and CS8.19's Pageant clause (§3.18); CS8.3's routing rule, its test rows and dependencies, and the sketch's two CS8.3 edges (§3.19); CS3.4's transport half (§3.15, §3.19).
- `dev-docs/GwzCoreSessionCrateMap.md`: its sentence on when candidate crates are published (line 25), and the ruling on the thirteen crates.io names (lines 156 and 199), which TR3.3 applies (§3.16). From revision 7, its count of names reads fourteen with gwz-sspi (§3.21).
- `dev-docs/GwzConnectionReuseDesign.md`: its open item on network operations without a binding (line 377) is decided for 1.1.0 by TR2.11; its supersessions of the retry plan, the HTTPS design and `SshEndpointConfig::from_environment` take effect with its steps in 1.2.0 (§3.1). §3.19 changes its two key-type sentences (lines 120 and 385).
- `dev-docs/GwzCoreServerDesign.md`: its rule "One agent source per session" says "This rule governs the transport", so it governs 1.1.0's in-process transport, whose release now precedes the server's (§3.9). Under OD15, that rule's Pageant bullet changes, and on Windows the logon session joins every session's must-match rows (§3.18). Under OD10's and OD11's reversal, its routed native operations go (§3.19).

Only the clauses in §3 change. The rest of each document stays authoritative as written.

## 2. Problem evidence

1. **Schedule.** Measured with `git diff -M` from each product's `v1.0.17` tag (2026-09-18), and from the first commit of gwz-transport and gwz-git, which did not exist then, to the heads of 2026-10-01:
   - about 29,200 production lines were added, 26,000 without comments or blank lines. About 23,600 of them are the transport: gwz-transport, and gwz-core's endpoint and transport host. They were written from 2026-09-19 to 2026-09-24;
   - about 42,000 test lines were added. Generated code and lock files are excluded.
   - The plans budget about 44,000 production lines still to write:
     - the session plan, 38,300 over 109 steps: Phases 2–6 16,350, reuse (its Phase 7) 8,450, the server (its Phase 8) 12,600;
     - the rest, about 6,000: Phase 2, Windows, TR3.2, and activation's wiring and documents.
   - The plan's §8 already says: "Splitting the release later is an operator decision."
2. **The candidate's speed.** Measured on 2026-10-01 on macOS ARM64: a no-op `gwz fetch` of 32 small public repositories over SSH, the set the pool-capacity brief used, medians of three rounds. A `connect()` interposer counted each build's TCP connections, because the CLI's transport rows do not show connections. The candidate was built from gwz-core `0ddc513c` and gwz-cli `5ebb001`.

   | Setting | 1.0.17 | Candidate |
   |---|---|---|
   | Each build's defaults | 8.7 s, 32 connections | 7.0 s, 24 connections |
   | `--max-per-host 32` | 2.5 s, 32 connections | 7.0 s, 24 connections |
   | `--max-per-host 4` | 18.3 s, 32 connections | 8.1 s, 4 connections |

   - Pooling works: at 4 per host, all 32 fetches ran over 4 connections. No run of either build failed a member.
   - At its defaults, which allow 32 per host, the candidate misses TR8.1's target by a factor of 2.8: 7.0 s against 1.0.17's 2.5 s at `--max-per-host 32`. Two causes were found:
     - **Open admission.** `MAX_OPEN_JOBS = 8` (`src/git/endpoint/placement_endpoint.rs:31`) admits 8 opens at a time and queues the rest (`:280`). The pool's caps follow the operation since the retry plan's Phase 1, but this limit does not. The candidate opened its connections in three waves of 8, at 0 s, 1.26 s and 2.58 s.
     - **Closes.** With that limit raised to 64 in a scratch build, all 32 connections reached the SSH agent within 1 s, but the fetch took 12.6 s at its defaults (12.3 s at `--max-per-host 32`). A process sample shows every member thread waiting in `stream_io::BlockingStream::close`, called from libgit2's `git_smart__close`, and the members finishing one at a time about 0.3 s apart, while every gwz thread is idle. A one-repository fetch took 2.2 s on both builds, in a run whose log was not kept. The root cause is not yet known.
   - No step of the plan covers either cause. The raw logs were kept only in a scratch directory; TR2.9, TR2.10 and TR8.1 measure again under §2's evidence rules.
3. **Windows.**
   - 15 candidate sites are gated `all(unix, gwz_transport_candidate)`: 11 in gwz-core, 2 in gwz-cli and 2 in gwz-py's native extension. No Windows candidate has been built. 1.1.0 S4.5 opens the transport's sites.
   - No push-triggered CI job builds or tests gwz-core on Windows. The boundary job runs a cross-target Clippy for the platform-table crates only (`checked-artifact-boundary.yml:206-220`), and `windows-matrix.yml` and `platform-matrix.yml` run on dispatch only.
   - The native path on Windows does things the transport does not:
     - **Pageant.** libssh2 tries Pageant first whenever a Pageant window is visible, and only then the pipe `SSH_AUTH_SOCK` names, or `\\.\pipe\openssh-ssh-agent` (agent.c:436-439; agent_win.c:124-138, as the server design cites them). The transport uses Pageant only when `SSH_AUTH_SOCK` names its pipe (line 341, and the server design's "One agent source per session").
     - **The SSH home.** libgit2 resolves the home from `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE` (`libgit2/src/libgit2/sysdir.c:329-330, 357`), and reads `known_hosts` under it. The transport takes the home from `HOME` alone (`src/transport_host/mod.rs:59-62`). In PowerShell and cmd, `HOME` is usually unset.
     - **WinHTTP.** libgit2's WinHTTP backend opens its session with `WINHTTP_ACCESS_TYPE_DEFAULT_PROXY` (`libgit2/src/libgit2/transports/winhttp.c:830`), so a proxy configured for WinHTTP applies, and gwz sets no proxy options, so the environment's proxy variables are ignored. As the server design's native rows record, it can also send the logon session's default credentials, when a 401 offers `NTLM` or `Negotiate` (`winhttp.c:184-212, 600-626`). The transport reads proxies only from the environment, refusing the forms it does not support before any open, and authenticates HTTPS only through `gh`.
   - A Windows user of 1.0.17 who relies on any of these would break on upgrade. That is the break OD10 and OD11 already route around for other credentials.
4. **Publishing.** Main's ordinary build depends on these unpublished crates, so any release from main, 1.1.0 or a 1.0.x, needs them published first:
   - `gwz-git2` and `gwz-libgit2-sys`, by path, both `publish = false`. crates.io has only their `0.0.0-bootstrap.1` placeholders. v1.0.17 built against crates.io's `git2` 0.21;
   - the internal crates `gwz-ids`, `gwz-session-channel`, `gwz-session-contract` and `gwz-session-host`. Their names are not registered on crates.io. gwz-core's other internal crates are published at `0.0.8` by its release workflow.
5. **Routes without a host context.** `transport_binding::configure` installs the HTTPS route only with a host context (`src/git/gitbackend/transport_binding.rs:184-186`). For SSH it installs the transport either way. Without a host context, it uses a lazy endpoint built from the process's `HOME` and `SSH_AUTH_SOCK` (`:36-48`, `:238-254`). Only gwz-cli's `with_local_transport` installs a host context. Removing the candidate switch as revision 0 wrote it would therefore put every gwz-py SSH operation on the transport, and on Windows with `HOME` unset fail it.
6. **gwz-py's `TransportSession`** was removed at gwz-py `a342b95` on 2026-09-28. gwz-py's two remaining candidate sites (`native/src/dispatch/mod.rs:423`, `dispatch/merge.rs:95`) are schema arms that fill the candidate protocol's `transport_message` with `None`.

## 3. Superseded clauses and their replacements

### 3.1 The status block and §1 (lines 11, 18, 24–28, 30–32, 40 and 42)

- After line 11: "On 2026-10-01 the operator decided OD13: the release ships as 1.1.0 and 1.2.0, and 1.1.0 has Windows parity ([amendment 2](GwzTransportReleasePlanAmendment-2.md)). OD14 and OD15 were decided the same day (§3.17, §3.14); OD16, and the reversal of OD10 and OD11, on 2026-10-02 (§3.19)."
- Line 18 becomes: "**Name and versions.** This is the **transport release**. It ships as two minor releases of gwz-core, the `gwz` CLI and gwz-py: **1.1.0, the transport**, and **1.2.0, the session release**. Each version is fixed when its run of Phase 10 starts. In adopted text, "1.1.0" and "v1.1.0" mean the version of the release that runs the text, including in pins such as `gwz-core = "=1.1.0"` and in tags: in text this plan assigns to 1.2.0 they read 1.2.0 and v1.2.0. A step ID keeps its source's prefix: "1.1.0 S7.1" names the 1.1.0 plan's step, whichever release runs it. A step that runs for both releases is written with the release after it, as "S7.1 (1.1.0)". The published line today is 1.0.17."
- Lines 24–28 become:
  - "**Outcome of 1.1.0.** The normal builds of gwz, gwz-core and gwz-py, on macOS ARM64, Linux x86-64 and Windows x86-64, ship:
    - the endpoint-owned SSH and HTTPS transport, with HTTPS authenticated through the user's configured credential helpers (TR1.6), in the `local` placement, with connection pooling within one command and connection-setup retry, used in process by the `gwz` CLI's network commands and by gwz-py's network operations, each inside its own per-operation runtime (OD14). A gwz-core caller that opens no runtime takes the native route (TR2.11);
    - Windows parity, as OD13 defines it;
    - gwz-py with its public API unchanged. Its network operations run on the transport, one runtime per operation, at most 8 at once per `Client`, cancellable, and joined by `close()` and at interpreter exit (OD14, §3.17)."
  - "**Outcome of 1.2.0.** The normal builds on the same platforms also ship:" followed by lines 26–28 as written, with line 27 as amendment 1's §3.1 and its changelog's OD12 entry left it.
- After line 32, the decisions list gains: "2026-10-01, by amendment 2: OD13, two minor releases, and Windows parity in 1.1.0; OD14, gwz-py's network operations on the per-operation transport entry in 1.1.0; OD15, Windows parity built into the transport; 2026-10-02: OD16, the default-credential offer to any host, and OD10 and OD11 reversed onto the transport."
- Line 40, for 1.1.0: the retry plan, including its §3 item 7, §6 and S1.4, and the HTTPS design's §4 snapshot apply as written. TR1.2's amendments to them, and the reuse design's other supersessions, take effect with the reuse steps in 1.2.0.
- Line 42's session contract, server design and session plan govern 1.2.0. The server design's rule "One agent source per session" also governs 1.1.0 (§3.9).

### 3.2 §2, scope (lines 48–57)

- The table gains a first column, **Release**:
  - **1.1.0** for the rows at lines 50, 52, 55 and 57, and for amendment 1's §3.2 row on agent keys;
  - **1.2.0** for the rows at lines 53, 54 (as amendment 1's §3.2 left it) and 56.
- The row at line 51 reads, for 1.1.0, "SSH and HTTPS, authenticated through the user's configured credential helpers (TR1.6), in the `local` placement, with connections pooled within one CLI command or one gwz-py operation"; and for 1.2.0, "SSH and HTTPS, authenticated through the user's configured credential helpers (TR1.6), in the `local` placement, with connections pooled in the host context".
- The row at line 57 gains, in its first column: "and the internal crates `gwz-ids`, `gwz-session-channel`, `gwz-session-contract` and `gwz-session-host` (TR3.3)".
- Two rows are added for 1.1.0:

  | Release | In this release | Recorded as unsupported in this release |
  |---|---|---|
  | 1.1.0 | gwz-py's network operations on the transport, one runtime per operation, with its public API unchanged (OD14, §3.17) | Reuse across a Python process's operations, which 1.2.0 ships |
  | 1.1.0 | On Windows, the transport does what 1.0.17's native path does: Pageant, the WinHTTP machine proxy, the logon session's default credentials under OD16, and libgit2's SSH home order (OD15, TR1.8, TR4.8–TR4.10) | — |

- Line 64's dabeest rules apply to every Windows row of both releases.

### 3.3 Each phase's release

| Release | Parts of the plan |
|---|---|
| 1.1.0 | Phase 1's TR1.5 and TR1.6, as §3.6 restricts them, and TR1.8. Phases 2, 3 and 4, with the steps §3.5 adds, including TR4.8–TR4.10 in Phase 4, and §3.17's 1.1.0 S6.1–S6.3 in Phase 2. Phases 8, 9 and 10 as §3.10–§3.12 restrict them |
| 1.2.0 | Phases 5, 6 and 7. Phases 8, 9 and 10 again, for what §3.10–§3.12 assign to 1.2.0 |
| Done | TR1.1 (2026-09-27); TR1.2, TR1.3, TR1.4a, TR1.4b and TR1.7 (2026-09-28); TR3.1, whose exit holds in the tree (the rename at git2-rs `d13951f` and gwz-git `a9d7ee0`, and the candidate build in CI). The checkpoint records TR3.1's closure when this amendment is accepted |

Amendment 1's sections are assigned as follows:

| Amendment 1 | Release |
|---|---|
| §3.1: line 7's decision record and line 28's decisions | Both: OD11 is 1.1.0's, OD12 and the stdio mode are 1.2.0's |
| §3.1: line 23's server bullet | 1.2.0 |
| §3.2: the agent-key row, and the redaction of agent key comments and fingerprints | 1.1.0, and 1.2.0 |
| §3.2: the server row | 1.2.0 |
| §3.3 (TR1.2) and §3.4 (TR1.3) | 1.2.0 |
| §3.5 (TR2.7, TR2.8, and TR2.6's scope) | 1.1.0, with TR2.8 as §3.19 rewrites it. TR2.8's "Through a server" bullet is removed (§3.19) |
| §3.6 (Phase 7's exit) | 1.2.0 |
| §3.7 (the agent key types cell) | 1.1.0, and 1.2.0 |
| §3.8: the notes on key types, RSA, agent confirmation and CA roots, as §3.19 rewrites the first two. Its route-ledger row is removed (§3.19) | 1.1.0, and 1.2.0 |
| §3.8: the stdio items, the address grammar and the SSH remote form | 1.2.0 |
| §3.9 (the stdio post-release row) | 1.2.0 |
| §3.10: the TR2.7 and TR2.8 sketch edges and line 484's list | 1.1.0 |
| §3.10: line 493's server text | 1.2.0 |
| §3.11 (OD11 and OD12) | OD11: 1.1.0. OD12: 1.2.0 |
| §3.12: the risk on keys the transport cannot sign with | Removed (§3.19) |
| §3.12: the server risks | 1.2.0 |
| §3.13 (out of scope) | Both, as written, less its bullet on admitting security keys or certificates (§3.19) |
| §3.14 (unchanged on purpose) | 1.1.0, and 1.2.0 |
| §4: Phase 2's tests, S5.6's cell, S7.2's notes, and the manual security-key row, run on the transport (§3.19) | 1.1.0 |
| §4: Phase 7's exit, S7.3's fifth route check, and the stdio post-release row | 1.2.0 |
| §5 | Done: amendment 1 was accepted and applied |

Phase 5's milestone (line 344), Phase 6's (line 352) and Phase 7's (line 367) are unchanged, and are 1.2.0's.

### 3.4 §4 (line 114) and the NO-GO

- The row at line 114, the session plan's CS1.1–CS6.5, is 1.2.0's, except S6.2's release pins, which TR3.4 brings into 1.1.0.
- The plan's §4 retirement of the 1.1.0 amendment's S6.1–S6.3 as steps (lines 118–136) is reversed for 1.1.0's run (§3.17). Their obligations stay mapped in the session plan for 1.2.0, whose session host replaces the stage.
- Line 159's closing condition for the NO-GO of 2026-09-23 is 1.2.0's. The code that NO-GO concerned, gwz-py's `TransportSession`, was removed at gwz-py `a342b95` on 2026-09-28.
- The table gains: "This plan's amendment 2 | TR1.8, TR2.9–TR2.12, TR3.3, TR3.4, TR4.6–TR4.10 and TR8.4 | Phases 1, 2, 3, 4 and 8".

### 3.5 New steps

- **TR1.8: Windows parity design (OD15, OD16)** *(design only)*. It designs, in the transport, each thing 1.0.17's native path does on Windows that the transport does not yet do, so that no native route stands in for it (OD15). TR4.8–TR4.10 implement it.
  - **Pageant.** The transport's agent client speaks Pageant's window protocol, the one libssh2 uses. A visible Pageant window is the session's agent source, as libssh2 chooses it (agent.c:436-439); otherwise the pipe `SSH_AUTH_SOCK` names, or `\\.\pipe\openssh-ssh-agent`. The source is chosen once per session, before any connection opens, and a failure never falls back to another source (§3.18). TR1.8 states:
    - the shared-memory exchange: the mapping's name, owner and access, and the largest message;
    - whether the window's process must run as the caller's user, and what happens when it does not;
    - how long one request may wait, since Pageant can hold a request for the user's confirmation;
    - the message when Pageant holds no key the transport can sign with.
  - **The machine proxy.** On Windows the transport reads WinHTTP's default proxy configuration (`WinHttpGetDefaultProxyConfiguration`, which `netsh winhttp set proxy` writes), the proxy 1.0.17's WinHTTP sessions apply (`winhttp.c:830`). It uses it as it uses an environment proxy: a CONNECT tunnel, with the configuration's bypass list. A form it does not support is refused before any open, naming the setting. TR1.8 states:
    - when it is read: once per runtime, beside the environment snapshot, in gwz-cli's runtime and in 1.1.0 S6.1's variant alike;
    - how the bypass list's entries, `<local>` among them, map onto the transport's bypass rule;
    - the inverse case, a proxy in the environment, which 1.0.17 ignores on Windows: which of the two wins when both are set, and whether the transport uses an environment proxy on Windows at all. The migration notes state the outcome against 1.0.17;
    - what the transport does when the machine proxy itself challenges, with a 407. libgit2 cannot answer it in 1.0.17: it passes the proxy options' URL and credentials, both unset, since gwz sets no proxy options (`winhttp.c:1257-1262`). TR1.8 records with 1.0.17 whether WinHTTP's own automatic logon answers the proxy. Unless that row authenticates, the transport refuses a 407 before sending any credential, naming the setting and the off switch.
  - **The logon session's default credentials (OD16), on Windows only.** This covers HTTPS that 1.0.17 authenticates with the logon session's default credentials. The transport answers the challenge itself, through SSPI, on its own connection, so no route changes after an open.
    - **The trigger.** The transport's first request carries no credential and asks only for the ref advertisement. The answer is a 401 whose `WWW-Authenticate` offers `Negotiate` or `NTLM`, from whatever host the request reached, after the transport's one validated discovery redirect. There is no zone bound (OD16): libgit2's own fallback has one (`winhttp.c:230-282`), but 1.0.17 never reaches it. A redirect that arrives during the exchange ends it, with no further credential sent.
    - **Its effect.** On the connection that received the challenge, the transport acquires the logon session's default credentials (`AcquireCredentialsHandleW`), runs the exchange (`InitializeSecurityContextW` for each round), and sends the request again with its token. 1.0.17 offers the same credentials to that host: gwz's credential callback answers a default-credential request (`transport_support.rs:272-274`) before libgit2's zone check would run.
    - **Precedence of the credential helpers.** When the challenge offers `NTLM`, `Basic` or `Digest`, a configured credential helper (TR1.6), `gh` among them, with a credential for the URL the credential will be sent to answers first. On Windows it goes over the scheme WinHTTP picks for a user and password: `Negotiate`, then `NTLM`, then `Digest`, then `Basic` (`winhttp.c:139-147`), through SSPI with the helper's identity (`SEC_WINNT_AUTH_IDENTITY_W`) for the first two. A challenge that offers only `Negotiate` takes the logon session's default credentials, and no helper is asked (`winhttp.c:624-627`). 1.0.17's credential callback orders them so: its helper branch runs only when a user and password are allowed (`transport_support.rs:265-274`). TR1.8 states the precedence and how `Digest` is answered, and S7.2 (1.1.0)'s notes record both.
    - **The hazard (OD16).** Any host that answers with `NTLM` receives the logon session's NetNTLMv2 response, as on 1.0.17. The migration notes state it.
    - **The exchange's parameters.** TR1.8 states the target name (`HTTP/<host>`), the context flags, and the TLS channel-binding token, which WinHTTP supplies and a server that requires Extended Protection demands.
    - **The connection.** An NTLM exchange belongs to one connection. TR1.8 states that the connection stays with its operation until the exchange completes, how an authenticated connection is pooled, and what the retry plan's per-key machine does with a failed exchange.
    - TR1.8 states the trigger exactly.
  - **The same challenge on macOS and Linux.** OD16 does not apply there. TR1.8 records, with 1.0.17 against the loopback `Negotiate` fixture, whether 1.0.17 authenticates on each. If it does, the migration notes list the case as unsupported on the transport, refused with a message naming the off switch.
  - **The SSH home.** The transport resolves it on Windows as libgit2 does: `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE`. `known_hosts` is read under it. 1.1.0 S4.4's Windows arm implements it.
  - **Also:** the agent forms 1.1.0 S4.3 admits, with the server design's local-pipe rule; how `gh` and credential helpers are found and started on Windows, including TR2.2's two helper forms and paths with spaces; and the off switch's three forms on Windows.
  - **Evidence first.** Before the design freezes, each behaviour it designs is recorded with 1.0.17 on dabeest: whether it works there, and how. At least one row runs with `HOME` unset, recorded as its own row. One 1.0.17 row on macOS and one on Linux run against the loopback `Negotiate` fixture. A row against a live account needs the operator's go (§2).
  - **Fixtures,** under line 64's rules:
    - for Pageant: Pageant from one pinned PuTTY release, holding a disposable key, with rows for Pageant alone, Pageant and the OpenSSH agent both running, and neither;
    - for the WinHTTP proxy: a loopback CONNECT proxy, the administrator step that sets it, and a restore step that ends every row (`netsh winhttp reset proxy`);
    - for OD16: a loopback HTTPS server that answers `WWW-Authenticate: Negotiate` or `NTLM`, and on Windows completes the exchange through SSPI's server side for the logged-on user. On Windows it is reached under a name that maps to the Intranet zone and under one that maps to the Internet zone; TR1.8 names both. Under either name the operation authenticates on the transport, as on 1.0.17 (OD16). A row whose challenge offers `Negotiate, Basic`, with a configured helper able to answer, the fake `gh` and then a non-gh helper, authenticates with the helper's identity over `Negotiate`, as WinHTTP picks it, with no default-credential offer. An `NTLM`-only row and a `Digest` row, each with a helper able to answer, authenticate with the helper's identity. A `Negotiate`-only row with a helper credential present asserts that the helper is not asked and the logon session authenticates. A row in which one name redirects discovery to the other authenticates at the second. A row whose server requires channel binding authenticates;
    - for the machine proxy's own challenge: the loopback CONNECT proxy answering 407 with `Negotiate`, recorded with 1.0.17 first, then asserting TR1.8's choice.
  - **Review:** dual Consistency and Safety, plus Surface, since each behaviour changes what a Windows user sees. Its Safety finding list names the forced-authentication hazard the operator accepted (OD16), Pageant's window and shared memory, the machine proxy's bypass rule, and, if the 1.0.17 row authenticates to the proxy, the default-credential exchange with the machine proxy.
  - **Timing:** after 1.1.0 S4.1, and before S4.3's agent forms freeze. TR4.8–TR4.10 wait on its GO.
- **TR2.9: concurrent stream closes** *(under 300 lines)*.
  - **Test first,** against the disposable SSH fixture, with the fixture delaying each channel's close by D: for N = 8 and N = 32 exchanges on as many connections, closing concurrently, the closes complete within the same bound, 2D plus a fixed margin, not N × D. The test fails before the fix.
  - Find the root cause of §2 item 2's serialized closes, then fix it where it lies: gwz-core's endpoint; gwz-transport, whose change then ships in 1.1.0's gwz-transport release; or the git2-rs fork's vendored libgit2, whose change moves Phase 10 step 1's libgit2 hash by amendment before Phase 10.
  - A closed stream's connection still returns to the pool only after a clean close, as today.
- **TR2.10: open admission follows the operation** *(under 150 lines)*.
  - `MAX_OPEN_JOBS` gives way to the operation's per-host limit, as the retry plan's §6 defines it, bounded by the endpoint's `MAX_REQUESTS` and the pool's ceilings.
  - **Test first:** 32 opens against a fixture whose setup takes time S all start within S. A test that asserts the queueing beyond the bound stays.
- **TR2.11: no host context, no transport** *(under 200 lines)*.
  - `transport_binding::configure` installs the SSH transport only when the backend carries a host context, as it already does for HTTPS. Without one, SSH takes the native route, chosen before any connection opens.
  - The lazy endpoint then has no production caller. `Runtime::default()`'s environment factory and the `Route::reporting(runtime.endpoint())` branch are removed, with their `env` debt entry. That brings forward the part of CS7.24 that retires the lazy endpoint; CS7.24 keeps the rest of its step.
  - Candidate tests that drive a backend without a host context install one.
  - For 1.1.0 this decides the reuse design's open item on network operations without a binding: both schemes take the native route. Its recommendation to refuse binding-less SSH is decided at S7.1 (1.2.0).
  - **Test first:** in the candidate build, `Git2Backend::new()` against the disposable SSH fixture constructs no transport endpoint, and the same operation inside `with_local_transport` takes the transport. Both fail before the change in the expected direction.
- **TR2.12: the second switch** *(under 100 lines)*. It can start now.
  - It declares `gwz_session_candidate` in the three `check-cfg` declarations (`gwz-core/build.rs`, `gwz-cli/build.rs`, `gwz-py/Cargo.toml`).
  - The process-globals checker's definition of production, which names `gwz_transport_candidate` (`scripts/checks/check_process_globals.py:63`), gains `gwz_session_candidate`. The conditional-compilation check needs no change.
  - **CI.** A second candidate configuration builds with both cfgs. It runs beside the transport-only configuration, which stays until S7.1 (1.1.0) and then becomes the ordinary build's job.
  - **Inventories.** Each product repository keeps an inventory file of each switch's sites in that repository. The checkpoint records each file's digest.
  - **Tests:**
    - a source test, in each product repository's own CI, that the switch's sites equal that repository's inventory file (§3.13, rule (a));
    - a workflow-text test that the candidate job has both legs until S7.1 (1.1.0);
    - `check_process_globals.py` lists a `debt` entry planted under `cfg(gwz_session_candidate)`.
- **TR3.3: the crates.io names** *(under 100 lines, plus the operator's registry steps)*. From revision 7, the thirteen names read fourteen with gwz-sspi, and its exit and order change as §3.21 states.
  - The operator's ruling of 2026-09-28 stands: the thirteen names the crate map lists are registered together, just before release preparation. For 1.1.0, that is before its Phase 10.
  - Its registry steps precede TR3.2's removal of gwz-core's bootstrap publish workflow and the revocation of its token.
  - In 1.1.0, the four ordinary crates on main, `gwz-ids`, `gwz-session-channel`, `gwz-session-contract` and `gwz-session-host`, publish real versions through gwz-core's release workflow, as it publishes the other internal crates. gwz-transport, already bootstrapped, is released by Phase 10 (1.1.0) step 2. The other names stay placeholders until 1.2.0 publishes them.
  - **Exit:** the thirteen names visible on crates.io, with trusted publishers configured for the workflows that publish them, recorded in the checkpoint before Phase 10 (1.1.0).
- **TR3.4: gwz-py's release pins** *(under 150 lines)*.
  - It brings the plan §4's S6.2 release-pin obligation into 1.1.0, from CS5.3: `RELEASE.md` and the publish workflow name every native dependency pin, and gwz-core on the release branch is `=1.1.0` from crates.io, which replaces `GwzCratesIoPlan.md` D7's git-tag pin.
  - Its artifacts: `scripts/release.py`, the pin assertions in `.github/workflows/publish.yml`, `RELEASE.md`, and `test_native_module_reports_compiled_core_provenance`.
  - **Test:** `publish.yml` refuses a git-tag pin and accepts `=1.1.0`; `release.py`'s unit test shows the registry form; the provenance test accepts it.
- **TR4.6: Windows CI** *(under 200 lines)*.
  - A job on `windows-latest`, on every push to main and every pull request, builds gwz-core's and gwz-cli's ordinary builds, runs their ordinary suites, and runs the conditional-compilation check. That starts now.
  - After 1.1.0 S4.5, it also builds the candidate and runs every candidate test that needs no fixture.
  - The fixture rows stay on dabeest.
- **TR4.7: Windows implementation review.** Dual peer-blind Code and State review, on the settled tree after 1.1.0 S4.2–S4.5, TR4.6, TR4.8, TR4.9, and TR4.10 with its own review's GO. It is Phase 4's exit.
- **TR4.8: Pageant** *(under 400 lines)*.
  - The Windows module of the transport's agent client speaks Pageant's window protocol, and a session's agent selection takes a visible Pageant window first, as TR1.8 designs them and §3.18 states the rule.
  - **Test first,** against TR1.8's Pageant fixture on dabeest: with Pageant holding the fixture key, an SSH fetch authenticates through it on the transport; with Pageant and the OpenSSH agent both running, Pageant signs, as on 1.0.17, and the OpenSSH agent records no request; with neither, the operation is refused before any connection opens.
- **TR4.9: the WinHTTP machine proxy** *(under 300 lines)*.
  - On Windows the transport reads WinHTTP's default proxy configuration where TR1.8 says, and uses it as it uses an environment proxy, with its bypass list. A form it does not support is refused before any open, naming the setting.
  - **Test first,** against TR1.8's loopback CONNECT proxy on dabeest: with the machine proxy set, an HTTPS fetch on the transport tunnels through it, and the proxy records the CONNECT; a host on its bypass list connects directly; TR1.8's choice for an environment proxy set beside it holds; a proxy that answers 407 gets TR1.8's behaviour, with no credential sent unless TR1.8 found that 1.0.17 authenticates; the restore step ends every row.
- **TR4.10: the logon session's default credentials (OD16)** *(under 500 lines)*.
  - The transport answers a 401 offering `Negotiate` or `NTLM` through SSPI, on the connection that received it, for any host, as 1.0.17 does (OD16) and TR1.8 designs.
  - **Test first,** TR1.8's `Negotiate` fixture rows on dabeest: both names authenticate on the transport; a `Negotiate, Basic` challenge with a configured helper able to answer authenticates with the helper's identity over `Negotiate`, with no default-credential offer; an `NTLM`-only challenge and a `Digest` challenge, each with a helper able to answer, authenticate with the helper's identity; a `Negotiate`-only challenge with a helper credential present takes the logon session, and the helper is not asked; a discovery redirect from one name to the other authenticates at the second; a server that requires channel binding authenticates.
  - **Review:** its own dual Code and State review, since it handles the logon session's credentials. TR4.7 then reviews Phase 4 as a whole.
- **TR8.4: Windows parity rows** *(evidence)*.
  - On dabeest, beside TR8.1 (1.1.0) and before S5.4 (1.1.0): TR8.1's three targets, measured against 1.0.17's Windows build. The live fetch needs the operator's go (§2).
  - A native-route row at the default settings, with the off switch on: 16 and 32 members, three rounds, no partial result.
  - One row per behaviour TR1.8 designs, Pageant, the machine proxy, and default credentials: the operation runs on the transport, and succeeds as it does on 1.0.17.
  - At least one row with `HOME` unset.
  - S5.5 (1.1.0) repeats the speed targets after S5.4 (1.1.0).

### 3.6 Phase 1 (lines 165–281)

- Line 167's second sentence becomes: "Phases 2, 3 and 4 do not wait on it, except: TR2.5 waits on TR1.5; TR2.2's helper handling waits on TR1.6; and 1.1.0 S4.3's agent forms and TR4.8–TR4.10 wait on TR1.8."
- TR1.5 and TR1.6 are 1.1.0's, as written, except TR1.5's **Servers** bullet and its timing sentence on TR1.3's server rule, which are 1.2.0's.
- TR1.8 joins the phase (§3.5).

### 3.7 Phase 2 (lines 282–319)

- Line 282's milestone gains: "and pooled transfers run concurrently, and only a host context reaches the transport: TR2.9 to TR2.11".
- TR2.6 (lines 305–309) also covers TR2.9 to TR2.12, and 1.1.0 S6.1 and S6.2 (§3.17). With amendment 1, it reviews the settled tree after TR2.1–TR2.5, TR2.7–TR2.12, S6.1 and S6.2.
- The rule for main (lines 314–319) stands for 1.1.0.

### 3.8 Phase 3 (lines 321–335)

- TR3.3 and TR3.4 join the phase (§3.5).
- TR3.2 (lines 328–331) waits on TR3.3's registry steps for its removal of gwz-core's bootstrap publish workflow and the token's revocation.

### 3.9 Phase 4 (lines 337–342)

- Line 337's milestone becomes: "SSH and HTTPS run on Windows x86-64 through the same host, with Windows parity (OD13), and TR4.7 has GO".
- Line 341 becomes: "Agent selection on Windows follows 1.1.0 S4.3 and the server design's rule "One agent source per session", which governs the in-process transport from 1.1.0. A missing agent is a refusal, not a fallback to another key store. A visible Pageant window is the session's agent source, as libssh2 chooses it; otherwise the pipe `SSH_AUTH_SOCK` names, or the OpenSSH agent's default pipe (TR1.8, TR4.8, §3.18). The SSH home follows TR1.8's resolution order."
- Line 342, the server's Windows primitives, is 1.2.0's Phase 7 work, as written.
- TR4.6 to TR4.10 join the phase (§3.5). TR4.8 to TR4.10 wait on TR1.8's GO, and TR4.7 on all of them.

### 3.10 Phase 8 (lines 382–406)

- **1.1.0's run:**
  - Line 382's milestone reads, for 1.1.0: "defaults chosen inside the frozen caps, on evidence from the CLI in process on the three platforms".
  - **S5.1 (1.1.0)** reads without "long-lived reuse", which is TR8.2's (1.2.0). "Both placements" stays `local` only (line 110). S5.2 and S5.3 apply as written, without a server.
  - **TR8.1 (1.1.0)** (lines 385–388) applies, measured with the CLI in process, after TR2.1, TR2.9 and TR2.10 have merged. Its connection counts come from the fixture or from a `connect()` record, since the transport rows do not show connections. It gains a native-route row at the default settings, with the off switch on: 16 and 32 members, three rounds, no partial result, on macOS.
  - **On a 1.1.0 miss,** S5.4 retunes first and the rows are measured again. A remaining gap goes to the operator as OD17: ship with the measured gap stated in the notes, hold 1.1.0, or bring in OD8's channels, whose re-run list for 1.1.0 is TR2.6's tests, TR8.1, TR8.4 and S7.4 (1.1.0). S7.1 (1.1.0) waits on TR8.1 being met or on OD17's answer. Lines 392–397 apply to 1.2.0's run.
  - **TR8.4** runs beside TR8.1 (1.1.0), after TR2.1 and before S5.4 (§3.5).
  - **Line 405's defaults** wait, for 1.1.0, on TR8.1, TR8.4 and S5.3. **S5.5 (1.1.0)** then repeats TR8.1's and TR8.4's speed targets on the chosen defaults.
  - **Line 406's sign-off,** for 1.1.0, covers TR2.8's agent key types and signature algorithms, TR1.6's credential helpers, and a cell for each behaviour TR1.8 designs. The server and reuse cells are 1.2.0's.
- **1.2.0's run:** lines 382–406 as written, including TR8.2 and TR8.3. It repeats TR8.1 with reuse.

### 3.11 Phase 9 (lines 408–444)

- **1.1.0's run:**
  - **Milestone (line 408):** "an ordinary build on the three platforms ships the transport, used in process by the CLI and, per operation, by gwz-py, with Windows parity, and the activation review has GO".
  - **Dependencies (line 410):** Phases 2, 3 and 4, and 1.1.0's run of Phase 8, including OD17's answer if TR8.1 (1.1.0) missed.
  - **S7.1 (1.1.0)** (lines 412–415):
    - **The inventory.** Before the step, each repository's inventory file (TR2.12) lists every site that gates 1.2.0 work, by file and symbol, and the checkpoint records the files' digests. The sites are: the session host's transport entry and its arms, the `server` command and its options, `SocketCoreBridge`, reuse, and the candidate protocol's placement projection. The projection is `TransportPlacement`, `TransportOptions.placement` and `.endpoint_path_base`, the `transport_message` fields, the cfg sites that select the candidate protocol in gwz-core, and gwz-py's two schema arms.
    - **The switch.** In all three crates, as line 413 says, every listed site moves to `gwz_session_candidate`, and `gwz_transport_candidate` is removed from every other site: the transport's sites, `--max-retries` and the off switch. Afterwards `rg gwz_session_candidate` equals the inventory files.
    - **The protocol.** The ordinary build compiles the transport against the production schema. Any candidate field the 1.1.0 transport needs in the ordinary protocol is named here, regenerated in gwz-py (`scripts/regen_protocol.py`, `scripts/check_protocol_drift.py`), and placed under S7.5 (1.1.0)'s Surface.
    - **The entries.** `with_local_transport` and 1.1.0 S6.1's variant are the only transport entries in 1.1.0's ordinary build (TR2.11).
    - **The 1.1.0 amendment's §3.5 S7.1 additions,** for 1.1.0: its gwz-py bullet applies to S6.2's sites, which this step activates, while gwz-py's two schema arms move with the projection; its bullet on "S6.1's variant and S6.2's arms" applies as written; its `check-cfg` and harness bullets apply to `gwz_transport_candidate`, while TR2.12's declarations of `gwz_session_candidate` stay.
    - The completeness check stands, for `gwz_transport_candidate`.
  - **S7.2 (1.1.0)** (lines 416–426):
    - Line 417's adoption of the 1.1.0 amendment's S7.2 addition applies for 1.1.0, with S6.3's and S7.3 (1.1.0)'s route assertions as its evidence. Its sentence on gwz-py's notes applies as written: one runtime per network operation, the environment captured at each operation's start, no reuse across operations, and at most 8 operations at once per `Client`.
    - The route ledger has rows for the CLI in process, gwz-py on the per-operation entry, a gwz-core caller without a host context on the native route (TR2.11), and the off switch. OD10's and OD11's routes are gone (§3.19). On Windows, Pageant, the machine proxy and default credentials are the transport route's own, not routes of their own.
    - Help, docs pages and migration notes cover `--max-retries`, the off switch, pooling within one command, Windows parity on the transport (Pageant, the machine proxy and the logon session's default credentials), the hazard of the default-credential offer to any host (OD16) and the credential helpers' precedence over it, the credential helpers the transport runs (TR1.6), the key types and signature algorithms it signs with (TR2.8), the first wave's failing handshakes on a dead key or bad credentials (OD18), the control characters the SSH path refuses (TR2.18), the public paths TR2.19 removes, gwz-py's `dispatch::record` limitation (decision 14), and, where TR1.8 records it, the `Negotiate` case on macOS and Linux, the transport's proxy policy against 1.0.17's on every platform, and amendment 1's §3.8 items.
    - The notes state gwz-py's per-operation transport, as the 1.1.0 amendment's S7.2 sentence gives it, and its cancellation, `close()` and interpreter-exit behaviour (the [design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md)).
    - Lines 419–421, the server, Python and reuse items of line 424, and line 425 are 1.2.0's.
  - **S7.3 (1.1.0)** (lines 427–433), on each platform's consumer build:
    - one CLI network operation in process over SSH, and one over HTTPS, each asserting the transport route through its observations;
    - one gwz-py SSH operation and one HTTPS operation, and two overlapping gwz-py operations, each asserting the transport route through its observations, as the 1.1.0 amendment's §3.5 S7.3 Python sentence states;
    - one Rust caller of gwz-core that opens no runtime, asserting the native route: no transport endpoint is constructed (TR2.11);
    - the absence of the 1.2.0 surfaces: `gwz server` is an unknown command; `--server` and `--no-server` are unknown options; `GWZ_SERVER` has no effect, since the `--verbose` row shows the in-process route; gwz-py exposes no `SocketCoreBridge` and no `server` entry; and `transport_capabilities` reports no session route;
    - on Windows, one operation per behaviour TR1.8 designs asserting the transport route, one Internet-zone `NTLM` challenge asserting that the transport authenticates, as 1.0.17 does (OD16), and one SSH operation with `HOME` unset;
    - on each platform, one HTTPS operation through a non-gh credential helper and one SSH operation through an agent certificate key, each asserting the transport route (§3.19).

    Lines 429–431, and line 433's sentences on the server and reuse, are 1.2.0's.
  - **S7.4 (1.1.0)** rechecks attribution without reuse. Line 434's reuse recheck is 1.2.0's.
  - **S7.5 (1.1.0)** (lines 435–441): Surface covers `--max-retries`, the off switch, the `--verbose` transport-row fields, TR2.3's `errors` contract as TR2.19 widens it, the public paths TR2.19 removes, TR1.8's Windows messages for Pageant and the machine proxy, OD16's notes on the hazard, TR1.6's helper messages, gwz-py's per-operation semantics (cancellation, `close()`, interpreter exit and the per-`Client` limit), any ordinary-protocol field S7.1 (1.1.0) names, every ordinary-path change admitted under rule (e) that §6(c) names, and the absence of the 1.2.0 surfaces. Lines 436 and 439 are 1.2.0's.
  - **Exit rows (lines 442–444):** "Network-entry ledger" maps, for 1.1.0, to S7.2 (1.1.0) and S7.3 (1.1.0).
- **1.2.0's run:** lines 408–444 as written, after 1.1.0's tag. Its S7.1 removes `gwz_session_candidate`, with the same completeness check for that name.

### 3.12 Phase 10 (lines 446–463)

- **1.1.0's run:**
  - It applies with 1.1.0's version. TR3.3 and TR3.4 are further prerequisites. From revision 7, step 2's mechanism, a new step ahead of step 6 for gwz-sspi, and the order of steps 6 and 7 read as §3.21 states.
  - **A Windows precondition.** Before step 5's gwz-core tag, `windows-matrix.yml` is dispatched on the release commit and passes, and the checkpoint records the run.
  - **Line 458:** 1.1.0's gwz-transport release includes TR2.4's feature, and TR2.9's change if it lands there. The plan's Phase 6 (reuse) transport changes go in 1.2.0's.
  - **Line 459 and step 7:** gwz-py is released with its per-operation transport. Its wheels use TR3.4's pins, with gwz-core at `=1.1.0`. The 1.1.0 amendment's §3.6 step-7 sentence applies as written.
  - **Post-release check (lines 460–463),** for 1.1.0, on each host:
    - one SSH and one HTTPS network command from the installed CLI takes the transport route, by its `--verbose` row;
    - one gwz-py network operation takes the transport route, as the 1.1.0 amendment's §3.6 post-release sentence states;
    - the absence of the 1.2.0 surfaces, as S7.3 (1.1.0) checks it;
    - on dabeest, one behaviour TR1.8 designs, on the transport, and one SSH command with `HOME` unset.

    Lines 461–463 are 1.2.0's.
- **1.2.0's run:** lines 446–463 as written, with 1.2.0's version (§3.1).

### 3.13 §6 (lines 465–508)

- **The sketch gains:**

  ```text
  Phase 1   S4.1 ── TR1.8 ── (S4.3's agent forms, TR4.8–TR4.10, TR8.4's parity rows)
  Phase 2   TR2.10 ── TR2.9 ── TR2.6;  TR2.11 ── TR2.6;  TR2.12 now
            1.1.0 S6.1 ── S6.2 (after TR2.11) ── TR2.6;  S6.2 ── S6.3 (its dabeest rows after S4.5)
            TR2.9 + TR2.10 ── TR8.1 (1.1.0)
  Phase 3   TR3.3's registry steps ── TR3.2's workflow removal and token revocation
            TR3.3 + TR3.4 ── release steps (1.1.0)
  Phase 4   TR4.6 now; S4.5 ── TR4.6's candidate part;  TR1.8 ── TR4.8, TR4.9, TR4.10 (its own review)
            S4.2–S4.5 + TR4.6 + TR4.8–TR4.10 ── TR4.7
  Phase 8   S4.5 + TR4.8–TR4.10 + TR2.9 + TR2.10 ── TR8.4;  TR8.1 (1.1.0) + TR8.4 + S5.3 ── S5.4 (1.1.0) ── S5.5 (1.1.0)
            a TR8.1 (1.1.0) miss ── S5.4 retune ── TR8.1 again ── OD17
  Phase 9   1.1.0: Phases 2, 3 and 4, and Phase 8 (1.1.0) ── S7.1 (1.1.0) … S7.5 (1.1.0)
            1.2.0: 1.1.0's tag, Phases 5–7 and Phase 8 (1.2.0) ── S7.1 (1.2.0) … S7.5 (1.2.0)
  Phase 10  1.1.0: S7.5 (1.1.0) + S2.3 + TR3.2 + TR3.3 + TR3.4 + the Windows precondition ── release steps
  ```

- **"What can start now"** (lines 486–492) becomes:
  - TR2.9, TR2.10, TR2.11 and TR2.12; 1.1.0 S6.1, then S6.2 and S6.3;
  - TR2.2's reproduction and fix, TR2.3, TR2.4, TR2.7 and TR2.8;
  - TR1.5, then TR2.5; TR1.6;
  - 1.1.0 S4.1, then TR1.8 and S4.2–S4.4, then TR4.8–TR4.10 on TR1.8's GO; TR4.6's ordinary-build job;
  - 1.1.0 S2.1–S2.3, TR3.3's code, and TR3.4; TR3.2 after TR3.3's registry steps;
  - 1.1.0 S3.1–S3.3;
  - the session plan's steps, under rule (e).
- **Rule (a)** (lines 495–502) becomes: "**(a) Two named switches.** `gwz_transport_candidate` gates the transport, `--max-retries` and the off switch until S7.1 (1.1.0) removes it. `gwz_session_candidate`, declared by TR2.12, gates the session host's transport entry, the `server` command and its options, `SocketCoreBridge`, reuse, and the candidate protocol's placement projection, until S7.1 (1.2.0) removes it. Each product repository keeps an inventory file of each switch's sites, which its source test reads, and the checkpoint records the files' digests. Until S7.1 (1.1.0), CI keeps two candidate shapes green: the transport switch alone, and both switches. A site that gates 1.2.0 work uses `gwz_session_candidate` from TR2.12's merge; S7.1 (1.1.0) moves any such site still under `gwz_transport_candidate`."
- **A new rule (e), after line 508:** "**(e) Main stays releasable for 1.1.0.** From TR2.12's merge until 1.1.0's tag, a session-plan step merges to main as the session plan's G2 says, except a step marked **Ordinary path**, which merges before the tag only if its review accepts its ordinary-build change for 1.1.0 and the checkpoint lists it under (d). Otherwise it waits in its lane until the tag, or puts its change behind `gwz_session_candidate`. Before TR2.12's merge, a marked step whose change has no such review waits."

### 3.14 §7, §8 and §9

- **§7 (line 512)** gains: "OD13, OD14 and OD15 were decided on 2026-10-01 (amendment 2), and OD16 on 2026-10-02, when the operator also reversed OD10 and OD11 (§3.19). OD17 is taken if TR8.1 (1.1.0) misses. OD18 was decided on 2026-10-02 (§3.20)." These entries follow line 535:
  - **OD13. Two minor releases, with Windows parity in 1.1.0.** Decided by the operator on 2026-10-01.
    - **1.1.0** ships the transport to the `gwz` CLI, in process, on macOS ARM64, Linux x86-64 and Windows x86-64.
    - **1.2.0** ships the session host, reuse, the server, and gwz-py through the session host, replacing 1.1.0's per-operation stage (OD14).
    - **Windows parity:**
      - every transport behaviour that 1.1.0 ships on macOS and Linux ships on Windows x86-64, with the same tests, and with its exit evidence from dabeest;
      - a Windows configuration that works on 1.0.17 keeps working on 1.1.0, on the transport (OD15). With OD10 and OD11 reversed (§3.19), the same holds on every platform for credential helpers and agent keys.
  - **OD14. gwz-py in 1.1.0.** Decided by the operator on 2026-10-01: the alternative. In 1.1.0, gwz-py's network operations take the transport, each inside its own per-operation runtime, through 1.1.0 S6.1's variant of `with_local_transport`. The 1.1.0 amendment's S6.1–S6.3 are restored for this (§3.17), and the Python [design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md) states the per-operation meanings. 1.2.0's session host replaces the stage.
    - Line 550's statement holds: the per-operation Python stage returns, at the cost OD1 named. The recommendation, gwz-py on the native path until 1.2.0, was not taken.
    - The operator directed that the decision be applied without the review loop, with one skim review at the end. TR2.11 still removes the hidden lazy endpoint: gwz-py opens its runtime deliberately.
  - **OD15. How Windows parity is reached.** Decided by the operator on 2026-10-01, under OD13: the transport itself does what 1.0.17's native path does on Windows. TR1.8 designs Pageant, the WinHTTP machine proxy and the logon session's default credentials in the transport, and adopts libgit2's SSH home resolution. TR4.8–TR4.10 implement them.
    - The recommendation, a native route for each such behaviour, was not taken. A native route runs libgit2 inside the core's process, so it gets no pooling, and no reuse once 1.2.0 ships. It cannot serve the SSH remote form (OD12), whose core runs on another machine, nor client placement if that is scheduled: only behaviours the transport implements can be served from the client's side.
    - Its cost is TR1.8's larger design and the three steps, which 1.1.0 waits on.
  - **OD16. HTTPS that relies on the logon session's default credentials, on Windows.** Decided by the operator on 2026-10-02: the unbounded alternative. Its only signal is the server's 401 challenge. Under OD15 the transport answers that challenge itself, through SSPI, on its own connection, as TR1.8 states it:
    - when an anonymous first request for the ref advertisement is answered with a 401 offering `Negotiate` or `NTLM`, the transport offers the logon session's default credentials to that host, whatever its zone, as 1.0.17 does;
    - **precedence:** when the challenge offers `NTLM`, `Basic` or `Digest`, a configured credential helper, `gh` among them, with a credential for the URL the credential goes to answers first, with its own identity, as TR1.8 states. A challenge that offers only `Negotiate` takes the logon session's default credentials, and no helper is asked, as on 1.0.17;
    - **the hazard, accepted for parity:** a host that answers with `NTLM` receives the logon session's NetNTLMv2 response, which can be cracked offline or relayed, as on 1.0.17. The migration notes state it. The off switch does not lessen it, since the native path makes the same offer.
    - The recommendation, revision 2's bound to the Local Machine, Intranet and Trusted zones with a refusal elsewhere, was not taken. Alternative (a), refusing every such challenge, does not meet OD13's parity.
  - **OD17. A TR8.1 (1.1.0) miss.** Taken when the measurement is in: ship with the measured gap stated in the notes, hold 1.1.0, or bring in OD8's channels.
  - **OD18. The cold start.** Decided by the operator on 2026-10-02: in each operation, a key's first wave of setups starts in parallel, up to the operation's per-host limit, as 1.0.17's connections do, and the retry plan's single-probe rules apply from the wave's first retriable failure. §3.20 states it, with its hazard.
- **§8:**
  - Line 539 becomes: "**Schedule.** 1.1.0 waits on Phases 2–4 and its measurements; 1.2.0 on the session program. Every Windows transport row that needs a fixture runs on dabeest, so dabeest's availability bounds 1.1.0." Lines 540–549 are 1.2.0's.
  - Line 550 becomes: "**The split** (OD13) ships the transport in 1.1.0 to the CLI and, one runtime per operation, to gwz-py (OD14). 1.2.0 replaces gwz-py's stage with the session host."
  - Line 557 gains: "Windows parity puts every 1.1.0 transport row that needs a fixture on dabeest, and TR4.8–TR4.10 add Windows-only transport code: Pageant's window protocol, the machine proxy and SSPI. TR4.6 is the only Windows job on push."
  - A new risk: "**Performance.** The candidate misses TR8.1 today (amendment 2, §2 item 2). If TR2.9 and TR2.10 do not close the gap, OD17 comes to the operator before S7.1 (1.1.0)."
- **§9** gains: "In 1.1.0: any client's use of the session host, reuse across operations or commands, and the server. They ship in 1.2.0."

### 3.15 The session plan

- **§1.1:**
  - The heading "1.1 The transport release" becomes "1.1 The transport release's 1.2.0".
  - In its first paragraph, "Its release, expected to be v1.1.0, ships in the normal builds on macOS ARM64, Linux x86-64 and Windows" becomes "Under its amendment 2 (OD13), the transport release ships as 1.1.0, the transport, and 1.2.0, this plan's release. 1.2.0 ships in the normal builds on macOS ARM64, Linux x86-64 and Windows".
  - In its last bullet, "Activation, 1.1.0 S7.1 in the transport plan's Phase 9," reads "Activation, S7.1 (1.2.0) in the transport plan's Phase 9,".
- **§2.3, G2:**
  - The bullet "**No tag precedes any phase.** The release's tags come in the transport plan's Phase 10, after this plan's phases and activation." becomes: "**1.1.0's tag precedes this plan's activation.** 1.1.0 is tagged in the transport plan's Phase 10 (1.1.0), while this plan's phases run. This plan's steps merge as the merge-timing bullet says, under rule (e) until that tag. 1.2.0's tags come in Phase 10 (1.2.0), after this plan's phases and activation."
  - In the merge-timing bullet, "(a)" reads: "under (a), `gwz_transport_candidate` gates the transport until S7.1 (1.1.0), and `gwz_session_candidate` gates this plan's candidate-only surfaces until S7.1 (1.2.0); rule (e) holds until 1.1.0's tag".
  - The list of candidate-only code reads without gwz-py's `native/src/transport_session.rs`.
- **S6.2's release pins:** §5.3's row for them, and CS5.3's files and implementation sentence for them, move to the transport plan's TR3.4. CS5.3 keeps the rest of its step.
- **gwz-py's `TransportSession`:** the plan's mentions of `native/src/transport_session.rs`, in §1.1, G2, CS1.1's files, CS4.8, §5.3 and §5.4, describe code removed at gwz-py `a342b95` on 2026-09-28. They read as done.
- **CS7.24** loses the lazy endpoint's retirement to TR2.11, and keeps the rest of its step. §5.4's `transport_binding.rs` row, Phase 6's preamble sentence on the lazy endpoint's debt entry, and CS6.5's two sentences on the lazy endpoint's read, read as TR2.11's.
- **CS3.4's transport half,** the `git credential fill` spawn for the transport's HTTPS, is TR1.6's in 1.1.0 (§3.19). CS3.4 keeps its native-callback change, its §15.8 rows and its tests, under rule (e).
- **§3.0's marker.** "a build without `gwz_transport_candidate`" reads "a build without the candidate switch rule (a) names at the time".
- **§2.4's candidate-build sentence** reads "the candidate switch rule (a) names at the time" in place of `gwz_transport_candidate`.

### 3.16 The crate map

- Line 25's "published at activation with gwz-transport, before gwz-core 1.1.0" reads "published at activation with gwz-transport, before gwz-core 1.2.0". No candidate crate enters 1.1.0's ordinary build.
- The ruling at line 199 stands. TR3.3 applies it, before 1.1.0's Phase 10.

### 3.17 The 1.1.0 amendment's Phase 6, restored for 1.1.0 (OD14)

- **The steps.** The plan's §4 retired the 1.1.0 amendment's S6.1–S6.3 as steps. That retirement is reversed for 1.1.0's run. The steps run in this plan's Phase 2, as the 1.1.0 amendment's §3.4 states them, with these changes:
  - **S6.1** also takes an environment snapshot from its caller, and the runtime reads no process environment. `SshEndpointConfig`, `HttpsEndpointConfig` and the TLS and proxy configuration come from the snapshot, with, on Windows, the machine proxy read where TR1.8 says, and the SSH home follows TR1.8's order on Windows (design §2.2).
  - **S6.2:**
    - its "What goes" bullet is done: gwz-py `a342b95`;
    - its release-pins bullet is TR3.4's;
    - "S1.1's revision" reads the [design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md);
    - gwz-py takes the snapshot at its native entry, while holding the GIL (design §2.3);
    - its transport scope is one predicate in gwz-core, shared with gwz-cli and pinned to gwz-core's production `with_transport` call sites (design §2.1);
    - its per-`Client` limit of 8 replaces the bridge's per-event-loop network lock, and counts both `call` and `submit` (design §2.4);
    - the limit, the cancellation registry and the cleanup reports belong to a per-`Client` native object, `ClientHost`, not to a static (design §2.4–§2.5);
    - TR1.5's environment and user-configuration forms of the off switch govern gwz-py's operations, resolved from the snapshot and the user configuration at each operation's start. With the switch on, an operation takes the native route and builds no runtime. TR1.5 also states gwz-py's form of the first-operation notice (design §2.2).
  - **S6.3** runs as written, plus the design's §3 rows, including one with the off switch on and one closing a `Client` with a failing operation in flight. Its dabeest rows wait on S4.5, and include a row with `HOME` unset.
  - **Phase 6's preamble:** "This phase does not change gwz-cli" reads "gwz-cli changes only to call the shared predicate".
- **The S1.2 gate.** The 1.1.0 amendment's Phase 6 preamble, "Phase 6 starts after S1.2 closes", reads "Phase 6 starts on the operator's OD14 decision".
  - The Python design is S1.1's revision.
  - The operator's decision and the [skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md), whose findings the design and this amendment apply, stand in for S1.2's GO.
  - S7.5 (1.1.0)'s Surface review covers the documented behaviour change that S1.2's Surface review would have read: the environment's capture point.
  - S1.1 and S1.2 stay retired as steps.
- **Review.** TR2.6 reviews S6.1 and S6.2 on the settled tree. S6.3's evidence feeds S7.2 (1.1.0) and S7.3 (1.1.0).
- **1.2.0.** The session plan's mappings of S6.1–S6.3's obligations to CS steps stay, for 1.2.0, whose session host replaces this stage.

### 3.18 The server design and the session plan, under OD15

The transport now uses the Windows logon session: Pageant's window belongs to it, and so do the default credentials OD16 offers. These edits follow. The in-process transport takes the agent rule from 1.1.0, through TR4.8; the rest is 1.2.0's, with the server.

- **Server design §5, "One agent source per session":**
  - Line 497's "It is derived from the session's snapshot, never from the server's environment" reads "It is derived from the session's snapshot, or on Windows from the Pageant window visible to the host's logon session, never from the server's environment".
  - Line 502, the Pageant bullet, becomes: "**Pageant.** On Windows, a visible Pageant window is the session's agent source, as libssh2 chooses it (agent.c:436-439); otherwise the snapshot's `SSH_AUTH_SOCK`, or the default pipe. The transport speaks Pageant's window protocol (amendment 2's TR1.8 and TR4.8). The source is chosen once, before any connection opens, and a session never falls back from one source to another. Through a server, the window is the one visible to the host's logon session, which every Windows session must share (below). A Pageant pipe that `SSH_AUTH_SOCK` names is a pipe like any other."
  - Line 512 becomes: "Through a server, with the switch on, the native path uses the server's view: its `SSH_AUTH_SOCK`, which must match, and the Pageant window visible to its logon session, which every Windows session's rows compare."
  - Line 513 is removed: no route decision lists the agent's keys any more (§3.19).
- **Server design §5, the must-match set.** On Windows the logon session leaves the native row and joins every session's rows, checked at `SessionOpen`, because the transport now uses it too:
  - The table gains a row after line 364: "| Every session, Windows: the logon session | the logon session, which the host reads from the client's token at the peer check | at `SessionOpen` | the transport's Pageant window and SSPI default credentials belong to it (amendment 2's TR4.8 and TR4.10), as do the native path's (agent.c:436-439; winhttp.c:184-212) |".
  - Line 365's values cell reads "`SSH_AUTH_SOCK`".
  - A client in another logon session is refused at open with `server_environment_mismatch`, by §17's existing "runs in a different Windows logon session" message. The `auto` key already covers it (§4), so no `auto` client meets the refusal.
  - Line 470's "The native row compares the logon session" reads "Every Windows session's rows compare the logon session", and line 812's Windows cell lists the logon session among every session's values.
- **Server design §11, §12, §14 and §17:**
  - Line 813's Windows cell, "SSH agent, transport build", reads "A visible Pageant window first, through the transport's own Pageant protocol (amendment 2's TR4.8); otherwise `SSH_AUTH_SOCK` from the snapshot, else the Windows OpenSSH agent's pipe, `\\.\pipe\openssh-ssh-agent`."
  - Line 926's row becomes: "**Windows' logon session:** a client in another logon session of the same user is refused at `SessionOpen`, naming the logon session, before any operation runs."
  - Line 987 reads "the transport build's SSH tests against the Windows OpenSSH agent, and against Pageant through TR1.8's fixture (amendment 2's TR4.8);".
  - Line 996 drops "except Pageant's" and its sentence "Pageant is not in the release's scope until a fixture proves it (plan TR1.3).".
  - Line 1007's row reads "| Pageant, a visible window first | dabeest: TR1.8's Pageant fixture (amendment 2's TR4.8) |".
  - Line 1044's risk reads "with the switch on, a client whose agent differs from the server's is refused at open, and on Windows a client whose logon session differs is refused at open" in place of "a client whose agent, or on Windows whose logon session, differs from the server's is refused for routed operations".
  - Line 1048's risk reads "on Windows" for "on Windows' native path", since the transport now uses the window too, and its last sentence, on OD11's route, is removed (§3.19).
  - Line 1348's "and Pageant is used only when `SSH_AUTH_SOCK` names its pipe" reads "and on Windows a visible Pageant window is the agent source first, as §5's Pageant bullet says".
  - Line 1499's closure row reads "| Plan TR1.3 | One agent source per session; on Windows a visible Pageant window first (amendment 2 §3.18) | §5 "One agent source per session"; §11; §12 cells; §17 |".
- **Session plan:**
  - CS8.1's "the libgit2 network timeout, and the native row, `SSH_AUTH_SOCK` and on Windows the logon session" reads "the libgit2 network timeout, on Windows the logon session as an every-session row, and the native row, `SSH_AUTH_SOCK`".
  - CS8.18's "the logon session read from the client's token at the peer check, for the native row" reads "the logon session read from the client's token at the peer check, for every session's rows".
  - CS8.18's test row "a client in another logon session has its transport operations run and a routed operation refused before any connection opens, naming the logon session" reads "a client in another logon session refused at `SessionOpen`, naming the logon session".
  - CS8.19's "Pageant only when `SSH_AUTH_SOCK` names its pipe" reads "a visible Pageant window first, then the snapshot's `SSH_AUTH_SOCK` or the default pipe, as server §5's Pageant bullet now says". The Pageant window protocol itself is TR4.8's, in 1.1.0.
- **Reuse design:** no sentence changes. Its agent-source field already holds "on Windows the session's one agent source, as kind and address", and a Pageant window is one kind. An HTTPS connection authenticated with the logon session's credentials serves only that logon session's sessions, which the must-match row above ensures within one server.

### 3.19 The operator's decisions of 2026-10-02: OD16, and OD10 and OD11 on the transport

The operator's words: "lift that", for OD16's zone bound; and "yes", for moving OD10's and OD11's cases onto the transport. With OD15, no configuration that 1.0.17 serves takes a native route in 1.1.0. Only the off switch, a caller without a host context (TR2.11), and `git://`, `http://` and `file://` remotes use libgit2's own transports.

- **OD16: no zone bound.** OD16 records the decision, and TR1.8, TR4.10, §3.2, OD13, S7.2 (1.1.0), S7.3 (1.1.0) and S7.5 (1.1.0) carry it.
- **OD10 reversed: credential helpers on the transport.** TR1.6 no longer chooses between a native route and a refusal. It designs how the transport answers HTTPS authentication through the user's configured credential helpers. `gh`'s route becomes one helper among them.
  - **The mechanism is the session plan's CS3.4,** brought into 1.1.0 for the transport, as TR3.4 brought S6.2's pins. It is one `git credential fill` spawn, which needs `git` on `PATH`:
    - its environment is `env_clear()` plus the environment the transport's runtime was built from (under 1.1.0 S6.1's variant, the snapshot), less `GIT_ASKPASS`, `SSH_ASKPASS`, `GIT_DIR`, `GIT_COMMON_DIR` and `GIT_WORK_TREE`, with `GIT_TERMINAL_PROMPT=0` and `-c credential.interactive=false`;
    - its working directory is explicit, `/` on POSIX and the system drive's root on Windows, so it reads no repository's local configuration;
    - it is bounded in time, and killed at the bound, on drop and on cancellation;
    - its output is secret, and only `username` and `password` are read.

    These are CS3.4's rules, with its addendum C10 and its working rule C11. CS3.4's change to the native path's own callback stays 1.2.0's, under rule (e).
  - **The parity reference** is 1.0.17's callback through git2's `Cred::credential_helper` (`transport_support.rs:265-271`). It reads `credential.helper`, `credential.<url>.helper`, the username keys and `useHttpPath`, and runs `get` only. TR1.6 records each difference between git's own helper matching and git2's, and S7.2's notes list them.
  - **The URL a helper is asked for** is the URL the credential will be sent to, after the transport's one validated discovery redirect. A redirect after the challenge ends the operation, with no credential sent. A fixture row redirects discovery from one host to another, and asserts that the helper is asked for the second.
  - **The policy** that turns configured helpers off (`CredentialHelperPolicy`) applies as 1.0.17 has it.
  - **Schemes.** A helper's credential answers `Basic`. On Windows it also answers `Negotiate`, `NTLM` and `Digest`, as 1.0.17's WinHTTP does with a helper's user and password (`winhttp.c:139-151`, `:618-640`). TR1.8 designs that part. On macOS and Linux, 1.0.17 serves neither `NTLM` nor `Negotiate`, since libgit2-sys defines no `GIT_NTLM` or `GIT_GSSAPI`, so the transport need not.

  TR1.6's review stands: dual Consistency and Safety, plus Surface. Its "amendment to three texts" now runs in this direction: the HTTPS design's §4; the transport design's §11 HTTPS cell, whose "other helpers rejected" reads "configured helpers, through `git credential fill` (TR1.6)"; and 1.1.0 S7.1's "gh-only authenticated HTTPS". In the plan:
  - line 52's row reads, in this release, "Private HTTPS authenticated through the user's configured credential helpers, `gh` among them (TR1.6)", and as unsupported "—";
  - OD10 (lines 533–534) gains: "Reversed by the operator on 2026-10-02: the transport runs the configured helpers itself (TR1.6)";
  - TR2.2's "The route for non-gh credentials follows TR1.6" (line 293) reads "Non-gh credentials follow TR1.6's design, on the transport";
  - line 423's "the non-gh credential route" leaves 1.2.0's ledger list.
- **OD11 reversed: every key type and signature 1.0.17 uses.** TR2.8 (amendment 1's §3.5) becomes **TR2.8: SSH keys and signatures as 1.0.17 uses them (OD11)** *(under 500 lines; a second step if its list needs one)*:
  - **The list.** The transport authenticates with every agent key type and signature algorithm that 1.0.17's agent authentication, through libssh2 1.11.1, uses. TR2.8 lists them from libssh2's source (`userauth.c`, `agent.c`) before writing code, per platform: libssh2's WinCNG build on Windows lacks Ed25519 and, without `LIBSSH2_ECDSA_WINCNG`, ECDSA (`wincng.h:74, 88-91`), so 1.0.17's list there is shorter, and the transport's list is at least 1.0.17's on each platform. At least:
    - Ed25519;
    - RSA with `rsa-sha2-512` and `rsa-sha2-256`, and with `ssh-rsa` (SHA-1) exactly where libssh2 uses it, which the operator confirmed on 2026-10-02 ("keep sha-1 fallback"): when the server sent no `server-sig-algs`; when `server-sig-algs` lists `ssh-rsa` and no `rsa-sha2-*`; and, once per key, when the agent answers a `rsa-sha2-*` request with an `ssh-rsa` signature. When `server-sig-algs` lists none of the three, the key fails, as libssh2's does (`userauth.c:1380-1383`, `:1503-1506`, `:1754-1763`; `openssl.c:5213-5218`; `agent.c:560-563`);
    - ECDSA P-256, P-384 and P-521;
    - security keys (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`), whose touch the agent asks for;
    - OpenSSH certificates of those types;
    - DSA, if libssh2 1.11.1 offers it.
  - A key of a type neither supports is skipped, as libssh2 skips it, and a later key is tried. No remote takes a native route for its keys, so the route check before any open goes.
  - TR2.8 also records whether the transport's key files for `--identity` and `--remote-identity` cover the formats libssh2 reads, and closes any gap the same way.
  - **Tests,** against the disposable SSH fixture and a disposable `sshd`: each listed type authenticates on the transport; a security key through a software authenticator the fixture provides, which TR2.8 names; a certificate against an `sshd` that trusts a test user CA; and, for `ssh-rsa`, one row for each of the three cases above, with `rsa-sha2-*` used whenever `server-sig-algs` lists it. TR2.8's Safety review names the SHA-1 cases, and that `server-sig-algs` travels after the key exchange, so an attacker on the path cannot strip it. Amendment 1's §4 manual security-key row runs on the transport, with a hardware key, and needs the operator's go.
  - **Amendment 1's texts:**
    - §3.8's "S7.2's route ledger gains a row for OD11's route" (line 264) is removed;
    - §3.2's row reads, in this release, "SSH through an agent with any key type 1.0.17 uses (TR2.8)", and as unsupported "—";
    - §3.11's OD11 gains: "Reversed by the operator on 2026-10-02: the transport signs with every key type and signature 1.0.17 uses (TR2.8)";
    - §3.12's risk "Security-key users lose the transport" and §3.13's bullet "Admitting security keys or certificates to the transport's own signing" are removed;
    - §3.14's bullet "RSA with SHA-1" is replaced by TR2.8's rule above;
    - §3.7's cell covers every type and algorithm TR2.8 admits;
    - §3.8's notes item "the key types that take the native route, and whether the native path authenticates with each" reads "the key types and signature algorithms the transport uses (TR2.8)", and "that the transport signs RSA only with SHA-2 (§3.14)" reads "that the transport signs RSA with SHA-1 exactly where libssh2 does, in TR2.8's three cases, as 1.0.17 does";
    - §4's rows on the native path's key types record the transport's instead.
  - **The agent design's** §5 sentence, as amendment 1 left it, reads: "Fixtures for every key type and signature algorithm the transport plan's TR2.8 lists are required. A key of any other type is skipped, and a later key is tried."
- **Routed native operations go (1.2.0).** Amendment 1's §3.4 sentence on TR1.6's and OD11's routes through a server, and its §3.6 Phase 7 exit rows for them, are removed. In the server design:
  - lines 476–478 become: "**With the switch off,** only remotes libgit2 always handles take the native path (below)." The route decision, its agent listing and helper check, and its refusal at routing go;
  - line 484's alternative for the `auto` key no longer refuses anything with the switch off, so the cost argument against it is gone. The server design's next revision chooses between the two before 1.2.0's Phase 7; until then line 482's recommendation stands;
  - lines 916–917, the Phase 7 exit rows, become: "With the switch on, at an explicit address: a server and a client with different `SSH_AUTH_SOCK` values, the session refused at `SessionOpen`; on Linux, different `SSL_CERT_FILE` values, refused at `SessionOpen`, as every session's row";
  - line 1435's routing message is removed. The timeout message (line 1436) stays, for `git://` and `http://` remotes;
  - line 1005's "the `SSH_AUTH_SOCK` rows with the switch on and off" reads "the `SSH_AUTH_SOCK` row with the switch on", and §18's row at line 1495, "switch off, the route-time check and its message", reads "switch off, only libgit2's own remotes".

  §3.18 already gives lines 512, 513, 1044 and 1048 their final text. In the session plan, CS8.3 implements only the timeout row's check for `git://` and `http://` remotes:
  - its routing rule drops TR1.6's and OD11's routes;
  - its test rows for them give way to the Phase 7 rows above;
  - its "TR2.8 row" and "TR1.6 row" dependencies go, with the sketch's edges "TR2.8 ── CS8.3's TR2.8 row" and "TR1.6 + TR2.2 ── CS8.3's TR1.6 row".
- **Reuse design.** Line 120's and line 385's sentences on presence keys and certificates read: "TR2.8 admits them, so they reach transport connections." A reused connection authenticated with a security key serves later operations without a new touch, as an SSH control master does. The reuse design's next revision decides, before 1.2.0's Phase 6, whether a presence key is revalidated once per operation, as its §15.2(b) does for a confirm-required key.

### 3.20 What implementation added, and the operator's decisions of 2026-10-02 (revision 6)

Phase 2's implementation found defects and dead code the plan did not foresee. The operator also decided fourteen follow-up questions on 2026-10-02 ("all as recommended"). This section records both. Each step below was or will be tested in its own lane, merged into `main`, and covered by TR2.6's review.

**Steps added to Phase 2:**
- **TR2.13, endpoint throughput** (done, gwz-core `67c680a7`). Every hop between libgit2 and the network polled on a fixed timer and moved one message a pass, which capped a session near 2 MiB/s. Each pass now moves every ready message, within the existing bounds, and work arriving wakes the session. A 32 MiB clone takes 462 ms over SSH against 15.7 s before, level with libgit2. Tests hold the transport within 1.5 times libgit2's time plus 1 s.
- **TR2.14, open admission without job permits** (done with the concurrency fix, gwz-core `ab5fee76`). TR2.10's 32 concurrent opens exhausted the 64 supervised-job permits and the key registry. An open now holds no job permit while it waits for the worker, and an identity check or key read waits for room instead of failing.
- **TR2.15, the dead blocking SSH open path** (done, gwz-core `9d4dd92f`). It is removed, the module-level `allow(dead_code)` that hid it is gone, and the `tests/transport_ssh` crate is folded into gwz-core's candidate tests.
- **TR2.16, SSH destination parity** (done, gwz-core `ff2bf4f2`). The transport parses SSH destinations exactly as libgit2 1.9.7 does, and refuses before any open what libgit2 refuses before connecting.
- **TR2.17, prompt cancel and prompt permit release** (done, gwz-core `a6cc1737`). A cancel returns in about 14 ms instead of waiting out the 5 s cleanup bound, and a job's permit is freed when its result is taken.
- **TR2.18, SSH parity follow-ups:**
  - a password in an SSH URL is used exactly where 1.0.17's libgit2 uses it (decision 5);
  - `known_hosts` host names match case-insensitively, which serves both of 1.0.17's cases (decision 7);
  - control characters in an SSH path stay refused (decision 6): the one deliberate difference from 1.0.17 here, which S7.2 (1.1.0)'s notes list.
- **TR2.19, the error contract and dead API:**
  - TR2.3's rule widens to `Failed` and `Rejected` aggregates, so every non-success result lists each failed or rejected member's error in `errors` (decision 8);
  - gwz-core's `OperationRuntime`, `ResponseBuilder::result` and `ExecutionReport`, which nothing called outside their own tests, are removed (decision 9). S7.2 (1.1.0)'s notes name each removed public path.
  - the widened contract and the removed paths are S7.5 (1.1.0)'s Surface items, under the plan's §6 rule (c). gwz-py, which pins gwz-core exactly (TR3.4), and gwz-cli are gwz-core's only known consumers, and neither uses the removed paths.
- **TR2.20, HTTPS tests onto the production mode** (after the split). About 100 HTTPS test call sites use worker wrappers that run a mode production never uses (`allow_transition = false`). They move onto `prepare_budget_for_transition`, and the test-only mode is removed (decision 12).
- **TR2.21, gwz-py's candidate CI.** A gwz-py job builds the candidate extension with `scripts/build_candidate_extension.py` and runs the Python suite with its transport rows, and gwz-py's Rust unit tests run in CI (decision 13).
- **TR2.22, credential helpers on the transport.** TR2.2 still reproduces and fixes defect 1, the two `gh` helper forms, now. TR2.22 follows it: after TR1.6's GO it implements that design (§3.19) for every configured helper, and keeps TR2.2's fixes and regression tests. It is under 500 lines, or two steps.
- **The split of `placement_endpoint.rs`,** movement only (decision 3). It is brought forward from the session plan's CS7.1, which keeps its other files.
- **S6.2's follow-ups** (done, gwz-py `766de53`):
  - gwz-py's model errors are built lazily, so a failure after the exit bound no longer touches a finalizing interpreter;
  - a request ID matching a live operation is refused, and one matching an ended operation gets a fresh record (the contract's §4.3);
  - the candidate extension has a committed build recipe.

**OD18. The cold start** (decided 2026-10-02, decision 1). In 1.1.0 every command starts its keys Cold, since there is no reuse across commands. The retry plan's single Cold setup would add a full SSH login ahead of every command's other members, and TR8.1 compares against 1.0.17, which opens its connections at once. So:
- **The retry plan's §5 "Cold" state reads:** "No successful setup yet for this operation. The first wave of setups starts in parallel, up to the operation's per-host limit, as 1.0.17's connections do; members beyond the limit wait. The wave is the key's first attempt. When a setup of the wave fails retriably, that attempt is counted, its member returns to the key's queue, the key leaves Cold, and every later setup follows the single-probe rules below. A success that comes before any failure marks the key Healthy; the wave's setups still in flight are then Healthy's own, and §5's Healthy and Degraded rules govern them. Setups of the wave still in flight once the key has left Cold other than for Healthy complete as Degraded's in-flight setups do: a success completes its member and does not change the key's state; a retriable failure counts no attempt and returns its member to the key's queue; a non-retriable failure puts the key in Closed as §4 says."
- **Its S3.1 sentences read:**
  - "Thirty-two cold members on one dead key open one handshake at a time and exactly four handshakes in total" becomes "Thirty-two cold members on one dead key open a first wave at the per-host limit, then one handshake at a time, and at most the per-host limit plus three handshakes in total".
  - "An authentication failure does not wait and does not open a second handshake for the queued members" becomes "… for the members not already in the first wave".
- **Its §4 sentence** "That stops `--jobs 1` from running a fresh budget for each member, and it stops an authentication failure from becoming a series of handshakes" becomes "…, and, beyond the first wave, it stops an authentication failure from becoming a series of handshakes".
- **The Safety finding it reverses.** The single Cold probe closed the retry plan's Safety review `[P2-4]`, the initial connection stampede (`GwzRemoteTransportRetryPlan-ReviewSafety.md`). OD18 reverses that probe. The first wave, at most the per-host limit and counted as one attempt, followed by single probes, is bounded, but it is not the "bounded mechanism with equivalent safety" that finding's correction asks for: on a dead key it sends the whole wave at once. The operator accepts that difference, the hazard below, for 1.0.17 parity.
- **A server's `MaxStartups`.** A stock OpenSSH server starts dropping new unauthenticated connections beyond its `MaxStartups` start value, 10 by default (`10:30:100`). TR2.1 classifies such a drop as a retriable setup failure, and its tests include it. They pin the error kinds each platform reports for the drop: `ssh_setup.rs` maps `UnexpectedEof` and `ConnectionReset` to `Io`, which is retriable, but `ConnectionAborted` to `Cancelled`, which is not.
- **The hazard,** which the migration notes state: on a dead key or bad credentials, the first wave sends up to the per-host limit of failing handshakes at once, as 1.0.17 does, before Closed stops the rest.
- TR2.10's 32-open test holds under this rule. TR2.1 implements it.

**Decision 14, a known limitation.** gwz-py's `dispatch::record` still formats a failed submit's error through the interpreter when its host is not exiting. A network operation that ends just before the exit hook runs, or a submitted non-network operation such as `clone_local_workspace`, can therefore still touch a finalizing interpreter. 1.2.0's session host replaces that code, so S7.2 (1.1.0)'s notes list this as a known 1.1.0 limitation.

**TR2.6's scope** gains TR2.13–TR2.22, the split and S6.2's follow-ups.

**The sketch and "What can start now"** (§3.13) gain the following, and TR2.1 leaves the second bullet of "What can start now", since its machine lives in the file being split:
- the split, then TR2.1 and TR2.20;
- TR2.18, TR2.19 and TR2.21 now;
- TR2.2 and TR1.6's GO, then TR2.22;
- in the sketch only: TR2.1 ── TR8.1 (1.1.0) and TR8.4, so both measurements against 1.0.17 run with the retry machine and OD18's first wave; and TR2.13–TR2.22, the split and S6.2's follow-ups ── TR2.6.

**Retry plan:** `GwzRemoteTransportRetryPlan.md`'s status gains: "Amended 2026-10-02 by `GwzTransportReleasePlanAmendment-2.md` revision 6. Its §4 sentence on what Closed stops, its §5 Cold state and two S3.1 test sentences read as that amendment's §3.20 (OD18) states."

**Reuse design (1.2.0).** Its §9 (line 200), which the retry plan's 2026-09-28 amendment makes §5's Cold state for the transport host, and the session plan's CS7.23 still say that only new setups wait for "the one probe". This revision does not amend them. The reuse design's next revision carries OD18's first wave into its §9 and CS7.23 before CS7.23 starts, as §3.19 leaves that revision the presence-key question.

### 3.21 The crates.io names, gwz-transport's release mechanism and the publication order (revision 7)

**Status: DRAFT, not yet reviewed.** This section records three changes that the program checkpoint's entries of 2026-10-05 say this amendment owes (the checkpoint's [entry on gwz-transport](../../dev-docs/CurrentProgramCheckpoint.md) says "amendment 2's revision 7 records this as Phase 10 step 2's mechanism", and its entry on gwz-sspi says the fourteenth name "is owed in amendment 2's revision 7"). It records no new operator decision. Where the checkpoint leaves a point unsettled, the point is listed as open at the end of the section. Nothing here authorizes an implementation, a commit, a tag, a push or a publish.

**A. TR3.3: thirteen names become fourteen with gwz-sspi.**

*Source:* the checkpoint's entry "gwz-sspi registered on crates.io, trusted publishing only, 2026-10-05"; root `97b57f7d`, and its correction in root `c6c427c9`; gwz-sspi `4a24d94` and `8078278`.

- **Why.** gwz-cli's release publishes to crates.io (cargo-dist's `publish-crate` job), and gwz-cli requires `gwz-sspi =0.1.0`, which was on no registry.
- **What was done.** The operator ran gwz-sspi's bootstrap workflow once, with a token (run 37207954643). The placeholder `gwz-sspi 0.0.0-bootstrap.1`, a prerelease with no implementation and no dependencies, which no ordinary version requirement selects, has been on crates.io since 2026-10-04 14:05 UTC. The operator then deleted the secret and set the crate to trusted publishing only: owner owebeeone, repository gwz-sspi, workflow `release.yml`. The publishing workflow is named `release.yml`, as in gwz-core and gwz-cli.
- **What stays.** The real crate keeps `publish = false`. gwz-sspi's `release_checks.py` refuses to publish until its reviewed activation, after implementation acceptance and Windows qualification (gwz-sspi's `RELEASE.md`).
- **The ruling of 2026-09-28 is unchanged for the thirteen names the crate map lists.** gwz-sspi is not on that list: it was registered on its own, early, for the reason above, and not "together, just before release preparation". This revision records that fact. It does not rule on whether the "together" rule is to be read as covering it, because the checkpoint records no such ruling.

Old text, TR3.3's first sub-bullet:
> The operator's ruling of 2026-09-28 stands: the thirteen names the crate map lists are registered together, just before release preparation. For 1.1.0, that is before its Phase 10.

New text:
> The operator's ruling of 2026-09-28 stands for the thirteen names the crate map lists: they are registered together, just before release preparation. For 1.1.0, that is before its Phase 10. A fourteenth name, `gwz-sspi`, was registered on its own on 2026-10-04 (§3.21), so TR3.3's registry steps cover thirteen names to register and one already registered.

Old text, TR3.3's third sub-bullet, last two sentences:
> gwz-transport, already bootstrapped, is released by Phase 10 (1.1.0) step 2. The other names stay placeholders until 1.2.0 publishes them.

New text:
> gwz-transport, already bootstrapped, is released by Phase 10 (1.1.0) step 2. `gwz-sspi`, already bootstrapped, publishes its real 0.1.0 at its reviewed activation, which Phase 10 (1.1.0) needs before step 6 (§3.21, part C). The other names stay placeholders until 1.2.0 publishes them.

Old text, TR3.3's exit:
> **Exit:** the thirteen names visible on crates.io, with trusted publishers configured for the workflows that publish them, recorded in the checkpoint before Phase 10 (1.1.0).

New text:
> **Exit:** the fourteen names visible on crates.io, with trusted publishers configured for the workflows that publish them, recorded in the checkpoint before Phase 10 (1.1.0).

State of the exit at this revision, from the checkpoint:
- gwz-sspi: visible (placeholder). Trusted publisher configured, with the environment unconfirmed (open item O1).
- gwz-transport: visible (bootstrapped before this amendment). Trusted publisher configured by the operator on 2026-10-05: owner owebeeone (GitHub ID 366621, verified), repository gwz-transport, workflow `release.yml`, and no environment, by the operator's choice ("no crates-io environment"). The checkpoint says this covers TR3.3's exit for gwz-transport.
- The other twelve names: this revision records nothing about them, because the checkpoint's entries it draws on say nothing.

The crate map's sentence that reads "Thirteen names need their one-time bootstrap ([CrateBootstrapHowTo.md](CrateBootstrapHowTo.md)): the six ordinary crates, the six candidate crates and gwz-transport." (its line 164 as the file stands at this draft) reads, with this amendment: "Fourteen names need their one-time bootstrap, …: the six ordinary crates, the six candidate crates, gwz-transport and gwz-sspi. gwz-sspi's was run by its own workflow (`bootstrap-crate.yml`)." The crate map's decision 3, "When to bootstrap the thirteen crates.io names: decided 2026-09-28", and its dated review rows stay as written, as the record of the decision.

**B. Phase 10 step 2: gwz-transport releases with Gearu, by Trusted Publishing.**

*Source:* the checkpoint's entry "gwz-transport releases with Gearu, 2026-10-05"; gwz-transport `2c12922e` and `ff6083b`; root `977b795c` and `e124244d`.

Step 2 of Phase 10 (1.1.0), as the 1.1.0 plan's Phase 8 left it and §3.12 and the plan's Phase 10 bullets amend it, already reads "`gwz-transport`. `gearu release <version> --push --github-release`", and "Step 2's gwz-transport release includes … TR2.4's feature" (§3.12 adds TR2.9's change if it lands there). Those sentences are unchanged. This revision adds the mechanism step 2 runs, which gwz-transport now carries:

- **The operator's decision (2026-10-05):** "use gearu for gwz-transport too - core, cli and py need a dependency check that gearu lacks". gwz-core, gwz-cli and gwz-py keep `scripts/release.py`, so steps 5, 6 and 7 are unchanged in mechanism.
- **Configuration.** `gearu init` added its managed sections to gwz-transport's `AGENTS.md` and `RELEASE.md`. `gearu.toml` takes the version from `Cargo.toml`, regenerates the lock offline (safe, because gwz-transport has no dependencies), and runs `scripts/release_checks.py` under `{repo}/.release-venv`, which holds the pinned taut-proto release that `regen.py` requires.
- **`scripts/release_checks.py`** has two stages, with six unit tests:
  - a candidate stage, which runs the contracts job's gates;
  - an exact stage, which packages the release commit, because `cargo package` refuses Gearu's uncommitted candidate.

  It refuses while `publish = false` stands, because 0.1.0 is Phase 10's step 2. The 1.1.0 plan's S2.1 holds that `publish = false` remains until the package's release commit.
- **Publication.** `.github/workflows/release.yml` runs on a published GitHub Release, reruns both stages on the tag, then publishes through crates.io Trusted Publishing only, in no GitHub environment (`ff6083b`). The operator chose no environment. `2c12922e`'s own message names `crates-io`, which `ff6083b` then removes, because the publisher the operator configured names none.
- **Operator step before step 2.** Create the release venv once, as gwz-transport's `RELEASE.md` says.
- **Verified** (per the checkpoint): both stages passed end to end on a copy with the guard lifted (12 script tests, 4 generated artifacts, formatting, both suites, 81 files packaged), and `gearu plan 0.1.0` accepts the configuration. Those runs were on a copy, not on the release commit.

Old text, the plan's Phase 10 bullet (the 1.1.0 plan's step 1 sentence it replaces):
> Step 1's sentence "The first publish of each new name uses the operator-held token from S2.3" is replaced: each name publishes through its trusted publisher (Phase 3).

New text: this bullet stands. It now also applies to gwz-transport's step 2 as the paragraph above states, and `gwz-sspi`'s publisher is named in part A.

**C. The publication order: gwz-sspi 0.1.0 first, for gwz-cli.**

*Source:* the checkpoint's entry on gwz-sspi ("gwz-cli's crates.io publication of 1.1.0 waits on gwz-sspi 0.1.0"; "gwz-cli now requires gwz-sspi `=0.1.0`"). The `gwz-sspi` pin in `gwz-cli/Cargo.toml` and `gwz-cli/scripts/release.py`, which rewrites it to the registry form `gwz-sspi = "=0.1.0"`, were read for this draft.

Old text, Phase 10 (1.1.0) steps 5 to 7, as the 1.1.0 plan's Phase 8 left them and the plan's Phase 10 adopts them:
> Product repositories, each with the existing release script, tag `v1.1.0`. Each waits until the previous product crate is visible. Pins are registry versions, not git pins and not sibling paths.
>
> 5. `gwz-core`: `scripts/release.py v1.1.0 --push`. The release commit pins `gwz-transport` and the new `git2` API crate by the versions just published. Internal gwz-core crates bump the way the 1.0.12 script bumped them.
> 6. `gwz-cli`: `scripts/release.py v1.1.0 --push`. The `release` branch pin becomes `gwz-core = "=1.1.0"`.
> 7. `gwz-py`: `scripts/release.py v1.1.0 --push`, after `gwz-core` 1.1.0 and the binding's publishing step (step 2 or step 3) are on their registries. Wheels use the pins S6.2 wrote into `RELEASE.md`.

New text. Steps 5 and 7 and the opening paragraph are unchanged; a step is added after step 5, and step 6 gains one sentence:
> 5a. `gwz-sspi`: its reviewed activation lifts `publish = false`, then `gearu release 0.1.0 --push --github-release` (its `RELEASE.md`'s order), published by `release.yml` through Trusted Publishing. It depends on no other repository of this release (its dependencies are `zeroize` and, on Windows, `windows-sys`, both from crates.io), so it can also run earlier, beside steps 1 and 2. Its position is fixed only by step 6: 0.1.0 must be visible on crates.io before step 6.
> 6. `gwz-cli`: `scripts/release.py v1.1.0 --push`. The `release` branch pin becomes `gwz-core = "=1.1.0"`. It also waits until `gwz-sspi` 0.1.0 is visible on crates.io: the release branch's `gwz-sspi = "=0.1.0"` resolves from the registry, and cargo-dist's `publish-crate` job publishes gwz-cli to crates.io.

Step 5a is a new prerequisite of Phase 10 (1.1.0): the activation and its review (gwz-sspi's `RELEASE.md`: after implementation acceptance and Windows qualification) must be done first. Whether the activation's review is a dual review, and when it happens, are not decided here (open item O3). This revision adds no step to the plan's other phases. Its row in the plan's sketch is step 5a ── step 6.

**Open at this revision:**

- **O1. gwz-sspi's trusted-publisher environment.** The checkpoint's correction of 2026-10-05: the setup steps asked for environment `crates-io`, and gwz-sspi's `release.yml` runs in it, but the operator has not confirmed that the publisher names it. If the publisher names none, publishing still works, but gwz-sspi's `RELEASE.md` overstates the restriction. TR3.3's exit records the answer. gwz-transport's case is settled (no environment).
- **O2. gwz-py's wait on gwz-sspi.** The checkpoint names only gwz-cli. gwz-py's `Cargo.toml` has the same `gwz-sspi = { path = "../gwz-sspi", version = "=0.1.0" }` dependency, and its `scripts/release.py` rewrites it to `"=0.1.0"` from the registry, so step 7 appears to wait on 0.1.0 too. The checkpoint's follow-up also says gwz-cli's manual platform gate and release workflows, and gwz-py's `publish.yml`, have no gwz-sspi checkout yet. This revision does not change step 7. A later revision, or the lane owner, settles both.
- **O3. gwz-sspi 0.1.0's activation.** Its review route and date are not in the checkpoint. 1.1.0's Phase 10 cannot reach step 6 without it.
- **O4. Whether the "together" registration ruling covers gwz-sspi** (part A). The checkpoint records no ruling.
- **O5. Owed to this revision by older checkpoint entries, and not drafted here.** Entries of 2026-10-02 say amendment 2's "next revision" carries: TR2.23, credential helpers for an SSH server that offers only `password`, reusing TR1.6's lookup after its GO (a parity gap found in TR2.18); a consumer-build row in S7.3 (1.1.0) for TR1.5's OQ4; an erratum to the retry plan's §8 for the `--ssh-timeout` help text; and the TR1.6 design's items C2 (what "one validated discovery redirect" means), C3 (texts outside §3.19's list, with status lines on those documents) and C7 (TR2.22's reading of TR2.2's fixes). Each is a decision or a text of its own, and the checkpoint does not word them. They need their own revision, or the lane owner's instruction to include them here.

## 4. Affected tests and evidence

- **Phase 2:** TR2.9's to TR2.12's tests, including TR2.12's two CI legs and each repository's inventory test; 1.1.0 S6.1's and S6.3's tests, with the design's §3 rows (§3.17); TR2.8's key-type and signature tests, and TR2.2's tests under TR1.6's helper design (§3.19); and TR2.6's review of them.
- **Phase 2, from revision 6:** the tests of TR2.13–TR2.22 and of S6.2's follow-ups, inside their steps; OD18's first-wave and `MaxStartups` rows in TR2.1; TR2.21's CI jobs.
- **Phase 3:** TR3.4's tests; TR3.3's registry record, which from revision 7 covers fourteen names (§3.21).
- **Phase 4:** TR4.6's job, 1.1.0 S4.2–S4.5's tests on dabeest, TR4.8–TR4.10's tests against TR1.8's Pageant, proxy and `Negotiate` fixtures, TR1.8's `HOME`-unset rows and its macOS and Linux 1.0.17 rows, TR4.10's review, and TR4.7's review.
- **Phase 8:** TR8.1 (1.1.0) on macOS and Linux, and TR8.4 on dabeest, both against 1.0.17, with their off-switch rows, and S5.5 (1.1.0)'s repeat; S5.6's cells for the behaviours TR1.8 designs.
- **Phase 9:** the completeness check for `gwz_transport_candidate` and the inventory check for `gwz_session_candidate` at S7.1 (1.1.0); the completeness check for `gwz_session_candidate` at S7.1 (1.2.0); S7.3 (1.1.0)'s route checks, gwz-py's transport-route rows, the native-route row of a caller without a host context, and the absence checks.
- **Phase 10:** the Windows precondition's run, and the post-release check's rows. From revision 7: gwz-transport's `release_checks.py` stages, which `release.yml` reruns on the tag, and the check that gwz-sspi 0.1.0 is visible on crates.io before steps 6 and 7 (§3.21).
- **Evidence rules.** §2's redaction and live-account rules apply to every new row. TR1.8's 1.0.17 rows and the live fetches of TR8.1 and TR8.4 need the operator's go.

## 5. Review and application

- **Review.** Dual peer-blind Consistency and Safety review of this draft's text, identified by its SHA-256. There is no Surface review of this amendment, because it freezes no command, option or API. TR1.8 and S7.5 (1.1.0) carry Surface. Revision 3 was skim-reviewed only, on the operator's instruction ([skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md)); S7.5 (1.1.0)'s Surface covers gwz-py's per-operation semantics. Revisions 4 and 5, which apply OD15 and then OD16 with OD10's and OD11's reversal, were skim-reviewed in the same way ([skim review 2](GwzTransportReleasePlanAmendment-2-ReviewSkim-2.md), [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-3.md)), and so was revision 6 ([skim review 7](GwzTransportReleasePlanAmendment-2-ReviewSkim-7.md)). The Windows designs themselves get TR1.8's dual and Surface review, and TR4.10 its own dual review.
- **On GO,** these edits follow under AgentProcessRules §7.2, each with a changelog entry:
  - **`GwzTransportReleasePlan.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for its status block's decision record, §1's name, outcome, decisions and authorities, §2's table, §4's adopted table and closing condition, each phase's release, Phases 1–4 and 8–10 as that amendment restricts them, §6, §7, §8 and §9."
  - **`GwzTransportReleasePlanAmendment.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for the release each of its sections belongs to, and, from revision 5, for its OD11 texts (§3.19)."
  - **`GwzV110PlanAmendment.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for its §3.4 (Phase 6) and its §3.5 and §3.6 sentences on gwz-py, in 1.1.0's run."
  - **`GwzCoreSessionPlan.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for §1.1, §2.3's G2, §2.4's candidate-build sentence, §3.0's marker, S6.2's release pins in §5.3 and CS5.3, its mentions of gwz-py's `TransportSession`, and the lazy endpoint's retirement in CS6.5, CS7.24, §5.4 and Phase 6's preamble, and, from revision 4, CS8.1's and CS8.18's sentences on the logon session, CS8.18's logon-session test row, and CS8.19's Pageant clause, and, from revision 5, CS8.3's routing rule and CS3.4's transport half (§3.15, §3.19)."
  - **`GwzCoreSessionCrateMap.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for the release its candidate crates are published in, and, from revision 7, its count of crates.io names (§3.21)."
  - **`GwzConnectionReuseDesign.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for its open item on network operations without a binding, in 1.1.0, the release its supersessions take effect in, and, from revision 5, its key-type sentences (§3.19)."
  - **`GwzCoreServerDesign.md`'s status** gains: "Amended <date of GO> by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for the release from which its rule "One agent source per session" governs the in-process transport, and, from revision 4, every sentence on Pageant and on the Windows logon session that §3.18 lists, and, from revision 5, the routed native operations §3.19 removes."
  - **`GwzRemoteTransportSshAgentDesign.md`'s status** gains, from revision 5: "Amended 2026-10-02 by `GwzTransportReleasePlanAmendment-2.md`. This document remains authoritative only as amended for its §5 fixture sentence (that amendment's §3.19)."
  - **`GwzRemoteTransportRetryPlan.md`'s status** gains, from revision 6: "Amended 2026-10-02 by `GwzTransportReleasePlanAmendment-2.md` revision 6. Its §4 sentence on what Closed stops, its §5 Cold state and two S3.1 test sentences read as that amendment's §3.20 (OD18) states."
  - **The program checkpoint** records the acceptance, OD14's and OD15's answers and OD16's applied shape, TR3.1's closure, and, once TR2.12 creates them, the digests of the switches' inventory files.
- **No authorization.** This amendment authorizes no implementation, commit, tag, push or publish.

## Changelog

- 2026-10-01: revision 0, drafted after the operator's OD13. Reviewed at SHA-256 `57ce8e46…`: [Consistency](GwzTransportReleasePlanAmendment-2-ReviewConsistency.md) NO-GO (6 P2, 12 P3), [Safety](GwzTransportReleasePlanAmendment-2-ReviewSafety.md) NO-GO (1 P1, 5 P2, 8 P3).
- 2026-10-01: revision 1 applies [remediation plan 1](GwzTransportReleasePlanAmendment-2-RemPlan.md): TR2.11, TR2.12 and TR3.4 are added; TR8.4 becomes its own step; TR1.8 gains the SSH home rule, the inverse proxy case, OD16's trigger and its fixtures; rules (a) and (e), S7.1 to S7.5 (1.1.0), Phase 10 (1.1.0), the session plan's G2 and the crate map's release are corrected; OD16 and OD17 are named. Re-verdicts at SHA-256 `42b91afd…`: [Consistency-1](GwzTransportReleasePlanAmendment-2-ReviewConsistency-1.md) GO (6 new P3), [Safety-1](GwzTransportReleasePlanAmendment-2-ReviewSafety-1.md) NO-GO (1 new P2, 1 new P3).
- 2026-10-01: revision 2 applies [remediation plan 2](GwzTransportReleasePlanAmendment-2-RemPlan-2.md): OD16's route is bounded to the Local Machine, Intranet and Trusted zones, on Windows only, with its effect stated and an edit list for each alternative; TR1.8 gains the two-zone fixture rows and the macOS and Linux 1.0.17 rows; TR2.12 names the process-globals checker, keeps the transport-only CI leg, and moves the inventories into per-repository files; TR3.3 names gwz-transport's release; §1 and §3.15 name the session plan's remaining lazy-endpoint sentences and §3.0's marker.
- 2026-10-01: accepted at SHA-256 `c5850e52…` (revision 2) after [Consistency-2](GwzTransportReleasePlanAmendment-2-ReviewConsistency-2.md) and [Safety-2](GwzTransportReleasePlanAmendment-2-ReviewSafety-2.md) reported GO ([verdict](GwzTransportReleasePlanAmendment-2-Verdict.md)). The corrections they cleared without a further round were then applied:
  - OD16's trigger gives way to the gh route when the challenge also offers a scheme it answers, and a failed zone lookup does not fire it (Consistency-2 P3-C);
  - OD16's refusal names the Trusted zone before the off switch, and the notes state that the off switch restores 1.0.17's default-credential offer for every host (Safety-2 P3-11);
  - each alternative's edit list now covers OD13's exception and S7.2 (1.1.0)'s notes item (Consistency-2 P3-B, Safety-2 P3-10);
  - §1, §3.15 and §5 name CS6.5's and §2.4's sentences (Consistency-2 P3-A);
  - §5's checkpoint bullet records the inventory files' digests once TR2.12 creates them, following revision 2's move of the inventories into files.
- 2026-10-01: revision 3. The operator decided OD14: the alternative, gwz-py's network operations on the per-operation transport entry in 1.1.0. §3.17 restores the 1.1.0 amendment's S6.1–S6.3 for 1.1.0, with the changes the Python [design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md) requires: the environment snapshot taken under the GIL, panic safety, one shared transport predicate, and cancellation, close and interpreter exit. §3.1, §3.2, §3.4, TR2.6, S7.1–S7.5 (1.1.0), Phase 10 (1.1.0), §3.13, OD14, §8 and §9 follow. On the operator's instruction this revision skips the review loop, and one [skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md) checks it.
- 2026-10-01: the [skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md) of revision 3 reported NO-GO, with six P2 and three P3 text defects. As the operator directed, they are applied without a further round. Here, that means: OD14 in the decision records and OD13's 1.2.0 bullet (P2-5); the S1.2 gate in §3.17, and §3.4 in the 1.1.0 amendment's status edit (P2-6); and the leftovers of P3-1. The Python design takes the rest: GIL-free waits, the per-`Client` host object and its limit, S1.1's API list, the off switch for gwz-py, and the cleanup carrier.
- 2026-10-01: the [skim re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-1.md) reported GO: all nine skim findings are closed. Its four new P3s are applied as it specified: `Client.meta(max_retries)` gets no Python form in 1.1.0; cleanup state is a running aggregate; each `ClientHost` registers its own exit callback; and TR1.5 states gwz-py's notice form (§3.17). The 1.1.0 amendment's changelog now names revision 3.
- 2026-10-02: revision 4. On 2026-10-01 the operator decided OD15 under OD13's parity, rejecting native routes as the way to reach it: the transport itself does on Windows what 1.0.17's native path does. TR1.8 now designs Pageant's window protocol, the WinHTTP machine proxy and SSPI default credentials in the transport, and TR4.8–TR4.10 implement them; OD16 keeps revision 2's zone bound, applied by the transport rather than a native route, so no route changes after an open. §3.18 changes the server design's Pageant bullet, moves the Windows logon session into every session's must-match rows, and carries the session plan's CS8.18 and CS8.19 with them. §3.1, §3.2, §3.3, §3.4, TR4.7, TR8.4, §3.6, §3.9–§3.13, OD13, §8, §3.17's S6.1, §4 and §5 follow. It skips the review loop, as revision 3 did, and one [skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim-2.md) checks it; TR1.8's and TR4.10's dual reviews carry the Windows designs.
- 2026-10-02: [skim review 2](GwzTransportReleasePlanAmendment-2-ReviewSkim-2.md) of revision 4 reported NO-GO, with four P2 and four P3 text defects. All eight are applied without a further round:
  - §3.4's step list (P2-1);
  - every server design sentence on Pageant and on the logon session, now listed in §3.18, with the new every-session row's text (P2-2, P3-1);
  - CS8.1's, CS8.3's and CS8.18's sentences on the logon session (P2-3);
  - the zone check on the URL the credentials would go to, after the discovery redirect, with a redirect fixture row (P2-4);
  - the unbounded alternative's edit list (P3-2);
  - OD10's and OD11's native routes in OD13's parity bullet (P3-3);
  - the machine proxy's own 407, and SSPI's target name, flags and channel binding, with fixture rows (P3-4).
- 2026-10-02: the [skim re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-3.md) reported GO: all eight findings of skim review 2 are closed. Its one new P3 is applied as it specified: TR1.8's Safety finding list names the default-credential exchange with the machine proxy, if the 1.0.17 row authenticates to the proxy (P3-5).
- 2026-10-02: revision 5. The operator decided OD16's unbounded alternative and reversed OD10 and OD11 onto the transport (§3.19): the transport offers the logon session's default credentials to any host, runs the user's configured credential helpers (TR1.6), and signs with every agent key type and signature algorithm 1.0.17 uses, `ssh-rsa` included for a server that offers no SHA-2 RSA algorithm (TR2.8). No configuration that 1.0.17 serves takes a native route. The status block, §1, §3.1, §3.2, TR1.8, TR4.10, the Phase 8 sign-off, S7.2, S7.3 and S7.5 (1.1.0), OD13, OD16, §3.14's §7 sentence, §3.18, §4 and §5 follow. It is skim-reviewed only.
- 2026-10-02: [skim review 4](GwzTransportReleasePlanAmendment-2-ReviewSkim-4.md) of revision 5 reported NO-GO, with five P2 and three P3 text defects. All eight are applied without a further round:
  - TR1.6 adopts the session plan's CS3.4 `git credential fill` spawn as its mechanism, with git2's semantics as the parity reference (P2-1);
  - a helper is asked for the URL the credential goes to, after the discovery redirect (P2-2);
  - on Windows a helper's credential answers `Negotiate`, `NTLM` and `Digest`, as 1.0.17 does (P2-3);
  - the `ssh-rsa` rule is libssh2's exact one, which the operator confirmed, and TR2.8 lists per platform (P2-4);
  - the release definition drops "gh-only HTTPS" (P2-5);
  - the plan's line numbers are the pinned file's (P3-1);
  - the leftovers in §3.3, §3.6, TR8.4, amendment 1's line 264 and the server design's lines 1005 and 1495 are listed (P3-2);
  - the checkpoint's and the verdict's stale sentences are fixed (P3-3).
- 2026-10-02: the [skim re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-5.md) of revision 5 closed all eight findings, and found one new P2 and three new P3s, all applied as it specified:
  - a helper answers first only when the challenge offers `NTLM`, `Basic` or `Digest`, over WinHTTP's scheme order, and a `Negotiate`-only challenge takes the logon session with no helper asked, as on 1.0.17 (P2-6);
  - the transport design's HTTPS cell names `git credential fill` (P3-4);
  - amendment 1's §3.8 notes item states TR2.8's three SHA-1 cases (P3-5);
  - CS3.4's transport half is TR1.6's in 1.1.0 (§3.15) (P3-6).
- 2026-10-02: the [second re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-6.md) reported **GO**: P2-6, P3-4, P3-5 and P3-6 are closed, with no new issue. Revision 5 carries a skim-review GO, not a dual-review GO.
- 2026-10-02: revision 6. It records the steps Phase 2's implementation added: TR2.13 throughput, TR2.14 open admission without job permits, TR2.15 the dead blocking SSH path, TR2.16 SSH destination parity, TR2.17 prompt cancel, TR2.18 SSH parity follow-ups, TR2.19 the widened error contract and dead API, TR2.20 HTTPS tests onto the production mode, TR2.21 gwz-py's candidate CI, TR2.22 credential helpers after TR1.6, and the split of `placement_endpoint.rs`. It also records S6.2's follow-ups, the operator's decisions of 2026-10-02, OD18 (the cold start's first wave in parallel, amending the retry plan's §4 sentence on what Closed stops, its §5 Cold state and two S3.1 sentences) and decision 14's known limitation (§3.20). §1, §4 and §5 follow. It is skim-reviewed only.
- 2026-10-02: [skim review 7](GwzTransportReleasePlanAmendment-2-ReviewSkim-7.md) reported **GO** on revision 6 (`32c6ae18…`), with eight P3s and no P0–P2. Six are applied as given. P3-1 is applied with one sentence reworded: the first wave is bounded, but it lacks the equivalent safety that the retry plan's Safety `[P2-4]` asked of another mechanism, and the operator accepts the difference for parity. P3-7 is applied in part: the boundary between TR2.2 and TR2.22 is added, and the change to line 507's edge is disputed, since revision 5 already removes that edge. The same reviewer's [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-8.md) (`0ac4f118…`) reported **GO**: all eight are closed and the dispute is upheld. Its two new P3s are applied: a wave setup still in flight when the key turns Healthy is governed by Healthy's rules, and §3.10 runs TR8.1 (1.1.0) and TR8.4 after TR2.1. Revision 6 carries a skim-review GO, not a dual-review GO.
- 2026-10-05: revision 7, a **DRAFT, not yet reviewed**. It records what the program checkpoint's entries of 2026-10-05 owe this amendment (§3.21): TR3.3's thirteen crates.io names become fourteen with gwz-sspi (registered on 2026-10-04 as a placeholder, trusted publishing only, its real crate still gated), with TR3.3's exit and the crate map's count following; Phase 10 step 2's mechanism for gwz-transport, which releases with Gearu (`gearu.toml`, and `release_checks.py`'s candidate and exact stages) and publishes through `release.yml` by Trusted Publishing with no GitHub environment, while gwz-core, gwz-cli and gwz-py keep `scripts/release.py`; and a new step 5a for `gwz-sspi` 0.1.0, which must be visible before gwz-cli's step 6. §1, §3.12's and TR3.3's pointers, §4 and §5's crate-map status follow. Open: gwz-sspi's publisher environment (O1), gwz-py's wait on gwz-sspi (O2), gwz-sspi 0.1.0's activation (O3), the "together" ruling (O4), and the items older entries owe a "next revision" (O5).
