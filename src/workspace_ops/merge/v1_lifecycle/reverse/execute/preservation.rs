use super::super::*;
use crate::git::GitCheckedPreservationMutation;
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
};
use crate::workspace_ops::merge::preserve::{
    v1_preservation_owners, v1_root_preservation_spec, v1_write_bundle_checked,
};

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn execute<B: GitBackend>(
    backend: &B,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    action: &PhysicalActionKind,
) -> ExecutionDiagnostic {
    match execute_checked(backend, lease, current, action) {
        Ok(()) => ExecutionDiagnostic::Success,
        Err(error) => failure(error),
    }
}

fn execute_checked<B: GitBackend>(
    backend: &B,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    action: &PhysicalActionKind,
) -> ModelResult<()> {
    if !lease.covers(current.location()) || current.record().state != OperationState::Preserving {
        return Err(route_error(
            "preservation execution is outside its checked lease or durable state",
        ));
    }
    let PhysicalActionKind::Preservation(action) = action else {
        return Err(route_error(
            "preservation executor received another action kind",
        ));
    };
    if current.record().pending_preservation.as_ref() != Some(action) {
        return Err(route_error(
            "preservation executor action does not match the persisted journal",
        ));
    }
    crate::workspace_ops::merge::v1_lifecycle::authority::preservation_execution_prefix_is_exact(
        backend, current, action,
    )?;
    let plans = v1_preservation_owners(backend, current.location().root(), current.record())?;
    let owner = match action {
        PendingPreservationActionV1::BackupRef { owner, .. }
        | PendingPreservationActionV1::Stash { owner, .. }
        | PendingPreservationActionV1::ResetAttachedRef { owner, .. } => owner,
    };
    let plan = plans
        .iter()
        .find(|plan| &plan.owner == owner)
        .ok_or_else(|| route_error("preservation owner is outside the frozen owner order"))?;
    match action {
        PendingPreservationActionV1::BackupRef {
            name,
            target_commit,
            ..
        } => {
            backend.create_backup_ref(&plan.path, name, target_commit)?;
        }
        PendingPreservationActionV1::Stash {
            phase: S::WriteBundle,
            ..
        } => v1_write_bundle_checked(
            backend,
            current.location().root(),
            current.record(),
            &plans,
            owner,
        )?,
        PendingPreservationActionV1::Stash {
            phase: S::Complete, ..
        } => return Err(route_error("complete stash phase has no physical mutation")),
        PendingPreservationActionV1::Stash {
            phase,
            head_commit,
            preimage_sha256,
            ..
        } => {
            if let Some(spec) =
                v1_root_preservation_spec(backend, current.record(), plan, head_commit)?
            {
                backend.execute_root_preservation_step_checked(
                    &plan.path,
                    &spec,
                    &crate::workspace_ops::merge::v1_lifecycle::authority::preservation_stash_step(
                        *phase,
                        &current.record().merge_id,
                    )?,
                    &crate::workspace_ops::merge::v1_lifecycle::authority::preservation_stash_guard(
                        *phase,
                        preimage_sha256,
                    ),
                )?;
            } else if *phase == S::CreateStash {
                backend.stash_for_merge_preservation(
                    &plan.path,
                    &current.record().merge_id,
                    true,
                )?;
            } else {
                return Err(route_error("non-root stash carries a root-only phase"));
            }
        }
        PendingPreservationActionV1::ResetAttachedRef {
            phase: R::Complete, ..
        } => return Err(route_error("complete reset phase has no physical mutation")),
        PendingPreservationActionV1::ResetAttachedRef {
            branch,
            expected_commit,
            restore_commit,
            phase,
            ..
        } => {
            if let Some(spec) =
                v1_root_preservation_spec(backend, current.record(), plan, expected_commit)?
            {
                backend.execute_root_preservation_step_checked(
                    &plan.path,
                    &spec,
                    &crate::workspace_ops::merge::v1_lifecycle::authority::preservation_reset_step(
                        *phase,
                    )?,
                    &crate::git::GitRootPreservationGuard::OtherwiseClean,
                )?;
            } else if *phase == R::ResetRef {
                backend.set_branch_target_checked(
                    &plan.path,
                    branch,
                    expected_commit,
                    restore_commit,
                )?;
            } else {
                return Err(route_error("non-root reset carries a root-only phase"));
            }
        }
    }
    Ok(())
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn durability_diagnostic(
    result: ModelResult<GitCheckedPreservationMutation>,
) -> ExecutionDiagnostic {
    match result {
        Ok(GitCheckedPreservationMutation::Applied)
        | Ok(GitCheckedPreservationMutation::AlreadyComplete) => ExecutionDiagnostic::Success,
        Ok(_) => failure(route_error(
            "durability barrier returned a non-parent preservation mutation",
        )),
        Err(error) => failure(error),
    }
}
