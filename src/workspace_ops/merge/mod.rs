mod abort;
mod acceptance;
mod continue_op;
mod finalize;
mod finalize_dispatch;
mod finalize_support;
mod gc;
mod integration;
pub(crate) mod marker;
mod model;
mod participant_semantics;
mod plan;
mod preserve;
mod publication;
mod record_wire;
mod recovery;
mod response;
pub(crate) mod root;
mod runtime;
mod start;
mod status;
mod store;
#[cfg(test)]
mod v1_lifecycle;
mod validate;

#[cfg(test)]
#[path = "tests/acceptance_v0/mod.rs"]
mod acceptance_v0;
#[cfg(test)]
#[path = "tests/transition_matrix_v0.rs"]
mod transition_matrix_v0;

#[cfg(test)]
pub(crate) use abort::{EvidenceRollbackMutation, fail_next_evidence_rollback_after};
#[cfg(test)]
pub(crate) use acceptance::{finalization_next_action_for_i2, finalization_next_action_for_v1};
#[cfg(test)]
pub(crate) use finalize::{CandidatePublicationMutation, fail_next_candidate_publication_after};
#[cfg(test)]
use preserve::classify_index_aligned_root_publication_for_i2;
#[cfg(test)]
pub(crate) use record_wire::{
    OpenV0Adaptation, PreparedOpenV0Upgrade, VerifiedV0Descriptor, adapt_open_v0_for_r3_tests,
    decode_v0_for_r3_tests, decode_v1_for_r3_tests, prepare_upgrade, verified_v0_descriptor,
};
#[cfg(test)]
pub(crate) use store::{AtomicUpgradeFault, AtomicUpgradeOutcome, upgrade_open_v0_for_r3_tests};

#[cfg(test)]
pub(crate) use model::v1::RecordVersion;
#[cfg(test)]
pub(crate) use model::v1::test_record as test_v1_record;
pub(crate) use model::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use record_wire::{
    CanonicalMergeLocations, MAX_CHECKED_OWNER_RECORD_BYTES, acquire_canonical_merge_locations,
    archived_fixture_for_test, observe_checked_archive_source_v0_leaves_for_test,
    observe_checked_archive_source_v1, observe_checked_owner_v0, observe_checked_owner_v1,
    observe_checked_owner_v1_from_canonical,
};
#[allow(unused_imports)]
pub(crate) use record_wire::{
    CheckedArchiveSourceObservation, CheckedOwnerObservationError, CheckedOwnerRecordObservation,
    CheckedOwnerRecordVersion, observe_checked_archive_source_v0,
    observe_checked_owner_v0_from_canonical,
};
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
