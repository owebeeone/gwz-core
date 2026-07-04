//! `--cached` (index-vs-tree) parity, including the unborn-HEAD empty-tree case,
//! and the `<commit>` worktree-vs-tree form.

use super::*;
use crate::diff::{ComparisonSpec, RepoDiffComparisonKind, RepoDiffOptions, RepoDiffStatus};

#[test]
fn cached_matches_git_diff_cached() {
    let temp = TempDir::new("cached");
    let root = temp.path();
    init_repo(root);
    write_file(root, "tracked.txt", "v1\n");
    commit_all(root, "seed");

    // Stage a modification and a new file, leave one unstaged change on disk.
    write_file(root, "tracked.txt", "v2\n");
    write_file(root, "staged_new.txt", "new\n");
    run_git(root, &["add", "tracked.txt", "staged_new.txt"]);
    write_file(root, "unstaged.txt", "not staged\n");

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::IndexVsTree,
        ..Default::default()
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["--cached"]),
        "--cached manifest must match git diff --cached"
    );
    // The purely-unstaged file is NOT in the cached diff.
    assert!(
        manifest
            .entries
            .iter()
            .all(|e| e.primary_path() != Some("unstaged.txt")),
        "untracked/unstaged file must not appear in --cached"
    );
}

#[test]
fn cached_on_unborn_head_uses_empty_tree() {
    let temp = TempDir::new("cached-unborn");
    let root = temp.path();
    init_repo(root);

    // No commits yet: stage two files, then --cached should show both as added
    // against the empty tree.
    write_file(root, "a.txt", "aaa\n");
    write_file(root, "b.txt", "bbb\n");
    run_git(root, &["add", "a.txt", "b.txt"]);

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::IndexVsTree,
        ..Default::default()
    };
    // Resolution must succeed against an unborn HEAD and pick the empty tree.
    let backend = Git2Backend::new();
    let comparison = backend.resolve_comparison(root, &spec).unwrap();
    assert!(
        comparison.old_tree.is_none(),
        "unborn HEAD --cached old side must be the empty tree (None)"
    );

    let manifest = backend
        .diff_manifest(root, &comparison, &RepoDiffOptions::full_repo())
        .unwrap();

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["--cached"]),
        "unborn --cached must match git (both files added)"
    );
    assert!(
        manifest
            .entries
            .iter()
            .all(|e| e.status == RepoDiffStatus::Added)
    );
    assert_eq!(manifest.entries.len(), 2);
}

#[test]
fn commit_form_matches_git_diff_head() {
    let temp = TempDir::new("head");
    let root = temp.path();
    init_repo(root);
    write_file(root, "f.txt", "committed\n");
    commit_all(root, "seed");

    // Stage a delete AND leave a fresh worktree modification: diff <commit>
    // blends index + worktree (diff_tree_to_workdir_with_index).
    write_file(root, "f.txt", "committed then edited\n");
    write_file(root, "g.txt", "new tracked\n");
    run_git(root, &["add", "g.txt"]);

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::WorktreeVsTree,
        left: Some("HEAD".to_owned()),
        ..Default::default()
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["HEAD"]),
        "diff <commit> must match git diff HEAD"
    );
}

#[test]
fn commit_form_on_unborn_repo_is_typed_error() {
    let temp = TempDir::new("unborn-commit");
    let root = temp.path();
    init_repo(root);
    write_file(root, "a.txt", "a\n");
    run_git(root, &["add", "a.txt"]);

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::WorktreeVsTree,
        left: None, // defaults to HEAD, which is unborn
        ..Default::default()
    };
    let err = Git2Backend::new()
        .resolve_comparison(root, &spec)
        .expect_err("diff <commit> against an unborn repo must error");
    assert_eq!(err.code, crate::model::ErrorCode::GitCommandFailed);
}
