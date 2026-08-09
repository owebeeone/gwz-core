use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1,
};
use crate::workspace_ops::merge::{OperationState, PreservationEvidence};

use super::super::super::authority::{
    BoundAuthority, PreservationCursorPosition, PreservationPayload,
};
use super::super::super::checked::StoredV1Record;
use super::super::PreservationTransition;
use super::super::effect::{EffectKind, TransitionEffect};

pub(super) fn apply(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: PreservationTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    require(current.record().state == OperationState::Preserving)?;
    let owner = match transition {
        PreservationTransition::BeginBackupRef(intent) => {
            begin(current, next, &*intent, intent.value(), Action::Backup)?;
            intent.value().owner.clone()
        }
        PreservationTransition::FinishBackupRef(proof) => {
            finish(current, next, &*proof, proof.value(), Action::Backup)?;
            proof.value().owner.clone()
        }
        PreservationTransition::BeginStash(intent) => {
            begin(current, next, &*intent, intent.value(), Action::Stash)?;
            intent.value().owner.clone()
        }
        PreservationTransition::AdvanceStash(proof) => {
            advance(current, next, &*proof, proof.value(), Action::Stash)?;
            proof.value().owner.clone()
        }
        PreservationTransition::FinishStash(proof) => {
            finish(current, next, &*proof, proof.value(), Action::Stash)?;
            proof.value().owner.clone()
        }
        PreservationTransition::BeginResetAttachedRef(intent) => {
            begin(current, next, &*intent, intent.value(), Action::Reset)?;
            intent.value().owner.clone()
        }
        PreservationTransition::AdvanceResetAttachedRef(proof) => {
            advance(current, next, &*proof, proof.value(), Action::Reset)?;
            proof.value().owner.clone()
        }
        PreservationTransition::FinishResetAttachedRef(proof) => {
            finish(current, next, &*proof, proof.value(), Action::Reset)?;
            proof.value().owner.clone()
        }
    };
    Ok(TransitionEffect::preservation(kind, owner))
}

#[derive(Clone, Copy)]
enum Action {
    Backup,
    Stash,
    Reset,
}

fn begin(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    token: &impl BoundAuthority,
    payload: &PreservationPayload,
    action: Action,
) -> ModelResult<()> {
    require(current.record().pending_preservation.is_none() && payload.evidence.is_none())?;
    let pending = payload.pending.as_ref().ok_or_else(rejected)?;
    require(action_matches(pending, &payload.owner, action))?;
    require(begin_position(pending, action) == Some(payload.observed_position))?;
    let action_name = match action {
        Action::Backup => "begin_backup_ref",
        Action::Stash => "begin_stash",
        Action::Reset => "begin_reset_attached_ref",
    };
    bound(
        token,
        current,
        owner_id(&payload.owner),
        action_name,
        "cursor_checked",
    )?;
    next.pending_preservation = Some(pending.clone());
    install_prefix(next, payload)
}

fn advance(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    token: &impl BoundAuthority,
    payload: &PreservationPayload,
    action: Action,
) -> ModelResult<()> {
    let old = current
        .record()
        .pending_preservation
        .as_ref()
        .ok_or_else(rejected)?;
    let new = payload.pending.as_ref().ok_or_else(rejected)?;
    require(
        action_matches(old, &payload.owner, action) && action_matches(new, &payload.owner, action),
    )?;
    require(current_position(old, action) == Some(payload.observed_position))?;
    require(match action {
        Action::Stash => next_stash(stash_phase(old)?, has_prefix(old)) == Some(stash_phase(new)?),
        Action::Reset => next_reset(reset_phase(old)?, has_prefix(old)) == Some(reset_phase(new)?),
        Action::Backup => false,
    })?;
    let name = match action {
        Action::Stash => "advance_stash",
        Action::Reset => "advance_reset_attached_ref",
        Action::Backup => unreachable!(),
    };
    bound(token, current, owner_id(&payload.owner), name, "completed")?;
    next.pending_preservation = Some(new.clone());
    if let Some(evidence) = payload.evidence.clone() {
        set_evidence(next, &payload.owner, evidence)?;
    }
    Ok(())
}

fn finish(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    token: &impl BoundAuthority,
    payload: &PreservationPayload,
    action: Action,
) -> ModelResult<()> {
    let old = current
        .record()
        .pending_preservation
        .as_ref()
        .ok_or_else(rejected)?;
    require(action_matches(old, &payload.owner, action))?;
    require(current_position(old, action) == Some(payload.observed_position))?;
    require(match action {
        Action::Backup => payload.evidence.is_some(),
        Action::Stash => {
            stash_phase(old)? == PreservationStashPhaseV1::Complete && payload.evidence.is_none()
        }
        Action::Reset => {
            reset_phase(old)? == PreservationRefResetPhaseV1::Complete && payload.evidence.is_none()
        }
    })?;
    let name = match action {
        Action::Backup => "finish_backup_ref",
        Action::Stash => "finish_stash",
        Action::Reset => "finish_reset_attached_ref",
    };
    bound(token, current, owner_id(&payload.owner), name, "completed")?;
    next.pending_preservation = None;
    if let Some(evidence) = payload.evidence.clone() {
        set_evidence(next, &payload.owner, evidence)?;
    }
    Ok(())
}

fn set_evidence(
    record: &mut MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
    evidence: PreservationEvidence,
) -> ModelResult<()> {
    let rows = match owner {
        PreservationOwnerV1::Participant { member_id } => {
            &mut record
                .participants
                .get_mut(member_id)
                .ok_or_else(rejected)?
                .preservation
        }
        PreservationOwnerV1::PublicationRoot => {
            &mut record
                .publication
                .as_mut()
                .ok_or_else(rejected)?
                .root_preservation
        }
    };
    if rows.is_empty() {
        rows.push(evidence)
    } else {
        require(rows.len() == 1)?;
        rows[0] = evidence
    }
    Ok(())
}

