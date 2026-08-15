use std::fs;

use crate::model::ErrorCode;

use super::*;

mod root_preservation;

fn seeded_repo(name: &str) -> (TempDir, Git2Backend, std::path::PathBuf, String) {
    let temp = TempDir::new(name);
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    // Pin conversion off: these tests compare worktree bytes to blob bytes,
    // and Windows CI inherits core.autocrlf=true from the runner image.
    git2::Repository::open(&repo)
        .unwrap()
        .config()
        .unwrap()
        .set_bool("core.autocrlf", false)
        .unwrap();
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
fn checked_backup_ref_rejects_an_advanced_attached_head_without_creating_a_ref() {
    let (_temp, backend, repo, first) = seeded_repo("merge-preservation-checked-ref");
    let second = commit_file(
        &repo,
        "second.txt",
        "second\n",
        "second",
        &[first.parse().unwrap()],
    )
    .unwrap();
    let name = "refs/gwz/merge/merge_1/mem_app/head";
    let error = backend
        .create_backup_ref_checked(&repo, "main", &first, name, &first)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(
        backend.head(&repo).unwrap().commit.as_deref(),
        Some(second.as_str())
    );
    assert_eq!(
        backend.observe_direct_ref(&repo, name).unwrap(),
        GitDirectRefObservation::Absent
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
fn checked_preservation_stash_binds_head_and_complete_preimage() {
    let (_temp, backend, repo, head) = seeded_repo("merge-preservation-checked-stash");
    fs::write(repo.join("tracked.txt"), "first pending value\n").unwrap();
    fs::write(repo.join("untracked.txt"), "first untracked value\n").unwrap();
    let expected = backend.preservation_image(&repo, true).unwrap();
    fs::write(repo.join("tracked.txt"), "changed after intent\n").unwrap();
    let error = backend
        .stash_for_merge_preservation_checked(
            &repo,
            "main",
            &head,
            &expected.preimage_sha256,
            "merge_1",
            true,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert!(
        backend
            .preservation_stashes(&repo, "merge_1")
            .unwrap()
            .is_empty()
    );
    assert_text_eq(repo.join("tracked.txt"), "changed after intent\n");
    assert_text_eq(repo.join("untracked.txt"), "first untracked value\n");

    let exact = backend.preservation_image(&repo, true).unwrap();
    let saved = backend
        .stash_for_merge_preservation_checked(
            &repo,
            "main",
            &head,
            &exact.preimage_sha256,
            "merge_1",
            true,
        )
        .unwrap();
    assert_eq!(
        backend
            .stash_for_merge_preservation_checked(
                &repo,
                "main",
                &head,
                &exact.preimage_sha256,
                "merge_1",
                true,
            )
            .unwrap(),
        saved
    );
}

#[test]
fn checked_preservation_stash_rejects_changes_at_the_native_boundary() {
    for case in ["head", "tracked", "staged", "untracked"] {
        let (_temp, backend, repo, head) =
            seeded_repo(&format!("merge-preservation-boundary-{case}"));
        fs::write(repo.join("tracked.txt"), "pending value\n").unwrap();
        let expected = backend.preservation_image(&repo, true).unwrap();
        let callback_repo = repo.clone();
        let callback_head = head.clone();
        Git2Backend::before_next_preservation_stash(move || match case {
            "head" => {
                commit_file(
                    &callback_repo,
                    "late.txt",
                    "late\n",
                    "late",
                    &[callback_head.parse().unwrap()],
                )
                .unwrap();
            }
            "tracked" => fs::write(callback_repo.join("tracked.txt"), "late tracked\n").unwrap(),
            "staged" => {
                fs::write(callback_repo.join("late-staged.txt"), "late staged\n").unwrap();
                let status = std::process::Command::new("git")
                    .args(["add", "late-staged.txt"])
                    .current_dir(&callback_repo)
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            "untracked" => {
                fs::write(callback_repo.join("late-untracked.txt"), "late untracked\n").unwrap();
            }
            _ => unreachable!(),
        });
        let error = backend
            .stash_for_merge_preservation_checked(
                &repo,
                "main",
                &head,
                &expected.preimage_sha256,
                "merge_1",
                true,
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::PreservationEvidenceMismatch,
            "{case}"
        );
        assert!(
            backend
                .preservation_stashes(&repo, "merge_1")
                .unwrap()
                .is_empty(),
            "{case}"
        );
    }
}

#[test]
fn checked_preservation_stash_rejects_matching_and_foreign_stash_set_changes_at_the_native_boundary()
 {
    for matching in [false, true] {
        let label = if matching { "matching" } else { "foreign" };
        let (_temp, backend, repo, head) =
            seeded_repo(&format!("merge-preservation-boundary-{label}-stash"));
        fs::write(repo.join("tracked.txt"), "pending value\n").unwrap();
        let expected = backend.preservation_image(&repo, true).unwrap();
        let callback_repo = repo.clone();
        Git2Backend::before_next_preservation_stash(move || {
            let callback_backend = Git2Backend::new();
            if matching {
                callback_backend
                    .stash_for_merge_preservation(&callback_repo, "merge_1", true)
                    .unwrap();
            } else {
                callback_backend
                    .stash_push(
                        &callback_repo,
                        "foreign stash inserted at boundary",
                        GitStashPushOptions {
                            include_untracked: true,
                            include_ignored: false,
                            preserve_index: false,
                        },
                    )
                    .unwrap();
            }
        });
        let error = backend
            .stash_for_merge_preservation_checked(
                &repo,
                "main",
                &head,
                &expected.preimage_sha256,
                "merge_1",
                true,
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::PreservationEvidenceMismatch,
            "{label}"
        );
        let stashes = backend.stash_list(&repo).unwrap();
        assert_eq!(stashes.len(), 1, "{label}");
        assert_eq!(
            backend
                .preservation_stashes(&repo, "merge_1")
                .unwrap()
                .len(),
            usize::from(matching),
            "{label}"
        );
    }
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
fn complete_checkout_comparison_rejects_hidden_drift_and_honors_only_exact_exclusions() {
    let (_temp, backend, repo, head) = seeded_repo("complete-checkout-comparison");
    assert!(
        backend
            .checkout_matches_commit_except(&repo, &head, &[])
            .unwrap()
    );

    let status = std::process::Command::new("git")
        .args(["update-index", "--assume-unchanged", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(repo.join("tracked.txt"), "hidden drift\n").unwrap();
    assert_eq!(
        backend
            .checkout_matches_commit_except(&repo, &head, &[])
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );

    let status = std::process::Command::new("git")
        .args(["update-index", "--no-assume-unchanged", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let status = std::process::Command::new("git")
        .args(["checkout", "--", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let status = std::process::Command::new("git")
        .args(["update-index", "--skip-worktree", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(repo.join("tracked.txt"), "skip-worktree drift\n").unwrap();
    assert_eq!(
        backend
            .checkout_matches_commit_except(&repo, &head, &[])
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    let status = std::process::Command::new("git")
        .args(["update-index", "--no-skip-worktree", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        backend
            .checkout_matches_commit_except(&repo, &head, &["tracked.txt".into()])
            .unwrap()
    );
    fs::write(repo.join("other.txt"), "unrelated\n").unwrap();
    assert!(
        !backend
            .checkout_matches_commit_except(&repo, &head, &["tracked.txt".into()])
            .unwrap()
    );
}

#[test]
fn complete_checkout_excludes_only_the_checked_artifact_private_tree() {
    let (_temp, backend, repo, head) = seeded_repo("complete-checkout-private-recovery");
    let head = commit_file(
        &repo,
        ".gwz/checked-artifacts/protocol",
        "private baseline\n",
        "private recovery state",
        &[git2::Oid::from_str(&head).unwrap()],
    )
    .unwrap();
    let head = commit_file(
        &repo,
        ".gwz/sibling",
        "sibling baseline\n",
        "neighboring workspace state",
        &[git2::Oid::from_str(&head).unwrap()],
    )
    .unwrap();

    fs::write(
        repo.join(".gwz/checked-artifacts/protocol"),
        "private live residue\n",
    )
    .unwrap();
    assert!(
        backend
            .checkout_matches_commit_except(&repo, &head, &[])
            .unwrap()
    );

    fs::write(
        repo.join(".gwz/checked-artifacts/protocol"),
        "private baseline\n",
    )
    .unwrap();
    fs::write(repo.join(".gwz/sibling"), "sibling drift\n").unwrap();
    assert!(
        !backend
            .checkout_matches_commit_except(&repo, &head, &[])
            .unwrap()
    );
}

#[test]
fn complete_checkout_overlays_worktree_without_erasing_index_authority() {
    let (_temp, backend, repo, head) = seeded_repo("complete-checkout-domain-overlay");
    let overlay = GitCheckoutOverlay {
        worktree_paths: vec!["tracked.txt".into()],
        index_paths: Vec::new(),
    };
    fs::write(repo.join("tracked.txt"), "worktree overlay\n").unwrap();
    assert!(
        backend
            .checkout_matches_commit_with_overlay(&repo, &head, &overlay)
            .unwrap()
    );

    stage_path(&repo, "tracked.txt").unwrap();
    assert!(
        !backend
            .checkout_matches_commit_with_overlay(&repo, &head, &overlay)
            .unwrap()
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
