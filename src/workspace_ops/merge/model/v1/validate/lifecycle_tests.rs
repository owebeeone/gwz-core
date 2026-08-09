use super::super::super::MergeRecordError;
use super::super::super::{
    OperationState, ParticipantState, PendingMergeActionKind, PendingMergeExpectedResult,
};
use super::super::{RecoveryContextV1, RecoveryOriginStateV1};
use super::tests::{oid, record};
use super::validate_v1_lifecycle;
use crate::model::ErrorCode;

fn set_state(record: &mut super::super::MergeOperationRecordV1, state: ParticipantState) {
    let participant = record.participants.get_mut("mem_a").unwrap();
    participant.state = state;
    participant.resulting_commit = match state {
        ParticipantState::UpToDate => Some(participant.before_commit.clone()),
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
        | ParticipantState::RolledBack => Some(oid('d')),
        _ => None,
    };
    participant.expected_merge_head =
        (state == ParticipantState::Conflicted).then(|| participant.source_commit.clone());
    participant.conflict_paths.clear();
    participant.conflict_snapshot.clear();
    participant.error = (state == ParticipantState::Failed).then(|| MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "failed".to_owned(),
        detail: None,
    });
}

#[test]
fn direct_operation_states_enforce_the_closed_participant_matrix() {
    let legal = [
        (OperationState::Executing, ParticipantState::Planned),
        (OperationState::Executing, ParticipantState::Failed),
        (
            OperationState::AwaitingResolution,
            ParticipantState::Conflicted,
        ),
        (OperationState::Halted, ParticipantState::Failed),
        (OperationState::Finalizing, ParticipantState::Merged),
        (OperationState::Preserving, ParticipantState::Unattempted),
        (OperationState::RollingBack, ParticipantState::Merged),
        (OperationState::RollingBack, ParticipantState::RolledBack),
        (OperationState::Completed, ParticipantState::UpToDate),
        (OperationState::Aborted, ParticipantState::Aborted),
        (OperationState::Aborted, ParticipantState::RolledBack),
    ];
    for (operation, participant) in legal {
        let mut case = record();
        case.state = operation;
        set_state(&mut case, participant);
        validate_v1_lifecycle(&case)
            .unwrap_or_else(|error| panic!("{operation:?}/{participant:?}: {error:?}"));
    }

    let illegal = [
        (OperationState::Executing, ParticipantState::Aborted),
        (
            OperationState::AwaitingResolution,
            ParticipantState::Planned,
        ),
        (OperationState::Halted, ParticipantState::Planned),
        (OperationState::Finalizing, ParticipantState::Conflicted),
        (OperationState::Preserving, ParticipantState::RolledBack),
        (OperationState::Completed, ParticipantState::Failed),
        (OperationState::Aborted, ParticipantState::Unattempted),
    ];
    for (operation, participant) in illegal {
        let mut case = record();
        case.state = operation;
        set_state(&mut case, participant);
        assert!(
            validate_v1_lifecycle(&case).is_err(),
            "{operation:?}/{participant:?} unexpectedly passed"
        );
    }
}

#[test]
fn recovery_uses_the_recorded_origin_matrix() {
    let cases = [
        (RecoveryOriginStateV1::Executing, ParticipantState::Planned),
        (
            RecoveryOriginStateV1::AwaitingResolution,
            ParticipantState::Conflicted,
        ),
        (RecoveryOriginStateV1::Halted, ParticipantState::Failed),
        (RecoveryOriginStateV1::Finalizing, ParticipantState::Merged),
        (
            RecoveryOriginStateV1::Preserving,
            ParticipantState::Unattempted,
        ),
        (
            RecoveryOriginStateV1::RollingBack,
            ParticipantState::RolledBack,
        ),
    ];
    for (origin_state, participant) in cases {
        let mut case = record();
        case.state = OperationState::RecoveryRequired;
        case.recovery_context = Some(RecoveryContextV1 { origin_state });
        set_state(&mut case, participant);
        validate_v1_lifecycle(&case)
            .unwrap_or_else(|error| panic!("{origin_state:?}/{participant:?}: {error:?}"));
    }

    let mut mismatch = record();
    mismatch.state = OperationState::RecoveryRequired;
    mismatch.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Finalizing,
    });
    assert_eq!(
        validate_v1_lifecycle(&mismatch).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );
}

#[test]
fn forward_actions_exist_only_in_execution_or_halt_windows() {
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().pending_action =
        Some(super::action_tests::pending(
            PendingMergeActionKind::FastForward,
            PendingMergeExpectedResult::FastForward,
            false,
        ));
    validate_v1_lifecycle(&case).unwrap();

    case.state = OperationState::Finalizing;
    set_state(&mut case, ParticipantState::Merged);
    assert_eq!(
        validate_v1_lifecycle(&case).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );

    case.state = OperationState::Halted;
    set_state(&mut case, ParticipantState::Failed);
    validate_v1_lifecycle(&case).unwrap();

    case.state = OperationState::Preserving;
    assert_eq!(
        validate_v1_lifecycle(&case).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );

    case.state = OperationState::RecoveryRequired;
    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Halted,
    });
    validate_v1_lifecycle(&case).unwrap();

    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Preserving,
    });
    assert_eq!(
        validate_v1_lifecycle(&case).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );
}

#[test]
fn at_most_one_forward_action_is_legal_record_wide() {
    let mut case = record();
    let pending = super::action_tests::pending(
        PendingMergeActionKind::FastForward,
        PendingMergeExpectedResult::FastForward,
        false,
    );
    case.participants.get_mut("mem_a").unwrap().pending_action = Some(pending.clone());
    let mut second = case.participants["mem_a"].clone();
    second.path = "members/b".to_owned();
    second.pending_action = Some(pending);
    case.selected_targets.push("mem_b".to_owned());
    case.participants.insert("mem_b".to_owned(), second);

    assert_eq!(
        validate_v1_lifecycle(&case).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );
}

#[test]
fn participant_result_conflict_and_error_shapes_are_closed() {
    let mut case = record();
    case.state = OperationState::Finalizing;
    set_state(&mut case, ParticipantState::Merged);
    case.participants.get_mut("mem_a").unwrap().resulting_commit = None;
    assert!(validate_v1_lifecycle(&case).is_err());

    let mut case = record();
    set_state(&mut case, ParticipantState::Failed);
    case.participants.get_mut("mem_a").unwrap().error = None;
    assert!(validate_v1_lifecycle(&case).is_err());

    let mut case = record();
    case.participants
        .get_mut("mem_a")
        .unwrap()
        .expected_merge_head = Some(oid('b'));
    assert!(validate_v1_lifecycle(&case).is_err());
}
