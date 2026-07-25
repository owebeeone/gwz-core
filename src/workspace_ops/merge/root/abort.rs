use super::super::{MergeStatusSnapshot, MergeTargetKind};
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(in crate::workspace_ops::merge) fn normalize_evidence_observation(
    snapshot: &mut MergeStatusSnapshot,
) -> ModelResult<()> {
    let participant = snapshot.record.participants.get("@root").ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a durable root participant",
        )
    })?;
    if participant.target_kind != MergeTargetKind::Root || participant.path != "." {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence participant identity is inconsistent",
        ));
    }
    let observation = snapshot.participants.get_mut("@root").ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a root status observation",
        )
    })?;
    observation.live_commit = participant.resulting_commit.clone();
    observation.conflict_paths.clear();
    observation.drift.clear();
    observation.abort_eligibility.eligible = true;
    observation.abort_eligibility.blockers.clear();
    Ok(())
}
