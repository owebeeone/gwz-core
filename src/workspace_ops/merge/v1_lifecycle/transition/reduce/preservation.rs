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
    Ok(())
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
    require(progress_matches(old, new, action))?;
    require(current_position(old, action) == Some(payload.observed_position))?;
    require(match action {
        Action::Stash => {
            next_stash(stash_phase(old)?, has_root_handoff(old)) == Some(stash_phase(new)?)
                && payload.evidence.is_some()
                    == (stash_phase(old)? == PreservationStashPhaseV1::CreateStash)
        }
        Action::Reset => {
            next_reset(reset_phase(old)?, has_root_handoff(old)) == Some(reset_phase(new)?)
                && payload.evidence.is_none()
        }
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

fn progress_matches(
    old: &PendingPreservationActionV1,
    new: &PendingPreservationActionV1,
    expected: Action,
) -> bool {
    match (old, new, expected) {
        (
            PendingPreservationActionV1::Stash {
                owner: old_owner,
                phase: old_phase,
                stash_id: old_stash_id,
                stash_object_id: old_object_id,
                message: old_message,
                head_commit: old_head,
                preimage_sha256: old_preimage,
                root_publication_handoff: old_handoff,
            },
            PendingPreservationActionV1::Stash {
                owner: new_owner,
                stash_id: new_stash_id,
                stash_object_id: new_object_id,
                message: new_message,
                head_commit: new_head,
                preimage_sha256: new_preimage,
                root_publication_handoff: new_handoff,
                ..
            },
            Action::Stash,
        ) => {
            old_owner == new_owner
                && old_message == new_message
                && old_head == new_head
                && old_preimage == new_preimage
                && old_handoff == new_handoff
                && (*old_phase == PreservationStashPhaseV1::CreateStash
                    || old_stash_id == new_stash_id && old_object_id == new_object_id)
        }
        (
            PendingPreservationActionV1::ResetAttachedRef {
                owner: old_owner,
                branch: old_branch,
                expected_commit: old_expected,
                restore_commit: old_restore,
                root_publication_handoff: old_handoff,
                ..
            },
            PendingPreservationActionV1::ResetAttachedRef {
                owner: new_owner,
                branch: new_branch,
                expected_commit: new_expected,
                restore_commit: new_restore,
                root_publication_handoff: new_handoff,
                ..
            },
            Action::Reset,
        ) => {
            old_owner == new_owner
                && old_branch == new_branch
                && old_expected == new_expected
                && old_restore == new_restore
                && old_handoff == new_handoff
        }
        _ => false,
    }
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
                phase: PreservationStashPhaseV1::NormalizeParent,
                root_publication_handoff: Some(_),
                ..
            },
            Action::Stash,
        ) => Some(PreservationCursorPosition::Stash(
            PreservationStashPhaseV1::NormalizeParent,
        )),
        (
            PendingPreservationActionV1::Stash {
                phase: PreservationStashPhaseV1::CreateStash,
                root_publication_handoff: None,
                ..
            },
            Action::Stash,
        ) => Some(PreservationCursorPosition::Stash(
            PreservationStashPhaseV1::CreateStash,
        )),
        (
            PendingPreservationActionV1::ResetAttachedRef {
                phase: PreservationRefResetPhaseV1::PrepareParent,
                root_publication_handoff: Some(_),
                ..
            },
            Action::Reset,
        ) => Some(PreservationCursorPosition::ResetAttachedRef(
            PreservationRefResetPhaseV1::PrepareParent,
        )),
        (
            PendingPreservationActionV1::ResetAttachedRef {
                phase: PreservationRefResetPhaseV1::ResetRef,
                root_publication_handoff: None,
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

fn has_root_handoff(action: &PendingPreservationActionV1) -> bool {
    match action {
        PendingPreservationActionV1::Stash {
            root_publication_handoff,
            ..
        }
        | PendingPreservationActionV1::ResetAttachedRef {
            root_publication_handoff,
            ..
        } => root_publication_handoff.is_some(),
        PendingPreservationActionV1::BackupRef { .. } => false,
    }
}

fn next_stash(
    phase: PreservationStashPhaseV1,
    root_handoff: bool,
) -> Option<PreservationStashPhaseV1> {
    Some(match phase {
        PreservationStashPhaseV1::NormalizeParent if root_handoff => {
            PreservationStashPhaseV1::NormalizeMarker
        }
        PreservationStashPhaseV1::NormalizeMarker if root_handoff => {
            PreservationStashPhaseV1::NormalizeLock
        }
        PreservationStashPhaseV1::NormalizeLock if root_handoff => {
            PreservationStashPhaseV1::NormalizeIndex
        }
        PreservationStashPhaseV1::NormalizeIndex if root_handoff => {
            PreservationStashPhaseV1::CreateStash
        }
        PreservationStashPhaseV1::CreateStash if root_handoff => {
            PreservationStashPhaseV1::RestoreIndex
        }
        PreservationStashPhaseV1::CreateStash => PreservationStashPhaseV1::WriteBundle,
        PreservationStashPhaseV1::RestoreIndex if root_handoff => {
            PreservationStashPhaseV1::RestoreLock
        }
        PreservationStashPhaseV1::RestoreLock if root_handoff => {
            PreservationStashPhaseV1::RestoreParent
        }
        PreservationStashPhaseV1::RestoreParent if root_handoff => {
            PreservationStashPhaseV1::RestoreMarker
        }
        PreservationStashPhaseV1::RestoreMarker if root_handoff => {
            PreservationStashPhaseV1::WriteBundle
        }
        PreservationStashPhaseV1::WriteBundle => PreservationStashPhaseV1::Complete,
        PreservationStashPhaseV1::Complete => return None,
        PreservationStashPhaseV1::NormalizeParent
        | PreservationStashPhaseV1::NormalizeMarker
        | PreservationStashPhaseV1::NormalizeLock
        | PreservationStashPhaseV1::NormalizeIndex
        | PreservationStashPhaseV1::RestoreIndex
        | PreservationStashPhaseV1::RestoreLock
        | PreservationStashPhaseV1::RestoreParent
        | PreservationStashPhaseV1::RestoreMarker => return None,
    })
}

fn next_reset(
    phase: PreservationRefResetPhaseV1,
    root_handoff: bool,
) -> Option<PreservationRefResetPhaseV1> {
    Some(match phase {
        PreservationRefResetPhaseV1::PrepareParent if root_handoff => {
            PreservationRefResetPhaseV1::PrepareMarker
        }
        PreservationRefResetPhaseV1::PrepareMarker if root_handoff => {
            PreservationRefResetPhaseV1::PrepareLock
        }
        PreservationRefResetPhaseV1::PrepareLock if root_handoff => {
            PreservationRefResetPhaseV1::PrepareIndex
        }
        PreservationRefResetPhaseV1::PrepareIndex if root_handoff => {
            PreservationRefResetPhaseV1::ResetRef
        }
        PreservationRefResetPhaseV1::ResetRef if root_handoff => {
            PreservationRefResetPhaseV1::RestoreIndex
        }
        PreservationRefResetPhaseV1::ResetRef => PreservationRefResetPhaseV1::Complete,
        PreservationRefResetPhaseV1::RestoreIndex if root_handoff => {
            PreservationRefResetPhaseV1::RestoreLock
        }
        PreservationRefResetPhaseV1::RestoreLock if root_handoff => {
            PreservationRefResetPhaseV1::RestoreParent
        }
        PreservationRefResetPhaseV1::RestoreParent if root_handoff => {
            PreservationRefResetPhaseV1::RestoreMarker
        }
        PreservationRefResetPhaseV1::RestoreMarker if root_handoff => {
            PreservationRefResetPhaseV1::Complete
        }
        PreservationRefResetPhaseV1::Complete => return None,
        PreservationRefResetPhaseV1::PrepareParent
        | PreservationRefResetPhaseV1::PrepareMarker
        | PreservationRefResetPhaseV1::PrepareLock
        | PreservationRefResetPhaseV1::PrepareIndex
        | PreservationRefResetPhaseV1::RestoreIndex
        | PreservationRefResetPhaseV1::RestoreLock
        | PreservationRefResetPhaseV1::RestoreParent
        | PreservationRefResetPhaseV1::RestoreMarker => return None,
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
