use super::super::*;
use crate::git::{
    GitRootManagedFormName, GitRootManagedObject, GitRootManagedTransition,
    GitRootPreservationGuard, GitRootPreservationPhysicalStep,
};
use crate::workspace_ops::merge::model::v1::{
    GitObjectIdV1, PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn stash_step(
    phase: S,
    merge_id: &str,
) -> ModelResult<GitRootPreservationPhysicalStep> {
    let managed = |object, source, goal| {
        GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object,
            source,
            goal,
        })
    };
    Ok(match phase {
        S::NormalizeParent => managed(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        S::NormalizeMarker => managed(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        S::NormalizeLock => managed(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        S::NormalizeIndex => managed(
            GitRootManagedObject::Index,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        S::CreateStash => GitRootPreservationPhysicalStep::CreateStash {
            merge_id: merge_id.into(),
        },
        S::RestoreIndex => managed(
            GitRootManagedObject::Index,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        S::RestoreLock => managed(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        S::RestoreParent => managed(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        S::RestoreMarker => managed(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        S::WriteBundle | S::Complete => {
            return Err(preservation_error("bundle phase has no root Git step"));
        }
    })
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn reset_step(
    phase: R,
) -> ModelResult<GitRootPreservationPhysicalStep> {
    let managed = |object, source, goal| {
        GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object,
            source,
            goal,
        })
    };
    Ok(match phase {
        R::PrepareParent => managed(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        R::PrepareMarker => managed(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        R::PrepareLock => managed(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        R::PrepareIndex => managed(
            GitRootManagedObject::Index,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        R::ResetRef => GitRootPreservationPhysicalStep::ResetAttachedRef,
        R::RestoreIndex => managed(
            GitRootManagedObject::Index,
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff,
        ),
        R::RestoreLock => managed(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff,
        ),
        R::RestoreParent => managed(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff,
        ),
        R::RestoreMarker => managed(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff,
        ),
        R::Complete => {
            return Err(preservation_error("complete reset has no Git step"));
        }
    })
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn stash_guard(
    phase: S,
    preimage_sha256: &str,
) -> GitRootPreservationGuard {
    if matches!(
        phase,
        S::NormalizeParent
            | S::NormalizeMarker
            | S::NormalizeLock
            | S::NormalizeIndex
            | S::CreateStash
    ) {
        GitRootPreservationGuard::NormalizedPreimage {
            sha256: preimage_sha256.into(),
        }
    } else {
        GitRootPreservationGuard::OtherwiseClean
    }
}

pub(super) fn next_stash(phase: S, root: bool) -> ModelResult<S> {
    Ok(match phase {
        S::NormalizeParent if root => S::NormalizeMarker,
        S::NormalizeMarker if root => S::NormalizeLock,
        S::NormalizeLock if root => S::NormalizeIndex,
        S::NormalizeIndex if root => S::CreateStash,
        S::CreateStash if root => S::RestoreIndex,
        S::CreateStash => S::WriteBundle,
        S::RestoreIndex if root => S::RestoreLock,
        S::RestoreLock if root => S::RestoreParent,
        S::RestoreParent if root => S::RestoreMarker,
        S::RestoreMarker if root => S::WriteBundle,
        S::WriteBundle => S::Complete,
        _ => {
            return Err(preservation_error("stash phase has no legal successor"));
        }
    })
}

pub(super) fn next_reset(phase: R, root: bool) -> ModelResult<R> {
    Ok(match phase {
        R::PrepareParent if root => R::PrepareMarker,
        R::PrepareMarker if root => R::PrepareLock,
        R::PrepareLock if root => R::PrepareIndex,
        R::PrepareIndex if root => R::ResetRef,
        R::ResetRef if root => R::RestoreIndex,
        R::ResetRef => R::Complete,
        R::RestoreIndex if root => R::RestoreLock,
        R::RestoreLock if root => R::RestoreParent,
        R::RestoreParent if root => R::RestoreMarker,
        R::RestoreMarker if root => R::Complete,
        _ => {
            return Err(preservation_error("reset phase has no legal successor"));
        }
    })
}

pub(super) fn with_stash_phase(
    action: &PendingPreservationActionV1,
    phase: S,
    stash_id: Option<String>,
    stash_object_id: Option<GitObjectIdV1>,
) -> ModelResult<PendingPreservationActionV1> {
    let PendingPreservationActionV1::Stash {
        owner,
        message,
        head_commit,
        preimage_sha256,
        root_publication_handoff,
        ..
    } = action
    else {
        return Err(preservation_error(
            "stash successor received another action",
        ));
    };
    Ok(PendingPreservationActionV1::Stash {
        owner: owner.clone(),
        phase,
        stash_id,
        stash_object_id,
        message: message.clone(),
        head_commit: head_commit.clone(),
        preimage_sha256: preimage_sha256.clone(),
        root_publication_handoff: *root_publication_handoff,
    })
}

pub(super) fn with_reset_phase(
    action: &PendingPreservationActionV1,
    phase: R,
) -> ModelResult<PendingPreservationActionV1> {
    let PendingPreservationActionV1::ResetAttachedRef {
        owner,
        branch,
        expected_commit,
        restore_commit,
        root_publication_handoff,
        ..
    } = action
    else {
        return Err(preservation_error(
            "reset successor received another action",
        ));
    };
    Ok(PendingPreservationActionV1::ResetAttachedRef {
        owner: owner.clone(),
        branch: branch.clone(),
        expected_commit: expected_commit.clone(),
        restore_commit: restore_commit.clone(),
        phase,
        root_publication_handoff: *root_publication_handoff,
    })
}
