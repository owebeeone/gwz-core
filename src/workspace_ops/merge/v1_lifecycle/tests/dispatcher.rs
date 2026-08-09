use std::collections::BTreeMap;

use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, BoundObservationRequest, CompletedObservation,
    ExactObservationFact, ExecutionDiagnostic, NotStartedObservation, ObservationKind,
    ParticipantActionPayload, ParticipantObservation, PhysicalActionKind,
    PreparedParticipantAction, ResolvedV1Action, V1LifecycleRequest, V1NextAction,
    V1ResponseDisposition, next_action, resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::prepare;
use super::fixtures::up_to_date_action;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{RecoveryOriginStateV1, test_record as record};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PendingCommitSpec, PendingGitSignature,
    PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn dispatcher_routes_status_and_open_work_without_consumer_branching() {
    let root = TempDir::new("merge-v1-dispatch-routing");
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();
    assert!(matches!(
        next_action(&current, V1LifecycleRequest::Status).unwrap(),
        V1NextAction::Respond(V1ResponseDisposition::Status)
    ));
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("executing record did not request participant preparation")
    };
    assert_eq!(
        request.kind(),
        &ObservationKind::ParticipantPreparation {
            member_id: "mem_a".into(),
        }
    );
    assert!(matches!(
        next_action(&current, V1LifecycleRequest::ResumeStart).unwrap(),
        V1NextAction::Observe(_)
    ));
}

#[test]
fn completed_observation_is_the_only_authority_for_a_prepared_result() {
    let root = TempDir::new("merge-v1-dispatch-completed");
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("planned participant did not request preparation")
    };
    let mut row = current.record().participants["mem_a"].clone();
    row.pending_action = Some(up_to_date_action());
    let prepared = PreparedParticipantAction::for_test(
        &current,
        "mem_a",
        "prepare_participant",
        "prepared",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::Prepared(Box::new(prepared)),
        )),
    )
    .unwrap();
    let ResolvedV1Action::Apply(transition) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("completed preparation did not return its bound transition")
    };
    assert!(prepare(&lease, &current, transition).is_ok());
}

#[test]
fn ambiguity_halts_retained_failure_before_recording_literal_recovery_origin() {
    let root = TempDir::new("merge-v1-dispatch-ambiguity-order");
    let mut executing = record();
    let participant = executing.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Failed;
    participant.error = Some(git_error("retained failure"));
    participant.pending_action = Some(up_to_date_action());
    let current = StoredV1Record::for_test(&root.path, executing).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("persisted participant owner was not reconciled")
    };
    let ambiguity = BoundAmbiguityEvidence::for_test(
        &current,
        "@operation",
        "enter_recovery",
        "ambiguous",
        RecoveryOriginStateV1::Executing,
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Ambiguous(ambiguity),
    )
    .unwrap();
    let ResolvedV1Action::Apply(halt) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("ambiguity did not first preserve the literal halted history")
    };
    let halted = prepare(&lease, &current, halt).unwrap();
    assert_eq!(halted.next().state, OperationState::Halted);

    let halted = StoredV1Record::for_test(&root.path, halted.next().clone()).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&halted, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("halted persisted owner was not reobserved")
    };
    let ambiguity = BoundAmbiguityEvidence::for_test(
        &halted,
        "@operation",
        "enter_recovery",
        "ambiguous",
        RecoveryOriginStateV1::Halted,
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &halted,
        &request,
        ExactObservationFact::Ambiguous(ambiguity),
    )
    .unwrap();
    let ResolvedV1Action::Apply(recover) = resolve_observation(
        &halted,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("fresh halted ambiguity did not enter recovery")
    };
    let recovered = prepare(&lease, &halted, recover).unwrap();
    assert_eq!(recovered.next().state, OperationState::RecoveryRequired);
    assert_eq!(
        recovered
            .next()
            .recovery_context
            .as_ref()
            .unwrap()
            .origin_state,
        RecoveryOriginStateV1::Halted
    );
}

