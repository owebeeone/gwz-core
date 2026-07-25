use super::*;

fn interrupted_root_finalization(
    name: &str,
) -> (
    TempDir,
    crate::git::Git2Backend,
    FaultingMergeStore,
    MergeOperationRecord,
) {
    let temp = TempDir::new(name);
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, name);
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(&backend, temp.path(), "root-feature.txt", "root feature\n");
    let mut request = request(false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);
    let error = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request,
        "op_root_status_fault",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    (temp, backend, store, record)
}

#[test]
fn status_recognizes_exact_root_composition_evidence() {
    let (temp, backend, store, record) =
        interrupted_root_finalization("merge-root-status-evidence");
    let publication = record.publication.as_ref().unwrap();
    let evidence = publication.composition_commit.as_deref().unwrap();
    let root_result = record.participants["@root"]
        .resulting_commit
        .as_deref()
        .unwrap();
    assert_ne!(evidence, root_result);

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_status",
    )
    .unwrap();
    let root = merge_repo(&status, "@root");
    assert_eq!(root.resulting_commit.as_deref(), Some(root_result));
    assert_eq!(root.live_commit.as_deref(), Some(evidence));
    assert!(root.drift.is_empty());
    assert_eq!(root.continue_eligible, Some(true));
    assert_eq!(root.abort_eligible, Some(true));
    assert!(status.operation_drift.is_empty());
}

#[test]
fn status_reports_candidate_boundary_drift_after_root_evidence() {
    let (temp, backend, store, _record) =
        interrupted_root_finalization("merge-root-status-boundary");
    fs::write(
        crate::workspace_ops::workspace_exclude_path(temp.path()),
        "# tampered after root evidence\n",
    )
    .unwrap();

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_status_tampered",
    )
    .unwrap();
    assert!(
        status.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
}
