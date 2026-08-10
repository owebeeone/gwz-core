mod cleanup;
pub(in crate::workspace_ops::merge::record_wire::archive) mod fixtures;
mod v0;
mod v1;

fn location_test_root(name: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gwz-record-location-{name}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join(".gwz/merge")).unwrap();
    root
}

#[test]
fn canonical_location_rejects_real_directory_parent_replacement() {
    let root = location_test_root("parent-replacement");
    std::fs::write(root.join(".gwz/merge/merge_1.yaml"), b"record").unwrap();
    crate::workspace_ops::merge::record_wire::replace_parent_before_final_check_for_test();

    let error = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap_err();

    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn complete_location_identity_observes_an_archive_that_appears_after_absence() {
    let root = location_test_root("archive-appearance");
    let first = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap();
    assert!(first.open().is_absent());
    assert!(first.archived().is_absent());

    std::fs::create_dir(root.join(".gwz/merge/done")).unwrap();
    std::fs::write(root.join(".gwz/merge/done/merge_1.yaml"), b"record").unwrap();
    let second = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap();

    assert_ne!(first, second);
    assert!(!second.archived().is_absent());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn canonical_location_rejects_open_leaf_appearance_in_the_same_call() {
    let root = location_test_root("open-appearance");
    crate::workspace_ops::merge::record_wire::appear_open_before_final_check_for_test();

    let error = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap_err();

    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn canonical_location_rejects_byte_identical_leaf_replacement_in_the_same_call() {
    let root = location_test_root("open-replacement");
    std::fs::write(root.join(".gwz/merge/merge_1.yaml"), b"same bytes").unwrap();
    crate::workspace_ops::merge::record_wire::replace_open_before_final_check_for_test();

    let error = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap_err();

    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn canonical_location_rejects_archive_appearance_in_the_same_call() {
    let root = location_test_root("archive-appearance-same-call");
    crate::workspace_ops::merge::record_wire::appear_archived_before_final_check_for_test();

    let error = crate::workspace_ops::merge::record_wire::acquire_canonical_merge_locations(
        &root, "merge_1",
    )
    .unwrap_err();

    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);
    let _ = std::fs::remove_dir_all(root);
}
