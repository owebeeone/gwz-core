use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::{
    MergeExecutionMode, ParticipantState, PendingCommitSpec, PendingGitSignature,
    PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};
use super::super::MergeOperationRecordV1;

pub(crate) fn validate_v1_actions(record: &MergeOperationRecordV1) -> ModelResult<()> {
    for (target_id, participant) in &record.participants {
        let Some(pending) = participant.pending_action.as_ref() else {
            continue;
        };
        validate_intent(record, target_id, participant, pending)?;
        validate_matrix(record, target_id, participant.state, pending)?;
        if let Some(spec) = pending.commit_spec.as_ref() {
            validate_commit_spec(record, target_id, spec)?;
        }
    }
    Ok(())
}

fn validate_intent(
    record: &MergeOperationRecordV1,
    target_id: &str,
    participant: &super::super::super::MergeParticipantRecord,
    pending: &PendingMergeAction,
) -> ModelResult<()> {
    if crate::workspace_ops::merge::integration::decode_for_participant(pending, participant)
        .is_err()
    {
        return Err(action_error(
            record,
            target_id,
            "pending integration intent does not equal the frozen participant",
        ));
    }
    Ok(())
}

fn validate_matrix(
    record: &MergeOperationRecordV1,
    target_id: &str,
    state: ParticipantState,
    pending: &PendingMergeAction,
) -> ModelResult<()> {
    use PendingMergeActionKind as Kind;
    use PendingMergeExpectedResult as ResultKind;

    let exact_shape = matches!(
        (
            pending.kind,
            pending.expected_result,
            pending.commit_spec.as_ref()
        ),
        (Kind::VerifyUpToDate, Some(ResultKind::Unchanged), None)
            | (Kind::FastForward, Some(ResultKind::FastForward), None)
            | (Kind::TrueMerge, Some(ResultKind::ExpectedConflict), None)
            | (Kind::TrueMerge, Some(ResultKind::Commit), Some(_))
            | (Kind::ResolveConflict, Some(ResultKind::Commit), Some(_))
    );
    let mode_allows = match record.mode {
        MergeExecutionMode::Normal => true,
        MergeExecutionMode::FfOnly => {
            matches!(pending.kind, Kind::VerifyUpToDate | Kind::FastForward)
        }
        MergeExecutionMode::NoFf => !matches!(pending.kind, Kind::FastForward),
    };
    let state_allows = match pending.kind {
        Kind::ResolveConflict => state == ParticipantState::Conflicted,
        Kind::VerifyUpToDate | Kind::FastForward | Kind::TrueMerge => matches!(
            state,
            ParticipantState::Planned | ParticipantState::Failed | ParticipantState::Unattempted
        ),
    };
    if !exact_shape || !mode_allows || !state_allows {
        return Err(action_error(
            record,
            target_id,
            "pending action violates the frozen mode, result, commit-spec, or state matrix",
        ));
    }
    Ok(())
}

fn validate_commit_spec(
    record: &MergeOperationRecordV1,
    target_id: &str,
    spec: &PendingCommitSpec,
) -> ModelResult<()> {
    let valid =
        is_oid(&spec.tree_oid) && valid_signature(&spec.author) && valid_signature(&spec.committer);
    if !valid {
        return Err(action_error(
            record,
            target_id,
            "pending commit specification is not canonical",
        ));
    }
    Ok(())
}

fn valid_signature(signature: &PendingGitSignature) -> bool {
    !signature.name.trim().is_empty()
        && !signature.email.trim().is_empty()
        && !signature.name.contains(['\0', '\n', '\r'])
        && !signature.email.contains(['\0', '\n', '\r'])
        && (-1_440..=1_440).contains(&signature.timezone_offset_minutes)
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn action_error(record: &MergeOperationRecordV1, target_id: &str, reason: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!(
            "merge record '{}' participant '{target_id}' action is invalid: {reason}",
            record.merge_id
        ),
    )
}
