use super::super::super::*;
use crate::git::{
    GitBackend, GitRootManagedFormName, GitRootManagedObject, GitRootManagedTransition,
    GitRootPreservationPhysicalStep, GitRootPreservationStepObservation,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn observe<B: GitBackend>(
    _backend: &B,
    _context: &OperationContext,
    current: &StoredV1Record,
    _request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    // Keep the accepted phase-to-physical mapping compiled at the disabled seam
    // without enabling preservation observation before P1.
    let _mapped_parent_step = current
        .record()
        .pending_preservation
        .as_ref()
        .and_then(preclean_parent_step);
    Err(ModelError::new(
        ErrorCode::MergePhaseUnsupported,
        "preservation observation is not implemented",
    ))
}

fn preclean_parent_step(
    action: &PendingPreservationActionV1,
) -> Option<GitRootPreservationPhysicalStep> {
    matches!(
        action,
        PendingPreservationActionV1::Stash {
            phase: PreservationStashPhaseV1::NormalizeParent,
            ..
        } | PendingPreservationActionV1::ResetAttachedRef {
            phase: PreservationRefResetPhaseV1::PrepareParent,
            ..
        }
    )
    .then(|| {
        GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object: GitRootManagedObject::MarkerParentDirectory,
            source: GitRootManagedFormName::Handoff,
            goal: GitRootManagedFormName::AttachedClean,
        })
    })
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn durability_fact(
    observation: GitRootPreservationStepObservation,
    completion: PreservationObservation,
    prefix: VerifiedPreservationCursorPrefix,
    action: PendingPreservationActionV1,
) -> ModelResult<ExactObservationFact> {
    if observation != GitRootPreservationStepObservation::AfterNeedsDurability {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation durability fact requires exact final-only pending structure",
        ));
    }
    Ok(ExactObservationFact::PreservationDurabilityPending {
        completion,
        prefix,
        action,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preclean_parent_phases_map_to_the_same_managed_transition() {
        let expected = Some(GitRootPreservationPhysicalStep::Managed(
            GitRootManagedTransition {
                object: GitRootManagedObject::MarkerParentDirectory,
                source: GitRootManagedFormName::Handoff,
                goal: GitRootManagedFormName::AttachedClean,
            },
        ));
        assert_eq!(
            preclean_parent_step(&stash(PreservationStashPhaseV1::NormalizeParent)),
            expected
        );
        assert_eq!(
            preclean_parent_step(&reset(PreservationRefResetPhaseV1::PrepareParent)),
            expected
        );
        assert_eq!(
            preclean_parent_step(&stash(PreservationStashPhaseV1::NormalizeMarker)),
            None
        );
        assert_eq!(
            preclean_parent_step(&reset(PreservationRefResetPhaseV1::PrepareMarker)),
            None
        );
    }

    fn stash(phase: PreservationStashPhaseV1) -> PendingPreservationActionV1 {
        PendingPreservationActionV1::Stash {
            owner: PreservationOwnerV1::PublicationRoot,
            phase,
            stash_id: None,
            stash_object_id: None,
            message: String::new(),
            head_commit: String::new(),
            preimage_sha256: String::new(),
            root_publication_handoff: None,
        }
    }

    fn reset(phase: PreservationRefResetPhaseV1) -> PendingPreservationActionV1 {
        PendingPreservationActionV1::ResetAttachedRef {
            owner: PreservationOwnerV1::PublicationRoot,
            branch: String::new(),
            expected_commit: String::new(),
            restore_commit: String::new(),
            phase,
            root_publication_handoff: None,
        }
    }
}
