use std::collections::BTreeMap;
use std::fs;

use sha2::{Digest, Sha256};

use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, BoundObservationRequest, CompletedObservation,
    EntryFact, ExactObservationFact, ExecutionDiagnostic, NotStartedObservation, ObservationKind,
    ParticipantActionPayload, ParticipantObservation, PhysicalActionKind,
    PreparedParticipantAction, V1LifecycleRequest, V1ResponseDisposition,
    VerifiedParticipantOutcome,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::service::{ExactObserver, PhysicalExecutor, run_test as run};
use super::super::store::CheckedV1Store;
use crate::artifact::ManifestArtifact;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, RecoveryOriginStateV1, test_record,
};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PendingMergeAction, PendingMergeActionKind,
    PendingMergeExpectedResult,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn new_conflict_does_not_block_later_targets_before_awaiting_resolution() {
    let root = TempDir::new("merge-v1-service-conflict-sequence");
    let mut model = test_record();
    add_second_participant(&mut model);
    model.state = OperationState::Halted;
    let first = model.participants.get_mut("mem_a").unwrap();
    first.state = ParticipantState::Failed;
    first.error = Some(git_error("retry first"));
    model.participants.get_mut("mem_b").unwrap().state = ParticipantState::Unattempted;
    seed_open(&root, &model);
    let mut runtime = ConflictThenCompleteRuntime::default();

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
        V1ResponseDisposition::Stopped(OperationState::AwaitingResolution)
    );
    assert_eq!(runtime.preparations, ["mem_a", "mem_b"]);
    assert_eq!(runtime.executions, 1);
    let record = response.current().record();
    assert_eq!(record.state, OperationState::AwaitingResolution);
    assert_eq!(
        record.participants["mem_a"].state,
        ParticipantState::Conflicted
    );
    assert_eq!(
        record.participants["mem_b"].state,
        ParticipantState::UpToDate
    );
}

#[test]
fn ambiguous_halt_cause_commits_halt_reobserves_and_stops_in_recovery() {
    let root = TempDir::new("merge-v1-service-ambiguity-sequence");
    let mut model = test_record();
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Failed;
    row.error = Some(git_error("retained halt cause"));
    row.pending_action = Some(up_to_date_action(row));
    seed_open(&root, &model);
    let mut runtime = AmbiguousRuntime::default();

    let response = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(
        runtime.observed_states,
        [OperationState::Executing, OperationState::Halted]
    );
    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
    );
    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        response
            .current()
            .record()
            .recovery_context
            .as_ref()
            .unwrap()
            .origin_state,
        RecoveryOriginStateV1::Halted
    );
}

#[test]
fn physical_execution_rejects_an_intervening_record_rewrite() {
    let root = TempDir::new("merge-v1-service-execution-record-drift");
    let mut model = test_record();
    let row = model.participants.get_mut("mem_a").unwrap();
    row.pending_action = Some(up_to_date_action(row));
    seed_open(&root, &model);
    let mut runtime = RewriteRecordRuntime;

    let error = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .err()
    .unwrap();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(
        error.message,
        "checked v1 source bytes changed across physical execution"
    );
}

#[test]
fn resume_start_reconciles_completed_halted_owner_before_continuing() {
    let root = TempDir::new("merge-v1-service-resume-halted-owner");
    let mut model = test_record();
    model.state = OperationState::Halted;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Failed;
    row.error = Some(git_error("prior attempt failed"));
    row.pending_action = Some(up_to_date_action(row));
    seed_open(&root, &model);
    let mut runtime = CompletedHaltedRuntime;

    let error = run(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        V1LifecycleRequest::ResumeStart,
        &mut runtime,
    )
    .err()
    .unwrap();

    assert_eq!(error.message, "participants-complete checkpoint reached");
    let current = CheckedV1Store::default()
        .load_open(&root.path, "merge_1")
        .unwrap();
    assert_eq!(current.record().state, OperationState::Executing);
    let row = &current.record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::UpToDate);
    assert!(row.pending_action.is_none());
    assert!(row.error.is_none());
}

#[derive(Default)]
struct ConflictThenCompleteRuntime {
    preparations: Vec<String>,
    executions: usize,
}

impl ExactObserver for ConflictThenCompleteRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let fact = match request.kind() {
            ObservationKind::ParticipantPreparation { member_id } => {
                self.preparations.push(member_id.clone());
                prepared(current, member_id, member_id == "mem_a")?
            }
            ObservationKind::ParticipantAction { member_id }
                if member_id == "mem_a" && self.executions == 0 =>
            {
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
                completed_participant(current, member_id, member_id == "mem_a")?
            }
            kind => panic!("unexpected sequence observation: {kind:?}"),
        };
        BoundExactObservation::for_test(current, request, fact)
    }
}

impl PhysicalExecutor for ConflictThenCompleteRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        assert!(matches!(
            action,
            PhysicalActionKind::Participant { member_id, action }
                if member_id == "mem_a" && action.kind == PendingMergeActionKind::TrueMerge
        ));
        self.executions += 1;
        ExecutionDiagnostic::Success
    }
}

#[derive(Default)]
struct AmbiguousRuntime {
    observed_states: Vec<OperationState>,
}

