use std::collections::BTreeSet;

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::MemberPath;

use super::super::super::{MergeTargetKind, PendingMergeAction};
use super::super::{
    AcceptedLockMemberV1, GitObjectAlgorithmV1, MERGE_RECORD_SCHEMA_V1,
    MERGE_RECORD_SCHEMA_VERSION_V1, MergeOperationRecordV1, PendingPreservationActionV1,
};

pub(crate) fn validate_common_v1_record(record: &MergeOperationRecordV1) -> ModelResult<()> {
    validate_common_record(record, true)
}

#[cfg(test)]
pub(crate) fn validate_common_v0_view(record: &MergeOperationRecordV1) -> ModelResult<()> {
    validate_common_record(record, false)
}

fn validate_common_record(
    record: &MergeOperationRecordV1,
    require_complete_baseline: bool,
) -> ModelResult<()> {
    if record.schema != MERGE_RECORD_SCHEMA_V1
        || record.record_schema_version != MERGE_RECORD_SCHEMA_VERSION_V1
    {
        return Err(unreadable(record, "the v1 envelope is inconsistent"));
    }
    validate_portable_id(record, "workspace_id", "ws_", &record.workspace_id)?;
    validate_portable_id(record, "operation_id", "op_", &record.operation_id)?;
    validate_slug(record, "merge_id", &record.merge_id)?;
    require_text(record, "writer_version", &record.writer_version)?;
    require_text(record, "source_ref", &record.source_ref)?;
    require_text(record, "created_at", &record.created_at)?;
    validate_sha256(record, "baseline.lock_sha256", &record.baseline.lock_sha256)?;
    validate_sha256(
        record,
        "baseline.manifest_sha256",
        &record.baseline.manifest_sha256,
    )?;
    validate_optional_sha256(
        record,
        "baseline.lock_commit_sha256",
        record.baseline.lock_commit_sha256.as_deref(),
    )?;
    validate_optional_sha256(
        record,
        "baseline.manifest_commit_sha256",
        record.baseline.manifest_commit_sha256.as_deref(),
    )?;
    if let Some(root_head) = record.baseline.root_head.as_deref() {
        validate_oid(record, "baseline.root_head", root_head)?;
    }
    if let Some(root_branch) = record.baseline.root_branch.as_deref() {
        validate_short_branch(record, "baseline.root_branch", root_branch)?;
    }
    let mut selected = BTreeSet::new();
    for target_id in &record.selected_targets {
        validate_target_id(record, "selected target", target_id)?;
        if !selected.insert(target_id.as_str()) {
            return Err(unreadable(
                record,
                format!("selected target '{target_id}' is duplicated"),
            ));
        }
        if !record.participants.contains_key(target_id) {
            return Err(unreadable(
                record,
                format!("selected target '{target_id}' has no participant"),
            ));
        }
    }
    if selected.is_empty() {
        return Err(unreadable(record, "selected target set is empty"));
    }
    if record
        .selected_targets
        .iter()
        .position(|target| target == "@root")
        .is_some_and(|position| position + 1 != record.selected_targets.len())
    {
        return Err(unreadable(record, "selected root is not the final target"));
    }
    if record.participants.len() != selected.len() {
        return Err(unreadable(
            record,
            "participant keys do not equal selected targets",
        ));
    }
    for (target_id, participant) in &record.participants {
        validate_target_id(record, "participant id", target_id)?;
        if !selected.contains(target_id.as_str()) {
            return Err(unreadable(
                record,
                format!("participant '{target_id}' is not selected"),
            ));
        }
        match participant.target_kind {
            MergeTargetKind::Root if target_id == "@root" && participant.path == "." => {}
            MergeTargetKind::Member if target_id != "@root" => {
                MemberPath::parse(&participant.path).map_err(|error| {
                    unreadable(
                        record,
                        format!(
                            "participant '{target_id}' path is invalid: {}",
                            error.message
                        ),
                    )
                })?;
            }
            _ => {
                return Err(unreadable(
                    record,
                    format!("participant '{target_id}' target identity is inconsistent"),
                ));
            }
        }
        validate_short_branch(
            record,
            &format!("participants.{target_id}.target_branch"),
            &participant.target_branch,
        )?;
        validate_oid(
            record,
            &format!("participants.{target_id}.before_commit"),
            &participant.before_commit,
        )?;
        validate_oid(
            record,
            &format!("participants.{target_id}.source_commit"),
            &participant.source_commit,
        )?;
        validate_commit_message(record, &participant.commit_message)?;
        for (field, value) in [
            ("resulting_commit", participant.resulting_commit.as_deref()),
            (
                "expected_merge_head",
                participant.expected_merge_head.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                validate_oid(record, &format!("participants.{target_id}.{field}"), value)?;
            }
        }
        for evidence in &participant.conflict_snapshot {
            require_text(record, "conflict path", &evidence.path)?;
            validate_sha256(record, "conflict evidence sha256", &evidence.sha256)?;
        }
        if let Some(pending) = participant.pending_action.as_ref() {
            validate_pending(record, target_id, pending)?;
        }
        validate_preservation_evidence(record, &participant.preservation)?;
    }
    if require_complete_baseline {
        super::validate_v1_baseline(record)?;
    }
    if let Some(accepted) = record.accepted_workspace.as_ref() {
        validate_sha256(
            record,
            "accepted_workspace.operation_baseline_lock_sha256",
            &accepted.operation_baseline_lock_sha256,
        )?;
        validate_sha256(
            record,
            "accepted_workspace.metadata_base.manifest_sha256",
            &accepted.metadata_base.manifest_sha256,
        )?;
        validate_sha256(
            record,
            "accepted_workspace.metadata_base.lock_sha256",
            &accepted.metadata_base.lock_sha256,
        )?;
        validate_sha256(
            record,
            "accepted_workspace.lock.sha256",
            &accepted.lock.sha256,
        )?;
        for (member_id, audit) in &accepted.member_audit {
            validate_portable_id(record, "accepted member id", "mem_", member_id)?;
            match audit {
                super::super::MemberAcceptanceV1::Selected {
                    integration,
                    final_checkout,
                    lock_member,
                } => {
                    validate_short_branch(
                        record,
                        "accepted integration branch",
                        &integration.branch,
                    )?;
                    validate_oid(record, "accepted before commit", &integration.before_commit)?;
                    validate_oid(
                        record,
                        "accepted resulting commit",
                        &integration.resulting_commit,
                    )?;
                    validate_short_branch(
                        record,
                        "accepted checkout branch",
                        &final_checkout.branch,
                    )?;
                    validate_oid(record, "accepted checkout commit", &final_checkout.commit)?;
                    validate_lock_member(record, lock_member)?;
                }
                super::super::MemberAcceptanceV1::UnselectedPresent { lock_member } => {
                    validate_lock_member(record, lock_member)?;
                }
                super::super::MemberAcceptanceV1::Absent => {}
            }
        }
    }
    if let Some(publication) = record.publication.as_ref() {
        validate_preservation_evidence(record, &publication.root_preservation)?;
    }
    validate_preservation_object_ids(record)
}

