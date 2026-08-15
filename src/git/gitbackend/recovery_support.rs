use std::collections::BTreeMap;

use super::merge_support::{prepared_merge_mismatch, signature_from_prepared};
use super::repository_support::{
    branch_ref_name, parse_existing_commit, status_dirty_outside_checked_artifact_private,
    verify_merge_result,
};
use super::*;

pub(super) fn recovery_drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message)
}

pub(super) fn recovery_dirty(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::DirtyMember, message)
}

/// M5-8 A1 Decision Packet, Decision 2 (A′) — refined foreign-filter refusal.
///
/// Before a recovery-grade (filters-disabled) checkout moves the worktree
/// from `from` to `to`, refuse when any path it would WRITE is covered by a
/// configured, non-passthrough foreign `filter` driver. Writing raw blob
/// bytes through such a driver breaks Clause A's clean-idempotence
/// precondition (`clean(blob_bytes) != blob_bytes`, the git-crypt class): the
/// exact post-verification then fails only AFTER `transaction.commit()`, and
/// the retry re-fails in the idempotent arm — a retry-proof recovery wedge
/// (`GwzM5-8ExactEvidencePlatformAmendment.md`, Clause A OPEN DECISION,
/// closed by the A1 packet as A′). This preflight replaces that post-commit
/// wedge with a typed refusal before any ref or worktree mutation.
///
/// Predicate, per written path — deltas whose new (`to`) side exists;
/// deletions write no bytes through any filter:
/// - the `filter` attribute names a driver (bare/boolean or unset attributes
///   cannot run one),
/// - the name is foreign — not the allowlisted `lfs`, whose pointer blobs
///   round-trip clean (the pointer-bytes-on-disk surprise is disclosed
///   doctrine, not a refusal),
/// - and the driver is configured non-passthrough: `filter.<name>.clean` or
///   `filter.<name>.process` present in the effective config. An attribute
///   whose driver has neither is passthrough and cannot wedge, so it does
///   not cost rollback availability.
///
/// Cost: one tree diff (the checkout walks the same delta anyway) plus
/// O(rewrite set) attribute lookups (libgit2 caches attribute stacks) and one
/// config probe per distinct driver name (memoized here).
pub(super) fn refuse_foreign_filtered_rewrites(
    repo: &git2::Repository,
    from: &git2::Tree<'_>,
    to: &git2::Tree<'_>,
) -> ModelResult<()> {
    let diff = repo
        .diff_tree_to_tree(Some(from), Some(to), None)
        .map_err(git_error)?;
    let mut probed: BTreeMap<String, bool> = BTreeMap::new();
    let mut config: Option<git2::Config> = None;
    for delta in diff.deltas() {
        if delta.status() == git2::Delta::Deleted {
            continue;
        }
        let Some(path) = delta.new_file().path() else {
            continue;
        };
        let attr = repo
            .get_attr_bytes(path, "filter", git2::AttrCheckFlags::default())
            .map_err(git_error)?;
        let name = match git2::AttrValue::from_bytes(attr) {
            git2::AttrValue::String(name) => name,
            // A non-UTF-8 driver name cannot be probed in config; fail closed
            // rather than guess at what it might run.
            git2::AttrValue::Bytes(_) => {
                return Err(foreign_filter_refusal(path, "<non-utf8 filter name>"));
            }
            _ => continue,
        };
        if name == "lfs" {
            continue;
        }
        let configured = match probed.get(name) {
            Some(known) => *known,
            None => {
                if config.is_none() {
                    config = Some(open_repo_config_snapshot(repo)?);
                }
                let snapshot = config.as_ref().expect("config snapshot just opened");
                let known = foreign_filter_is_configured(snapshot, name)?;
                probed.insert(name.to_owned(), known);
                known
            }
        };
        if configured {
            return Err(foreign_filter_refusal(path, name));
        }
    }
    Ok(())
}

fn open_repo_config_snapshot(repo: &git2::Repository) -> ModelResult<git2::Config> {
    repo.config()
        .and_then(|mut config| config.snapshot())
        .map_err(git_error)
}

fn foreign_filter_is_configured(config: &git2::Config, name: &str) -> ModelResult<bool> {
    for key in [
        format!("filter.{name}.clean"),
        format!("filter.{name}.process"),
    ] {
        match config.get_entry(&key) {
            Ok(_) => return Ok(true),
            Err(error) if error.code() == git2::ErrorCode::NotFound => {}
            Err(error) => return Err(git_error(error)),
        }
    }
    Ok(false)
}

