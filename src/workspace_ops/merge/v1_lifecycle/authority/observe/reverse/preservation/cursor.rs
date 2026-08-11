use super::*;
use crate::git::{GitBackend, GitPreservationDirtySummary};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
};
use crate::workspace_ops::merge::preserve::{
    V1BundleObservation, v1_bundle_cursor_is_exact, v1_bundle_observation, v1_owner_evidence,
    v1_preservation_image, v1_preservation_owners,
};
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    ReverseEntryPredecessor, preview_reverse_entry,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_cursor<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let plans = v1_preservation_owners(backend, current.location().root(), current.record())?;
    if let Some(action) = current.record().pending_preservation.as_ref() {
        let plan = plan_for_action(&plans, action)?;
        let position = action_position(action);
        verify_pending_prefix(backend, current, &plans, plan, action)?;
        let prefix = issue_prefix(current, plan, position)?;
        return phase::observe_pending(backend, current, &plans, plan, action, prefix);
    }
    verify_bundle_prefix(backend, current, &plans)?;

    for (index, plan) in plans.iter().enumerate() {
        if !backup_complete(current.record(), plan)? {
            reject_later_durable_owner(current.record(), &plans, index, plan)?;
            return backup_intent(current, plan);
        }
        if !stash_complete(backend, current, &plans, plan)? {
            reject_later_durable_owner(current.record(), &plans, index, plan)?;
            return stash_intent(backend, current, plan);
        }
    }
    for plan in &plans {
        if !reset_complete(backend, current, plan)? {
            return reset_intent(current, plan);
        }
    }
    exhausted(backend, context, current)
}

