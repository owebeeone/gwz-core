use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::model::ModelResult;

use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, OperationDrift, OperationDriftKind,
    ParticipantDrift, ParticipantDriftKind, participant_semantics,
};
use super::*;

pub(super) fn expected_head(participant: &MergeParticipantRecord) -> ModelResult<&str> {
    participant_semantics::status::expected_head(participant)
}

pub(super) fn missing_observation(
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> MergeParticipantObservation {
    participant_semantics::status::missing_repository_observation(target_id, participant)
}

pub(super) fn participant_drift(
    kind: ParticipantDriftKind,
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
    guidance: &str,
) -> ParticipantDrift {
    participant_semantics::status::participant_drift(kind, target_id, participant, live, guidance)
}

pub(super) fn compare_digest(
    root: &Path,
    relative: &str,
    expected: &str,
    kind: OperationDriftKind,
    drift: &mut Vec<OperationDrift>,
) {
    let actual = fs::read(root.join(relative))
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    if actual.as_deref() != Some(expected) && !drift.iter().any(|item| item.kind == kind) {
        drift.push(OperationDrift {
            kind,
            message: format!(
                "workspace artifact '{relative}' changed from the recorded merge baseline"
            ),
        });
    }
}

pub(super) fn push_operation_drift(
    drift: &mut Vec<OperationDrift>,
    kind: OperationDriftKind,
    message: &str,
) {
    if !drift.iter().any(|item| item.kind == kind) {
        drift.push(OperationDrift {
            kind,
            message: message.to_owned(),
        });
    }
}
