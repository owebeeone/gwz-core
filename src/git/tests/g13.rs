use std::{fs, path::Path};

use sha2::{Digest, Sha256};

use crate::model::ErrorCode;

use super::*;

fn candidate(path: &str, bytes: &str) -> GitCandidateFile {
    GitCandidateFile {
        path: path.into(),
        bytes: bytes.as_bytes().to_vec(),
    }
}

fn seed(root: &Path, files: &[(&str, &str)]) -> String {
    Git2Backend::new().create_repo(root).unwrap();
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "seed"]);
    let repo = git2::Repository::open(root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "GWZ Publisher").unwrap();
    config
        .set_str("user.email", "publisher@example.invalid")
        .unwrap();
    rev_parse(root, "HEAD")
}

#[test]
fn scoped_commit_is_exact_recoverable_and_preserves_user_state() {
    let temp = TempDir::new("scoped-commit");
    let root = temp.path().join("repo");
    let parent = seed(
        &root,
        &[
            ("tracked.txt", "keep\n"),
            ("staged.txt", "base staged\n"),
            ("dirty.txt", "base dirty\n"),
            ("gwz.conf/gwz.lock", "old lock\n"),
            ("gwz.conf/user-note", "tracked note\n"),
        ],
    );
    fs::write(root.join("staged.txt"), "user staged\n").unwrap();
    stage_path(&root, "staged.txt").unwrap();
    fs::write(root.join("dirty.txt"), "user dirty\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
    fs::write(root.join("gwz.conf/local-only"), "local\n").unwrap();
    let index_before = fs::read(root.join(".git/index")).unwrap();
    let backend = Git2Backend::new();
    let result = backend
        .commit_gwz_paths_checked(
            &root,
            Some(&parent),
            &[
                candidate("gwz.conf/merge/marker.yml", "marker\n"),
                candidate("gwz.conf/gwz.lock", "new lock\n"),
            ],
            "Publish composition\n",
        )
        .unwrap();

    let repo = git2::Repository::open(&root).unwrap();
    let commit = repo.find_commit(result.commit.parse().unwrap()).unwrap();
    assert_eq!(commit.parent_id(0).unwrap().to_string(), parent);
    assert_eq!(commit.tree_id().to_string(), result.tree);
    assert_eq!(commit.message_bytes(), b"Publish composition\n");
    assert_eq!(commit.author().name(), Ok("GWZ Publisher"));
    let tree = commit.tree().unwrap();
    for (path, expected) in [
        ("gwz.conf/gwz.lock", b"new lock\n".as_slice()),
        ("gwz.conf/merge/marker.yml", b"marker\n".as_slice()),
        ("tracked.txt", b"keep\n".as_slice()),
    ] {
        let entry = tree.get_path(Path::new(path)).unwrap();
        assert_eq!(repo.find_blob(entry.id()).unwrap().content(), expected);
    }
    assert_eq!(
        result.candidate_hashes,
        ["gwz.conf/gwz.lock", "gwz.conf/merge/marker.yml"]
            .into_iter()
            .zip([b"new lock\n".as_slice(), b"marker\n".as_slice()])
            .map(|(path, bytes)| GitCandidateHash {
                path: path.into(),
                sha256: format!("{:x}", Sha256::digest(bytes)),
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
    for (path, expected) in [
        ("staged.txt", "user staged\n"),
        ("dirty.txt", "user dirty\n"),
        ("untracked.txt", "untracked\n"),
        ("gwz.conf/gwz.lock", "old lock\n"),
        ("gwz.conf/local-only", "local\n"),
        ("gwz.conf/user-note", "tracked note\n"),
    ] {
        assert_text_eq(root.join(path), expected);
    }
    assert_eq!(
        backend
            .commit_gwz_paths_checked(
                &root,
                Some(&parent),
                &[candidate("gwz.conf/gwz.lock", "new lock\n")],
                "repeat",
            )
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_eq!(rev_parse(&root, "HEAD"), result.commit);
}

#[test]
fn scoped_commit_supports_unborn_root_without_creating_a_real_index() {
    let temp = TempDir::new("scoped-unborn");
    let root = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&root).unwrap();
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "GWZ").unwrap();
    config.set_str("user.email", "gwz@example.invalid").unwrap();
    drop((config, repo));
    let result = backend
        .commit_gwz_paths_checked(
            &root,
            None,
            &[
                candidate("gwz.conf/workspace.yml", "workspace\n"),
                candidate("gwz.conf/gwz.lock", "lock\n"),
            ],
            "initial",
        )
        .unwrap();
    let repo = git2::Repository::open(&root).unwrap();
    let commit = repo.find_commit(result.commit.parse().unwrap()).unwrap();
    assert_eq!(commit.parent_count(), 0);
    assert_eq!(commit.tree().unwrap().len(), 1);
    assert!(!root.join(".git/index").exists());
    assert_eq!(
        backend
            .commit_gwz_paths_checked(
                &root,
                None,
                &[candidate("gwz.conf/gwz.lock", "lock\n")],
                "repeat",
            )
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
}

#[test]
fn scoped_commit_rejects_invalid_duplicate_and_detached_requests() {
    let temp = TempDir::new("scoped-invalid");
    let root = temp.path().join("repo");
    let parent = seed(&root, &[("tracked", "base")]);
    let backend = Git2Backend::new();
    for path in [
        "",
        "gwz.conf",
        "gwz.conf/",
        "/gwz.conf/file",
        "outside/file",
        "gwz.conf/../file",
        "gwz.conf/./file",
        "gwz.conf//file",
        "gwz.conf/.git/config",
        r"gwz.conf\file",
    ] {
        assert_eq!(
            backend
                .commit_gwz_paths_checked(&root, Some(&parent), &[candidate(path, "x")], "invalid")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest,
            "{path:?}"
        );
    }
    for files in [
        vec![],
        vec![
            candidate("gwz.conf/same", "one"),
            candidate("gwz.conf/same", "two"),
        ],
    ] {
        assert_eq!(
            backend
                .commit_gwz_paths_checked(&root, Some(&parent), &files, "invalid")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    run_git(&root, &["checkout", "--detach"]);
    assert_eq!(
        backend
            .commit_gwz_paths_checked(
                &root,
                Some(&parent),
                &[candidate("gwz.conf/valid", "x")],
                "detached",
            )
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_eq!(rev_parse(&root, "HEAD"), parent);
}

#[test]
fn scoped_commit_fails_closed_on_concurrent_ref_movement() {
    let temp = TempDir::new("scoped-race");
    let root = temp.path().join("repo");
    let base = seed(&root, &[("base", "base")]);
    fs::write(root.join("moved"), "moved").unwrap();
    run_git(&root, &["add", "moved"]);
    run_git(&root, &["commit", "-m", "moved"]);
    let moved = rev_parse(&root, "HEAD");
    run_git(&root, &["reset", "--hard", &base]);
    let index_before = fs::read(root.join(".git/index")).unwrap();
    let (callback_root, callback_moved) = (root.clone(), moved.clone());
    Git2Backend::before_next_scoped_commit_ref_lock(move || {
        git2::Repository::open(callback_root)
            .unwrap()
            .reference(
                "refs/heads/main",
                callback_moved.parse().unwrap(),
                true,
                "test race",
            )
            .unwrap();
    });
    let error = Git2Backend::new()
        .commit_gwz_paths_checked(
            &root,
            Some(&base),
            &[candidate("gwz.conf/gwz.lock", "candidate")],
            "raced",
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(rev_parse(&root, "HEAD"), moved);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
    assert!(!root.join("gwz.conf/gwz.lock").exists());
}

#[test]
fn scoped_evidence_rollback_preserves_unrelated_index_and_worktree_state() {
    let temp = TempDir::new("scoped-rollback-born");
    let root = temp.path().join("repo");
    let parent = seed(
        &root,
        &[
            ("gwz.conf/gwz.lock", "baseline\n"),
            ("staged.txt", "base\n"),
            ("dirty.txt", "base\n"),
        ],
    );
    fs::write(root.join("staged.txt"), "staged user work\n").unwrap();
    stage_path(&root, "staged.txt").unwrap();
    fs::write(root.join("dirty.txt"), "dirty user work\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked user work\n").unwrap();
    let index_before = fs::read(root.join(".git/index")).unwrap();
    let files = vec![
        candidate("gwz.conf/gwz.lock", "candidate\n"),
        candidate("gwz.conf/merge/marker.yaml", "marker\n"),
    ];
    let backend = Git2Backend::new();
    let evidence = backend
        .commit_gwz_paths_checked(&root, Some(&parent), &files, "evidence")
        .unwrap();

    backend
        .rollback_gwz_paths_commit_checked(
            &root,
            "main",
            &evidence.commit,
            Some(&parent),
            &files,
            "evidence",
        )
        .unwrap();

    assert_eq!(
        backend.head(&root).unwrap().commit.as_deref(),
        Some(parent.as_str())
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
    for (path, expected) in [
        ("staged.txt", "staged user work\n"),
        ("dirty.txt", "dirty user work\n"),
        ("untracked.txt", "untracked user work\n"),
        ("gwz.conf/gwz.lock", "baseline\n"),
    ] {
        assert_text_eq(root.join(path), expected);
    }
}

#[test]
fn scoped_evidence_rollback_restores_an_unborn_attached_branch() {
    let temp = TempDir::new("scoped-rollback-unborn");
    let root = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&root).unwrap();
    let repo = git2::Repository::open(&root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "GWZ").unwrap();
    config.set_str("user.email", "gwz@example.invalid").unwrap();
    drop((config, repo));
    fs::write(root.join("unrelated.txt"), "preserve\n").unwrap();
    let files = vec![candidate("gwz.conf/gwz.lock", "candidate\n")];
    let evidence = backend
        .commit_gwz_paths_checked(&root, None, &files, "initial evidence")
        .unwrap();

    backend
        .rollback_gwz_paths_commit_checked(
            &root,
            "main",
            &evidence.commit,
            None,
            &files,
            "initial evidence",
        )
        .unwrap();

    let head = backend.head(&root).unwrap();
    assert_eq!(head.branch.as_deref(), Some("main"));
    assert!(head.commit.is_none());
    assert!(!head.is_detached);
    assert_text_eq(root.join("unrelated.txt"), "preserve\n");
    assert!(!root.join(".git/index").exists());
}
