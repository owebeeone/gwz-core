use super::super::transition::{EffectKind, TransitionEffect};
use super::fixtures::up_to_date_action;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::merge::{MergeRecordError, ParticipantState};

#[test]
fn participant_outcomes_retire_a_prior_retry_error_for_every_result_shape() {
    let effect =
        TransitionEffect::participant_for_test(EffectKind::RecordParticipantOutcome, "mem_a");
    for state in [
        ParticipantState::UpToDate,
        ParticipantState::FastForwarded,
        ParticipantState::Merged,
        ParticipantState::Conflicted,
        ParticipantState::Continued,
    ] {
        let mut old = record();
        let row = old.participants.get_mut("mem_a").unwrap();
        row.state = ParticipantState::Failed;
        row.error = Some(MergeRecordError {
            code: ErrorCode::GitCommandFailed,
            message: "prior retry failed".into(),
            detail: None,
        });
        row.pending_action = Some(up_to_date_action());
        let mut next = old.clone();
        let row = next.participants.get_mut("mem_a").unwrap();
        row.state = state;
        row.resulting_commit = match state {
            ParticipantState::Conflicted => None,
            ParticipantState::UpToDate => Some(row.before_commit.clone()),
            _ => Some("d".repeat(40)),
        };
        row.expected_merge_head =
            (state == ParticipantState::Conflicted).then(|| row.source_commit.clone());
        row.conflict_paths = (state == ParticipantState::Conflicted)
            .then(|| "conflict.txt".into())
            .into_iter()
            .collect();
        row.error = None;
        row.pending_action = None;

        effect.verify_known_diff(&old, &next).unwrap();
    }
}

#[test]
fn repeated_failed_preparation_replaces_only_error_and_operation_state() {
    let mut old = record();
    old.participants.get_mut("mem_a").unwrap().state = ParticipantState::Failed;
    old.participants.get_mut("mem_a").unwrap().error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "first failure".into(),
        detail: None,
    });
    let mut next = old.clone();
    next.state = crate::workspace_ops::merge::OperationState::Halted;
    next.participants.get_mut("mem_a").unwrap().error = Some(MergeRecordError {
        code: ErrorCode::MergeValidationFailed,
        message: "replacement failure".into(),
        detail: Some("retry remains safe".into()),
    });

    TransitionEffect::failure_for_test(EffectKind::RecordPreparationFailureAndHalt, "mem_a", &[])
        .verify_known_diff(&old, &next)
        .unwrap();
    TransitionEffect::failure_for_test(EffectKind::RecordOwnedRetryFailureAndHalt, "mem_a", &[])
        .verify_known_diff(&old, &next)
        .unwrap();
}
