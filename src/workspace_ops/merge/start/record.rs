#[cfg(test)]
use super::super::PendingCommitSpec;
use super::super::integration::{IntegrationIntent, PreparedIntegration};
use super::super::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeOperationRecord, MergeParticipantPlan,
    MergeParticipantRecord, MergeRecordError, OperationState, ParticipantState,
};
use super::prepared::{PreparedAction, Row};
use crate::MergeParticipantState as PState;
use crate::artifact;
#[cfg(test)]
use crate::git::GitPreparedMerge;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::runtime::clock::Clock;
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn freeze_merge_messages(
    participants: &mut [MergeParticipantPlan],
    source_ref: &str,
    merge_id: &str,
    context: &OperationContext,
) {
    for participant in participants {
        participant.commit_message = super::super::integration::final_member_commit_message(
            None,
            source_ref,
            &participant.target_branch,
            merge_id,
            &context.operation_id,
        )
        .expect("internally generated merge message is always valid");
    }
}

pub(super) fn create_record<C: Clock>(
    root: &Path,
    plan: &super::super::MergePlan,
    merge_id: &str,
    clock: &C,
    context: &OperationContext,
) -> ModelResult<MergeOperationRecord> {
    let manifest = artifact::read_manifest(root)?;
    let participants = plan
        .participants
        .iter()
        .map(|participant| {
            (
                participant.target_id.clone(),
                MergeParticipantRecord {
                    path: participant.path.clone(),
                    target_kind: participant.target_kind,
                    target_branch: participant.target_branch.clone(),
                    before_commit: participant.before_commit.clone(),
                    source_commit: participant.source_commit.clone(),
                    commit_message: participant.commit_message.clone(),
                    state: ParticipantState::Planned,
                    resulting_commit: None,
                    expected_merge_head: None,
                    conflict_paths: Vec::new(),
                    conflict_snapshot: Vec::new(),
                    error: None,
                    pending_action: None,
                    preservation: Vec::new(),
                    drift: Vec::new(),
                    extensions: BTreeMap::new(),
                },
            )
        })
        .collect();
    Ok(MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: crate::VERSION.to_owned(),
        workspace_id: manifest.workspace.id,
        merge_id: merge_id.to_owned(),
        operation_id: context.operation_id.clone(),
        state: OperationState::Executing,
        source_ref: plan.source_ref.clone(),
        mode: plan.mode,
        created_at: clock.now_ms().0.to_string(),
        baseline: plan.baseline.clone(),
        selected_targets: plan
            .participants
            .iter()
            .map(|participant| participant.target_id.clone())
            .collect(),
        participants,
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    })
}

pub(super) fn set_pending_action(
    record: &mut MergeOperationRecord,
    plan: &MergeParticipantPlan,
    prepared: &PreparedAction,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&plan.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", plan.target_id),
            )
        })?;
    let integration = PreparedIntegration::from_merge(
        IntegrationIntent::from_plan(plan),
        prepared.kind,
        &prepared.result,
    )
    .map_err(|reason| ModelError::new(ErrorCode::InternalError, reason))?;
    participant.pending_action = Some(integration.to_pending());
    Ok(())
}

#[cfg(test)]
pub(super) fn pending_commit_spec(result: &GitPreparedMerge) -> Option<PendingCommitSpec> {
    match result {
        GitPreparedMerge::Commit(spec) => {
            Some(super::super::integration::pending_commit_spec(spec))
        }
        _ => None,
    }
}

pub(super) fn apply_row(
    record: &mut MergeOperationRecord,
    plan: &MergeParticipantPlan,
    row: &Row<'_>,
    error: Option<&ModelError>,
    conflict_snapshot: Vec<super::super::ConflictFileEvidence>,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&plan.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", plan.target_id),
            )
        })?;
    let next = match row.state {
        PState::UpToDate => ParticipantState::UpToDate,
        PState::FastForwarded => ParticipantState::FastForwarded,
        PState::Merged => ParticipantState::Merged,
        PState::Conflicted => ParticipantState::Conflicted,
        PState::Failed => ParticipantState::Failed,
        PState::Unattempted => ParticipantState::Unattempted,
        _ => {
            return Err(ModelError::new(
                ErrorCode::InternalError,
                "start produced an invalid durable participant state",
            ));
        }
    };
    participant.state = participant.state.transition(next)?;
    participant.resulting_commit.clone_from(&row.oid);
    participant.conflict_paths.clone_from(&row.paths);
    participant.conflict_snapshot = conflict_snapshot;
    participant.expected_merge_head =
        (next == ParticipantState::Conflicted).then(|| plan.source_commit.clone());
    participant.error = error.map(|error| MergeRecordError {
        code: error.code,
        message: error.message.clone(),
        detail: None,
    });
    Ok(())
}
