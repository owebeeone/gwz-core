use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1, MergeOperationRecordV1, PendingRollbackActionV1,
    RootMetadataRollbackStepV1,
};

use super::super::super::authority::BoundAuthority;
use super::super::super::checked::StoredV1Record;
use super::super::RollbackTransition;
use super::super::effect::{EffectKind, TransitionEffect};

pub(super) fn rollback(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: RollbackTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    require(current.record().state == OperationState::RollingBack)?;
    match transition {
        RollbackTransition::BeginParticipant(intent) => {
            require(current.record().pending_rollback.is_none())?;
            let pending = intent.value();
            let member_id = participant_owner(pending)?;
            bound(
                &*intent,
                current,
                member_id,
                "begin_participant_rollback",
                "prepared",
            )?;
            next.pending_rollback = Some(pending.clone());
            Ok(TransitionEffect::participant(kind, member_id))
        }
        RollbackTransition::FinishParticipant(proof) => {
            let payload = proof.value();
            require(matches_participant(
                current.record().pending_rollback.as_ref(),
                &payload.member_id,
            ))?;
            bound(
                &*proof,
                current,
                &payload.member_id,
                "finish_participant_rollback",
                "completed",
            )?;
            next.pending_rollback = None;
            next.participants
                .insert(payload.member_id.clone(), payload.row.clone());
            Ok(TransitionEffect::participant(kind, &payload.member_id))
        }
        RollbackTransition::BeginEvidence(intent) => {
            require(current.record().pending_rollback.is_none())?;
            require(matches!(
                intent.value(),
                PendingRollbackActionV1::PublicationEvidence {
                    next_step: EvidenceRollbackStepV1::EvidenceCommit
                }
            ))?;
            bound(
                &*intent,
                current,
                "@publication",
                "begin_evidence_rollback",
                "prepared",
            )?;
            next.pending_rollback = Some(intent.value().clone());
            Ok(TransitionEffect::operation(kind))
        }
        RollbackTransition::AdvanceEvidence(proof) => {
            let old = evidence_step(current.record().pending_rollback.as_ref())?;
            let new = evidence_step(Some(proof.value()))?;
            require(next_evidence(old) == Some(new))?;
            bound(
                &*proof,
                current,
                "@publication",
                "advance_evidence_rollback",
                phase_evidence(old),
            )?;
            next.pending_rollback = Some(proof.value().clone());
            Ok(TransitionEffect::operation(kind))
        }
        RollbackTransition::FinishEvidence(proof) => {
            require(
                evidence_step(current.record().pending_rollback.as_ref())?
                    == EvidenceRollbackStepV1::Complete,
            )?;
            bound(
                &proof,
                current,
                "@publication",
                "finish_evidence_rollback",
                "complete",
            )?;
            next.pending_rollback = None;
            next.publication
                .as_mut()
                .ok_or_else(rejected)?
                .evidence_rolled_back = true;
            Ok(TransitionEffect::operation(kind))
        }
        RollbackTransition::BeginSelectedRoot(intent) => {
            require(current.record().pending_rollback.is_none())?;
            require(matches!(
                intent.value(),
                PendingRollbackActionV1::SelectedRootMetadata {
                    next_step: RootMetadataRollbackStepV1::Manifest
                }
            ))?;
            bound(
                &*intent,
                current,
                "@root",
                "begin_root_metadata_rollback",
                "prepared",
            )?;
            next.pending_rollback = Some(intent.value().clone());
            Ok(TransitionEffect::operation(kind))
        }
        RollbackTransition::AdvanceSelectedRoot(proof) => {
            let old = root_step(current.record().pending_rollback.as_ref())?;
            let new = root_step(Some(proof.value()))?;
            require(next_root(old) == Some(new))?;
            bound(
                &*proof,
                current,
                "@root",
                "advance_root_metadata_rollback",
                phase_root(old),
            )?;
            next.pending_rollback = Some(proof.value().clone());
            Ok(TransitionEffect::operation(kind))
        }
        RollbackTransition::FinishSelectedRoot(proof) => {
            require(
                root_step(current.record().pending_rollback.as_ref())?
                    == RootMetadataRollbackStepV1::Complete,
            )?;
            bound(
                &proof,
                current,
                "@root",
                "finish_root_metadata_rollback",
                "complete",
            )?;
            next.pending_rollback = None;
            Ok(TransitionEffect::operation(kind))
        }
    }
}

fn evidence_step(action: Option<&PendingRollbackActionV1>) -> ModelResult<EvidenceRollbackStepV1> {
    match action {
        Some(PendingRollbackActionV1::PublicationEvidence { next_step }) => Ok(*next_step),
        _ => Err(rejected()),
    }
}

fn root_step(action: Option<&PendingRollbackActionV1>) -> ModelResult<RootMetadataRollbackStepV1> {
    match action {
        Some(PendingRollbackActionV1::SelectedRootMetadata { next_step }) => Ok(*next_step),
        _ => Err(rejected()),
    }
}

fn next_evidence(step: EvidenceRollbackStepV1) -> Option<EvidenceRollbackStepV1> {
    Some(match step {
        EvidenceRollbackStepV1::EvidenceCommit => EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Boundary => EvidenceRollbackStepV1::Lock,
        EvidenceRollbackStepV1::Lock => EvidenceRollbackStepV1::Marker,
        EvidenceRollbackStepV1::Marker => EvidenceRollbackStepV1::Index,
        EvidenceRollbackStepV1::Index => EvidenceRollbackStepV1::Complete,
        EvidenceRollbackStepV1::Complete => return None,
    })
}

fn next_root(step: RootMetadataRollbackStepV1) -> Option<RootMetadataRollbackStepV1> {
    Some(match step {
        RootMetadataRollbackStepV1::Manifest => RootMetadataRollbackStepV1::Lock,
        RootMetadataRollbackStepV1::Lock => RootMetadataRollbackStepV1::Complete,
        RootMetadataRollbackStepV1::Complete => return None,
    })
}

fn phase_evidence(step: EvidenceRollbackStepV1) -> &'static str {
    match step {
        EvidenceRollbackStepV1::EvidenceCommit => "evidence_commit",
        EvidenceRollbackStepV1::Boundary => "boundary",
        EvidenceRollbackStepV1::Lock => "lock",
        EvidenceRollbackStepV1::Marker => "marker",
        EvidenceRollbackStepV1::Index => "index",
        EvidenceRollbackStepV1::Complete => "complete",
    }
}
fn phase_root(step: RootMetadataRollbackStepV1) -> &'static str {
    match step {
        RootMetadataRollbackStepV1::Manifest => "manifest",
        RootMetadataRollbackStepV1::Lock => "lock",
        RootMetadataRollbackStepV1::Complete => "complete",
    }
}

fn participant_owner(action: &PendingRollbackActionV1) -> ModelResult<&str> {
    match action {
        PendingRollbackActionV1::Participant { member_id, .. } => Ok(member_id),
        _ => Err(rejected()),
    }
}

fn matches_participant(action: Option<&PendingRollbackActionV1>, member_id: &str) -> bool {
    matches!(action, Some(PendingRollbackActionV1::Participant { member_id: actual, .. }) if actual == member_id)
}

fn bound(
    value: &impl BoundAuthority,
    current: &StoredV1Record,
    owner: &str,
    action: &str,
    phase: &str,
) -> ModelResult<()> {
    require(value.matches(current, owner, action, phase))
}

fn require(condition: bool) -> ModelResult<()> {
    condition.then_some(()).ok_or_else(rejected)
}
fn rejected() -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        "v1 journal transition predecessor or authority mismatch",
    )
}
