use super::transition::TEMP_SEQUENCE;
use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[path = "tests/durability.rs"]
mod durability;
#[path = "tests/exact_source.rs"]
mod exact_source;
#[path = "tests/leaf_publication.rs"]
mod leaf_publication;
#[path = "tests/recovery_protocol.rs"]
mod recovery_protocol;
#[path = "tests/removal_recovery.rs"]
mod removal_recovery;
#[path = "tests/staging_recovery.rs"]
mod staging_recovery;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gwz-checked-artifact-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        git2::Repository::init(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn artifact(root: &Path, relative: &str) -> CheckedArtifact {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        Path::new(relative),
        ErrorCode::MergeRecoveryRequired,
        "test artifact",
    )
    .unwrap()
}

#[test]
fn exact_missing_and_existing_replacements_are_durable_and_observable() {
    let root = TempRoot::new("replace");
    fs::create_dir_all(root.0.join("a/b")).unwrap();
    let checked = artifact(&root.0, "a/b/value");
    assert_eq!(checked.observe().unwrap(), CheckedArtifactFact::Missing);
    checked
        .replace_exact(&CheckedArtifactFact::Missing, b"first")
        .unwrap();
    assert_eq!(
        checked.observe().unwrap(),
        CheckedArtifactFact::Bytes(b"first".to_vec())
    );
    checked
        .replace_exact(&CheckedArtifactFact::Bytes(b"first".to_vec()), b"second")
        .unwrap();
    assert_eq!(fs::read(root.0.join("a/b/value")).unwrap(), b"second");
}

#[test]
fn foreign_leaf_inserted_before_final_check_is_not_overwritten() {
    let root = TempRoot::new("foreign-insert");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let checked = artifact(&root.0, "a/value");
    let destination = root.0.join("a/value");
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck, move || {
        fs::write(destination, b"foreign").unwrap();
    });
    let error = checked
        .replace_exact(&CheckedArtifactFact::Missing, b"gwz")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"foreign");
}

#[test]
fn foreign_leaf_replaced_before_remove_is_not_deleted() {
    let root = TempRoot::new("foreign-remove");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let destination = root.0.join("a/value");
    fs::write(&destination, b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    let replacement = destination.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck, move || {
        fs::remove_file(&replacement).unwrap();
        fs::write(replacement, b"foreign").unwrap();
    });
    let error = checked
        .remove_exact(&CheckedArtifactFact::Bytes(b"owned".to_vec()))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(destination).unwrap(), b"foreign");
}

#[test]
fn exact_removal_is_durable_and_classifies_after_mutation() {
    let root = TempRoot::new("remove");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterMutation);
    assert!(
        checked
            .remove_exact(&CheckedArtifactFact::Bytes(b"owned".to_vec()))
            .is_err()
    );
    assert_eq!(checked.observe().unwrap(), CheckedArtifactFact::Missing);
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn replaced_parent_invalidates_the_retained_capability() {
    let root = TempRoot::new("parent-replaced");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap();
    fs::create_dir(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"foreign").unwrap();
    assert_eq!(checked.observe().unwrap(), CheckedArtifactFact::Invalid);
    assert!(
        checked
            .replace_exact(
                &CheckedArtifactFact::Bytes(b"owned".to_vec()),
                b"replacement"
            )
            .is_err()
    );
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"foreign");
}

// Positive Windows counterpart of the gated substitution injections:
// production retains the parent through a plain cap-std directory open,
// which holds no DELETE sharing on Windows, so the OS itself denies
// displacing the retained directory with a sharing violation.
#[cfg(windows)]
#[test]
fn retained_directory_blocks_substitution_rename_windows() {
    let root = TempRoot::new("retained-parent-pins-name");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    let error = fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32), "{error:?}");
    assert_eq!(
        checked.observe().unwrap(),
        CheckedArtifactFact::Bytes(b"owned".to_vec())
    );
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn symlink_parent_is_invalid_and_cannot_escape_the_root() {
    use std::os::unix::fs::symlink;

    let root = TempRoot::new("symlink-parent");
    let outside = TempRoot::new("outside");
    fs::create_dir_all(root.0.join("a")).unwrap();
    symlink(&outside.0, root.0.join("a/b")).unwrap();
    let checked = artifact(&root.0, "a/b/value");
    assert_eq!(checked.observe().unwrap(), CheckedArtifactFact::Invalid);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Missing, b"outside")
            .is_err()
    );
    assert!(!outside.0.join("value").exists());
}

#[test]
fn injected_faults_classify_on_both_sides_of_mutation() {
    let root = TempRoot::new("faults");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .is_err()
    );
    assert_eq!(checked.observe().unwrap(), CheckedArtifactFact::Missing);

    fail_next_checked_artifact_at(CheckedArtifactFault::AfterMutation);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .is_err()
    );
    assert_eq!(
        checked.observe().unwrap(),
        CheckedArtifactFact::Bytes(b"goal".to_vec())
    );
}
