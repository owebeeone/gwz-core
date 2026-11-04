//! The checkout pull's merge and fast-forward run (`WorktreeRule::checkout_tree`): under
//! pull's rule libgit2 refuses to overwrite an ignored entry, and gwz merge's rule still
//! replaces one, as in 1.0.18.

use std::fs;

use crate::git::tests::{TempDir, commit_file, run_git};

use super::*;

/// `main` at A (`f.txt`), a branch `feature` one commit ahead that adds `out.txt` (and,
/// with `directory`, a file `build` where the user has an ignored directory); the
/// worktree is on `main` and holds the ignored entries.
fn repository(temp: &TempDir, directory: bool) -> (std::path::PathBuf, git2::Oid) {
    let repo = temp.path().join("repo");
    Git2Backend::new().create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let b = commit_file(&repo, "out.txt", "incoming\n", "B", &[a_oid]).unwrap();
    let mut tip = git2::Oid::from_str(&b).unwrap();
    if directory {
        tip = git2::Oid::from_str(&commit_file(&repo, "build", "incoming\n", "C", &[tip]).unwrap())
            .unwrap();
    }
    run_git(&repo, &["checkout", "main"]);
    fs::write(repo.join(".git/info/exclude"), "out.txt\nbuild\n").unwrap();
    fs::write(repo.join("out.txt"), "MINE\n").unwrap();
    if directory {
        fs::create_dir_all(repo.join("build")).unwrap();
        fs::write(repo.join("build/obj.o"), "MINE\n").unwrap();
    }
    (repo, tip)
}

#[test]
fn pull_checkout_refuses_to_overwrite_an_ignored_file_or_directory() {
    let temp = TempDir::new("pull-checkout-ignored");
    let (repo_path, tip) = repository(&temp, true);
    let repo = git2::Repository::open(&repo_path).unwrap();
    let object = repo.find_object(tip, None).unwrap();

    let error = WorktreeRule::AllowUntracked
        .checkout_tree(&repo, &object)
        .unwrap_err();

    assert_eq!(error.code(), git2::ErrorCode::Conflict, "{error}");
    assert_eq!(
        fs::read_to_string(repo_path.join("out.txt")).unwrap(),
        "MINE\n"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("build/obj.o")).unwrap(),
        "MINE\n"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("f.txt")).unwrap(),
        "a\n",
        "the conflict pass wrote before it failed"
    );
}

#[test]
fn the_released_merge_rule_still_replaces_an_ignored_entry() {
    let temp = TempDir::new("merge-checkout-ignored");
    let (repo_path, tip) = repository(&temp, false);
    let repo = git2::Repository::open(&repo_path).unwrap();
    let object = repo.find_object(tip, None).unwrap();

    WorktreeRule::RequireClean
        .checkout_tree(&repo, &object)
        .unwrap();

    assert_eq!(
        fs::read_to_string(repo_path.join("out.txt")).unwrap(),
        "incoming\n"
    );
}
