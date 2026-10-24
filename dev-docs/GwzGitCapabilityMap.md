# gwz-git capability map (Phase 5, step 5.1)

Date: 2026-10-24. Status: **DRAFT for the Phase 5 lane owner; a document, no code.**
It is the finite mapping that [the Git boundary plan](../../dev-docs/GwzGitBoundaryRemediationPlan.md) §4 asks
for (intended capability -> current implementation -> actual production consumer -> target gwz-git
API), under [the convergence plan](../../dev-docs/GwzTransportReleaseConvergencePlan.md) §4 step 5.1.
Read at gwz-core `461ddf4b`, gwz-git `a9d7ee0`, root `9fbe5108` (lane `p5-gitmap`).
Target APIs are **proposals**; none is frozen. They do not extend
[GwzGitLibraryApi.md](GwzGitLibraryApi.md); each row's first step must amend that guide first
([design](GwzGitLibraryDesign.md): "affected clauses must be updated before any subsequent behavior expansion").

## 0. Method and conventions

- Paths are relative to `gwz-core/` unless they start with `gwz-` or `dev-docs/`. `P` = production lines
  (physical, comments and blanks included; inline `#[cfg(test)]` modules counted as tests). `T` = test lines.
  Counts are `wc -l` and a cut at the first inline test module, so they are good to a few percent.
- "Production caller" means a call site outside `cfg(test)` code, test directories and `*_tests.rs`, found by
  text search (`.name(` / `::name(`) over `src/`, `gwz-cli/src`, `gwz-py/src`. It is lexical, not name resolution.
  The candidate cfg (`gwz_transport_candidate`) is production (`scripts/checks/check_process_globals.py` header).
- gwz-git today: 367 source lines in `gwz-git/src/` (`Repository::open_exact`, `git_dir`, `common_dir`,
  `work_dir`, `object_format`, `read_commit`; `ObjectId`; `CommitRecord`, `Signature`; `Error`, `ErrorKind`
  with four kinds, `NativeDiagnostic`). **No file in gwz-core, gwz-cli, gwz-py or `scripts/` names the crate**
  `gwz-git` (only `gwz-git2`, the fork); `gwz-git/Cargo.toml` names the same fork path as core (`Cargo.toml:89`).
- Not gwz-git (stays out of the map on purpose): everything under `src/git/endpoint/` except the three
  libgit2-facing files in G6 (credential sources, pools, helpers, fixtures: the Connection/credential side);
  `src/git/url_scheme.rs`, `git_host.rs`, `regular_file.rs` (URL and filesystem policy, no repository access);
  `transport_support/identity.rs` (411 P; SSH identity selection and the remote-identity setting: the 4.3
  credential seam); the native-route credential callbacks `transport_support.rs:173-333`.

## 1. The rows (compact)

Class: **A** local, can move alongside Phases 1-4; **B** local, after the named dependency;
**C** network, after Phase 4.2; **D** residual, no move proposed (needs a recorded disposition).

| Row | Capability | Cls | P moved | T moved | Production caller (file:line) | Target gwz-git API (proposal) | Order |
|---|---|---|---:|---:|---|---|---|
| G9 | `git_error` translation, open modes, `is_repository` | A | ~50 | small | `git/verify_checkout_state.rs:132` (every row; also `diff/git_diff.rs`, `diff/render/patch.rs`, 3 files in `checked_artifact/`) | `Error` + additive `ErrorKind::{NotARepository, ..}`; `OpenMode::{Exact, Environment(&Env)}` | 1 |
| G2 | Refs and reads: `head`, `read_ref`, `tag_list`, `commit_exists`, `read_file_at_commit`, `is_ancestor`, `ahead_behind`, `merge_base`, `changed_paths_between`, `status`, `remotes` | A | ~345 | ~1,700 mixed | `workspace_ops/handle_fetch.rs:429,437`, `status/status_member.rs:125-187`, `diff/classify.rs:206-228`, `diff/tagged.rs:63`, 40+ files for `head`/`status` | `Repository::{head, resolve, tag_names, commit_exists, blob_at, is_ancestor, ahead_behind, merge_base, changed_paths, status, remotes}`; owned records | 2 (G2a: `head`, `resolve`, `tag_names` first, G1 needs them); **landed in step 5.3, §8** |
| G1 | Native commit and tag (+ CLI fallbacks) | A | ~2,210 | ~4,360 | `workspace_ops/handle_commit.rs:186,260`; `handle_tag.rs:221,259` | `commit::commit`, `tag::{create,delete}` taking `&Services{environment, clock, process, ..}`; returns `Outcome{id, route}` | 3 (in 3 mechanical sub-steps, §2.1) |
| G3 | Worktree, branch and index mutation: `stage_paths*`, `checkout_*`, `reset_hard`, `switch_branch`, `branch_*`, `fast_forward`, `create_repo` | B (G2) | ~480 | unsplit (`tests/g10.rs` 175 + parts of `g01`, `g12`) | `handle_stage.rs:139,198`, `handle_branch.rs:213-310`, `handle_materialize/apply.rs:131-143`, `pull_head_member_preflight/root_lock.rs:107-158` | `index::stage`, `branch::{list,create,delete,switch}`, `checkout`, `reset_hard`, `Repository::init` | 5, **outside plan §4's rows** |
| G4a | Push planning: `prepare_push`, `last_known_ref` | B (G2) | 214 | ~31 + `g08` | `workspace_ops/push_member.rs:200`, `handle_tag.rs:122`, `push_state.rs:140` | `remote::plan_push`, `remote::last_known_ref` | 4 |
| G4b | Local-peer transfer: `fetch_anonymous`, `push_anonymous`, local push remote, tracking-ref write | B (G2, before G5) | ~281 | ~84 inline + `g08` mixed | `local_clone/transport.rs:109,121` | `remote::local::{fetch,push}` | 4 |
| G5 | Fetch, push, ls-remote, clone, `read_remote_file` adapters | C | ~590 (est.) | ~1,850 mixed | `handle_fetch.rs:430`, `push_member.rs:331,387`, `handle_tag.rs:275,335`, `handle_materialize/apply.rs:63`, `publication.rs:61,251,324` | `remote::{fetch,push,ls_remote,clone}(&Repository, .., &dyn Connection)` | 7, after 4.2 |
| G6 | Per-remote stream binding to libgit2 | C | ~815 (est.) | ~856 | `git/gitbackend/transport_support.rs:149-170` (`remote_callbacks`) | `remote::binding` (SmartSubtransport over a port) | 7, with G5 |
| G7 | History cursor (`gwz log`): revision resolve, revwalk, path walk | B (G2, L4 design) | ~350 | ~3,700 mixed | `operation/commit_log/handler.rs:336` -> `gwz-cli/src/log_exec.rs:117` | `history::{Walk, Cursor}` | 6, **outside plan §4's rows** |
| G8 | Single-repo diff: comparison resolve, manifest, patch render | B (G2) | ~915 + 271 types | ~1,440 | `diff/handle_diff.rs:265-271,321` | `diff::{resolve, manifest, render_patch}` | 6, **outside plan §4's rows** |
| G10 | Residual: merge, preservation, stash, scoped commit, recovery | D | ~5,700 | ~5,970 | `workspace_ops/merge/**`, `pull_head*`, `handle_stash/` | none proposed | decision, §4 |

