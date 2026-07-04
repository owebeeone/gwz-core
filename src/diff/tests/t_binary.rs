//! Binary-file parity: a changed binary blob must be flagged `is_binary` with no
//! textual line stats, and the entry must agree with `git diff --numstat` (which
//! prints `-\t-` for binary files) and Git's `Binary files ... differ` marker.

use super::*;
use crate::diff::{ComparisonSpec, RepoDiffComparisonKind, RepoDiffOptions};

#[test]
fn binary_change_is_flagged_without_line_stats() {
    let temp = TempDir::new("binary");
    let root = temp.path();
    init_repo(root);

    // A blob with NUL bytes is treated as binary by Git.
    write_bytes(root, "blob.bin", &[0u8, 1, 2, 3, 0, 255, 10, 0, 42]);
    commit_all(root, "seed binary");

    write_bytes(root, "blob.bin", &[0u8, 9, 8, 7, 0, 1, 2, 3, 0, 99, 100]);
    commit_all(root, "change binary");

    let spec = ComparisonSpec {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some("HEAD~1".to_owned()),
        right: Some("HEAD".to_owned()),
        merge_base: false,
    };
    let manifest = diff_spec(root, &spec, RepoDiffOptions::full_repo());

    assert_eq!(manifest.entries.len(), 1);
    let entry = &manifest.entries[0];
    assert!(entry.is_binary, "binary blob change must set is_binary");
    assert_eq!(
        entry.insertions, None,
        "binary entry has no line insertions"
    );
    assert_eq!(entry.deletions, None, "binary entry has no line deletions");
    assert_eq!(manifest.insertions, 0);
    assert_eq!(manifest.deletions, 0);

    // git --numstat prints "-\t-\tpath" for binary files.
    let numstat = git_stdout(root, &["diff", "--numstat", "HEAD~1", "HEAD"]);
    assert!(
        numstat.starts_with("-\t-\t"),
        "git numstat marks the file binary: {numstat}"
    );

    // And the status matches git's name-status.
    assert_eq!(
        name_status_lines(&manifest),
        git_name_status(root, &["HEAD~1", "HEAD"]),
    );
}
