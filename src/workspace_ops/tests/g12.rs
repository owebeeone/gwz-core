use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::artifact::{LockArtifact, ResolvedMemberArtifact};

use super::*;

// The workspace boundary is a managed block in .git/info/exclude: members + the tmp dir,
// regenerated from the lock on every run, preserving any user lines. Members are hidden,
// not tracked (no gitlinks, no .gitignore).

fn lock_with_members(paths: &[&str]) -> LockArtifact {
    let mut members = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        members.insert(
            format!("mem_{i}"),
            ResolvedMemberArtifact {
                path: (*path).to_owned(),
                ..Default::default()
            },
        );
    }
    LockArtifact {
        schema: "gwz.lock/v0".to_owned(),
        workspace_id: "ws_test".to_owned(),
        manifest_schema: "gwz.workspace/v0".to_owned(),
        members,
    }
}

fn read_exclude(root: &Path) -> String {
    fs::read_to_string(root.join(".git/info/exclude")).unwrap()
}

#[test]
fn exclude_lists_tmp_and_member_paths() {
    let temp = TempDir::new("exclude-members");
    ensure_workspace_exclude(temp.path(), &lock_with_members(&["gwz-cli", "gwz-core"])).unwrap();
    let exclude = read_exclude(temp.path());
    assert!(exclude.contains("/gwz.conf/.tmp/"));
    assert!(exclude.contains("/gwz-cli/"));
    assert!(exclude.contains("/gwz-core/"));
    assert!(exclude.contains("# BEGIN GWZ managed member repositories"));
}

#[test]
fn exclude_is_idempotent() {
    let temp = TempDir::new("exclude-idem");
    let lock = lock_with_members(&["gwz-cli"]);
    ensure_workspace_exclude(temp.path(), &lock).unwrap();
    let once = read_exclude(temp.path());
    ensure_workspace_exclude(temp.path(), &lock).unwrap();
    assert_eq!(
        once,
        read_exclude(temp.path()),
        "second run must not change the file"
    );
}

#[test]
fn exclude_preserves_user_lines_and_reconciles_members() {
    let temp = TempDir::new("exclude-reconcile");
    let info = temp.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("exclude"), "# user line\n/scratch/\n").unwrap();

    ensure_workspace_exclude(temp.path(), &lock_with_members(&["a", "b"])).unwrap();
    let after = read_exclude(temp.path());
    assert!(after.contains("/scratch/"), "user lines preserved");
    assert!(after.contains("/a/") && after.contains("/b/"));

    // Drop a member → its entry is reconciled away; user lines stay.
    ensure_workspace_exclude(temp.path(), &lock_with_members(&["a"])).unwrap();
    let after = read_exclude(temp.path());
    assert!(after.contains("/a/"));
    assert!(
        !after.contains("/b/"),
        "removed member dropped from the block"
    );
    assert!(after.contains("/scratch/"), "user lines still preserved");
}