Estimated rows (est.) mix libgit2-facing code with host glue in the same file; §2.5 and §2.6 give the split.
Plan §4's estimates were 5.2 "~500 moved + 100", 5.3 "~300 + 80", 5.4 "~400 + 300": G2 agrees, **G1 is four times
larger**, G5+G6 about three times (moved code is counted separately from new code in plan §4, so this is a
size warning, not a budget breach).

## 2. Row evidence

### 2.1 G1 native commit and tag

- **Implementation.** `git/gitbackend/native_commit.rs` (~150 P) picks the route; `native_commit/` holds
  `route.rs` 444, `system_config.rs` 359, `ident.rs` 250, `exec.rs` 205, `staging.rs` 141, `environment.rs` 124,
  `message.rs` 116, `hooks.rs` 109, `config_view.rs` 104, `notes.rs` 71 (1,923 P). CLI fallbacks:
  `repository.rs:294-334` (`commit_cli`, spawn at `:301`), `refs.rs:167-203` (`tag_create_cli`, `:174`),
  `:244-284` (`tag_delete_cli`, `:245`), plus self-verify `refs.rs:205-228,272-284`. The module needs core
  only for `crate::git::git_error` and `ModelError` (`exec.rs:3-4`, 15 references; `config_view.rs` 2): the move
  is close to mechanical.
- **Route reasons for every CLI spawn** (`native_commit/route.rs:21-52`, `CliReason`): `Forced`, `Environment`,
  `CannotOpen`, `Bare`, `ObjectFormat`, `Config`, `SystemConfig`, `SystemFile`, `Hook`, `Signing`, `InProgress`,
  `IndexState`, `Identity`, `Message`, `AllCommit` (`commit -a` always runs Git: operator 2026-10-13,
  `native_commit.rs:10-13`), `TagName`, `HeadShape`. Each is reported through `RouteNotes` (pub, used by
  `gwz-cli/src/globalargs/dispatch.rs:40-43`).
- **Tests.** `native_commit/tests/*.rs` + `system_config/tests.rs` 3,133, `tests.rs` 22, inline ~336;
  `commit_tag_characterization.rs` 323 and `commit_tag_identity_characterization.rs` 420 (both oracles against
  Git 2.52); `tests/g09.rs` 125 (tags). Commit messages record 82 native_commit tests, 79 on Windows (9d872e88).
- **Public types today.** `GitCommitResult{commit}`, `GitTagResult{name, commit}`, `RouteNotes` (a closure
  wrapper), `Git2Backend::with_route_notes`. `GitEnvironment` is crate-internal. No `git2` type is public;
  internally `route::open` (`route.rs:82`) returns `git2::Repository`, and `HeadState`/`TagPlan` hold `git2::Oid`.
