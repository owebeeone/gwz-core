use std::collections::BTreeMap;
use std::fs;

use super::super::authority::{
    BoundExactObservation, BoundObservationRequest, CompletedObservation, EntryFact,
    ExactObservationFact, ExecutionDiagnostic, NotStartedObservation, ObservationKind,
    ParticipantActionPayload, ParticipantFailurePayload, ParticipantObservation,
    PhysicalActionKind, PreparedFailureHaltBatch, PreparedParticipantAction, V1LifecycleRequest,
    V1ResponseDisposition, VerifiedParticipantOutcome, VerifiedParticipants,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::service::{ExactObserver, PhysicalExecutor, run_test as run};
use super::super::store::CheckedV1Store;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, test_record};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PendingCommitSpec, PendingGitSignature,
    PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn read_only_status_responds_without_observation_or_execution() {
    let root = TempDir::new_git("merge-v1-service-status");
    seed_open(&root, &test_record());
    let mut runtime = PanicRuntime;

    let response = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Status,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(response.disposition(), V1ResponseDisposition::Status);
    assert_eq!(response.current().record(), &test_record());
}

#[test]
fn continue_selects_conflict_and_crosses_each_durable_owner_once() {
    let root = TempDir::new_git("merge-v1-service-continue-conflict");
    let mut model = test_record();
    model.state = OperationState::AwaitingResolution;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    seed_open(&root, &model);
    let mut runtime = ConflictRuntime::default();

    let error = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Continue,
        &mut runtime,
    );
    let Err(error) = error else {
        panic!("acceptance checkpoint unexpectedly completed")
    };

    assert_eq!(error.message, "acceptance checkpoint reached");
    assert_eq!(runtime.executions, 1);
    let current = CheckedV1Store::default()
        .load_open(&root.path, "merge_1")
        .unwrap();
    assert_eq!(current.record().state, OperationState::Finalizing);
    let row = &current.record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Continued);
    assert!(row.pending_action.is_none());
}

#[test]
fn failed_owned_action_is_not_executed_twice_in_one_invocation() {
    let root = TempDir::new_git("merge-v1-service-attempt-fence");
    let mut model = test_record();
    let action = up_to_date_action(&model);
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(action);
    seed_open(&root, &model);
    let mut runtime = FailedRuntime::default();

    let response = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Stopped(OperationState::Halted)
    );
    assert_eq!(runtime.executions, 1);
    let current = CheckedV1Store::default()
        .load_open(&root.path, "merge_1")
        .unwrap();
    assert_eq!(current.record().state, OperationState::Halted);
    let row = &current.record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Failed);
    assert!(row.pending_action.is_some());
    assert_eq!(
        row.error.as_ref().unwrap().message,
        "injected participant failure"
    );
}

#[test]
fn preparation_failure_halts_and_returns_without_same_invocation_retry() {
    let root = TempDir::new_git("merge-v1-service-preparation-fence");
    seed_open(&root, &test_record());
    let mut runtime = PreparationFailureRuntime::default();

    let response = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(runtime.observations, 1);
    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Stopped(OperationState::Halted)
    );
    let current = CheckedV1Store::default()
        .load_open(&root.path, "merge_1")
        .unwrap();
    assert_eq!(current.record().state, OperationState::Halted);
    let row = &current.record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Failed);
    assert!(row.pending_action.is_none());
}

#[test]
fn resume_start_returns_the_new_stopped_state_without_redispatching() {
    let root = TempDir::new_git("merge-v1-service-resume-start-stop");
    let mut model = test_record();
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    seed_open(&root, &model);
    let mut runtime = PanicRuntime;

    let response = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::ResumeStart,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Stopped(OperationState::AwaitingResolution)
    );
    assert_eq!(
        response.current().record().state,
        OperationState::AwaitingResolution
    );
}

struct PanicRuntime;

impl ExactObserver for PanicRuntime {
    fn observe(
        &mut self,
        _current: &StoredV1Record,
        _request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        panic!("status must not observe")
    }
}

impl PhysicalExecutor for PanicRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        panic!("status must not execute")
    }
}

#[derive(Default)]
struct ConflictRuntime {
    executions: usize,
}

