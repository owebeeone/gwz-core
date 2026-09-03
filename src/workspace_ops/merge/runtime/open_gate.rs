use std::path::Path;

use super::super::{
    MergeTargetKind, classify_open_record, discover_open_envelope_before_manifest,
    discover_open_record, participant_semantics,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Central pre-dispatch guard used by synchronous drivers. Recovery discovery
/// intentionally precedes manifest parsing so an invalid in-flight root merge
/// cannot make the gate disappear.
///
/// A1: discovery is by ENVELOPE, not through the v0 store. `discover_open`
/// reads with the v0-only decoder, so an open v1 record answered
/// `UnsupportedRecordVersion` here and this gate — which owns the only message
/// that names `merge status`, `merge continue` and `merge abort` — was never
/// reached. `merge start` already classified first (`start.rs`); every other
/// gated command now does the same.
pub fn enforce_workspace_open_merge_gate(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<()> {
    if command.gate_decision() == crate::operation::OpenMergeGateDecision::NotGated {
        return Ok(());
    }
    let open = if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        classify_open_record(Path::new(root))?
    } else {
        discover_open_envelope_before_manifest(start)?
    };
    crate::operation::enforce_open_merge_gate(
        open.as_ref().map(|envelope| envelope.merge_id.as_str()),
        command,
    )
}

/// `add`'s open-merge scope check, over an open record of either version.
pub(crate) fn enforce_open_merge_stage_targets(
    root: &Path,
    targets: &[crate::workspace_ops::StageTarget],
) -> ModelResult<()> {
    let Some(open) = discover_open_record(root)? else {
        return Ok(());
    };
    let record = open.view();
    let allowed = record
        .participants()
        .values()
        .filter(|participant| {
            participant_semantics::status::status_policy(participant.state).conflict_role
                == participant_semantics::status::ConflictRole::NativeMerge
        })
        .map(|participant| match participant.target_kind {
            MergeTargetKind::Member => Some(participant.path.as_str()),
            MergeTargetKind::Root => None,
        })
        .collect::<Vec<_>>();
    if targets
        .iter()
        .all(|target| allowed.contains(&target.member_path.as_deref()))
    {
        return Ok(());
    }
    Err(ModelError::new(
        ErrorCode::OpenOperation,
        format!(
            "merge '{}' is open; add may target only its conflicted participants; \
             use merge status to inspect the allowed repositories",
            record.merge_id()
        ),
    ))
}
