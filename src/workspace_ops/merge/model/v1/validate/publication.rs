use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::{OperationState, PublicationProgress, PublicationStep};
use super::super::MergeOperationRecordV1;

pub(crate) fn validate_v1_publication(record: &MergeOperationRecordV1) -> ModelResult<()> {
    if record.state == OperationState::Aborted
        && (record.pending_rollback.is_some() || record.pending_preservation.is_some())
    {
        return Err(terminal_rollback_error(record));
    }
    let Some(publication) = record.publication.as_ref() else {
        if record.state == OperationState::Completed {
            return Err(terminal_error(record));
        }
        return Ok(());
    };

    match publication.candidate.as_ref() {
        None => validate_no_candidate(record, publication)?,
        Some(_) => validate_candidate_progress(record, publication)?,
    }
    validate_terminal(record, publication)
}

fn validate_no_candidate(
    record: &MergeOperationRecordV1,
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
        return Err(unexpected_publication(record));
    }
    Ok(())
}

fn validate_candidate_progress(
    record: &MergeOperationRecordV1,
    publication: &PublicationProgress,
) -> ModelResult<()> {
    let preservation_prefix_valid = publication
        .preservation_prefix
        .as_deref()
        .is_none_or(|prefix| matches!(prefix, "baseline" | "marker" | "lock" | "boundary"));
    if !publication_required(record)
        || matches!(
            publication.step,
            PublicationStep::NotStarted | PublicationStep::ValidatingResults
        )
        || super::acceptance::validate_candidate_semantics_for_v1(record).is_err()
        || !preservation_prefix_valid
    {
        return Err(candidate_error(record));
    }

    let composition = composition_shape(record, publication)?;
    let phase_valid = match publication.step {
        PublicationStep::PreparingCandidate => composition == CompositionShape::Absent,
        PublicationStep::CommittingEvidence => true,
        PublicationStep::PublishingCandidate
        | PublicationStep::VerifyingPublication
        | PublicationStep::Complete => composition == CompositionShape::Complete,
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => false,
    };
    if !phase_valid {
        return Err(recorded_evidence_error(record));
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CompositionShape {
    Absent,
    Complete,
}

fn composition_shape(
    record: &MergeOperationRecordV1,
    publication: &PublicationProgress,
) -> ModelResult<CompositionShape> {
    let absent = publication.composition_commit.is_none()
        && publication.composition_tree.is_none()
        && publication.candidate_hashes.is_empty();
    if absent {
        return Ok(CompositionShape::Absent);
    }
    let Some(commit) = publication.composition_commit.as_deref() else {
        return Err(recorded_evidence_error(record));
    };
    let Some(tree) = publication.composition_tree.as_deref() else {
        return Err(recorded_evidence_error(record));
    };
    if !is_oid(commit) || !is_oid(tree) || !candidate_hashes_are_exact(publication) {
        return Err(recorded_evidence_error(record));
    }
    Ok(CompositionShape::Complete)
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
            .all(|(recorded, (path, sha256))| recorded.path == path && recorded.sha256 == sha256)
}

fn validate_terminal(
    record: &MergeOperationRecordV1,
    publication: &PublicationProgress,
) -> ModelResult<()> {
    let complete_evidence = composition_shape(record, publication)? == CompositionShape::Complete;
    if publication.evidence_rolled_back && !complete_evidence {
        return Err(recorded_evidence_error(record));
    }
    if record.state == OperationState::Aborted {
        if publication.candidate.is_some() && publication.evidence_rolled_back != complete_evidence
        {
            return Err(terminal_rollback_error(record));
        }
        return Ok(());
    }
    if record.state != OperationState::Completed {
        let rollback_lifecycle = record.state == OperationState::RollingBack
            || record.state == OperationState::RecoveryRequired
                && record.recovery_context.as_ref().is_some_and(|context| {
                    context.origin_state == super::super::RecoveryOriginStateV1::RollingBack
                });
        if publication.evidence_rolled_back && !rollback_lifecycle {
            return Err(recorded_evidence_error(record));
        }
        return Ok(());
    }
    let candidate_complete = publication.candidate.is_some()
        && publication.step == PublicationStep::Complete
        && !publication.evidence_rolled_back;
    let no_publication_complete = publication.candidate.is_none()
        && publication.step == PublicationStep::Complete
        && !publication_required(record);
    if record.accepted_workspace.is_none() || (!candidate_complete && !no_publication_complete) {
        return Err(terminal_error(record));
    }
    Ok(())
}

fn publication_required(record: &MergeOperationRecordV1) -> bool {
    super::acceptance::publication_required_for_v1(record)
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

fn candidate_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::CandidateIntegrityMismatch,
        "candidate integrity check failed",
        "candidate cross-field identity or hash validation failed",
    )
}

fn recorded_evidence_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::RecordedEvidenceDrift,
        "recorded evidence changed",
        "recorded composition commit, tree, parent, message, files, or hashes changed",
    )
}

fn unexpected_publication(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::UnexpectedPublicationEvidence,
        "has unexpected publication evidence",
        "no-publication completion contains candidate, composition, hash, or published-prefix evidence",
    )
}

fn terminal_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::TerminalEvidenceMismatch,
        "terminal evidence is invalid",
        "completed record is not published or no-publication-complete",
    )
}

fn terminal_rollback_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::TerminalRollbackMismatch,
        "terminal rollback evidence is invalid",
        "aborted record retains an incomplete or contradictory rollback action",
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
