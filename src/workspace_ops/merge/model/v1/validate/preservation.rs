use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::PreservationEvidence;
use super::super::{
    GitObjectIdV1, MergeOperationRecordV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1, PublicationPrefixV1,
};

pub(crate) fn validate_v1_preservation(record: &MergeOperationRecordV1) -> ModelResult<()> {
    validate_all_evidence(record)?;
    if let Some(action) = record.pending_preservation.as_ref() {
        validate_action(record, action)?;
    }
    Ok(())
}

fn validate_all_evidence(record: &MergeOperationRecordV1) -> ModelResult<()> {
    for (member_id, participant) in &record.participants {
        let owner = PreservationOwnerV1::Participant {
            member_id: member_id.clone(),
        };
        validate_owner_evidence(record, &owner, &participant.preservation)?;
    }
    let publication_evidence = record
        .publication
        .as_ref()
        .map(|publication| publication.root_preservation.as_slice())
        .unwrap_or_default();
    validate_owner_evidence(
        record,
        &PreservationOwnerV1::PublicationRoot,
        publication_evidence,
    )?;
    let selected_root_evidence = record
        .participants
        .get("@root")
        .is_some_and(|participant| !participant.preservation.is_empty());
    if selected_root_evidence && !publication_evidence.is_empty() {
        return Err(preservation_error(record));
    }
    Ok(())
}

fn validate_owner_evidence(
    record: &MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
    rows: &[PreservationEvidence],
) -> ModelResult<()> {
    if rows.len() > 1 {
        return Err(preservation_error(record));
    }
    let Some(evidence) = rows.first() else {
        return Ok(());
    };
    if !owner_is_valid(record, owner) {
        return Err(preservation_error(record));
    }
    let backup_pair = match (
        evidence.backup_ref.as_deref(),
        evidence.backup_commit.as_deref(),
    ) {
        (None, None) => false,
        (Some(name), Some(commit))
            if name == canonical_ref(record, owner)
                && is_oid(commit)
                && owner_anchor(record, owner).is_some() =>
        {
            true
        }
        _ => return Err(preservation_exactness_error(record)),
    };
    let stash_pair = match (
        evidence.stash_id.as_deref(),
        evidence.stash_object_id.as_deref(),
    ) {
        (None, None) => false,
        (Some(stash_id), Some(object_id))
            if stash_id == format!("stash_{}", record.merge_id) && is_oid(object_id) =>
        {
            true
        }
        _ => return Err(preservation_exactness_error(record)),
    };
    if !backup_pair && !stash_pair {
        return Err(preservation_exactness_error(record));
    }
    Ok(())
}

