use crate::git::Git2Backend;
use crate::runtime::clock::TimestampMs;
use crate::workspace_ops::tests::TempDir;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
pub(super) struct MemoryStore {
    pub(super) records: Mutex<Vec<MergeOperationRecord>>,
    pub(super) trace: Mutex<Vec<String>>,
    pub(super) events: Mutex<Vec<crate::OperationEvent>>,
    pub(super) writes: Mutex<usize>,
    pub(super) fail_write_at: Mutex<Option<usize>>,
}

impl MergeStore for MemoryStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        Ok(self.records.lock().unwrap().last().cloned())
    }

    fn write_open(&self, _root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        let mut writes = self.writes.lock().unwrap();
        *writes += 1;
        if self.fail_write_at.lock().unwrap().as_ref() == Some(&*writes) {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "injected record write failure",
            ));
        }
        self.trace
            .lock()
            .unwrap()
            .push(format!("write:{:?}", record.state));
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }

    fn archive(&self, _root: &Path, _merge_id: &str) -> ModelResult<()> {
        Ok(())
    }
}

pub(super) struct TraceSink<'a>(pub(super) &'a MemoryStore);

#[test]
fn retry_true_merge_uses_request_author_and_committer() {
    let root = TempDir::new("merge-retry-attribution");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::TrueMerge);
    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::RecoveryRequired;
    record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
    store.write_open(root.path(), &record).unwrap();

    let response =
        resume_with_context(&backend, &store, root.path(), &attributed_context()).unwrap();
    let repository = git2::Repository::open(root.path().join("app")).unwrap();
    let commit = repository
        .find_commit(
            git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(commit.author().name(), Ok("Merge Request Author"));
    assert_eq!(commit.author().email(), Ok("merge-author@example.invalid"));
    assert_eq!(commit.committer().name(), Ok("Merge Request Committer"));
    assert_eq!(
        commit.committer().email(),
        Ok("merge-committer@example.invalid")
    );
}

#[test]
fn pending_true_merge_not_started_executes_frozen_spec() {
    let root = TempDir::new("merge-pending-true-merge-frozen");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::TrueMerge);
    let app = root.path().join("app");
    let frozen_context = attributed_context();
    let result = backend
        .prepare_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            frozen_context.attribution.as_ref(),
        )
        .unwrap();
    let GitPreparedMerge::Commit(frozen_commit) = &result else {
        panic!("fixture must prepare a clean true merge")
    };
    let frozen_commit = frozen_commit.clone();
    let repository_before_validation = file_snapshot(&app);
    backend
        .validate_prepared_merge_upstream_state(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &result,
        )
        .unwrap();
    assert_eq!(file_snapshot(&app), repository_before_validation);
    let prepared = PreparedAction {
        kind: GitMergeAnalysisKind::TrueMerge,
        result,
    };
    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::RecoveryRequired;
    record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
    set_pending_action(&mut record, &plan.participants[0], &prepared).unwrap();
    let frozen_pending = record.participants["mem_app"]
        .pending_action
        .clone()
        .unwrap();
    store.write_open(root.path(), &record).unwrap();

    let mut retry_context = attributed_context();
    let attribution = retry_context.attribution.as_mut().unwrap();
    attribution.git_author.as_mut().unwrap().name = "Replacement Author".to_owned();
    attribution.git_author.as_mut().unwrap().time_ms = Some(TimestampMs(1_800_000_000_000));
    attribution.git_committer.as_mut().unwrap().name = "Replacement Committer".to_owned();
    attribution.git_committer.as_mut().unwrap().time_ms = Some(TimestampMs(1_800_000_100_000));
    Git2Backend::reset_preparation_call_count();

    let response = resume_with_context(&backend, &store, root.path(), &retry_context).unwrap();

    assert_eq!(Git2Backend::preparation_call_count(), 0);
    let repository = git2::Repository::open(&app).unwrap();
    let commit = repository
        .find_commit(
            git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(commit.tree_id().to_string(), frozen_commit.tree_oid);
    assert_eq!(
        commit.author().name(),
        Ok(frozen_commit.author.name.as_str())
    );
    assert_eq!(
        commit.author().email(),
        Ok(frozen_commit.author.email.as_str())
    );
    assert_eq!(
        commit.author().when().seconds(),
        frozen_commit.author.time_seconds
    );
    assert_eq!(
        commit.author().when().offset_minutes(),
        frozen_commit.author.timezone_offset_minutes
    );
    assert_eq!(
        commit.committer().name(),
        Ok(frozen_commit.committer.name.as_str())
    );
    assert_eq!(
        commit.committer().email(),
        Ok(frozen_commit.committer.email.as_str())
    );
    assert_eq!(
        commit.committer().when().seconds(),
        frozen_commit.committer.time_seconds
    );
    assert_eq!(
        commit.committer().when().offset_minutes(),
        frozen_commit.committer.timezone_offset_minutes
    );
    assert!(store.records.lock().unwrap().iter().all(|record| {
        record.participants["mem_app"]
            .pending_action
            .as_ref()
            .is_none_or(|pending| pending == &frozen_pending)
    }));
}

