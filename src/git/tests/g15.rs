use std::fs;

use crate::model::ErrorCode;

use super::*;

fn seeded_repo(name: &str) -> (TempDir, Git2Backend, std::path::PathBuf, String) {
    let temp = TempDir::new(name);
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let head = commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();
    (temp, backend, repo, head)
}

#[test]
fn preservation_ref_creation_and_checked_deletion_are_idempotent_and_fail_closed() {
    let (_temp, backend, repo, first) = seeded_repo("merge-preservation-ref");
    let first_oid = git2::Oid::from_str(&first).unwrap();
    let second = commit_file(&repo, "second.txt", "second\n", "second", &[first_oid]).unwrap();
    let name = "refs/gwz/merge/merge_1/mem_app/head";

    let created = backend.create_backup_ref(&repo, name, &first).unwrap();
    assert_eq!(created.name, name);
    assert_eq!(created.target, first);
    assert_eq!(
        backend.create_backup_ref(&repo, name, &first).unwrap(),
        created
    );
    assert_eq!(
        backend
            .create_backup_ref(&repo, name, &second)
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_eq!(backend.read_ref(&repo, name).unwrap(), Some(first.clone()));
    assert_eq!(
        backend
            .delete_backup_ref_checked(&repo, name, &second)
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    backend
        .delete_backup_ref_checked(&repo, name, &first)
        .unwrap();
    assert_eq!(backend.read_ref(&repo, name).unwrap(), None);
    backend
        .delete_backup_ref_checked(&repo, name, &first)
        .unwrap();

    assert_eq!(
        backend
            .create_backup_ref(&repo, "refs/heads/not-private", &first)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn merge_preservation_stash_is_verified_idempotent_and_excludes_ignored_files() {
    let (_temp, backend, repo, head) = seeded_repo("merge-preservation-stash");
    let head_oid = git2::Oid::from_str(&head).unwrap();
    commit_file(&repo, ".gitignore", "ignored.txt\n", "ignore", &[head_oid]).unwrap();
    fs::write(repo.join("tracked.txt"), "changed\n").unwrap();
    fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
    fs::write(repo.join("ignored.txt"), "ignored\n").unwrap();

    let saved = backend
        .stash_for_merge_preservation(&repo, "merge_1", true)
        .unwrap();
    assert_eq!(saved.message, "gwz:stash_merge_1: merge preservation");
    assert_eq!(backend.status(&repo).unwrap(), GitStatus::clean());
    assert!(repo.join("ignored.txt").exists());
    assert!(!repo.join("untracked.txt").exists());
    assert!(
        backend
            .stash_list(&repo)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == saved.object_id)
    );
    assert_eq!(
        backend
            .stash_for_merge_preservation(&repo, "merge_1", true)
            .unwrap(),
        saved
    );

    fs::write(repo.join("tracked.txt"), "later\n").unwrap();
    assert_eq!(
        backend
            .stash_for_merge_preservation(&repo, "merge_1", true)
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_text_eq(repo.join("tracked.txt"), "later\n");
}
