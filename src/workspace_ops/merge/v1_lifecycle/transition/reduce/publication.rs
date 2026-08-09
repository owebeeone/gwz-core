use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;
use crate::workspace_ops::merge::{OperationState, PublicationProgress, PublicationStep};

use super::super::super::authority::BoundAuthority;
use super::super::super::checked::StoredV1Record;
use super::super::PublicationTransition;
use super::super::effect::{EffectKind, TransitionEffect};

pub(super) fn apply(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: PublicationTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    require(current.record().state == OperationState::Finalizing)?;
    require(current.record().accepted_workspace.is_some())?;
    match transition {
        PublicationTransition::ClassifyRequired(decision) => {
            require(current.record().publication.is_none() && *decision.value())?;
            bound(&decision, current, "classify_publication", "required")?;
            next.publication = Some(empty_progress(PublicationStep::PreparingCandidate));
        }
        PublicationTransition::ClassifyNone(decision) => {
            require(current.record().publication.is_none() && !*decision.value())?;
            bound(&decision, current, "classify_publication", "none")?;
            next.publication = Some(empty_progress(PublicationStep::Complete));
        }
        PublicationTransition::BeginMigratedValidation => {
            require(progress(current)?.step == PublicationStep::NotStarted)?;
            progress_mut(next)?.step = PublicationStep::ValidatingResults;
        }
        PublicationTransition::ClassifyMigratedRequired(proof) => {
            require(
                progress(current)?.step == PublicationStep::ValidatingResults && *proof.value(),
            )?;
            bound(
                &proof,
                current,
                "validate_migrated_results",
                "publication_required",
            )?;
            progress_mut(next)?.step = PublicationStep::PreparingCandidate;
        }
        PublicationTransition::ClassifyMigratedNone(proof) => {
            require(
                progress(current)?.step == PublicationStep::ValidatingResults && !*proof.value(),
            )?;
            bound(
                &proof,
                current,
                "validate_migrated_results",
                "no_publication",
            )?;
            progress_mut(next)?.step = PublicationStep::Complete;
        }
        PublicationTransition::RecordCandidate(candidate) => {
            let old = progress(current)?;
            require(old.step == PublicationStep::PreparingCandidate && old.candidate.is_none())?;
            bound(&*candidate, current, "prepare_candidate", "prepared")?;
            let value = candidate.value();
            let publication = progress_mut(next)?;
            publication.candidate = Some(value.candidate.clone());
            publication.candidate_marker_path = Some(value.marker_path.clone());
            publication.candidate_lock_sha256 = Some(value.lock_sha256.clone());
        }
        PublicationTransition::BeginEvidence(intent) => {
            let publication = progress(current)?;
            require(
                publication.step == PublicationStep::PreparingCandidate
                    && publication.candidate.is_some(),
            )?;
            require(publication.composition_commit.is_none())?;
            bound(&intent, current, "begin_evidence", "preflight")?;
            progress_mut(next)?.step = PublicationStep::CommittingEvidence;
        }
        PublicationTransition::RecordEvidence(proof) => {
            let publication = progress(current)?;
            require(
                publication.step == PublicationStep::CommittingEvidence
                    && publication.composition_commit.is_none(),
            )?;
            bound(&*proof, current, "record_evidence", "completed")?;
            let value = proof.value();
            let publication = progress_mut(next)?;
            publication.composition_commit = Some(value.composition_commit.clone());
            publication.composition_tree = Some(value.composition_tree.clone());
            publication.root_merge_commit = value.root_merge_commit.clone();
            publication
                .candidate_hashes
                .clone_from(&value.candidate_hashes);
        }
        PublicationTransition::BeginCandidatePublication(intent) => {
            let publication = progress(current)?;
            require(
                publication.step == PublicationStep::CommittingEvidence
                    && complete_evidence(publication),
            )?;
            bound(&intent, current, "begin_candidate_publication", "preflight")?;
            progress_mut(next)?.step = PublicationStep::PublishingCandidate;
        }
        PublicationTransition::RecordCandidatePublished(proof) => {
            require(progress(current)?.step == PublicationStep::PublishingCandidate)?;
            bound(&proof, current, "candidate_publication", "completed")?;
            progress_mut(next)?.step = PublicationStep::VerifyingPublication;
        }
        PublicationTransition::RecordPublicationVerified(proof) => {
            require(progress(current)?.step == PublicationStep::VerifyingPublication)?;
            bound(&proof, current, "publication", "verified")?;
            progress_mut(next)?.step = PublicationStep::Complete;
        }
    }
    Ok(TransitionEffect::operation(kind))
}

fn empty_progress(step: PublicationStep) -> PublicationProgress {
    PublicationProgress {
        step,
        candidate_lock_sha256: None,
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}

fn complete_evidence(publication: &PublicationProgress) -> bool {
    publication.candidate.is_some()
        && publication.composition_commit.is_some()
        && publication.composition_tree.is_some()
        && !publication.candidate_hashes.is_empty()
}

fn progress(current: &StoredV1Record) -> ModelResult<&PublicationProgress> {
    current.record().publication.as_ref().ok_or_else(rejected)
}

fn progress_mut(record: &mut MergeOperationRecordV1) -> ModelResult<&mut PublicationProgress> {
    record.publication.as_mut().ok_or_else(rejected)
}

fn bound(
    value: &impl BoundAuthority,
    current: &StoredV1Record,
    action: &str,
    phase: &str,
) -> ModelResult<()> {
    require(value.matches(current, "@publication", action, phase))
}

fn require(condition: bool) -> ModelResult<()> {
    condition.then_some(()).ok_or_else(rejected)
}

fn rejected() -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        "v1 publication transition predecessor or authority mismatch",
    )
}
