mod accepted;
mod common;
mod journals;

use serde_yaml::Value;

use super::support::{Path, child, collect_unknown, mapping};
use super::{UnknownFieldManifest, UnknownFieldManifestError};

pub(super) fn extract_v0(raw: &Value) -> Result<UnknownFieldManifest, UnknownFieldManifestError> {
    let root = mapping(raw, "merge record")?;
    let mut manifest = UnknownFieldManifest::default();
    let path = Path::new();
    collect_unknown(
        root,
        &[
            "schema",
            "record_schema_version",
            "writer_version",
            "workspace_id",
            "merge_id",
            "operation_id",
            "state",
            "source_ref",
            "mode",
            "created_at",
            "baseline",
            "selected_targets",
            "participants",
            "publication",
            "operation_drift",
        ],
        &path,
        &mut manifest,
    )?;
    common::extract(root, &path, common::EvidenceKeys::V0, &mut manifest)?;
    Ok(manifest)
}

pub(super) fn extract_v1(raw: &Value) -> Result<UnknownFieldManifest, UnknownFieldManifestError> {
    let root = mapping(raw, "merge record")?;
    let mut manifest = UnknownFieldManifest::default();
    let path = Path::new();
    collect_unknown(
        root,
        &[
            "schema",
            "record_schema_version",
            "writer_version",
            "workspace_id",
            "merge_id",
            "operation_id",
            "state",
            "source_ref",
            "mode",
            "created_at",
            "baseline",
            "selected_targets",
            "participants",
            "publication",
            "operation_drift",
            "accepted_workspace",
            "recovery_context",
            "pending_rollback",
            "pending_preservation",
            "preservation_publication_handoff",
        ],
        &path,
        &mut manifest,
    )?;
    common::extract(root, &path, common::EvidenceKeys::V1, &mut manifest)?;
    accepted::extract(root, &child(&path, "accepted_workspace"), &mut manifest)?;
    journals::extract(root, &path, &mut manifest)?;
    Ok(manifest)
}
