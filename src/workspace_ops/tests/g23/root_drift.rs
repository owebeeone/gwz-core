use super::*;

fn mixed_request() -> crate::MergeRequest {
    let mut request = request(false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_remote".to_owned(), "@root".to_owned()],
        ..Default::default()
    });
    request
}

fn start_open_mixed_merge(
    root: &Path,
    backend: &crate::git::Git2Backend,
    operation_id: &str,
) -> crate::MergeResponse {
    let _fixture = init_one_member_workspace(root, backend, operation_id);
    let member = root.join("remote");
    let (member_base, _) = feature_commit(backend, &member, "README.md", "source\n");
    commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_base).unwrap()],
    )
    .unwrap();
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(backend, root, "root-source.txt", "source\n");

    let response = handle_merge(backend, root, mixed_request(), operation_id).unwrap();
    assert_eq!(
        response.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert_eq!(
        merge_repo(&response, "@root").state,
        crate::MergeParticipantState::FastForwarded
    );
    response
}

#[test]
fn root_head_advance_is_reported_and_blocks_abort_without_mutation() {
    let temp = TempDir::new("merge-root-head-drift");
    let backend = crate::git::Git2Backend::new();
    let started = start_open_mixed_merge(temp.path(), &backend, "op_root_head_drift");
    let root_result = backend.head(temp.path()).unwrap().commit.unwrap();
    let advanced = commit_file(
        temp.path(),
        "post-merge.txt",
        "keep\n",
        "post merge work",
        &[git2::Oid::from_str(&root_result).unwrap()],
    )
    .unwrap();
    let member = temp.path().join("remote");
    let member_head = backend.head(&member).unwrap();
    let record_before = open_record(temp.path()).unwrap();

    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_head_drift_status",
    )
    .unwrap();
    let root_status = merge_repo(&status, "@root");
    assert!(
        root_status
            .drift
            .iter()
            .any(|drift| { drift.kind == crate::MergeParticipantDriftKind::HeadAdvanced })
    );
    assert_eq!(root_status.abort_eligible, Some(false));

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_head_drift_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(advanced.as_str())
    );
    assert_eq!(backend.head(&member).unwrap(), member_head);
    assert_eq!(
        open_record(temp.path()).unwrap(),
        record_before
    );
}

#[test]
fn wrong_id_is_rejected_after_restart_before_root_abort_mutates_any_repo() {
    let temp = TempDir::new("merge-root-wrong-id");
    let backend = crate::git::Git2Backend::new();
    let started = start_open_mixed_merge(temp.path(), &backend, "op_root_wrong_id");
    let root_head = backend.head(temp.path()).unwrap();
    let member = temp.path().join("remote");
    let member_head = backend.head(&member).unwrap();
    let record_before = open_record(temp.path()).unwrap();

    let restarted_backend = crate::git::Git2Backend::new();
    let error = handle_merge(
        &restarted_backend,
        temp.path(),
        recovery_request(
            crate::MergeOp::Abort,
            Some("merge_different_operation".to_owned()),
        ),
        "op_root_wrong_id_rejected",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeIdMismatch);
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(backend.head(&member).unwrap(), member_head);
    assert_eq!(
        open_record(temp.path()).unwrap(),
        record_before
    );

    let aborted = handle_merge(
        &restarted_backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_restart_abort",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
}
