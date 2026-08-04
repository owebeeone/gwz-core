use serde_yaml::Mapping;

use super::support::{
    field, identity, mapping, optional_identity, required_field, scalar_identity, string_field,
};
use super::{IdentityValue, SemanticIdentity, UnknownFieldManifestError, error};

pub(super) fn conflict(
    row: &Mapping,
    occurrence: usize,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "conflict_evidence",
        vec![
            (
                "path",
                scalar_identity(required_field(row, "path", "conflict evidence")?, "path")?,
            ),
            (
                "sha256",
                scalar_identity(
                    required_field(row, "sha256", "conflict evidence")?,
                    "sha256",
                )?,
            ),
        ],
        occurrence,
    ))
}

pub(super) fn candidate_hash(
    row: &Mapping,
    occurrence: usize,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "candidate_hash",
        vec![(
            "path",
            scalar_identity(required_field(row, "path", "candidate hash")?, "path")?,
        )],
        occurrence,
    ))
}

pub(super) fn participant_drift(
    row: &Mapping,
    occurrence: usize,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    let context = "participant drift";
    Ok(identity(
        "participant_drift",
        vec![
            (
                "kind",
                scalar_identity(required_field(row, "kind", context)?, "kind")?,
            ),
            (
                "expected_branch",
                optional_identity(row, "expected_branch", context)?,
            ),
            (
                "live_branch",
                optional_identity(row, "live_branch", context)?,
            ),
            (
                "expected_head",
                optional_identity(row, "expected_head", context)?,
            ),
            ("live_head", optional_identity(row, "live_head", context)?),
            (
                "expected_merge_head",
                optional_identity(row, "expected_merge_head", context)?,
            ),
            (
                "live_merge_head",
                optional_identity(row, "live_merge_head", context)?,
            ),
        ],
        occurrence,
    ))
}

pub(super) fn operation_drift(
    row: &Mapping,
    occurrence: usize,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "operation_drift",
        vec![(
            "kind",
            scalar_identity(
                required_field(row, "kind", "operation drift")?,
                "operation drift kind",
            )?,
        )],
        occurrence,
    ))
}

pub(super) fn participant_error(
    row: &Mapping,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "participant_error",
        vec![
            (
                "code",
                scalar_identity(
                    required_field(row, "code", "participant error")?,
                    "error code",
                )?,
            ),
            (
                "detail",
                optional_identity(row, "detail", "participant error")?,
            ),
        ],
        0,
    ))
}

pub(super) fn participant_error_scope(
    participant: &Mapping,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "participant_error_scope",
        vec![
            (
                "member_path",
                optional_identity(participant, "path", "participant")?,
            ),
            (
                "target_kind",
                optional_identity(participant, "target_kind", "participant")?,
            ),
        ],
        0,
    ))
}

pub(super) fn pending_action(row: &Mapping) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    let context = "pending action";
    let mut fields = required_scalars(
        row,
        context,
        &[
            "kind",
            "target_branch",
            "before_commit",
            "source_commit",
            "commit_message",
        ],
    )?;
    fields.push((
        "expected_result",
        optional_identity(row, "expected_result", context)?,
    ));
    if let Some(spec) = field(row, "commit_spec").filter(|value| !value.is_null()) {
        append_commit_spec(&mut fields, mapping(spec, "pending action.commit_spec")?)?;
    } else {
        fields.push(("commit_spec", IdentityValue::Null));
    }
    Ok(identity("pending_action", fields, 0))
}

pub(super) fn pending_rollback(
    row: &Mapping,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    let kind = string_field(row, "kind", "pending rollback")?;
    let mut fields = vec![("kind", IdentityValue::String(kind.clone()))];
    if kind == "participant" {
        fields.extend(required_scalars(
            row,
            "pending rollback",
            &["member_id", "action", "terminal_state"],
        )?);
    }
    Ok(identity("pending_rollback", fields, 0))
}

