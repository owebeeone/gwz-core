mod evidence;
mod steps;

use super::*;
use crate::git::{
    GitDirectRefObservation, GitPreservationDirtySummary, GitRootPreservationGuard,
    GitRootPreservationStepObservation, MergeAuthorityBackend,
};
use crate::workspace_ops::merge::PreservationEvidence;
use crate::workspace_ops::merge::model::v1::{
    PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S, RecoveryOriginStateV1,
};
use crate::workspace_ops::merge::preserve::{
    V1BundleObservation, v1_bundle_observation, v1_root_preservation_spec,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) use steps::{
    reset_step, stash_guard, stash_step,
};

pub(super) fn observe_pending<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plans: &[V1PreservationOwnerPlan],
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    match action {
        PendingPreservationActionV1::BackupRef {
            name,
            target_commit,
            ..
        } => match backend.observe_direct_ref(&plan.path, name)? {
            GitDirectRefObservation::Absent
                if exact_attached_head(backend, plan, target_commit)? =>
            {
                not_started(action, prefix)
            }
            GitDirectRefObservation::Absent => Err(owner_error(
                plan,
                "persisted backup-ref action no longer has its recorded attached HEAD",
            )),
            GitDirectRefObservation::Direct { target } if target == *target_commit => {
                backup_done(current, plan, action, prefix)
            }
            _ => ambiguous(current, prefix),
        },
        PendingPreservationActionV1::Stash {
            phase: S::WriteBundle,
            ..
        } => match v1_bundle_observation(
            current.location().root(),
            current.record(),
            plans,
            &plan.owner,
        )? {
            V1BundleObservation::Before => not_started(action, prefix),
            V1BundleObservation::After => stash_done(backend, current, plan, action, prefix),
            V1BundleObservation::Ambiguous => ambiguous(current, prefix),
        },
        PendingPreservationActionV1::Stash {
            phase: S::Complete, ..
        } => {
            if super::cursor::stash_complete(backend, current, plans, plan)? {
                stash_done(backend, current, plan, action, prefix)
            } else {
                ambiguous(current, prefix)
            }
        }
        PendingPreservationActionV1::Stash { .. } => {
            observe_stash_phase(backend, current, plan, action, prefix)
        }
        PendingPreservationActionV1::ResetAttachedRef {
            phase: R::Complete, ..
        } => {
            if super::cursor::reset_complete(backend, current, plan)? {
                reset_done(current, plan, action, prefix)
            } else {
                ambiguous(current, prefix)
            }
        }
        PendingPreservationActionV1::ResetAttachedRef { .. } => {
            observe_reset_phase(backend, current, plan, action, prefix)
        }
    }
}

fn observe_stash_phase<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    let PendingPreservationActionV1::Stash {
        phase,
        head_commit,
        preimage_sha256,
        ..
    } = action
    else {
        return Err(preservation_error(
            "stash phase observer received another action",
        ));
    };
    let observation = if let Some(spec) =
        v1_root_preservation_spec(backend, current.record(), plan, head_commit)?
    {
        backend.observe_root_preservation_step(
            &plan.path,
            &spec,
            &stash_step(*phase, &current.record().merge_id)?,
            &stash_guard(*phase, preimage_sha256),
        )?
    } else {
        observe_plain_stash(backend, current, plan, action)?
    };
    classify_phase(backend, current, plan, action, prefix, observation, true)
}

fn observe_reset_phase<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    let PendingPreservationActionV1::ResetAttachedRef {
        expected_commit,
        restore_commit,
        phase,
        ..
    } = action
    else {
        return Err(preservation_error(
            "reset phase observer received another action",
        ));
    };
    let observation = if let Some(spec) =
        v1_root_preservation_spec(backend, current.record(), plan, expected_commit)?
    {
        backend.observe_root_preservation_step(
            &plan.path,
            &spec,
            &reset_step(*phase)?,
            &GitRootPreservationGuard::OtherwiseClean,
        )?
    } else if backend.checkout_matches_commit(&plan.path, &plan.branch, restore_commit)? {
        GitRootPreservationStepObservation::After
    } else if backend.checkout_matches_commit(&plan.path, &plan.branch, expected_commit)? {
        GitRootPreservationStepObservation::Before
    } else {
        GitRootPreservationStepObservation::Ambiguous
    };
    classify_phase(backend, current, plan, action, prefix, observation, false)
}

