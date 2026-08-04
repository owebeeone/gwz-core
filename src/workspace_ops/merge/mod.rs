mod abort;
mod continue_op;
mod finalize;
mod gc;
mod integration;
pub(crate) mod marker;
mod model;
mod participant_semantics;
mod plan;
mod preserve;
mod publication;
mod recovery;
mod response;
pub(crate) mod root;
mod runtime;
mod start;
mod status;
mod store;
mod validate;

#[cfg(test)]
#[path = "tests/transition_matrix_v0.rs"]
mod transition_matrix_v0;

#[cfg(test)]
pub(crate) use abort::{EvidenceRollbackMutation, fail_next_evidence_rollback_after};
#[cfg(test)]
pub(crate) use finalize::validate_candidate_for_i2_fixture;
#[cfg(test)]
pub(crate) use finalize::{CandidatePublicationMutation, fail_next_candidate_publication_after};
#[cfg(test)]
use preserve::classify_index_aligned_root_publication_for_i2;
#[cfg(test)]
pub(crate) use publication::normalized_i2_root_observation;

pub(crate) use model::*;
pub(crate) use recovery::*;
#[cfg(test)]
pub(crate) use runtime::handle_merge_with_dependencies;
pub(crate) use runtime::{
    MergeDependencies, enforce_open_merge_stage_targets, guarded_workspace_root,
};
pub use runtime::{
    WorkspaceMutationGuard, acquire_workspace_mutation_guard, enforce_workspace_open_merge_gate,
    handle_merge, handle_merge_with_events,
};
pub(crate) use store::{
    FileMergeStore, MergeStore, archive_merge_record, enter_finalizing, persist_merge_record,
    persist_operation_transition,
};
pub(crate) use validate::validate_merge_request;

use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) fn emit_merge_member_finished(
    emitter: &crate::operation::EventEmitter<'_>,
    record: &MergeOperationRecord,
    target_id: &str,
) -> ModelResult<()> {
    let participant = record.participants.get(target_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{target_id}'"),
        )
    })?;
    emitter.merge_member_finished(participant.to_protocol(target_id, &record.source_ref));
    Ok(())
}
