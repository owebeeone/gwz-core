use super::{MergeStore, OperationState, plan::plan_merge};
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

use super::participant_semantics::continue_eligibility::post_start_state;
use execution::execute_durable;
use record::{create_record, freeze_merge_messages};
use response::{decorate_start_response, handle_dry_run, start_response};

#[cfg(test)]
use super::{
    MergeOperationRecord, MergeParticipantPlan, ParticipantState, PendingMergeAction,
    PendingMergeActionKind, PendingMergeExpectedResult,
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
    v1: &dyn super::runtime::V1Router,
    start_guard: Option<super::WorkspaceMutationGuard>,
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

    // A1: the open-merge guard is version-agnostic. `discover_open` reads
    // through the v0 store, whose decoder installs v0 only, so an open v1
    // record must be seen by its envelope or a second start would clobber it.
    if let Some(open) = super::classify_open_record(root)? {
        return Err(open_operation_error(&open.merge_id));
    }
    if let Some(open) = store.discover_open(root)? {
        return Err(open_operation_error(&open.merge_id));
    }
    let mut plan = plan_merge(backend, root, request)?;
    let merge_id = ids.next_id("merge").to_string();
    freeze_merge_messages(
        &mut plan.participants,
        request.message.as_deref(),
        &plan.source_ref,
        &merge_id,
        context,
    )?;
    let mut record = create_record(root, &plan, &merge_id, clock, context)?;
    // A1 (Safety review §2.2 R4 / §2.4): the contract-§2 writer floor chose
    // this record's version at creation, and the record goes to that
    // version's owner. A v1 record is created and driven by the v1 lifecycle;
    // it never enters the v0 persistence seam.
    if super::select_record_version(super::RequestedSemantics::from_mode(plan.mode))?
        == super::RecordVersion::V1
    {
        // The v1 lifecycle owns the workspace mutator lock for its whole
        // operation — `service::run` takes its own `V1MutationLease`, and that
        // lock is a workspace-wide OS advisory exclusive lock, not a
        // re-entrant one. Release the start gate's guard here, after it has
        // already enforced the open-merge policy, exactly as the v0 recovery
        // commands run: they take no start guard and let their own handler
        // hold the lock. `create_open` re-checks that no record exists, so the
        // handoff cannot publish a second record.
        drop(start_guard);
        // DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1,
        // 2026-09-03): `--filesystem-strict` is a START-only flag, and this is
        // the only place a start's request meets the v1 owner that decides.
        return v1.start(
            root,
            record,
            request.filesystem_strict.unwrap_or(false),
            context,
            emitter,
        );
    }
    let _start_guard = start_guard;
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

    let next = post_start_state(
        record
            .participants
            .values()
            .map(|participant| participant.state),
    );
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

#[cfg(test)]
#[path = "start/tests/root_execution.rs"]
mod root_execution_tests;