fn observe_plain_stash<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
) -> ModelResult<GitRootPreservationStepObservation> {
    let PendingPreservationActionV1::Stash {
        phase: S::CreateStash,
        head_commit,
        message,
        preimage_sha256,
        ..
    } = action
    else {
        return Err(owner_error(
            plan,
            "non-root stash carries a root-only phase",
        ));
    };
    let stashes =
        crate::git::observe_preservation_stashes_read_only(&plan.path, &current.record().merge_id)?;
    let image = backend.preservation_image(&plan.path, true)?;
    let attached = exact_attached_head(backend, plan, head_commit)?;
    if let [stash] = stashes.as_slice()
        && attached
        && stash.message == *message
        && stash.head_commit == *head_commit
        && stash.image.preimage_sha256 == *preimage_sha256
        && image.dirty == GitPreservationDirtySummary::default()
    {
        return Ok(GitRootPreservationStepObservation::After);
    }
    if stashes.is_empty() {
        if !attached || image.preimage_sha256 != *preimage_sha256 {
            return Err(owner_error(
                plan,
                "persisted stash action no longer has its recorded HEAD and preimage",
            ));
        }
        if image.dirty != GitPreservationDirtySummary::default() {
            return Ok(GitRootPreservationStepObservation::Before);
        }
    }
    Ok(GitRootPreservationStepObservation::Ambiguous)
}

fn exact_attached_head<B: MergeAuthorityBackend>(
    backend: &B,
    plan: &V1PreservationOwnerPlan,
    commit: &str,
) -> ModelResult<bool> {
    if backend.repository_state(&plan.path)? != crate::git::GitRepositoryState::Clean {
        return Ok(false);
    }
    let head = backend.head(&plan.path)?;
    let branch = backend.read_ref(&plan.path, &format!("refs/heads/{}", plan.branch))?;
    Ok(!head.is_detached
        && head.branch.as_deref() == Some(plan.branch.as_str())
        && head.commit.as_deref() == Some(commit)
        && branch.as_deref() == Some(commit))
}

fn classify_phase<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
    observation: GitRootPreservationStepObservation,
    stash: bool,
) -> ModelResult<ExactObservationFact> {
    match observation {
        GitRootPreservationStepObservation::Before => not_started(action, prefix),
        GitRootPreservationStepObservation::After => {
            if stash {
                stash_done(backend, current, plan, action, prefix)
            } else {
                reset_done(current, plan, action, prefix)
            }
        }
        GitRootPreservationStepObservation::AfterNeedsDurability => {
            let completion_prefix =
                super::cursor::issue_prefix(current, plan, action_position(action))?;
            let completion = if stash {
                stash_completion(backend, current, plan, action, completion_prefix)?
            } else {
                reset_completion(current, plan, action, completion_prefix)?
            };
            durability_fact(observation, completion, prefix, action.clone())
        }
        GitRootPreservationStepObservation::Ambiguous => ambiguous(current, prefix),
    }
}

fn backup_done(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    let PendingPreservationActionV1::BackupRef {
        name,
        target_commit,
        ..
    } = action
    else {
        return Err(preservation_error(
            "backup completion received another action",
        ));
    };
    let payload = PreservationPayload {
        owner: plan.owner.clone(),
        observed_position: PreservationCursorPosition::BackupRef,
        pending: None,
        // `GwzM5-8DurableCursorAmendment.md` §2.2: both markers are immutable
        // once written. This whole-row successor therefore carries any marker
        // the prior row already holds rather than dropping it — the artifact
        // edges install artifact evidence, they never retire a marker.
        evidence: Some(PreservationEvidence {
            backup_ref: Some(name.clone()),
            backup_commit: Some(target_commit.clone()),
            stash_id: None,
            stash_object_id: None,
            noop_commit: prior_marker(current, plan, |row| row.noop_commit.clone())?,
            reset_commit: prior_marker(current, plan, |row| row.reset_commit.clone())?,
        }),
        publication_prefix: None,
    };
    let proof = VerifiedBackupRef {
        bound: BoundValue::new(
            current,
            owner_binding(&plan.owner),
            "finish_backup_ref",
            "completed",
            payload,
        )?,
        prefix,
    };
    Ok(completed(CompletedObservation::Preservation(
        PreservationObservation::BackupDone(Box::new(proof)),
    )))
}

fn stash_done<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    let completion = stash_completion(backend, current, plan, action, prefix)?;
    Ok(completed(CompletedObservation::Preservation(completion)))
}

fn stash_completion<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<PreservationObservation> {
    let PendingPreservationActionV1::Stash {
        phase,
        stash_id,
        stash_object_id,
        ..
    } = action
    else {
        return Err(preservation_error(
            "stash completion received another action",
        ));
    };
    let payload = if *phase == S::Complete {
        PreservationPayload {
            owner: plan.owner.clone(),
            observed_position: action_position(action),
            pending: None,
            evidence: None,
            publication_prefix: None,
        }
    } else {
        let (next, ids, evidence) = if *phase == S::CreateStash {
            evidence::stash_evidence(backend, current, plan, action)?
        } else {
            (
                steps::next_stash(*phase, plan.root_handoff.is_some())?,
                (stash_id.clone(), stash_object_id.clone()),
                None,
            )
        };
        PreservationPayload {
            owner: plan.owner.clone(),
            observed_position: action_position(action),
            pending: Some(steps::with_stash_phase(action, next, ids.0, ids.1)?),
            evidence,
            publication_prefix: None,
        }
    };
    if *phase == S::Complete {
        Ok(PreservationObservation::StashDone(Box::new(
            VerifiedStashCompletion {
                bound: BoundValue::new(
                    current,
                    owner_binding(&plan.owner),
                    "finish_stash",
                    "completed",
                    payload,
                )?,
                prefix,
            },
        )))
    } else {
        Ok(PreservationObservation::StashPhase(Box::new(
            VerifiedStashPhase {
                bound: BoundValue::new(
                    current,
                    owner_binding(&plan.owner),
                    "advance_stash",
                    "completed",
                    payload,
                )?,
                prefix,
            },
        )))
    }
}

