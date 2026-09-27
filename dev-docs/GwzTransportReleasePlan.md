# GWZ transport release plan

Date: 2026-09-27. Status: **accepted at SHA-256 `4ec6ba33da5311921edfe15e5e7f8c9b7cd245fec5eb7e9465875997c0ba996d` after [Consistency-1](GwzTransportReleasePlan-ReviewConsistency-1.md) and [Safety-1](GwzTransportReleasePlan-ReviewSafety-1.md) reported GO; this accepts the plan text only**.
- This status sentence was added after that GO.
- So were the corrections the reviewers cleared without a further round. [Verdict-1](GwzTransportReleasePlan-Verdict-1.md) records them.
- Revision 1 applied the [first remediation plan](GwzTransportReleasePlan-RemPlan.md) after the [first verdict](GwzTransportReleasePlan-Verdict.md).
- On 2026-09-27 the operator adopted every recommendation in §7 (OD1–OD10).
- Acceptance authorizes no implementation, commit, tag, push or publish.

This plan replaces the [1.1.0 plan](GwzV110Plan.md), as its [amendment](GwzV110PlanAmendment.md) left it, as the plan for the next minor release. It adopts, by their IDs, the accepted steps of those documents that still hold (§4), and adds the work the operator put into the release on 2026-09-27. Superseded text in both documents stays readable.

## 1. The release

**Name and version.** This is the **transport release**: the next minor release of gwz-core, the `gwz` CLI and gwz-py, expected to be v1.1.0. The version is fixed when Phase 10 starts. In adopted text, "1.1.0" and "v1.1.0" mean that version, including in pins such as `gwz-core = "=1.1.0"`. The published line today is 1.0.17.

**Reading adopted text.**
- In adopted text, "Phase 7" means this plan's Phase 9, "Phase 8" means Phase 10, and "S7" means S7.1–S7.5.
- The 1.1.0 plan and the retry plan use the same IDs for their S3 steps, so this plan always prefixes those with their source: "1.1.0 S3.3", "retry S3.3". Any other bare S-step is a 1.1.0 step, and a CS-step is the session plan's.

**Outcome.** The normal builds of gwz, gwz-core and gwz-py, on macOS ARM64, Linux x86-64 and Windows x86-64, ship:
- the endpoint-owned SSH and gh-only HTTPS transport in the `local` placement, with connection pooling and connection-setup retry;
- the core session host of the [session contract](../../dev-docs/GwzCoreSessionDesign.md), with gwz-cli and gwz-py as its clients;
- `gwz server` and `gwz-py server` from the [server design](../../dev-docs/GwzCoreServerDesign.md), reached with `--server` and `SocketCoreBridge`;
- **connection reuse across operations.** A later operation uses an idle connection that an earlier operation left in the same host context, when their endpoint configurations match. Through a server, that is reuse across commands and across clients. In a Python process, it is reuse across that process's operations. An in-process CLI command reuses connections within itself, as today.

**Operator decisions this plan records.**
- 2026-09-26: gwz-cli and gwz-py reach the transport through one layer, and gwz-py's public API stays. This was the amendment's decision (c).
- 2026-09-27: the release's name; `gwz server` ships in this release; so does connection reuse across commands.

Decision (c) used the per-operation entry because the session host was then outside the release. Now the host is in the release, so the one layer both clients share is the session host (OD1).

**Authorities that stay in force.**
- [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md), and the Phase 6 exit rows of [GwzRemoteTransportPlan.md](GwzRemoteTransportPlan.md), read for placement as §2 says.
- [GwzRemoteTransportReleaseReadiness.md](../../dev-docs/GwzRemoteTransportReleaseReadiness.md).
- The [timeout plan](GwzRemoteTransportAlphaTimeoutPlan.md), which 1.1.0 S3.1 and 1.1.0 S3.3 close. (1.1.0 S3.2 is the Q6 review.)
- The [retry plan](GwzRemoteTransportRetryPlan.md), as TR1.2 amends its §3 item 7, §6 and S1.4.
- [GwzRemoteTransportQualification.md](GwzRemoteTransportQualification.md), for what Batch A already ran.
- The session contract at the revision TR1.1 accepts, as TR1.2 amends it. The server design as TR1.3 revises it. The [session plan](../../dev-docs/GwzCoreSessionPlan.md) as TR1.4a and TR1.4b revise it.
- `gearu` for every repository that still needs a release script. gwz-core, gwz-cli and gwz-py keep `scripts/release.py`.
- The review loop, under `dev-docs/AgentProcessRules.md` as amended by `dev-docs/GwzProcessOptimization.md`.

## 2. Scope

| In this release | Recorded as unsupported in this release |
|---|---|
| macOS ARM64, Linux x86-64, Windows x86-64 | Linux ARM64, macOS x86-64 |
| SSH and gh-only HTTPS in the `local` placement, with connections pooled in the host context | iroh, a physical byte carrier, and client placement: a `cli`-placed endpoint, in process or in another process. Frame tags 16–31 stay reserved |
| Private HTTPS authenticated through `gh` | Private HTTPS authenticated through another credential helper. Under OD10's recommendation it takes the native route and is listed in the migration notes; under its alternative it is refused (TR1.6) |
| Connection reuse across the operations of one host context: a server's, or one Python process's. An in-process CLI command reuses within itself, as today | Reuse between processes other than through a server; reuse between users |
| `gwz server` and `gwz-py server` over a Unix-domain socket or a named pipe, for the same user on the same machine | A server on another machine or for another user; any TCP listener |
| The two setup clocks, 9 s stall and 30 s aggregate, and setup retry with `--max-retries`, per the retry plan | Any other new timeout flag; a changed 60 s idle default, unless TR1.2 amends this row and the sentences its idle question lists |
| gwz-py on the session host, in process or through a server. Its public API is unchanged apart from the new `SocketCoreBridge` and `NativeCoreBridge`'s optional host context | A Python binding of `gwz-transport`; a second pool in Python |
| The new crates `gwz-libgit2-sys`, `gwz-git2`, `gwz-transport` and `gwz-git` | Publishing the forks as `libgit2-sys` or `git2` |

- **Placement.** Only `local` is in this release. Two sentences therefore read as `local` only:
  - adopted 1.1.0 S5.1's "both placements";
  - the Transport Plan's Phase 6 sentence "The network-entry ledger is complete for SSH and HTTPS in both placements".

  This plan is the amendment that narrowing requires. `transport_capabilities` advertises only `local`.
