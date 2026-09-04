use crate::artifact::{LockArtifact, MARKER_DIR, MarkerArtifact, MarkerMergeTargetKind};
use crate::model::{ErrorCode, ModelError, ModelResult};
use sha2::{Digest, Sha256};

use super::super::model::v1::{
    AcceptedMetadataSourceV1, AcceptedRootBaseV1, MemberAcceptanceV1, MergeOperationRecordV1,
};
use super::super::{OperationState, PublicationCandidate, PublicationProgress, PublicationStep};

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

#[allow(dead_code, reason = "retained v0 wrapper over the shared view")]
pub(in crate::workspace_ops::merge) fn classify_candidate_publication(
    record: &MergeOperationRecordV1,
    observation: &CandidatePublicationObservation,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    classify_candidate_publication_view(
        super::super::status::MergeStatusRecordView::from_v1(record),
        observation,
    )
}

pub(in crate::workspace_ops::merge) fn classify_candidate_publication_view(
    view: super::super::status::MergeStatusRecordView<'_>,
    observation: &CandidatePublicationObservation,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    let publication = view
        .publication()
        .ok_or_else(|| unreadable("publication progress is missing"))?;
    let candidate = publication
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))?;
    Ok(classify_candidate_parts(
        candidate,
        publication.candidate_lock_sha256.as_deref(),
        observation,
    ))
}

pub(in crate::workspace_ops::merge) fn classify_candidate_publication_for_v1(
    record: &MergeOperationRecordV1,
    observation: &CandidatePublicationObservation,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    classify_candidate_publication_view(
        super::super::status::MergeStatusRecordView::from_v1(record),
        observation,
    )
}

fn classify_candidate_parts(
    candidate: &PublicationCandidate,
    candidate_lock_sha256: Option<&str>,
    observation: &CandidatePublicationObservation,
) -> Option<CandidatePublicationPrefix> {
    let baseline_lock_sha256 =
        super::super::publication::sha256(candidate.baseline_lock_yaml.as_bytes());
    let baseline_lock = observation.lock_sha256.as_deref() == Some(baseline_lock_sha256.as_str());
    let candidate_lock = observation.lock_sha256.as_deref() == candidate_lock_sha256;
    let marker_absent = observation.marker_sha256.is_none();
    let candidate_marker =
        observation.marker_sha256.as_deref() == Some(candidate.marker_sha256.as_str());
    let baseline_boundary = observation.boundary_sha256.as_deref()
        == Some(candidate.baseline_boundary_sha256.as_str())
        || (observation.boundary_sha256.is_none() && candidate.baseline_boundary_text.is_empty());
    let candidate_boundary =
        observation.boundary_sha256.as_deref() == Some(candidate.boundary_sha256.as_str());

    if baseline_lock && marker_absent && baseline_boundary {
        Some(CandidatePublicationPrefix::Baseline)
    } else if baseline_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Marker)
    } else if candidate_lock && candidate_marker && candidate_boundary {
        Some(CandidatePublicationPrefix::Boundary)
    } else if candidate_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Lock)
    } else {
        None
    }
}

#[allow(dead_code, reason = "retained v0 wrapper over the shared view")]
pub(in crate::workspace_ops::merge) fn publication_prefix_allowed(
    record: &MergeOperationRecordV1,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    publication_prefix_allowed_view(
        super::super::status::MergeStatusRecordView::from_v1(record),
        prefix,
    )
}

pub(in crate::workspace_ops::merge) fn publication_prefix_allowed_view(
    view: super::super::status::MergeStatusRecordView<'_>,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    let publication = view
        .publication()
        .ok_or_else(|| unreadable("publication progress is missing"))?;
    let candidate = || {
        publication
            .candidate
            .as_ref()
            .ok_or_else(|| unreadable("publication candidate is missing"))
    };
    Ok(match publication.step {
        PublicationStep::NotStarted
        | PublicationStep::ValidatingResults
        | PublicationStep::PreparingCandidate
        | PublicationStep::CommittingEvidence => prefix == CandidatePublicationPrefix::Baseline,
        PublicationStep::PublishingCandidate => true,
        PublicationStep::VerifyingPublication | PublicationStep::Complete => {
            prefix == CandidatePublicationPrefix::Boundary
                || (prefix == CandidatePublicationPrefix::Marker
                    && publication.candidate_lock_sha256.as_deref()
                        == Some(
                            super::super::publication::sha256(
                                candidate()?.baseline_lock_yaml.as_bytes(),
                            )
                            .as_str(),
                        )
                    && candidate()?.boundary_sha256 == candidate()?.baseline_boundary_sha256)
        }
    })
}

