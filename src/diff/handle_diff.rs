//! `handle_diff` — the D3 integration keystone.
//!
//! Composes the landed phases into the `diff` planning response and, when patch
//! output is requested, mints an output log and runs the producer:
//!
//! 1. **Plan (D2).** [`parse_comparison`] lowers the operands + `cached` /
//!    `merge_base` flags; [`plan_diff`] resolves the per-repo target set and the
//!    snapshot-narrowed [`ExcludedTarget`]s.
//! 2. **Manifest (D1).** For each planned target, [`resolve_comparison`] peels
//!    the comparison to tree sides and [`diff_repo`] builds the repo-scoped
//!    manifest. Root deltas under member paths / `.gwz/` / `gwz.conf/.tmp/` are
//!    dropped (AD11) before projection.
//! 3. **Project.** Repo entries become wire [`DiffFileEntry`]s with a
//!    [`DiffRepoScope`] and workspace-relative paths, plus per-repo and aggregate
//!    [`DiffSummary`].
//! 4. **Short-circuit.** `--quiet` / metadata-only formats (`no_patch`,
//!    `name_*`, `*stat`, `summary`) return the manifest with **no** output-log
//!    ref — the client answers exit-code / name / stat from the manifest.
//! 5. **Produce.** For byte formats (`patch`, `raw`, …), mint a `log_id`
//!    ([`DiffOutputLogRef`]) and run the [`output`](super::output) producer over
//!    the planned targets, emitting `file_started` / `patch_bytes` /
//!    `file_finished` (and `stale_file` on a worktree race) per file.
//!
//! `operation.result` stays metadata-only (AD5): patch bytes live only in the
//! output log, never in [`DiffManifestResponse`].

use std::path::{Path, PathBuf};

use crate::artifact::{self, ArtifactSourceKind, ManifestMember};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::protocol::generated::{
    AggregateStatus, DiffChunkEncoding, DiffFileEntry, DiffManifestMode, DiffManifestResponse,
    DiffOutputFormat, DiffOutputLogRef, DiffRepoScope, DiffRepoSummary, DiffRequest, DiffStatus,
    DiffSummary, ResponseEnvelope, ResponseMeta,
};
use crate::workspace_ops::{assert_workspace_id, resolve_workspace_root};

use super::log_service::DiffLogRegistry;
use super::output::{ProducerEntry, ProducerOptions, ProducerTarget, run_producer};
use super::plan::{DiffPlan, PlanScope, PlannedTarget};
use super::render::{PrefixPolicy, RenderOptions, ScopeRender};
use super::{
    RepoDiffAlgorithm, RepoDiffComparison, RepoDiffEntry, RepoDiffManifest, RepoDiffOptions,
    RepoDiffStatus, RepoDiffWhitespace, diff_repo, parse_comparison, plan_diff,
    reject_unsupported_options, resolve_comparison,
};

/// The outcome of a `diff` planning call: the wire response plus, when patch
/// output was produced, the minted `log_id` (so the caller / test can read the
/// `diff.output` log). The producer has already run to completion (or a stop /
/// failure) by the time this returns in the in-process v0 model.
pub struct DiffOutcome {
    pub response: DiffManifestResponse,
    /// `Some` iff a byte-output log was minted and produced into.
    pub log_id: Option<String>,
}

