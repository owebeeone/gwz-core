use super::*;
use crate::workspace_ops::merge::ParticipantState;

#[derive(Clone, Copy)]
struct ChangedMergeWindow {
    name: &'static str,
    fault: FinalizationFault,
    publication_step: Option<PublicationStep>,
    has_candidate: bool,
    has_recorded_evidence: bool,
    candidate_is_published: bool,
    state: OperationState,
}

const CHANGED_MERGE_WINDOWS: &[ChangedMergeWindow] = &[
    ChangedMergeWindow {
        name: "finalizing_before_publication_record",
        fault: FinalizationFault::AfterEnteringFinalizing,
        publication_step: None,
        has_candidate: false,
        has_recorded_evidence: false,
        candidate_is_published: false,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "validating_before_candidate",
        fault: FinalizationFault::BeforeCandidateCreation,
        publication_step: Some(PublicationStep::ValidatingResults),
        has_candidate: false,
        has_recorded_evidence: false,
        candidate_is_published: false,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "candidate_persisted_before_evidence",
        fault: FinalizationFault::AfterCandidatePersistence,
        publication_step: Some(PublicationStep::PreparingCandidate),
        has_candidate: true,
        has_recorded_evidence: false,
        candidate_is_published: false,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "evidence_created_before_recording",
        fault: FinalizationFault::AfterEvidenceCommit,
        publication_step: Some(PublicationStep::CommittingEvidence),
        has_candidate: true,
        has_recorded_evidence: false,
        candidate_is_published: false,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "evidence_recorded_before_publication",
        fault: FinalizationFault::AfterEvidencePersistence,
        publication_step: Some(PublicationStep::CommittingEvidence),
        has_candidate: true,
        has_recorded_evidence: true,
        candidate_is_published: false,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "candidate_published_before_recording",
        fault: FinalizationFault::AfterLockPublication,
        publication_step: Some(PublicationStep::PublishingCandidate),
        has_candidate: true,
        has_recorded_evidence: true,
        candidate_is_published: true,
        state: OperationState::Finalizing,
    },
    ChangedMergeWindow {
        name: "completed_before_archive",
        fault: FinalizationFault::BeforeArchive,
        publication_step: Some(PublicationStep::Complete),
        has_candidate: true,
        has_recorded_evidence: true,
        candidate_is_published: true,
        state: OperationState::Completed,
    },
];

#[test]
fn v0_changed_merge_windows_have_named_exact_durable_shapes() {
    for window in CHANGED_MERGE_WINDOWS {
        let temp = TempDir::new(&format!("v0-window-{}", window.name));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("v0-window-{}", window.name));
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let store = FaultingMergeStore::new(window.fault);

        let error = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_window",
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::MergeRecoveryRequired,
            "{}",
            window.name
        );

        let record = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.schema, "gwz.merge-operation/v0", "{}", window.name);
        assert_eq!(record.record_schema_version, 0, "{}", window.name);
        assert_eq!(record.state, window.state, "{}", window.name);
        assert_eq!(
            record.publication.as_ref().map(|progress| progress.step),
            window.publication_step,
            "{}",
            window.name
        );
        assert_eq!(
            record
                .publication
                .as_ref()
                .and_then(|progress| progress.candidate.as_ref())
                .is_some(),
            window.has_candidate,
            "{}",
            window.name
        );
        assert_eq!(
            record
                .publication
                .as_ref()
                .and_then(|progress| progress.composition_commit.as_ref())
                .is_some(),
            window.has_recorded_evidence,
            "{}",
            window.name
        );
        assert_eq!(
            record
                .publication
                .as_ref()
                .and_then(|progress| progress.composition_tree.as_ref())
                .is_some(),
            window.has_recorded_evidence,
            "{}",
            window.name
        );
        assert_eq!(
            record
                .publication
                .as_ref()
                .is_some_and(|progress| !progress.candidate_hashes.is_empty()),
            window.has_recorded_evidence,
            "{}",
            window.name
        );

        let participant = record.participants.get("mem_remote").unwrap();
        assert_eq!(
            participant.state,
            ParticipantState::FastForwarded,
            "{}",
            window.name
        );
        assert!(participant.resulting_commit.is_some(), "{}", window.name);

        if let Some(candidate) = record
            .publication
            .as_ref()
            .and_then(|progress| progress.candidate.as_ref())
        {
            assert_eq!(
                fs::read_to_string(temp.path().join(crate::artifact::LOCK_PATH)).unwrap()
                    == candidate.lock_yaml,
                window.candidate_is_published,
                "{}",
                window.name
            );
            assert_eq!(
                crate::artifact::marker_path(temp.path(), &candidate.marker_id).is_file(),
                window.candidate_is_published,
                "{}",
                window.name
            );
        }

        let root_head = backend.head(temp.path()).unwrap().commit;
        assert_eq!(
            root_head.is_some(),
            matches!(
                window.fault,
                FinalizationFault::AfterEvidenceCommit
                    | FinalizationFault::AfterEvidencePersistence
                    | FinalizationFault::AfterLockPublication
                    | FinalizationFault::BeforeArchive
            ),
            "{}",
            window.name
        );
    }
}