pub(in crate::workspace_ops::merge) fn publication_required_for_v1(
    record: &MergeOperationRecordV1,
) -> bool {
    record.participants.values().any(|participant| {
        participant
            .resulting_commit
            .as_deref()
            .is_some_and(|result| result != participant.before_commit)
    })
}

pub(in crate::workspace_ops::merge) fn validate_candidate_semantics_for_v1(
    record: &MergeOperationRecordV1,
) -> ModelResult<()> {
    let publication = record
        .publication
        .as_ref()
        .ok_or_else(|| candidate_error(record))?;
    let candidate = publication
        .candidate
        .as_ref()
        .ok_or_else(|| candidate_error(record))?;
    let lock_sha256 = digest(&candidate.lock_yaml);
    let marker_path = format!("{MARKER_DIR}/{}.yaml", candidate.marker_id);
    let selected_root_result = record
        .selected_targets
        .iter()
        .any(|target| target == "@root")
        .then(|| record.participants.get("@root"))
        .flatten()
        .and_then(|root| root.resulting_commit.as_deref());
    let evidence_complete = publication.composition_commit.is_some()
        && publication.composition_tree.is_some()
        && !publication.candidate_hashes.is_empty();
    let recorded_root_result = evidence_complete.then_some(selected_root_result).flatten();
    if publication.candidate_lock_sha256.as_deref() != Some(lock_sha256.as_str())
        || publication.candidate_marker_path.as_deref() != Some(marker_path.as_str())
        || publication.root_merge_commit.as_deref() != recorded_root_result
        || (selected_root_result.is_none()
            && digest(&candidate.baseline_lock_yaml) != record.baseline.lock_sha256)
        || candidate.baseline_boundary_sha256 != digest(&candidate.baseline_boundary_text)
        || candidate.marker_sha256 != digest(&candidate.marker_yaml)
        || candidate.boundary_sha256 != digest(&candidate.boundary_text)
        || record
            .accepted_workspace
            .as_ref()
            .and_then(|accepted| accepted.root.publication_branch.as_deref())
            != Some(candidate.root_branch.as_str())
    {
        return Err(candidate_error(record));
    }
    let baseline_lock = LockArtifact::from_yaml(&candidate.baseline_lock_yaml)
        .map_err(|_| candidate_error(record))?;
    let lock =
        LockArtifact::from_yaml(&candidate.lock_yaml).map_err(|_| candidate_error(record))?;
    let marker =
        MarkerArtifact::from_yaml(&candidate.marker_yaml).map_err(|_| candidate_error(record))?;
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| candidate_error(record))?;
    let root_before = match &accepted.root.base {
        AcceptedRootBaseV1::BornAttached { commit, .. }
        | AcceptedRootBaseV1::BornDetached { commit } => Some(commit.as_str()),
        AcceptedRootBaseV1::UnbornAttached { .. } => None,
    };
    let mut committed_targets = Vec::new();
    for target in &record.selected_targets {
        let row = record
            .participants
            .get(target)
            .ok_or_else(|| candidate_error(record))?;
        let result = accepted_result(record, target)?;
        if result != row.before_commit {
            committed_targets.push(target.clone());
        }
    }
    if !committed_targets.iter().any(|target| target == "@root") {
        committed_targets.push("@root".into());
    }
    if candidate.baseline_lock_yaml != accepted.metadata_base.lock_exact_yaml
        || candidate.lock_yaml != accepted.lock.exact_yaml
        || baseline_lock.workspace_id != record.workspace_id
        || lock.workspace_id != record.workspace_id
        || marker.workspace_id != record.workspace_id
        || marker.gwz_commit_id != candidate.marker_id
        || marker.origin_url_hash.is_some()
        || marker.created_at != record.created_at
        || marker.created_by.actor_id != candidate.actor_id
        || marker.root.path != "."
        || marker.root.before_commit.as_deref() != root_before
        || marker.root.branch.as_deref() != Some(candidate.root_branch.as_str())
        || marker.selected_targets != record.selected_targets
        || marker.committed_targets != committed_targets
        || marker.members != lock.members
        || !marker_merge_is_exact(record, &marker)
    {
        return Err(candidate_error(record));
    }
    Ok(())
}