impl ExactObserver for ConflictRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let fact = match request.kind() {
            ObservationKind::ParticipantPreparation { member_id } => {
                let mut row = current.record().participants[member_id].clone();
                row.pending_action = Some(resolve_action(&row));
                let proof = PreparedParticipantAction::for_test(
                    current,
                    member_id,
                    "prepare_participant",
                    "prepared",
                    ParticipantActionPayload {
                        member_id: member_id.clone(),
                        row,
                    },
                )?;
                ExactObservationFact::Completed(CompletedObservation::Participant(
                    ParticipantObservation::Prepared(Box::new(proof)),
                ))
            }
            ObservationKind::ParticipantAction { member_id } if self.executions == 0 => {
                let action = current.record().participants[member_id]
                    .pending_action
                    .clone()
                    .unwrap();
                ExactObservationFact::NotStarted(NotStartedObservation::Participant {
                    member_id: member_id.clone(),
                    action: Box::new(action),
                })
            }
            ObservationKind::ParticipantAction { member_id } => {
                let mut row = current.record().participants[member_id].clone();
                row.state = ParticipantState::Continued;
                row.resulting_commit = Some("d".repeat(40));
                row.expected_merge_head = None;
                row.conflict_paths.clear();
                row.conflict_snapshot.clear();
                row.error = None;
                row.pending_action = None;
                let proof = VerifiedParticipantOutcome::for_test(
                    current,
                    member_id,
                    "participant_outcome",
                    "completed",
                    ParticipantActionPayload {
                        member_id: member_id.clone(),
                        row,
                    },
                )?;
                ExactObservationFact::Completed(CompletedObservation::Participant(
                    ParticipantObservation::Outcome(Box::new(proof), EntryFact::None),
                ))
            }
            ObservationKind::ParticipantsComplete => {
                let proof = VerifiedParticipants::for_test(
                    current,
                    "@operation",
                    "enter_finalizing",
                    "executing",
                    (),
                )?;
                ExactObservationFact::Completed(CompletedObservation::Participants(proof))
            }
            ObservationKind::Acceptance => {
                return Err(ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    "acceptance checkpoint reached",
                ));
            }
            kind => panic!("unexpected observation: {kind:?}"),
        };
        BoundExactObservation::for_test(current, request, fact)
    }
}

impl PhysicalExecutor for ConflictRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        assert!(matches!(
            action,
            PhysicalActionKind::Participant { action, .. }
                if action.kind == PendingMergeActionKind::ResolveConflict
        ));
        self.executions += 1;
        ExecutionDiagnostic::Success
    }
}

#[derive(Default)]
struct FailedRuntime {
    executions: usize,
}

impl ExactObserver for FailedRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let ObservationKind::ParticipantAction { member_id } = request.kind() else {
            panic!("failure fixture must retain its participant owner")
        };
        let action = current.record().participants[member_id]
            .pending_action
            .clone()
            .unwrap();
        BoundExactObservation::for_test(
            current,
            request,
            ExactObservationFact::NotStarted(NotStartedObservation::Participant {
                member_id: member_id.clone(),
                action: Box::new(action),
            }),
        )
    }
}

impl PhysicalExecutor for FailedRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.executions += 1;
        ExecutionDiagnostic::Failed {
            code: ErrorCode::GitCommandFailed,
            message: "injected participant failure".into(),
            detail: None,
        }
    }
}

#[derive(Default)]
struct PreparationFailureRuntime {
    observations: usize,
}

impl ExactObserver for PreparationFailureRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let ObservationKind::ParticipantPreparation { member_id } = request.kind() else {
            panic!("preparation failure fixture selected another owner")
        };
        self.observations += 1;
        let mut row = current.record().participants[member_id].clone();
        row.state = ParticipantState::Failed;
        row.error = Some(MergeRecordError {
            code: ErrorCode::GitCommandFailed,
            message: "injected preparation failure".into(),
            detail: None,
        });
        let batch = PreparedFailureHaltBatch::for_test(
            current,
            member_id,
            "preparation_failure",
            "verified",
            ParticipantFailurePayload {
                member_id: member_id.clone(),
                row,
                later_unattempted: Vec::new(),
            },
        )?;
        BoundExactObservation::for_test(
            current,
            request,
            ExactObservationFact::Completed(CompletedObservation::Participant(
                ParticipantObservation::PreparationFailed(Box::new(batch)),
            )),
        )
    }
}

impl PhysicalExecutor for PreparationFailureRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        panic!("preparation failure must not execute")
    }
}

fn seed_open(root: &TempDir, model: &MergeOperationRecordV1) {
    let merge_root = root.path.join(".gwz/merge");
    fs::create_dir_all(&merge_root).unwrap();
    fs::write(
        merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap(),
    )
    .unwrap();
}

fn up_to_date_action(model: &MergeOperationRecordV1) -> PendingMergeAction {
    let row = &model.participants["mem_a"];
    PendingMergeAction {
        kind: PendingMergeActionKind::VerifyUpToDate,
        target_branch: row.target_branch.clone(),
        before_commit: row.before_commit.clone(),
        source_commit: row.source_commit.clone(),
        commit_message: row.commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Unchanged),
        commit_spec: None,
        extensions: BTreeMap::new(),
    }
}

fn resolve_action(row: &crate::workspace_ops::merge::MergeParticipantRecord) -> PendingMergeAction {
    PendingMergeAction {
        kind: PendingMergeActionKind::ResolveConflict,
        target_branch: row.target_branch.clone(),
        before_commit: row.before_commit.clone(),
        source_commit: row.source_commit.clone(),
        commit_message: row.commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: Some(PendingCommitSpec {
            tree_oid: "c".repeat(40),
            author: signature("author"),
            committer: signature("committer"),
            extensions: BTreeMap::new(),
        }),
        extensions: BTreeMap::new(),
    }
}

fn signature(name: &str) -> PendingGitSignature {
    PendingGitSignature {
        name: name.into(),
        email: format!("{name}@example.test"),
        time_seconds: 123,
        timezone_offset_minutes: 600,
        extensions: BTreeMap::new(),
    }
}
