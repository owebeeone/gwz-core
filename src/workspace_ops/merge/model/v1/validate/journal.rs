use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::{OperationState, ParticipantState, PublicationStep};
use super::super::{
    AcceptedMetadataSourceV1, EvidenceRollbackStepV1, MergeOperationRecordV1,
    ParticipantRollbackKindV1, PendingRollbackActionV1, RecoveryOriginStateV1,
    RootMetadataRollbackStepV1,
};

pub(crate) fn validate_v1_journal(record: &MergeOperationRecordV1) -> ModelResult<()> {
    validate_recovery_context(record)?;
    validate_pending_legality(record)?;
    if let Some(action) = record.pending_rollback.as_ref() {
        validate_rollback(record, action)?;
    }
    super::validate_v1_preservation(record)?;
    Ok(())
}

fn validate_recovery_context(record: &MergeOperationRecordV1) -> ModelResult<()> {
    match (record.state, record.recovery_context.as_ref()) {
        (OperationState::RecoveryRequired, Some(context))
            if derived_origin(record) == Some(context.origin_state) =>
        {
            Ok(())
        }
        (OperationState::RecoveryRequired, _) => Err(recovery_error(record)),
        (_, None) => Ok(()),
        (_, Some(_)) => Err(recovery_error(record)),
    }
}

fn derived_origin(record: &MergeOperationRecordV1) -> Option<RecoveryOriginStateV1> {
    if record.pending_preservation.is_some() {
        return Some(RecoveryOriginStateV1::Preserving);
    }
    if record.pending_rollback.is_some() {
        return Some(RecoveryOriginStateV1::RollingBack);
    }
    let states = record
        .participants
        .values()
        .map(|participant| participant.state)
        .collect::<Vec<_>>();
    if record.participants.values().any(|participant| {
        participant.state == ParticipantState::Failed
            || participant.state == ParticipantState::Conflicted && participant.error.is_some()
    }) {
        return Some(RecoveryOriginStateV1::Halted);
    }
    if record
        .participants
        .values()
        .any(|participant| participant.pending_action.is_some())
    {
        return Some(RecoveryOriginStateV1::Executing);
    }
    if states.contains(&ParticipantState::Conflicted) {
        return Some(RecoveryOriginStateV1::AwaitingResolution);
    }
    let complete = !states.is_empty()
        && states.iter().all(|state| {
            matches!(
                state,
                ParticipantState::UpToDate
                    | ParticipantState::FastForwarded
                    | ParticipantState::Merged
                    | ParticipantState::Continued
            )
        });
    if record.accepted_workspace.is_some() || record.publication.is_some() || complete {
        finalization_resume_is_unique(record).then_some(RecoveryOriginStateV1::Finalizing)
    } else {
        Some(RecoveryOriginStateV1::Executing)
    }
}

fn finalization_resume_is_unique(record: &MergeOperationRecordV1) -> bool {
    let base_phase_is_exact = match record.publication.as_ref() {
        None => true,
        Some(publication) if publication.candidate.is_none() => matches!(
            publication.step,
            PublicationStep::NotStarted
                | PublicationStep::ValidatingResults
                | PublicationStep::PreparingCandidate
                | PublicationStep::Complete
        ),
        Some(publication) if publication.composition_commit.is_none() => matches!(
            publication.step,
            PublicationStep::PreparingCandidate | PublicationStep::CommittingEvidence
        ),
        Some(publication) => matches!(
            publication.step,
            PublicationStep::CommittingEvidence
                | PublicationStep::PublishingCandidate
                | PublicationStep::VerifyingPublication
                | PublicationStep::Complete
        ),
    };
    let mut view = record.v0_common_view();
    view.state = OperationState::Finalizing;
    base_phase_is_exact
        && crate::workspace_ops::merge::acceptance::finalization_next_action_for_i2(&view).is_ok()
}

fn validate_pending_legality(record: &MergeOperationRecordV1) -> ModelResult<()> {
    if record.pending_rollback.is_some() && record.pending_preservation.is_some() {
        return Err(recovery_error(record));
    }
    let allowed = match record.state {
        OperationState::RollingBack => {
            record.pending_preservation.is_none() && record.recovery_context.is_none()
        }
        OperationState::Preserving => {
            record.pending_rollback.is_none() && record.recovery_context.is_none()
        }
        OperationState::RecoveryRequired => {
            record
                .recovery_context
                .as_ref()
                .is_some_and(|context| match context.origin_state {
                    RecoveryOriginStateV1::RollingBack => {
                        record.pending_rollback.is_some() && record.pending_preservation.is_none()
                    }
                    RecoveryOriginStateV1::Preserving => {
                        record.pending_preservation.is_some() && record.pending_rollback.is_none()
                    }
                    _ => record.pending_rollback.is_none() && record.pending_preservation.is_none(),
                })
        }
        _ => record.pending_rollback.is_none() && record.pending_preservation.is_none(),
    };
    if allowed {
        Ok(())
    } else {
        Err(recovery_error(record))
    }
}

