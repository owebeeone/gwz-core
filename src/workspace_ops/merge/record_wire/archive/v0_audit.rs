use std::collections::{BTreeMap, BTreeSet};

use super::super::super::model::archive_projection::*;
use super::super::super::{MergeOperationRecord, MergeParticipantRecord};
use crate::artifact::{LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact};

pub(super) fn complete_audit(
    record: &MergeOperationRecord,
    manifest: Option<&ManifestArtifact>,
    lock: &LockArtifact,
    metadata_base_lock: Option<&LockArtifact>,
) -> Result<Option<Vec<AcceptedMemberV1Projection>>, ()> {
    let selected = selected_members(record);
    let selected_rows_complete =
        validate_selected_rows(record, lock, metadata_base_lock, &selected)?;
    validate_unselected_preserved(lock, metadata_base_lock, &selected)?;
    let Some(manifest) = manifest else {
        return Ok(None);
    };
    if record.participants.contains_key("@root") {
        return Ok(None);
    }
    validate_selected_order(record, manifest, &selected)?;
    let active = active_members(manifest);
    let mut domain = active
        .keys()
        .map(|id| (*id).to_owned())
        .collect::<BTreeSet<_>>();
    domain.extend(lock.members.keys().cloned());
    domain.extend(selected.iter().cloned());
    if let Some(base) = metadata_base_lock {
        domain.extend(base.members.keys().cloned());
    }

    let mut complete = selected_rows_complete;
    let mut projected = Vec::with_capacity(domain.len());
    for member_id in domain {
        let row = lock.members.get(&member_id);
        if selected.contains(&member_id) {
            let participant = record.participants.get(&member_id).ok_or(())?;
            let manifest_member = active.get(member_id.as_str()).ok_or(())?;
            complete &= validate_manifest_identity(row, manifest_member)?;
            if let Some(base) = metadata_base_lock {
                complete &=
                    validate_manifest_identity(base.members.get(&member_id), manifest_member)?;
            }
            let (Some(result), Some(lock_member)) = (
                participant.resulting_commit.as_deref(),
                row.and_then(project_lock_member),
            ) else {
                complete = false;
                continue;
            };
            projected.push(AcceptedMemberV1Projection {
                member_id,
                kind: AcceptedMemberKind::Selected,
                integration: Some(integration(participant, result)),
                final_checkout: Some(AcceptedCheckoutProjection {
                    branch: participant.target_branch.clone(),
                    commit: result.to_owned(),
                }),
                lock_member: Some(lock_member),
            });
        } else {
            if let (Some(row), Some(member)) = (row, active.get(member_id.as_str())) {
                complete &= validate_manifest_identity(Some(row), member)?;
            }
            match row.and_then(project_lock_member) {
                Some(lock_member) => projected.push(AcceptedMemberV1Projection {
                    member_id,
                    kind: AcceptedMemberKind::UnselectedPresent,
                    integration: None,
                    final_checkout: None,
                    lock_member: Some(lock_member),
                }),
                None if row.is_some() => complete = false,
                None => projected.push(AcceptedMemberV1Projection {
                    member_id,
                    kind: AcceptedMemberKind::Absent,
                    integration: None,
                    final_checkout: None,
                    lock_member: None,
                }),
            }
        }
    }
    Ok(complete.then_some(projected))
}

pub(super) fn legacy_members(
    record: &MergeOperationRecord,
    lock: Option<&LockArtifact>,
) -> Result<Vec<LegacyMemberEvidence>, ()> {
    let mut domain = record
        .participants
        .keys()
        .filter(|member_id| member_id.as_str() != "@root")
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(lock) = lock {
        domain.extend(lock.members.keys().cloned());
    }
    Ok(domain
        .into_iter()
        .map(|member_id| {
            let participant = record.participants.get(&member_id);
            let integration = participant.and_then(|participant| {
                participant
                    .resulting_commit
                    .as_deref()
                    .map(|result| integration(participant, result))
            });
            LegacyMemberEvidence {
                selected: participant.is_some(),
                state: participant.map(|participant| participant.state),
                integration,
                lock_member: lock
                    .and_then(|lock| lock.members.get(&member_id))
                    .and_then(project_lock_member),
                member_id,
            }
        })
        .collect())
}

