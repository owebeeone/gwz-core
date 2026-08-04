use std::path::Path;

use super::super::{
    FileMergeStore, MergeStore, MergeTargetKind, discover_open_before_manifest,
    participant_semantics,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Central pre-dispatch guard used by synchronous drivers. Recovery discovery
/// intentionally precedes manifest parsing so an invalid in-flight root merge
/// cannot make the gate disappear.
pub fn enforce_workspace_open_merge_gate(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<()> {
    if command.gate_decision() == crate::operation::OpenMergeGateDecision::NotGated {
        return Ok(());
    }
    let store = FileMergeStore;
    let open = if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        store.discover_open(Path::new(root))?
    } else {
        discover_open_before_manifest(&store, start)?.map(|recovery| recovery.record)
    };
    crate::operation::enforce_open_merge_gate(
        open.as_ref().map(|record| record.merge_id.as_str()),
        command,
    )
}

pub(crate) fn enforce_open_merge_stage_targets(
    root: &Path,
    targets: &[crate::workspace_ops::StageTarget],
) -> ModelResult<()> {
    let store = FileMergeStore;
    let Some(record) = store.discover_open(root)? else {
        return Ok(());
    };
    let allowed = record
        .participants
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
            record.merge_id
        ),
    ))
}