#[test]
fn invalid_durable_true_merge_evidence_is_ambiguous_and_blocks_recovery() {
    #[derive(Clone, Copy, Debug)]
    enum InvalidEvidence {
        MissingTree,
        MalformedTree,
        InvalidAuthor,
        InvalidCommitter,
        InvalidTimezone,
    }

    for case in [
        InvalidEvidence::MissingTree,
        InvalidEvidence::MalformedTree,
        InvalidEvidence::InvalidAuthor,
        InvalidEvidence::InvalidCommitter,
        InvalidEvidence::InvalidTimezone,
    ] {
        let root = TempDir::new(&format!("merge-invalid-pending-{case:?}"));
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, ActionFixture::TrueMerge);
        let app = root.path().join("app");
        let prepared = backend
            .prepare_merge_upstream_checked(
                &app,
                &plan.participants[0].target_branch,
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                attributed_context().attribution.as_ref(),
            )
            .unwrap();
        let GitPreparedMerge::Commit(prepared_commit) = &prepared else {
            panic!("fixture must prepare a clean true merge")
        };
        let recorded_tree = prepared_commit.tree_oid.clone();
        let action = PreparedAction {
            kind: GitMergeAnalysisKind::TrueMerge,
            result: prepared,
        };
        let store = MemoryStore::default();
        let mut record = durable_record(root.path(), &plan);
        record.state = OperationState::RecoveryRequired;
        record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
        set_pending_action(&mut record, &plan.participants[0], &action).unwrap();
        let spec = record
            .participants
            .get_mut("mem_app")
            .unwrap()
            .pending_action
            .as_mut()
            .unwrap()
            .commit_spec
            .as_mut()
            .unwrap();
        match case {
            InvalidEvidence::MissingTree => remove_loose_object(&app, &recorded_tree),
            InvalidEvidence::MalformedTree => spec.tree_oid = "not-an-object-id".to_owned(),
            InvalidEvidence::InvalidAuthor => spec.author.name = "<invalid>".to_owned(),
            InvalidEvidence::InvalidCommitter => {
                spec.committer.email = "invalid\n@example.test".to_owned();
            }
            InvalidEvidence::InvalidTimezone => {
                spec.committer.timezone_offset_minutes = 1_441;
            }
        }
        let super::super::pending::DurablePreparedAction::Merge(durable_prepared) =
            super::super::pending::decode_durable_prepared_action(
                record.participants["mem_app"]
                    .pending_action
                    .as_ref()
                    .unwrap(),
            )
            .unwrap()
        else {
            panic!("true-merge intent must decode as a prepared merge")
        };
        let durable_before = record.clone();
        store.write_open(root.path(), &record).unwrap();
        let repository_before = file_snapshot(&app);
        let lock_before = std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap();
        let manifest_before =
            std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
        Git2Backend::reset_preparation_call_count();

        let observed = super::super::status::observe_participant(
            &backend,
            root.path(),
            "mem_app",
            &record.participants["mem_app"],
        )
        .unwrap();

        assert_eq!(
            observed.pending_action.unwrap().state,
            super::super::PendingActionObservationState::Ambiguous,
            "case={case:?}"
        );
        assert!(!observed.continue_eligibility.eligible, "case={case:?}");
        assert!(!observed.abort_eligibility.eligible, "case={case:?}");
        assert_eq!(
            backend
                .execute_prepared_merge_upstream_checked(
                    &app,
                    &plan.participants[0].target_branch,
                    &plan.participants[0].before_commit,
                    &plan.participants[0].source_commit,
                    &plan.participants[0].commit_message,
                    &durable_prepared,
                )
                .unwrap_err()
                .code,
            ErrorCode::MergeRecoveryRequired,
            "case={case:?}"
        );
        assert_eq!(
            resume(&backend, &store, root.path()).unwrap_err().code,
            ErrorCode::MergeRecoveryRequired,
            "case={case:?}"
        );
        assert_eq!(
            abort(&backend, &store, root.path()).unwrap_err().code,
            ErrorCode::MergeDrift,
            "case={case:?}"
        );
        assert_eq!(Git2Backend::preparation_call_count(), 0, "case={case:?}");
        assert_eq!(*store.writes.lock().unwrap(), 1, "case={case:?}");
        assert_eq!(
            store.records.lock().unwrap().as_slice(),
            &[durable_before],
            "case={case:?}"
        );
        assert_eq!(file_snapshot(&app), repository_before, "case={case:?}");
        assert_eq!(
            std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap(),
            lock_before,
            "case={case:?}"
        );
        assert_eq!(
            std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
            manifest_before,
            "case={case:?}"
        );
    }
}