fn install_prefix(
    record: &mut MergeOperationRecordV1,
    payload: &PreservationPayload,
) -> ModelResult<()> {
    let Some(prefix) = payload.publication_prefix.as_ref() else {
        return Ok(());
    };
    let publication = record.publication.as_mut().ok_or_else(rejected)?;
    require(
        publication
            .preservation_prefix
            .as_ref()
            .is_none_or(|old| old == prefix),
    )?;
    publication.preservation_prefix = Some(prefix.clone());
    Ok(())
}

fn action_matches(
    action: &PendingPreservationActionV1,
    owner: &PreservationOwnerV1,
    expected: Action,
) -> bool {
    match (action, expected) {
        (PendingPreservationActionV1::BackupRef { owner: actual, .. }, Action::Backup)
        | (PendingPreservationActionV1::Stash { owner: actual, .. }, Action::Stash)
        | (PendingPreservationActionV1::ResetAttachedRef { owner: actual, .. }, Action::Reset) => {
            actual == owner
        }
        _ => false,
    }
}

fn begin_position(
    action: &PendingPreservationActionV1,
    expected: Action,
) -> Option<PreservationCursorPosition> {
    match (action, expected) {
        (PendingPreservationActionV1::BackupRef { .. }, Action::Backup) => {
            Some(PreservationCursorPosition::BackupRef)
        }
        (
            PendingPreservationActionV1::Stash {
                phase: PreservationStashPhaseV1::NormalizeRoot,
                root_publication_prefix: Some(_),
                ..
            },
            Action::Stash,
        ) => Some(PreservationCursorPosition::Stash(
            PreservationStashPhaseV1::NormalizeRoot,
        )),
        (
            PendingPreservationActionV1::Stash {
                phase: PreservationStashPhaseV1::CreateStash,
                root_publication_prefix: None,
                ..
            },
            Action::Stash,
        ) => Some(PreservationCursorPosition::Stash(
            PreservationStashPhaseV1::CreateStash,
        )),
        (
            PendingPreservationActionV1::ResetAttachedRef {
                phase: PreservationRefResetPhaseV1::ResetRef,
                ..
            },
            Action::Reset,
        ) => Some(PreservationCursorPosition::ResetAttachedRef(
            PreservationRefResetPhaseV1::ResetRef,
        )),
        _ => None,
    }
}

fn current_position(
    action: &PendingPreservationActionV1,
    expected: Action,
) -> Option<PreservationCursorPosition> {
    match (action, expected) {
        (PendingPreservationActionV1::BackupRef { .. }, Action::Backup) => {
            Some(PreservationCursorPosition::BackupRef)
        }
        (PendingPreservationActionV1::Stash { phase, .. }, Action::Stash) => {
            Some(PreservationCursorPosition::Stash(*phase))
        }
        (PendingPreservationActionV1::ResetAttachedRef { phase, .. }, Action::Reset) => {
            Some(PreservationCursorPosition::ResetAttachedRef(*phase))
        }
        _ => None,
    }
}

fn stash_phase(action: &PendingPreservationActionV1) -> ModelResult<PreservationStashPhaseV1> {
    match action {
        PendingPreservationActionV1::Stash { phase, .. } => Ok(*phase),
        _ => Err(rejected()),
    }
}

fn reset_phase(action: &PendingPreservationActionV1) -> ModelResult<PreservationRefResetPhaseV1> {
    match action {
        PendingPreservationActionV1::ResetAttachedRef { phase, .. } => Ok(*phase),
        _ => Err(rejected()),
    }
}

fn has_prefix(action: &PendingPreservationActionV1) -> bool {
    match action {
        PendingPreservationActionV1::Stash {
            root_publication_prefix,
            ..
        }
        | PendingPreservationActionV1::ResetAttachedRef {
            root_publication_prefix,
            ..
        } => root_publication_prefix.is_some(),
        PendingPreservationActionV1::BackupRef { .. } => false,
    }
}

fn next_stash(phase: PreservationStashPhaseV1, prefix: bool) -> Option<PreservationStashPhaseV1> {
    Some(match phase {
        PreservationStashPhaseV1::NormalizeRoot => PreservationStashPhaseV1::CreateStash,
        PreservationStashPhaseV1::CreateStash if prefix => PreservationStashPhaseV1::RestoreRoot,
        PreservationStashPhaseV1::CreateStash => PreservationStashPhaseV1::WriteBundle,
        PreservationStashPhaseV1::RestoreRoot => PreservationStashPhaseV1::WriteBundle,
        PreservationStashPhaseV1::WriteBundle => PreservationStashPhaseV1::Complete,
        PreservationStashPhaseV1::Complete => return None,
    })
}

fn next_reset(
    phase: PreservationRefResetPhaseV1,
    prefix: bool,
) -> Option<PreservationRefResetPhaseV1> {
    Some(match phase {
        PreservationRefResetPhaseV1::ResetRef if prefix => PreservationRefResetPhaseV1::RestoreRoot,
        PreservationRefResetPhaseV1::ResetRef | PreservationRefResetPhaseV1::RestoreRoot => {
            PreservationRefResetPhaseV1::Complete
        }
        PreservationRefResetPhaseV1::Complete => return None,
    })
}

fn owner_id(owner: &PreservationOwnerV1) -> &str {
    match owner {
        PreservationOwnerV1::Participant { member_id } => member_id,
        PreservationOwnerV1::PublicationRoot => "@publication-root",
    }
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
        "v1 preservation transition predecessor or authority mismatch",
    )
}
