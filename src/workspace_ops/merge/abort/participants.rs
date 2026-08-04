use super::{
    super::{
        MergeOperationRecord, MergeStore,
        participant_semantics::rollback::{RollbackGitAction, participant_rollback_decision},
    },
    preflight::AbortPreflight,
    runtime::AbortRuntime,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;
use std::path::Path;

pub(super) fn rollback_participants<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    preflight: &AbortPreflight,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for target_id in record.selected_targets.clone().into_iter().rev() {
        let (prior, next) = {
            let participant = record.participants.get(&target_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("merge record is missing participant '{target_id}'"),
                )
            })?;
            let path =
                super::super::status::validated_participant_path(root, &target_id, participant)?;
            let prior = participant.state;
            let decision =
                participant_rollback_decision(prior, preflight.no_op_targets.contains(&target_id));
            let Some(next) = decision.terminal_state else {
                continue;
            };
            emitter.member_started(&target_id, &participant.path);
            match decision.git_action {
                RollbackGitAction::None => {}
                RollbackGitAction::AbortConflict => runtime.abort_merge(
                    &path,
                    &participant.before_commit,
                    participant
                        .expected_merge_head
                        .as_deref()
                        .unwrap_or(&participant.source_commit),
                )?,
                RollbackGitAction::ResetIntegrated => runtime.reset_branch(
                    &path,
                    &participant.target_branch,
                    participant.resulting_commit.as_deref().ok_or_else(|| {
                        ModelError::new(
                            ErrorCode::MergeRecordUnreadable,
                            format!("merge participant '{target_id}' has no resulting commit"),
                        )
                    })?,
                    &participant.before_commit,
                )?,
            }
            (prior, next)
        };
        record.participants.get_mut(&target_id).unwrap().state = prior.transition(next)?;
        super::super::persist_merge_record(store, root, record, emitter)?;
        super::super::emit_merge_member_finished(emitter, record, &target_id)?;
    }
    Ok(())
}
