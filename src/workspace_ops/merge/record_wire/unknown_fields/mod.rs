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
        const V1_TOP_LEVEL: [&str; 5] = [
            "accepted_workspace",
            "recovery_context",
            "pending_rollback",
            "pending_preservation",
            "preservation_publication_handoff",
        ];
        if let Some(locator) = self.entries.keys().find(|locator| {
            locator.container.is_empty() && V1_TOP_LEVEL.contains(&locator.field.as_str())
        }) {
            return Err(error(format!(
                "v0 unknown field '{}' collides with a v1 top-level field",
                locator.field
            )));
        }
        // `GwzM5-8DurableCursorAmendment.md` §2.3: the collision doctrine
        // extends into preservation evidence rows. Presence of either durable
        // cursor marker inside a v0 record's evidence row makes migration
        // ineligible; the value is never adopted, overwritten, or moved.
        const V1_EVIDENCE_ROW: [&str; 2] = ["noop_commit", "reset_commit"];
        if let Some(locator) = self.entries.keys().find(|locator| {
            V1_EVIDENCE_ROW.contains(&locator.field.as_str())
                && matches!(
                    locator.container.last(),
                    Some(ContainerSegment::Identity(identity))
                        if identity.kind == "preservation_evidence"
                )
        }) {
            return Err(error(format!(
                "v0 unknown field '{}' collides with a v1 preservation evidence field",
                locator.field
            )));
        }
        Ok(self.clone())
    }

    pub(crate) fn authorize_derived_accepted_lock_members(
        &mut self,
        replacement: &Self,
    ) -> Result<(), UnknownFieldManifestError> {
        for (locator, value) in replacement.entries() {
            if let Some(source) = self.entries.get(locator) {
                if source != value {
                    return Err(error(format!(
                        "unknown field '{}' changed while deriving accepted lock audit",
                        locator.field
                    )));
                }
                continue;
            }
            if !is_accepted_lock_member(locator) {
                return Err(error(format!(
                    "derived v1 record introduced unauthorized unknown field '{}'",
                    locator.field
                )));
            }
            self.entries.insert(locator.clone(), value.clone());
        }
        Ok(())
    }

    pub(crate) fn entries(&self) -> &BTreeMap<UnknownFieldLocator, Value> {
        &self.entries
    }

    pub(crate) fn after_participant_drift_retirement(
        &self,
        member_id: &str,
        retired: &SemanticIdentity,
    ) -> Result<Self, UnknownFieldManifestError> {
        let mut next = Self::default();
        for (locator, value) in &self.entries {
            let mut locator = locator.clone();
            if let Some(identity) = participant_drift_identity(&mut locator, member_id)
                && same_identity_key(identity, retired)
            {
                if identity.occurrence == retired.occurrence {
                    continue;
                }
                if identity.occurrence > retired.occurrence {
                    identity.occurrence -= 1;
                }
            }
            next.insert(locator, value.clone())?;
        }
        Ok(next)
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

fn participant_drift_identity<'a>(
    locator: &'a mut UnknownFieldLocator,
    member_id: &str,
) -> Option<&'a mut SemanticIdentity> {
    match locator.container.as_mut_slice() {
        [
            ContainerSegment::Field(participants),
            ContainerSegment::MapKey(actual_member),
            ContainerSegment::Field(drift),
            ContainerSegment::Identity(identity),
            ..,
        ] if participants == "participants"
            && actual_member == member_id
            && drift == "drift"
            && identity.kind == "participant_drift" =>
        {
            Some(identity)
        }
        _ => None,
    }
}

fn same_identity_key(left: &SemanticIdentity, right: &SemanticIdentity) -> bool {
    left.kind == right.kind && left.fields == right.fields
}

fn is_accepted_lock_member(locator: &UnknownFieldLocator) -> bool {
    matches!(
        locator.container.as_slice(),
        [
            ContainerSegment::Field(accepted),
            ContainerSegment::Field(audit),
            ContainerSegment::MapKey(_),
            ContainerSegment::Field(lock_member),
        ] if accepted == "accepted_workspace"
            && audit == "member_audit"
            && lock_member == "lock_member"
    )
}

fn error(detail: impl Into<String>) -> UnknownFieldManifestError {
    UnknownFieldManifestError {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
