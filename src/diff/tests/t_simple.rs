//! Modify / add / delete parity, plus per-file line-stat (`--numstat`) parity.

use std::collections::BTreeMap;

use super::*;
use crate::diff::{RepoDiffOptions, RepoDiffStatus};

#[test]
fn modify_add_delete_match_git_name_status() {
    let temp = TempDir::new("simple");
    let root = temp.path();
    init_repo(root);

    write_file(root, "keep.txt", "one\ntwo\nthree\n");
    write_file(root, "gone.txt", "delete me\n");
    commit_all(root, "seed");

    // Modify + delete tracked files in the worktree, and drop an untracked file
    // on disk. Plain `git diff` (worktree-vs-index) shows the modify and delete
    // but NOT the untracked add — the manifest must match that exactly.
    write_file(root, "keep.txt", "one\nTWO\nthree\nfour\n");
    write_file(root, "untracked.txt", "brand new\n");
    std::fs::remove_file(root.join("gone.txt")).unwrap();

    let manifest = diff_worktree(root, RepoDiffOptions::full_repo());

    assert_eq!(name_status_lines(&manifest), git_name_status(root, &[]));

    // Spot-check the internal status mapping directly.
    let by_path: BTreeMap<&str, RepoDiffStatus> = manifest
        .entries
        .iter()
        .map(|e| (e.primary_path().unwrap(), e.status))
        .collect();
    assert_eq!(by_path["keep.txt"], RepoDiffStatus::Modified);
    assert_eq!(by_path["gone.txt"], RepoDiffStatus::Deleted);
    // Untracked files are not part of `git diff`.
    assert!(
        !by_path.contains_key("untracked.txt"),
        "untracked file must not appear in worktree-vs-index diff"
    );
}

#[test]
fn per_file_line_stats_match_git_numstat() {
    let temp = TempDir::new("numstat");
    let root = temp.path();
    init_repo(root);

    write_file(root, "a.txt", "l1\nl2\nl3\n");
    write_file(root, "b.txt", "keep\n");
    commit_all(root, "seed");

    write_file(root, "a.txt", "l1\nCHANGED\nl3\nl4\nl5\n");
    write_file(root, "c.txt", "added1\nadded2\n");
    commit_all(root, "changes");

    // Two-tree HEAD~1..HEAD so stats are deterministic.
    let spec = crate::diff::ComparisonSpec {
        kind: crate::diff::RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~1".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    // Build our (path -> (adds, dels)) map.
    let mut ours: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for entry in &manifest.entries {
        ours.insert(
            entry.primary_path().unwrap().to_owned(),
            (entry.insertions.unwrap(), entry.deletions.unwrap()),
        );
    }

    // Parse `git diff --numstat HEAD~1 HEAD`: "adds\tdels\tpath".
    let raw = git_stdout(root, &["diff", "--numstat", "HEAD~1", "HEAD"]);
    let mut theirs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for line in raw.lines() {
        let mut cols = line.split('\t');
        let adds: usize = cols.next().unwrap().parse().unwrap();
        let dels: usize = cols.next().unwrap().parse().unwrap();
        let path = cols.next().unwrap().to_owned();
        theirs.insert(path, (adds, dels));
    }

    assert_eq!(ours, theirs, "per-file numstat must match git");

    // Aggregate totals also agree with git's summed numstat.
    let (total_adds, total_dels): (usize, usize) = theirs
        .values()
        .fold((0, 0), |(a, d), (na, nd)| (a + na, d + nd));
    assert_eq!(manifest.insertions, total_adds);
    assert_eq!(manifest.deletions, total_dels);
}

#[test]
fn clean_worktree_reports_no_differences() {
    let temp = TempDir::new("clean");
    let root = temp.path();
    init_repo(root);
    write_file(root, "x.txt", "stable\n");
    commit_all(root, "seed");

    let manifest = diff_worktree(root, RepoDiffOptions::full_repo());
    assert!(!manifest.has_differences());
    assert_eq!(manifest.files_changed(), 0);
    assert!(git_name_status(root, &[]).is_empty());
}
