use serde_yaml::Mapping;

use super::super::support::{
    Path, child, collect_unknown, field, map_child, mapping, string_field,
};
use super::super::{UnknownFieldManifest, UnknownFieldManifestError, error};

pub(super) fn extract(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "accepted_workspace").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let accepted = mapping(value, "accepted workspace")?;
    collect_unknown(
        accepted,
        &[
            "operation_baseline_lock_sha256",
            "metadata_base",
            "lock",
            "member_audit",
            "root",
        ],
        path,
        manifest,
    )?;
    extract_metadata(accepted, path, manifest)?;
    extract_lock(accepted, path, manifest)?;
    extract_member_audit(accepted, path, manifest)?;
    extract_root(accepted, path, manifest)
}

fn extract_metadata(
    accepted: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(accepted, "metadata_base") else {
        return Ok(());
    };
    let metadata = mapping(value, "accepted metadata base")?;
    let metadata_path = child(path, "metadata_base");
    collect_unknown(
        metadata,
        &[
            "source",
            "manifest_exact_yaml",
            "manifest_sha256",
            "lock_exact_yaml",
            "lock_sha256",
        ],
        &metadata_path,
        manifest,
    )?;
    let Some(source) = field(metadata, "source") else {
        return Ok(());
    };
    let source = mapping(source, "accepted metadata source")?;
    let known = match string_field(source, "kind", "accepted metadata source")?.as_str() {
        "operation_baseline" => vec!["kind"],
        "selected_root_result" => vec!["kind", "commit"],
        _ => return Err(error("accepted metadata source kind is unknown")),
    };
    collect_unknown(source, &known, &child(&metadata_path, "source"), manifest)
}

fn extract_lock(
    accepted: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(accepted, "lock") else {
        return Ok(());
    };
    collect_unknown(
        mapping(value, "accepted lock")?,
        &["exact_yaml", "sha256"],
        &child(path, "lock"),
        manifest,
    )
}

fn extract_member_audit(
    accepted: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(accepted, "member_audit") else {
        return Ok(());
    };
    let audit_path = child(path, "member_audit");
    for (member_id, value) in mapping(value, "member audit")? {
        let Some(member_id) = member_id.as_str() else {
            return Err(error("member-audit key is not a string"));
        };
        let member = mapping(value, "member acceptance")?;
        let member_path = map_child(&audit_path, member_id);
        let known = match string_field(member, "kind", "member acceptance")?.as_str() {
            "selected" => vec!["kind", "integration", "final_checkout", "lock_member"],
            "unselected_present" => vec!["kind", "lock_member"],
            "absent" => vec!["kind"],
            _ => return Err(error("member acceptance kind is unknown")),
        };
        collect_unknown(member, &known, &member_path, manifest)?;
        extract_member_children(member, &member_path, manifest)?;
    }
    Ok(())
}

fn extract_member_children(
    member: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    if let Some(value) = field(member, "integration") {
        collect_unknown(
            mapping(value, "accepted integration")?,
            &["branch", "before_commit", "resulting_commit"],
            &child(path, "integration"),
            manifest,
        )?;
    }
    if let Some(value) = field(member, "final_checkout") {
        collect_unknown(
            mapping(value, "accepted checkout")?,
            &["branch", "commit"],
            &child(path, "final_checkout"),
            manifest,
        )?;
    }
    if let Some(value) = field(member, "lock_member") {
        collect_unknown(
            mapping(value, "accepted lock member")?,
            &[
                "path",
                "source_id",
                "source_kind",
                "commit",
                "branch",
                "detached",
                "upstream",
                "dirty",
                "materialized",
            ],
            &child(path, "lock_member"),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_root(
    accepted: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(accepted, "root") else {
        return Ok(());
    };
    let root = mapping(value, "accepted root")?;
    let root_path = child(path, "root");
    collect_unknown(
        root,
        &["base", "publication_branch", "baseline_artifact_hashes"],
        &root_path,
        manifest,
    )?;
    if let Some(value) = field(root, "base") {
        let base = mapping(value, "accepted root base")?;
        let known = match string_field(base, "kind", "accepted root base")?.as_str() {
            "born_attached" => vec!["kind", "commit", "symbolic_branch"],
            "born_detached" => vec!["kind", "commit"],
            "unborn_attached" => vec!["kind", "symbolic_branch"],
            _ => return Err(error("accepted root base kind is unknown")),
        };
        collect_unknown(base, &known, &child(&root_path, "base"), manifest)?;
    }
    if let Some(value) = field(root, "baseline_artifact_hashes") {
        collect_unknown(
            mapping(value, "root artifact hashes")?,
            &[
                "lock_worktree_sha256",
                "manifest_worktree_sha256",
                "lock_commit_sha256",
                "manifest_commit_sha256",
            ],
            &child(&root_path, "baseline_artifact_hashes"),
            manifest,
        )?;
    }
    Ok(())
}
