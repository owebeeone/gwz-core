//! Preservation — `gwz merge --abort --preserve`'s durable evidence.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** This file was the v0 engine's
//! `preserve_then_abort` coordinator: discover the open v0 record, transition
//! it to `Preserving`, create backup refs and stashes, then abort. The v1
//! reverse service drives the same artifacts through its own phase kernel
//! (`v1_lifecycle/authority/observe/reverse/preservation/`), so what remains
//! here is the observation and planning surface that service calls.

use std::path::{Path, PathBuf};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::{MergeParticipantRecord, MergeTargetKind, PreservationEvidence};

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
