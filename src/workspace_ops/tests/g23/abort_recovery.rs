use super::*;

#[test]
fn abort_accepts_every_pre_candidate_finalization_fault_point() {
    for fault in [
        FinalizationFault::AfterEnteringFinalizing,
        FinalizationFault::BeforeCandidateCreation,
    ] {
        let temp = TempDir::new(&format!("merge-pre-candidate-abort-{fault:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("pre-candidate-{fault:?}"));
        let member = temp.path().join("remote");
        let (member_before, _) = feature_commit(&backend, &member, "README.md", "source\n");
        let root_before = backend.head(temp.path()).unwrap();
        let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
        let store = FaultingMergeStore::new(fault);

        invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_pre_candidate_abort",
        )
        .unwrap_err();
        let record = store.discover_open(temp.path()).unwrap().unwrap();
        assert!(
            record
                .publication
                .as_ref()
                .is_none_or(|publication| publication.candidate.is_none()),
            "{fault:?}"
        );

        let aborted = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
            "op_pre_candidate_abort_resume",
        )
        .unwrap();
        assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
        assert_eq!(backend.head(temp.path()).unwrap(), root_before);
        assert_eq!(
            backend.head(&member).unwrap().commit.as_deref(),
            Some(member_before.as_str())
        );
        assert_eq!(
            fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
            lock_before
        );
    }
}

#[test]
fn born_root_evidence_abort_recovers_both_record_persistence_windows() {
    for fault in [
        FinalizationFault::AfterEvidenceCommit,
        FinalizationFault::AfterEvidencePersistence,
    ] {
        assert_root_evidence_abort_recovers(fault, true);
    }
}

#[test]
fn unborn_root_evidence_abort_recovers_both_record_persistence_windows() {
    for fault in [
        FinalizationFault::AfterEvidenceCommit,
        FinalizationFault::AfterEvidencePersistence,
    ] {
        assert_root_evidence_abort_recovers(fault, false);
    }
}

#[test]
fn evidence_abort_resumes_after_each_artifact_restoration_mutation() {
    use crate::workspace_ops::merge::{
        EvidenceRollbackMutation, fail_next_evidence_rollback_after,
    };

    for mutation in [
        EvidenceRollbackMutation::Boundary,
        EvidenceRollbackMutation::Lock,
        EvidenceRollbackMutation::Marker,
        EvidenceRollbackMutation::Staging,
    ] {
        let temp = TempDir::new(&format!("merge-evidence-artifact-abort-{mutation:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("artifact-{mutation:?}"));
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );

        let candidate_store = FaultingMergeStore::new(FinalizationFault::AfterCandidatePersistence);
        invoke_with_store(
            &backend,
            &candidate_store,
            temp.path(),
            request(false),
            "op_artifact_candidate",
        )
        .unwrap_err();
        let mut record = candidate_store.discover_open(temp.path()).unwrap().unwrap();
        let candidate = record
            .publication
            .as_mut()
            .and_then(|publication| publication.candidate.as_mut())
            .unwrap();
        candidate.boundary_text.push_str("# rollback-window\n");
        candidate.boundary_sha256 =
            format!("{:x}", Sha256::digest(candidate.boundary_text.as_bytes()));
        assert_ne!(
            candidate.boundary_text, candidate.baseline_boundary_text,
            "{mutation:?}"
        );
        FileMergeStore.write_open(temp.path(), &record).unwrap();

        let publication_store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
        invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
            "op_artifact_publication",
        )
        .unwrap_err();
        let record = publication_store
            .discover_open(temp.path())
            .unwrap()
            .unwrap();

        fail_next_evidence_rollback_after(mutation);
        let error = invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone())),
            "op_artifact_abort_interrupted",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);

        let status = invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Status, None),
            "op_artifact_abort_status",
        )
        .unwrap();
        assert!(
            status.operation_drift.iter().all(|drift| {
                drift.kind != crate::MergeOperationDriftKind::RootCandidateStateChanged
            }),
            "{mutation:?}"
        );

        let aborted = invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone())),
            "op_artifact_abort_resume",
        )
        .unwrap();
        assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
        assert!(
            publication_store
                .load(temp.path(), &record.merge_id)
                .unwrap()
                .publication
                .unwrap()
                .evidence_rolled_back,
            "{mutation:?}"
        );
    }
}

