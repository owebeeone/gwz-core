use super::*;
use crate::artifact::LOCK_PATH;
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::RootMetadataRollbackStepV1;
use crate::workspace_ops::merge::root::{
    V1RootRollbackObservation as O, execute_v1_root_metadata_rollback,
    observe_v1_root_metadata_rollback,
};

fn root_fixture(name: &str) -> (TempDir, Git2Backend, MergeOperationRecordV1) {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    std::fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let manifest = "result manifest\n";
    let first = commit_file(&root.path, WORKSPACE_MANIFEST, manifest, "manifest", &[]).unwrap();
    let lock = "result lock\n";
    let result = commit_file(
        &root.path,
        LOCK_PATH,
        lock,
        "lock",
        &[first.parse().unwrap()],
    )
    .unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    model.selected_targets = vec!["@root".into()];
    let mut row = model.participants.remove("mem_a").unwrap();
    row.path = ".".into();
    row.target_kind = MergeTargetKind::Root;
    row.target_branch = "main".into();
    row.state = ParticipantState::Merged;
    row.resulting_commit = Some(result);
    model.participants.clear();
    model.participants.insert("@root".into(), row);
    model.baseline.manifest_yaml = Some("baseline manifest\n".into());
    model.baseline.lock_yaml = Some("baseline lock\n".into());
    (root, backend, model)
}

#[test]
fn selected_root_steps_are_exact_and_sequential() {
    let (root, backend, model) = root_fixture("v1-rollback-root-phases");
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Complete,
        )
        .unwrap(),
        O::Before
    );
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Manifest,
        )
        .unwrap(),
        O::Before
    );
    execute_v1_root_metadata_rollback(
        &backend,
        &root.path,
        &model,
        RootMetadataRollbackStepV1::Manifest,
    )
    .unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Manifest,
        )
        .unwrap(),
        O::After
    );
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Lock,
        )
        .unwrap(),
        O::Before
    );
    execute_v1_root_metadata_rollback(
        &backend,
        &root.path,
        &model,
        RootMetadataRollbackStepV1::Lock,
    )
    .unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Complete,
        )
        .unwrap(),
        O::After
    );
}

#[cfg(unix)]
#[test]
fn selected_root_rejects_a_symlink_leaf() {
    use std::os::unix::fs::symlink;
    let (root, backend, model) = root_fixture("v1-rollback-root-symlink");
    std::fs::remove_file(root.path.join(WORKSPACE_MANIFEST)).unwrap();
    symlink("target", root.path.join(WORKSPACE_MANIFEST)).unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Manifest,
        )
        .unwrap(),
        O::Ambiguous
    );
}

#[test]
fn selected_root_checked_write_preserves_a_leaf_replaced_before_linearization() {
    use crate::checked_artifact::{CheckedArtifactFault, run_next_checked_artifact_at};

    let (root, backend, model) = root_fixture("v1-rollback-root-replaced-leaf");
    let manifest = root.path.join(WORKSPACE_MANIFEST);
    let replacement = manifest.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck, move || {
        std::fs::remove_file(&replacement).unwrap();
        std::fs::write(replacement, "foreign manifest\n").unwrap();
    });
    let error = execute_v1_root_metadata_rollback(
        &backend,
        &root.path,
        &model,
        RootMetadataRollbackStepV1::Manifest,
    )
    .unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);
    assert_eq!(
        std::fs::read_to_string(manifest).unwrap(),
        "foreign manifest\n"
    );
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Manifest,
        )
        .unwrap(),
        O::Ambiguous
    );
}

#[test]
fn selected_root_rejects_lock_restored_ahead_of_manifest() {
    let (root, backend, model) = root_fixture("v1-rollback-root-out-of-order");
    std::fs::write(
        root.path.join(LOCK_PATH),
        model.baseline.lock_yaml.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Manifest,
        )
        .unwrap(),
        O::Ambiguous
    );
}

#[test]
fn selected_root_lock_and_complete_reject_third_states() {
    let (root, backend, model) = root_fixture("v1-rollback-root-lock-third");
    execute_v1_root_metadata_rollback(
        &backend,
        &root.path,
        &model,
        RootMetadataRollbackStepV1::Manifest,
    )
    .unwrap();
    std::fs::write(root.path.join(WORKSPACE_MANIFEST), "result manifest\n").unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Lock,
        )
        .unwrap(),
        O::Ambiguous
    );

    let (root, backend, model) = root_fixture("v1-rollback-root-complete-third");
    for step in [
        RootMetadataRollbackStepV1::Manifest,
        RootMetadataRollbackStepV1::Lock,
    ] {
        execute_v1_root_metadata_rollback(&backend, &root.path, &model, step).unwrap();
    }
    std::fs::write(root.path.join(LOCK_PATH), "foreign\n").unwrap();
    assert_eq!(
        observe_v1_root_metadata_rollback(
            &backend,
            &root.path,
            &model,
            RootMetadataRollbackStepV1::Complete,
        )
        .unwrap(),
        O::Ambiguous
    );
}
