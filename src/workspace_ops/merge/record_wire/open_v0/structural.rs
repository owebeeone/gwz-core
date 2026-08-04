use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    MERGE_RECORD_SCHEMA_V1, MERGE_RECORD_SCHEMA_VERSION_V1, MergeOperationRecordV1,
    RecoveryContextV1, RecoveryOriginStateV1, validate_common_v0_view, validate_v1_lifecycle,
    validate_v1_preservation,
};
use crate::workspace_ops::merge::{
    MergeExecutionMode, MergeOperationRecord, OperationState, ParticipantState,
    PendingMergeActionKind, PublicationProgress, PublicationStep,
};

pub(super) fn validate_v0_structure(record: &MergeOperationRecord) -> ModelResult<()> {
    let view = common_view(record);
    validate_common_v0_view(&view)?;
    validate_v0_actions(record)?;
    validate_v1_preservation(&view)?;
    validate_lifecycle(record, view)?;
    validate_publication(record)
}

fn validate_v0_actions(record: &MergeOperationRecord) -> ModelResult<()> {
    for (target_id, participant) in &record.participants {
        let Some(pending) = participant.pending_action.as_ref() else {
            continue;
        };
        if crate::workspace_ops::merge::integration::decode_for_participant(pending, participant)
            .is_err()
        {
            return Err(action_error(record, target_id));
        }
        let mode_allows = match record.mode {
            MergeExecutionMode::Normal => true,
            MergeExecutionMode::FfOnly => matches!(
                pending.kind,
                PendingMergeActionKind::VerifyUpToDate | PendingMergeActionKind::FastForward
            ),
            MergeExecutionMode::NoFf => pending.kind != PendingMergeActionKind::FastForward,
        };
        let state_allows = match pending.kind {
            PendingMergeActionKind::ResolveConflict => {
                participant.state == ParticipantState::Conflicted
            }
            PendingMergeActionKind::VerifyUpToDate
            | PendingMergeActionKind::FastForward
            | PendingMergeActionKind::TrueMerge => matches!(
                participant.state,
                ParticipantState::Planned
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
            ),
        };
        if !mode_allows || !state_allows || !valid_commit_spec(pending.commit_spec.as_ref()) {
            return Err(action_error(record, target_id));
        }
    }
    Ok(())
}

fn valid_commit_spec(spec: Option<&crate::workspace_ops::merge::PendingCommitSpec>) -> bool {
    let Some(spec) = spec else {
        return true;
    };
    is_oid(&spec.tree_oid) && valid_signature(&spec.author) && valid_signature(&spec.committer)
}

fn valid_signature(signature: &crate::workspace_ops::merge::PendingGitSignature) -> bool {
    !signature.name.trim().is_empty()
        && !signature.email.trim().is_empty()
        && !signature.name.contains(['\0', '\n', '\r'])
        && !signature.email.contains(['\0', '\n', '\r'])
        && (-1_440..=1_440).contains(&signature.timezone_offset_minutes)
}

fn action_error(record: &MergeOperationRecord, target_id: &str) -> ModelError {
    typed_error(
        record,
        ErrorCode::MergeRecordUnreadable,
        &format!(
            "participant '{target_id}' pending action violates the v0 intent, mode, result, commit-spec, or state matrix"
        ),
    )
}

fn common_view(record: &MergeOperationRecord) -> MergeOperationRecordV1 {
    MergeOperationRecordV1 {
        schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
        writer_version: record.writer_version.clone(),
        workspace_id: record.workspace_id.clone(),
        merge_id: record.merge_id.clone(),
        operation_id: record.operation_id.clone(),
        state: record.state,
        source_ref: record.source_ref.clone(),
        mode: record.mode,
        created_at: record.created_at.clone(),
        baseline: record.baseline.clone(),
        selected_targets: record.selected_targets.clone(),
        participants: record.participants.clone(),
        publication: record.publication.clone(),
        operation_drift: record.operation_drift.clone(),
        accepted_workspace: None,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        extensions: record.extensions.clone(),
    }
}

fn validate_lifecycle(
    record: &MergeOperationRecord,
    mut view: MergeOperationRecordV1,
) -> ModelResult<()> {
    if record.state != OperationState::RecoveryRequired {
        return validate_v1_lifecycle(&view);
    }
    let origins = [
        RecoveryOriginStateV1::Executing,
        RecoveryOriginStateV1::AwaitingResolution,
        RecoveryOriginStateV1::Halted,
        RecoveryOriginStateV1::Finalizing,
        RecoveryOriginStateV1::Preserving,
        RecoveryOriginStateV1::RollingBack,
    ];
    if origins.into_iter().any(|origin_state| {
        view.recovery_context = Some(RecoveryContextV1 { origin_state });
        validate_v1_lifecycle(&view).is_ok()
    }) {
        Ok(())
    } else {
        Err(typed_error(
            record,
            ErrorCode::RecoveryEvidenceMismatch,
            "recovery evidence does not match any legal origin",
        ))
    }
}

fn validate_publication(record: &MergeOperationRecord) -> ModelResult<()> {
    let Some(publication) = record.publication.as_ref() else {
        return if record.state == OperationState::Completed {
            Err(typed_error(
                record,
                ErrorCode::TerminalEvidenceMismatch,
                "completed record is not published or no-publication-complete",
            ))
        } else {
            Ok(())
        };
    };
    match publication.candidate.as_ref() {
        None => validate_without_candidate(record, publication)?,
        Some(_) => validate_with_candidate(record, publication)?,
    }
    validate_terminal(record, publication)
}