fn validate_selected_rows(
    record: &MergeOperationRecord,
    lock: &LockArtifact,
    metadata_base_lock: Option<&LockArtifact>,
    selected: &BTreeSet<String>,
) -> Result<bool, ()> {
    let mut complete = true;
    for member_id in selected {
        let participant = record.participants.get(member_id).ok_or(())?;
        let Some(row) = lock.members.get(member_id) else {
            complete = false;
            continue;
        };
        complete &=
            validate_checkout_row(row, participant, participant.resulting_commit.as_deref())?;
        if let Some(base) = metadata_base_lock {
            let Some(base_row) = base.members.get(member_id) else {
                complete = false;
                continue;
            };
            complete &= validate_checkout_row(
                base_row,
                participant,
                Some(participant.before_commit.as_str()),
            )?;
        }
    }
    Ok(complete)
}

fn validate_checkout_row(
    row: &ResolvedMemberArtifact,
    participant: &MergeParticipantRecord,
    expected_commit: Option<&str>,
) -> Result<bool, ()> {
    if row.path != participant.path {
        return Err(());
    }
    let mut complete = validate_optional_equal(row.commit.as_deref(), expected_commit)?;
    complete &= validate_optional_equal(
        row.branch.as_deref(),
        Some(participant.target_branch.as_str()),
    )?;
    complete &= validate_optional_equal(row.detached, Some(false))?;
    complete &= validate_optional_equal(row.dirty, Some(false))?;
    complete &= validate_optional_equal(row.materialized, Some(true))?;
    Ok(complete)
}

fn validate_unselected_preserved(
    lock: &LockArtifact,
    metadata_base_lock: Option<&LockArtifact>,
    selected: &BTreeSet<String>,
) -> Result<(), ()> {
    let Some(base) = metadata_base_lock else {
        return Ok(());
    };
    let mut domain = lock.members.keys().cloned().collect::<BTreeSet<_>>();
    domain.extend(base.members.keys().cloned());
    for member_id in domain {
        if !selected.contains(&member_id)
            && lock.members.get(&member_id) != base.members.get(&member_id)
        {
            return Err(());
        }
    }
    Ok(())
}

fn validate_manifest_identity(
    row: Option<&ResolvedMemberArtifact>,
    member: &ManifestMember,
) -> Result<bool, ()> {
    let Some(row) = row else {
        return Ok(true);
    };
    if row.path != member.path || row.source_kind != member.source_kind {
        return Err(());
    }
    validate_optional_equal(row.source_id.as_deref(), Some(member.source_id.as_str()))
}

fn validate_optional_equal<T: Eq>(actual: Option<T>, expected: Option<T>) -> Result<bool, ()> {
    match (actual, expected) {
        (Some(actual), Some(expected)) if actual == expected => Ok(true),
        (None, _) | (_, None) => Ok(false),
        _ => Err(()),
    }
}

fn validate_selected_order(
    record: &MergeOperationRecord,
    manifest: &ManifestArtifact,
    selected: &BTreeSet<String>,
) -> Result<(), ()> {
    let ordered_selected = manifest
        .members
        .iter()
        .filter(|member| member.active && selected.contains(&member.id))
        .map(|member| &member.id)
        .collect::<Vec<_>>();
    (ordered_selected
        == record
            .selected_targets
            .iter()
            .filter(|target| target.as_str() != "@root")
            .collect::<Vec<_>>())
    .then_some(())
    .ok_or(())
}

fn active_members(manifest: &ManifestArtifact) -> BTreeMap<&str, &ManifestMember> {
    manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| (member.id.as_str(), member))
        .collect()
}

fn selected_members(record: &MergeOperationRecord) -> BTreeSet<String> {
    record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
        .cloned()
        .collect()
}

fn project_lock_member(row: &ResolvedMemberArtifact) -> Option<AcceptedLockMemberProjection> {
    Some(AcceptedLockMemberProjection {
        path: row.path.clone(),
        source_id: row.source_id.clone()?,
        source_kind: row.source_kind,
        commit: row.commit.clone(),
        branch: row.branch.clone(),
        detached: row.detached,
        upstream: row.upstream.clone(),
        dirty: row.dirty,
        materialized: row.materialized,
    })
}

fn integration(
    participant: &MergeParticipantRecord,
    result: &str,
) -> AcceptedIntegrationProjection {
    AcceptedIntegrationProjection {
        branch: participant.target_branch.clone(),
        before_commit: participant.before_commit.clone(),
        resulting_commit: result.to_owned(),
    }
}
