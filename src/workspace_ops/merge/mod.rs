mod acceptance;
mod gc;
mod integration;
pub(crate) mod marker;
mod model;
mod open_record;
mod participant_semantics;
mod plan;
mod preserve;
mod publication;
mod record_wire;
mod response;
pub(crate) mod root;
mod runtime;
mod start;
mod status;
mod store;
mod v1_lifecycle;
mod v1_rollback;
// The boundary checker's positive-compile witness for the v1 compiler root
// (`test_v1_compiler_root_has_a_positive_sentinel`). A1's G1 dropped this
// const's `cfg(test)`, not the const: an anonymous production reference costs
// nothing and keeps the witness that the v1 tree is compiler-reachable.
const _: &str = v1_lifecycle::COMPILER_ROOT_SENTINEL;
mod validate;

pub(crate) use model::v1::RecordVersion;
#[cfg(test)]
pub(crate) use model::v1::test_record as test_v1_record;
pub(crate) use model::*;
#[cfg(test)]
pub(crate) use open_record::{ArchivedMergeRecord, read_archived_record};
// `OpenMergeRecord` is named only from `cfg(test)` callers today, but it is the
// return type of the `discover_open_v1_record` row beside it; the re-export is
// held at its existing crate visibility rather than narrowed to `cfg(test)`.
#[allow(unused_imports)]
pub(crate) use open_record::{
    OpenMergeRecord, OpenRecordEnvelope, classify_open_record,
    discover_open_envelope_before_manifest, discover_open_v1_record,
};
#[cfg(test)]
pub(crate) use preserve::v1_write_preservation_bundle_for_test;
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
pub(crate) use runtime::{
    MergeDependencies, enforce_open_merge_stage_targets, guarded_workspace_root,
};
pub use runtime::{
    WorkspaceMutationGuard, acquire_workspace_mutation_guard, enforce_workspace_open_merge_gate,
    handle_merge, handle_merge_with_events,
};
pub(in crate::workspace_ops) use status::MergeStatusRecordView;
pub(crate) use store::{FileMergeStore, MergeStore};
pub(crate) use validate::{validate_merge_request, validate_open_merge_id};
