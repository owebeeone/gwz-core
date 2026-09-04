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

fn init_root_feature(root: &Path, backend: &crate::git::Git2Backend) -> (String, String) {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let baseline = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    let (reported_baseline, source) = feature_commit(backend, root, "root-source.txt", "source\n");
    assert_eq!(reported_baseline, baseline);
    (baseline, source)
}

fn init_root_manifest_conflict(
    root: &Path,
    backend: &crate::git::Git2Backend,
) -> (String, Vec<u8>, Vec<u8>) {
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    let baseline_manifest = fs::read(&manifest_path).unwrap();
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let base = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();
    let feature = String::from_utf8(baseline_manifest.clone())
        .unwrap()
        .replacen(
            "schema: gwz.workspace/v0",
            "schema: gwz.workspace/v0 # feature",
            1,
        );
    commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &feature,
        "feature manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.switch_branch(root, "main").unwrap();
    let main = String::from_utf8(baseline_manifest).unwrap().replacen(
        "schema: gwz.workspace/v0",
        "schema: gwz.workspace/v0 # main",
        1,
    );
    let before = commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &main,
        "main manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    let before_manifest = fs::read(&manifest_path).unwrap();
    let baseline_lock = fs::read(root.join(crate::artifact::LOCK_PATH)).unwrap();
    (before, before_manifest, baseline_lock)
}

#[test]
fn mixed_abort_rolls_root_back_before_restoring_members() {
    let temp = TempDir::new("merge-root-mixed-abort");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-abort-source");
    let member = temp.path().join("remote");
    let (member_before, _) = feature_commit(&backend, &member, "README.md", "source\n");
    commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_before).unwrap()],
    )
    .unwrap();
    let member_before = backend.head(&member).unwrap().commit.unwrap();
    let (root_before, _) = init_root_feature(temp.path(), &backend);

    let started = handle_merge(
        &backend,
        temp.path(),
        mixed_request(),
        "op_root_mixed_abort",
    )
    .unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_mixed_aborted",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        merge_repo(&aborted, "@root").state,
        crate::MergeParticipantState::RolledBack
    );
    assert_eq!(
        merge_repo(&aborted, "mem_remote").state,
        crate::MergeParticipantState::Aborted
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_before.as_str())
    );
}

#[test]
fn conflicted_root_abort_uses_the_durable_record_when_live_manifest_is_invalid() {
    let temp = TempDir::new("merge-root-conflict-abort");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-conflict-abort");
    let (root_before, baseline_manifest, baseline_lock) =
        init_root_manifest_conflict(temp.path(), &backend);

    let started = handle_merge(
        &backend,
        temp.path(),
        root_request(),
        "op_root_conflict_abort",
    )
    .unwrap();
    assert_eq!(
        merge_repo(&started, "@root").state,
        crate::MergeParticipantState::Conflicted
    );
    assert!(
        fs::read_to_string(temp.path().join(crate::workspace::WORKSPACE_MANIFEST))
            .unwrap()
            .contains("<<<<<<<")
    );

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_conflict_aborted",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        merge_repo(&aborted, "@root").state,
        crate::MergeParticipantState::Aborted
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        baseline_manifest
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        baseline_lock
    );
}

#[test]
fn post_merge_root_work_blocks_abort_without_mutation() {
    let temp = TempDir::new("merge-root-abort-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-drift-source");
    let member = temp.path().join("remote");
    let (member_base, _) = feature_commit(&backend, &member, "README.md", "source\n");
    commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_base).unwrap()],
    )
    .unwrap();
    init_root_feature(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        mixed_request(),
        "op_root_abort_drift",
    )
    .unwrap();
    fs::write(temp.path().join("post-merge.txt"), "keep me\n").unwrap();
    let root_head = backend.head(temp.path()).unwrap();
    let member_head = backend.head(&member).unwrap();
    let record_before = open_record(temp.path()).unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_abort_blocked",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(backend.head(&member).unwrap(), member_head);
    assert_eq!(open_record(temp.path()).unwrap(), record_before);
    assert_eq!(
        fs::read_to_string(temp.path().join("post-merge.txt")).unwrap(),
        "keep me\n"
    );
}
