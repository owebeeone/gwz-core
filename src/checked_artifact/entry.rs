//! The complete production entry surface for checked artifacts.
//!
//! Keep callers fully qualified. The boundary checker enumerates every symbol
//! here and every production call site, so an unclassified entry or caller is
//! a hard failure.

use std::path::Path;

use super::{CheckedArtifact, CheckedArtifactPolicy};
use crate::model::{ErrorCode, ModelResult};

pub(crate) fn acquire_merge_root_artifact(
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{}'", relative.display()),
    )
}

pub(crate) fn acquire_merge_preservation_bundle(
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "preservation bundle",
    )
}

pub(crate) fn acquire_merge_preservation_workspace(
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
    )
}

pub(crate) fn acquire_merge_preservation_git_directory(
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::git_directory(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
    )
}

pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {
    CheckedArtifact::prepare_parent(
        root,
        Path::new(crate::stash::STASH_BUNDLE_DIR),
        ErrorCode::MergeRecoveryRequired,
        "preservation bundle parent",
    )
}