fn accepted_result<'a>(record: &'a MergeOperationRecordV1, target: &str) -> ModelResult<&'a str> {
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| candidate_error(record))?;
    if target == "@root" {
        let AcceptedMetadataSourceV1::SelectedRootResult { commit } =
            &accepted.metadata_base.source
        else {
            return Err(candidate_error(record));
        };
        return Ok(commit);
    }
    let Some(MemberAcceptanceV1::Selected { integration, .. }) = accepted.member_audit.get(target)
    else {
        return Err(candidate_error(record));
    };
    Ok(&integration.resulting_commit)
}

fn marker_merge_is_exact(record: &MergeOperationRecordV1, marker: &MarkerArtifact) -> bool {
    let Some(merge) = marker.merge.as_ref() else {
        return false;
    };
    if merge.merge_id != record.merge_id
        || merge.operation_id != record.operation_id
        || merge.source_ref != record.source_ref
        || merge.selected_targets != record.selected_targets
        || merge.participants.len() != record.selected_targets.len()
    {
        return false;
    }
    let mut root_result = None;
    for target in &record.selected_targets {
        let Some(durable) = record.participants.get(target) else {
            return false;
        };
        let Ok(result) = accepted_result(record, target) else {
            return false;
        };
        let Some(row) = merge.participants.get(target) else {
            return false;
        };
        let target_kind = if target == "@root" {
            root_result = Some(result);
            MarkerMergeTargetKind::Root
        } else {
            MarkerMergeTargetKind::Member
        };
        if row.target_kind != target_kind
            || row.target_branch != durable.target_branch
            || row.before_commit != durable.before_commit
            || row.source_commit != durable.source_commit
            || row.resulting_commit != result
        {
            return false;
        }
    }
    merge.root_merge_commit.as_deref() == root_result
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn candidate_error(record: &MergeOperationRecordV1) -> ModelError {
    ModelError::new(
        ErrorCode::CandidateIntegrityMismatch,
        format!(
            "merge record '{}' candidate integrity check failed: candidate bytes or digest do not match accepted workspace",
            record.merge_id
        ),
    )
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

fn finalization_next_action_from_parts(
    state: OperationState,
    merge_id: &str,
    publication: Option<&PublicationProgress>,
) -> ModelResult<FinalizationNextAction> {
    match state {
        OperationState::Completed => return Ok(FinalizationNextAction::ArchiveCompleted),
        OperationState::Finalizing => {}
        _ => {
            return Err(recovery(format!(
                "merge '{}' is not ready for finalization",
                merge_id
            )));
        }
    }
    let Some(publication) = publication else {
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

pub(crate) fn finalization_next_action_for_v1(
    record: &super::super::model::v1::MergeOperationRecordV1,
) -> ModelResult<&'static str> {
    let state = if record.state == OperationState::RecoveryRequired
        && record.recovery_context.as_ref().is_some_and(|context| {
            context.origin_state == super::super::model::v1::RecoveryOriginStateV1::Finalizing
        }) {
        OperationState::Finalizing
    } else {
        record.state
    };
    Ok(action_name(finalization_next_action_from_parts(
        state,
        &record.merge_id,
        record.publication.as_ref(),
    )?))
}

fn action_name(action: FinalizationNextAction) -> &'static str {
    match action {
        FinalizationNextAction::ValidateResults => "validate_results",
        FinalizationNextAction::PrepareCandidate => "prepare_candidate",
        FinalizationNextAction::CreateOrAdoptEvidence => "create_or_adopt_evidence",
        FinalizationNextAction::PublishCandidate => "publish_candidate",
        FinalizationNextAction::VerifyPublication => "verify_publication",
        FinalizationNextAction::CompleteNoPublication => "complete_no_publication",
        FinalizationNextAction::ArchiveCompleted => "archive_completed",
    }
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn recovery(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}
