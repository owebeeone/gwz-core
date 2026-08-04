mod extract;
mod identity;
mod overlay;
mod support;

use std::collections::BTreeMap;

use serde_yaml::Value;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum IdentityValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SemanticIdentity {
    pub(crate) kind: String,
    pub(crate) fields: Vec<(String, IdentityValue)>,
    pub(crate) occurrence: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ContainerSegment {
    Field(String),
    MapKey(String),
    Identity(SemanticIdentity),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct UnknownFieldLocator {
    pub(crate) container: Vec<ContainerSegment>,
    pub(crate) field: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct UnknownFieldManifest {
    entries: BTreeMap<UnknownFieldLocator, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnknownFieldManifestError {
    pub(crate) detail: String,
}

impl UnknownFieldManifest {
    pub(crate) fn extract_v0(raw: &Value) -> Result<Self, UnknownFieldManifestError> {
        extract::extract_v0(raw)
    }

    pub(crate) fn extract_v1(raw: &Value) -> Result<Self, UnknownFieldManifestError> {
        extract::extract_v1(raw)
    }

    pub(crate) fn apply_surviving(
        &self,
        next: &mut Value,
    ) -> Result<Self, UnknownFieldManifestError> {
        overlay::apply_surviving(self, next)
    }

    pub(crate) fn map_v0_to_v1(&self) -> Result<Self, UnknownFieldManifestError> {
        const V1_TOP_LEVEL: [&str; 4] = [
            "accepted_workspace",
            "recovery_context",
            "pending_rollback",
            "pending_preservation",
        ];
        if let Some(locator) = self.entries.keys().find(|locator| {
            locator.container.is_empty() && V1_TOP_LEVEL.contains(&locator.field.as_str())
        }) {
            return Err(error(format!(
                "v0 unknown field '{}' collides with a v1 top-level field",
                locator.field
            )));
        }
        Ok(self.clone())
    }

    pub(crate) fn entries(&self) -> &BTreeMap<UnknownFieldLocator, Value> {
        &self.entries
    }

    fn insert(
        &mut self,
        locator: UnknownFieldLocator,
        value: Value,
    ) -> Result<(), UnknownFieldManifestError> {
        if self.entries.insert(locator, value).is_some() {
            return Err(error("unknown-field locator is not unique"));
        }
        Ok(())
    }
}

fn error(detail: impl Into<String>) -> UnknownFieldManifestError {
    UnknownFieldManifestError {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