fn validate_action(
    record: &MergeOperationRecordV1,
    action: &PendingPreservationActionV1,
) -> ModelResult<()> {
    match action {
        PendingPreservationActionV1::BackupRef {
            owner,
            name,
            target_commit,
        } => {
            if !owner_is_valid(record, owner)
                || name != &canonical_ref(record, owner)
                || !is_oid(target_commit)
                || owner_anchor(record, owner).is_none()
                || recorded_backup_target(record, owner).is_some()
            {
                return Err(preservation_error(record));
            }
        }
        PendingPreservationActionV1::Stash {
            owner,
            phase,
            stash_id,
            stash_object_id,
            message,
            head_commit,
            preimage_sha256,
            root_publication_prefix,
        } => validate_stash(
            record,
            owner,
            *phase,
            stash_id.as_deref(),
            stash_object_id.as_ref(),
            message,
            head_commit,
            preimage_sha256,
            root_publication_prefix.as_ref(),
        )?,
        PendingPreservationActionV1::ResetAttachedRef {
            owner,
            branch,
            expected_commit,
            restore_commit,
            phase,
            root_publication_prefix,
        } => {
            let prefix_present = root_publication_prefix.is_some();
            let phase_valid = match phase {
                PreservationRefResetPhaseV1::ResetRef | PreservationRefResetPhaseV1::Complete => {
                    true
                }
                PreservationRefResetPhaseV1::RestoreRoot => prefix_present,
            };
            if !owner_is_valid(record, owner)
                || owner_branch(record, owner) != Some(branch.as_str())
                || recorded_backup_target(record, owner) != Some(expected_commit.as_str())
                || owner_anchor(record, owner) != Some(restore_commit.as_str())
                || (prefix_present && !owner_is_root(owner))
                || !prefix_matches_record(record, root_publication_prefix.as_ref())
                || !phase_valid
            {
                return Err(preservation_exactness_error(record));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_stash(
    record: &MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
    phase: PreservationStashPhaseV1,
    stash_id: Option<&str>,
    stash_object_id: Option<&GitObjectIdV1>,
    message: &str,
    head_commit: &str,
    preimage_sha256: &str,
    prefix: Option<&PublicationPrefixV1>,
) -> ModelResult<()> {
    let exact_id = format!("stash_{}", record.merge_id);
    let ids_absent = stash_id.is_none() && stash_object_id.is_none();
    let ids_present = stash_id == Some(exact_id.as_str()) && stash_object_id.is_some();
    let recorded = owner_evidence(record, owner);
    let recorded_ids_absent = recorded
        .is_none_or(|evidence| evidence.stash_id.is_none() && evidence.stash_object_id.is_none());
    let recorded_ids_match = recorded.is_some_and(|evidence| {
        evidence.stash_id.as_deref() == stash_id
            && evidence.stash_object_id.as_deref()
                == stash_object_id.map(|object_id| object_id.digest_hex.as_str())
    });
    let prefix_present = prefix.is_some();
    let phase_valid = match phase {
        PreservationStashPhaseV1::NormalizeRoot => {
            prefix_present && ids_absent && recorded_ids_absent
        }
        PreservationStashPhaseV1::CreateStash => ids_absent && recorded_ids_absent,
        PreservationStashPhaseV1::RestoreRoot => {
            prefix_present && ids_present && recorded_ids_match
        }
        PreservationStashPhaseV1::WriteBundle | PreservationStashPhaseV1::Complete => {
            ids_present && recorded_ids_match
        }
    };
    if !owner_is_valid(record, owner)
        || recorded_backup_target(record, owner).or_else(|| owner_anchor(record, owner))
            != Some(head_commit)
        || message != format!("gwz:{exact_id}: merge preservation")
        || !is_oid(head_commit)
        || !is_sha256(preimage_sha256)
        || (prefix_present && !owner_is_root(owner))
        || !prefix_matches_record(record, prefix)
        || !phase_valid
    {
        return Err(preservation_exactness_error(record));
    }
    Ok(())
}

fn owner_evidence<'a>(
    record: &'a MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
) -> Option<&'a PreservationEvidence> {
    let rows = match owner {
        PreservationOwnerV1::Participant { member_id } => record
            .participants
            .get(member_id)
            .map(|participant| participant.preservation.as_slice())?,
        PreservationOwnerV1::PublicationRoot => record
            .publication
            .as_ref()
            .map(|publication| publication.root_preservation.as_slice())?,
    };
    (rows.len() == 1).then(|| &rows[0])
}

fn prefix_matches_record(
    record: &MergeOperationRecordV1,
    prefix: Option<&PublicationPrefixV1>,
) -> bool {
    match prefix {
        None => true,
        Some(prefix) => {
            record
                .publication
                .as_ref()
                .and_then(|publication| publication.preservation_prefix.as_deref())
                == Some(prefix_name(*prefix))
        }
    }
}

fn prefix_name(prefix: PublicationPrefixV1) -> &'static str {
    match prefix {
        PublicationPrefixV1::Baseline => "baseline",
        PublicationPrefixV1::Marker => "marker",
        PublicationPrefixV1::Lock => "lock",
        PublicationPrefixV1::Boundary => "boundary",
    }
}

fn owner_is_valid(record: &MergeOperationRecordV1, owner: &PreservationOwnerV1) -> bool {
    owner_anchor(record, owner).is_some()
        && match owner {
            PreservationOwnerV1::Participant { member_id } => {
                record.participants.contains_key(member_id)
            }
            PreservationOwnerV1::PublicationRoot => {
                !record.participants.contains_key("@root")
                    && record
                        .publication
                        .as_ref()
                        .and_then(|publication| publication.candidate.as_ref())
                        .is_some()
            }
        }
}

fn owner_is_root(owner: &PreservationOwnerV1) -> bool {
    matches!(
        owner,
        PreservationOwnerV1::Participant { member_id } if member_id == "@root"
    ) || *owner == PreservationOwnerV1::PublicationRoot
}

fn owner_branch<'a>(
    record: &'a MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
) -> Option<&'a str> {
    match owner {
        PreservationOwnerV1::Participant { member_id } => record
            .participants
            .get(member_id)
            .map(|participant| participant.target_branch.as_str()),
        PreservationOwnerV1::PublicationRoot => record
            .publication
            .as_ref()
            .and_then(|publication| publication.candidate.as_ref())
            .map(|candidate| candidate.root_branch.as_str()),
    }
}

fn owner_anchor<'a>(
    record: &'a MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
) -> Option<&'a str> {
    match owner {
        PreservationOwnerV1::Participant { member_id } if member_id == "@root" => record
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_deref())
            .or_else(|| {
                record
                    .participants
                    .get(member_id)
                    .and_then(|participant| participant.resulting_commit.as_deref())
            }),
        PreservationOwnerV1::Participant { member_id } => record
            .participants
            .get(member_id)
            .and_then(|participant| participant.resulting_commit.as_deref()),
        PreservationOwnerV1::PublicationRoot => record
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_deref()),
    }
}

fn recorded_backup_target<'a>(
    record: &'a MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
) -> Option<&'a str> {
    let row = owner_evidence(record, owner)?;
    (row.backup_ref.as_deref() == Some(&canonical_ref(record, owner)))
        .then_some(row.backup_commit.as_deref())
        .flatten()
}

fn canonical_ref(record: &MergeOperationRecordV1, owner: &PreservationOwnerV1) -> String {
    format!(
        "refs/gwz/merge/{}/{}/head",
        record.merge_id,
        owner_key(owner)
    )
}

fn owner_key(owner: &PreservationOwnerV1) -> &str {
    match owner {
        PreservationOwnerV1::Participant { member_id } if member_id == "@root" => "root",
        PreservationOwnerV1::Participant { member_id } => member_id,
        PreservationOwnerV1::PublicationRoot => "root",
    }
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && is_lower_hex(value)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && is_lower_hex(value)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn preservation_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        "preservation evidence has no unique owner or action step",
    )
}

fn preservation_exactness_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        "preservation ref, stash, root prefix, bundle, or branch result is not exact",
    )
}

fn typed_error(record: &MergeOperationRecordV1, reason: &str) -> ModelError {
    ModelError::new(
        ErrorCode::PreservationEvidenceMismatch,
        format!(
            "merge record '{}' preservation evidence is invalid: {reason}",
            record.merge_id
        ),
    )
}
