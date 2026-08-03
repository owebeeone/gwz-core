use std::fs;

use crate::model::ErrorCode;

use super::*;

// GwzStashBranchPlan B2: local branch primitives only.

#[test]
fn branch_create_lists_and_is_idempotent_at_same_commit() {
    let temp = TempDir::new("branch-create");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let base = commit_file(&repo, "f.txt", "base\n", "base", &[]).unwrap();

    let result = backend.branch_create(&repo, "feature", "HEAD").unwrap();
    assert!(result.created);
    assert_eq!(result.branch.name, "feature");
    assert_eq!(result.branch.commit, base);
    assert!(!result.branch.is_current);

    let again = backend.branch_create(&repo, "feature", "HEAD").unwrap();
    assert!(!again.created, "existing branch at same commit is success");
    assert_eq!(again.branch.commit, base);

    assert_eq!(
        backend.branch_list(&repo).unwrap(),
        vec![
            GitBranch {
                name: "feature".to_owned(),
                commit: base.clone(),
                is_current: false,
            },
            GitBranch {
                name: "main".to_owned(),
                commit: base,
                is_current: true,
            },
        ]
    );
}

#[test]
fn branch_create_refuses_existing_branch_at_different_commit() {
    let temp = TempDir::new("branch-diverged");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    backend.branch_create(&repo, "feature", "HEAD").unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();

    let err = backend.branch_create(&repo, "feature", "HEAD").unwrap_err();
    assert_eq!(err.code, ErrorCode::DivergedMember);
    assert_eq!(
        rev_parse(&repo, "refs/heads/feature"),
        a,
        "failed create must not move the existing branch"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), b);
}

#[test]
fn switch_branch_checks_out_existing_branch_without_moving_it() {
    let temp = TempDir::new("branch-switch");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    backend.branch_create(&repo, "feature", "HEAD").unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();

    let result = backend.switch_branch(&repo, "feature").unwrap();

    assert!(result.updated);
    assert_eq!(result.commit.as_deref(), Some(a.as_str()));
    assert_eq!(rev_parse(&repo, "refs/heads/feature"), a);
    assert_eq!(rev_parse(&repo, "refs/heads/main"), b);
    assert_text_eq(repo.join("f.txt"), "a\n");
    let head = backend.head(&repo).unwrap();
    assert!(!head.is_detached);
    assert_eq!(head.branch.as_deref(), Some("feature"));
}

#[test]
fn switch_branch_rejects_missing_branch_without_mutation() {
    let temp = TempDir::new("branch-switch-missing");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let base = commit_file(&repo, "f.txt", "base\n", "base", &[]).unwrap();

    let err = backend.switch_branch(&repo, "missing").unwrap_err();
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
    assert_eq!(rev_parse(&repo, "HEAD"), base);
    let head = backend.head(&repo).unwrap();
    assert!(!head.is_detached);
    assert_eq!(head.branch.as_deref(), Some("main"));
}

#[test]
fn switch_branch_at_current_head_preserves_index_worktree_and_untracked_files() {
    let temp = TempDir::new("branch-switch-dirty-same-head");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let base = commit_file(&repo, "f.txt", "base\n", "base", &[]).unwrap();
    backend.branch_create(&repo, "feature", "HEAD").unwrap();
    fs::write(repo.join("f.txt"), "staged\n").unwrap();
    run_git(&repo, &["add", "f.txt"]);
    fs::write(repo.join("f.txt"), "unstaged\n").unwrap();
    fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
    let index_before = fs::read(repo.join(".git/index")).unwrap();
    let status_before = backend.status(&repo).unwrap();

    let result = backend.switch_branch(&repo, "feature").unwrap();

    assert!(result.updated);
    assert_eq!(result.commit.as_deref(), Some(base.as_str()));
    assert_eq!(
        backend.head(&repo).unwrap().branch.as_deref(),
        Some("feature")
    );
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_text_eq(repo.join("f.txt"), "unstaged\n");
    assert_text_eq(repo.join("untracked.txt"), "untracked\n");
    assert_eq!(backend.status(&repo).unwrap(), status_before);
}

#[test]
fn switch_branch_rejects_dirty_worktree_when_target_moves_head() {
    let temp = TempDir::new("branch-switch-dirty");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    backend.branch_create(&repo, "feature", "HEAD").unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();
    fs::write(repo.join("f.txt"), "dirty\n").unwrap();

    let err = backend.switch_branch(&repo, "feature").unwrap_err();
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
    assert_eq!(rev_parse(&repo, "HEAD"), b);
    assert_eq!(rev_parse(&repo, "refs/heads/feature"), a);
    assert_text_eq(repo.join("f.txt"), "dirty\n");
    assert_eq!(backend.head(&repo).unwrap().branch.as_deref(), Some("main"));
}

#[test]
fn branch_delete_refuses_current_branch_and_deletes_non_current_branch() {
    let temp = TempDir::new("branch-delete");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "f.txt", "base\n", "base", &[]).unwrap();
    backend.branch_create(&repo, "feature", "HEAD").unwrap();

    let err = backend.branch_delete(&repo, "main").unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert_eq!(backend.head(&repo).unwrap().branch.as_deref(), Some("main"));

    backend.branch_delete(&repo, "feature").unwrap();
    assert!(
        backend
            .branch_list(&repo)
            .unwrap()
            .iter()
            .all(|branch| branch.name != "feature"),
        "deleted branch should no longer be listed"
    );
}
