use serde::Serialize;
use serde_yaml::Value;

use super::super::authority::ParticipantDriftIdentity;
use super::super::checked::StoredV1Record;
use super::super::transition::{RetiredContainer, TransitionEffect};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::record_wire::{
    ContainerSegment, IdentityValue, SemanticIdentity, UnknownFieldLocator, UnknownFieldManifest,
};

pub(super) fn overlay(
    current: &StoredV1Record,
    effect: &TransitionEffect,
    next: &mut Value,
) -> ModelResult<UnknownFieldManifest> {
    let authorized = effect.retired()?;
    let mut authorized_source = current.unknown_fields().clone();
    for retirement in &authorized {
        if let RetiredContainer::ParticipantDrift {
            member_id,
            identity,
        } = retirement
        {
            authorized_source = authorized_source
                .after_participant_drift_retirement(member_id, &semantic_drift_identity(identity)?)
                .map_err(|error| rejected(error.detail))?;
        }
    }
    if effect.allows_derived_acceptance_unknowns()? {
        let replacement =
            UnknownFieldManifest::extract_v1(next).map_err(|error| rejected(error.detail))?;
        authorized_source
            .authorize_derived_accepted_lock_members(&replacement)
            .map_err(|error| rejected(error.detail))?;
    }
    let surviving = authorized_source
        .apply_surviving(next)
        .map_err(|error| rejected(error.detail))?;
    for (locator, value) in current.unknown_fields().entries() {
        if !surviving.entries().contains_key(locator)
            && !authorized
                .iter()
                .any(|retirement| matches_retirement(locator, retirement))
            && !rebased_survivor(locator, value, &authorized, &surviving)?
        {
            return Err(rejected(format!(
                "transition retired unauthorized unknown container for field '{}'",
                locator.field
            )));
        }
    }
    Ok(surviving)
}

fn matches_retirement(locator: &UnknownFieldLocator, retirement: &RetiredContainer) -> bool {
    use ContainerSegment::Field;
    let path = locator.container.as_slice();
    match retirement {
        RetiredContainer::RecoveryContext => starts_with_field(path, "recovery_context"),
        RetiredContainer::PendingRollback => starts_with_field(path, "pending_rollback"),
        RetiredContainer::PendingPreservation => starts_with_field(path, "pending_preservation"),
        RetiredContainer::ParticipantPendingAction(member_id) => {
            participant_field(path, member_id, "pending_action").is_some()
        }
        RetiredContainer::ParticipantConflictEvidence(member_id) => {
            participant_field(path, member_id, "conflict_snapshot").is_some()
        }
        RetiredContainer::ParticipantError(member_id) => {
            participant_field(path, member_id, "error").is_some()
        }
        RetiredContainer::ParticipantDrift {
            member_id,
            identity,
        } => participant_field(path, member_id, "drift")
            .and_then(first_identity)
            .is_some_and(|actual| {
                semantic_drift_identity(identity).is_ok_and(|expected| actual == &expected)
            }),
        RetiredContainer::OperationDrift(kind) => matches!(
            path,
            [Field(field), ContainerSegment::Identity(identity), ..]
                if field == "operation_drift"
                    && identity.kind == "operation_drift"
                    && identity_string(identity, "kind") == serialized_name(kind).as_deref()
        ),
    }
}

fn rebased_survivor(
    locator: &UnknownFieldLocator,
    value: &Value,
    retirements: &[RetiredContainer],
    surviving: &UnknownFieldManifest,
) -> ModelResult<bool> {
    let mut rebased = locator.clone();
    let mut changed = false;
    for retirement in retirements {
        let RetiredContainer::ParticipantDrift {
            member_id,
            identity,
        } = retirement
        else {
            continue;
        };
        let expected = semantic_drift_identity(identity)?;
        let Some(actual) = participant_drift_identity_mut(&mut rebased, member_id) else {
            continue;
        };
        if actual.kind == expected.kind
            && actual.fields == expected.fields
            && actual.occurrence > expected.occurrence
        {
            actual.occurrence -= 1;
            changed = true;
        }
    }
    Ok(changed && surviving.entries().get(&rebased) == Some(value))
}

fn participant_drift_identity_mut<'a>(
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

fn semantic_drift_identity(identity: &ParticipantDriftIdentity) -> ModelResult<SemanticIdentity> {
    Ok(SemanticIdentity {
        kind: "participant_drift".into(),
        fields: vec![
            (
                "kind".into(),
                IdentityValue::String(
                    serialized_name(&identity.kind).ok_or_else(|| {
                        rejected("participant drift kind could not be serialized")
                    })?,
                ),
            ),
            (
                "expected_branch".into(),
                optional(&identity.expected_branch),
            ),
            ("live_branch".into(), optional(&identity.live_branch)),
            ("expected_head".into(), optional(&identity.expected_head)),
            ("live_head".into(), optional(&identity.live_head)),
            (
                "expected_merge_head".into(),
                optional(&identity.expected_merge_head),
            ),
            (
                "live_merge_head".into(),
                optional(&identity.live_merge_head),
            ),
        ],
        occurrence: identity.occurrence,
    })
}

fn optional(value: &Option<String>) -> IdentityValue {
    value.as_ref().map_or(IdentityValue::Null, |value| {
        IdentityValue::String(value.clone())
    })
}

fn starts_with_field(path: &[ContainerSegment], field: &str) -> bool {
    matches!(path, [ContainerSegment::Field(actual), ..] if actual == field)
}

fn participant_field<'a>(
    path: &'a [ContainerSegment],
    member_id: &str,
    field: &str,
) -> Option<&'a [ContainerSegment]> {
    match path {
        [
            ContainerSegment::Field(participants),
            ContainerSegment::MapKey(actual_member),
            rest @ ..,
        ] if participants == "participants" && actual_member == member_id => rest
            .iter()
            .position(
                |segment| matches!(segment, ContainerSegment::Field(actual) if actual == field),
            )
            .map(|index| &rest[index + 1..]),
        _ => None,
    }
}

fn first_identity(path: &[ContainerSegment]) -> Option<&SemanticIdentity> {
    path.iter().find_map(|segment| match segment {
        ContainerSegment::Identity(identity) => Some(identity),
        _ => None,
    })
}

fn identity_string<'a>(identity: &'a SemanticIdentity, name: &str) -> Option<&'a str> {
    identity
        .fields
        .iter()
        .find_map(|(field, value)| match (field.as_str(), value) {
            (actual, IdentityValue::String(value)) if actual == name => Some(value.as_str()),
            _ => None,
        })
}

fn serialized_name(value: &impl Serialize) -> Option<String> {
    serde_yaml::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn rejected(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail)
}
