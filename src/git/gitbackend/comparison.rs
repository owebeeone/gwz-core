use super::repository_support::open_repo;
use super::*;

pub(super) fn merge_base(
    _backend: &Git2Backend,
    path: &Path,
    left: &str,
    right: &str,
) -> ModelResult<Option<String>> {
    let repo = open_repo(path)?;
    let left = git2::Oid::from_str(left).map_err(git_error)?;
    let right = git2::Oid::from_str(right).map_err(git_error)?;
    match repo.merge_base(left, right) {
        Ok(oid) => Ok(Some(oid.to_string())),
        Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(err) => Err(git_error(err)),
    }
}

pub(super) fn changed_paths_between(
    _backend: &Git2Backend,
    path: &Path,
    old_commit: &str,
    new_commit: &str,
) -> ModelResult<Vec<String>> {
    let repo = open_repo(path)?;
    let old = repo
        .find_commit(git2::Oid::from_str(old_commit).map_err(git_error)?)
        .map_err(git_error)?;
    let new = repo
        .find_commit(git2::Oid::from_str(new_commit).map_err(git_error)?)
        .map_err(git_error)?;
    let old_tree = old.tree().map_err(git_error)?;
    let new_tree = new.tree().map_err(git_error)?;
    let diff = repo
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
        .map_err(git_error)?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        for file in [delta.old_file(), delta.new_file()] {
            if let Some(path) = file.path() {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(super) fn diff_manifest(
    _backend: &Git2Backend,
    path: &Path,
    comparison: &crate::diff::RepoDiffComparison,
    options: &crate::diff::RepoDiffOptions,
) -> ModelResult<crate::diff::RepoDiffManifest> {
    let repo = open_repo(path)?;
    crate::diff::diff_repo(&repo, comparison, options)
}

pub(super) fn resolve_comparison(
    _backend: &Git2Backend,
    path: &Path,
    spec: &crate::diff::ComparisonSpec,
) -> ModelResult<crate::diff::RepoDiffComparison> {
    let repo = open_repo(path)?;
    crate::diff::resolve_comparison(&repo, spec)
}
