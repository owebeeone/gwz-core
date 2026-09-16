//! Field, schema and identifier validation shared by every artifact reader.

use super::*;

pub(super) fn require_schema(actual: &str, expected: &str) -> ModelResult<()> {
    let expected_major =
        schema_major(expected).ok_or_else(|| invalid("invalid expected schema"))?;
    match schema_major(actual) {
        Some(actual_major) if actual == expected && actual_major == expected_major => Ok(()),
        Some(_) => Err(ModelError::new(
            ErrorCode::SchemaUnsupported,
            format!("unsupported schema {actual}; expected {expected}"),
        )),
        None => Err(ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("invalid schema {actual}"),
        )),
    }
}

pub(super) fn schema_major(schema: &str) -> Option<u32> {
    let (_, major) = schema.rsplit_once("/v")?;
    major.parse().ok()
}

pub(super) fn validate_member_record(
    created_at: &str,
    created_by: &CreatedByArtifact,
    selected_members: &[String],
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> ModelResult<()> {
    require_non_empty("created_at", created_at)?;
    created_by.validate()?;
    for member_id in selected_members {
        parse_id("selected member", "mem_", member_id)?;
    }
    for (member_id, member) in members {
        parse_id("member id", "mem_", member_id)?;
        member.validate(false)?;
    }
    Ok(())
}

pub(super) fn validate_target_ref(field: &str, value: &str) -> ModelResult<()> {
    if value == "@root" || value == "@default" {
        return Ok(());
    }
    if value.starts_with('@') {
        return require_non_empty(field, value);
    }
    parse_id(field, "mem_", value)
}

pub(super) fn require_uuid_v7(field: &str, value: &str) -> ModelResult<()> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[14] == b'7'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes.iter().enumerate().all(|(idx, byte)| {
            [8, 13, 18, 23].contains(&idx) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
        });
    if valid {
        Ok(())
    } else {
        Err(invalid(format!("{field} must be a canonical UUIDv7")))
    }
}

pub(super) fn validate_origin_url_hash(value: &str) -> ModelResult<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(invalid("origin_url_hash must start with sha256:"));
    };
    let valid = hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "origin_url_hash must be sha256:<64 lowercase hex chars>",
        ))
    }
}

pub(super) fn reject_duplicate_remote_names(remotes: &[RemoteArtifact]) -> ModelResult<()> {
    let mut names = BTreeSet::new();
    for remote in remotes {
        if !names.insert(remote.name.as_str()) {
            return Err(invalid(format!("duplicate remote name '{}'", remote.name)));
        }
    }
    Ok(())
}

pub(super) fn parse_id(field: &str, prefix: &str, value: &str) -> ModelResult<()> {
    let valid = value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must start with {prefix} and contain only portable characters"
        )))
    }
}

pub(super) fn require_slug(field: &str, value: &str) -> ModelResult<()> {
    require_non_empty(field, value)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must contain only portable slug characters"
        )))
    }
}

// `gwz.snapshot/v0` shipped with unrestricted portable-slug ids. Reads MUST
// retain that schema-v0 grammar forever (including adjacent and boundary dots),
// while creation uses the disjoint range-safe subset below. Do not collapse
// these validators: doing so makes already-written v0 artifacts unreadable.
pub(super) fn validate_snapshot_id_for_read(value: &str) -> ModelResult<()> {
    require_slug("snapshot_id", value)
}

pub(super) fn validate_snapshot_id_for_creation(value: &str) -> ModelResult<()> {
    require_slug("snapshot_id", value)?;
    if value.contains("..") || value.starts_with('.') || value.ends_with('.') {
        return Err(invalid(
            "snapshot_id must not contain adjacent dots or start/end with a dot because those spellings are reserved for revision ranges",
        ));
    }
    Ok(())
}

pub(crate) fn snapshot_id_requires_legacy_compatibility(value: &str) -> bool {
    validate_snapshot_id_for_read(value).is_ok()
        && validate_snapshot_id_for_creation(value).is_err()
}

pub(super) fn optional_text_target(field: &str, value: &Option<String>) -> ModelResult<usize> {
    match value {
        Some(value) => {
            require_non_empty(field, value)?;
            Ok(1)
        }
        None => Ok(0),
    }
}

pub(super) fn validate_optional_text(field: &str, value: &Option<String>) -> ModelResult<()> {
    match value {
        Some(value) => require_non_empty(field, value),
        None => Ok(()),
    }
}

pub(super) fn require_non_empty(field: &str, value: &str) -> ModelResult<()> {
    if value.trim().is_empty() {
        Err(invalid(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

pub(super) fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

pub(super) fn io_error(err: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, err.to_string())
}

pub(super) fn manifest_io_error(err: io::Error) -> ModelError {
    match err.kind() {
        io::ErrorKind::NotFound => ModelError::new(ErrorCode::ManifestNotFound, err.to_string()),
        io::ErrorKind::PermissionDenied => {
            ModelError::new(ErrorCode::PermissionDenied, err.to_string())
        }
        _ => ModelError::new(ErrorCode::ManifestInvalid, err.to_string()),
    }
}
