use super::super::super::*;
use crate::git::{GitCheckoutOverlay, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::ParticipantState;
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1, PendingRollbackActionV1, RollbackCursor, RootMetadataRollbackStepV1,
    rollback_cursor,
};

pub(super) enum RollbackAggregateClassification {
    Exact(RollbackAggregatePayload),
    Mismatch,
}

pub(super) fn position(current: &StoredV1Record) -> RollbackAggregatePosition {
    if let Some(action) = current.record().pending_rollback.as_ref() {
        return match action {
            PendingRollbackActionV1::Participant {
                member_id, action, ..
            } => RollbackAggregatePosition::ParticipantPending {
                member_id: member_id.clone(),
                kind: *action,
            },
            PendingRollbackActionV1::PublicationEvidence { next_step } => {
                RollbackAggregatePosition::EvidencePending(*next_step)
            }
            PendingRollbackActionV1::SelectedRootMetadata { next_step } => {
                RollbackAggregatePosition::SelectedRootMetadataPending(*next_step)
            }
        };
    }
    match rollback_cursor(current.record()) {
        RollbackCursor::PublicationEvidence => {
            RollbackAggregatePosition::EvidencePending(EvidenceRollbackStepV1::EvidenceCommit)
        }
        RollbackCursor::Participant { member_id, .. } => {
            RollbackAggregatePosition::BetweenParticipants(member_id.into())
        }
        RollbackCursor::NoMutationParticipant { member_id } => {
            RollbackAggregatePosition::NoMutationParticipant(member_id.into())
        }
        RollbackCursor::SelectedRootMetadata => {
            RollbackAggregatePosition::SelectedRootMetadataPending(
                RootMetadataRollbackStepV1::Complete,
            )
        }
        RollbackCursor::Complete => RollbackAggregatePosition::Exhaustion,
    }
}

pub(super) fn recovery_position(
    current: &StoredV1Record,
) -> ModelResult<RollbackAggregatePosition> {
    current
        .record()
        .pending_rollback
        .clone()
        .map(RollbackAggregatePosition::RecoveryPending)
        .ok_or_else(|| prefix_error("rollback recovery has no exact pending action"))
}

pub(super) fn classify_rollback_aggregate<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    position: RollbackAggregatePosition,
) -> ModelResult<RollbackAggregateClassification> {
    let record = current.record();
    let root = current.location().root();
    let completed_participants = completed_prefix(record, &position);
    let selected_root_complete = completed_participants
        .iter()
        .any(|member_id| member_id == "@root");
    let selected_root_checkout_supersedes_evidence =
        selected_root_complete || pending_selected_root_is_after(backend, root, record, &position)?;

    let publication_evidence_complete = record.publication.as_ref().is_some_and(|publication| {
        publication.candidate.is_some()
            && publication.composition_commit.is_some()
            && publication.evidence_rolled_back
    });
    if publication_evidence_complete {
        let exact = if selected_root_checkout_supersedes_evidence {
            crate::workspace_ops::merge::abort::v1_evidence_residue_after_selected_root_is_exact(
                root, record,
            )?
        } else {
            crate::workspace_ops::merge::abort::observe_v1_evidence_rollback(
                backend,
                root,
                record,
                EvidenceRollbackStepV1::Complete,
            )? == crate::workspace_ops::merge::abort::V1EvidenceRollbackObservation::After
        };
        if !exact {
            return Ok(RollbackAggregateClassification::Mismatch);
        }
    }

    let selected_root_projection = selected_root_complete.then_some(match &position {
        RollbackAggregatePosition::SelectedRootMetadataPending(step) => *step,
        RollbackAggregatePosition::RecoveryPending(
            PendingRollbackActionV1::SelectedRootMetadata { next_step },
        ) => *next_step,
        _ => RootMetadataRollbackStepV1::Complete,
    });
    if let Some(step) = selected_root_projection {
        let observed = crate::workspace_ops::merge::root::observe_v1_root_metadata_rollback(
            backend, root, record, step,
        )?;
        if observed == crate::workspace_ops::merge::root::V1RootRollbackObservation::Ambiguous {
            return Ok(RollbackAggregateClassification::Mismatch);
        }
    }

    for member_id in &completed_participants {
        let row = record
            .participants
            .get(member_id)
            .ok_or_else(|| prefix_error("terminal rollback participant disappeared"))?;
        let overlay = if member_id == "@root" {
            GitCheckoutOverlay {
                worktree_paths: vec![
                    crate::workspace::WORKSPACE_MANIFEST.into(),
                    crate::artifact::LOCK_PATH.into(),
                ],
                index_paths: Vec::new(),
            }
        } else {
            GitCheckoutOverlay::default()
        };
        if !crate::workspace_ops::merge::abort::terminal_v1_participant_is_exact(
            backend, root, member_id, row, &overlay,
        )? {
            return Ok(RollbackAggregateClassification::Mismatch);
        }
    }

    let projection_sha256 = payload_hash(&(
        &position,
        &completed_participants,
        publication_evidence_complete,
        selected_root_projection,
    ))?;
    Ok(RollbackAggregateClassification::Exact(
        RollbackAggregatePayload {
            position,
            completed_participants,
            publication_evidence_complete,
            selected_root_projection,
            projection_sha256,
        },
    ))
}