/// Handle a `diff` request against the workspace rooted at `start`.
///
/// `registry` owns the operation-scoped output logs; for byte formats a fresh
/// `log_id` is minted into it and the producer runs synchronously in-process
/// (v0). Metadata-only modes never touch the registry.
pub fn handle_diff(
    start: &Path,
    request: DiffRequest,
    operation_id: impl Into<String>,
    registry: &DiffLogRegistry,
) -> ModelResult<DiffOutcome> {
    let _ = operation_id;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;

    let options = request.options.clone().unwrap_or_default();
    reject_unsupported_options(&options)?;

    // Bare-operand disambiguation (D5): with no `--`, each positional operand is a
    // revision or a pathspec, decided per git's rule (see `super::classify`). The
    // revision half feeds `parse_comparison`; the pathspec half is prepended to
    // the explicit `--` pathspecs below.
    // The workspace-relative logical cwd (AD10). The client sends one in
    // `workspace_cwd`, but it is computed against the raw invocation cwd (which,
    // without `--root`, is *not* the discovered workspace root) — so a bare path
    // operand entered from a member subdir would stat against the wrong base.
    // Recompute it here against the *resolved* `root` and the physical `start`
    // dir, which is authoritative; fall back to the client value only when `start`
    // cannot be expressed under `root`.
    let cwd_rel = resolved_cwd_rel(start, &root)
        .unwrap_or_else(|| request.workspace_cwd.clone().unwrap_or_default());
    let classified = {
        let ctx = super::RevContext {
            repos: super::candidate_repos(&root, &manifest),
            cwd: root.join(&cwd_rel),
            workspace_root: root.clone(),
            resolve: &super::default_rev_resolver,
        };
        super::classify_operands(&request.operands, &manifest, &ctx)?
    };

    let comparison = parse_comparison(
        &classified.revisions,
        request.cached.unwrap_or(false),
        request.merge_base.unwrap_or(false),
    )?;

    // Snapshots referenced by the comparison, read once for planning.
    let snapshots = read_referenced_snapshots(&root, &comparison_snapshot_ids(&comparison))?;

    // Pathspecs derived from bare operands precede the explicit (`--`) ones, so a
    // routing that intersects them keeps operand-order intent (git resolves both
    // against the cwd identically).
    let mut pathspecs = classified.pathspecs.clone();
    pathspecs.extend(request.explicit_pathspecs.iter().cloned());
    let oracle = FsMaterializationOracle { root: root.clone() };
    let plan = plan_diff(
        &manifest,
        request.meta.selection.as_ref(),
        &comparison,
        &cwd_rel,
        &pathspecs,
        &snapshots,
        &oracle,
    )?;

    let backend_options = build_backend_options(&options);
    let format = options.output_format.unwrap_or(DiffOutputFormat::Patch);
    let manifest_mode = options.manifest_mode.unwrap_or(DiffManifestMode::Full);
    let want_bytes = format_wants_bytes(format);

    // Resolve + diff every planned target, in root-first plan order.
    let mut repo_results: Vec<RepoResult> = Vec::new();
    for target in &plan.targets {
        repo_results.push(diff_one_target(
            &root,
            target,
            &backend_options,
            &options,
            manifest_mode,
        )?);
    }
    let parsed_targets = resolved_parsed_targets(&plan, &repo_results);

    // --quiet / any_difference: short-circuit once any repo differs. No file
    // list, summary-only, no output log.
    if matches!(manifest_mode, DiffManifestMode::AnyDifference) {
        let has_diff = repo_results.iter().any(|r| r.manifest.has_differences());
        return Ok(DiffOutcome {
            response: DiffManifestResponse {
                response: envelope(&request),
                files: Vec::new(),
                summary: Some(any_difference_summary(&repo_results, has_diff)),
                targets: parsed_targets,
                output: None,
                excluded_targets: plan.excluded_targets(),
            },
            log_id: None,
        });
    }

    // Full manifest projection: wire file entries + per-repo/aggregate summaries.
    let mut files: Vec<DiffFileEntry> = Vec::new();
    let mut repo_summaries: Vec<DiffRepoSummary> = Vec::new();
    for result in &repo_results {
        let scope = result.scope.to_wire();
        for (ri, entry) in result.manifest.entries.iter().enumerate() {
            let file_id = make_file_id(&result.scope, ri);
            files.push(project_entry(entry, &scope, &result.render.scope, file_id));
        }
        repo_summaries.push(repo_summary(&scope, &result.manifest));
    }
    let summary = aggregate_summary(repo_summaries);

    // Metadata-only formats: manifest answers them client-side, no output log.
    if !want_bytes {
        return Ok(DiffOutcome {
            response: DiffManifestResponse {
                response: envelope(&request),
                files,
                summary: Some(summary),
                targets: parsed_targets,
                output: None,
                excluded_targets: plan.excluded_targets(),
            },
            log_id: None,
        });
    }

    // Byte formats: mint a log, run the producer, attach the ref.
    let (log_id, log) = registry.create();
    let producer_targets = build_producer_targets(&root, &plan, &repo_results, &options, &files)?;
    let producer_opts = ProducerOptions {
        echo_manifest_entries: options.echo_manifest_entries.unwrap_or(false),
        format,
    };
    run_producer(&log, producer_targets, producer_opts)?;

    let output_ref = DiffOutputLogRef {
        log_id: log_id.clone(),
        format,
        encoding: Some(byte_encoding(format)),
    };

    Ok(DiffOutcome {
        response: DiffManifestResponse {
            response: envelope(&request),
            files,
            summary: Some(summary),
            targets: parsed_targets,
            output: Some(output_ref),
            excluded_targets: plan.excluded_targets(),
        },
        log_id: Some(log_id),
    })
}

