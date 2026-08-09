use super::super::authority::verify_participant_action;
use super::super::checked::StoredV1Record;
use crate::git::{GitBackend, GitIntegrateResult, GitPreparedMerge};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::integration::{PreparedIntegrationAction, decode_for_participant};
use crate::workspace_ops::merge::{MergeParticipantRecord, PendingMergeAction};

pub(super) fn participant<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    member_id: &str,
    action: &PendingMergeAction,
) -> ModelResult<()> {
    verify_participant_action(backend, current, member_id, action)?;
    let row = current
        .record()
        .participants
        .get(member_id)
        .ok_or_else(|| recovery("participant action owner is missing"))?;
    let path = crate::workspace_ops::merge::status::validated_participant_path(
        current.location().root(),
        member_id,
        row,
    )?;
    let prepared = decode_for_participant(action, row)
        .map_err(|reason| member(recovery(reason), member_id, &row.path))?;
    match prepared.action {
        PreparedIntegrationAction::ResolveConflict(commit) => {
            let merge_head = row
                .expected_merge_head
                .as_deref()
                .unwrap_or(&row.source_commit);
            backend.commit_prepared_merge_resolution_checked(
                &path,
                &row.target_branch,
                &row.before_commit,
                merge_head,
                &row.commit_message,
                &commit,
            )?;
        }
        value => {
            let prepared = match value {
                PreparedIntegrationAction::VerifyUpToDate => GitPreparedMerge::Unchanged,
                PreparedIntegrationAction::FastForward => GitPreparedMerge::FastForward,
                PreparedIntegrationAction::TrueMergeExpectedConflict => {
                    GitPreparedMerge::ExpectedConflict
                }
                PreparedIntegrationAction::TrueMergeCommit(commit) => {
                    GitPreparedMerge::Commit(commit)
                }
                PreparedIntegrationAction::ResolveConflict(_) => unreachable!(),
            };
            let result = backend.execute_prepared_merge_upstream_checked(
                &path,
                &row.target_branch,
                &row.before_commit,
                &row.source_commit,
                &row.commit_message,
                &prepared,
            )?;
            validate_result(row, &prepared, &result)?;
        }
    }
    Ok(())
}

fn validate_result(
    row: &MergeParticipantRecord,
    prepared: &GitPreparedMerge,
    result: &GitIntegrateResult,
) -> ModelResult<()> {
    let valid = match prepared {
        GitPreparedMerge::Unchanged => {
            result.conflicts.is_empty()
                && result.commit.as_deref() == Some(row.before_commit.as_str())
        }
        GitPreparedMerge::FastForward => {
            result.conflicts.is_empty()
                && result.commit.as_deref() == Some(row.source_commit.as_str())
        }
        GitPreparedMerge::ExpectedConflict => {
            !result.conflicts.is_empty() && result.commit.is_none()
        }
        GitPreparedMerge::Commit(_) => result.conflicts.is_empty() && result.commit.is_some(),
    };
    valid.then_some(()).ok_or_else(|| {
        recovery("participant backend returned a result outside the frozen action contract")
    })
}

fn member(mut error: ModelError, member_id: &str, path: &str) -> ModelError {
    if error.member_id.is_none() {
        error = error.with_member(member_id, path);
    }
    error
}

fn recovery(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail)
}