fn foreign_filter_refusal(path: &Path, filter: &str) -> ModelError {
    recovery_dirty(format!(
        "recovery checkout would rewrite '{}' through configured foreign filter '{filter}' \
         (filter.{filter}.clean/process); refusing before any ref or worktree mutation",
        path.display()
    ))
}

pub(super) fn attached_head_ref(repo: &git2::Repository) -> ModelResult<String> {
    let head = repo.head().map_err(git_error)?;
    if !head.is_branch() {
        return Err(recovery_drift(
            "merge recovery requires an attached local branch",
        ));
    }
    let name = head.name().map_err(git_error)?;
    Ok(name.to_owned())
}

pub(super) fn validate_expected_native_merge(
    repo: &git2::Repository,
    before: git2::Oid,
    expected_merge_head: git2::Oid,
) -> ModelResult<()> {
    if repo.state() != git2::RepositoryState::Merge {
        return Err(recovery_drift(format!(
            "expected native merge state, observed {:?}",
            repo.state()
        )));
    }
    let head = repo.head().map_err(git_error)?;
    let observed = head.peel_to_commit().map_err(git_error)?.id();
    if !head.is_branch() || observed != before {
        return Err(recovery_drift(format!(
            "merge target changed; expected {before}, observed {observed}"
        )));
    }
    let value = std::fs::read_to_string(repo.path().join("MERGE_HEAD"))
        .map_err(|error| recovery_drift(format!("failed to read expected MERGE_HEAD: {error}")))?;
    let mut heads = value.lines().filter(|line| !line.trim().is_empty());
    let observed_merge_head = heads
        .next()
        .and_then(|line| git2::Oid::from_str(line.trim()).ok());
    if heads.next().is_some() || observed_merge_head != Some(expected_merge_head) {
        return Err(recovery_drift(format!(
            "MERGE_HEAD changed; expected {expected_merge_head}"
        )));
    }
    Ok(())
}

pub(super) fn expected_conflicts_and_index(
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<(BTreeSet<Vec<u8>>, git2::Index)> {
    let before = repo.find_commit(before).map_err(git_error)?;
    let merge_head = repo.find_commit(merge_head).map_err(git_error)?;
    let index = repo
        .merge_commits(&before, &merge_head, None)
        .map_err(git_error)?;
    let mut conflicts = BTreeSet::new();
    for conflict in index.conflicts().map_err(git_error)? {
        let conflict = conflict.map_err(git_error)?;
        if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor) {
            conflicts.insert(entry.path);
        }
    }
    Ok((conflicts, index))
}

pub(super) fn comparable_index_entries(
    index: &git2::Index,
    excluded: &BTreeSet<Vec<u8>>,
) -> Vec<(Vec<u8>, u32, git2::Oid, u16)> {
    index
        .iter()
        .filter(|entry| !excluded.contains(&entry.path))
        .map(|entry| (entry.path, entry.mode, entry.id, (entry.flags >> 12) & 3))
        .collect()
}