pub(super) fn pending_preservation(
    row: &Mapping,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    let kind = string_field(row, "kind", "pending preservation")?;
    let mut fields = vec![("kind", IdentityValue::String(kind.clone()))];
    let owner = mapping(
        required_field(row, "owner", "pending preservation")?,
        "pending preservation.owner",
    )?;
    append_owner(&mut fields, owner)?;
    match kind.as_str() {
        "backup_ref" => fields.extend(required_scalars(
            row,
            "pending preservation",
            &["name", "target_commit"],
        )?),
        "stash" => {
            fields.extend(required_scalars(
                row,
                "pending preservation",
                &["message", "head_commit", "preimage_sha256"],
            )?);
            fields.push((
                "root_publication_prefix",
                optional_identity(row, "root_publication_prefix", "pending preservation")?,
            ));
        }
        "reset_attached_ref" => {
            fields.extend(required_scalars(
                row,
                "pending preservation",
                &["branch", "expected_commit", "restore_commit"],
            )?);
            fields.push((
                "root_publication_prefix",
                optional_identity(row, "root_publication_prefix", "pending preservation")?,
            ));
        }
        _ => return Err(error("pending preservation kind is unknown")),
    }
    Ok(identity("pending_preservation", fields, 0))
}

pub(super) fn recovery_context(
    row: &Mapping,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    Ok(identity(
        "recovery_context",
        vec![(
            "origin_state",
            scalar_identity(
                required_field(row, "origin_state", "recovery context")?,
                "recovery origin",
            )?,
        )],
        0,
    ))
}

pub(super) fn preservation_owner(owner: &str) -> SemanticIdentity {
    identity(
        "preservation_evidence",
        vec![("owner", IdentityValue::String(owner.to_owned()))],
        0,
    )
}

fn required_scalars(
    row: &Mapping,
    context: &str,
    names: &[&'static str],
) -> Result<Vec<(&'static str, IdentityValue)>, UnknownFieldManifestError> {
    names
        .iter()
        .map(|name| {
            Ok((
                *name,
                scalar_identity(required_field(row, name, context)?, name)?,
            ))
        })
        .collect()
}

fn append_commit_spec(
    fields: &mut Vec<(&'static str, IdentityValue)>,
    spec: &Mapping,
) -> Result<(), UnknownFieldManifestError> {
    fields.push((
        "commit_spec.tree_oid",
        scalar_identity(
            required_field(spec, "tree_oid", "commit spec")?,
            "commit spec.tree_oid",
        )?,
    ));
    for role in ["author", "committer"] {
        let signature = mapping(
            required_field(spec, role, "commit spec")?,
            &format!("commit spec.{role}"),
        )?;
        for name in ["name", "email", "time_seconds", "timezone_offset_minutes"] {
            fields.push((
                match (role, name) {
                    ("author", "name") => "commit_spec.author.name",
                    ("author", "email") => "commit_spec.author.email",
                    ("author", "time_seconds") => "commit_spec.author.time_seconds",
                    ("author", _) => "commit_spec.author.timezone_offset_minutes",
                    ("committer", "name") => "commit_spec.committer.name",
                    ("committer", "email") => "commit_spec.committer.email",
                    ("committer", "time_seconds") => "commit_spec.committer.time_seconds",
                    _ => "commit_spec.committer.timezone_offset_minutes",
                },
                scalar_identity(required_field(signature, name, role)?, name)?,
            ));
        }
    }
    Ok(())
}

fn append_owner(
    fields: &mut Vec<(&'static str, IdentityValue)>,
    owner: &Mapping,
) -> Result<(), UnknownFieldManifestError> {
    let kind = string_field(owner, "kind", "preservation owner")?;
    fields.push(("owner.kind", IdentityValue::String(kind.clone())));
    fields.push((
        "owner.member_id",
        if kind == "participant" {
            scalar_identity(
                required_field(owner, "member_id", "preservation owner")?,
                "owner.member_id",
            )?
        } else {
            IdentityValue::Null
        },
    ));
    Ok(())
}

pub(super) fn same_base(left: &SemanticIdentity, right: &SemanticIdentity) -> bool {
    left.kind == right.kind && left.fields == right.fields
}

pub(super) fn occurrence_for(prior: &[SemanticIdentity], candidate: &SemanticIdentity) -> usize {
    prior
        .iter()
        .filter(|identity| same_base(identity, candidate))
        .count()
}
