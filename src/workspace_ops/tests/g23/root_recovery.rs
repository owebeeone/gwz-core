use super::*;

fn root_request() -> crate::MergeRequest {
    let mut request = request(false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    request
}

fn mixed_request() -> crate::MergeRequest {
    let mut request = request(false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_remote".to_owned(), "@root".to_owned()],
        ..Default::default()
    });
    request
}

fn init_manifest_conflict(
    root: &Path,
    backend: &crate::git::Git2Backend,
) -> (String, String, String) {
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    let resolved = fs::read_to_string(&manifest_path).unwrap();
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let base = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();
    let feature = resolved.replacen(
        "schema: gwz.workspace/v0",
        "schema: gwz.workspace/v0 # feature",
        1,
    );
    fs::write(root.join("root.txt"), "feature\n").unwrap();
    backend.stage_paths(root, &["root.txt"]).unwrap();
    let source = commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &feature,
        "feature manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.switch_branch(root, "main").unwrap();
    let main = resolved.replacen(
        "schema: gwz.workspace/v0",
        "schema: gwz.workspace/v0 # main",
        1,
    );
    fs::write(root.join("root.txt"), "main\n").unwrap();
    backend.stage_paths(root, &["root.txt"]).unwrap();
    let before = commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &main,
        "main manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    (before, source, resolved)
}

#[test]
fn conflicted_root_status_and_continue_do_not_require_a_valid_live_manifest() {
    let temp = TempDir::new("merge-root-conflicted-manifest");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-recovery-source");
    let (before, source, resolved_manifest) = init_manifest_conflict(temp.path(), &backend);

    let started = handle_merge(&backend, temp.path(), root_request(), "op_root_conflict").unwrap();

    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert_eq!(
        merge_repo(&started, "@root").state,
        crate::MergeParticipantState::Conflicted
    );
    assert!(
        fs::read_to_string(temp.path().join(crate::workspace::WORKSPACE_MANIFEST))
            .unwrap()
            .contains("<<<<<<<")
    );

    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_status",
    )
    .unwrap();
    let root_status = merge_repo(&status, "@root");
    assert_eq!(root_status.target_kind, crate::TargetKind::Root);
    assert_eq!(root_status.state, crate::MergeParticipantState::Conflicted);
    assert_eq!(root_status.before_commit, before);
    assert_eq!(root_status.source_commit, source);

    fs::write(temp.path().join("root.txt"), "resolved root\n").unwrap();
    handle_stage(
        &backend,
        temp.path(),
        crate::StageRequest {
            meta: request_meta(),
            cwd: temp.path().to_string_lossy().into_owned(),
            pathspecs: vec!["root.txt".to_owned()],
            all: None,
        },
        "op_root_stage_non_metadata",
    )
    .unwrap();
    assert!(
        fs::read_to_string(temp.path().join(crate::workspace::WORKSPACE_MANIFEST))
            .unwrap()
            .contains("<<<<<<<")
    );

    fs::write(
        temp.path().join(crate::workspace::WORKSPACE_MANIFEST),
        resolved_manifest,
    )
    .unwrap();
    handle_stage(
        &backend,
        temp.path(),
        crate::StageRequest {
            meta: request_meta(),
            cwd: temp.path().to_string_lossy().into_owned(),
            pathspecs: vec![crate::workspace::WORKSPACE_MANIFEST.to_owned()],
            all: None,
        },
        "op_root_stage",
    )
    .unwrap();
    let continued = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, started.merge_id),
        "op_root_continue",
    )
    .unwrap();

    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&continued, "@root").state,
        crate::MergeParticipantState::Continued
    );
    let root_result = merge_repo(&continued, "@root")
        .resulting_commit
        .as_deref()
        .unwrap();
    let evidence = backend.head(temp.path()).unwrap().commit.unwrap();
    assert_ne!(evidence, root_result);
    let repo = git2::Repository::open(temp.path()).unwrap();
    let commit = repo
        .find_commit(git2::Oid::from_str(root_result).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
}

