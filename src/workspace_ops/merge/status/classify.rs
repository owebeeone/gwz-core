use std::path::Path;

use crate::git::GitBackend;
use crate::model::ModelResult;

use super::super::{MergeParticipantObservation, MergeParticipantRecord, participant_semantics};
use super::member_result;

pub(super) use participant_semantics::status::{HeadRelation, MissingObject, ParticipantLiveState};

pub(super) fn missing_recorded_objects<B: GitBackend>(
    backend: &B,
    path: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<Vec<MissingObject>> {
    let mut required = vec![
        ("before commit", participant.before_commit.as_str()),
        ("source commit", participant.source_commit.as_str()),
    ];
    if let Some(result) = participant.resulting_commit.as_deref() {
        required.push(("resulting commit", result));
    }
    if let Some(merge_head) = participant.expected_merge_head.as_deref() {
        required.push(("expected merge head", merge_head));
    }
    if let Some(pending) = participant.pending_action.as_ref() {
        required.extend([
            ("pending before commit", pending.before_commit.as_str()),
            ("pending source commit", pending.source_commit.as_str()),
        ]);
    }

    let mut missing = Vec::new();
    let mut checked = Vec::new();
    for (role, oid) in required {
        if checked.contains(&oid) {
            continue;
        }
        checked.push(oid);
        if !member_result(
            backend.commit_exists(path, oid),
            target_id,
            &participant.path,
        )? {
            missing.push(MissingObject {
                role: role.to_owned(),
                oid: oid.to_owned(),
            });
        }
    }
    Ok(missing)
}

pub(super) fn classify_participant(
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
) -> MergeParticipantObservation {
    participant_semantics::status::observation_from_projection(
        participant,
        participant_semantics::status::project_participant_drift(target_id, participant, live),
    )
}