#[test]
fn dispatcher_responses_rejections_and_success_without_progress_are_closed() {
    let root = TempDir::new("merge-v1-dispatch-closed-results");
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();

    let V1NextAction::Reject(error) = next_action(&current, V1LifecycleRequest::Archive).unwrap()
    else {
        panic!("open archive request was not rejected")
    };
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);

    let terminal = V1ResponseDisposition::Terminal(OperationState::Completed);
    let V1ResponseDisposition::Terminal(state) = terminal else {
        unreachable!()
    };
    assert_eq!(state, OperationState::Completed);

    let mut halted_record = record();
    halted_record.state = OperationState::Halted;
    let halted_participant = halted_record.participants.get_mut("mem_a").unwrap();
    halted_participant.state = ParticipantState::Failed;
    halted_participant.error = Some(git_error("halt cause"));
    let halted = StoredV1Record::for_test(&root.path, halted_record).unwrap();
    let V1NextAction::Apply(resume) = next_action(&halted, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("halted continuation did not request the resume transition")
    };
    let _ = resume.effect_kind();

    let mut finalizing_record = record();
    finalizing_record.state = OperationState::Finalizing;
    let finalizing_participant = finalizing_record.participants.get_mut("mem_a").unwrap();
    finalizing_participant.state = ParticipantState::UpToDate;
    finalizing_participant.resulting_commit = Some(finalizing_participant.before_commit.clone());
    let finalizing = StoredV1Record::for_test(&root.path, finalizing_record).unwrap();
    assert!(matches!(
        next_action(&finalizing, V1LifecycleRequest::Archive).unwrap(),
        V1NextAction::Reject(_)
    ));

    let archive_request = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Archive,
        ObservationKind::Archive,
    )
    .unwrap();
    let archive_observation = BoundExactObservation::for_test(
        &current,
        &archive_request,
        ExactObservationFact::Completed(CompletedObservation::Archive),
    )
    .unwrap();
    let ResolvedV1Action::Respond(disposition) = resolve_observation(
        &current,
        V1LifecycleRequest::Archive,
        archive_request,
        archive_observation,
        None,
    )
    .unwrap() else {
        panic!("archive completion did not respond")
    };
    assert!(matches!(disposition, V1ResponseDisposition::ArchiveReady));

    let read_request = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Continue,
        ObservationKind::Acceptance,
    )
    .unwrap();
    let read_observation = BoundExactObservation::for_test(
        &current,
        &read_request,
        ExactObservationFact::Completed(CompletedObservation::Archive),
    )
    .unwrap();
    let ResolvedV1Action::Reject(error) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        read_request,
        read_observation,
        None,
    )
    .unwrap() else {
        panic!("read-only no-progress observation was not rejected")
    };
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);

    let mut pending = record();
    pending
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(up_to_date_action());
    let pending = StoredV1Record::for_test(&root.path, pending).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&pending, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("pending participant was not observed")
    };
    let observation = not_started(&pending, &request);
    let ResolvedV1Action::Execute(action) = resolve_observation(
        &pending,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("not-started physical action was not authorized")
    };
    let attempt = action
        .record_attempt(&pending, ExecutionDiagnostic::Success)
        .unwrap();
    let V1NextAction::Observe(request) =
        next_action(&pending, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("owner was not freshly reobserved")
    };
    let observation = not_started(&pending, &request);
    let ResolvedV1Action::Reject(error) = resolve_observation(
        &pending,
        V1LifecycleRequest::Continue,
        request,
        observation,
        Some(attempt),
    )
    .unwrap() else {
        panic!("success without observed progress was not rejected")
    };
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
}

#[test]
pub(super) fn failed_attempt_is_not_outcome_authority_and_halts_through_a_bound_batch() {
    let root = TempDir::new("merge-v1-dispatch-attempt");
    let mut pending = record();
    pending
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(up_to_date_action());
    let current = StoredV1Record::for_test(&root.path, pending).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();

    let V1NextAction::Observe(first_request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("pending owner was not observed")
    };
    let first_observation = not_started(&current, &first_request);
    let ResolvedV1Action::Execute(action) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        first_request,
        first_observation,
        None,
    )
    .unwrap() else {
        panic!("not-started owner was not authorized exactly once")
    };
    assert!(matches!(
        action.kind(),
        PhysicalActionKind::Participant { member_id, .. } if member_id == "mem_a"
    ));
    let attempt = action
        .record_attempt(
            &current,
            ExecutionDiagnostic::Failed {
                code: ErrorCode::GitCommandFailed,
                message: "git failed".into(),
                detail: Some("diagnostic only".into()),
            },
        )
        .unwrap();

    let V1NextAction::Observe(second_request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("pending owner was not reobserved")
    };
    let second_observation = not_started(&current, &second_request);
    let ResolvedV1Action::Apply(transition) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        second_request,
        second_observation,
        Some(attempt),
    )
    .unwrap() else {
        panic!("failed attempt did not produce a bound failure transition")
    };
    let rewrite = prepare(&lease, &current, transition).unwrap();
    assert_eq!(rewrite.next().state, OperationState::Halted);
    let participant = &rewrite.next().participants["mem_a"];
    assert_eq!(participant.state, ParticipantState::Failed);
    assert!(participant.pending_action.is_some());
    assert_eq!(
        participant.error.as_ref().unwrap().code,
        ErrorCode::GitCommandFailed
    );
}

#[test]
pub(super) fn failed_resolution_retains_the_authoritative_conflict_and_owner() {
    let root = TempDir::new("merge-v1-dispatch-resolution-attempt");
    let mut model = record();
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    row.pending_action = Some(resolve_action());
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();

    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("resolution owner was not observed")
    };
    let observation = not_started(&current, &request);
    let ResolvedV1Action::Execute(action) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("resolution was not authorized once")
    };
    let attempt = action
        .record_attempt(
            &current,
            ExecutionDiagnostic::Failed {
                code: ErrorCode::GitCommandFailed,
                message: "commit failed".into(),
                detail: None,
            },
        )
        .unwrap();
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("resolution owner was not reobserved")
    };
    let observation = not_started(&current, &request);
    let ResolvedV1Action::Apply(transition) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        Some(attempt),
    )
    .unwrap() else {
        panic!("resolution diagnostic did not produce the bound halt batch")
    };
    let rewrite = prepare(&lease, &current, transition).unwrap();
    let row = &rewrite.next().participants["mem_a"];
    assert_eq!(rewrite.next().state, OperationState::Halted);
    assert_eq!(row.state, ParticipantState::Conflicted);
    assert!(row.pending_action.is_some());
    assert_eq!(row.expected_merge_head, Some("b".repeat(40)));
}

pub(super) fn resolve_action() -> PendingMergeAction {
    let mut action = up_to_date_action();
    action.kind = PendingMergeActionKind::ResolveConflict;
    action.expected_result = Some(PendingMergeExpectedResult::Commit);
    action.commit_spec = Some(PendingCommitSpec {
        tree_oid: "c".repeat(40),
        author: signature("author"),
        committer: signature("committer"),
        extensions: BTreeMap::new(),
    });
    action
}

fn not_started(
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> BoundExactObservation {
    let ObservationKind::ParticipantAction { member_id } = request.kind() else {
        panic!("fixture requires a participant action")
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
    .unwrap()
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

fn git_error(message: &str) -> MergeRecordError {
    MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    }
}