#[test]
fn mixed_merge_abort_restores_exact_baseline_and_archives_operation() {
    let temp = TempDir::new("merge-mixed-abort");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let manifest_before = fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    for (path, expected) in [
        ("app", fixture.app_before),
        ("lib", fixture.lib_before),
        ("docs", fixture.docs_before),
    ] {
        assert_eq!(
            backend.head(&temp.path().join(path)).unwrap().commit,
            Some(expected)
        );
        assert!(
            backend
                .merge_state(&temp.path().join(path))
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        manifest_before
    );
    assert!(
        !temp
            .path()
            .join(format!(".gwz/merge/{merge_id}.yaml"))
            .exists()
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Idle);
    assert!(!status.open);
}

#[test]
fn crash_reload_continue_foreign_rejection_and_external_restore_converge_on_abort() {
    let temp = TempDir::new("merge-adversarial-lifecycle");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    assert_eq!(
        merge_repo(&started, "mem_lib").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&started, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    // A new backend instance models a fresh process reloading only durable
    // operation state before the conflict is resolved.
    let reloaded = crate::git::Git2Backend::new();
    let status = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status_reload",
    )
    .unwrap();
    assert_eq!(
        merge_repo(&status, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    let docs = temp.path().join("docs");
    fs::write(docs.join("README.md"), "resolved after reload\n").unwrap();
    reloaded
        .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
        .unwrap();
    let late_drift = temp.path().join("lib/late-finalization.txt");
    let injected_drift = late_drift.clone();
    crate::git::Git2Backend::before_next_scoped_commit_ref_lock(move || {
        fs::write(injected_drift, "late drift\n").unwrap();
    });
    let continued = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_continue_reload",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(continued.open);
    fs::remove_file(late_drift).unwrap();
    let lib = temp.path().join("lib");
    let lib_result = merge_repo(&continued, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let docs_result = merge_repo(&continued, "mem_docs")
        .resulting_commit
        .clone()
        .unwrap();

    // Poison a participant that abort would have to roll back. Whole-operation
    // preflight must reject before changing the later docs participant.
    let lib_repo = git2::Repository::open(&lib).unwrap();
    let cherry_pick_head = lib_repo.path().join("CHERRY_PICK_HEAD");
    fs::write(&cherry_pick_head, format!("{lib_result}\n")).unwrap();
    let record_path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let record_before_rejection = fs::read(&record_path).unwrap();
    let error = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_foreign",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(
        reloaded.head(&lib).unwrap().commit.as_deref(),
        Some(lib_result.as_str())
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit.as_deref(),
        Some(docs_result.as_str())
    );
    assert_eq!(fs::read(&record_path).unwrap(), record_before_rejection);
    fs::remove_file(cherry_pick_head).unwrap();

    // Simulate an exact external restoration after the interrupted process.
    // Coordinated abort must recognize it as a no-op and roll back only the
    // participant that remains changed.
    reloaded
        .set_branch_target_checked(&docs, "main", &docs_result, &fixture.docs_before)
        .unwrap();
    let aborted = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_reloaded",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    assert_eq!(
        reloaded.head(&lib).unwrap().commit,
        Some(fixture.lib_before)
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
}

#[test]
fn post_merge_commit_rejects_abort_before_conflicted_member_changes() {
    let temp = TempDir::new("merge-mixed-abort-drift");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let lib = temp.path().join("lib");
    let lib_result = backend.head(&lib).unwrap().commit.unwrap();
    let post_merge = commit_file(
        &lib,
        "post-merge.txt",
        "later work\n",
        "later work",
        &[git2::Oid::from_str(&lib_result).unwrap()],
    )
    .unwrap();
    let docs = temp.path().join("docs");
    let docs_state = backend.merge_state(&docs).unwrap().unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(post_merge));
    assert_eq!(
        backend.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert_eq!(backend.merge_state(&docs).unwrap(), Some(docs_state));
}