- **Windows.** The 1.1.0 plan's §2 rules for dabeest apply unchanged: mingw bash, the `/e/gwz-tests/<name>` work root, the historical `D:` trees untouched, and the Cargo config isolation.
- **Evidence.** Phases 2 and 4–10 redact agent-socket paths, known_hosts bodies, `gh` tokens or headers, and private members' repository names from retained evidence and public reports. Python-visible errors and server logs follow the same rule. A secret in filed evidence fails that step.
- **Live accounts.** Every run against a real account, such as a GitHub fetch or push with the operator's credentials, needs the operator's go, in every phase.
- **Narrowing.** Taking a platform or a row out of this release is an amendment, not a footnote on the tag.

## 3. Starting point, 2026-09-27

- **Released:** 1.0.17 of gwz-core, gwz-cli and gwz-py.
- **Transport in the tree, candidate only.** The transport is built only under the `gwz_transport_candidate` cfg, on Unix.
- **Committed on main, never reviewed as code:**
  - the retry plan's Phases 1 and 2: the 100/32 defaults (`gwz-core/src/operation/resolve_jobs.rs`, `resolve_per_host.rs`), pool caps that follow the operation, the jobs-bounded scheduler, the 9 s stall (`gwz-cli/src/lib.rs`) and the 30 s setup budget;
  - the typed setup-failure causes;
  - the gh challenge-reuse code (gwz-core `26b30ca6`), whose design has GO;
  - the sequenced-stream kernel (gwz-transport `c4b632d`), exported as `pub mod sequenced`, whose design has GO as a contract only.
- **Retry plan Phase 3 is not written.** The committed `--ssh-timeout` help (`gwz-cli/src/globalargs/parser.rs`) already says stalled setups are retried, and names a `--max-retries` flag that does not exist.
- **Crate names.** `gwz-transport`, `gwz-git`, `gwz-git2` and `gwz-libgit2-sys` exist on crates.io as `0.0.0-bootstrap.1` placeholders. At the committed HEADs, the rename is half landed:
  - gwz-core depends on the new git2 package names, `gwz-git2` and `gwz-libgit2-sys`. Its committed manifest has no gwz-transport dependency; the candidate harness injects one;
  - git2-rs's manifest still says `git2`;
  - the candidate harness's patch table still names the old packages, which breaks the candidate build.
- **Publishing.**
  - There is no `gearu.toml` in any repository.
  - git2-rs still carries upstream's token-based `publish.yml` and `ci/publish.sh`.
  - git2-rs, gwz-git, gwz-transport, gwz-cli and gwz-core each carry a bootstrap publish workflow that uses a registry token secret.
- **Windows:** the four SSH endpoint modules (`ssh_network`, `agent_socket`, `ssh_key_auth`, `ssh_local`) have no Windows arm.
- **1.1.0 S3.1–S3.3:** none has closed. Focused timeout evidence from 2026-09-23 is in the private evidence member.
- **Designs:**
  - Contract revision 2 is accepted.
  - Revision 3 is NO-GO on one blocking root, B11. RemPlan-2 proposes revision 4.
  - The session plan and the server design are unreviewed drafts.
  - Both exclude connection reuse across operations.
- **Python:** gwz-py's candidate long-lived `TransportSession` still carries the Phase 6/7 NO-GO of 2026-09-23.
- **Defects reported on 2026-09-23**, in `gwz-core/dev-docs/GwzRemoteTransportBugReport.md` (uncommitted):
  1. On the transport build, a private HTTPS member fails `Authentication` where 1.0.17 succeeds. The row shows a credential method but `offered=false`.
  2. A `Partial` result carries an empty top-level `errors` array.
  3. 1.0.17's 3 s SSH timeout fails under load against GitHub.
- **Pool capacity.** The [pool-capacity brief](GwzRemoteTransportPoolCapacity.md) measured the pooled alpha slower than 1.0.17 once per-host concurrency was raised: 6.9 s against 3.1 s for 32 repositories.

## 4. What this plan adopts and retires

**Adopted, by ID:**

| Source | Steps | Here |
|---|---|---|
| 1.1.0 plan | S2.1–S2.3, with the amendment's S2.2 edge text | Phase 3, with Phase 3's note on S2.3 |
| 1.1.0 plan | S3.1–S3.3 | Phase 2 |
| 1.1.0 plan | S4.1–S4.5 | Phase 4 |
| 1.1.0 plan | S5.1–S5.6 | Phase 8, with TR8 additions. S5.1 reads as `local` only (§2) |
| 1.1.0 plan | S7.1–S7.5, with the amendment's additions: its S7.1 site list and check, its S7.2 addition, its S7.3 addition and its exit-row change, as Phase 9 restates them | Phase 9, extended |
| 1.1.0 plan | Phase 8: its preamble, steps 1–7, its product-repository sentence and its post-release check, as the amendment's §3.6 left them, with Phase 10's replacements | Phase 10, extended |
| Retry plan | Phase 3, its S3.1–S3.5 | Phase 2, as TR2.1 |
| Session plan | CS1.1–CS6.5 | Phase 5, as TR1.4a and TR1.4b revise them |

**Retired or replaced:**
- **The amendment's S1.1 and S1.2** (OD1). The per-operation revision of gwz-py's transport design is not written.
- **The amendment's S6.1–S6.3,** as steps. Their obligations move to the session plan, where TR1.4a or TR1.4b maps each one to a step (Phase 1 says which):
  - **S6.1's transport entry**, which takes the operation's cancellation token.
    - Its library-safety rule: catch the operation's panic first, run finish and shutdown under their own guard, report failure with cleanup unconfirmed, and never finish from `Drop` while unwinding.
    - Its unit tests: a cancel before the start; a cancel while running; the cleanup report each returns; and a fault-injected panic in finish after a panic in the operation, after which the process stays alive and the next operation succeeds.
  - **S6.2's removal** of `TransportSession` and its `CURRENT_SESSION` debt entry.
  - **S6.2's release pins.**
    - `gwz-py/RELEASE.md` and the publish workflow name every native dependency pin.
    - gwz-core on the release branch is `=1.1.0` from crates.io.
    - `GwzCratesIoPlan.md` D7's git-tag-only core pin is not the release form.
  - **S6.3's test rows**, restated for the session host and reuse. They run on macOS ARM64, Linux x86-64 and dabeest against disposable SSH and HTTPS fixtures, and the dabeest rows wait on 1.1.0 S4.5. Each network test asserts, through its transport observations, that it took the transport route. The rows:
    - two overlapping Python operations on one `Client` complete independently;
    - a gh failure and an unsupported proxy refuse, with no credential material in Python-visible errors;
    - default clocks apply, and 1.1.0 S3.3's stall regression passes through gwz-py;
    - a cancel returns its cleanup report;
    - a wrong, foreign or completed cancel fails harmlessly, and cancellation state stays bounded;
    - excess operations wait under the contract's session limits;
    - close and interpreter exit cancel or join, and leave no helper process;
    - the environment is captured when the session opens;
    - for 1, 2 and 8 overlapping operations, the construction cost of each operation's own part of the transport, as TR1.2 names it, and the connection counts are recorded, for S7.2's notes.
