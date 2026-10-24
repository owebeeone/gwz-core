use super::repository_support::open_repo;
use super::*;

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
