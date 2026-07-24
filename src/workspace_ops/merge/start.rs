use super::{MergeStore, OperationState, ParticipantState, plan::plan_merge};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;
use std::path::Path;

mod execution;
mod prepared;
mod record;
mod response;

use execution::execute_durable;
use record::{create_record, freeze_merge_messages};
use response::{decorate_start_response, handle_dry_run, start_response};

#[cfg(test)]
use super::{
    MergeOperationRecord, MergeParticipantPlan, PendingMergeAction, PendingMergeActionKind,
    PendingMergeExpectedResult,
};
#[cfg(test)]
use crate::artifact;
#[cfg(test)]
use crate::git::{
    GitHeadState, GitIntegrateResult, GitMergeAnalysis, GitMergeAnalysisKind, GitPreparedMerge,
    GitPreparedSignature, GitStatus,
};
#[cfg(test)]
use crate::operation::ActionKind;
#[cfg(test)]
use crate::{AggregateStatus, MergeOperationState as OpState, MergeParticipantState as PState};
#[cfg(test)]
use execution::{ExecutionBackend, Inspection};
#[cfg(test)]
use prepared::{PreparedAction, Row, execute_plan};
#[cfg(test)]
use record::{pending_commit_spec, set_pending_action};
#[cfg(test)]
use response::{merge_response, summary};
#[cfg(test)]
use std::collections::BTreeMap;

pub(super) fn handle_start_durable<B, S, C, I>(
    dependencies: super::MergeDependencies<'_, B, S, C, I>,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    let super::MergeDependencies {
        backend,
        store,
        clock,
        ids,
        events: _,
    } = dependencies;
    if request.meta.dry_run.unwrap_or(false) {
        return handle_dry_run(backend, root, request, context);
    }

    if let Some(open) = store.discover_open(root)? {
        return Err(open_operation_error(&open.merge_id));
    }
    let mut plan = plan_merge(backend, root, request)?;
    let merge_id = ids.next_id("merge").to_string();
    freeze_merge_messages(&mut plan.participants, &plan.source_ref, &merge_id, context);
    let mut record = create_record(root, &plan, &merge_id, clock, context)?;
    super::persist_merge_record(store, root, &record, emitter)?;
    emitter.operation_state_changed(record.state.into());
    execute_durable(
        backend,
        store,
        root,
        &plan.participants,
        context.attribution.as_ref(),
        &mut record,
        emitter,
    )?;

    let next = if record
        .participants
        .values()
        .any(|participant| participant.state == ParticipantState::Failed)
    {
        OperationState::Halted
    } else if record
        .participants
        .values()
        .any(|participant| participant.state == ParticipantState::Conflicted)
    {
        OperationState::AwaitingResolution
    } else {
        OperationState::Finalizing
    };
    if next == OperationState::Finalizing {
        super::enter_finalizing(store, root, &mut record, emitter)?;
        let completed =
            super::finalize::finalize(backend, store, root, &mut record, context, emitter)?;
        if !completed {
            let response = super::status::snapshot_status(backend, root, record.clone())?
                .to_response(context)?;
            return decorate_start_response(response, &plan.participants);
        }
    } else {
        super::persist_operation_transition(store, root, &mut record, next, emitter)?;
    }
    start_response(&record, &plan.participants, context)
}

fn open_operation_error(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::OpenOperation,
        format!("merge '{merge_id}' is open; use merge status, merge continue, or merge abort"),
    )
}

#[cfg(test)]
#[path = "start/tests/execution.rs"]
mod execution_tests;
#[cfg(test)]
use execution_tests::*;

#[cfg(test)]
#[path = "start/tests/prepared_recovery.rs"]
mod prepared_recovery_tests;
#[cfg(test)]
use prepared_recovery_tests::*;

#[cfg(test)]
#[path = "start/tests/event_sink.rs"]
mod event_sink_tests;

#[cfg(test)]
#[path = "start/tests/durable_recovery.rs"]
mod durable_recovery_tests;
#[cfg(test)]
use durable_recovery_tests::*;

#[cfg(test)]
#[path = "start/tests/resolution_race.rs"]
mod resolution_race_tests;
#[cfg(test)]
use resolution_race_tests::*;

#[cfg(test)]
#[path = "start/tests/resolution_validation.rs"]
mod resolution_validation_tests;
#[cfg(test)]
use resolution_validation_tests::*;
