use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::{
    MergeOperationRecord, OperationState, PublicationCandidate, PublicationProgress,
    PublicationStep,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::workspace_ops::merge) enum CandidatePublicationPrefix {
    Baseline,
    Marker,
    Lock,
    Boundary,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct CandidatePublicationObservation {
    lock_sha256: Option<String>,
    marker_sha256: Option<String>,
    boundary_sha256: Option<String>,
}

impl CandidatePublicationObservation {
    pub(in crate::workspace_ops::merge) fn new(
        lock_sha256: Option<String>,
        marker_sha256: Option<String>,
        boundary_sha256: Option<String>,
    ) -> Self {
        Self {
            lock_sha256,
            marker_sha256,
            boundary_sha256,
        }
    }
}

pub(in crate::workspace_ops::merge) fn classify_candidate_publication(
    record: &MergeOperationRecord,
    observation: &CandidatePublicationObservation,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    let candidate = candidate(record)?;
    let publication = progress(record)?;
    let baseline_lock_sha256 =
        super::super::publication::sha256(candidate.baseline_lock_yaml.as_bytes());
    let baseline_lock = observation.lock_sha256.as_deref() == Some(baseline_lock_sha256.as_str());
    let candidate_lock =
        observation.lock_sha256.as_deref() == publication.candidate_lock_sha256.as_deref();
    let marker_absent = observation.marker_sha256.is_none();
    let candidate_marker =
        observation.marker_sha256.as_deref() == Some(candidate.marker_sha256.as_str());
    let baseline_boundary = observation.boundary_sha256.as_deref()
        == Some(candidate.baseline_boundary_sha256.as_str())
        || (observation.boundary_sha256.is_none() && candidate.baseline_boundary_text.is_empty());
    let candidate_boundary =
        observation.boundary_sha256.as_deref() == Some(candidate.boundary_sha256.as_str());

    Ok(if baseline_lock && marker_absent && baseline_boundary {
        Some(CandidatePublicationPrefix::Baseline)
    } else if baseline_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Marker)
    } else if candidate_lock && candidate_marker && candidate_boundary {
        Some(CandidatePublicationPrefix::Boundary)
    } else if candidate_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Lock)
    } else {
        None
    })
}

pub(in crate::workspace_ops::merge) fn publication_prefix_allowed(
    record: &MergeOperationRecord,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    Ok(match progress(record)?.step {
        PublicationStep::NotStarted
        | PublicationStep::ValidatingResults
        | PublicationStep::PreparingCandidate
        | PublicationStep::CommittingEvidence => prefix == CandidatePublicationPrefix::Baseline,
        PublicationStep::PublishingCandidate => true,
        PublicationStep::VerifyingPublication | PublicationStep::Complete => {
            prefix == CandidatePublicationPrefix::Boundary
                || (prefix == CandidatePublicationPrefix::Marker
                    && progress(record)?.candidate_lock_sha256.as_deref()
                        == Some(
                            super::super::publication::sha256(
                                candidate(record)?.baseline_lock_yaml.as_bytes(),
                            )
                            .as_str(),
                        )
                    && candidate(record)?.boundary_sha256
                        == candidate(record)?.baseline_boundary_sha256)
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum FinalizationNextAction {
    ValidateResults,
    PrepareCandidate,
    CreateOrAdoptEvidence,
    PublishCandidate,
    VerifyPublication,
    CompleteNoPublication,
    ArchiveCompleted,
}

pub(in crate::workspace_ops::merge) fn finalization_next_action(
    record: &MergeOperationRecord,
) -> ModelResult<FinalizationNextAction> {
    match record.state {
        OperationState::Completed => return Ok(FinalizationNextAction::ArchiveCompleted),
        OperationState::Finalizing => {}
        _ => {
            return Err(recovery(format!(
                "merge '{}' is not ready for finalization",
                record.merge_id
            )));
        }
    }
    let Some(publication) = record.publication.as_ref() else {
        return Ok(FinalizationNextAction::ValidateResults);
    };
    Ok(match publication.step {
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => {
            FinalizationNextAction::ValidateResults
        }
        PublicationStep::PreparingCandidate if publication.candidate.is_none() => {
            FinalizationNextAction::PrepareCandidate
        }
        PublicationStep::PreparingCandidate => FinalizationNextAction::CreateOrAdoptEvidence,
        PublicationStep::CommittingEvidence if publication.composition_commit.is_none() => {
            FinalizationNextAction::CreateOrAdoptEvidence
        }
        PublicationStep::CommittingEvidence | PublicationStep::PublishingCandidate => {
            FinalizationNextAction::PublishCandidate
        }
        PublicationStep::VerifyingPublication => FinalizationNextAction::VerifyPublication,
        PublicationStep::Complete if publication.candidate.is_none() => {
            FinalizationNextAction::CompleteNoPublication
        }
        PublicationStep::Complete => FinalizationNextAction::VerifyPublication,
    })
}

#[cfg(test)]
pub(crate) fn finalization_next_action_for_i2(
    record: &MergeOperationRecord,
) -> ModelResult<&'static str> {
    Ok(match finalization_next_action(record)? {
        FinalizationNextAction::ValidateResults => "validate_results",
        FinalizationNextAction::PrepareCandidate => "prepare_candidate",
        FinalizationNextAction::CreateOrAdoptEvidence => "create_or_adopt_evidence",
        FinalizationNextAction::PublishCandidate => "publish_candidate",
        FinalizationNextAction::VerifyPublication => "verify_publication",
        FinalizationNextAction::CompleteNoPublication => "complete_no_publication",
        FinalizationNextAction::ArchiveCompleted => "archive_completed",
    })
}

fn progress(record: &MergeOperationRecord) -> ModelResult<&PublicationProgress> {
    record
        .publication
        .as_ref()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

fn candidate(record: &MergeOperationRecord) -> ModelResult<&PublicationCandidate> {
    progress(record)?
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn recovery(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}