fn validate_rollback(
    record: &MergeOperationRecordV1,
    action: &PendingRollbackActionV1,
) -> ModelResult<()> {
    match action {
        PendingRollbackActionV1::Participant {
            member_id,
            action,
            terminal_state,
        } => validate_participant_rollback(record, member_id, *action, *terminal_state),
        PendingRollbackActionV1::PublicationEvidence { next_step } => {
            validate_evidence_rollback(record, *next_step)
        }
        PendingRollbackActionV1::SelectedRootMetadata { next_step } => {
            validate_root_metadata_rollback(record, *next_step)
        }
    }
}

fn validate_participant_rollback(
    record: &MergeOperationRecordV1,
    member_id: &str,
    action: ParticipantRollbackKindV1,
    terminal_state: ParticipantState,
) -> ModelResult<()> {
    let Some(participant) = record.participants.get(member_id) else {
        return Err(rollback_error(record));
    };
    let valid = match action {
        ParticipantRollbackKindV1::AbortConflict => {
            terminal_state == ParticipantState::Aborted
                && participant.state == ParticipantState::Conflicted
        }
        ParticipantRollbackKindV1::ResetIntegrated => {
            terminal_state == ParticipantState::RolledBack
                && matches!(
                    participant.state,
                    ParticipantState::FastForwarded
                        | ParticipantState::Merged
                        | ParticipantState::Continued
                )
                && participant.resulting_commit.is_some()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(rollback_error(record))
    }
}

fn validate_evidence_rollback(
    record: &MergeOperationRecordV1,
    next_step: EvidenceRollbackStepV1,
) -> ModelResult<()> {
    let Some(publication) = record.publication.as_ref() else {
        return Err(rollback_error(record));
    };
    let complete_evidence = publication.candidate.is_some()
        && publication.composition_commit.is_some()
        && publication.composition_tree.is_some()
        && !publication.candidate_hashes.is_empty()
        && !publication.evidence_rolled_back;
    let phase_owns_evidence = match next_step {
        EvidenceRollbackStepV1::EvidenceCommit => {
            publication.step >= PublicationStep::CommittingEvidence
        }
        EvidenceRollbackStepV1::Boundary
        | EvidenceRollbackStepV1::Lock
        | EvidenceRollbackStepV1::Marker
        | EvidenceRollbackStepV1::Index
        | EvidenceRollbackStepV1::Complete => {
            publication.step >= PublicationStep::CommittingEvidence
                && publication.composition_commit.is_some()
        }
    };
    if complete_evidence && phase_owns_evidence {
        Ok(())
    } else {
        Err(rollback_result_error(record))
    }
}

fn validate_root_metadata_rollback(
    record: &MergeOperationRecordV1,
    next_step: RootMetadataRollbackStepV1,
) -> ModelResult<()> {
    let selected_root = record
        .selected_targets
        .iter()
        .any(|target| target == "@root")
        && record.participants.contains_key("@root");
    let Some(accepted) = record.accepted_workspace.as_ref() else {
        return Err(rollback_error(record));
    };
    let selected_source = matches!(
        accepted.metadata_base.source,
        AcceptedMetadataSourceV1::SelectedRootResult { .. }
    );
    let baseline_manifest = record.baseline.manifest_yaml.as_deref();
    let baseline_lock = record.baseline.lock_yaml.as_deref();
    let exact_baseline = baseline_manifest
        .is_some_and(|yaml| digest(yaml) == record.baseline.manifest_sha256)
        && baseline_lock.is_some_and(|yaml| digest(yaml) == record.baseline.lock_sha256);
    let prior_evidence_complete = record.publication.as_ref().is_none_or(|publication| {
        publication.candidate.is_none() || publication.evidence_rolled_back
    });
    let phase_has_input = match next_step {
        RootMetadataRollbackStepV1::Manifest => baseline_manifest.is_some(),
        RootMetadataRollbackStepV1::Lock | RootMetadataRollbackStepV1::Complete => {
            baseline_manifest.is_some() && baseline_lock.is_some()
        }
    };
    if selected_root
        && selected_source
        && exact_baseline
        && prior_evidence_complete
        && phase_has_input
    {
        Ok(())
    } else {
        Err(rollback_result_error(record))
    }
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn recovery_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::RecoveryEvidenceMismatch,
        "recovery evidence is invalid",
        "recovery evidence does not match any legal origin",
    )
}

fn rollback_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::RollbackEvidenceMismatch,
        "rollback evidence is invalid",
        "rollback evidence has no unique owner or action step",
    )
}

fn rollback_result_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::RollbackEvidenceMismatch,
        "rollback evidence is invalid",
        "participant, evidence, or selected-root rollback result is not exact",
    )
}

fn typed_error(
    record: &MergeOperationRecordV1,
    code: ErrorCode,
    prefix: &str,
    reason: &str,
) -> ModelError {
    ModelError::new(
        code,
        format!("merge record '{}' {prefix}: {reason}", record.merge_id),
    )
}
