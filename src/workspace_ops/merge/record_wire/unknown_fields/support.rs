use std::collections::BTreeSet;

use serde_yaml::{Mapping, Value};

use super::{
    ContainerSegment, IdentityValue, SemanticIdentity, UnknownFieldLocator, UnknownFieldManifest,
    UnknownFieldManifestError, error,
};

pub(super) type Path = Vec<ContainerSegment>;

pub(super) fn mapping<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a Mapping, UnknownFieldManifestError> {
    value
        .as_mapping()
        .ok_or_else(|| error(format!("{context} is not a mapping")))
}

pub(super) fn sequence<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a [Value], UnknownFieldManifestError> {
    value
        .as_sequence()
        .map(Vec::as_slice)
        .ok_or_else(|| error(format!("{context} is not a sequence")))
}

pub(super) fn field<'a>(mapping: &'a Mapping, name: &str) -> Option<&'a Value> {
    mapping.get(Value::String(name.to_owned()))
}

pub(super) fn required_field<'a>(
    mapping: &'a Mapping,
    name: &str,
    context: &str,
) -> Result<&'a Value, UnknownFieldManifestError> {
    field(mapping, name).ok_or_else(|| error(format!("{context}.{name} is missing")))
}

pub(super) fn scalar_identity(
    value: &Value,
    context: &str,
) -> Result<IdentityValue, UnknownFieldManifestError> {
    match value {
        Value::Null => Ok(IdentityValue::Null),
        Value::Bool(value) => Ok(IdentityValue::Bool(*value)),
        Value::Number(value) => Ok(IdentityValue::Number(value.to_string())),
        Value::String(value) => Ok(IdentityValue::String(value.clone())),
        _ => Err(error(format!("{context} is not a scalar identity field"))),
    }
}

pub(super) fn optional_identity(
    mapping: &Mapping,
    name: &str,
    context: &str,
) -> Result<IdentityValue, UnknownFieldManifestError> {
    field(mapping, name).map_or(Ok(IdentityValue::Null), |value| {
        scalar_identity(value, &format!("{context}.{name}"))
    })
}

pub(super) fn string_field(
    mapping: &Mapping,
    name: &str,
    context: &str,
) -> Result<String, UnknownFieldManifestError> {
    required_field(mapping, name, context)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error(format!("{context}.{name} is not a string")))
}

pub(super) fn child(path: &Path, name: &str) -> Path {
    let mut child = path.clone();
    child.push(ContainerSegment::Field(name.to_owned()));
    child
}

pub(super) fn map_child(path: &Path, key: &str) -> Path {
    let mut child = path.clone();
    child.push(ContainerSegment::MapKey(key.to_owned()));
    child
}

pub(super) fn identity_child(path: &Path, identity: SemanticIdentity) -> Path {
    let mut child = path.clone();
    child.push(ContainerSegment::Identity(identity));
    child
}

pub(super) fn collect_unknown(
    raw: &Mapping,
    known: &[&str],
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let known = known.iter().copied().collect::<BTreeSet<_>>();
    for (key, value) in raw {
        let Some(key) = key.as_str() else {
            return Err(error("unknown field key is not a string"));
        };
        if !known.contains(key) {
            manifest.insert(
                UnknownFieldLocator {
                    container: path.clone(),
                    field: key.to_owned(),
                },
                value.clone(),
            )?;
        }
    }
    Ok(())
}

pub(super) fn identity(
    kind: &str,
    fields: Vec<(&str, IdentityValue)>,
    occurrence: usize,
) -> SemanticIdentity {
    SemanticIdentity {
        kind: kind.to_owned(),
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        occurrence,
    }
}

pub(super) fn require_unique(
    seen: &mut BTreeSet<SemanticIdentity>,
    identity: &SemanticIdentity,
    context: &str,
) -> Result<(), UnknownFieldManifestError> {
    if !seen.insert(identity.clone()) {
        return Err(error(format!("{context} identity is duplicated")));
    }
    Ok(())
}