fn validate_preservation_evidence(
    record: &MergeOperationRecordV1,
    evidence: &[super::super::super::PreservationEvidence],
) -> ModelResult<()> {
    for row in evidence {
        if let Some(name) = row.backup_ref.as_deref() {
            require_text(record, "preservation backup ref", name)?;
        }
        if let Some(commit) = row.backup_commit.as_deref() {
            validate_oid(record, "preservation backup commit", commit)?;
        }
        if let Some(stash_id) = row.stash_id.as_deref() {
            require_text(record, "preservation stash id", stash_id)?;
        }
        if let Some(object_id) = row.stash_object_id.as_deref() {
            validate_oid(record, "preservation stash object id", object_id)?;
        }
    }
    Ok(())
}

fn validate_pending(
    record: &MergeOperationRecordV1,
    target_id: &str,
    pending: &PendingMergeAction,
) -> ModelResult<()> {
    validate_short_branch(record, "pending target branch", &pending.target_branch)?;
    validate_oid(record, "pending before commit", &pending.before_commit)?;
    validate_oid(record, "pending source commit", &pending.source_commit)?;
    if let Some(spec) = pending.commit_spec.as_ref() {
        validate_oid(record, "pending tree oid", &spec.tree_oid)?;
    }
    require_text(
        record,
        &format!("participants.{target_id}.pending_action.commit_message"),
        &pending.commit_message,
    )
}

