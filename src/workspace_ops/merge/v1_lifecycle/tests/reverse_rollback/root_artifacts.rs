use super::*;
use crate::artifact::LOCK_PATH;
use crate::filesystem::{FileSystem, make_filesystem, remove_file_for_test, write_for_test};
use crate::git::{
    GitRepository, GitTestRepository, TestCommitSpec, TestHead, TestRefTarget, TestRepoSpec,
    make_repository,
};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::RootMetadataRollbackStepV1;
use crate::workspace_ops::merge::root::{
    V1RootRollbackObservation as O, execute_v1_root_metadata_rollback,
    observe_v1_root_metadata_rollback,
};

fn root_fixture(name: &str) -> (TempDir, GitTestRepository, MergeOperationRecordV1) {
    let root = TempDir::new(name);
    let backend = make_repository();
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    make_filesystem()
        .create_directories(&root.path.join("gwz.conf"))
        .unwrap();
    let manifest = "result manifest\n";
    let first = commit_fixture_file(
        &backend,
        &root.path,
        WORKSPACE_MANIFEST,
        manifest,
        "manifest",
        &[],
    );
    let lock = "result lock\n";
    let result = commit_fixture_file(&backend, &root.path, LOCK_PATH, lock, "lock", &[first]);
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

fn commit_fixture_file(
    backend: &GitTestRepository,
    root: &std::path::Path,
    relative: &str,
    bytes: &str,
    message: &str,
    parents: &[String],
) -> String {
    write_for_test(&root.join(relative), bytes.as_bytes()).unwrap();
    backend.stage_paths(root, &[relative]).unwrap();
    let commit = backend
        .test_create_commit(root, &TestCommitSpec::from_index(message, parents.to_vec()))
        .unwrap();
    backend
        .test_set_ref(
            root,
            "refs/heads/main",
            Some(&TestRefTarget::Direct(commit.clone())),
        )
        .unwrap();
    backend
        .test_set_head(root, &TestHead::Attached("refs/heads/main".into()))
        .unwrap();
    commit
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

#[test]
fn selected_root_rejects_a_symlink_leaf() {
    let (root, backend, model) = root_fixture("v1-rollback-root-symlink");
    remove_file_for_test(&root.path.join(WORKSPACE_MANIFEST)).unwrap();
    let manifest = std::path::Path::new(WORKSPACE_MANIFEST);
    let parent = make_filesystem()
        .open_directory(&root.path.join(manifest.parent().unwrap()))
        .unwrap();
    make_filesystem()
        .test_create_symlink_at(&parent, manifest.file_name().unwrap(), "target".as_ref())
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
fn selected_root_checked_write_preserves_a_leaf_replaced_before_linearization() {
    use crate::checked_artifact::{CheckedArtifactFault, run_next_checked_artifact_at};

    let (root, backend, model) = root_fixture("v1-rollback-root-replaced-leaf");
    let manifest = root.path.join(WORKSPACE_MANIFEST);
    let replacement = manifest.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck, move || {
        remove_file_for_test(&replacement).unwrap();
        write_for_test(&replacement, b"foreign manifest\n").unwrap();
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
        String::from_utf8(make_filesystem().read(&manifest).unwrap()).unwrap(),
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
    write_for_test(
        &root.path.join(LOCK_PATH),
        model.baseline.lock_yaml.as_deref().unwrap().as_bytes(),
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
    write_for_test(&root.path.join(WORKSPACE_MANIFEST), b"result manifest\n").unwrap();
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
    write_for_test(&root.path.join(LOCK_PATH), b"foreign\n").unwrap();
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
