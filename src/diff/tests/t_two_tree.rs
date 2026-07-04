//! Two-tree and merge-base (`A...B`) comparison parity, plus type-change and the
//! `find_copies` rejection.

use super::*;
use crate::diff::{
    ComparisonSpec, RepoDiffComparisonKind, RepoDiffOptions, RepoDiffStatus,
    reject_unsupported_options,
};
use crate::model::ErrorCode;
use crate::protocol::generated::DiffAlgorithm;

#[test]
fn two_tree_range_matches_git() {
    let temp = TempDir::new("two-tree");
    let root = temp.path();
    init_repo(root);
    write_file(root, "a.txt", "a1\n");
    commit_all(root, "c1");
    write_file(root, "a.txt", "a2\n");
    write_file(root, "b.txt", "b\n");
    commit_all(root, "c2");
    std::fs::remove_file(root.join("a.txt")).unwrap();
    write_file(root, "c.txt", "c\n");
    commit_all(root, "c3");

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~2".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["HEAD~2", "HEAD"]),
        "two-tree diff must match git diff HEAD~2 HEAD"
    );
}

#[test]
fn merge_base_range_matches_git_three_dot() {
    let temp = TempDir::new("merge-base");
    let root = temp.path();
    init_repo(root);
    write_file(root, "base.txt", "base\n");
    commit_all(root, "base");

    // topic branch diverges from main.
    run_git(root, &["checkout", "-b", "topic"]);
    write_file(root, "topic.txt", "topic\n");
    commit_all(root, "topic work");

    run_git(root, &["checkout", "main"]);
    write_file(root, "main.txt", "main\n");
    commit_all(root, "main work");

    // git diff main...topic == diff(merge-base(main,topic), topic). The merge
    // base is the shared "base" commit, so only topic.txt shows.
    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("main".to_owned()),
        right: Some("topic".to_owned()),
        merge_base: true,
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["main...topic"]),
        "A...B must match git's three-dot diff"
    );
    // Sanity: only the topic-side addition, not main.txt.
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].primary_path(), Some("topic.txt"));
    assert_eq!(manifest.entries[0].status, RepoDiffStatus::Added);
}

#[cfg(unix)]
#[test]
fn type_change_is_reported() {
    let temp = TempDir::new("typechange");
    let root = temp.path();
    init_repo(root);
    write_file(root, "thing", "regular file\n");
    commit_all(root, "seed regular");

    // Replace the regular file with a symlink of the same path.
    std::fs::remove_file(root.join("thing")).unwrap();
    std::os::unix::fs::symlink("target", root.join("thing")).unwrap();
    commit_all(root, "now a symlink");

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~1".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["HEAD~1", "HEAD"]),
    );
    let entry = manifest
        .entries
        .iter()
        .find(|e| e.primary_path() == Some("thing"))
        .unwrap();
    assert_eq!(entry.status, RepoDiffStatus::TypeChanged);
    assert_eq!(entry.old_mode, Some(0o100_644));
    assert_eq!(entry.new_mode, Some(0o120_000));
}

#[test]
fn pathspec_narrows_the_manifest() {
    let temp = TempDir::new("pathspec");
    let root = temp.path();
    init_repo(root);
    write_file(root, "src/a.rs", "a\n");
    write_file(root, "docs/b.md", "b\n");
    commit_all(root, "seed");
    write_file(root, "src/a.rs", "a changed\n");
    write_file(root, "docs/b.md", "b changed\n");

    let options = RepoDiffOptions {
        pathspecs: vec!["src".to_owned()],
        ..RepoDiffOptions::full_repo()
    };
    let manifest = diff_worktree(root, options);

    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].primary_path(), Some("src/a.rs"));
    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["--", "src"]),
    );
}

#[test]
fn find_copies_is_rejected() {
    let err = reject_unsupported_options(Some(true), None)
        .expect_err("find_copies=true must be rejected in v0");
    assert_eq!(err.code, ErrorCode::UnsupportedOperation);
    assert!(
        err.message.contains("find_copies"),
        "rejection must name the offending option: {}",
        err.message
    );

    // A supported algorithm is accepted.
    assert!(reject_unsupported_options(None, Some(DiffAlgorithm::Patience)).is_ok());
    assert!(reject_unsupported_options(Some(false), None).is_ok());
}
