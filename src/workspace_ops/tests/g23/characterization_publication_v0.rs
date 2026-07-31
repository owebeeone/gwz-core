use super::*;
use crate::git::{GitBackend, GitCandidateFile};
use crate::workspace_ops::merge::ParticipantState;

#[derive(Clone, Copy, Debug)]
struct PublicationWindow {
    fault: FinalizationFault,
    step: Option<PublicationStep>,
    candidate: bool,
    evidence: bool,
    published: bool,
}

const PUBLICATION_WINDOWS: &[PublicationWindow] = &[
    PublicationWindow {
        fault: FinalizationFault::AfterEnteringFinalizing,
        step: None,
        candidate: false,
        evidence: false,
        published: false,
    },
    PublicationWindow {
        fault: FinalizationFault::BeforeCandidateCreation,
        step: Some(PublicationStep::ValidatingResults),
        candidate: false,
        evidence: false,
        published: false,
    },
    PublicationWindow {
        fault: FinalizationFault::AfterCandidatePersistence,
        step: Some(PublicationStep::PreparingCandidate),
        candidate: true,
        evidence: false,
        published: false,
    },
    PublicationWindow {
        fault: FinalizationFault::AfterEvidenceCommit,
        step: Some(PublicationStep::CommittingEvidence),
        candidate: true,
        evidence: false,
        published: false,
    },
    PublicationWindow {
        fault: FinalizationFault::AfterEvidencePersistence,
        step: Some(PublicationStep::CommittingEvidence),
        candidate: true,
        evidence: true,
        published: false,
    },
    PublicationWindow {
        fault: FinalizationFault::AfterLockPublication,
        step: Some(PublicationStep::PublishingCandidate),
        candidate: true,
        evidence: true,
        published: true,
    },
    PublicationWindow {
        fault: FinalizationFault::BeforeArchive,
        step: Some(PublicationStep::Complete),
        candidate: true,
        evidence: true,
        published: true,
    },
];

#[derive(Default)]
struct VerifyingPostWriteStore {
    fired: Cell<bool>,
}

impl MergeStore for VerifyingPostWriteStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        FileMergeStore.write_open(root, record)?;
        if !self.fired.get()
            && record.publication.as_ref().map(|progress| progress.step)
                == Some(PublicationStep::VerifyingPublication)
        {
            self.fired.set(true);
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "injected post-write verifying-publication failure",
            ));
        }
        Ok(())
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        FileMergeStore.archive(root, merge_id)
    }
}