- **Verdict-2's five carried items.** The amendment's Verdict-2 routed them to S1.1 and S1.2, which are retired. They move as follows:

  | Item | Moves to |
  | --- | --- |
  | Whether a waiting operation holds a native thread, and a bound on the waiting set | TR1.4a |
  | The interpreter-exit bound when `configure_transport_runtime` disables deadlines | TR1.4b |
  | The environment capture's race with `os.environ` mutation | TR1.4b |
  | Which reading of "paths" S7.2's notes use for the native branch | Phase 9, S7.2 |
  | The fixtures S7.3's Linux run needs on the CI host, now including a server | Phase 9, S7.3 |
- **The amendment's other retired sections, and their replacements here:**

  | Amendment section | Replaced by |
  | --- | --- |
  | §3.1, the scope rows | §2 |
  | §3.7, out of scope | §9 |
  | §3.8, the closing note | The closing condition below |
  | §3.9, the dependency sketch | §6 |
  | §4, affected tests | The moved obligations above |
  | §6, the core session contract | §1, and TR1.1 to TR1.4b |
- **The amendment's S7.2 sentence on gwz-py's notes** (one runtime per operation, no reuse, 8 per `Client`). It is replaced in Phase 9.
- **The 1.1.0 plan's §1, §5 and closing note.** They are replaced by §1, §9 and the closing condition below.

**The NO-GO of 2026-09-23 closes** when the first restated S6.3 row above passes through the session host on all three platforms: two overlapping Python operations on one `Client` complete independently.

## 5. Phases

Step IDs of this plan are `TR<phase>.<step>`. Adopted steps keep their source's IDs, prefixed with the source. Product steps aim at under 500 lines each.

### Phase 1 — Designs (milestone: every design the release builds on has GO; no product code)

This phase can start now. Phases 2, 3 and 4 do not wait on it.

- **TR1.1: contract revision 4.**
  - Apply RemPlan-2 as OD3 decides.
  - The same two axes give a focused re-verdict (RemPlan-2 §5).
  - TR1.1 closes on a filed Verdict-4 with both axes GO.
- **TR1.2: the reuse design** *(`dev-docs/GwzConnectionReuseDesign.md`; design only)*.
  - **What it amends:**
    - **The contract:**
      - §1's exclusion of reuse, and its adopted "one transport runtime per operation";
      - §2's worker row;
      - §4.2, which must state whether `TransportCapacityConflict` re-enters, and how the capacity answer maps to it;
      - §4.3;
      - §5.2's per-operation runtime;
      - §5.5;
      - §5.6's host context;
      - §5.7;
      - §8's close report;
      - §13's "no change to gwz-transport";
      - §14's per-operation runtime sentences;
      - §15 and §16;
      - every other contract sentence that rests on the per-operation runtime.
    - **The retry plan:** its §3 item 7, §6 and S1.4, by quotation (question 6).
    - **Construction-time configuration:** the HTTPS design's §4 construction-time snapshot, and `SshEndpointConfig::from_environment` (question 2).
    - **Where needed:** the placement design's §2, and gwz-transport's pool.
  - **Prior design.** The accepted [placement design](GwzRemoteTransportPlacementDesign.md) §2 already has one runtime per backend family, shared by operations. It says "Healthy pooling survives operations within a runtime". The contract narrowed that to one runtime per operation.
  - **The pool today.** It already scopes ownership by session and operation (`pool::Owner`). Its key does not yet cover the endpoint configuration.
  - **Timing.** Drafting may start beside TR1.1. Review waits on TR1.1's GO.
  - **It answers:**
    1. **Ownership and lifetime.** Which parts of the transport the host context owns and shares, which parts each operation owns, and when each is built and dropped. What an in-process CLI command, a Python process and a server each get: reuse within one command, within the process, or across clients.
    2. **Configuration ownership.** Each operation's endpoint configuration derives from its own session's snapshot (contract §5.6). That covers the SSH home, the agent, the known_hosts inputs, TLS roots, proxies and the `gh` environment. A shared runtime never serves a session with configuration derived from another session's snapshot, or from the server's own environment. The design states the mechanism, and supersedes the HTTPS design's §4 construction-time snapshot and `SshEndpointConfig::from_environment`.
    3. **Eligibility.** Which per-operation inputs must be equal for an operation to reuse a connection another operation or session left:
       - for SSH: user, host and port; the identity (agent or explicit key); the agent itself, identified by the agent's own identity rather than its socket path (on Windows, by its kind and address); the SSH home; and the known_hosts inputs;
       - for HTTPS: TLS roots and proxies.

       It also says how equality is computed, without retaining secrets.
    4. **Trust inputs that change between operations.** For each case below, the design says whether a live connection is revalidated per lease, retired or kept, and bounds its age:
       - a changed known_hosts file;
       - a key removed from the agent;
       - a changed key file;
       - a different agent behind the same path;
       - a key whose agent requires confirmation or presence, or whose lifetime has expired.
    5. **What a live connection grants.** An authenticated SSH connection outlives the command that authenticated it. The design says what that grants a later session that passed the server's peer and sandbox checks, and confirms that nothing beyond the connection itself is retained.
    6. **Capacity under overlap.** A shared pool cannot keep the retry plan's rule, which installs one operation's caps and refuses an operation while a lease is live:
       - its §3 item 7: "While a lease is non-idle, a request cannot raise them; that operation is refused";
       - its §6: "If any lease is non-idle, the new operation is refused with a typed error";
       - its S1.4.

       The design quotes these as superseded. It states what replaces the refusal: physical ceilings, per-operation limits, fairness between sessions, and `max_requests`. It keeps the Transport Plan's rule that runtime defaults cannot "change per-operation fan-out semantics".
    7. **Retry and backoff state.** Whether the retry plan's per-key machine stays per operation or is shared per key.
    8. **Isolation.** Cancelling or failing one operation never closes another operation's lease or a healthy shared connection. Retirement and cleanup accounting stay correct across operations, including Q6's monotonic retirement fix.
    9. **Timeouts.** Which clocks belong to a request and which to a connection, when sessions with different `configure_transport_runtime` settings share a connection.
    10. **Idle and keepalive.** Whether the 60 s idle default serves reuse across commands. Changing it amends §2's row and needs Surface. It also supersedes, by quotation:
        - 1.1.0 S5.4's "The 60-second idle default stays";
        - the Transport Plan's "Runtime defaults cannot override the agreed 60-second idle default" and "use the agreed 60-second idle timeout";
        - the transport design §12's "60 s idle";
        - the release-readiness guide's "Preserve the agreed 60-second idle default".
    11. **Observations.** A reused connection is reported as reused. No other session's member, URL, private-member fact or configuration appears in an observation.
    12. **gwz-transport changes.** Which changes the design needs. They ship in gwz-transport's Phase 10 release.
    13. **Tests and cells.** The contract §15 rows it adds, including Phase 6's exit tests, and the Design §11 cells (1.1.0 S5.6 rows) it adds, with their evidence kind.
  - **Review.** Dual peer-blind Consistency and Safety. Add Surface if the design changes a user-visible default or option.
  - **Closure.** TR1.2 closes on a filed verdict that accepts the design and the contract text it amends, with the contract's status edited under AgentProcessRules §7.2.
