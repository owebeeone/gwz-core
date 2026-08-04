use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::MergeOperationRecordV1;

pub(crate) fn validate_v1_baseline(record: &MergeOperationRecordV1) -> ModelResult<()> {
    let manifest_yaml = record
        .baseline
        .manifest_yaml
        .as_deref()
        .ok_or_else(|| unreadable(record, "baseline manifest bytes are missing"))?;
    let lock_yaml = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| unreadable(record, "baseline lock bytes are missing"))?;
    if digest(manifest_yaml) != record.baseline.manifest_sha256
        || digest(lock_yaml) != record.baseline.lock_sha256
    {
        return Err(unreadable(
            record,
            "baseline bytes do not match their digests",
        ));
    }
    let manifest = ManifestArtifact::from_yaml(manifest_yaml)
        .map_err(|_| unreadable(record, "baseline manifest bytes are invalid"))?;
    let lock = LockArtifact::from_yaml(lock_yaml)
        .map_err(|_| unreadable(record, "baseline lock bytes are invalid"))?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(unreadable(record, "baseline workspace identity changed"));
    }

    let selected = record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
        .map(String::as_str)
        .collect::<Vec<_>>();
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let ordered = manifest
        .members
        .iter()
        .filter(|member| member.active && selected_set.contains(member.id.as_str()))
        .collect::<Vec<_>>();
    if ordered
        .iter()
        .map(|member| member.id.as_str())
        .collect::<Vec<_>>()
        != selected
    {
        return Err(unreadable(
            record,
            "selected members are not active baseline members in manifest order",
        ));
    }
    for member in ordered {
        let participant = &record.participants[&member.id];
        let lock_matches = lock.members.get(&member.id).is_none_or(|row| {
            row.path == member.path
                && row.source_id.as_deref() == Some(member.source_id.as_str())
                && row.source_kind == member.source_kind
        });
        if participant.path != member.path || !lock_matches {
            return Err(unreadable(
                record,
                "selected member baseline identity changed",
            ));
        }
    }

    if let Some(root) = record.participants.get("@root") {
        if record.baseline.root_head.as_deref() != Some(root.before_commit.as_str())
            || record.baseline.root_branch.as_deref() != Some(root.target_branch.as_str())
            || record.baseline.lock_commit_sha256.is_none()
            || record.baseline.manifest_commit_sha256.is_none()
        {
            return Err(unreadable(
                record,
                "selected root baseline checkout changed",
            ));
        }
    } else if record.baseline.root_head.is_none() && record.baseline.root_branch.is_none() {
        return Err(unreadable(
            record,
            "baseline root checkout is neither born nor unborn-attached",
        ));
    }
    Ok(())
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn unreadable(record: &MergeOperationRecordV1, reason: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!("merge record '{}' is unreadable: {reason}", record.merge_id),
    )
}
