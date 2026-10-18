# 1.1.0 pre-activation review (operator decision D19): the review package

Status: **accepted at root `90dff378`, gwz-core `aee4c257`, gwz-transport `a27817b9`, gwz-cli `8d909ec4`, gwz-py `da098f5c`, gwz-sspi `d3038882` after `GwzV110PreActivationReview-ReviewCode-3.md` and `GwzV110PreActivationReview-ReviewState-3.md` reported GO (2026-10-16), at round 2 of the two-round cap; this accepts the five D19 objects (TR2.6, S3.1, S3.2, WPP 4.8/TR4.10, WPP 4.2/4.3) only.** Review package of 2026-10-15. One combined Code and State review point before S7.1's switch removal (remaining plan C3/C4), per D19 (`GwzTransportTR18-OperatorDecisions.md`). Two reviewers, peer-blind, the two-round cap. Reports are filed verbatim as `GwzV110PreActivationReview-ReviewCode.md` and `-ReviewState.md` beside this file.

## Tuple (review workspace `/Volumes/projects/limbo/gwz-dev-review19`, a frozen `gwz local clone`)

| Repo | Commit |
|---|---|
| root | `8fd99553cc1239f7242c7eb9e7427f2d8cb6e347` |
| gwz-core | `d6cb37e0f10a354730ef5086633c9f244b58373e` |
| gwz-transport | `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` |
| gwz-cli | `bebcc198baaacf75de9ab45113e4f26dcc051faa` |
| gwz-py | `37e4c151c92b157f8b9a24a49ed00e7ebc119b93` |
| gwz-sspi | `d3038882f7937be023d1835fe1b03e0309260acb` |
| git2-rs | `d13951f7e0bfb6e0efcee1207ac5b140adefa455` (reference only) |
| gwz-core-evidence | `1d8bd29d58d6bd9af8b5405b6f31c2d52208fc02` (evidence, reference only) |

The object is the code **as it stands at this tuple** for the five items below, not their historical diffs. Commit lists are given so the reviewers can read each item's own account; findings are against the tuple.

## The five objects

1. **TR2.6, the Phase 2 transport tree** (TR2.1–TR2.22, S6.1, S6.2). Scope: `GwzTransportReleasePlan.md:306-312`, `GwzTransportReleasePlanAmendment.md:283,354`, `GwzTransportReleasePlanAmendment-2.md:245,553`. gwz-core `1ac248ca`, `26b30ca6`, and `6941c1ae^..ec43f585` (88 non-merge commits, 2026-10-01 to 10-03); gwz-transport `14f0d09`, `36ae2b1`, `a24e70a..1aab733`; gwz-cli `7943532..f925e11`; gwz-py `950064d..5aeff4b`. Never reviewed as code: the retry machine (TR2.1 S3.2–S3.5) and OD18, retry Phases 1–2, typed setup causes, gh challenge reuse, TR2.2–TR2.4, TR2.7, TR2.9–TR2.21, S6.1, S6.2, the splits. Already reviewed in part (do not re-review those parts; they disclaim TR2.6): TR2.1's wire field (`GwzTransportRetryWireField-Review*.md`), TR2.8 (`GwzTransportSshKeyTypes-Review*-1.md`), TR2.22 (`GwzTransportCredentialHelpersAcceptance.md`), TR2.5 CLI and Python (`../../dev-docs/GwzTransportSettingCli-Review*-1.md`, `../../dev-docs/GwzPyTransportSetting-Review*.md`). Known open items to rule on: the nine TR2.5 step-1 interpretations (`../../dev-docs/GwzTransportHandoff.md:256`), the macOS keychain pin left to TR2.6 (`GwzTransportReleasePlanAmendment.md:215`), HandOff 165-170 and 242-247. TR2.2's code location is unclear (nearest `6941c1ae`); TR2.23/TR2.24 are out of scope.
2. **S3.1, the timeout plan's S5.1 review.** `GwzV110Plan.md:155-163`, `GwzRemoteTransportAlphaTimeoutPlan.md:33-70` (§2) and `:245-251`. Implementation `1ac248ca`; 28 later gwz-core commits and gwz-transport `b59e7ec` (SetupClock), `9eef731` changed the clocks. Checklist (V110:155-163): stall resets only on a completed native attempt; idle slices do not reset it; the aggregate does not reset; interaction pauses both; zero disables network deadlines and still disposes; a late result is neither reused nor `PermissionDenied`; `stall` and `aggregate` stay distinct.
3. **S3.2, the Q6 aggregate review.** `GwzV110Plan.md:165-170`. Fix gwz-core `a2a7878d` (the monotonic retirement fix), now at `src/transport_host/session/driver/pump.rs:340-390` and `session.rs:134`, test `cleanup_tests.rs:180`; the runner diagnostic correction in gwz-core-evidence `a05523cd`; the bounded measurement claims in `GwzRemoteTransportQualification.md:76-111`. Controlling: `GwzRemoteTransportQualification.md`, `../../dev-docs/GwzRemoteTransportQualification-DiscoveryState.md`, `-RemPlan.md`, `../../dev-docs/GwzRemoteTransportReleaseReadiness.md:201-243`.
4. **WPP 4.8 / TR4.10, SSPI and gwz-sspi.** `GwzTransportWindowsParityPlan.md:515-541`, `GwzTransportReleasePlanAmendment-2.md:218,225-228`. gwz-core `34f6a403`, `7e16b491`, `aadcd3d9` (and the reviewed WH1 base `efdd0a2c`, `28f564a6`, `398158b3`); gwz-sspi all 29 commits to `d3038882` (its `src/` unchanged since the piecemeal-accepted `582ec00`: `../../dev-docs/GwzSspi*Acceptance.md`); hosts gwz-cli `src/worker_host.rs`, gwz-py `native/src/worker_host.rs`. Attack list (A2:225-228): the OD16 hazard, SSPI secret ownership, secret-leak rows (`Debug`, logs, errors), channel binding, the eight-round bound per operation, rejection terminal, Digest refused, no caller of explicit-identity SSPI or WDigest (D3). TD11's policy (no cross-user or logon-session refusal) is to be judged explicitly. Nobody has read: `https_worker/native/owners.rs`, `native/sspi.rs`, all of gwz-sspi at this tuple.
5. **WPP 4.2/4.3, the WH2 helper process owner (the dual on the final text).** `GwzTransportWindowsParityPlan.md:457-481`, `GwzTransportWindowsHelperOwnerContract-DRAFT.md` (C1–C9, decisions OQ-A to OQ-G). gwz-core `10c078ce`, `0c56fdb1`, `115993d7`, `ba722da0` (round-2 remediation, never re-reviewed), `7e791915` (4.3). Round 1 NO-GO both; round 2 Safety+State GO, Code+Consistency NO-GO on N1–N3 (`GwzTransportWindowsHelperOwner-Review*-2.md`); `ba722da0` claims to close N1–N3. Current files: `https_auth/process_tree*`, `owner.rs`, `pipes.rs`, `runner*.rs` (4.2), `executable.rs`, `lookup.rs`, `file_worker.rs`, `view*`, `secret.rs` (4.3).

