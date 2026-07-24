use super::{
    super::{
        MergeOperationRecord, MergeParticipantObservation, MergeParticipantRecord,
        MergeStatusSnapshot, OperationState, ParticipantDriftKind, ParticipantState,
        status::PendingActionReconciliation,
    },
    reconciliation::pending_reconciliation,
};
use crate::artifact;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Default)]
pub(super) struct AbortPreflight {
    pub(super) no_op_targets: BTreeSet<String>,
    pub(super) pending: BTreeMap<String, PendingActionReconciliation>,
}

pub(super) fn preflight(snapshot: &MergeStatusSnapshot) -> ModelResult<AbortPreflight> {
    if let Some(drift) = snapshot.operation_drift.first() {
        return Err(ModelError::new(ErrorCode::MergeDrift, &drift.message));
    }
    let mut preflight = AbortPreflight::default();
    for target_id in &snapshot.record.selected_targets {
        let observation = snapshot.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge status is missing participant '{target_id}'"),
            )
        })?;
        let participant = snapshot.record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "status participant is missing",
            )
        })?;
        if participant.pending_action.is_some() {
            let reconciliation = pending_reconciliation(target_id, participant, observation)?;
            preflight.pending.insert(target_id.clone(), reconciliation);
            continue;
        }
        if !observation.abort_eligibility.eligible {
            let message = observation
                .drift
                .first()
                .map(|drift| drift.message.clone())
                .unwrap_or_else(|| "participant is not eligible for coordinated abort".to_owned());
            let mut error = ModelError::new(ErrorCode::MergeDrift, message);
            error.member_id = Some(target_id.clone());
            error.member_path = Some(participant.path.clone());
            return Err(error);
        }
        if verified_no_op(snapshot.record.state, participant, observation) {
            preflight.no_op_targets.insert(target_id.clone());
        }
    }
    Ok(preflight)
}

/// Select only no-ops already accepted by the shared status classifier. This
/// function decides whether Git must be called; it never overrides an
/// ineligible snapshot (in particular, foreign sequencer state).
fn verified_no_op(
    operation: OperationState,
    participant: &MergeParticipantRecord,
    observation: &MergeParticipantObservation,
) -> bool {
    if !observation.abort_eligibility.eligible {
        return false;
    }
    if matches!(
        participant.state,
        ParticipantState::Aborted | ParticipantState::RolledBack
    ) {
        return true;
    }
    if observation.live_commit.as_deref() != Some(&participant.before_commit)
        || observation.drift.is_empty()
    {
        return false;
    }
    match participant.state {
        ParticipantState::Conflicted => observation
            .drift
            .iter()
            .all(|drift| drift.kind == ParticipantDriftKind::MergeStateMissing),
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
            if operation == OperationState::RollingBack =>
        {
            observation.drift.iter().all(|drift| {
                matches!(
                    drift.kind,
                    ParticipantDriftKind::TargetRefChanged | ParticipantDriftKind::HeadRewound
                )
            })
        }
        _ => false,
    }
}

pub(super) fn verify_baseline(root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
    for (relative, expected) in [
        (artifact::LOCK_PATH, record.baseline.lock_sha256.as_str()),
        (WORKSPACE_MANIFEST, record.baseline.manifest_sha256.as_str()),
    ] {
        let actual = fs::read(root.join(relative))
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        if actual.as_deref() != Some(expected) {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("workspace artifact '{relative}' does not match the abort baseline"),
            ));
        }
    }
    Ok(())
}