/// The physical invocation dir (`start`) expressed relative to the resolved
/// workspace `root`, as a forward-slash, workspace-relative path (`""` at the
/// root, `gwz-py`, `gwz-py/native/src`, …). This is the authoritative logical cwd
/// for operand/pathspec routing (AD10): unlike the client-supplied
/// `workspace_cwd`, it is computed against the *discovered* root, so it is correct
/// even when the CLI was invoked from a member subdir without `--root`.
///
/// Both paths are canonicalized first so symlinks and `..` compare correctly.
/// Returns `None` when `start` is not under `root` (the caller then falls back to
/// the client value), so an out-of-tree invocation degrades rather than misroutes.
fn resolved_cwd_rel(start: &Path, root: &Path) -> Option<String> {
    let start_abs = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let root_abs = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let rel = start_abs.strip_prefix(&root_abs).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// One planned target after resolution + diff: the repo path, its resolved
/// comparison, the repo-scoped manifest (root-filtered), the scope, and the
/// render scope for byte output.
struct RepoResult {
    repo_path: PathBuf,
    scope: PlanScope,
    comparison: RepoDiffComparison,
    options: RepoDiffOptions,
    manifest: RepoDiffManifest,
    render: RenderOptions,
}

/// Resolve one target's comparison, run its manifest diff, apply the root delta
/// post-filter (AD11), and build its render scope.
fn diff_one_target(
    root: &Path,
    target: &PlannedTarget,
    backend_options: &RepoDiffOptions,
    wire_options: &crate::protocol::generated::DiffOptions,
    _manifest_mode: DiffManifestMode,
) -> ModelResult<RepoResult> {
    let repo_path = repo_path_for(root, &target.scope);
    let repo = crate::git::open_repo(&repo_path)?;

    let mut options = backend_options.clone();
    options.pathspecs = target.pathspecs.clone();

    let comparison = resolve_comparison(&repo, &target.spec)?;
    let mut manifest = diff_repo(&repo, &comparison, &options)?;

    // Root delta post-filter (AD11): drop entries under member paths / .gwz /
    // gwz.conf/.tmp before projection, stats, and patch selection.
    if matches!(target.scope, PlanScope::Root) && !target.root_exclude.is_empty() {
        filter_root_deltas(&mut manifest, &target.root_exclude);
    }

    let render = build_render_options(&target.scope, wire_options);
    Ok(RepoResult {
        repo_path,
        scope: target.scope.clone(),
        comparison,
        options,
        manifest,
        render,
    })
}

/// Drop root manifest entries whose primary path is under an excluded prefix and
/// recompute the repo's insertion/deletion rollup from the survivors.
fn filter_root_deltas(manifest: &mut RepoDiffManifest, exclude: &[String]) {
    manifest.entries.retain(|entry| {
        let path = entry.primary_path().unwrap_or("");
        !exclude.iter().any(|prefix| is_under(path, prefix))
    });
    manifest.insertions = manifest.entries.iter().filter_map(|e| e.insertions).sum();
    manifest.deletions = manifest.entries.iter().filter_map(|e| e.deletions).sum();
}

/// True when `path` equals `prefix` or is a descendant (`prefix/…`).
fn is_under(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Build the producer targets: reopen each repo, pair every planned entry with
/// its wire [`DiffFileEntry`] (matched by scope + manifest order), and carry the
/// resolved comparison / options / render scope.
fn build_producer_targets(
    root: &Path,
    plan: &DiffPlan,
    repo_results: &[RepoResult],
    _wire_options: &crate::protocol::generated::DiffOptions,
    files: &[DiffFileEntry],
) -> ModelResult<Vec<ProducerTarget>> {
    let mut targets: Vec<ProducerTarget> = Vec::new();
    // `files` is in the same root-first, manifest order as repo_results, so a
    // running cursor over `files` pairs each entry with its wire projection.
    let mut cursor = 0usize;
    for result in repo_results {
        let repo = crate::git::open_repo(&result.repo_path)?;
        let scope_wire = result.scope.to_wire();
        let mut entries: Vec<ProducerEntry> = Vec::new();
        for entry in &result.manifest.entries {
            let wire = files
                .get(cursor)
                .cloned()
                .expect("file projection aligns with manifest order");
            cursor += 1;
            entries.push(ProducerEntry {
                entry: entry.clone(),
                wire,
            });
        }
        targets.push(ProducerTarget {
            repo,
            comparison: result.comparison.clone(),
            options: result.options.clone(),
            render: result.render.clone(),
            scope: scope_wire,
            entries,
        });
    }
    let _ = (root, plan);
    Ok(targets)
}

fn resolved_parsed_targets(
    plan: &DiffPlan,
    repo_results: &[RepoResult],
) -> Vec<crate::protocol::generated::DiffParsedTarget> {
    let mut targets = plan.parsed_targets();
    for (target, result) in targets.iter_mut().zip(repo_results) {
        target.left_oid = result.comparison.left_oid.clone();
        target.right_oid = result.comparison.right_oid.clone();
        target.merge_base_oid = result.comparison.merge_base_oid.clone();
    }
    targets
}

// ── projection helpers ──────────────────────────────────────────────────────

fn project_entry(
    entry: &RepoDiffEntry,
    scope: &DiffRepoScope,
    render_scope: &ScopeRender,
    file_id: String,
) -> DiffFileEntry {
    DiffFileEntry {
        file_id,
        scope: scope.clone(),
        status: status_to_wire(entry.status),
        old_path: entry
            .old_path
            .as_deref()
            .map(|p| render_scope.workspace_path(p)),
        new_path: entry
            .new_path
            .as_deref()
            .map(|p| render_scope.workspace_path(p)),
        old_mode: entry.old_mode.map(|m| m as i64),
        new_mode: entry.new_mode.map(|m| m as i64),
        similarity: entry.similarity.map(|s| s as i64),
        insertions: entry.insertions.map(|v| v as i64),
        deletions: entry.deletions.map(|v| v as i64),
        is_binary: entry.is_binary.then_some(true),
    }
}

fn status_to_wire(status: RepoDiffStatus) -> DiffStatus {
    status.to_wire()
}

fn repo_summary(scope: &DiffRepoScope, manifest: &RepoDiffManifest) -> DiffRepoSummary {
    DiffRepoSummary {
        scope: scope.clone(),
        has_differences: manifest.has_differences(),
        files_changed: manifest.files_changed() as i64,
        insertions: manifest.insertions as i64,
        deletions: manifest.deletions as i64,
        files_manifested: manifest.files_changed() as i64,
    }
}

fn aggregate_summary(repo_summaries: Vec<DiffRepoSummary>) -> DiffSummary {
    let repos_examined = repo_summaries.len() as i64;
    let repos_with_differences = repo_summaries.iter().filter(|s| s.has_differences).count() as i64;
    let files_changed = repo_summaries.iter().map(|s| s.files_changed).sum();
    let insertions = repo_summaries.iter().map(|s| s.insertions).sum();
    let deletions = repo_summaries.iter().map(|s| s.deletions).sum();
    DiffSummary {
        has_differences: repos_with_differences > 0,
        repos_examined,
        repos_with_differences,
        files_changed,
        insertions,
        deletions,
        repo_summaries,
    }
}

/// Summary for `any_difference` mode: repo rollups without file lists (early
/// exit means counts may be partial; only `has_differences` is contractually
/// meaningful for the exit-code decision, AD8).
fn any_difference_summary(results: &[RepoResult], has_diff: bool) -> DiffSummary {
    let repo_summaries: Vec<DiffRepoSummary> = results
        .iter()
        .map(|r| repo_summary(&r.scope.to_wire(), &r.manifest))
        .collect();
    let repos_examined = repo_summaries.len() as i64;
    let repos_with_differences = repo_summaries.iter().filter(|s| s.has_differences).count() as i64;
    DiffSummary {
        has_differences: has_diff,
        repos_examined,
        repos_with_differences,
        files_changed: repo_summaries.iter().map(|s| s.files_changed).sum(),
        insertions: repo_summaries.iter().map(|s| s.insertions).sum(),
        deletions: repo_summaries.iter().map(|s| s.deletions).sum(),
        repo_summaries,
    }
}

/// A stable, opaque `file_id`: `<target_id>#<index>`. Opaque to clients (never
/// parsed as a path — D0 invariant 6); identity is scope/status/old/new_path.
fn make_file_id(scope: &PlanScope, index: usize) -> String {
    let target = match scope {
        PlanScope::Root => "@root".to_owned(),
        PlanScope::Member { member_id, .. } => member_id.clone(),
    };
    format!("{target}#{index}")
}

// ── option mapping ──────────────────────────────────────────────────────────

/// Map the wire options onto the backend's [`RepoDiffOptions`] (the subset that
/// changes which deltas/bytes libgit2 emits). Presentation-only fields (prefixes,
/// line-prefix, `-z`, binary display) are applied by the render scope, not here.
fn build_backend_options(options: &crate::protocol::generated::DiffOptions) -> RepoDiffOptions {
    RepoDiffOptions {
        pathspecs: Vec::new(),
        context_lines: options.context_lines.map(|v| v.max(0) as u32),
        interhunk_lines: options.interhunk_lines.map(|v| v.max(0) as u32),
        algorithm: options
            .algorithm
            .map(RepoDiffAlgorithm::from_wire)
            .unwrap_or_default(),
        whitespace: options
            .whitespace
            .map(RepoDiffWhitespace::from_wire)
            .unwrap_or_default(),
        find_renames: options.find_renames.unwrap_or(false),
        rename_threshold: options.rename_threshold.map(|v| v.clamp(0, 100) as u16),
        rename_limit: options.rename_limit.map(|v| v.max(0) as usize),
        force_text: options.text.unwrap_or(false),
        include_typechange: true,
        reverse: options.reverse.unwrap_or(false),
    }
}

/// Build the per-repo render scope from the wire prefix options + this repo's
/// scope (empty member prefix = root). `--line-prefix`, `-z`, and `--binary`
/// travel on the [`RenderOptions`].
fn build_render_options(
    scope: &PlanScope,
    options: &crate::protocol::generated::DiffOptions,
) -> RenderOptions {
    let policy = prefix_policy(options);
    let render_scope = match scope {
        PlanScope::Root => ScopeRender::root(policy),
        PlanScope::Member { member_path, .. } => ScopeRender::member(member_path.clone(), policy),
    };
    RenderOptions {
        scope: render_scope,
        line_prefix: options.line_prefix.clone(),
        null_terminated: options.null_terminated.unwrap_or(false),
        show_binary: options.binary.unwrap_or(false),
    }
}

fn prefix_policy(options: &crate::protocol::generated::DiffOptions) -> PrefixPolicy {
    if options.no_prefix.unwrap_or(false) {
        return PrefixPolicy::None;
    }
    match (&options.src_prefix, &options.dst_prefix) {
        (None, None) => PrefixPolicy::Default,
        (src, dst) => PrefixPolicy::Custom {
            src: src.clone().unwrap_or_else(|| "a/".to_owned()),
            dst: dst.clone().unwrap_or_else(|| "b/".to_owned()),
        },
    }
}

/// Whether a format asks for byte output (needs the output log) or is answered
/// from the manifest alone.
fn format_wants_bytes(format: DiffOutputFormat) -> bool {
    matches!(
        format,
        DiffOutputFormat::Patch
            | DiffOutputFormat::Raw
            | DiffOutputFormat::PatchWithRaw
            | DiffOutputFormat::PatchWithStat
    )
}

/// The byte encoding advertised on the output log: patch/binary are `bytes`.
fn byte_encoding(_format: DiffOutputFormat) -> DiffChunkEncoding {
    DiffChunkEncoding::Bytes
}

// ── plumbing ────────────────────────────────────────────────────────────────

/// The repo directory for a planned scope: the workspace root, or `root/<member
/// path>` for a member.
fn repo_path_for(root: &Path, scope: &PlanScope) -> PathBuf {
    match scope {
        PlanScope::Root => root.to_path_buf(),
        PlanScope::Member { member_path, .. } => root.join(member_path),
    }
}

/// The snapshot ids referenced by the comparison (0..2), for reading artifacts.
fn comparison_snapshot_ids(comparison: &super::ParsedComparison) -> Vec<String> {
    comparison
        .snapshot_ids()
        .into_iter()
        .map(|s| s.to_owned())
        .collect()
}

/// Read each referenced snapshot artifact. A missing snapshot is a typed
/// `SnapshotNotFound` (matching the plan's error taxonomy).
fn read_referenced_snapshots(
    root: &Path,
    ids: &[String],
) -> ModelResult<Vec<artifact::SnapshotArtifact>> {
    let mut out = Vec::new();
    for id in ids {
        out.push(artifact::read_snapshot(root, id).map_err(|err| {
            if err.code == ErrorCode::SnapshotNotFound {
                err
            } else {
                ModelError::new(
                    ErrorCode::SnapshotNotFound,
                    format!("snapshot '{id}' could not be read: {}", err.message),
                )
            }
        })?);
    }
    Ok(out)
}

/// Filesystem-backed materialization check: a member is materialized when its
/// worktree directory holds a Git repository.
struct FsMaterializationOracle {
    root: PathBuf,
}

impl super::MaterializationOracle for FsMaterializationOracle {
    fn is_materialized(&self, member: &ManifestMember) -> bool {
        if member.source_kind != ArtifactSourceKind::Git {
            return false;
        }
        let member_root = self.root.join(&member.path);
        crate::git::Git2Backend::new()
            .is_repository(&member_root)
            .unwrap_or(false)
    }
}

/// Build the response envelope. Diff is read-only, so aggregate status is `Ok`.
fn envelope(request: &DiffRequest) -> ResponseEnvelope {
    ResponseEnvelope {
        meta: ResponseMeta {
            request_id: request.meta.request_id.clone(),
            schema_version: request.meta.schema_version.clone(),
            action: crate::protocol::generated::ActionKind::Diff,
            aggregate_status: AggregateStatus::Ok,
            operation_id: None,
            message: None,
            attribution: None,
        },
        members: Vec::new(),
        errors: Vec::new(),
    }
}
