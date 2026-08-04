use serde_yaml::{Mapping, Value};

use super::identity;
use super::support::{field, mapping};
use super::{
    ContainerSegment, SemanticIdentity, UnknownFieldManifest, UnknownFieldManifestError, error,
};

pub(super) fn apply_surviving(
    source: &UnknownFieldManifest,
    next: &mut Value,
) -> Result<UnknownFieldManifest, UnknownFieldManifestError> {
    let replacement = UnknownFieldManifest::extract_v1(next)?;
    let mut expected = UnknownFieldManifest::default();
    for (locator, value) in source.entries() {
        let Some(container) = resolve_container(next, &locator.container)? else {
            continue;
        };
        expected.entries.insert(locator.clone(), value.clone());
        let _ = container;
    }
    for (locator, value) in replacement.entries() {
        if expected.entries.get(locator) != Some(value) {
            return Err(error(format!(
                "replacement contains unauthorized unknown field '{}'",
                locator.field
            )));
        }
    }
    for (locator, value) in expected.entries() {
        let Some(container) = resolve_container(next, &locator.container)? else {
            return Err(error("surviving unknown-field container disappeared"));
        };
        let key = Value::String(locator.field.clone());
        match container.get(&key) {
            Some(existing) if existing != value => {
                return Err(error(format!(
                    "unknown field '{}' collides with a different replacement value",
                    locator.field
                )));
            }
            Some(_) => {}
            None => {
                container.insert(key, value.clone());
            }
        }
    }
    let verified = UnknownFieldManifest::extract_v1(next)?;
    if verified != expected {
        return Err(error(
            "unknown-field manifest changed while applying surviving fields",
        ));
    }
    Ok(verified)
}

fn resolve_container<'a>(
    value: &'a mut Value,
    path: &[ContainerSegment],
) -> Result<Option<&'a mut Mapping>, UnknownFieldManifestError> {
    let Some((segment, rest)) = path.split_first() else {
        return value
            .as_mapping_mut()
            .map(Some)
            .ok_or_else(|| error("unknown-field container is not a mapping"));
    };
    match segment {
        ContainerSegment::Field(name) | ContainerSegment::MapKey(name) => {
            let Some(mapping) = value.as_mapping_mut() else {
                return Err(error("unknown-field path traverses a non-mapping value"));
            };
            let Some(child) = mapping.get_mut(Value::String(name.clone())) else {
                return Ok(None);
            };
            if child.is_null() {
                return Ok(None);
            }
            resolve_container(child, rest)
        }
        ContainerSegment::Identity(wanted) => resolve_identity(value, wanted, rest),
    }
}

fn resolve_identity<'a>(
    value: &'a mut Value,
    wanted: &SemanticIdentity,
    rest: &[ContainerSegment],
) -> Result<Option<&'a mut Mapping>, UnknownFieldManifestError> {
    if value.is_mapping() {
        let actual = identity_for_mapping(mapping(value, "identity container")?, wanted)?;
        if actual != *wanted {
            return Ok(None);
        }
        return resolve_container(value, rest);
    }
    let Some(sequence) = value.as_sequence_mut() else {
        return Err(error(
            "identity path traverses neither a mapping nor a sequence",
        ));
    };
    let mut prior = Vec::new();
    let mut matched_index = None;
    for (index, value) in sequence.iter().enumerate() {
        let row = mapping(value, "identity sequence row")?;
        let mut actual = identity_for_mapping(row, wanted)?;
        if wanted.kind == "participant_drift" {
            actual.occurrence = identity::occurrence_for(&prior, &actual);
            prior.push(actual.clone());
        }
        if actual == *wanted && matched_index.replace(index).is_some() {
            return Err(error("identity sequence contains a duplicate identity"));
        }
    }
    let Some(index) = matched_index else {
        return Ok(None);
    };
    resolve_container(&mut sequence[index], rest)
}

fn identity_for_mapping(
    row: &Mapping,
    wanted: &SemanticIdentity,
) -> Result<SemanticIdentity, UnknownFieldManifestError> {
    match wanted.kind.as_str() {
        "conflict_evidence" => identity::conflict(row, 0),
        "candidate_hash" => identity::candidate_hash(row, 0),
        "participant_drift" => identity::participant_drift(row, 0),
        "operation_drift" => identity::operation_drift(row, 0),
        "participant_error" => identity::participant_error(row),
        "participant_error_scope" => identity::participant_error_scope(row),
        "pending_action" => identity::pending_action(row),
        "pending_rollback" => identity::pending_rollback(row),
        "pending_preservation" => identity::pending_preservation(row),
        "recovery_context" => identity::recovery_context(row),
        "preservation_evidence" => {
            if field(row, "backup_ref").is_none()
                && field(row, "backup_commit").is_none()
                && field(row, "stash_id").is_none()
                && field(row, "stash_object_id").is_none()
            {
                return Err(error("preservation evidence row has no known fields"));
            }
            Ok(wanted.clone())
        }
        _ => Err(error(format!(
            "unknown semantic identity kind '{}'",
            wanted.kind
        ))),
    }
}