## The ruling recorded with D19

Step 5.6's accepted review counts as the Phase 1, 3, 4, 4.4 and 3.5 reviews (Phase 1 read on the State axis only). This review does not re-open those.

## What 5.6 already read (skim, do not re-attack)

State axis, fully: `setup_retry.rs` `classify`, `setup_retry/machine.rs`, `setup_retry/limit/**`, `ssh_connection.rs`, `ssh_key_container.rs`, `agent_job/control.rs`, `control/room.rs`, `ssh_network.rs`, `kex_retry.rs`, the post-`ba722da0` helper-stack diff (4.3), `native/exchange.rs`, `machine_proxy*`. Code axis: Pageant and `agent_source`, `ssh_home.rs`, `capacity.rs`, `notes.rs`, `native_commit/**`. Nobody has read in this form: `session/driver/pump.rs`, `ssh_worker*`, `placement_endpoint*` (outside two diffs), `ssh_destination.rs`, `ssh_admission.rs`, `https_opening.rs`, `https_pool*`, `https_endpoint/*` (outside call sites), gwz-py `client_host*` and `dispatch/*`, `transport_setting`, the gh-reuse code, `process_tree/**`, `owner.rs`, `native/owners.rs`, `native/sspi.rs`, all of gwz-sspi.

## Size and the split

About 61,000 production lines and 80,000 test lines are in scope; no reviewer can read all of it. Each report states what it read fully, sampled and did not read. Generated files (`protocol/{generated,candidate_generated}.rs`, gwz-transport `src/protocol.rs`, gwz-sspi's generated protocol) are checked by their regeneration scripts, not read.

- **Both reviewers, read fully:** object 4 (gwz-core `https_worker/native*` 1,165 lines and its `precedence`, `rejection`, `real_sspi` tests; gwz-sspi `src/secret.rs`, `values.rs`, `worker/**`, `supervisor/**`; the two `worker_host.rs`).
- **Code reviewer, in priority order:** S6.1 and S6.2 entries (`cancellable.rs`, `request.rs`, `transport_scope.rs`, `session_host/*`, gwz-py `client_host*`, `route/*`, `error.rs`); wire fields (`OperationPolicy.max_retries`, `FailureDetail`, `SetupFailureCause`); TR2.11 `transport_binding.rs`; TR2.3/TR2.19 contract changes; TR2.5 plumbing in core, CLI and Python; TR2.4 (`binding.rs`, `sequenced.rs` cfg); TR2.12 inventories and checkers; TR2.16/TR2.18 `ssh_destination.rs`; TR2.7; the WH2 interfaces (`process_tree` arms, 4.3 path rules); the splits and dead-code removals last.
- **State reviewer, in priority order:** S3.1's clocks (`agent_job*`, `ssh_setup.rs`, `ssh_network.rs`, `session/driver.rs`, gwz-transport `pool/setup_clock*`) against the checklist; the Q6 fix (`pump.rs:340-390`); the retry machine and OD18 (`setup_retry/*`, `placement_endpoint/{completion,admission}.rs`, `https_endpoint/retry.rs`); the WH2 Job owner and `ba722da0` (`process_tree/**`, `owner.rs`, `runner*.rs`); gh reuse (`https_connection.rs`, `https_worker/{prepare,serve}.rs`, `https_endpoint/*`); TR2.9/2.10/2.13/2.14/2.17 (concurrency, permits, cancel); TR2.22's helper process and shared clock; S6.2's GIL and exit paths.

## Deferred (outcomes not to be re-litigated; shape in scope)

Operator decisions TD1–TD14, D1–D22, OD5, OD7, OD13–OD18, the 14 decisions of 2026-10-02, TR1.5 D2/D3/E3, TR1.6 OQ1–OQ7, the WH2 contract's OQ-A to OQ-G, "no minimum Git version". Already accepted objects: step 5.6 (and the D19 ruling), the adaptive limit machine (`GwzTransportAdaptiveStep-ReviewCodeState*.md`), the HTTPS fixed-cost fix, native commit and tag. Also deferred: TR8/S5 speed and memory evidence, Phase 6 evidence, the Pageant 0.83 rows.