fn validate_commit_message(record: &MergeOperationRecordV1, message: &str) -> ModelResult<()> {
    let trailer = format!(
        "\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        record.merge_id, record.operation_id
    );
    let body = message.strip_suffix(&trailer).unwrap_or_default();
    if !body.trim().is_empty() && !body.contains(['\0', '\r']) && !body.ends_with('\n') {
        Ok(())
    } else {
        Err(unreadable(
            record,
            "participant commit message is not canonical",
        ))
    }
}

fn validate_lock_member(
    record: &MergeOperationRecordV1,
    member: &AcceptedLockMemberV1,
) -> ModelResult<()> {
    MemberPath::parse(&member.path).map_err(|error| {
        unreadable(
            record,
            format!("accepted lock member path is invalid: {}", error.message),
        )
    })?;
    validate_portable_id(
        record,
        "accepted lock member source_id",
        "src_",
        &member.source_id,
    )?;
    if let Some(commit) = member.commit.as_deref() {
        validate_oid(record, "accepted lock member commit", commit)?;
    }
    if let Some(branch) = member.branch.as_deref() {
        validate_short_branch(record, "accepted lock member branch", branch)?;
    }
    Ok(())
}

fn validate_preservation_object_ids(record: &MergeOperationRecordV1) -> ModelResult<()> {
    let Some(PendingPreservationActionV1::Stash {
        stash_object_id: Some(object_id),
        ..
    }) = record.pending_preservation.as_ref()
    else {
        return Ok(());
    };
    let expected = match object_id.algorithm {
        GitObjectAlgorithmV1::Sha1 => 40,
        GitObjectAlgorithmV1::Sha256 => 64,
    };
    if object_id.digest_hex.len() == expected && is_lower_hex(&object_id.digest_hex) {
        Ok(())
    } else {
        Err(unreadable(
            record,
            "pending preservation object id is inconsistent with its algorithm",
        ))
    }
}

fn validate_target_id(
    record: &MergeOperationRecordV1,
    field: &str,
    value: &str,
) -> ModelResult<()> {
    if value == "@root" {
        Ok(())
    } else {
        validate_portable_id(record, field, "mem_", value)
    }
}

fn validate_portable_id(
    record: &MergeOperationRecordV1,
    field: &str,
    prefix: &str,
    value: &str,
) -> ModelResult<()> {
    if value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(unreadable(
            record,
            format!("{field} is not a portable {prefix} identifier"),
        ))
    }
}

fn validate_slug(record: &MergeOperationRecordV1, field: &str, value: &str) -> ModelResult<()> {
    if !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(unreadable(
            record,
            format!("{field} is not a portable slug"),
        ))
    }
}

fn require_text(record: &MergeOperationRecordV1, field: &str, value: &str) -> ModelResult<()> {
    if value.trim().is_empty() {
        Err(unreadable(record, format!("{field} is empty")))
    } else {
        Ok(())
    }
}

fn validate_sha256(record: &MergeOperationRecordV1, field: &str, value: &str) -> ModelResult<()> {
    if value.len() == 64 && is_lower_hex(value) {
        Ok(())
    } else {
        Err(unreadable(
            record,
            format!("{field} is not a lowercase SHA-256"),
        ))
    }
}

fn validate_optional_sha256(
    record: &MergeOperationRecordV1,
    field: &str,
    value: Option<&str>,
) -> ModelResult<()> {
    value.map_or(Ok(()), |value| validate_sha256(record, field, value))
}

fn validate_oid(record: &MergeOperationRecordV1, field: &str, value: &str) -> ModelResult<()> {
    if matches!(value.len(), 40 | 64) && is_lower_hex(value) {
        Ok(())
    } else {
        Err(unreadable(
            record,
            format!("{field} is not a lowercase Git object id"),
        ))
    }
}

fn validate_short_branch(
    record: &MergeOperationRecordV1,
    field: &str,
    branch: &str,
) -> ModelResult<()> {
    let invalid_byte = branch.bytes().any(|byte| {
        byte <= b' '
            || byte == 0x7f
            || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
    });
    let invalid_component = branch
        .split('/')
        .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"));
    if branch.is_empty()
        || branch.starts_with("refs/")
        || branch.starts_with('-')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || invalid_byte
        || invalid_component
    {
        Err(unreadable(
            record,
            format!("{field} is not a canonical short local branch"),
        ))
    } else {
        Ok(())
    }
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn unreadable(record: &MergeOperationRecordV1, reason: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!(
            "merge record '{}' is unreadable: {}",
            record.merge_id,
            reason.into()
        ),
    )
}