#[test]
fn merged_root_cannot_redefine_an_in_flight_member_and_remains_abortable() {
    let temp = TempDir::new("merge-root-redefines-participant");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-redefine-source");
    let member = temp.path().join("remote");
    let (member_before, _) = feature_commit(&backend, &member, "member.txt", "member\n");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before =
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    let initial_manifest =
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let changed_manifest =
        String::from_utf8(initial_manifest)
            .unwrap()
            .replacen("path: remote", "path: renamed", 1);
    let (reported_root_before, _) = feature_commit(
        &backend,
        temp.path(),
        crate::workspace::WORKSPACE_MANIFEST,
        &changed_manifest,
    );
    assert_eq!(reported_root_before, root_before);
    let baseline_manifest =
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();

    let error =
        handle_merge(&backend, temp.path(), mixed_request(), "op_root_redefines").unwrap_err();

    assert_eq!(error.code, ErrorCode::ManifestInvalid);
    assert!(error.message.contains("identity changed"));
    assert_eq!(error.member_id.as_deref(), Some("mem_remote"));
    assert_eq!(error.member_path.as_deref(), Some("remote"));
    let open = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(open.state, OperationState::Finalizing);
    let member_result = open.participants["mem_remote"]
        .resulting_commit
        .clone()
        .unwrap();
    assert_ne!(member_result, member_before);

    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_redefines_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Finalizing);
    assert_eq!(status.repos.len(), 2);
    assert!(status.operation_drift.iter().any(|drift| {
        drift.kind == crate::MergeOperationDriftKind::RootCandidateMetadataInvalid
    }));

    let retry_error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(open.merge_id.clone())),
        "op_root_redefines_retry",
    )
    .unwrap_err();
    assert_eq!(retry_error.code, ErrorCode::ManifestInvalid);
    assert_eq!(retry_error.member_id.as_deref(), Some("mem_remote"));
    assert_eq!(retry_error.member_path.as_deref(), Some("remote"));

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(open.merge_id)),
        "op_root_redefines_abort",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        baseline_manifest
    );
}

#[test]
fn merged_root_schema_and_path_errors_remain_durable_retryable_and_abortable() {
    for (name, before, after, expected_code) in [
        (
            "unsupported-schema",
            "schema: gwz.workspace/v0",
            "schema: gwz.workspace/v99",
            ErrorCode::SchemaUnsupported,
        ),
        (
            "invalid-member-path",
            "path: remote",
            "path: ../escape",
            ErrorCode::PathEscape,
        ),
    ] {
        let temp = TempDir::new(&format!("merge-root-invalid-metadata-{name}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(temp.path(), &backend, &format!("invalid-{name}"));
        let member = temp.path().join("remote");
        let (member_before, _) = feature_commit(&backend, &member, "member.txt", "member source\n");
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        let root_before =
            commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
        let baseline_manifest =
            fs::read_to_string(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
        let changed_manifest = baseline_manifest.replacen(before, after, 1);
        feature_commit(
            &backend,
            temp.path(),
            crate::workspace::WORKSPACE_MANIFEST,
            &changed_manifest,
        );

        let error = handle_merge(
            &backend,
            temp.path(),
            mixed_request(),
            format!("op_root_invalid_{name}"),
        )
        .unwrap_err();
        assert_eq!(error.code, expected_code, "{name}");
        assert_eq!(error.member_id.as_deref(), Some("@root"), "{name}");
        assert_eq!(error.member_path.as_deref(), Some("."), "{name}");
        let open = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();

        let status = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Status, None),
            format!("op_root_invalid_status_{name}"),
        )
        .unwrap();
        assert!(
            status.operation_drift.iter().any(|drift| {
                drift.kind == crate::MergeOperationDriftKind::RootCandidateMetadataInvalid
            }),
            "{name}: {:?}",
            status.operation_drift
        );

        let retry = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(open.merge_id.clone())),
            format!("op_root_invalid_retry_{name}"),
        )
        .unwrap_err();
        assert_eq!(retry.code, expected_code, "{name}");
        assert_eq!(retry.member_id.as_deref(), Some("@root"), "{name}");

        let aborted = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(open.merge_id)),
            format!("op_root_invalid_abort_{name}"),
        )
        .unwrap();
        assert_eq!(aborted.state, crate::MergeOperationState::Aborted, "{name}");
        assert_eq!(
            backend.head(temp.path()).unwrap().commit.as_deref(),
            Some(root_before.as_str()),
            "{name}"
        );
        assert_eq!(
            backend.head(&member).unwrap().commit.as_deref(),
            Some(member_before.as_str()),
            "{name}"
        );
    }
}
