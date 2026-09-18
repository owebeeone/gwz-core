use super::*;

#[test]
pub(crate) fn reports_commit_ancestry_without_moving_head() {
    let temp = TempDir::new("ancestry");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();
    let first = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
    let first_oid = git2::Oid::from_str(&first).unwrap();
    let second = commit_file(&repo_path, "README.md", "two", "second", &[first_oid]).unwrap();

    assert!(backend.is_ancestor(&repo_path, &first, &second).unwrap());
    assert!(!backend.is_ancestor(&repo_path, &second, &first).unwrap());
    assert_eq!(backend.head(&repo_path).unwrap().commit, Some(second));
}

/// GwzFetchPlan.md D6: `gwz fetch` reports `+A -B`, which `is_ancestor` cannot
/// answer. The counts are symmetric, zero for the same commit, and a linear
/// history reports one side only.
#[test]
pub(crate) fn counts_how_far_a_commit_is_ahead_of_and_behind_another() {
    let temp = TempDir::new("ahead-behind");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();
    let base = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    let second = commit_file(&repo_path, "README.md", "two", "second", &[base_oid]).unwrap();
    let second_oid = git2::Oid::from_str(&second).unwrap();
    let third = commit_file(&repo_path, "README.md", "three", "third", &[second_oid]).unwrap();

    let same = backend.ahead_behind(&repo_path, &base, &base).unwrap();
    assert_eq!((same.ahead, same.behind), (0, 0));

    let forward = backend.ahead_behind(&repo_path, &third, &base).unwrap();
    assert_eq!((forward.ahead, forward.behind), (2, 0));

    let backward = backend.ahead_behind(&repo_path, &base, &third).unwrap();
    assert_eq!((backward.ahead, backward.behind), (0, 2));

    // A divergence: a second child of the base on its own branch, so each
    // side has exactly one commit the other does not.
    backend.branch_create(&repo_path, "side", &base).unwrap();
    backend.switch_branch(&repo_path, "side").unwrap();
    let sibling = commit_file(&repo_path, "OTHER.md", "side", "sibling", &[base_oid]).unwrap();
    let diverged = backend.ahead_behind(&repo_path, &sibling, &second).unwrap();
    assert_eq!((diverged.ahead, diverged.behind), (1, 1));
}
