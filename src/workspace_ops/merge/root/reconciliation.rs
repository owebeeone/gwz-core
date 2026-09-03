use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::ManifestArtifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::{MergeStatusRecordView, MergeTargetKind};

/// The manifest the open merge froze, read through the version-agnostic
/// common view so an open v1 record reaches it too.
pub(in crate::workspace_ops::merge) fn frozen_manifest<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeStatusRecordView<'_>,
) -> ModelResult<ManifestArtifact> {
    let bytes = match record.participants().get("@root") {
        Some(participant)
            if participant.target_kind == MergeTargetKind::Root && participant.path == "." =>
        {
            backend
                .read_file_at_commit(root, &participant.before_commit, WORKSPACE_MANIFEST)?
                .ok_or_else(|| {
                    unreadable("root before commit does not contain the workspace manifest")
                })?
        }
        Some(_) => return Err(unreadable("root participant identity is inconsistent")),
        None => fs::read(root.join(WORKSPACE_MANIFEST)).map_err(|error| {
            ModelError::new(
                ErrorCode::IoError,
                format!("failed to read workspace manifest: {error}"),
            )
        })?,
    };
    if record
        .baseline()
        .manifest_commit_sha256
        .as_deref()
        .is_some_and(|expected| format!("{:x}", Sha256::digest(&bytes)) != expected)
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "frozen workspace manifest does not match the merge baseline",
        ));
    }
    let yaml = String::from_utf8(bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("frozen workspace manifest is not UTF-8: {error}"),
        )
    })?;
    let manifest = ManifestArtifact::from_yaml(&yaml)?;
    if manifest.workspace.id != record.workspace_id() {
        return Err(ModelError::new(
            ErrorCode::SourceIdentityMismatch,
            "frozen workspace manifest identifies a different workspace",
        ));
    }
    Ok(manifest)
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}