fn pending_selected_root_is_after<B: MergeAuthorityBackend>(
    backend: &B,
    root: &std::path::Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    position: &RollbackAggregatePosition,
) -> ModelResult<bool> {
    let action = match position {
        RollbackAggregatePosition::ParticipantPending {
            member_id, kind, ..
        } if member_id == "@root" => Some(*kind),
        RollbackAggregatePosition::RecoveryPending(PendingRollbackActionV1::Participant {
            member_id,
            action,
            ..
        }) if member_id == "@root" => Some(*action),
        _ => None,
    };
    let Some(action) = action else {
        return Ok(false);
    };
    let row = record
        .participants
        .get("@root")
        .ok_or_else(|| prefix_error("selected-root rollback participant disappeared"))?;
    Ok(
        crate::workspace_ops::merge::abort::observe_v1_participant_rollback(
            backend, root, record, "@root", row, action,
        )? == crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::After,
    )
}

fn completed_prefix(
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    position: &RollbackAggregatePosition,
) -> Vec<String> {
    let current_participant = match position {
        RollbackAggregatePosition::BetweenParticipants(member_id)
        | RollbackAggregatePosition::NoMutationParticipant(member_id) => Some(member_id.as_str()),
        RollbackAggregatePosition::ParticipantPending { member_id, .. } => Some(member_id.as_str()),
        RollbackAggregatePosition::RecoveryPending(PendingRollbackActionV1::Participant {
            member_id,
            ..
        }) => Some(member_id.as_str()),
        _ => None,
    };
    if let Some(current_participant) = current_participant {
        return record
            .selected_targets
            .iter()
            .rev()
            .take_while(|member_id| member_id.as_str() != current_participant)
            .filter(|member_id| terminal(record, member_id))
            .cloned()
            .collect();
    }
    if matches!(
        position,
        RollbackAggregatePosition::SelectedRootMetadataPending(_)
            | RollbackAggregatePosition::Exhaustion
            | RollbackAggregatePosition::RecoveryPending(
                PendingRollbackActionV1::SelectedRootMetadata { .. }
            )
    ) {
        return record
            .selected_targets
            .iter()
            .filter(|member_id| terminal(record, member_id))
            .cloned()
            .collect();
    }
    Vec::new()
}

fn terminal(
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    member_id: &str,
) -> bool {
    record.participants.get(member_id).is_some_and(|row| {
        matches!(
            row.state,
            ParticipantState::Aborted | ParticipantState::RolledBack
        )
    })
}

pub(super) fn issue_verified_rollback_prefix<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    position: RollbackAggregatePosition,
) -> ModelResult<VerifiedRollbackPrefix> {
    let RollbackAggregateClassification::Exact(facts) =
        classify_rollback_aggregate(backend, current, position)?
    else {
        return Err(prefix_error("live rollback aggregate prefix has drifted"));
    };
    VerifiedRollbackPrefix::issue(&AuthorityIssuer::for_observer(current), facts)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn require_rollback_aggregate<
    B: MergeAuthorityBackend,
>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    match classify_rollback_aggregate(backend, current, position(current))? {
        RollbackAggregateClassification::Exact(_) => Ok(()),
        RollbackAggregateClassification::Mismatch => {
            Err(prefix_error("live rollback aggregate prefix has drifted"))
        }
    }
}

fn prefix_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::RecoveryEvidenceMismatch, detail.into())
}