#[test]
fn pending_resolution_exact_retry_uses_frozen_signatures() {
    let root = TempDir::new("merge-pending-resolution-frozen");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
    let app = root.path().join("app");
    let conflict = backend
        .prepare_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            None,
        )
        .unwrap();
    assert_eq!(conflict, GitPreparedMerge::ExpectedConflict);
    backend
        .execute_prepared_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &plan.participants[0].commit_message,
            &conflict,
        )
        .unwrap();
    std::fs::write(app.join("README.md"), "frozen resolution\n").unwrap();
    backend.stage_paths(&app, &["README.md"]).unwrap();
    let frozen_context = attributed_context();
    let frozen_commit = backend
        .prepare_merge_resolution_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            frozen_context.attribution.as_ref(),
        )
        .unwrap();
    let repository_before_validation = file_snapshot(&app);
    backend
        .validate_prepared_merge_resolution_state(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &frozen_commit,
        )
        .unwrap();
    assert_eq!(file_snapshot(&app), repository_before_validation);

    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::AwaitingResolution;
    let participant = record.participants.get_mut("mem_app").unwrap();
    participant.state = ParticipantState::Conflicted;
    participant.expected_merge_head = Some(plan.participants[0].source_commit.clone());
    participant.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::ResolveConflict,
        target_branch: plan.participants[0].target_branch.clone(),
        before_commit: plan.participants[0].before_commit.clone(),
        source_commit: plan.participants[0].source_commit.clone(),
        commit_message: plan.participants[0].commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: pending_commit_spec(&GitPreparedMerge::Commit(frozen_commit.clone())),
        extensions: BTreeMap::new(),
    });
    let frozen_pending = participant.pending_action.clone().unwrap();
    store.write_open(root.path(), &record).unwrap();

    let mut retry_context = attributed_context();
    let attribution = retry_context.attribution.as_mut().unwrap();
    attribution.git_author.as_mut().unwrap().name = "Replacement Author".to_owned();
    attribution.git_author.as_mut().unwrap().time_ms = Some(TimestampMs(1_800_000_000_000));
    attribution.git_committer.as_mut().unwrap().name = "Replacement Committer".to_owned();
    attribution.git_committer.as_mut().unwrap().time_ms = Some(TimestampMs(1_800_000_100_000));
    Git2Backend::reset_preparation_call_count();

    let response = resume_with_context(&backend, &store, root.path(), &retry_context).unwrap();

    assert_eq!(Git2Backend::preparation_call_count(), 0);
    let repository = git2::Repository::open(&app).unwrap();
    let commit = repository
        .find_commit(
            git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(commit.tree_id().to_string(), frozen_commit.tree_oid);
    assert_eq!(
        commit.author().name(),
        Ok(frozen_commit.author.name.as_str())
    );
    assert_eq!(
        commit.author().when().seconds(),
        frozen_commit.author.time_seconds
    );
    assert_eq!(
        commit.committer().name(),
        Ok(frozen_commit.committer.name.as_str())
    );
    assert_eq!(
        commit.committer().when().seconds(),
        frozen_commit.committer.time_seconds
    );
    assert!(store.records.lock().unwrap().iter().all(|record| {
        record.participants["mem_app"]
            .pending_action
            .as_ref()
            .is_none_or(|pending| pending == &frozen_pending)
    }));
}

#[test]
fn recovery_required_retry_store_failure_adopts_without_repeating_git() {
    let root = TempDir::new("merge-retry-action-recovery");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::FastForward);
    let source = plan.participants[0].source_commit.clone();
    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::RecoveryRequired;
    record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
    store.write_open(root.path(), &record).unwrap();
    *store.fail_write_at.lock().unwrap() = Some(4);
    resume(&backend, &store, root.path()).unwrap_err();
    assert_eq!(
        backend.head(&root.path().join("app")).unwrap().commit,
        Some(source.clone())
    );
    assert_eq!(
        store.records.lock().unwrap().last().unwrap().participants["mem_app"]
            .pending_action
            .as_ref()
            .unwrap()
            .kind,
        PendingMergeActionKind::FastForward
    );

    *store.fail_write_at.lock().unwrap() = None;
    let response = resume(&backend, &store, root.path()).unwrap();
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        response.repos[0].resulting_commit.as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        backend.head(&root.path().join("app")).unwrap().commit,
        Some(source)
    );
}