fn reset_done(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    Ok(completed(CompletedObservation::Preservation(
        reset_completion(current, plan, action, prefix)?,
    )))
}

/// The marker a prior evidence row already holds, if any. §2.2 makes both
/// markers immutable once written, so every whole-row successor built at a
/// non-marker evidence edge must carry them forward.
fn prior_marker(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    field: impl Fn(&PreservationEvidence) -> Option<String>,
) -> ModelResult<Option<String>> {
    Ok(
        crate::workspace_ops::merge::preserve::v1_owner_evidence(current.record(), &plan.owner)?
            .and_then(field),
    )
}

fn reset_completion(
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<PreservationObservation> {
    let PendingPreservationActionV1::ResetAttachedRef { phase, .. } = action else {
        return Err(preservation_error(
            "reset completion received another action",
        ));
    };
    let pending = if *phase == R::Complete {
        None
    } else {
        let next = steps::next_reset(*phase, plan.root_handoff.is_some())?;
        Some(steps::with_reset_phase(action, next)?)
    };
    // `GwzM5-8DurableCursorAmendment.md` §3.1 edge 1: the reset completion bit
    // rides the same atomic rewrite as the reset journal's retirement — the
    // one action that today retires with no durable trace anywhere. When the
    // owner's row carries neither `noop_commit` nor a stash pair (the shape a
    // pre-amendment record presents after §4's live re-proof, where the
    // retained pending action blocked any earlier marker write), the same
    // write backfills `noop_commit`, so the result is only ever a §2.2-legal
    // `B+N+R` / `N+R` / `B+S+R` / `S+R` — never `B+R` or `R` alone.
    let evidence = if *phase == R::Complete {
        let row = crate::workspace_ops::merge::preserve::v1_owner_evidence(
            current.record(),
            &plan.owner,
        )?;
        let backfill = !row.is_some_and(|row| row.noop_commit.is_some() || row.stash_id.is_some());
        Some(super::cursor::marker_row(
            current.record(),
            plan,
            backfill,
            true,
        )?)
    } else {
        None
    };
    let payload = PreservationPayload {
        owner: plan.owner.clone(),
        observed_position: action_position(action),
        pending,
        evidence,
        publication_prefix: None,
    };
    if *phase == R::Complete {
        Ok(PreservationObservation::ResetDone(Box::new(
            VerifiedRefResetCompletion {
                bound: BoundValue::new(
                    current,
                    owner_binding(&plan.owner),
                    "finish_reset_attached_ref",
                    "completed",
                    payload,
                )?,
                prefix,
            },
        )))
    } else {
        Ok(PreservationObservation::ResetPhase(Box::new(
            VerifiedRefResetPhase {
                bound: BoundValue::new(
                    current,
                    owner_binding(&plan.owner),
                    "advance_reset_attached_ref",
                    "completed",
                    payload,
                )?,
                prefix,
            },
        )))
    }
}

fn not_started(
    action: &PendingPreservationActionV1,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    Ok(ExactObservationFact::NotStarted(
        NotStartedObservation::Preservation {
            action: action.clone(),
            prefix,
        },
    ))
}

fn ambiguous(
    current: &StoredV1Record,
    prefix: VerifiedPreservationCursorPrefix,
) -> ModelResult<ExactObservationFact> {
    let proof = BoundAmbiguityEvidence::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "enter_recovery",
        "ambiguous",
        RecoveryOriginStateV1::Preserving,
    )?;
    Ok(ExactObservationFact::PreservationAmbiguous(proof, prefix))
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn durability_fact(
    observation: GitRootPreservationStepObservation,
    completion: PreservationObservation,
    prefix: VerifiedPreservationCursorPrefix,
    action: PendingPreservationActionV1,
) -> ModelResult<ExactObservationFact> {
    if observation != GitRootPreservationStepObservation::AfterNeedsDurability {
        return Err(preservation_error(
            "preservation durability fact requires exact final-only pending structure",
        ));
    }
    Ok(ExactObservationFact::PreservationDurabilityPending {
        completion,
        prefix,
        action,
    })
}
