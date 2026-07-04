//! Rename-detection parity: with `find_renames`, a moved file must surface as a
//! single `Renamed` entry carrying both paths and a similarity, agreeing with
//! `git diff --name-status -M`. Without it, the same change degrades to
//! add+delete (matching plain `git diff --no-renames`).

use super::*;
use crate::diff::{ComparisonSpec, RepoDiffComparisonKind, RepoDiffOptions, RepoDiffStatus};

fn rename_repo() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new("rename");
    let root = temp.path().to_path_buf();
    init_repo(&root);
    // A sizeable file so a near-identical move is detected as a rename.
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    write_file(&root, "old_name.txt", &body);
    commit_all(&root, "seed");

    // Move it and tweak one line so similarity < 100 but well above threshold.
    let mut moved = body.clone();
    moved = moved.replace("line 3\n", "line 3 changed\n");
    std::fs::remove_file(root.join("old_name.txt")).unwrap();
    write_file(&root, "new_name.txt", &moved);
    commit_all(&root, "rename");
    (temp, root)
}

#[test]
fn rename_with_detection_matches_git() {
    let (_temp, root) = rename_repo();

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~1".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    let options = RepoDiffOptions {
        find_renames: true,
        ..RepoDiffOptions::full_repo()
    };
    let manifest = diff_spec(&root, &spec, options);

    assert_eq!(manifest.entries.len(), 1, "one rename entry expected");
    let entry = &manifest.entries[0];
    assert_eq!(entry.status, RepoDiffStatus::Renamed);
    assert_eq!(entry.old_path.as_deref(), Some("old_name.txt"));
    assert_eq!(entry.new_path.as_deref(), Some("new_name.txt"));
    let sim = entry.similarity.expect("rename must carry similarity");
    // libgit2 and Git use slightly different similarity metrics, so allow a
    // small tolerance rather than an exact match.
    let git_sim = git_rename_similarity(&root, &["HEAD~1", "HEAD"]);
    assert!(
        sim.abs_diff(git_sim) <= 5,
        "our similarity {sim} within 5 of git's {git_sim}"
    );

    // Detection + pairing must match git's -M name-status (metric-agnostic).
    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(&root, &["HEAD~1", "HEAD"]),
    );
}

#[test]
fn rename_without_detection_is_add_plus_delete() {
    let (_temp, root) = rename_repo();

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~1".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    // find_renames = false (default).
    let manifest = diff_spec(&root, &spec, RepoDiffOptions::full_repo());

    let mut statuses: Vec<RepoDiffStatus> = manifest.entries.iter().map(|e| e.status).collect();
    statuses.sort_by_key(|s| s.status_char());
    assert_eq!(
        statuses,
        vec![RepoDiffStatus::Added, RepoDiffStatus::Deleted],
        "without rename detection the move is add+delete"
    );
    assert!(manifest.entries.iter().all(|e| e.similarity.is_none()));
}
