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

fn init_root_metadata_feature(
    root: &Path,
    backend: &crate::git::Git2Backend,
) -> (String, String, Vec<u8>, Vec<u8>) {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let baseline = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    let baseline_manifest = fs::read(root.join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let baseline_lock = fs::read(root.join(crate::artifact::LOCK_PATH)).unwrap();
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();

    let mut lock = crate::artifact::read_lock(root).unwrap();
    lock.members.get_mut("mem_remote").unwrap().branch = Some("feature/root-abort".to_owned());
    fs::write(
        root.join(crate::artifact::LOCK_PATH),
        lock.to_yaml().unwrap(),
    )
    .unwrap();
    let manifest = String::from_utf8(baseline_manifest.clone())
        .unwrap()
        .replacen(
            "schema: gwz.workspace/v0",
            "schema: gwz.workspace/v0 # root abort source",
            1,
        );
    backend
        .stage_paths(root, &[crate::artifact::LOCK_PATH])
        .unwrap();
    let source = commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &manifest,
        "root metadata source",
        &[git2::Oid::from_str(&baseline).unwrap()],
    )
    .unwrap();
    backend.switch_branch(root, "main").unwrap();
    (baseline, source, baseline_manifest, baseline_lock)
}

fn init_root_manifest_conflict(
    root: &Path,
    backend: &crate::git::Git2Backend,
) -> (String, Vec<u8>, Vec<u8>) {
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    let baseline_manifest = fs::read(&manifest_path).unwrap();
    let baseline_lock = fs::read(root.join(crate::artifact::LOCK_PATH)).unwrap();
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
fn abort_after_root_evidence_restores_the_pre_merge_metadata_and_head() {
    let temp = TempDir::new("merge-root-evidence-abort");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-evidence-source");
    let (root_before, _source, baseline_manifest, baseline_lock) =
        init_root_metadata_feature(temp.path(), &backend);
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);

    let error = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        root_request(),
        "op_root_evidence",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(store.fired.get());
    let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert_ne!(
        backend.head(temp.path()).unwrap().commit,
        Some(root_before.clone())
    );

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
        "op_root_evidence_abort",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
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
fn root_evidence_abort_resumes_after_each_rollback_mutation() {
    use crate::workspace_ops::merge::{
        EvidenceRollbackMutation, fail_next_evidence_rollback_after,
    };

    for mutation in [
        EvidenceRollbackMutation::Boundary,
        EvidenceRollbackMutation::Lock,
        EvidenceRollbackMutation::Marker,
        EvidenceRollbackMutation::Staging,
    ] {
        let temp = TempDir::new(&format!("merge-root-evidence-resume-{mutation:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("root-resume-{mutation:?}"));
        let member = temp.path().join("remote");
        let (member_before, _) = feature_commit(&backend, &member, "member.txt", "member source\n");
        let (root_before, _, baseline_manifest, baseline_lock) =
            init_root_metadata_feature(temp.path(), &backend);
        let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);

        invoke_with_store(
            &backend,
            &store,
            temp.path(),
            mixed_request(),
            "op_root_evidence_resume",
        )
        .unwrap_err();
        let record = store.discover_open(temp.path()).unwrap().unwrap();

        fail_next_evidence_rollback_after(mutation);
        let error = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone())),
            "op_root_evidence_abort_interrupted",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{mutation:?}");

        let interrupted = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(interrupted.state, OperationState::RollingBack);
        assert!(
            !interrupted.publication.unwrap().evidence_rolled_back,
            "{mutation:?}"
        );

        let status = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Status, None),
            "op_root_evidence_abort_status",
        )
        .unwrap();
        let root = merge_repo(&status, "@root");
        assert!(root.drift.is_empty(), "{mutation:?}: {:?}", root.drift);
        assert_eq!(root.abort_eligible, Some(true), "{mutation:?}");
        assert!(
            status.operation_drift.iter().all(|drift| {
                drift.kind != crate::MergeOperationDriftKind::RootCandidateStateChanged
            }),
            "{mutation:?}: {:?}",
            status.operation_drift
        );

        let aborted = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
            "op_root_evidence_abort_resumed",
        )
        .unwrap();
        assert_eq!(
            aborted.state,
            crate::MergeOperationState::Aborted,
            "{mutation:?}"
        );
        assert_eq!(
            backend.head(temp.path()).unwrap().commit.as_deref(),
            Some(root_before.as_str()),
            "{mutation:?}"
        );
        assert_eq!(
            backend.head(&member).unwrap().commit.as_deref(),
            Some(member_before.as_str()),
            "{mutation:?}"
        );
        assert_eq!(
            fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
            baseline_manifest,
            "{mutation:?}"
        );
        assert_eq!(
            fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
            baseline_lock,
            "{mutation:?}"
        );
    }
}

#[test]
fn interrupted_root_evidence_abort_rejects_unrelated_root_work() {
    use crate::workspace_ops::merge::{
        EvidenceRollbackMutation, fail_next_evidence_rollback_after,
    };

    let temp = TempDir::new("merge-root-evidence-unrelated-work");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "root-evidence-unrelated-work");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "member.txt",
        "member source\n",
    );
    init_root_metadata_feature(temp.path(), &backend);
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        mixed_request(),
        "op_root_evidence_unrelated",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();

    fail_next_evidence_rollback_after(EvidenceRollbackMutation::Boundary);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone())),
        "op_root_evidence_unrelated_interrupted",
    )
    .unwrap_err();
    fs::write(temp.path().join("unrelated-after-interrupt.txt"), "keep\n").unwrap();

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_root_evidence_unrelated_status",
    )
    .unwrap();
    let root = merge_repo(&status, "@root");
    assert!(!root.drift.is_empty());
    assert_eq!(root.abort_eligible, Some(false));

    let error = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
        "op_root_evidence_unrelated_retry",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(
        fs::read_to_string(temp.path().join("unrelated-after-interrupt.txt")).unwrap(),
        "keep\n"
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
    let record_before = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_root_abort_blocked",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(backend.head(&member).unwrap(), member_head);
    assert_eq!(
        FileMergeStore.discover_open(temp.path()).unwrap().unwrap(),
        record_before
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("post-merge.txt")).unwrap(),
        "keep me\n"
    );
}
