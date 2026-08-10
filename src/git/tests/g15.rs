use std::fs;

use crate::model::ErrorCode;

use super::*;

mod root_preservation;

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
fn direct_ref_observation_never_resolves_symbolic_references() {
    let (_temp, backend, repo, head) = seeded_repo("merge-preservation-direct-ref");
    let name = "refs/gwz/merge/merge_1/mem_app/head";
    assert_eq!(
        backend.observe_direct_ref(&repo, name).unwrap(),
        GitDirectRefObservation::Absent
    );

    backend.create_backup_ref(&repo, name, &head).unwrap();
    assert_eq!(
        backend.observe_direct_ref(&repo, name).unwrap(),
        GitDirectRefObservation::Direct {
            target: head.clone()
        }
    );

    let native = git2::Repository::open(&repo).unwrap();
    native
        .reference_symbolic(name, "refs/heads/main", true, "test resolving symref")
        .unwrap();
    assert_eq!(
        backend.observe_direct_ref(&repo, name).unwrap(),
        GitDirectRefObservation::NonDirect
    );

    native
        .reference_symbolic(
            name,
            "refs/heads/does-not-exist",
            true,
            "test broken symref",
        )
        .unwrap();
    assert_eq!(
        backend.observe_direct_ref(&repo, name).unwrap(),
        GitDirectRefObservation::NonDirect
    );
}

#[test]
fn merge_preservation_stash_is_verified_idempotent_and_excludes_ignored_files() {
    let (_temp, backend, repo, head) = seeded_repo("merge-preservation-stash");
    let head_oid = git2::Oid::from_str(&head).unwrap();
    commit_file(&repo, ".gitignore", "ignored.txt\n", "ignore", &[head_oid]).unwrap();
    fs::write(repo.join("tracked.txt"), "changed\n").unwrap();
    backend.stage_paths(&repo, &["tracked.txt"]).unwrap();
    fs::write(repo.join("tracked.txt"), "changed again\n").unwrap();
    fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
    fs::write(repo.join("ignored.txt"), "ignored\n").unwrap();

    let before = backend.preservation_image(&repo, true).unwrap();
    assert_eq!(
        before.dirty,
        GitPreservationDirtySummary {
            staged: true,
            unstaged: true,
            untracked: true,
        }
    );

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
    let decoded = backend.preservation_stashes(&repo, "merge_1").unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].object_id, saved.object_id);
    assert_eq!(decoded[0].message, saved.message);
    assert_eq!(decoded[0].image, before);
    assert!(
        backend
            .checkout_matches_commit(&repo, "main", &decoded[0].head_commit)
            .unwrap()
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
    assert!(
        !backend
            .checkout_matches_commit(&repo, "main", &decoded[0].head_commit)
            .unwrap()
    );
}

#[test]
fn preservation_image_rejects_semantic_index_flags() {
    let (_temp, backend, repo, _head) = seeded_repo("merge-preservation-index-flags");
    let status = std::process::Command::new("git")
        .args(["update-index", "--assume-unchanged", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        backend.preservation_image(&repo, true).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn preservation_image_orders_non_utf8_paths_as_raw_bytes() {
    let first = (vec![b'z', 0x80], b"first".to_vec());
    let second = (vec![b'z', 0x81], b"second".to_vec());
    let forward = raw_path_preimage_for_test([first.clone(), second.clone()]).unwrap();
    let reverse = raw_path_preimage_for_test([second, first]).unwrap();

    assert_eq!(forward, reverse);
    assert!(forward.dirty.untracked);
    assert_ne!(
        forward.preimage_sha256,
        raw_path_preimage_for_test([(vec![b'z', 0x81], b"first".to_vec())])
            .unwrap()
            .preimage_sha256
    );
}