- **Platform splits (production).** `hooks.rs:25` (unix exec bit vs Windows name and `.exe`), `route.rs:99`
  (`includeIf` is Git's on Windows), `staging.rs:47` (autocrlf default), `system_config.rs:40` (`Host::current`),
  `:79` (`is_program`), `:224` (Windows `%PROGRAMDATA%`); `git/spawn_gate.rs:47` (exclusion on Windows only).
  18 further gated lines are in the tests. **No Windows-parity inventory row covers any of them** (§3, F4).
- **Seam.** Local. The CLI fallback needs the host's `SpawnGate` (`git/spawn_gate.rs:122,131`; D25, D52): gwz-git
  defines a narrow `ProcessGate` trait and core implements it with the gate (Phase 4.1's `ProcessSafety` port
  is the later owner). The environment snapshot (`GitEnvironment::process()`, `environment.rs`, `env::vars_os`,
  a `debt` entry in `process_globals_allowlist.json`) and the single injected `now` (`exec.rs:33-40`) become
  explicit inputs; core keeps the capture.
- **Proposed order inside G1.** (a) route rule: `route.rs`, `system_config.rs`, `environment.rs`, `config_view.rs`,
  `hooks.rs` (1,140 P, read-only decisions, no CLI); (b) native execution: `exec.rs`, `ident.rs`, `message.rs`,
  `staging.rs`, `notes.rs`; (c) CLI fallbacks and the spawn port, deleting `commit_cli`/`tag_*_cli` from core in
  the same commit that core delegates. Each is above 500 moved lines: mechanical moves (`rust-split`) separate
  from the behaviour edit.

### 2.2 G2 refs and reads

- **Implementation (line = function start).** `refs.rs`: `tag_list:230`, `read_ref:286`, `ahead_behind:306`,
  `is_ancestor:324`. `repository.rs`: `is_repository:7`, `commit_exists:15`, `read_file_at_commit:28`,
  `status:155`, `status_with_options:159`, `head:200`. `comparison.rs`: `merge_base:4`,
  `changed_paths_between:20`. `transport.rs:227` (`remotes`). Shared helpers: `repository_support.rs:3`
  (`open_repo`), `git/verify_checkout_state.rs:21` (`repo_head`), `:8` (`find_remote`), `repository_support.rs:206-223`.
- **Callers (production sites / files).** `head` 68/43, `status` 64/40, `is_repository` 53/32, `read_ref` 30/21,
  `read_file_at_commit` 13/10, `is_ancestor` 12/7, `tag_list` 5/2, `commit_exists` 4/4, `merge_base` 3/3,
  `changed_paths_between` 2/2, `ahead_behind` 1/1, `remotes` 17/13, plus `repository_paths` 8/4 and
  `repository_index` 1/1 (both `checked_artifact/`, e.g. `capability/pre_catalog/provider/retained.rs:199`,
  `index.rs:26`). All reach the backend through the `GitBackend` trait, which stays in core and delegates:
  **no caller edit is needed**, the same-commit delegation is inside `Git2Backend`.
- **Public types.** Owned only: `GitHeadState`, `GitStatus`, `GitFileStatus`, `GitRemote`, `GitAheadBehind`,
  `GitRepositoryPaths`, `GitIndexSnapshot` (`gitbackend/types.rs`). The leak is crate-internal:
  `pub(crate) open_repo -> git2::Repository` (`repository_support.rs:3`), used outside `src/git` only by
  `diff/handle_diff.rs:265,321` (G8).
- **API gaps against G0.** (1) `read_ref` is `revparse_single` (`refs.rs:286`), a revision expression, while G0 says
  `read_commit` "does not peel tags or resolve refs" (API guide, "IDs and commit data"): a `resolve` primitive
  is an API amendment. (2) `is_repository` needs "not a repository" apart from other open failures
  (`repository.rs:7-13` maps `ErrorCode::NotFound` to false): `ErrorKind::RepositoryOpen` carries a bare
  native code today; add a kind rather than ask core to compare `-3`. (3) Open mode, see G9.
- **Platform.** No `cfg` in these functions; none in the parity inventory.
- **Tests.** `repository_contract_tests.rs` 517 (fake vs native contract), `tests/g01.rs` 1,114 (28 tests, mixed
  with create/commit_exists/CRLF cases), `g14.rs` 41. Splitting by test is G2's first red step.

### 2.3 G9 error, open modes (cross-cutting, lands first)

- `git_error` (`verify_checkout_state.rs:132-141`) is the only `git2::Error -> ModelError` mapping: three codes
  (`GitCommandFailed`, `CredentialHelperTimeout`, `ExternalToolMissing`) and it copies `error.message()`.
  The leak of libgit2 parse text through it is plan §10 D-F (fixed in 6.2) and D50; the mapping to `ModelError`
  stays in core, gwz-git returns `Error` with kind and `NativeDiagnostic{code, class, message}` and the code and
  class remain distinct domains (design, "Errors").
- Three open conventions exist: G0 `open_exact` = `NO_SEARCH`, ignores `GIT_DIR` (`gwz-git/src/repository.rs`);
  core `git2::Repository::open` (`repository_support.rs:4`); native commit `NO_SEARCH | FROM_ENV`
  (`route.rs:82-89`) so Git's own variables choose the repository. G1 cannot run on `open_exact`.

### 2.4 G3, G4a, G4b (local, after G2)

- **G3** (`repository.rs:64` `create_repo`, `:76` `reset_hard`, `:102` `checkout_commit`, `:122` `checkout_branch`,
  `:205` `stage_paths`, `:247` `stage_paths_allowing_other_conflicts`; `refs.rs:6` `fast_forward`, `:48-165` branch
  functions; `verify_checkout_state.rs` 145). All call `verify_checkout_state` (AD1 self-verify). Callers are
  workspace sequences (materialize, branch, pull, stage) that keep their own ordering and policy; the per-call
  Git step is what moves. Not in plan §4's rows; listed because it is single-repository Git with production
  callers and shares `open`, `head` and `status` with G2.
- **G4a** `push_plan.rs:4` (`prepare`), `:133` (`last_known_ref`): reads local refs and remote-tracking refs; the
  `GitPreparedPush{remote, url, refspecs}` record is owned (`types.rs:320`).
- **G4b** `transport.rs:279-303` (`admitted_local_peer`), `:305-399` (`fetch_anonymous`, `push_anonymous`),
  `:531-690` (`local_push_remote`, `served_by_local_transport`, `update_tracking_refs`, `tracking_ref`,
  `glob_matches`). Local transport only, no connection. `perform_push` (`:450`, G5) shares `local_push_remote`
  and `update_tracking_refs`: move G4b first so G5 does not carry it. **Platform:** `transport.rs:587`
  `cfg!(windows)` in `served_by_local_transport`; `transport.rs` is not a parity-inventory root (F4).
- `GitBackend::add_remote` (`transport.rs:245`) has no production caller (F7).

### 2.5 G5 fetch, push, ls-remote, clone

- **Implementation.** `transport.rs`: `clone_repo_named:25`, `fetch:72`, `tag_fetch:111`, `ls_remote:151`,
  `ls_remote_url:173`, `advertised_refs:194`, `push:401`, `push_prepared:427`, `perform_push:450`,
  `read_remote_file:693`, `clone_error:742` (~488 P; with G4b removed). Options and callbacks:
  `transport_support.rs:72-170` (~100 P). Observation sink `transport_observations.rs` (165 P; core's
  reporting model: stays in core).
- **Stays in core / credential seam** (not moved): native-route credential callbacks `transport_support.rs:173-333`
  (agent, key file, helper: credentials inside the Git adapter, against R7), `identity.rs` 411 P, the
  process-wide libgit2 timeout `TIMEOUT_STATE` (`transport_support.rs:16`, `permanent`, `imposed_by libgit2` in
  the process-globals allowlist): gwz-git must not grow a second owner of that setting.
- **Public types.** Owned: `GitFetchResult`, `GitPushResult`, `GitPreparedPush`, `GitRemoteRef`, `GitCloneResult`,
  `TransportObservations` (pub). `git2::RemoteCallbacks`, `Remote`, `FetchOptions` stay internal.
- **Callers.** `fetch` 14/7, `push_prepared` 3/2, `ls_remote` 3/2, `ls_remote_url` 5/2, `clone_repo_with_progress`
  3/3 (`handle_repo_lifecycle.rs:104`, `handle_init_from_sources.rs:167`, `handle_materialize/clone_workspace.rs:142`),
  `clone_repo_named` 1/1, `read_remote_file` 1/1 (`clone_workspace.rs:69`), `tag_fetch` 1/1.
- **Seam.** Needs the Phase 1-4 owner: contract §12 `run_git_via_gwz_git(&op)` takes the operation capability, "cannot
  fall back by constructing a private host", and the libgit2 bridge waits on WouldBlock under the operation's bounds
  (`dev-docs/GwzConnectionApiContract.md:779-783`). Today a backend with no host context installs nothing and libgit2
  uses its own transports (`transport_binding.rs`, `configure`): that second route must not survive the move (Git plan G3).
- **Tests.** `transport_candidate_tests/` 1,773 + `https_transport_binding_tests.rs` 208 (candidate cfg, mixed with host
  tests), `transport.rs:770-853` inline 84; `tests/transport_native`, `tests/transport_backend` (Python harnesses).

### 2.6 G6 per-remote stream binding

- **Libgit2-facing (move):** `git/endpoint/ssh_remote.rs` 123 (`SmartSubtransport`, `OpenStream`),
  `https_remote.rs` 303 (`OpenRpc`, `install`), `stream_io.rs` 288 (`BlockingStream`; `:246` maps `WouldBlock` and
  `WaiterCapacity` to `io::ErrorKind::Other`, the case Git plan G3 names), and in `transport_binding.rs` (362 P)
  `configure` and `repository_refused`/`credential_helper_*` (~100). **Host glue (stays, adapter over the
  Connection):** `Runtime`, `HostRoute`, `HostHttpsRoute`, https policy, the http warning (~260).
- **Candidate gating.** The whole binding is under `cfg(all(gwz_transport_candidate, any(unix, windows)))`
  (`transport_binding.rs:9`); the `else` arm (`:332`) is empty stubs. `build.rs:8` declares the cfg.
- **Windows rows.** Parity inventory `4.4`: `transport_binding.rs` `cfg!(windows)` x3 (paired),
  `https_transport_binding_tests.rs` x2 (paired). `src/git/endpoint` is a root; the three files carry no gates.
- **Tests.** `https_remote_tests.rs` 420, `stream_io_tests.rs` 228, `https_transport_binding_tests.rs` 208.
- **Decision before 4.2 starts** (F6): 4.2 moves "the host, bindings and pool-owner glue" into
  `gwz-connection`, which must not depend on git2; these three files and `configure` are git2-facing and Git
  plan §3 assigns the per-remote binding to gwz-git. And gwz-git (its own repository) cannot depend on a
  `gwz-connection` that lives under `gwz-core/candidate-crates` (D-C) without inverting the repository DAG.

### 2.7 G7 history cursor and G8 diff (single-repository slices only)

- **G7.** Git: `operation/commit_log/request.rs:461` (open), `:602-680` (`resolve_walk`: HEAD, `merge_bases`),
  `:682-720` (`resolve_oid`: `revparse_single` + peel), `mod.rs:178-262` (revwalk cursor), `:322-398` (**PathWalk,
  CLI**), `:516-553` (`entry`, `identity`); ~350 P. Coordination (stays): request and range parsing, filters,
  merge and coalesce across members, paging, spool (`aggregate`, `coalesce`, `filter`, `handler`, `merge`,
  ~2,300 P). The CLI spawn: `mod.rs:330` `git rev-list --max-count=1 --skip=N` once per returned commit, when
  `pathspecs` is non-empty (`mod.rs:186-187`); reason: libgit2's revwalk has no path-limited history
  simplification (NoFallbackPlan §7, design L4); `debt` entry in the process-globals allowlist. Platform: none in
  production. The log path opens `git2::Repository` itself and takes the concrete `&Git2Backend` only for its spawn
  gate (`handler.rs:336-344`), bypassing the trait. The file header `commit_log/mod.rs:1-7` still allows `dead_code`
  for "later log-engine steps" although the path is live.
- **G8.** Git: `diff/git_diff.rs` 545 (`resolve_comparison:48`, `diff_repo:221`, `build_manifest:359`,
  `rename_similarity:444`), `render/patch.rs` 277, `output.rs:113-205` (re-diff per entry), `model.rs` 271 (owned
  records, the natural gwz-git types). Coordination (stays): `plan.rs` 748, `operands.rs` 420, `classify.rs` 233,
  `tagged.rs` 138, `handle_diff.rs` 619, `log_service.rs` 392, `render/{names,options,summary}.rs`. Public leak:
  `gwz_core::diff::{diff_repo, resolve_comparison}` and `render::{render_entry, render_raw_entry}` take
  `&git2::Repository` (`diff/mod.rs:30`, `render/patch.rs:31,75`), and `ProducerTarget.repo: git2::Repository`
  is pub (`output.rs:49`). The trait methods `diff_manifest` and `resolve_comparison` (`comparison.rs:51-68`) have no
  production caller: the diff path never goes through the trait.
- Neither is a row in plan §4. Both are single-repository Git with production callers and both bypass the
  `GitBackend` trait, so the proposed guard (§6) must name them or they stay invisible.

### 2.8 Every `git` CLI spawn in `src/git/` and its route reason

| Site | Command | Reason it runs | Gate |
|---|---|---|---|
| `repository.rs:301` | `git commit [-a] -m` | `CliReason` (above); `-a` always | `spawn_gate.output` |
| `refs.rs:174` | `git tag [-a/-s] [-m]` | `CliReason`: hook, signing, config, identity, state | `spawn_gate.output` |
| `refs.rs:245` | `git tag -d` | `CliReason` (reference-transaction hook, config, state) | `spawn_gate.output` |
| `operation/commit_log/mod.rs:330` | `git rev-list` | pathspec history (G7) | `gate.spawn` (D25) |
| `transport_support.rs:313-321` | libgit2 `Cred::credential_helper` spawns configured helpers | native-route HTTPS credentials | `spawn_gate.hold()` whole call |
| `endpoint/helper_script.rs:83`, `endpoint/https_auth/*` | credential helper programs | Connection/credential seam, not Git | helper slots |

All of `Command::new("git")` in `src/git/gitbackend/` outside tests is the three lines above. Test-only spawns
(fixtures, oracles) are excluded.

## 3. Platform splits and the Windows-parity inventory

- Inventory roots (`scripts/checks/windows_parity/meta.json`): `src/git/endpoint`, `src/transport_host`,
  `src/transport_setting*`, `src/git/gitbackend.rs`, `src/git/gitbackend/transport_*`,
  `src/git/gitbackend/https_transport_*`. These are also the Windows-receipt trigger paths
  (`scripts/windows_lane_check.py:112-118`, plus `Cargo.toml`, `Cargo.lock`, `prepare.py`, `.github/*.commit`).
- Rows that cover the map: `4.4.json` for `transport_binding.rs` (`cfg!(windows)` x3) and
  `https_transport_binding_tests.rs` (x2); `3.1.json` `home_rows.rs`, `url_parity.rs`; `4.11.json` `drivers.rs`.
  Nothing else under `src/git/gitbackend/` or `src/diff`, `src/operation/commit_log` has a row.
- **F4.** The eight production platform gates of G1 and G4b (listed above) are outside every root, so a change to
  them never triggers a Windows receipt, and after a move to gwz-git they leave core's scan altogether. The
  trigger that survives is the pin bump (`Cargo.lock` is in `TRIGGER_EXTRA`). gwz-git needs its own Windows
  run for its platform gates (the Windows host is dabeest; libgit2 1.9.7 vendored, Git for Windows 2.55 as the
  oracle per 9d872e88).

## 4. Residual (G10): what stays in `src/git/`, with the disposition the plan asks for

Not proposed for Phase 5; the Git plan §3 requires a "concrete responsibility and disposition" for each allowed
residual use. The proposal below is a **decision for the operator**, not made here.

| Group (P, T lines) | Responsibility | Why not now | Proposed disposition |
|---|---|---|---|
| merge/rebase: `merge_prepared.rs` 466, `merge_recovery.rs` 502, `merge_support.rs` 508 | prepared merge, recovery, abort, analysis | callers are the v1 merge lifecycle (`workspace_ops/merge/**`); semantics are workspace-recovery-coupled (design: "contracts include preservation and workspace recovery") | core-retained; owner: merge engine; exit: revisit after Phase 6 |
| preservation: `preservation*.rs` ~3,250, `stash*.rs` 164, `recovery_support.rs` 437 | preservation images, backup refs, stash | same | core-retained as above |
| scoped commit: `scoped_evidence.rs` 306, `scoped_support.rs` 215 | `commit_gwz_paths_checked`, rollback | GWZ-path policy, not ordinary commit | core-retained; may call G1/G3 primitives later |
| `Git2Backend` factory/policy: `backend.rs` 228, `factory.rs`, `contract/` | trait and construction | design keeps the trait in core | stays |
| Tests: `src/git/tests/g11,g12,g13,g15*,g16,g17` ~5,970 | the above | | stay |

Trait methods with **no production caller outside `src/git/gitbackend/`** (verified by search, F7): `add_remote`,
`merge_upstream`, `commit_matches_merge`, `commit_merge_resolution_checked`, `create_backup_ref`,
`delete_branch_target_checked`, `index_matches_candidate_files`, and the trait entries `diff_manifest`,
`resolve_comparison`. `rewrite_root_managed_index_checked`, `root_managed_index_matches`, `root_preservation_image`,
`stash_for_merge_preservation`, `validate_root_preservation_spec` are called only inside `preservation_root.rs` (G10 internals).
Dead code is to be removed after proof; this step edits nothing, so it is a finding.

Direct `git2` use in production **outside `src/git/`** (the register §6 starts from): `transport_setting.rs:310-317`
(`Config::open`), `local_clone/adapters/{git_config.rs:69, inventory.rs:220, generated_marker.rs:15,83}`,
`operation/commit_log/{mod.rs, request.rs}` (G7), `diff/{git_diff.rs, output.rs, render/patch.rs}` (G8). A second
local-read implementation exists as a crate: `crates/repo-inspect` (3,044 lines, git2 direct, consumers
`local_clone/adapters/{disposal,copy_witness,install}.rs`), outside Phase 5; it duplicates part of G2 and needs a
disposition (adapter over gwz-git, or permanent).

## 5. Findings for the lane owner (evidence, not fixes)

- **F1 size.** G1 is ~2,210 P and ~4,360 T to move (plan: ~500). Confirmed defect in the estimate, not in the design.
- **F2 recorded decision.** The operator decision of **2026-10-14** (`history/parked/GwzNoFallbackPlan.md` §6, quoted
  in 8cefbe40) says native commit and tag "stay a module of gwz-core ... no code change follows". Plan §10 D-G
  (2026-10-24) authorises 5.1-5.3 and 5.2 moves them. The 10-14 text is not annotated. Needs one recorded line:
  "superseded by D-G for G1", before 5.2 starts. Nothing in this map assumes it.
- **F3 API.** G1 and G2 need three additions the G0 guide excludes: environment-honouring open, a revision
  `resolve`, and a "not a repository" kind (§2.2, §2.3). One amendment to `GwzGitLibraryApi.md`, reviewed, first.
- **F4 Windows coverage** (§3).
- **F5 spawn ratchet.** `check_process_globals.py` scans core's `roots` (`src/lib.rs`, `crates/*/src/lib.rs`). Moving
  `commit_cli`, `tag_*_cli` out of core removes them from the ratchet silently. gwz-git needs its own allowlist
  (precedent: `process_globals_allowlist_gwz_transport.json` with `reconciled_commit`, wired at `run_tests.py:191`).
- **F6 DAG and 4.2 overlap** (§2.6): decide before 4.2 starts which side owns `ssh_remote.rs`, `https_remote.rs`,
  `stream_io.rs`, and where the connection capability type lives (a gwz-git-defined port that core implements over
  `gwz-connection` keeps the DAG; or `gwz-connection` gets its own repository, which D-C allows with a second
  consumer, and gwz-git would be it).
- **F7 dead trait methods** (§4). **F8 trait bypass:** `commit_log` and `diff` open `git2` directly and leak it.
  **F9** `Git2Backend::new()` is constructed per call in `diff/classify.rs:204-225`, `diff/tagged.rs:63`: new gwz-git
  capabilities reached that way have no host context.
- **F10 second reader** `crates/repo-inspect` (§4).

## 6. The Git plan §4 questions, from repository evidence

Search limits: I read the active documents of both dev-docs directories, `history/reference/GwzGitLibrary*.md`,
`history/parked/{GwzNoFallbackPlan,GwzGitLibraryNextPackages}.md`, `CurrentProgramCheckpoint.md` around the cited
lines, and ran text searches for `gwz-git` over core, CLI, Python and scripts. Other archived files were not read.

**1. What G0 promised.** A read-only foundation: exact open, paths, object format, full ids, `read_commit`, owned
errors (API guide). The guide says it "does not provide commit/tag/history/fetch operations yet"; the design lists
L3 commit/tag, L4 history, L1/L2 fetch and remote adapter as "requirements, not frozen signatures", with "No
production dependency switch" in G0's integrator bullet and "none is authorized by a vague 'move the remaining
backend' instruction". Delivered 2026-09-21: 367 source and 652 test lines (`gwz-git` `fcf5296`, `aa77c2c`).
The next packages (C1, H1, Q1; accepted the same day) were characterization with "0 production" lines. No document
read assigns an owner or date to a production-adoption package.

**2. When behaviour intended for the library landed in core.** The CLI routes pre-date the library: the next-packages
scope cites them at `repository.rs:291-332`, `refs.rs:164-263` and `commit_log/mod.rs:311-350` (2026-09-21).
Recorded choices: operator decisions of 2026-10-12 (libgit2 fast path for commit and tag, `9d872e88`; the rule's
home is `native_commit/route.rs`), 2026-10-13 (`commit -a` always Git) and 2026-10-14 (stays a module of gwz-core,
`8cefbe40`); a Q1 header records operator sequencing of 2026-09-21 deferring platform and distribution checks "into
one later batch" so that they did not block core-side pool and per-remote work (`f39a6ed2` 2026-09-21,
`2f12bbd6` 2026-09-22 created `transport_binding.rs`, which says it is "isolated until source/platform
qualification permits activation"). Missing evidence: no text found that chooses core over gwz-git for the
per-remote binding, and the 10-12 decision records the rule's location but not why the design's L3 owner (gwz-git,
ownership table since 2026-09-21) was not used.

**3. Which gate let an unused library count as progress.** Not one gate; none required a caller: (i) G0 acceptance
reviewed the library with no production switch in scope (G0 record, "Acceptance covers this unpublished macOS
arm64 local foundation only"); (ii) Q3 and Q5 "consumer" qualification ran instrumented copies and examples
("instrumented artifacts, not unchanged production/release binaries", `GwzGitLibraryNativeConsumers.md`);
(iii) TR3.1 is recorded closed, in the 2026-10-01 checkpoint section, with exit "the rename ... gwz-git `a9d7ee0` and the candidate build in CI"
(`CurrentProgramCheckpoint.md:1665`); (iv) D35 (2026-10-18) "gwz-git stays in 1.1.0 (plan step 4 unchanged)", and
`GwzV110RemainingPlan.md:37` lists gwz-git only as `publish = false` to lift at Phase 8; (v) `AgentProcessRules.md`
has no production-consumer rule (searches for "production consumer", "first consumer", "real consumer" find none);
(vi) no check under `scripts/checks/` names the crate. Motives are not established.

**4. Real blockers versus preferences.** Real, on the evidence: the native fix and per-remote binding exist only in the
fork, which is unpublished (`publish = false`; placeholder registrations only, D34/D37 of 2026-10-18 prepare
publication, and gwz-git's scaffolding commit `6ec091c` is unpushed, `dev-docs/GwzLaneCloseout-2026-10-19.md:121`):
that blocks a **published** gwz-git, not a path sibling, since core already names the same fork by path
(`Cargo.toml:89`). The synchronous blocking-stream contract is unproved (`stream_io.rs:246`; Git plan G3):
a G5 concern, not a G1/G2 one. Platform rows: Windows library and four consumers accepted 2026-09-21
(`GwzGitLibraryWindows.md`, `...WindowsConsumers.md`), macOS `...NativeConsumers.md`; Linux rows are not in those
reports. Not blockers on the evidence: the Q1 note that gwz-py and gwz-cli resolved `libgit2-sys 0.18.5+1.9.4` is
gone (all four locks now resolve `0.18.8+1.9.7`: `gwz-core/Cargo.lock:970`, `gwz-git/Cargo.lock:81`,
`gwz-cli/Cargo.lock:587`, `gwz-py/Cargo.lock:458`); the 10-14 "stay a module" is a recorded preference. `git log -S` finds
no attempt to depend on gwz-git (the only Cargo hit, `26b30ca6`, adds the `gwz-git2` line), so "try the supported
route first" has not been tried: G2's first red test is that feasibility test (open and `head` through gwz-git
from `handle_commit`'s path, one platform, no upgrade).

**5. The adoption gate.** Proposed concrete check for **step 5.5** (a ratchet, in the style of
`check_process_globals.py`, whose lexer already reads every platform branch without compiling it):

1. `scripts/checks/gwz_git_adoption.json`, seeded from §1: per capability `{name, status: planned|live,
   callers: [file:line], retired_paths: [...], retired_symbols: [...]}`. A new `pub` item reachable from
   `gwz-git/src/lib.rs` that is absent from the file fails; a `live` capability with no production caller token
   in `gwz-core/src` (not `cfg(test)`, not `tests/`, not `*_tests.rs`) fails; a `retired_paths` entry that still
   exists, or a `retired_symbols` token still present (e.g. `commit_cli`, `tag_create_cli`, `tag_delete_cli`,
   `Command::new("git")` in `gitbackend/{repository,refs}.rs`), fails.
2. `scripts/checks/gwz_git_residuals.json`: every production file with a `git2` path token, each with
   `{owner, responsibility, exit}`; a new file with such a token fails, a listed file whose tokens vanish fails
   (the list only shrinks). Seed: §4's last list plus G7, G8. The migrated globs (`gitbackend/native_commit/**`,
   `refs.rs`, `repository.rs`, `comparison.rs`, ..., per live row) may hold no `git2` token at all. No unrelated
   file is banned: unlisted new files must add an entry, which is a reviewed act.
3. `process_globals_allowlist_gwz_git.json` for gwz-git's own spawns (F5), wired beside the transport one.
4. Edge guard: add `gwz-git` to `local_clone_inventory.json` as an allowed core edge; forbid gwz-git -> gwz-core,
   gwz-credential and (until F6 is decided) gwz-connection; keep one native provider (`NATIVE_BUILD_DEPENDENCIES`).
5. Tests first, red: `test_check_gwz_git_adoption.py` with fixtures (a capability whose only caller is under
   `#[cfg(test)]`; a caller in `tests/`; a reintroduced retired symbol; a new `git2` file) each failing.
6. Run in `run_boundary_job.py` and `run_tests.py` with the other fast source checks (D52 precedent); pure Python,
   so it runs on all three platforms with no build. Reviewers' checklist (a one-line addition to
   `AgentProcessRules.md`, not made here): "the caller path, not only the library".

## 7. Proposed order and what each step needs

1. **G9 + API amendment** (reviewed): open mode, `resolve`, `NotARepository`. No code beyond tests.
2. **G2a** (`head`, `resolve`, `tag_names`, `is_repository`, `commit_exists`), then **G1** (a, b, c). G1 uses G2a for
   its self-verify (`native_commit.rs:181`, `refs.rs:205-228,272-284`), so 5.2 and 5.3 are parallel only if 5.3 lands
   G2a first or G1 carries private equivalents it later deletes (a second live implementation: avoid).
3. **G2b** (`status`, `blob_at`, `is_ancestor`, `ahead_behind`, `merge_base`, `changed_paths`, `remotes`).
4. **G4a, G4b**, **G3** (decision: in or out of Phase 5).
5. After 4.2 and F6: **G5 + G6 together** (5.4), then 5.5's guard flips each row to `live`.
6. **G7, G8**: outside plan §4; they need L4/G8 design and a decision; until then they are residual (§4) with owner and exit.
7. 5.5's guard can start now with G1 and G2 as `planned`, so the first migrated row is born under it.

Open decisions for the operator: F2 (supersession line), F6 (DAG), G3/G7/G8 in or out of Phase 5, F10, and the G10 dispositions.

## 8. Landing records

### G2 refs and reads (step 5.3; macOS verified, Windows receipt in the commit trailer)

- **Moved and live.** `Repository::{head, tag_names, commit_exists, blob_at, is_ancestor, ahead_behind, merge_base,
  changed_paths, status, remotes}` and the existing `resolve` (A1) in gwz-git; the guide's section A3 is the contract.
  Core's `Git2Backend` methods `head`, `status`, `status_with_options`, `tag_list`, `read_ref`, `commit_exists`,
  `read_file_at_commit`, `is_ancestor`, `ahead_behind`, `merge_base`, `changed_paths_between`, `remotes` delegate through
  `git/gitbackend/reads.rs` (the one file: the 12 backend methods and `head_at`, no `git2` token). Production callers are unchanged (the `GitBackend`
  trait), e.g. `workspace_ops/handle_fetch.rs:429,437`, `status/status_member.rs:125-187`, `diff/classify.rs:206-228`,
  `diff/tagged.rs:63`; the ratchet's `callers` for G2 is `git/gitbackend/reads.rs` itself, which is the only core file that
  names the gwz-git methods.
- **Retired (the seed for 5.5's `retired_paths` and `retired_symbols`).** Paths: `git/index_status_char.rs`,
  `git/worktree_status_char.rs`, `git/tests/g05.rs`, `git/tests/g14.rs`. Symbols (no longer present in core): `fn tag_list(`,
  `fn read_ref(`, `fn ahead_behind(`, `fn is_ancestor(` in `gitbackend/refs.rs`; `fn commit_exists(`, `fn read_file_at_commit(`,
  `fn status(`, `fn status_with_options(`, `fn head(` in `gitbackend/repository.rs`; `fn merge_base(`,
  `fn changed_paths_between(` in `gitbackend/comparison.rs`; `fn remotes(` in `gitbackend/transport.rs`; `git_file_status`,
  `rename_delta`, `index_status_char`, `worktree_status_char`, `unborn_head`. The probe test
  `refs_and_reads_live_in_gwz_git_and_core_keeps_no_copy` (`gitbackend/gwz_git_tests.rs`) asserts them today.
- **Left in core on purpose, and why.** `verify_checkout_state.rs`'s `repo_head(&git2::Repository)` stays as a *delegation*
  (`head_at(repo.path())`, so gwz-git reads HEAD) because rows still in core hold a native handle and call it: G3's
  `branch_list`, `branch_create`, `branch_delete` and `switch_branch` (`refs.rs`), and G10's `recovery_support.rs:358` and
  `merge_support.rs:42`. It dies with the last of them. `staged_statuses()` and `unstaged_statuses()` (two flag sets) stay
  because `verify_checkout_state` (G3) uses them with its own status options; gwz-git holds its own private pair.
  `open_repo` and `find_remote` stay: every G3/G4/G5/G10 caller uses them.
- **Named by this map's §2.2 caller list but not in the row's function list, so not moved.** `repository_paths` (8 production
  sites in 4 files) and `repository_index` (1 site, `checked_artifact/capability/pre_catalog/provider/retained.rs:199`) are
  implemented inline in `gitbackend.rs` over `open_repo`. `repository_paths` needs no new gwz-git API
  (`git_dir`, `common_dir`, `work_dir` exist); `repository_index` needs an owned index-snapshot record (path, entries with
  mode, flags, times, stat words). Both are `git2` reads that a "reads" row would hold: a decision for the operator
  (add them to G2's tail or give them a row), not taken here. **Taken in step 5.5** (they became G2's tail; record below).
- **Tests.** Eight core tests moved (5 from `tests/g01.rs`, the 2 of `g05.rs`, the 1 of `g14.rs`) into
  `gwz-git/tests/refs_reads.rs`, which has 18 (the eight rewritten against the library API, ten new: typed-id and
  object-format refusals, detached HEAD, tag names, merge base, changed paths, conflicts, remotes, bare repositories).
  Composition tests that use a row still in core (`reads_and_adds_remotes`, the `g09` tag tests) stay in core. Core gained 2
  delegation tests (projection onto the model, error translation) and the probe. `repository_contract_tests.rs` (fake
  against native, through the trait) stays: it tests the trait, not a function.

### G2 tail and the boundary guard (step 5.5; macOS verified, Windows receipt in the commit trailer)

- **`repository_paths` and `repository_index` moved** (the lane owner's decision on the 5.3 finding; the two reads §8 above
  left inline in `gitbackend.rs`). `repository_index` is `Repository::index_snapshot` with the owned records `IndexSnapshot` and
  `IndexEntry` (guide A3); `repository_paths` needed no gwz-git API: core's `reads.rs` builds its record from `git_dir`,
  `common_dir` and `work_dir`. Both delegate through `reads.rs`, which still holds no `git2` token. The `GitBackend` trait and
  its callers (`checked_artifact/`, `preservation_root/files.rs`) are unchanged. With them, `reads.rs` is the one file that
  names the gwz-git reads.
- **The guard** extends `scripts/checks/check_filesystem_boundary.py` (the existing check whose job is "this migrated area does not
  import the native thing directly"; the process-globals and cfg checks police state and conditional compilation, not imports) with
  the migrated area of `scripts/checks/gwz_git_boundary.json`: `zero` (no production `git2` token) holds `commit_tag.rs`, `reads.rs`,
  `backend.rs`, `comparison.rs`, `spawn_gate.rs` and the replaced `native_commit` tree, `index_status_char.rs` and
  `worktree_status_char.rs`; `exceptions` holds the six files the moves rewrote that still serve a row in core, with an exact token
  count, rows, responsibility, exit and the date 2026-10-25: `verify_checkout_state.rs` (18: `repo_head` for G3/G10 callers, `git_error`,
  G3's self-verify), `repository_support.rs` (17), `repository.rs` (11, G3), `refs.rs` (8, G3), `transport.rs` (26, G4/G5/G6),
  `gitbackend.rs` (2, G6). A count may fall (the entry must then be lowered, deleted at zero) and never rise. G7 and G8 uses sit in
  `operation/commit_log/` and `diff/`, outside the area, so they are not entries; they enter the area, and the list, when a step
  migrates them. The proposal of §6 item 2 (a register of every `git2` file) was narrowed to the migrated area as the lane owner
  directed: no unrelated file is banned.
- **The adoption check** is `scripts/checks/check_gwz_git_adoption.py` (new; there was no adoption-style check to extend): it reads
  gwz-git's `src/lib.rs` re-exports and the `pub fn` of the exported types and public modules, requires each to be in the guide and to
  have a production caller in gwz-core (a non-test file that names `gwz_git` and the owner, outside `use` lines), and keeps
  `gwz_git_adoption.json`, a dated list of the public capabilities that have none today: `Repository::read_commit`,
  `Repository::open_from_env`, `Error::native`, `ObjectId::format` and `RouteNotes::new` (consumed by gwz-cli only). §6 item 1's
  `retired_paths` and `retired_symbols` stay with the probe tests in `gwz_git_tests.rs`; the zero list also covers the replaced paths.
- **Wiring.** `run_tests.py` runs the adoption check after the gwz-git process-global check over the same pinned checkout; the boundary
  workflow runs it and the new unit tests in its check step and unittest list; `check_filesystem_boundary.py` was already in both.

### Phase 5 review round 1 remediation (one patch; Code and State reports of 2026-10-25)

- **S-P2-1.** `native_commit/verify.rs` tests tag existence by exact reference lookup (`Repository::tag_commit`, `tag_ref_exists`, both
  crate-private, over `find_reference("refs/tags/<name>")`); `resolve("HEAD")` stays for the commit check. A `t-1-g<abbrev>` tag and a branch
  `refs/tags/X` no longer stand in for a tag. S-P3-2: a verification read that fails after the write is an `Operation` error saying so.
- **C-P2-1, C-P2-2.** Both `transport-candidate.yml` jobs fetch gwz-git at its pin (`checkout-siblings.sh`) and check it is on main;
  `scripts/checks/test_workflow_siblings.py` holds every workflow job that builds gwz-core to every `path = "../X"` sibling. gwz-git's `ci.yml`
  gained Linux (arm64) and macOS legs.
- **C-P3-1.** The guard also counts core's native-handle producers (`open_repo`, `repo_head`, `find_remote`, `git_error`); `comparison.rs`
  is an exception (G8) with its counts; every exception records its producer count.
- **C-P3-2 = S-P3-1, C-P3-4.** The adoption check credits a call only when the file takes the owner from gwz-git, treats a test-gated `mod`'s
  file as test source, and lists public associated constants. `RoutePolicy::CLI_ONLY` and `Services::with_policy` are behind the gwz-git
  feature `test-support` (gwz-core `[dev-dependencies]` only); both are on the dated ratchet as unconsumed, with that reason.
- **C-P3-3.** `route::open` delegates to `Repository::open_from_env`; `open_from_env` keeps its ratchet entry (no gwz-core caller), reason corrected.
