use super::{PhysicalExecutor, V1ResponseDisposition};
use crate::model::ModelResult;

use super::super::authority::{
    BoundExecutionAttempt, BoundPhysicalAction, PhysicalActionKind, V1Invocation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::events::{LifecycleEvents, action_member};
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
    events: &mut LifecycleEvents<'_>,
) -> ModelResult<ExecutionOutcome> {
    let action_kind = action.authorize(current)?.clone();
    invocation.before_execute(&action_kind)?;
    if action_kind == PhysicalActionKind::Archive {
        store.archive(lease, current)?;
        events.archived(&current.record().merge_id);
        return Ok(ExecutionOutcome::Respond(
            V1ResponseDisposition::ArchiveReady,
        ));
    }
    // M5d charter §4: the reverse arms announce their participant here, where
    // `abort/participants.rs:37` announces it — immediately before the Git
    // action, not before the journal write that authorized it. A forward
    // participant already announced itself at its preparation observation, and
    // the announcement is idempotent per invocation.
    if let Some(member_id) = action_member(&action_kind) {
        events.before_action(current.record(), member_id);
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
    events: &mut LifecycleEvents<'_>,
) -> ModelResult<()> {
    if disposition == V1ResponseDisposition::ArchiveReady {
        store.archive(lease, current)?;
        events.archived(&current.record().merge_id);
    }
    Ok(())
}
