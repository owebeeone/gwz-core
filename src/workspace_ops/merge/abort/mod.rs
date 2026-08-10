mod evidence;
mod participants;
mod preflight;
mod reconciliation;
mod runtime;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use evidence::{EvidenceRollbackMutation, fail_next_evidence_rollback_after};

#[cfg(test)]
pub(in crate::workspace_ops::merge) use evidence::{
    V1EvidenceRollbackObservation, execute_v1_evidence_rollback, observe_v1_evidence_rollback,
    preflight_v1_evidence,
};
#[cfg(test)]
pub(in crate::workspace_ops::merge) use participants::{
    V1ParticipantRollbackObservation, execute_v1_participant_rollback,
    observe_v1_participant_rollback, verify_v1_no_mutation_participant,
};
#[cfg(test)]
pub(in crate::workspace_ops::merge) use preflight::preflight_v1_rollback;

use self::{
    evidence::{preflight_evidence, rollback_evidence, verify_evidence_baseline},
    participants::rollback_participants,
    preflight::{preflight, restore_baseline, verify_baseline},
    reconciliation::apply_pending_reconciliations,
    runtime::{AbortRuntime, GitAbortRuntime},
};
use super::{MergeStore, OperationState};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext, WorkspaceMutatorLock};
use std::path::Path;

pub(crate) fn handle_abort<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    if request.preserve == Some(true) {
        return super::preserve::preserve_then_abort(
            backend, store, root, request, context, emitter,
        );
    }
    if let Some(record) = store.discover_open(root)? {
        super::validate::validate_open_merge_id(request.merge_id.as_deref(), &record.merge_id)?;
        if record.state == OperationState::Preserving {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "merge preservation is incomplete; retry `gwz merge --abort --preserve` so every preservation artifact is reconciled and verified before rollback",
            ));
        }
    }
    abort_locked(
        backend,
        store,
        root,
        request.merge_id.as_deref(),
        context,
        emitter,
    )
}

pub(super) fn abort_locked<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    requested_id: Option<&str>,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    abort_with_runtime(
        &GitAbortRuntime(backend),
        store,
        root,
        requested_id,
        context,
        emitter,
    )
}

fn abort_with_runtime<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    requested_id: Option<&str>,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let Some(mut record) = store.discover_open(root)? else {
        return closed_or_missing(store, root, requested_id, context, emitter);
    };
    super::validate::validate_open_merge_id(requested_id, &record.merge_id)?;
    if record.state == OperationState::Completed {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("merge in state {:?} cannot be aborted", record.state),
        ));
    }

    // A terminal record in the open directory is archive-pending. Its baseline
    // and participant outcomes were verified before Aborted was written, so a
    // retry must finish closing it without allowing later unrelated repository
    // work to strand the durable terminal record.
    if record.state == OperationState::Aborted {
        super::archive_merge_record(store, root, &record.merge_id, emitter)?;
        return record.to_response(context);
    }

    let evidence = preflight_evidence(runtime, root, &record)?;
    record.operation_drift.retain(|drift| {
        !matches!(
            drift.kind,
            super::OperationDriftKind::RootCandidateMetadataInvalid
                | super::OperationDriftKind::RootCandidateStateChanged
        )
    });
    let mut snapshot = runtime.snapshot(root, record)?;
    if evidence
        .as_ref()
        .is_some_and(|evidence| evidence.root_participant_evidence_present)
    {
        super::root::normalize_evidence_observation(&mut snapshot)?;
    }
    if snapshot.record.state == OperationState::RollingBack && evidence.is_some() {
        snapshot.operation_drift.retain(|drift| {
            !matches!(
                drift.kind,
                super::OperationDriftKind::RootCandidateMetadataInvalid
                    | super::OperationDriftKind::RootCandidateStateChanged
            )
        });
    }
    let preflight = preflight(&snapshot)?;
    record = snapshot.record;
    for target_id in preflight.pending.keys() {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        emitter.member_started(target_id, &participant.path);
    }
    if apply_pending_reconciliations(&mut record, &preflight.pending)? {
        super::persist_merge_record(store, root, &record, emitter)?;
        for target_id in preflight.pending.keys() {
            super::emit_merge_member_finished(emitter, &record, target_id)?;
        }
    }

    if record.state == OperationState::Executing {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::Halted,
            emitter,
        )?;
    }
    if record.state != OperationState::RollingBack {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::RollingBack,
            emitter,
        )?;
    }
    if let Some(evidence) = evidence.as_ref() {
        rollback_evidence(runtime, store, root, &mut record, evidence, emitter)?;
        verify_evidence_baseline(runtime, root, evidence)?;
    }
    rollback_participants(runtime, store, root, &mut record, &preflight, emitter)?;
    restore_baseline(root, &record)?;
    verify_baseline(root, &record)?;
    super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Aborted,
        emitter,
    )?;
    super::archive_merge_record(store, root, &record.merge_id, emitter)?;
    record.to_response(context)
}

fn closed_or_missing<S: MergeStore>(
    store: &S,
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let Some(merge_id) = merge_id else {
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            "no coordinated merge is open",
        ));
    };
    let record = store.load(root, merge_id)?;
    if record.state != OperationState::Aborted {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "merge '{merge_id}' in state {:?} cannot be aborted",
                record.state
            ),
        ));
    }
    // This is idempotent when the prior archive rename succeeded but a later
    // sync, verification, retention, or response step failed.
    super::archive_merge_record(store, root, merge_id, emitter)?;
    record.to_response(context)
}