#[test]
fn v0_terminal_completed_before_archive_is_read_only_and_closes_byte_exactly() {
    let temp = TempDir::new("v0-completed-before-archive");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "v0-completed-before-archive");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_v0_completed_open",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(record.state, OperationState::Completed);
    let open_path = temp
        .path()
        .join(format!(".gwz/merge/{}.yaml", record.merge_id));
    let before = fs::read(&open_path).unwrap();

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, Some(record.merge_id.clone())),
        "op_v0_completed_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Completed);
    assert!(status.open);
    assert_eq!(fs::read(&open_path).unwrap(), before);

    let closed = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_v0_completed_close",
    )
    .unwrap();
    assert_eq!(closed.state, crate::MergeOperationState::Completed);
    assert!(!closed.open);
    assert!(!open_path.exists());
    assert_eq!(
        fs::read(
            temp.path()
                .join(format!(".gwz/merge/done/{}.yaml", record.merge_id)),
        )
        .unwrap(),
        before
    );
}

#[test]
fn v0_terminal_aborted_before_archive_is_read_only_and_closes_byte_exactly() {
    let temp = TempDir::new("v0-aborted-before-archive");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    let started = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_v0_aborted_start",
    )
    .unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    let merge_id = started.merge_id.unwrap();

    let error = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_v0_aborted_open",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(record.state, OperationState::Aborted);
    let open_path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let before = fs::read(&open_path).unwrap();

    let status = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Status, Some(merge_id.clone())),
        "op_v0_aborted_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Aborted);
    assert!(status.open);
    assert_eq!(fs::read(&open_path).unwrap(), before);

    let closed = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_v0_aborted_close",
    )
    .unwrap();
    assert_eq!(closed.state, crate::MergeOperationState::Aborted);
    assert!(!closed.open);
    assert!(!open_path.exists());
    assert_eq!(
        fs::read(temp.path().join(format!(".gwz/merge/done/{merge_id}.yaml")),).unwrap(),
        before
    );
}

