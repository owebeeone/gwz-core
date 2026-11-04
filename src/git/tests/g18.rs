//! gwz 1.0.19: the checkouts pull runs through the backend (`fast_forward`, the rebase of
//! `rebase_onto`) overwrite no ignored entry. libgit2 treats one at a path it writes as a
//! conflict, reported by its conflict pass before it writes or removes anything, so a race
//! between pull's check and the update fails safe instead of replacing the entry. Pull's
//! own checks refuse every such entry first (rows 17 to 34 of `g28`); these cover the
//! checkouts themselves.
use std::fs;

use crate::model::ErrorCode;

use super::*;

/// `main` and `feature` diverge from A: `feature` adds `out.txt`; `main` adds `main.txt`.
/// The worktree is on `main` with `out.txt` ignored and holding the user's content.
fn ignored_entry_in_the_way_of_feature(temp: &TempDir, diverge: bool) -> std::path::PathBuf {
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    commit_file(&repo, "out.txt", "incoming\n", "B", &[a_oid]).unwrap();
    run_git(&repo, &["checkout", "main"]);
    if diverge {
        commit_file(&repo, "main.txt", "main\n", "D", &[a_oid]).unwrap();
    }
    fs::write(repo.join(".git/info/exclude"), "out.txt\n").unwrap();
    fs::write(repo.join("out.txt"), "MINE\n").unwrap();
    repo
}

#[test]
fn fast_forward_does_not_overwrite_an_ignored_entry() {
    let temp = TempDir::new("ff-ignored");
    let repo = ignored_entry_in_the_way_of_feature(&temp, false);
    let before = rev_parse(&repo, "HEAD");

    let error = Git2Backend::new()
        .fast_forward(&repo, "main", "refs/heads/feature")
        .unwrap_err();

    eprintln!("fast_forward refusal: {error:?}");
    assert_eq!(error.code, ErrorCode::GitCommandFailed, "{}", error.message);
    assert_eq!(rev_parse(&repo, "HEAD"), before, "the branch moved");
    assert_text_eq(repo.join("out.txt"), "MINE\n");
    assert_text_eq(repo.join("f.txt"), "a\n");
}

#[test]
fn rebase_onto_does_not_overwrite_an_ignored_entry_in_the_way_of_the_onto_checkout() {
    let temp = TempDir::new("rebase-ignored-onto");
    let repo = ignored_entry_in_the_way_of_feature(&temp, true);
    let before = rev_parse(&repo, "HEAD");

    let error = Git2Backend::new()
        .rebase_onto(&repo, "main", "refs/heads/feature")
        .unwrap_err();

    eprintln!(
        "onto-checkout refusal: {error:?} (rebase-merge left behind: {})",
        repo.join(".git/rebase-merge").exists()
    );
    assert_eq!(error.code, ErrorCode::GitCommandFailed, "{}", error.message);
    assert_eq!(rev_parse(&repo, "HEAD"), before, "the branch moved");
    assert_text_eq(repo.join("out.txt"), "MINE\n");
}

#[test]
fn rebase_onto_does_not_overwrite_an_ignored_entry_in_the_way_of_a_replayed_commit() {
    // main: C adds x.txt, D removes it again; feature: B adds other.txt. The replay of C
    // writes x.txt, which the user has as an ignored file (the onto checkout does not
    // touch it). Overwriting it would have D delete it afterwards.
    let temp = TempDir::new("rebase-ignored-replay");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    commit_file(&repo, "other.txt", "other\n", "B", &[a_oid]).unwrap();
    run_git(&repo, &["checkout", "main"]);
    let c = commit_file(&repo, "x.txt", "x\n", "C", &[a_oid]).unwrap();
    run_git(&repo, &["rm", "-q", "x.txt"]);
    run_git(&repo, &["commit", "-q", "-m", "D"]);
    assert!(!repo.join("x.txt").exists(), "{c}");
    fs::write(repo.join(".git/info/exclude"), "x.txt\n").unwrap();
    fs::write(repo.join("x.txt"), "MINE\n").unwrap();

    let error = backend
        .rebase_onto(&repo, "main", "refs/heads/feature")
        .unwrap_err();

    eprintln!(
        "replayed-commit refusal: {error:?} (rebase-merge left behind: {})",
        repo.join(".git/rebase-merge").exists()
    );
    assert_eq!(error.code, ErrorCode::GitCommandFailed, "{}", error.message);
    assert_text_eq(repo.join("x.txt"), "MINE\n");
}