#[test]
fn v0_publication_windows_have_born_and_unborn_root_twins() {
    for born in [false, true] {
        for window in PUBLICATION_WINDOWS {
            let kind = if born { "born" } else { "unborn" };
            let label = format!("{kind}-{:?}", window.fault);
            let temp = TempDir::new(&format!("v0-publication-{label}"));
            let backend = crate::git::Git2Backend::new();
            let _fixture = init_one_member_workspace(temp.path(), &backend, &label);
            make_root_born_if_requested(&backend, temp.path(), born);
            feature_commit(
                &backend,
                &temp.path().join("remote"),
                "README.md",
                "source\n",
            );
            let baseline = backend.head(temp.path()).unwrap();
            let store = FaultingMergeStore::new(window.fault);

            let error = invoke_with_store(
                &backend,
                &store,
                temp.path(),
                request(false),
                "op_v0_publication_twin",
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{label}");
            let record = store.discover_open(temp.path()).unwrap().unwrap();

            let expected_state = if matches!(window.fault, FinalizationFault::BeforeArchive) {
                OperationState::Completed
            } else {
                OperationState::Finalizing
            };
            assert_eq!(record.state, expected_state, "{label}");
            assert_eq!(record.baseline.root_head, baseline.commit, "{label}");
            assert_eq!(record.baseline.root_branch, baseline.branch, "{label}");
            assert_eq!(
                record.publication.as_ref().map(|progress| progress.step),
                window.step,
                "{label}"
            );
            assert_publication_shape(
                &backend,
                temp.path(),
                &record,
                window.candidate,
                window.evidence,
                window.published,
                &label,
            );
        }
    }
}

#[test]
fn v0_verifying_publication_is_durable_for_born_and_unborn_roots() {
    for born in [false, true] {
        let kind = if born { "born" } else { "unborn" };
        let label = format!("verifying-{kind}");
        let temp = TempDir::new(&format!("v0-{label}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(temp.path(), &backend, &label);
        make_root_born_if_requested(&backend, temp.path(), born);
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let store = VerifyingPostWriteStore::default();

        let error = invoke_with_dependencies(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_verifying",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{label}");
        assert!(store.fired.get(), "{label}");
        let record = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.state, OperationState::Finalizing, "{label}");
        assert_eq!(
            record.publication.as_ref().unwrap().step,
            PublicationStep::VerifyingPublication,
            "{label}"
        );
        assert_publication_shape(&backend, temp.path(), &record, true, true, true, &label);
        assert_status_is_byte_exact(
            &backend,
            temp.path(),
            &record,
            Some(crate::MergePublicationStep::VerifyingPublication),
        );
    }
}

#[test]
fn v0_recovery_required_overlays_keep_each_constructible_publication_row_byte_exact() {
    let awaiting = TempDir::new("v0-recovery-awaiting");
    let awaiting_backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(awaiting.path(), &awaiting_backend);
    let started = handle_merge(
        &awaiting_backend,
        awaiting.path(),
        request(false),
        "op_v0_recovery_awaiting",
    )
    .unwrap();
    let mut awaiting_record = FileMergeStore
        .load(awaiting.path(), started.merge_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(awaiting_record.state, OperationState::AwaitingResolution);
    awaiting_record.state = OperationState::RecoveryRequired;
    FileMergeStore
        .write_open(awaiting.path(), &awaiting_record)
        .unwrap();
    assert_status_is_byte_exact(&awaiting_backend, awaiting.path(), &awaiting_record, None);

    for window in PUBLICATION_WINDOWS {
        let label = format!("recovery-{:?}", window.fault);
        let temp = TempDir::new(&format!("v0-{label}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(temp.path(), &backend, &label);
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let store = FaultingMergeStore::new(window.fault);
        invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_recovery_source",
        )
        .unwrap_err();
        let mut record = store.discover_open(temp.path()).unwrap().unwrap();
        record.state = OperationState::RecoveryRequired;
        FileMergeStore.write_open(temp.path(), &record).unwrap();
        assert_status_is_byte_exact(&backend, temp.path(), &record, window.step.map(Into::into));
    }
}

#[test]
fn v0_no_publication_completed_open_record_closes_byte_exactly() {
    let temp = TempDir::new("v0-no-publication-terminal-open");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "v0-no-publication-terminal-open");
    backend
        .branch_create(&temp.path().join("remote"), "feature/source", "HEAD")
        .unwrap();
    let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_v0_no_publication_open",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(record.state, OperationState::Completed);
    let publication = record.publication.as_ref().unwrap();
    assert_eq!(publication.step, PublicationStep::Complete);
    assert!(publication.candidate.is_none());
    assert!(publication.composition_commit.is_none());
    assert!(publication.composition_tree.is_none());
    assert!(publication.candidate_hashes.is_empty());
    assert_status_is_byte_exact(
        &backend,
        temp.path(),
        &record,
        Some(crate::MergePublicationStep::Complete),
    );
    assert_terminal_close_is_byte_exact(
        &backend,
        &store,
        temp.path(),
        &record,
        crate::MergeOp::Resume,
    );
}

#[test]
fn v0_candidate_aborted_open_record_closes_byte_exactly() {
    let temp = TempDir::new("v0-candidate-aborted-terminal-open");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "v0-candidate-aborted-terminal-open");
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
        "op_v0_candidate_open",
    )
    .unwrap_err();
    let candidate = candidate_store.discover_open(temp.path()).unwrap().unwrap();
    let terminal_store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    invoke_with_store(
        &backend,
        &terminal_store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(candidate.merge_id)),
        "op_v0_candidate_abort",
    )
    .unwrap_err();
    let record = terminal_store.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(record.state, OperationState::Aborted);
    assert!(
        record
            .publication
            .as_ref()
            .and_then(|progress| progress.candidate.as_ref())
            .is_some()
    );
    assert_eq!(
        record.participants["mem_remote"].state,
        ParticipantState::RolledBack
    );
    assert_status_is_byte_exact(
        &backend,
        temp.path(),
        &record,
        Some(crate::MergePublicationStep::PreparingCandidate),
    );
    assert_terminal_close_is_byte_exact(
        &backend,
        &terminal_store,
        temp.path(),
        &record,
        crate::MergeOp::Abort,
    );
}

