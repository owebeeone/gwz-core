use super::*;

#[test]
fn post_candidate_manifest_drift_blocks_until_exact_repair() {
    let temp = TempDir::new("merge-finalize-manifest-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-manifest-drift");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let manifest_path = temp.path().join(crate::workspace::WORKSPACE_MANIFEST);
    let manifest_before = fs::read(&manifest_path).unwrap();
    let root_before = backend.head(temp.path()).unwrap();
    let store = FaultingMergeStore::new(FinalizationFault::AfterCandidatePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_manifest_drift",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&manifest_path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_manifest_status",
    )
    .unwrap();
    assert!(
        status
            .operation_drift
            .iter()
            .any(|drift| { drift.kind == crate::MergeOperationDriftKind::BaselineManifestChanged })
    );
    let continued = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_manifest_continue",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert_eq!(backend.head(temp.path()).unwrap(), root_before);
    assert!(
        store
            .discover_open(temp.path())
            .unwrap()
            .unwrap()
            .publication
            .unwrap()
            .composition_commit
            .is_none()
    );

    fs::write(&manifest_path, manifest_before).unwrap();
    let completed = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_manifest_repaired",
    )
    .unwrap();
    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    let archived = store.load(temp.path(), &record.merge_id).unwrap();
    let composition = archived.publication.unwrap().composition_commit.unwrap();
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(composition.as_str())
    );
}

#[test]
fn status_detects_marker_and_boundary_drift_without_a_prior_recovery_mutation() {
    let temp = TempDir::new("merge-finalize-status-artifact-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-status-artifact-drift");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_status_artifact_drift",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .unwrap();
    let marker_path = crate::artifact::marker_path(temp.path(), &candidate.marker_id);
    let durable_before = fs::read(
        temp.path()
            .join(format!(".gwz/merge/{}.yaml", record.merge_id)),
    )
    .unwrap();
    fs::remove_file(&marker_path).unwrap();

    let marker_status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_marker_status",
    )
    .unwrap();
    assert!(
        marker_status.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join(format!(".gwz/merge/{}.yaml", record.merge_id)),
        )
        .unwrap(),
        durable_before
    );

    fs::write(&marker_path, &candidate.marker_yaml).unwrap();
    let boundary_path = crate::workspace_ops::workspace_exclude_path(temp.path());
    fs::write(&boundary_path, "corrupt boundary\n").unwrap();
    let boundary_status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_boundary_status",
    )
    .unwrap();
    assert!(
        boundary_status.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join(format!(".gwz/merge/{}.yaml", record.merge_id)),
        )
        .unwrap(),
        durable_before
    );
}

#[test]
fn candidate_artifact_drift_blocks_continue_and_abort_without_mutation() {
    let temp = TempDir::new("merge-finalize-candidate-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-candidate-drift-source");
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault").unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .unwrap();
    let marker = crate::artifact::marker_path(temp.path(), &candidate.marker_id);
    fs::remove_file(&marker).unwrap();
    let root_head = backend.head(temp.path()).unwrap();
    let member_head = backend.head(&member).unwrap();

    let continued = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_continue",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(
        continued.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    assert!(!marker.exists());

    let aborted = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
        "op_abort",
    )
    .unwrap_err();
    assert_eq!(aborted.code, ErrorCode::MergeDrift);
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(backend.head(&member).unwrap(), member_head);
}

#[test]
fn root_branch_switch_at_the_same_commit_blocks_finalization() {
    let temp = TempDir::new("merge-finalize-root-branch-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "merge-root-branch-drift-source");
    commit_file(temp.path(), "root.txt", "root\n", "root baseline", &[]).unwrap();
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");
    let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
    invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault").unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let baseline_root = backend.head(temp.path()).unwrap().commit.unwrap();
    backend.branch_create(temp.path(), "other", "HEAD").unwrap();
    backend.switch_branch(temp.path(), "other").unwrap();

    let continued = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_continue",
    )
    .unwrap();

    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(
        continued.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    let root_head = backend.head(temp.path()).unwrap();
    assert_eq!(root_head.branch.as_deref(), Some("other"));
    assert_eq!(root_head.commit.as_deref(), Some(baseline_root.as_str()));
    assert!(
        store
            .discover_open(temp.path())
            .unwrap()
            .unwrap()
            .publication
            .unwrap()
            .composition_commit
            .is_none()
    );

    backend.switch_branch(temp.path(), "main").unwrap();
    let completed = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_continue_repaired_branch",
    )
    .unwrap();
    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    let archived = store.load(temp.path(), &record.merge_id).unwrap();
    let composition = archived.publication.unwrap().composition_commit.unwrap();
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(composition.as_str())
    );
}