pub(super) fn validate_recovery_index(
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<BTreeSet<Vec<u8>>> {
    let (conflicts, expected) = expected_conflicts_and_index(repo, before, merge_head)?;
    let current = repo.index().map_err(git_error)?;
    if comparable_index_entries(&current, &conflicts)
        != comparable_index_entries(&expected, &conflicts)
    {
        return Err(recovery_dirty(
            "merge index contains changes outside the expected conflict paths",
        ));
    }
    Ok(conflicts)
}

pub(super) fn validate_abort_index_and_worktree(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<()> {
    let conflicts = validate_recovery_index(repo, before, merge_head)?;
    let status = backend.status(path)?;
    let unexpected_worktree_change = status
        .files
        .iter()
        .any(|file| file.worktree_status != " " && !conflicts.contains(file.path.as_bytes()));
    if status.untracked > 0 || unexpected_worktree_change {
        return Err(recovery_dirty(
            "merge abort would overwrite work outside the expected conflict paths",
        ));
    }
    Ok(())
}

pub(super) fn validate_resolution_index_and_worktree(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<()> {
    validate_recovery_index(repo, before, merge_head)?;
    let status = backend.status(path)?;
    if status.unresolved > 0 || status.unstaged > 0 || status.untracked > 0 {
        return Err(recovery_dirty(
            "merge resolution must be fully resolved and staged with no unrelated worktree changes",
        ));
    }
    Ok(())
}

pub(super) struct ValidatedPreparedMergeResolution<'repo> {
    pub(super) before: git2::Oid,
    pub(super) merge_head: git2::Oid,
    pub(super) tree: git2::Tree<'repo>,
    pub(super) author: git2::Signature<'static>,
    pub(super) committer: git2::Signature<'static>,
}

pub(super) fn validate_expected_resolution_repository_state(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
) -> ModelResult<(git2::Oid, git2::Oid)> {
    let before = parse_existing_commit(repo, expected_before)?;
    let merge_head = parse_existing_commit(repo, expected_merge_head)?;
    let expected_ref = branch_ref_name(target_branch);
    let observed_ref = attached_head_ref(repo)?;
    if observed_ref != expected_ref {
        return Err(recovery_drift(format!(
            "merge target changed; expected attached branch '{target_branch}'"
        )));
    }
    let target = repo
        .find_reference(&expected_ref)
        .and_then(|reference| reference.peel_to_commit())
        .map_err(git_error)?
        .id();
    if target != before {
        return Err(recovery_drift(format!(
            "merge target branch '{target_branch}' changed; expected {before}, observed {target}"
        )));
    }
    validate_expected_native_merge(repo, before, merge_head)?;
    validate_resolution_index_and_worktree(backend, path, repo, before, merge_head)?;
    Ok((before, merge_head))
}

/// Validate a durable resolution specification without creating an object or
/// changing repository state. Both observation and checked execution use this
/// exact definition, including the durable target branch.
pub(super) fn validate_prepared_merge_resolution_in_repo<'repo>(
    backend: &impl GitBackend,
    path: &Path,
    repo: &'repo git2::Repository,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
    prepared: &GitPreparedCommit,
) -> ModelResult<ValidatedPreparedMergeResolution<'repo>> {
    let (before, merge_head) = validate_expected_resolution_repository_state(
        backend,
        path,
        repo,
        target_branch,
        expected_before,
        expected_merge_head,
    )?;

    // The live repository identity is deliberately unused: the durable
    // specification owns both exact signatures.
    let author = signature_from_prepared(&prepared.author)?;
    let committer = signature_from_prepared(&prepared.committer)?;
    let tree_oid = git2::Oid::from_str(&prepared.tree_oid)
        .map_err(|_| prepared_merge_mismatch("recorded resolution tree object id is malformed"))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|_| prepared_merge_mismatch("recorded resolution tree object is unavailable"))?;
    let index = repo.index().map_err(git_error)?;
    let diff = repo
        .diff_tree_to_index(Some(&tree), Some(&index), None)
        .map_err(git_error)?;
    if diff.deltas().len() != 0 {
        return Err(prepared_merge_mismatch(
            "resolved index tree changed after intent persistence",
        ));
    }
    Ok(ValidatedPreparedMergeResolution {
        before,
        merge_head,
        tree,
        author,
        committer,
    })
}

pub(super) fn ensure_clean_recovery_state(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    branch: &str,
) -> ModelResult<()> {
    if repo.state() != git2::RepositoryState::Clean {
        return Err(recovery_drift(format!(
            "rollback found integration state {:?}",
            repo.state()
        )));
    }
    let head = repo_head(repo)?;
    if head.is_detached || head.branch.as_deref() != Some(branch) {
        return Err(recovery_drift(format!(
            "rollback target is not the attached branch '{branch}'"
        )));
    }
    // Rollback availability: untracked checked-artifact private residue (the
    // permanent Windows durability anchor) is product infrastructure, never a
    // reason to refuse a checked rollback.
    if status_dirty_outside_checked_artifact_private(&backend.status(path)?) {
        return Err(recovery_dirty(
            "rollback requires a clean index and worktree",
        ));
    }
    Ok(())
}

pub(super) fn verify_restored_merge_state(
    backend: &impl GitBackend,
    path: &Path,
    before: git2::Oid,
) -> ModelResult<()> {
    let branch = backend.head(path)?.branch.ok_or_else(|| {
        recovery_drift("repository is detached after restoring the pre-merge state")
    })?;
    verify_merge_result(backend, path, &branch, &before.to_string())
}
