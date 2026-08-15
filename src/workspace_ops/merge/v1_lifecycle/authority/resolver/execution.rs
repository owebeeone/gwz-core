use super::*;

pub(super) fn no_progress(
    current: &StoredV1Record,
    attempt: BoundExecutionAttempt,
) -> ModelResult<ResolvedV1Action> {
    let (key, diagnostic) = attempt.0.value;
    let ExecutionDiagnostic::Failed {
        code,
        message,
        detail,
    } = diagnostic
    else {
        return reject("owned action reported success without progress");
    };
    let PhysicalActionKind::Participant { member_id, .. } = key.action else {
        if matches!(key.action, PhysicalActionKind::Preservation(_)) {
            return Ok(ResolvedV1Action::Reject(ModelError::new(code, message)));
        }
        // The underlying cause must survive into the reject: the bare label
        // here masked a config failure across a full Windows-matrix
        // diagnosis cycle (classification run 10 → probe).
        return Ok(ResolvedV1Action::Reject(ModelError::new(
            code,
            format!("owned non-participant action made no progress: {message}"),
        )));
    };
    let row = current
        .record()
        .participants
        .get(&member_id)
        .ok_or_else(rejected)?;
    let mut result = row.clone();
    result.error = Some(MergeRecordError {
        code,
        message,
        detail,
    });
    let issuer = AuthorityIssuer::for_observer(current);
    let mut payload = failure_payload(current.record(), member_id.clone(), result);
    let value = if row
        .pending_action
        .as_ref()
        .is_some_and(|value| value.kind == PendingMergeActionKind::ResolveConflict)
    {
        ParticipantTransition::RecordOwnedResolutionFailureAndHalt(B::new(
            BoundOwnedResolutionFailureHaltBatch::issue(
                &issuer,
                &member_id,
                "owned_resolution_failure",
                "verified",
                payload,
            )?,
        ))
    } else {
        row_failed(&mut payload.row);
        ParticipantTransition::RecordOwnedRetryFailureAndHalt(B::new(
            BoundOwnedRetryFailureHaltBatch::issue(
                &issuer,
                &member_id,
                "owned_retry_failure",
                "verified",
                payload,
            )?,
        ))
    };
    part(value)
}

pub(super) fn attempt_succeeded(attempt: &BoundExecutionAttempt) -> bool {
    matches!(attempt.0.value.1, ExecutionDiagnostic::Success)
}

pub(super) fn attempt_failure(attempt: &BoundExecutionAttempt) -> Option<ModelError> {
    let ExecutionDiagnostic::Failed { code, message, .. } = &attempt.0.value.1 else {
        return None;
    };
    Some(ModelError::new(*code, message.clone()))
}

pub(super) fn physical_matches(
    current: &StoredV1Record,
    kind: &ObservationKind,
    action: &PhysicalActionKind,
) -> bool {
    match (kind, action) {
        (
            ObservationKind::ParticipantAction { member_id },
            PhysicalActionKind::Participant {
                member_id: id,
                action,
            },
        ) => {
            member_id == id
                && current
                    .record()
                    .participants
                    .get(id)
                    .and_then(|row| row.pending_action.as_ref())
                    == Some(action.as_ref())
        }
        (ObservationKind::Publication, PhysicalActionKind::Publication(action)) => current
            .record()
            .publication
            .as_ref()
            .is_some_and(|progress| match action {
                PublicationPhysicalAction::EvidenceCommit => {
                    progress.step == PublicationStep::CommittingEvidence
                        && progress.candidate.is_some()
                        && progress.composition_commit.is_none()
                }
                _ => {
                    progress.step == PublicationStep::PublishingCandidate
                        && progress.candidate.is_some()
                        && progress.composition_commit.is_some()
                }
            }),
        (ObservationKind::PreservationCursor, PhysicalActionKind::Preservation(action)) => {
            current.record().pending_preservation.as_ref() == Some(action)
                && preservation_incomplete(action)
        }
        (ObservationKind::RollbackCursor, PhysicalActionKind::Rollback(action)) => {
            current.record().pending_rollback.as_ref() == Some(action)
                && rollback_incomplete(action)
        }
        (ObservationKind::Archive, PhysicalActionKind::Archive) => matches!(
            current.record().state,
            OperationState::Completed | OperationState::Aborted
        ),
        _ => false,
    }
}

fn preservation_incomplete(value: &PendingPreservationActionV1) -> bool {
    use super::super::super::super::model::v1::{
        PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
    };
    match value {
        PendingPreservationActionV1::BackupRef { .. } => true,
        PendingPreservationActionV1::Stash { phase, .. } => *phase != S::Complete,
        PendingPreservationActionV1::ResetAttachedRef { phase, .. } => *phase != R::Complete,
    }
}

fn rollback_incomplete(value: &PendingRollbackActionV1) -> bool {
    use super::super::super::super::model::v1::{
        EvidenceRollbackStepV1 as E, RootMetadataRollbackStepV1 as R,
    };
    match value {
        PendingRollbackActionV1::Participant { .. } => true,
        PendingRollbackActionV1::PublicationEvidence { next_step } => *next_step != E::Complete,
        PendingRollbackActionV1::SelectedRootMetadata { next_step } => *next_step != R::Complete,
    }
}

fn failure_payload(
    record: &MergeOperationRecordV1,
    member_id: String,
    row: MergeParticipantRecord,
) -> ParticipantFailurePayload {
    let start = record
        .selected_targets
        .iter()
        .position(|id| id == &member_id)
        .map_or(record.selected_targets.len(), |index| index + 1);
    let later_unattempted = record.selected_targets[start..]
        .iter()
        .filter(|id| {
            record
                .participants
                .get(*id)
                .is_some_and(|row| row.state == ParticipantState::Planned)
        })
        .cloned()
        .collect();
    ParticipantFailurePayload {
        member_id,
        row,
        later_unattempted,
    }
}

fn row_failed(row: &mut MergeParticipantRecord) {
    row.state = ParticipantState::Failed;
    row.resulting_commit = None;
    row.expected_merge_head = None;
    row.conflict_paths.clear();
    row.conflict_snapshot.clear();
}
