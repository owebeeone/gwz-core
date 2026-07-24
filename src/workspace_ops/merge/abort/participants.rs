use super::{
    super::{MergeOperationRecord, MergeStore, ParticipantState},
    preflight::AbortPreflight,
    runtime::AbortRuntime,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;
use crate::workspace::MemberPath;
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
            let path = root.join(MemberPath::parse(&participant.path)?.as_str());
            let prior = participant.state;
            if matches!(
                prior,
                ParticipantState::Aborted | ParticipantState::RolledBack
            ) {
                continue;
            }
            emitter.member_started(&target_id, &participant.path);
            match (preflight.no_op_targets.contains(&target_id), prior) {
                (true, _) => {}
                (false, ParticipantState::Conflicted) => runtime.abort_merge(
                    &path,
                    &participant.before_commit,
                    participant
                        .expected_merge_head
                        .as_deref()
                        .unwrap_or(&participant.source_commit),
                )?,
                (
                    false,
                    ParticipantState::FastForwarded
                    | ParticipantState::Merged
                    | ParticipantState::Continued,
                ) => runtime.reset_branch(
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
                (
                    false,
                    ParticipantState::Planned
                    | ParticipantState::UpToDate
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted,
                ) => {}
                (false, ParticipantState::Aborted | ParticipantState::RolledBack) => unreachable!(),
            }
            let next = if matches!(
                prior,
                ParticipantState::FastForwarded
                    | ParticipantState::Merged
                    | ParticipantState::Continued
            ) {
                ParticipantState::RolledBack
            } else {
                ParticipantState::Aborted
            };
            (prior, next)
        };
        record.participants.get_mut(&target_id).unwrap().state = prior.transition(next)?;
        super::super::persist_merge_record(store, root, record, emitter)?;
        super::super::emit_merge_member_finished(emitter, record, &target_id)?;
    }
    Ok(())
}
