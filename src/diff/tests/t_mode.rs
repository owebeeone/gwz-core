//! Executable mode-change parity: a content-preserving chmod +x must appear as a
//! modified entry with old/new modes matching Git.

#[cfg(unix)]
use super::*;

#[cfg(unix)]
#[test]
fn executable_mode_change_matches_git() {
    use crate::diff::{RepoDiffOptions, RepoDiffStatus};
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new("mode");
    let root = temp.path();
    init_repo(root);
    write_file(root, "script.sh", "#!/bin/sh\necho hi\n");
    commit_all(root, "seed non-exec");

    // chmod +x without touching content.
    let path = root.join("script.sh");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();

    let manifest = diff_worktree(root, RepoDiffOptions::full_repo());

    // Git reports a mode change as a Modified (M) entry.
    assert_eq!(name_status_lines(&manifest), git_name_status(root, &[]));

    let entry = manifest
        .entries
        .iter()
        .find(|e| e.primary_path() == Some("script.sh"))
        .expect("script.sh must be in the diff");
    assert_eq!(entry.status, RepoDiffStatus::Modified);
    assert_eq!(entry.old_mode, Some(0o100_644));
    assert_eq!(entry.new_mode, Some(0o100_755));

    // Cross-check the modes against git's raw diff header.
    let raw = git_stdout(root, &["diff"]);
    assert!(
        raw.contains("old mode 100644") && raw.contains("new mode 100755"),
        "git diff must report the 644->755 mode change: {raw}"
    );
}
