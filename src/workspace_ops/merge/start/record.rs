//! Freezing one accepted plan into a durable v1 record.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** `merge/start/` was the v0 engine's
//! start half; `create_record` and `freeze_merge_messages` were the shared
//! part of it and are what remains. The record this builds is v1 — the only
//! version this binary writes — so the v0-shaped intermediate the v1 lifecycle
//! used to lift (`created_v1_record`) is gone: creation states the v1-only
//! fields absent directly, which is what it always meant.

use super::super::integration::{IntegrationIntent, PreparedIntegration};
use super::super::model::v1::MergeOperationRecordV1;
use super::super::{
    MergeParticipantPlan, MergeParticipantRecord, OperationState, ParticipantState,
    RequestedSemantics, creation_envelope, select_record_version,
};
use crate::artifact;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::runtime::clock::Clock;
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn freeze_merge_messages(
    participants: &mut [MergeParticipantPlan],
    custom_body: Option<&str>,
    source_ref: &str,
    merge_id: &str,
    context: &OperationContext,
) -> ModelResult<()> {
    for participant in participants {
        participant.commit_message = super::super::integration::final_member_commit_message(
            custom_body,
            source_ref,
            &participant.target_branch,
            merge_id,
            &context.operation_id,
        )?;
    }
    Ok(())
}

/// Create the durable record for one accepted start, at the version the
/// contract-§2 writer floor selects.
///
/// The version is chosen by `select_record_version` —
/// `max(active_writer_floor, highest requested semantic version)` — and
/// unsupported requested semantics (A2-A4) reject here, before any record
/// exists. With `ACTIVE_WRITER_FLOOR` at `V1` that selection has one
/// answer for every servable request, and the envelope it names is v1's.
pub(super) fn create_record<C: Clock>(
    root: &Path,
    plan: &super::super::MergePlan,
    merge_id: &str,
    clock: &C,
    context: &OperationContext,
) -> ModelResult<MergeOperationRecordV1> {
    let (schema, record_schema_version) = creation_envelope(select_record_version(
        RequestedSemantics::from_mode(plan.mode),
    )?);
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
    Ok(MergeOperationRecordV1 {
        schema: schema.to_owned(),
        record_schema_version,
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
        // The v1-only fields. A record that has not started executing carries
        // none of them: the accepted workspace, the recovery context, the two
        // pending journals and the preservation/publication handoff are all
        // written by the lifecycle, never by creation.
        accepted_workspace: None,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        preservation_publication_handoff: None,
        extensions: BTreeMap::new(),
    })
}