fn reject_later_durable_owner(
    record: &MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
    index: usize,
    current: &V1PreservationOwnerPlan,
) -> ModelResult<()> {
    for later in &plans[index + 1..] {
        if v1_owner_evidence(record, &later.owner)?.is_some() {
            return Err(owner_error(
                current,
                "preservation owner acquired new work ahead of a later durable owner",
            ));
        }
    }
    Ok(())
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn pending_recovery_is_exact<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<bool> {
    let action = current
        .record()
        .pending_preservation
        .as_ref()
        .ok_or_else(|| {
            preservation_error("preserving recovery has no retained preservation journal")
        })?;
    let plans = v1_preservation_owners(backend, current.location().root(), current.record())?;
    let plan = plan_for_action(&plans, action)?;
    verify_pending_prefix(backend, current, &plans, plan, action)?;
    let prefix = issue_prefix(current, plan, action_position(action))?;
    Ok(!matches!(
        phase::observe_pending(backend, current, &plans, plan, action, prefix)?,
        ExactObservationFact::PreservationAmbiguous(..) | ExactObservationFact::Ambiguous(..)
    ))
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn execution_prefix_is_exact<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    action: &PendingPreservationActionV1,
) -> ModelResult<()> {
    let plans = v1_preservation_owners(backend, current.location().root(), current.record())?;
    let plan = plan_for_action(&plans, action)?;
    verify_pending_prefix(backend, current, &plans, plan, action)
}

fn verify_pending_prefix<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plans: &[V1PreservationOwnerPlan],
    current_plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
) -> ModelResult<()> {
    verify_pending_bundle_prefix(backend, current, plans, current_plan, action)?;
    match action {
        PendingPreservationActionV1::BackupRef { .. } => {
            for plan in plans
                .iter()
                .take_while(|plan| plan.owner != current_plan.owner)
            {
                require_artifact_complete(backend, current, plans, plan)?;
            }
            if v1_owner_evidence(current.record(), &current_plan.owner)?
                .is_some_and(|row| row.backup_ref.is_some())
            {
                return Err(owner_error(
                    current_plan,
                    "pending backup follows durable backup evidence",
                ));
            }
        }
        PendingPreservationActionV1::Stash { .. } => {
            for plan in plans
                .iter()
                .take_while(|plan| plan.owner != current_plan.owner)
            {
                require_artifact_complete(backend, current, plans, plan)?;
            }
            if !backup_complete(current.record(), current_plan)? {
                return Err(owner_error(
                    current_plan,
                    "pending stash precedes its backup-ref position",
                ));
            }
        }
        PendingPreservationActionV1::ResetAttachedRef { .. } => {
            for plan in plans {
                require_artifact_complete(backend, current, plans, plan)?;
            }
            for plan in plans
                .iter()
                .take_while(|plan| plan.owner != current_plan.owner)
            {
                if !reset_complete(backend, current, plan)? {
                    return Err(owner_error(
                        current_plan,
                        "pending reset skips an earlier reset owner",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn verify_bundle_prefix<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plans: &[V1PreservationOwnerPlan],
) -> ModelResult<()> {
    let owner = plans.last();
    let exact =
        v1_bundle_cursor_is_exact(backend, current.location().root(), current.record(), plans)
            .map_err(|error| attach_plan(error, owner))?;
    if !exact {
        return Err(owner.map_or_else(
            || preservation_error("preservation bundle exists for an empty owner set"),
            |plan| {
                owner_error(
                    plan,
                    "preservation bundle does not match the exact durable cursor prefix",
                )
            },
        ));
    }
    Ok(())
}

fn verify_pending_bundle_prefix<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plans: &[V1PreservationOwnerPlan],
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
) -> ModelResult<()> {
    let before_bundle_write = matches!(
        action,
        PendingPreservationActionV1::Stash {
            phase: S::RestoreIndex
                | S::RestoreLock
                | S::RestoreParent
                | S::RestoreMarker
                | S::WriteBundle,
            ..
        }
    );
    if !before_bundle_write {
        return verify_bundle_prefix(backend, current, plans);
    }
    let observed = v1_bundle_observation(
        backend,
        current.location().root(),
        current.record(),
        plans,
        &plan.owner,
    )
    .map_err(|error| attach_plan(error, Some(plan)))?;
    if observed == V1BundleObservation::Ambiguous {
        return Err(owner_error(
            plan,
            "preservation bundle is neither the exact prior nor completed owner prefix",
        ));
    }
    Ok(())
}

fn attach_plan(mut error: ModelError, plan: Option<&V1PreservationOwnerPlan>) -> ModelError {
    if error.member_id.is_none()
        && let Some(plan) = plan
    {
        error.member_id = Some(plan.target_id.clone());
        error.member_path = Some(plan.relative_path.clone());
    }
    error
}

fn require_artifact_complete<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plans: &[V1PreservationOwnerPlan],
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<()> {
    if !backup_complete(current.record(), plan)? || !stash_complete(backend, current, plans, plan)?
    {
        return Err(owner_error(
            plan,
            "preservation cursor skips an unfinished earlier artifact",
        ));
    }
    Ok(())
}

fn backup_complete(
    record: &MergeOperationRecordV1,
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<bool> {
    Ok(
        v1_owner_evidence(record, &plan.owner)?.is_some_and(|row| row.backup_ref.is_some())
            || plan.protected_commit == plan.anchor,
    )
}

pub(super) fn stash_complete<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    _plans: &[V1PreservationOwnerPlan],
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<bool> {
    let evidence = v1_owner_evidence(current.record(), &plan.owner)?;
    if evidence.is_some_and(|row| row.stash_id.is_some()) {
        // The caller has already proved either the exact action-free durable
        // bundle cursor or the pending owner's exact before/after bundle row.
        // Reclassifying an earlier owner against the complete durable set here
        // would reject the legitimate prior prefix while a later WriteBundle
        // action is pending.
        let image = v1_preservation_image(backend, current.record(), plan, &plan.live_commit)?;
        return Ok(image.dirty == GitPreservationDirtySummary::default());
    }
    Ok(
        v1_preservation_image(backend, current.record(), plan, &plan.live_commit)?.dirty
            == GitPreservationDirtySummary::default(),
    )
}

pub(super) fn reset_complete<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<bool> {
    let Some(target) = v1_owner_evidence(current.record(), &plan.owner)?
        .and_then(|row| row.backup_commit.as_deref())
    else {
        return Ok(plan.live_commit == plan.anchor);
    };
    if target == plan.anchor {
        return Ok(plan.live_commit == plan.anchor);
    }
    if plan.live_commit != plan.anchor {
        return Ok(false);
    }
    Ok(
        v1_preservation_image(backend, current.record(), plan, &plan.anchor)?.dirty
            == GitPreservationDirtySummary::default(),
    )
}

fn backup_intent(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<ExactObservationFact> {
    let position = PreservationCursorPosition::BackupRef;
    let prefix = issue_prefix(current, plan, position)?;
    let payload = PreservationPayload {
        owner: plan.owner.clone(),
        observed_position: position,
        pending: Some(PendingPreservationActionV1::BackupRef {
            owner: plan.owner.clone(),
            name: plan.backup_ref.clone(),
            target_commit: plan.protected_commit.clone(),
        }),
        evidence: None,
        publication_prefix: None,
    };
    let proof = PreparedBackupRefIntent {
        bound: BoundValue::new(
            current,
            owner_binding(&plan.owner),
            "begin_backup_ref",
            "cursor_checked",
            payload,
        )?,
        prefix,
    };
    Ok(completed(CompletedObservation::Preservation(
        PreservationObservation::BackupIntent(Box::new(proof)),
    )))
}

fn stash_intent<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<ExactObservationFact> {
    let image = v1_preservation_image(backend, current.record(), plan, &plan.protected_commit)?;
    if image.dirty == GitPreservationDirtySummary::default() {
        return Err(owner_error(
            plan,
            "clean owner cannot begin a preservation stash",
        ));
    }
    let phase = if plan.root_handoff.is_some() {
        S::NormalizeParent
    } else {
        S::CreateStash
    };
    let position = PreservationCursorPosition::Stash(phase);
    let prefix = issue_prefix(current, plan, position)?;
    let stash_id = format!("stash_{}", current.record().merge_id);
    let payload = PreservationPayload {
        owner: plan.owner.clone(),
        observed_position: position,
        pending: Some(PendingPreservationActionV1::Stash {
            owner: plan.owner.clone(),
            phase,
            stash_id: None,
            stash_object_id: None,
            message: format!("gwz:{stash_id}: merge preservation"),
            head_commit: plan.protected_commit.clone(),
            preimage_sha256: image.preimage_sha256,
            root_publication_handoff: plan.root_handoff,
        }),
        evidence: None,
        publication_prefix: None,
    };
    let proof = PreparedStashIntent {
        bound: BoundValue::new(
            current,
            owner_binding(&plan.owner),
            "begin_stash",
            "cursor_checked",
            payload,
        )?,
        prefix,
    };
    Ok(completed(CompletedObservation::Preservation(
        PreservationObservation::StashIntent(Box::new(proof)),
    )))
}

fn reset_intent(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
) -> ModelResult<ExactObservationFact> {
    let expected = v1_owner_evidence(current.record(), &plan.owner)?
        .and_then(|row| row.backup_commit.clone())
        .ok_or_else(|| owner_error(plan, "reset owner has no durable backup target"))?;
    let phase = if plan.root_handoff.is_some() {
        R::PrepareParent
    } else {
        R::ResetRef
    };
    let position = PreservationCursorPosition::ResetAttachedRef(phase);
    let prefix = issue_prefix(current, plan, position)?;
    let payload = PreservationPayload {
        owner: plan.owner.clone(),
        observed_position: position,
        pending: Some(PendingPreservationActionV1::ResetAttachedRef {
            owner: plan.owner.clone(),
            branch: plan.branch.clone(),
            expected_commit: expected,
            restore_commit: plan.anchor.clone(),
            phase,
            root_publication_handoff: plan.root_handoff,
        }),
        evidence: None,
        publication_prefix: None,
    };
    let proof = PreparedRefResetIntent {
        bound: BoundValue::new(
            current,
            owner_binding(&plan.owner),
            "begin_reset_attached_ref",
            "cursor_checked",
            payload,
        )?,
        prefix,
    };
    Ok(completed(CompletedObservation::Preservation(
        PreservationObservation::ResetIntent(Box::new(proof)),
    )))
}

fn exhausted<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let proof = VerifiedPreservationExhausted::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "preservation_exhausted",
        "verified",
        (),
    )?;
    let preview = preview_reverse_entry(
        current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )?;
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(backend, context, current, &preview)?
    else {
        return Err(preservation_error(
            "preservation exhaustion cannot defer publication evidence",
        ));
    };
    let preflight = super::super::rollback::preflight_entry_with_handoff(
        backend,
        current,
        &preview,
        handoff.value(),
    )?;
    let entry = prepare_exhausted_rollback_entry(current, &preview, handoff, preflight, proof)?;
    Ok(completed(CompletedObservation::Preservation(
        PreservationObservation::Exhausted(Box::new(entry)),
    )))
}

pub(super) fn issue_prefix(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    position: PreservationCursorPosition,
) -> ModelResult<VerifiedPreservationCursorPrefix> {
    VerifiedPreservationCursorPrefix::issue(
        &AuthorityIssuer::for_observer(current),
        owner_binding(&plan.owner),
        "preservation_cursor",
        "prefix_verified",
        PreservationCursorPrefix {
            owner: plan.owner.clone(),
            position,
        },
    )
}
