use std::path::Path;

use super::MergeStore;
use crate::model::ModelResult;

use super::super::{MergeOperationRecord, OperationState};

pub(crate) fn persist_operation_transition<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    next: OperationState,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    record.state = record.state.transition(next)?;
    persist_merge_record(store, root, record, emitter)?;
    emitter.operation_state_changed(record.state.into());
    Ok(())
}

pub(crate) fn persist_merge_record<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &MergeOperationRecord,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    store.write_open(root, record)?;
    emitter.artifact_written(open_merge_artifact_path(&record.merge_id));
    Ok(())
}

pub(crate) fn archive_merge_record<S: MergeStore>(
    store: &S,
    root: &Path,
    merge_id: &str,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    store.archive(root, merge_id)?;
    emitter.artifact_written(done_merge_artifact_path(merge_id));
    Ok(())
}

fn open_merge_artifact_path(merge_id: &str) -> String {
    format!(".gwz/merge/{merge_id}.yaml")
}

fn done_merge_artifact_path(merge_id: &str) -> String {
    format!(".gwz/merge/done/{merge_id}.yaml")
}

/// M2a's stable handoff into publication. M2b replaces the implementation
/// behind this seam; callers do not publish or advance the accepted lock.
pub(crate) fn enter_finalizing<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    persist_operation_transition(store, root, record, OperationState::Finalizing, emitter)
}
