//! Preservation — `gwz merge --abort --preserve`'s durable evidence.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** This file was the v0 engine's
//! `preserve_then_abort` coordinator: discover the open v0 record, transition
//! it to `Preserving`, create backup refs and stashes, then abort. The v1
//! reverse service drives the same artifacts through its own phase kernel
//! (`v1_lifecycle/authority/observe/reverse/preservation/`), so what remains
//! here is the observation and planning surface that service calls.

use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::PreservationEvidence;

mod artifacts;
mod checked_bundle;
mod plan;

#[cfg(test)]
pub(in crate::workspace_ops::merge) use artifacts::V1_PRESERVATION_IMAGE_CAPTURES;
pub(in crate::workspace_ops::merge) use artifacts::{
    v1_preservation_image, v1_root_preservation_spec,
};
pub(in crate::workspace_ops::merge) use checked_bundle::{
    V1BundleObservation, v1_bundle_cursor_is_exact, v1_bundle_observation, v1_write_bundle_checked,
};
pub(in crate::workspace_ops::merge) use plan::{
    V1PreservationOwnerPlan, v1_owner_evidence, v1_preservation_owners,
};

/// Write `member_id`'s on-disk preservation bundle exactly as an interrupted
/// production run would have left it, for suites outside `merge` that hand-build
/// a crashed `Preserving` record.
///
/// A member's stash step writes its durable `PreservationEvidence` and then its
/// bundle entry (`PreservationStashPhaseV1::WriteBundle`, the last phase before
/// the owner completes), so a fixture that writes only the evidence stages a
/// half-step no production run can produce, and the entry preflight refuses it:
/// `v1_bundle_cursor_is_exact` derives the expected bundle from every owner
/// whose evidence carries a `stash_object_id` and finds nothing on disk. This
/// runs the production writer itself over the record as it now stands, so the
/// fixture cannot drift from what production would have written.
///
/// Deliberately narrow: it takes a member id and returns nothing, keeping
/// `MergeOperationRecordV1`, `PreservationOwnerV1` and the owner plans inside
/// `merge`. It is `#[cfg(test)]` and re-exported from `merge/mod.rs` the way
/// `discover_open_v1_record` and `read_archived_record` already are.
#[cfg(test)]
pub(crate) fn v1_write_preservation_bundle_for_test<B: crate::git::MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    member_id: &str,
) -> ModelResult<()> {
    let open = super::open_record::discover_open_v1_record(root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::OperationNotFound,
            "no open merge record to build a preservation bundle for",
        )
    })?;
    let record = open.record_for_test();
    let plans = v1_preservation_owners(backend, root, record)?;
    v1_write_bundle_checked(
        root,
        record,
        &plans,
        &super::model::v1::PreservationOwnerV1::Participant {
            member_id: member_id.to_owned(),
        },
    )
}