- **TR1.3: server design revision.** It follows TR1.2's GO and makes these changes:
  - **Reuse.** It brings reuse into scope (§1, §10). It amends §4's "gains nothing it couldn't already do" as far as TR1.2's answer to question 5 allows.
  - **Listener verification.** The client verifies the listener before it sends any byte:
    - on Linux and macOS, the connected socket's peer credentials (`SO_PEERCRED`, `getpeereid`) name the caller's user, and the socket file and its directory are owned by that user, with no group or other permission bits and no symlink;
    - on Windows, the pipe server's process token names the caller's user SID, and the client connects with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`;
    - the `/tmp/gwz-<uid>/` fallback is refused when the directory exists with another owner or mode. A missing directory is not a refusal: the auto-started host creates it with mode 0700, and the client then verifies it and connects;
    - a failed check sends no frame, and refuses with a named error code.
  - **The off switch through a server.** With TR1.5's switch on, every process-wide read the native path makes joins the session's must-match set. That is at least `SSH_AUTH_SOCK`, and each read the contract's §5.8 lists. A mismatch refuses with `server_environment_mismatch`. The `auto` key covers those values when the switch is on.
  - **One agent source per session.** It reconciles the §11 agent row with 1.1.0 S4.3. Each session has one agent source, chosen explicitly: the snapshot's `SSH_AUTH_SOCK`, or a documented Windows default pipe when that is unset. Pageant is used only when selected explicitly, after a fixture proves it.
  - **Defaults.** It records OD2's default. It says what a sandboxed caller with `GWZ_SERVER` set sees: a refusal that names the cause and `--no-server`.
  - **Idle.** It states how `--idle-exit` relates to the pool's idle expiry.
  - **The `auto` key.** It reconciles the key's must-match partition with TR1.2's eligibility rule.
  - **G1.** It records OD9's wording of proposals G1.
  - **Cells.** It lists the Design §11 cells (1.1.0 S5.6 rows) the server adds, with their evidence kind.
  - **Review:** dual Consistency and Safety, plus Surface. The Surface review covers the `server` command family, `--server`, `GWZ_SERVER`, `--no-server` and `SocketCoreBridge`.
- **TR1.4a: session plan revision, part 1.** It follows TR1.1 alone. It revises every section and step of `GwzCoreSessionPlan.md` that does not depend on the runtime model, in any phase, as follows:
  - **D2 and D9.** It supersedes them: everything ships in this release, and a step's code merges once its review passes, subject to §6's rules.
  - **Scope.** It rewrites §1.1. It removes §1.3's exclusions of the 1.1.0 S6 steps, of the server, of gwz-transport changes and of public API changes.
  - **Other sections.** It rewrites §2.4, R1 and §5.3, and CS3.1's 1.1.0 dependency.
  - **G2.** It becomes the build prerequisites for transport rows: TR3.1 for the candidate build, and 1.1.0 S4.5 for the Windows arms. No tag precedes any phase.
  - **Ordinary-path steps.** It marks each session-plan step that changes ordinary-build behaviour outside the switch (§6(b)).
  - **Mapping.** It maps to steps:
    - those of §4's moved obligations that land outside CS3.7 and the session plan's Phase 4;
    - Verdict-2's first carried item.
  - **Review:** dual Consistency and Safety. This is the session plan's G1 for the parts it revises.
- **TR1.4b: session plan revision, part 2.** It follows TR1.2's and TR1.3's GO. It revises CS3.7, the session plan's Phase 4, and every other step that depends on the runtime model, reuse or the server:
  - It replaces the session plan's per-operation runtime ("each on its own runtime") with TR1.2's model.
  - It rewrites the 1.1.0 dependencies of CS3.7 and CS4.1.
  - **The panic model.** CS3.7's phrase "as today's `Command` in `local_command.rs` does" does not stand, because `Command`'s `Drop` finishes without a guard for unwinding.
  - **Mapping.** It maps §4's remaining moved obligations, and Verdict-2's second and third carried items, to steps.
  - It adds the reuse steps (Phase 6 here) and the server steps (Phase 7 here), each under 500 lines.
  - **Review:** dual Consistency and Safety. This is the session plan's G1 for the rest.
- **TR1.5: off-switch design (OD6).** A short design for a documented opt-out that sends every network operation to the native path. It covers:
  - **Scope:** the user's process only, through a flag, an environment variable or user-level configuration. A workspace or repository value is ignored, with a message that names the scope.
  - **Precedence:** flag, then environment, then user configuration.
  - **Notice:** one notice, outside `--verbose`, at a session's first network operation with the switch on.
  - **Servers:** the client resolves the switch's three forms into one value before `open`, and carries that value in `SessionOpen`, as an entry of the environment snapshot or a `ProcessAttributes` field. The `auto` key and the must-match set use the resolved value, and TR1.3's must-match rule applies through a server.
  - **Reporting:** `--verbose` shows the switch, and the design says how the ledger reports it.

  The switch is not a fallback after an open.
  - **Review:** one dual Consistency and Safety pass, plus Surface.
  - **Timing:** independent of TR1.1 to TR1.4b. TR1.3 states the server rule to match: whichever of the two closes second matches the first.
- **TR1.6: non-gh HTTPS credentials (OD10).** An amendment to three texts:
  - [GwzRemoteTransportHttpsDesign.md](GwzRemoteTransportHttpsDesign.md) §4;
  - the transport design's §11 HTTPS cell ("other helpers rejected");
  - 1.1.0 S7.1's "Credential locality and gh-only authenticated HTTPS stay as designed".

  Today a caller whose credential-helper policy does not permit gh gets the original refusal. The amendment either chooses the native route before any connection opens, or keeps the refusal with a message that names the gh-only policy and the off switch. Either way there is no fallback after an open.
  - **Review:** dual Consistency and Safety, plus Surface, since either option changes what an upgraded user sees.
- **TR1.7: Python design disposition** *(status only)*.
  - `gwz-py/dev-docs/GwzPyTransportDesign.md` is marked superseded by the contract's §9, §10 and §14, as TR1.2 and TR1.4b leave them. Its NO-GO closing condition becomes §4's. This lands with TR1.4b's acceptance.
  - The session plan's gwz-py phase carries the Surface review of gwz-py's changed documented behaviour: the changes the session plan's R8 and CS4.5 list, plus reuse across operations.

### Phase 2 — The transport, finished and reviewed (milestone: retry, the reported defects and the open corrections are in the tree, and the transport code has GO)

This phase can start now. Its transport tests need TR3.1; TR2.3 and TR2.4 do not.

- **TR2.1: retry plan Phase 3** *(retry S3.1–S3.5 as written; about 1,400 lines across its five steps)*.
  - The retry state stays per operation, as the retry plan has it; TR1.2 decides whether it widens.
  - After TR2.1, the `--ssh-timeout` help describes the transport build's behaviour.
  - Once TR2.1 lands, a retried stall no longer fails a member. 1.1.0 S3.3's live runs then take their two-clock evidence from its production-graph regression, or run at `--max-retries 0`.
- **TR2.2: defect 1** *(under 500 lines)*.
  - Reproduce it once TR3.1 lets the candidate build compile. Use the disposable HTTPS fixture: a local Git HTTP server, a temporary CA and a fake gh. Test with a credential helper that names gh by an absolute path, and with one that names it through `PATH`.
  - Fix it, with a regression test for each.
  - The route for non-gh credentials follows TR1.6.
- **TR2.3: defect 2 (OD7)** *(under 300 lines)*.
  - A `Partial` result lists each failed member's error in its top-level `errors`.
  - The change covers gwz-core's result, `gwz-cli/docs/MachineOutput.md`, and gwz-py's handling. It has tests on fetch and push.
  - It changes a documented contract, so it is released only with Surface review (§6).
- **TR2.4: sequenced module out of the default build (OD5)** *(under 100 lines)*.
  - gwz-transport's `sequenced` module moves behind a non-default Cargo feature, `unstable-sequenced`, inside an explicit `cfg_if` boundary.
  - The two version-3 branches in `binding.rs` that call into the module go inside the same boundary. Without the feature, a Bind that offers version 3 is refused before any Git, credential or pool effect.
  - The crate docs mark the feature as unreviewed, with no compatibility promise.
  - The module's tests run with the feature in gwz-transport's CI, and one more test shows version 3 refused without it.
  - The Envelope's `message_seq` field stays, as the internal compatibility amendment allows.
- **TR2.5: the off switch**, after TR1.5's GO *(under 300 lines)*.
- **TR2.6: transport implementation review.** Dual peer-blind Code and State review, on the settled tree after TR2.1–TR2.5. It covers:
  - the typed setup causes;
  - the retry plan's Phases 1–3;
  - the gh challenge-reuse code;
  - TR2.2–TR2.5.

  It may share reviewers with 1.1.0 S3.2, but it is its own gate.
- **Adopted: 1.1.0 S3.1, S3.2 and S3.3,** unchanged. 1.1.0 S3.3's live cold fetches also run defect 3's check: five consecutive default fetches of a 16-member SSH workspace all exit 0 on the transport build.

**Rule for main.** A release cut from main before 1.1.0 S7.1 activates the transport and TR2.6 has GO ships none of these:
- the retry help text;
- `--max-retries`;
- any ordinary-path change §3 lists as unreviewed.

The exception is a release that makes the help and the option conditional on the transport build, and whose own review covers the ordinary-path changes it ships. That exception amends retry S2.2's and S3.4's help pins for the ordinary build, by the same retry-plan amendment OD4 names. §6's rules apply as well.

### Phase 3 — Crate identity (milestone: `gearu plan` succeeds for each new package; nothing is tagged)

- **TR3.1: finish the crate rename** *(under 200 lines)*.
  - Every manifest, `[patch]` table and harness names the new packages. The harness is `gwz-core/tests/transport_backend/prepare.py` and its tests.
  - The candidate build compiles.
  - This step is foundational: the transport tests of Phases 2, 4 and 5 need it.
  - The lane that started the rename owns this step, unless the operator reassigns it.
- **TR3.2: retire stray publish paths** *(under 100 lines)*.
  - Remove git2-rs's upstream `publish.yml` and `ci/publish.sh`, and every bootstrap publish workflow: those in git2-rs, gwz-git, gwz-transport, gwz-cli and gwz-core.
  - A workflow-text test in each new repository asserts that no workflow references a registry token secret, and that only the `release.published` workflow runs `cargo publish`.
  - The operator revokes the bootstrap token secret.
- **Adopted: 1.1.0 S2.1–S2.3.** All four names already exist as bootstrap placeholders, published by each repository's bootstrap workflow.
  - That publish satisfies S2.3's first-publish token clause.
  - S2.3's exit therefore also requires a configured trusted publisher for each of the four names.
  - Phase 10 replaces step 1's token sentence to match.

### Phase 4 — Windows (milestone: SSH and HTTPS run on Windows x86-64 through the same host)

- **Adopted: 1.1.0 S4.1–S4.5.**
  - S4.5 also waits on TR3.1.
  - Agent selection on Windows follows S4.3: a missing agent is a refusal, not a fallback to another key store. Pageant is used only when selected explicitly (TR1.3).
  - The server's Windows primitives are Phase 7 work: the named pipe, its ACL, the AppContainer and integrity refusals, and the client's listener verification.

### Phase 5 — The session host (milestone: gwz-cli and gwz-py send every protocol request through the session host, and the two-bridge wire proof passes)

- **Steps:** the session plan's Phases 1–6, as TR1.4a and TR1.4b revise them.
- **Gates:**
  - TR1.1 (the session plan's G0) and TR1.4a gate every step TR1.4a revises: the steps that do not depend on the runtime model.
  - TR1.4b gates CS3.7, the session plan's Phase 4, and every other step that depends on the runtime model, reuse or the server.
- **Build prerequisite:** the transport tests in the session plan's Phase 3 need TR3.1.

### Phase 6 — Connection reuse (milestone: operations in one host context reuse matching idle connections, and nothing else)

- **Steps:** those TR1.4b places for TR1.2, after the session plan's network phase. They include every gwz-transport change TR1.2 names.
- **Exit tests,** at least:
  - two sessions with one endpoint configuration run sequential operations over one physical connection;
  - two sessions on one host context with different `GH_TOKEN` and `SSH_AUTH_SOCK` values, against the disposable fixtures. Each `gh` invocation sees its own session's token, each new SSH connection asks its own session's agent, and no connection is shared;
  - a different agent or explicit identity never shares a connection;
  - the agent behind one socket path, swapped between two sessions: no reuse;
  - a key added to the agent with a confirmation requirement: the second command either triggers a fresh confirmation or opens a new connection;
  - a changed trust input that TR1.2 names is handled as TR1.2 says;
  - cancelling or failing one operation leaves another's lease, and a healthy shared connection, intact;
  - two sessions' operations with different `--max-per-host` values are both admitted while a lease is non-idle;
  - overlapping operations from several sessions stay within the pool's caps;
  - physical-connection and channel counts appear in `--verbose` transport rows.

### Phase 7 — The server (milestone: both CLIs host and use a server on all three platforms, with reuse through it)

- **Steps:** those TR1.4b places for TR1.3, ordered as follows:
  - `gwz server` follows the session plan's gwz-cli phase;
  - `gwz-py server` and `SocketCoreBridge` follow its gwz-py phase;
  - Windows follows Phase 4;
  - reuse through the server follows Phase 6.
- **Exit:**
  - the server design's §12 verification, on all three platforms. That includes both CLI suites run through a server, the soak, and the sandbox refusal;
  - the squatted-listener refusal, checked from the client side on all three platforms: a listener owned by another user, at the `auto` path and at an explicit address, receives zero bytes;
  - a server started with one `SSH_AUTH_SOCK`, and a client with another `SSH_AUTH_SOCK` and the off switch on, run once with the switch given as a flag and once as an environment variable. Both runs are refused, and the server's agent records zero signature requests;
  - a sandboxed caller with `GWZ_SERVER` set gets a refusal that names the cause and `--no-server`;
  - a second command through a server reuses the first command's connection;
  - gwz-core's checker, in the mode the server design adds, reports no `env` or `process` debt. That is the server design's release gate.

### Phase 8 — Measurements and defaults (milestone: defaults chosen inside the frozen caps, on evidence that includes reuse)

- **Adopted: 1.1.0 S5.1–S5.6,** with these additions:
  - **TR8.1: parity, in S5.1.** Targets:
    - a no-op fetch of 32 small repositories over SSH, at default settings, is no slower than 1.0.17 with `--max-per-host 32`;
    - no partial result in three rounds at 16 and at 32 members;
    - physical connection counts are recorded.

    **On a miss:**
    - S5.4 first retunes within the shared pool's ceilings, and the rows are measured again.
    - Only then is OD8 put to the operator, with the measured gap and the exits that would re-run if several channels per connection came in:
      - Phase 6's exit tests;
      - Phase 7's soak;
      - TR8.2;
      - S7.4;
      - TR1.2's capacity answer, by amendment.
  - **TR8.2: reuse rows, after Phase 7.** On macOS and Linux:
    - back-to-back commands through a server against the same commands in process: wall time, and the second command's new connections;
    - sequential Python operations in one process;
    - several clients on one server.

    S5.5 repeats them on dabeest.
  - **TR8.3: S5.3's sustained-memory row** also covers a long-lived server under a client soak.
- **Defaults.** S5.4 waits on TR8.1–TR8.3, and also chooses the shared pool's ceilings.
- **Sign-off.** S5.6's table gains the server and reuse cells that TR1.2 and TR1.3 list. It keeps its rule: nothing is advertised without evidence.

### Phase 9 — Activation (milestone: an ordinary build on the three platforms ships the transport, the session host, the server and reuse, and the activation review has GO)

Depends on Phases 2, 4, 5, 6, 7 and 8.

- **1.1.0 S7.1, adopted.**
  - The candidate switch goes from every site that exists at that point, in all three crates: the transport's sites, the session host's transport entry and its arms, and the surface §6(a) puts behind the switch.
  - The completeness check stands: `rg gwz_transport_candidate` over the source, tests, scripts and manifests of gwz-core, gwz-cli and gwz-py finds nothing, with `dev-docs` excluded.
  - The native branch stays for the paths this release does not support, and for the off switch.
- **1.1.0 S7.2, adopted and extended.**
  - The amendment's S7.2 addition applies, with Phase 5's route tests and S7.3 as its evidence in place of S6.3.
  - The route ledger gains rows for:
    - the CLI through a server;
    - Python in process and through `SocketCoreBridge`;
    - reuse;
    - the off switch;
    - the non-gh credential route.
  - Help, docs pages and migration notes cover `--max-retries`, the server command and its options, reuse and its lifetime, and the off switch.
  - The notes state gwz-py's session-host behaviour. That replaces the amendment's per-operation sentence.
  - The notes say which reading of "paths" they use for the native branch.
- **1.1.0 S7.3, adopted and extended.** Each platform's consumer build runs four route checks:
  - one CLI network operation in process;
  - one CLI network operation through a server;
  - two overlapping Python operations in process;
  - one Python operation through `SocketCoreBridge`.

  Each asserts its transport route through its observations. A second CLI command through the server also asserts reuse. The Linux run's fixtures on the CI host, including a server, are part of this step.
- **1.1.0 S7.4, adopted.** It also rechecks reuse attribution.
- **1.1.0 S7.5, adopted.** Surface review is required. It covers:
  - `server`, `--server`, `GWZ_SERVER` and `--no-server`;
  - `--max-retries`;
  - the off switch;
  - `SocketCoreBridge`;
  - the `--verbose` transport-row fields;
  - TR2.3's `errors` contract, if no earlier Surface review took it.
- **Exit-row mapping.** The 1.1.0 table applies, with the amendment's row, plus:
  - "Aggregate review" maps to S7.5 and TR2.6;
  - "Network-entry ledger" maps to S7.2, S7.3, Phase 5's route tests and Phase 7's exit, in place of the amendment's S6.3.

### Phase 10 — Release (milestone: the three product tags exist and their publish jobs have succeeded)

- **Adopted: 1.1.0 Phase 8,** as the amendment's §3.6 left it, with §1's version. That includes its preamble and its product-repository sentence:

  > Depends on S7.5 and S2.3. The operator runs the commands. This document does not run them. The sketch is normative: S3.3 and S5.5 are on the path here.
  >
  > If any step fails after a push, a GitHub Release, or a registry publish, stop. Do not run a later product tag. Do not claim v1.1.0 complete. Record the published artifact IDs. Resume only with a new patch or release-candidate version. Never move or reuse a tag.

  > Product repositories, each with the existing release script, tag `v1.1.0`. Each waits until the previous product crate is visible. Pins are registry versions, not git pins and not sibling paths.

  - TR3.2 is a further prerequisite.
  - Step 1's sentence "The first publish of each new name uses the operator-held token from S2.3" is replaced: each name publishes through its trusted publisher (Phase 3).
  - Step 2's gwz-transport release includes Phase 6's transport changes and TR2.4's feature.
  - Step 7's wheels use the pins that §4's moved S6.2 obligation names.
- **Post-release check,** added on each host:
  - `server --start`, `--status` and `--stop` from each installed CLI. Every server the check starts is stopped;
  - a second command through the server reuses the first command's connection;
  - one Python operation through `SocketCoreBridge`.

## 6. Dependency sketch

```text
Phase 1   TR1.1 ── TR1.2 ── TR1.3 ── TR1.4b ── TR1.7
          TR1.1 ── TR1.4a
          TR1.5;  TR1.6
