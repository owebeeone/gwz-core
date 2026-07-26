use std::path::Path;

use super::*;

fn index_entry(path: &[u8], stage: u16) -> git2::IndexEntry {
    git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: git2::Oid::ZERO_SHA1,
        flags: stage << 12,
        flags_extended: 0,
        path: path.to_vec(),
    }
}

fn conflict_index(paths: [Option<&[u8]>; 3]) -> git2::Index {
    let mut index = git2::Index::new().unwrap();
    for (stage, path) in paths.into_iter().enumerate() {
        if let Some(path) = path {
            index.add(&index_entry(path, (stage + 1) as u16)).unwrap();
        }
    }
    assert!(index.has_conflicts());
    index
}

fn commit_raw_path(
    repo: &git2::Repository,
    parent: Option<&git2::Commit<'_>>,
    path: &[u8],
    contents: &[u8],
    message: &str,
) -> git2::Oid {
    let blob = repo.blob(contents).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert(path, blob, 0o100644).unwrap();
    let tree_oid = builder.write().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let signature =
        git2::Signature::new("gwz", "gwz@example.invalid", &git2::Time::new(1, 0)).unwrap();
    let parents = parent.into_iter().collect::<Vec<_>>();
    repo.commit(None, &signature, &signature, message, &tree, &parents)
        .unwrap()
}

fn commit_tree(
    repo: &git2::Repository,
    parent: &git2::Commit<'_>,
    entries: &[(&[u8], &[u8])],
    message: &str,
) -> git2::Oid {
    let mut builder = repo.treebuilder(None).unwrap();
    for (path, contents) in entries {
        let blob = repo.blob(contents).unwrap();
        builder.insert(*path, blob, 0o100644).unwrap();
    }
    let tree_oid = builder.write().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let signature =
        git2::Signature::new("gwz", "gwz@example.invalid", &git2::Time::new(2, 0)).unwrap();
    repo.commit(None, &signature, &signature, message, &tree, &[parent])
        .unwrap()
}

fn seed_raw_path_conflict(path: &Path) -> (String, String) {
    let repo = git2::Repository::init(path).unwrap();
    let raw_path = b"config-\xFF.toml";
    let base_oid = commit_raw_path(&repo, None, raw_path, b"base\n", "base");
    let base = repo.find_commit(base_oid).unwrap();
    let target = commit_tree(&repo, &base, &[(raw_path, b"target\n")], "target");
    let source = commit_tree(&repo, &base, &[(raw_path, b"source\n")], "source");
    (target.to_string(), source.to_string())
}

fn seed_rename_rename_conflict(path: &Path) -> (String, String) {
    let repo = git2::Repository::init(path).unwrap();
    let base_oid = commit_raw_path(&repo, None, b"old.txt", b"contents\n", "base");
    let base = repo.find_commit(base_oid).unwrap();
    let target = commit_tree(&repo, &base, &[(b"ours.txt", b"contents\n")], "ours");
    let source = commit_tree(&repo, &base, &[(b"theirs.txt", b"contents\n")], "theirs");
    drop(base);
    repo.reference("refs/heads/main", target, true, "test target")
        .unwrap();
    repo.reference("refs/heads/feature", source, true, "test source")
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout)).unwrap();
    (target.to_string(), source.to_string())
}

#[test]
fn git_path_rendering_is_byte_safe_and_platform_independent() {
    let rows: &[(&[u8], &str)] = &[
        (b"src/lib.rs", "src/lib.rs"),
        ("café.txt".as_bytes(), "café.txt"),
        (b"quote\"and\\slash", "\"quote\\\"and\\\\slash\""),
        (b"line\nfeed", "\"line\\nfeed\""),
        (b"config-\xFF.toml", "\"config-\\xFF.toml\""),
        (b"bad-\xF0\x9F-tail", "\"bad-\\xF0\\x9F-tail\""),
    ];
    for (raw, expected) in rows {
        assert_eq!(render_git_path(raw), *expected);
    }
}

#[test]
fn conflict_paths_include_every_stage_then_sort_and_deduplicate() {
    let index = conflict_index([Some(b"old.txt"), Some(b"ours.txt"), Some(b"theirs.txt")]);
    assert_eq!(
        conflict_paths(&index).unwrap(),
        ["old.txt", "ours.txt", "theirs.txt"]
    );

    let duplicate = conflict_index([Some(b"same.txt"), Some(b"same.txt"), Some(b"same.txt")]);
    assert_eq!(conflict_paths(&duplicate).unwrap(), ["same.txt"]);
}

#[test]
fn invalid_utf8_conflict_path_is_never_dropped() {
    let index = conflict_index([None, Some(b"config-\xFF.toml"), None]);
    assert_eq!(conflict_paths(&index).unwrap(), ["\"config-\\xFF.toml\""]);
}

#[test]
fn merge_simulation_classifies_raw_byte_conflict_from_the_index_truth() {
    let temp = TempDir::new("merge-raw-byte-conflict");
    let (target, source) = seed_raw_path_conflict(temp.path());
    let backend = Git2Backend::new();

    assert_eq!(
        backend
            .merge_simulate(temp.path(), &target, &source)
            .unwrap(),
        GitMergeSimulation::Conflicts(vec!["\"config-\\xFF.toml\"".to_owned()])
    );
    assert_eq!(
        git2::Repository::open(temp.path()).unwrap().state(),
        git2::RepositoryState::Clean
    );
}

#[test]
fn simulated_and_native_merge_report_identical_rename_conflict_paths() {
    let temp = TempDir::new("merge-rename-stage-paths");
    let (target, source) = seed_rename_rename_conflict(temp.path());
    let backend = Git2Backend::new();
    let expected = vec![
        "old.txt".to_owned(),
        "ours.txt".to_owned(),
        "theirs.txt".to_owned(),
    ];

    assert_eq!(
        backend
            .merge_simulate(temp.path(), &target, &source)
            .unwrap(),
        GitMergeSimulation::Conflicts(expected.clone())
    );
    let native = backend
        .merge_upstream(temp.path(), "main", "refs/heads/feature")
        .unwrap();
    assert_eq!(native.conflicts, expected);
    assert_eq!(
        backend
            .merge_state(temp.path())
            .unwrap()
            .unwrap()
            .conflict_paths,
        native.conflicts
    );
}