#[test]
fn v0_no_publication_completion_preserves_born_and_unborn_root_inputs() {
    for born in [false, true] {
        let kind = if born { "born" } else { "unborn" };
        let temp = TempDir::new(&format!("v0-no-publication-{kind}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("v0-no-publication-{kind}"));
        if born {
            commit_file(
                temp.path(),
                "root-baseline.txt",
                "baseline\n",
                "root baseline",
                &[],
            )
            .unwrap();
        }
        let member = temp.path().join("remote");
        backend
            .branch_create(&member, "feature/source", "HEAD")
            .unwrap();
        let root_before = backend.head(temp.path()).unwrap();

        let response = handle_merge(
            &backend,
            temp.path(),
            request(false),
            format!("op_v0_no_publication_{kind}"),
        )
        .unwrap();
        assert_eq!(
            response.state,
            crate::MergeOperationState::Completed,
            "{kind}"
        );
        assert_eq!(
            response.publication_step,
            Some(crate::MergePublicationStep::Complete)
        );
        assert_eq!(backend.head(temp.path()).unwrap(), root_before, "{kind}");
        let record = FileMergeStore
            .load(temp.path(), response.merge_id.as_deref().unwrap())
            .unwrap();
        let publication = record.publication.as_ref().unwrap();
        assert_eq!(publication.step, PublicationStep::Complete, "{kind}");
        assert!(publication.candidate.is_none(), "{kind}");
        assert!(publication.composition_commit.is_none(), "{kind}");
        assert!(publication.composition_tree.is_none(), "{kind}");
        assert!(publication.candidate_hashes.is_empty(), "{kind}");
        assert_eq!(record.baseline.root_head, root_before.commit, "{kind}");
        assert_eq!(record.baseline.root_branch, root_before.branch, "{kind}");
        assert_eq!(
            record.participants["mem_remote"].state,
            ParticipantState::UpToDate,
            "{kind}"
        );
    }
}

#[test]
fn v0_recovery_required_overlays_preserve_candidate_and_no_publication_evidence() {
    let candidate_temp = TempDir::new("v0-recovery-candidate");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(candidate_temp.path(), &backend, "v0-recovery-candidate");
    feature_commit(
        &backend,
        &candidate_temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let faulting = FaultingMergeStore::new(FinalizationFault::AfterCandidatePersistence);
    invoke_with_store(
        &backend,
        &faulting,
        candidate_temp.path(),
        request(false),
        "op_v0_recovery_candidate",
    )
    .unwrap_err();
    let mut candidate_record = faulting
        .discover_open(candidate_temp.path())
        .unwrap()
        .unwrap();
    assert_recovery_required_status_is_read_only(
        &backend,
        candidate_temp.path(),
        &mut candidate_record,
        crate::MergePublicationStep::PreparingCandidate,
    );

    let no_publication_temp = TempDir::new("v0-recovery-no-publication");
    let no_publication_backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(
        no_publication_temp.path(),
        &no_publication_backend,
        "v0-recovery-no-publication",
    );
    let member = no_publication_temp.path().join("remote");
    no_publication_backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    let terminal_store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    invoke_with_store(
        &no_publication_backend,
        &terminal_store,
        no_publication_temp.path(),
        request(false),
        "op_v0_recovery_no_publication",
    )
    .unwrap_err();
    let mut no_publication_record = terminal_store
        .discover_open(no_publication_temp.path())
        .unwrap()
        .unwrap();
    assert!(
        no_publication_record
            .publication
            .as_ref()
            .unwrap()
            .candidate
            .is_none()
    );
    assert_recovery_required_status_is_read_only(
        &no_publication_backend,
        no_publication_temp.path(),
        &mut no_publication_record,
        crate::MergePublicationStep::Complete,
    );
}

fn assert_recovery_required_status_is_read_only(
    backend: &crate::git::Git2Backend,
    root: &Path,
    record: &mut MergeOperationRecord,
    expected_step: crate::MergePublicationStep,
) {
    record.state = OperationState::RecoveryRequired;
    FileMergeStore.write_open(root, record).unwrap();
    let path = root.join(format!(".gwz/merge/{}.yaml", record.merge_id));
    let before = fs::read(&path).unwrap();

    let status = handle_merge(
        backend,
        root,
        recovery_request(crate::MergeOp::Status, Some(record.merge_id.clone())),
        "op_v0_recovery_status",
    )
    .unwrap();

    assert_eq!(status.state, crate::MergeOperationState::RecoveryRequired);
    assert_eq!(status.publication_step, Some(expected_step));
    assert!(status.open);
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        FileMergeStore
            .load(root, &record.merge_id)
            .unwrap()
            .publication,
        record.publication
    );
}
