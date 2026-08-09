use super::{PhysicalExecutor, V1ResponseDisposition};
use crate::model::ModelResult;

use super::super::authority::{
    BoundExecutionAttempt, BoundPhysicalAction, PhysicalActionKind, V1Invocation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::store::CheckedV1Store;

pub(super) enum ExecutionOutcome {
    Attempt(Box<BoundExecutionAttempt>),
    Respond(V1ResponseDisposition),
}

pub(super) fn execute_owned<R: PhysicalExecutor>(
    lease: &V1MutationLease,
    store: &CheckedV1Store,
    current: &StoredV1Record,
    action: Box<BoundPhysicalAction>,
    invocation: &mut V1Invocation,
    runtime: &mut R,
) -> ModelResult<ExecutionOutcome> {
    let action_kind = action.authorize(current)?.clone();
    invocation.before_execute(&action_kind)?;
    if action_kind == PhysicalActionKind::Archive {
        store.archive(lease, current)?;
        return Ok(ExecutionOutcome::Respond(
            V1ResponseDisposition::ArchiveReady,
        ));
    }
    let diagnostic = runtime.execute(lease, current, &action_kind);
    let attempt = action.record_attempt(current, diagnostic.clone())?;
    invocation.record_execution(action_kind, diagnostic);
    Ok(ExecutionOutcome::Attempt(Box::new(attempt)))
}

pub(super) fn complete_response(
    lease: &V1MutationLease,
    store: &CheckedV1Store,
    current: &StoredV1Record,
    disposition: V1ResponseDisposition,
) -> ModelResult<()> {
    if disposition == V1ResponseDisposition::ArchiveReady {
        store.archive(lease, current)?;
    }
    Ok(())
}