impl ExactObserver for AmbiguousRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let origin = match current.record().state {
            OperationState::Executing => RecoveryOriginStateV1::Executing,
            OperationState::Halted => RecoveryOriginStateV1::Halted,
            state => panic!("unexpected ambiguity state: {state:?}"),
        };
        self.observed_states.push(current.record().state);
        let proof = BoundAmbiguityEvidence::for_test(
            current,
            "@operation",
            "enter_recovery",
            "ambiguous",
            origin,
        )?;
        BoundExactObservation::for_test(current, request, ExactObservationFact::Ambiguous(proof))
    }
}

impl PhysicalExecutor for AmbiguousRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        panic!("ambiguous observation must not execute")
    }
}

struct RewriteRecordRuntime;

impl ExactObserver for RewriteRecordRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let ObservationKind::ParticipantAction { member_id } = request.kind() else {
            panic!("record rewrite fixture selected another owner")
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

impl PhysicalExecutor for RewriteRecordRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let mut changed = current.record().clone();
        changed.source_ref = "topic-drift".into();
        fs::write(
            current.location().path(),
            serde_yaml::to_string(&changed).unwrap(),
        )
        .unwrap();
        ExecutionDiagnostic::Success
    }
}

struct CompletedHaltedRuntime;

impl ExactObserver for CompletedHaltedRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        match request.kind() {
            ObservationKind::ParticipantAction { member_id } => BoundExactObservation::for_test(
                current,
                request,
                completed_participant(current, member_id, false)?,
            ),
            ObservationKind::ParticipantsComplete => Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "participants-complete checkpoint reached",
            )),
            kind => panic!("unexpected restart observation: {kind:?}"),
        }
    }
}

impl PhysicalExecutor for CompletedHaltedRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        panic!("completed retained owner must not execute")
    }
}

fn prepared(
    current: &StoredV1Record,
    member_id: &str,
    conflicts: bool,
) -> ModelResult<ExactObservationFact> {
    let mut row = current.record().participants[member_id].clone();
    row.pending_action = Some(if conflicts {
        true_merge_action(&row)
    } else {
        up_to_date_action(&row)
    });
    let proof = PreparedParticipantAction::for_test(
        current,
        member_id,
        "prepare_participant",
        "prepared",
        ParticipantActionPayload {
            member_id: member_id.into(),
            row,
        },
    )?;
    Ok(ExactObservationFact::Completed(
        CompletedObservation::Participant(ParticipantObservation::Prepared(Box::new(proof))),
    ))
}

fn completed_participant(
    current: &StoredV1Record,
    member_id: &str,
    conflicted: bool,
) -> ModelResult<ExactObservationFact> {
    let mut row = current.record().participants[member_id].clone();
    row.state = if conflicted {
        ParticipantState::Conflicted
    } else {
        ParticipantState::UpToDate
    };
    row.resulting_commit = (!conflicted).then(|| row.before_commit.clone());
    row.expected_merge_head = conflicted.then(|| row.source_commit.clone());
    row.conflict_paths = conflicted
        .then(|| "conflict.txt".into())
        .into_iter()
        .collect();
    row.error = None;
    row.pending_action = None;
    let proof = VerifiedParticipantOutcome::for_test(
        current,
        member_id,
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: member_id.into(),
            row,
        },
    )?;
    Ok(ExactObservationFact::Completed(
        CompletedObservation::Participant(ParticipantObservation::Outcome(
            Box::new(proof),
            EntryFact::None,
        )),
    ))
}

fn up_to_date_action(
    row: &crate::workspace_ops::merge::MergeParticipantRecord,
) -> PendingMergeAction {
    action(
        row,
        PendingMergeActionKind::VerifyUpToDate,
        PendingMergeExpectedResult::Unchanged,
    )
}

fn true_merge_action(
    row: &crate::workspace_ops::merge::MergeParticipantRecord,
) -> PendingMergeAction {
    action(
        row,
        PendingMergeActionKind::TrueMerge,
        PendingMergeExpectedResult::ExpectedConflict,
    )
}

fn action(
    row: &crate::workspace_ops::merge::MergeParticipantRecord,
    kind: PendingMergeActionKind,
    expected_result: PendingMergeExpectedResult,
) -> PendingMergeAction {
    PendingMergeAction {
        kind,
        target_branch: row.target_branch.clone(),
        before_commit: row.before_commit.clone(),
        source_commit: row.source_commit.clone(),
        commit_message: row.commit_message.clone(),
        expected_result: Some(expected_result),
        commit_spec: None,
        extensions: BTreeMap::new(),
    }
}

fn add_second_participant(model: &mut MergeOperationRecordV1) {
    let mut manifest =
        ManifestArtifact::from_yaml(model.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    let mut member = manifest.members[0].clone();
    member.id = "mem_b".into();
    member.path = "members/b".into();
    member.source_id = "src_b".into();
    manifest.members.push(member);
    let manifest = manifest.to_yaml().unwrap();
    model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    model.baseline.manifest_yaml = Some(manifest);
    let mut second = model.participants["mem_a"].clone();
    second.path = "members/b".into();
    model.participants.insert("mem_b".into(), second);
    model.selected_targets.push("mem_b".into());
}

fn git_error(message: &str) -> MergeRecordError {
    MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
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
