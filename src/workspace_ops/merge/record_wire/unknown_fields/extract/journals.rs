use serde_yaml::Mapping;

use super::super::identity;
use super::super::support::{
    Path, child, collect_unknown, field, identity_child, mapping, string_field,
};
use super::super::{UnknownFieldManifest, UnknownFieldManifestError, error};

pub(super) fn extract(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    extract_recovery(root, path, manifest)?;
    extract_rollback(root, path, manifest)?;
    extract_preservation(root, path, manifest)
}

fn extract_recovery(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "recovery_context").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let recovery = mapping(value, "recovery context")?;
    collect_unknown(
        recovery,
        &["origin_state"],
        &identity_child(
            &child(path, "recovery_context"),
            identity::recovery_context(recovery)?,
        ),
        manifest,
    )
}

fn extract_rollback(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "pending_rollback").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let rollback = mapping(value, "pending rollback")?;
    let known = match string_field(rollback, "kind", "pending rollback")?.as_str() {
        "participant" => vec!["kind", "member_id", "action", "terminal_state"],
        "publication_evidence" | "selected_root_metadata" => vec!["kind", "next_step"],
        _ => return Err(error("pending rollback kind is unknown")),
    };
    collect_unknown(
        rollback,
        &known,
        &identity_child(
            &child(path, "pending_rollback"),
            identity::pending_rollback(rollback)?,
        ),
        manifest,
    )
}

fn extract_preservation(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "pending_preservation").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let preservation = mapping(value, "pending preservation")?;
    let kind = string_field(preservation, "kind", "pending preservation")?;
    let known = match kind.as_str() {
        "backup_ref" => vec!["kind", "owner", "name", "target_commit"],
        "stash" => vec![
            "kind",
            "owner",
            "phase",
            "stash_id",
            "stash_object_id",
            "message",
            "head_commit",
            "preimage_sha256",
            "root_publication_prefix",
        ],
        "reset_attached_ref" => vec![
            "kind",
            "owner",
            "branch",
            "expected_commit",
            "restore_commit",
            "phase",
            "root_publication_prefix",
        ],
        _ => return Err(error("pending preservation kind is unknown")),
    };
    let preservation_path = identity_child(
        &child(path, "pending_preservation"),
        identity::pending_preservation(preservation)?,
    );
    collect_unknown(preservation, &known, &preservation_path, manifest)?;
    if let Some(owner) = field(preservation, "owner") {
        let owner = mapping(owner, "preservation owner")?;
        let known = match string_field(owner, "kind", "preservation owner")?.as_str() {
            "participant" => vec!["kind", "member_id"],
            "publication_root" => vec!["kind"],
            _ => return Err(error("preservation owner kind is unknown")),
        };
        collect_unknown(owner, &known, &child(&preservation_path, "owner"), manifest)?;
    }
    if let Some(object_id) = field(preservation, "stash_object_id").filter(|value| !value.is_null())
    {
        collect_unknown(
            mapping(object_id, "stash object id")?,
            &["algorithm", "digest_hex"],
            &child(&preservation_path, "stash_object_id"),
            manifest,
        )?;
    }
    Ok(())
}