fn validate_without_candidate(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
) -> ModelResult<()> {
    let outputs_present = publication.candidate_lock_sha256.is_some()
        || publication.candidate_marker_path.is_some()
        || publication.root_merge_commit.is_some()
        || publication.composition_commit.is_some()
        || publication.composition_tree.is_some()
        || !publication.candidate_hashes.is_empty()
        || publication.evidence_rolled_back
        || !publication.root_preservation.is_empty()
        || publication.preservation_prefix.is_some();
    let step_valid = matches!(
        publication.step,
        PublicationStep::NotStarted
            | PublicationStep::ValidatingResults
            | PublicationStep::PreparingCandidate
            | PublicationStep::Complete
    );
    let required = publication_required(record);
    let decision_valid = match publication.step {
        PublicationStep::PreparingCandidate => required,
        PublicationStep::Complete => !required,
        _ => true,
    };
    if outputs_present || !step_valid || !decision_valid {
        return Err(typed_error(
            record,
            ErrorCode::UnexpectedPublicationEvidence,
            "no-publication completion contains candidate, composition, hash, or published-prefix evidence",
        ));
    }
    Ok(())
}

fn validate_with_candidate(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
) -> ModelResult<()> {
    let prefix_valid = publication
        .preservation_prefix
        .as_deref()
        .is_none_or(|prefix| matches!(prefix, "baseline" | "marker" | "lock" | "boundary"));
    if !publication_required(record)
        || matches!(
            publication.step,
            PublicationStep::NotStarted | PublicationStep::ValidatingResults
        )
        || !prefix_valid
        || crate::workspace_ops::merge::finalize::validate_candidate_for_i2_fixture(record).is_err()
    {
        return Err(typed_error(
            record,
            ErrorCode::CandidateIntegrityMismatch,
            "candidate cross-field identity or hash validation failed",
        ));
    }
    let complete = composition_complete(record, publication)?;
    let phase_valid = match publication.step {
        PublicationStep::PreparingCandidate => !complete,
        PublicationStep::CommittingEvidence => true,
        PublicationStep::PublishingCandidate
        | PublicationStep::VerifyingPublication
        | PublicationStep::Complete => complete,
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => false,
    };
    if !phase_valid {
        return Err(evidence_error(record));
    }
    Ok(())
}

fn composition_complete(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
) -> ModelResult<bool> {
    let absent = publication.composition_commit.is_none()
        && publication.composition_tree.is_none()
        && publication.candidate_hashes.is_empty();
    if absent {
        return Ok(false);
    }
    let complete = publication
        .composition_commit
        .as_deref()
        .is_some_and(is_oid)
        && publication.composition_tree.as_deref().is_some_and(is_oid)
        && candidate_hashes_are_exact(publication);
    if complete {
        Ok(true)
    } else {
        Err(evidence_error(record))
    }
}

fn validate_terminal(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
) -> ModelResult<()> {
    let complete = composition_complete(record, publication)?;
    if publication.evidence_rolled_back && !complete {
        return Err(evidence_error(record));
    }
    if record.state == OperationState::Aborted {
        if publication.candidate.is_some() && publication.evidence_rolled_back != complete {
            return Err(typed_error(
                record,
                ErrorCode::TerminalRollbackMismatch,
                "aborted record retains an incomplete or contradictory rollback action",
            ));
        }
        return Ok(());
    }
    if record.state != OperationState::Completed {
        if publication.evidence_rolled_back
            && !matches!(
                record.state,
                OperationState::RollingBack | OperationState::RecoveryRequired
            )
        {
            return Err(evidence_error(record));
        }
        return Ok(());
    }
    let candidate_complete = publication.candidate.is_some()
        && publication.step == PublicationStep::Complete
        && !publication.evidence_rolled_back;
    let no_publication_complete = publication.candidate.is_none()
        && publication.step == PublicationStep::Complete
        && !publication_required(record);
    if candidate_complete || no_publication_complete {
        Ok(())
    } else {
        Err(typed_error(
            record,
            ErrorCode::TerminalEvidenceMismatch,
            "completed record is not published or no-publication-complete",
        ))
    }
}

fn candidate_hashes_are_exact(publication: &PublicationProgress) -> bool {
    let Some(candidate) = publication.candidate.as_ref() else {
        return false;
    };
    let Some(marker_path) = publication.candidate_marker_path.as_deref() else {
        return false;
    };
    let mut expected = [
        (
            crate::artifact::LOCK_PATH,
            digest(candidate.lock_yaml.as_bytes()),
        ),
        (marker_path, digest(candidate.marker_yaml.as_bytes())),
    ];
    expected.sort_by(|left, right| left.0.cmp(right.0));
    publication.candidate_hashes.len() == expected.len()
        && publication
            .candidate_hashes
            .iter()
            .zip(expected)
            .all(|(actual, (path, hash))| actual.path == path && actual.sha256 == hash)
}

fn publication_required(record: &MergeOperationRecord) -> bool {
    crate::workspace_ops::merge::acceptance::publication_required(record)
}

fn evidence_error(record: &MergeOperationRecord) -> ModelError {
    typed_error(
        record,
        ErrorCode::RecordedEvidenceDrift,
        "recorded composition commit, tree, parent, message, files, or hashes changed",
    )
}

fn typed_error(record: &MergeOperationRecord, code: ErrorCode, reason: &str) -> ModelError {
    ModelError::new(
        code,
        format!("merge record '{}' is invalid: {reason}", record.merge_id),
    )
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