Phase 2   TR3.1 ── TR2.1, TR2.2 (its route after TR1.6)
          TR2.3;  TR2.4;  TR1.5 ── TR2.5;  TR2.1–TR2.5 ── TR2.6
          S3.1 ── S3.3;  S3.2
Phase 3   TR3.1 ── (S4.5, and the transport tests of Phases 2 and 5);  TR3.2;  S2.1 ── S2.2 ── S2.3
Phase 4   S4.1 ── S4.2, S4.3, S4.4 ── S4.5
Phase 5   TR1.1 + TR1.4a ── the session-plan steps that do not depend on the runtime model
          TR1.4b ── CS3.7, the session plan's Phase 4, and its other runtime, reuse and server steps
Phase 6   TR1.2 + the session plan's network phase ── reuse steps
Phase 7   TR1.3 + the session plan's gwz-cli and gwz-py phases + S4.5 + Phase 6 ── server steps
Phase 8   S3.2 ── S5.1 (TR8.1) ── S5.2 ── S5.3 (TR8.3);  Phase 7 ── TR8.2;  all ── S5.4 ── S5.5;  S5.6
          a TR8.1 miss ── S5.4 retune ── TR8.1 again ── OD8
Phase 9   Phases 2, 4–8 ── S7.1 ── S7.2 ── S7.3 ── S7.4 ── S7.5
Phase 10  S7.5 + S2.3 + TR3.2 ── release steps 1–7 ── post-release check
```

**What can start now:**
- TR3.1 and TR3.2.
- After TR3.1: TR2.1, TR2.2's reproduction and fix, S2.2 and S2.3, and S4.2–S4.4.
- Without a transport build: TR2.3, TR2.4, S2.1 and S4.1.
- TR1.1 once OD3 is decided, then TR1.4a.
- TR1.2's draft.
- TR1.5 and TR1.6.

**Merges and releases from main.** Code reaches main one reviewed step at a time, under four rules:
- **(a) One named switch.** Until Phase 9, the candidate cfg `gwz_transport_candidate` gates:
  - the transport;
  - the `server` command and its options;
  - `SocketCoreBridge`;
  - `--max-retries`;
  - the off switch.

  1.1.0 S7.1 removes the switch, and its completeness check covers every site. `SocketCoreBridge` and gwz-py's `server` command are Python. The switch gates the extension's socket host and socket channel that they need, so both fail closed until Phase 9.
- **(b) No 1.0.x release from main after an ordinary-path session step.** Some session-plan steps change ordinary-build behaviour and are not behind the switch; TR1.4a marks each one.
  - Once the first marked step merges to a product repository's main, no 1.0.x release is cut from that main.
  - A 1.0.x patch is cut instead from the last 1.0.x tag, on a release lineage (AgentProcessRules L1-27), and recorded in `dev-docs/GwzMergeCheckpoint-v1.0.x.md` (L3-13).
  - The retry plan's Phases 1 and 2, already on main, also change ordinary-build behaviour. They do not trigger (b). Phase 2's rule for main governs any release that carries them.
- **(c) Contract changes need Surface.** A defect fix that changes a documented contract, such as TR2.3's `errors` array, is released only with Surface review: S7.5's, or its own if a 1.0.x patch carries it.
- **(d) The checkpoint lists what main carries.** At each patch decision, the program checkpoint lists the unreviewed and unactivated changes on main.

## 7. Decisions for the operator

**Decided 2026-09-27: the operator adopted every recommendation below.**

- **OD1. Python's path.** Recommended: retire decision (c)'s per-operation stage. gwz-py goes straight to the session host, and the amendment's S6.1–S6.3 obligations move as §4 lists.
  - Keeping (c) would cost a design revision and a review that describe behaviour this release would not ship.
  - Its one benefit is a fallback: a transport-only release if the server slips.
- **OD2. Is the server on by default?** Recommended for this release: opt-in, with `--server auto` or `GWZ_SERVER`. In-process stays the default, so reuse across commands needs that one setting. On by default would:
  - fail sandboxed callers who never asked for a server, because a refused server never falls back;
  - put every command in one crash scope;
  - ship a daemon on by default in its first release.
- **OD3. RemPlan-2's three decisions** (its §4). Recommended:
  - apply it;
  - include the ten P3 corrections in revision 4;
  - accept the `reconciled_commit` pin `46e65a9…`. It moves only in a gwz-core commit that changes the allowlist, once gwz-transport is pushed.
- **OD4. Retry plan Phase 3 in this release.** Recommended: yes (TR2.1).
  - If not, the retry help is reverted. That needs a retry-plan amendment, since retry S2.2 and S3.4 pin the help by test.
  - Phase 2's rule for main applies either way.
- **OD5. The sequenced-stream kernel.** Recommended: behind the non-default `unstable-sequenced` feature (TR2.4). The alternatives are reviewing it before Phase 10 step 2, or excluding the module from the published package.
- **OD6. An off switch.** Recommended: yes (TR1.5, TR2.5). Without it, downgrading is the only way back to the native path.
- **OD7. Defect 2.** Recommended: list member failures in the top-level `errors`. The alternative is to document member rows as the only record.
- **OD8. Several channels per SSH connection.** Recommended: only if TR8.1 still misses after S5.4's retune. It is then an operator decision on the measured gap, with the re-run list TR8.1 names.
- **OD9. Proposals G1.** It would read: "Core starts no server or daemon on its own. A driver may host one on request, and no deployment may require one." This is the server design's §8 wording. Recommended: accept it.
- **OD10. Private HTTPS without gh.** Today's design refuses such a caller, so any user whose private HTTPS repositories authenticate through another credential helper breaks on upgrade.
  - Recommended: choose the native route before any connection opens (TR1.6). This amends the transport design's §11 HTTPS cell and 1.1.0 S7.1's gh-only sentence.
  - The alternative is to keep the refusal and point to the off switch.

## 8. Risks

- **Schedule.** The release now waits on the session program (51 steps before TR1.4b adds reuse and server steps), the server, and reuse. Phases 2–4 can finish well before that. Splitting the release later is an operator decision.
  - **If TR1.2 stops under the two-round cap:** the closed recovery is an amendment, approved by the operator, that ships the contract's per-operation session host and the server without reuse. It touches every reuse clause of §1, §2 and Phases 6–10:
    - §1's outcome bullet and its list of decisions;
    - §2's reuse row;
    - TR1.3's and TR1.4b's dependencies on TR1.2;
    - Phase 6;
    - Phase 7's reuse exit;
    - TR8.2 and S5.6's reuse cells;
    - S7.2's reuse row;
    - S7.3's and S7.4's reuse assertions;
    - the post-release reuse check.
  - **A transport-only split:** under OD1 it would need the per-operation Python stage again.
- **A pool shared by overlapping operations.** This is the problem the four retired Python designs failed on. Mitigations:
  - TR1.2 is designed and reviewed before any reuse code;
  - the session host, not a Python lock, owns admission;
  - the pool already scopes ownership by session and operation.
- **A squatted server address.** A client that sends its environment to an unverified listener discloses every secret in that environment. TR1.3's listener verification and Phase 7's exit test cover it.
- **Live authenticated connections in a server.** Any process that passes the peer and sandbox checks can drive them. The macOS sandbox check uses an interface Apple does not document, and it fails closed. TR1.2's eligibility and revalidation rules and TR1.3's trust rules all carry Safety review.
- **Windows.** The server's Windows primitives are new code on the least-tested platform, and dabeest is the only Windows transport host.
- **Main's help text.** It is wrong until TR2.1 lands, and it describes transport-only behaviour until 1.1.0 S7.1. Phase 2's rule for main and §6 cover it.
- **The half-landed rename** blocks transport builds until TR3.1.
- **Idle lifetime.** Under the 60 s idle default, reuse helps only commands that run within a minute of each other. TR1.2 decides whether to amend it.

## 9. Out of scope

- A server on another machine or for another user. Client placement; only frame tags 16–31 stay reserved for it.
- iroh, a physical carrier, or a separate-process wire other than the local server socket.
- Linux ARM64 and macOS x86-64.
- Several channels per SSH connection, unless OD8 brings them in.
- Publishing under the names `libgit2-sys` or `git2`.
- Replacing `scripts/release.py` with gearu in gwz-core, gwz-cli or gwz-py.
- A frozen hard cap changed outside S5.4's rule, or a Python implementation of the pool.
- The workspace route-mapping proposal (`dev-docs/GwzWorkspaceRouteMappingDesign.md`), which is not transport work.
- The release announcement. It uses Phase 8's evidence.
- Running Phase 10 because this file exists. The operator runs it after the phase exits above.

## 10. Review and application

- **Review.** Dual peer-blind Consistency and Safety review of this text, identified by its SHA-256. There is no Surface review of this plan, because it freezes no command, option or API. Surface is carried by:
  - TR1.2, where it changes a user-visible default or option;
  - TR1.3, TR1.5 and TR1.6;
  - TR2.3, under §6(c);
  - the session plan's gwz-py phase;
  - S7.5.
- **On GO,** these status-only edits follow, each with a changelog entry (AgentProcessRules §7.2):
  - **`GwzV110Plan.md`:** "Status: **superseded for its release scope, phases and steps, except those `GwzTransportReleasePlan.md` §4 adopts, by `GwzTransportReleasePlan.md` as of <date>. Historical evidence and already-completed gates remain valid only where the new document says they do**."
  - **`GwzV110PlanAmendment.md`:** "Status: **superseded for §3.1, §3.2, §3.4, §3.7, §3.8, §3.9, §4, §6 and §3.5's sentence on gwz-py's notes by `GwzTransportReleasePlan.md` as of <date>. Historical evidence and already-completed gates remain valid only where the new document says they do**." Its other clauses stay in force as that plan's §4 adopts them.
  - **`GwzPyTransportDesign.md`:** its sentence that S1.1 revises it, and its NO-GO closing condition, point at `GwzTransportReleasePlan.md` §4. TR1.7 supersedes the rest later.
  - **`GwzCoreSessionPlan.md` and `GwzCoreServerDesign.md`:** each gains a sentence that the transport release carries it, pending TR1.4a, TR1.4b and TR1.3.
  - **The program checkpoint** records the acceptance, once another lane's uncommitted edits to it have landed.
- **No authorization.** This plan authorizes no implementation, commit, tag, push or publish.

## Changelog

- 2026-09-27: accepted at SHA-256 `4ec6ba33…` ([Verdict-1](GwzTransportReleasePlan-Verdict-1.md)), with the corrections it records. The operator then adopted every §7 recommendation.
