//! Design §4.1's copy exclusions, resolved by core for the tree copier.
//!
//! The exclusion set is applied during traversal (an excluded entry is
//! never written and then removed), and it is the *copy-time* column of the
//! design's table: family files, the catalog, the runtime locks, the merge
//! store, stash bundles, the manifest that is regenerated last, and every
//! included repository's `.git/worktrees/`. The *at-ready* column is checked
//! separately by the destination observation ([`super::install`]).

use std::path::{Path, PathBuf};

use gwz_copy_contract::Exclusion;

/// Source-root-relative entries omitted from every verbatim copy, whatever
/// repositories the source holds (design §4.1, rows one to ten).
pub const FIXED_EXCLUSIONS: [&str; 10] = [
    gwz_family_model::INDEX_RELATIVE_PATH,
    gwz_family_model::LOCK_RELATIVE_PATH,
    gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH,
    gwz_family_model::POINTER_RELATIVE_PATH,
    ".gwz/catalog-final",
    ".gwz/checked-artifacts",
    ".gwz/locks",
    ".gwz/merge",
    crate::stash::STASH_BUNDLE_DIR,
    crate::workspace::WORKSPACE_MANIFEST,
];

/// The `.git/worktrees/` entry of one included repository, given its Git
/// directory relative to the source root (design §4.1, "`.git/worktrees/` in
/// every included repository: omit").
pub fn worktrees_of(relative_git_dir: &Path) -> PathBuf {
    relative_git_dir.join("worktrees")
}

/// The whole exclusion set for a verbatim copy of a source whose included
/// repositories have the given source-root-relative Git directories.
pub fn verbatim_exclusions<'a>(
    relative_git_dirs: impl IntoIterator<Item = &'a Path>,
) -> Vec<Exclusion> {
    let mut exclusions: Vec<Exclusion> = FIXED_EXCLUSIONS
        .iter()
        .map(|fixed| Exclusion::RelativePath(PathBuf::from(fixed)))
        .collect();
    exclusions.extend(
        relative_git_dirs
            .into_iter()
            .map(|git_dir| Exclusion::RelativePath(worktrees_of(git_dir))),
    );
    exclusions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Design §4.1's copy-time column, row for row, plus each repository's
    /// `.git/worktrees/`; the lock file is copied (it is not in the table's
    /// "omit" rows) and so is `gwz.conf/gwz.lock.yml`.
    #[test]
    fn the_exclusion_set_is_the_designs_copy_time_column() {
        let git_dirs = [PathBuf::from(".git"), PathBuf::from("app/.git")];
        let exclusions = verbatim_exclusions(git_dirs.iter().map(PathBuf::as_path));
        let excluded = |relative: &str| {
            exclusions
                .iter()
                .any(|exclusion| exclusion.matches(Path::new(relative)))
        };
        for omitted in [
            ".gwz/local-family.yml",
            ".gwz/local-family.lock",
            ".gwz/local-clone-allocation",
            ".gwz/family-root",
            ".gwz/catalog-final",
            ".gwz/catalog-final/anything",
            ".gwz/checked-artifacts/x",
            ".gwz/locks/workspace.lock",
            ".gwz/merge",
            ".gwz/merge/done/m1.yaml",
            ".gwz/stash/bundles/s.yaml",
            "gwz.conf/gwz.yml",
            ".git/worktrees/lane",
            "app/.git/worktrees/lane",
        ] {
            assert!(excluded(omitted), "{omitted} is omitted during copy");
        }
        for copied in [
            "gwz.conf/gwz.lock.yml",
            "gwz.conf/markers/conf-integrity.yml",
            ".git/config",
            ".git/index",
            ".git/info/exclude",
            "app/.git/HEAD",
            "app/src/main.rs",
            "target/debug/build",
            ".gwz/stash",
        ] {
            assert!(!excluded(copied), "{copied} is copied verbatim");
        }
        assert_eq!(exclusions.len(), FIXED_EXCLUSIONS.len() + 2);
    }
}
