use super::*;

#[test]
fn different_byte_replacement_after_final_proof_is_restored_not_overwritten() {
    let root = TempRoot::new("post-proof-different");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let path = root.0.join("a/value");
    fs::write(&path, b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    let replacement = path.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::AfterFinalProof, move || {
        fs::remove_file(&replacement).unwrap();
        fs::write(&replacement, b"foreign").unwrap();
    });
    let error = checked
        .replace_exact(&CheckedArtifactFact::Bytes(b"owned".to_vec()), b"goal")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(path).unwrap(), b"foreign");
}

#[cfg(unix)]
#[test]
fn same_byte_new_inode_after_final_proof_is_not_accepted_as_the_source() {
    use std::os::unix::fs::MetadataExt;
    use std::sync::{Arc, Mutex};

    let root = TempRoot::new("post-proof-same-bytes");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let path = root.0.join("a/value");
    fs::write(&path, b"owned").unwrap();
    let original_inode = fs::metadata(&path).unwrap().ino();
    let checked = artifact(&root.0, "a/value");
    let replacement = path.clone();
    let foreign_inode = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&foreign_inode);
    run_next_checked_artifact_at(CheckedArtifactFault::AfterFinalProof, move || {
        // Stage the same-byte replacement while the original still exists so
        // the two objects are provably distinct, then rename over the source.
        // Remove-then-create lets ext4-class allocators recycle the freed
        // inode number (observed on the Linux runners: old == new == 6029447),
        // which falsifies the new-object precondition below without weakening
        // the production guarantee under test.
        let staged = replacement.with_file_name("value.staged");
        fs::write(&staged, b"owned").unwrap();
        *recorded.lock().unwrap() = Some(fs::metadata(&staged).unwrap().ino());
        fs::rename(&staged, &replacement).unwrap();
    });
    let error = checked
        .remove_exact(&CheckedArtifactFact::Bytes(b"owned".to_vec()))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    let surviving_inode = fs::metadata(&path).unwrap().ino();
    assert_ne!(surviving_inode, original_inode);
    assert_eq!(Some(surviving_inode), *foreign_inode.lock().unwrap());
    assert_eq!(fs::read(path).unwrap(), b"owned");
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn parent_move_after_final_proof_restores_the_retained_source_and_rejects() {
    let root = TempRoot::new("post-proof-parent");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"owned").unwrap();
    let checked = artifact(&root.0, "a/value");
    let workspace = root.0.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::AfterFinalProof, move || {
        fs::rename(workspace.join("a"), workspace.join("old-a")).unwrap();
        fs::create_dir(workspace.join("a")).unwrap();
        fs::write(workspace.join("a/value"), b"foreign").unwrap();
    });
    let error = checked
        .remove_exact(&CheckedArtifactFact::Bytes(b"owned".to_vec()))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(root.0.join("old-a/value")).unwrap(), b"owned");
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"foreign");
}