fn invoke_with_dependencies<S: MergeStore>(
    backend: &crate::git::Git2Backend,
    store: &S,
    root: &Path,
    request: crate::MergeRequest,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request,
        operation_id,
    )
}

fn make_root_born_if_requested(backend: &crate::git::Git2Backend, root: &Path, born: bool) {
    if !born {
        return;
    }
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
}

fn assert_publication_shape(
    backend: &crate::git::Git2Backend,
    root: &Path,
    record: &MergeOperationRecord,
    has_candidate: bool,
    has_evidence: bool,
    published: bool,
    label: &str,
) {
    let publication = record.publication.as_ref();
    let candidate = publication.and_then(|progress| progress.candidate.as_ref());
    assert_eq!(candidate.is_some(), has_candidate, "{label}");
    assert_eq!(
        publication
            .and_then(|progress| progress.composition_commit.as_ref())
            .is_some(),
        has_evidence,
        "{label}"
    );
    assert_eq!(
        publication
            .and_then(|progress| progress.composition_tree.as_ref())
            .is_some(),
        has_evidence,
        "{label}"
    );
    assert_eq!(
        publication.is_some_and(|progress| !progress.candidate_hashes.is_empty()),
        has_evidence,
        "{label}"
    );
    let Some(candidate) = candidate else {
        return;
    };
    let progress = publication.unwrap();
    assert!(progress.candidate_lock_sha256.is_some(), "{label}");
    let marker_path = progress.candidate_marker_path.as_deref().unwrap();
    assert_eq!(
        fs::read_to_string(root.join(crate::artifact::LOCK_PATH)).unwrap() == candidate.lock_yaml,
        published,
        "{label}"
    );
    assert_eq!(root.join(marker_path).is_file(), published, "{label}");
    let expected_boundary = if published {
        &candidate.boundary_text
    } else {
        &candidate.baseline_boundary_text
    };
    assert_eq!(
        fs::read_to_string(crate::workspace_ops::workspace_exclude_path(root)).unwrap(),
        *expected_boundary,
        "{label}"
    );
    if published {
        let files = vec![
            GitCandidateFile {
                path: crate::artifact::LOCK_PATH.to_owned(),
                bytes: candidate.lock_yaml.as_bytes().to_vec(),
            },
            GitCandidateFile {
                path: marker_path.to_owned(),
                bytes: candidate.marker_yaml.as_bytes().to_vec(),
            },
        ];
        assert!(
            backend
                .index_matches_candidate_files(root, &files, &[])
                .unwrap(),
            "{label}"
        );
    }
}

fn assert_status_is_byte_exact(
    backend: &crate::git::Git2Backend,
    root: &Path,
    record: &MergeOperationRecord,
    expected_step: Option<crate::MergePublicationStep>,
) {
    let path = root.join(format!(".gwz/merge/{}.yaml", record.merge_id));
    let before = fs::read(&path).unwrap();
    let response = handle_merge(
        backend,
        root,
        recovery_request(crate::MergeOp::Status, Some(record.merge_id.clone())),
        "op_v0_byte_exact_status",
    )
    .unwrap();
    assert_eq!(response.publication_step, expected_step);
    assert!(response.open);
    assert_eq!(fs::read(path).unwrap(), before);
}

fn assert_terminal_close_is_byte_exact(
    backend: &crate::git::Git2Backend,
    store: &FaultingMergeStore,
    root: &Path,
    record: &MergeOperationRecord,
    op: crate::MergeOp,
) {
    let open = root.join(format!(".gwz/merge/{}.yaml", record.merge_id));
    let before = fs::read(&open).unwrap();
    let response = invoke_with_store(
        backend,
        store,
        root,
        recovery_request(op, Some(record.merge_id.clone())),
        "op_v0_byte_exact_close",
    )
    .unwrap();
    assert!(!response.open);
    assert!(!open.exists());
    assert_eq!(
        fs::read(root.join(format!(".gwz/merge/done/{}.yaml", record.merge_id))).unwrap(),
        before
    );
}
