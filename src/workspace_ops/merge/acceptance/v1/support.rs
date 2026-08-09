use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_yaml::Value;
use sha2::{Digest, Sha256};

use crate::artifact::{ArtifactSourceKind, LockArtifact, ManifestArtifact, ResolvedMemberArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::MergeParticipantRecord;
use crate::workspace_ops::merge::model::v1::AcceptedLockMemberV1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemberIdentity {
    path: String,
    source_id: String,
    source_kind: ArtifactSourceKind,
}

impl MemberIdentity {
    pub(super) fn to_lock_member(&self) -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: self.path.clone(),
            source_id: Some(self.source_id.clone()),
            source_kind: self.source_kind,
            commit: None,
            branch: None,
            detached: None,
            upstream: None,
            dirty: None,
            materialized: None,
        }
    }
}

pub(super) fn selected_identity(
    member_id: &str,
    participant: &MergeParticipantRecord,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    baseline_manifest: &ManifestArtifact,
    baseline_lock: &LockArtifact,
    merge_id: &str,
) -> ModelResult<MemberIdentity> {
    let baseline = manifest_identity(baseline_manifest, member_id)
        .or_else(|| lock_identity(baseline_lock, member_id))
        .ok_or_else(|| input_error(merge_id, "selected member has no frozen identity"))?;
    let identities = [
        Some(baseline.clone()),
        manifest_identity(manifest, member_id),
        lock_identity(lock, member_id),
        manifest_identity(baseline_manifest, member_id),
        lock_identity(baseline_lock, member_id),
    ];
    if identities
        .iter()
        .flatten()
        .any(|identity| identity != &baseline || identity.path != participant.path)
    {
        return Err(input_error(
            merge_id,
            "selected member identity changed before acceptance",
        ));
    }
    Ok(baseline)
}

fn manifest_identity(manifest: &ManifestArtifact, member_id: &str) -> Option<MemberIdentity> {
    manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .map(|member| MemberIdentity {
            path: member.path.clone(),
            source_id: member.source_id.clone(),
            source_kind: member.source_kind,
        })
}

fn lock_identity(lock: &LockArtifact, member_id: &str) -> Option<MemberIdentity> {
    let member = lock.members.get(member_id)?;
    Some(MemberIdentity {
        path: member.path.clone(),
        source_id: member.source_id.clone()?,
        source_kind: member.source_kind,
    })
}

#[derive(Deserialize)]
struct AcceptedLockRows {
    members: BTreeMap<String, AcceptedLockMemberV1>,
}

pub(super) fn parse_lock_rows(
    merge_id: &str,
    yaml: &str,
) -> ModelResult<BTreeMap<String, AcceptedLockMemberV1>> {
    serde_yaml::from_str::<AcceptedLockRows>(yaml)
        .map(|lock| lock.members)
        .map_err(|_| input_error(merge_id, "accepted lock rows are invalid"))
}

pub(super) fn parse_manifest(merge_id: &str, yaml: &str) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(yaml)
        .map_err(|_| input_error(merge_id, "accepted manifest bytes are invalid"))
}

pub(super) fn parse_lock(merge_id: &str, yaml: &str) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(yaml)
        .map_err(|_| input_error(merge_id, "accepted lock bytes are invalid"))
}

pub(super) fn render_complete_lock(
    merge_id: &str,
    metadata_yaml: &str,
    baseline_yaml: &str,
    complete: &LockArtifact,
    selected: &BTreeSet<String>,
) -> ModelResult<String> {
    let mut raw: Value = serde_yaml::from_str(metadata_yaml)
        .map_err(|_| input_error(merge_id, "accepted lock YAML is invalid"))?;
    let baseline: Value = serde_yaml::from_str(baseline_yaml)
        .map_err(|_| input_error(merge_id, "baseline lock YAML is invalid"))?;
    for member_id in selected {
        let typed = complete.members.get(member_id).ok_or_else(|| {
            input_error(merge_id, "selected member is absent from the complete lock")
        })?;
        let members = mapping_field_mut(&mut raw, "members", merge_id)?;
        let key = Value::String(member_id.clone());
        if !members.contains_key(&key) {
            let baseline_row = baseline
                .get("members")
                .and_then(|members| members.get(member_id))
                .cloned()
                .unwrap_or(serde_yaml::to_value(typed).map_err(|_| {
                    input_error(merge_id, "selected lock row cannot be serialized")
                })?);
            members.insert(key.clone(), baseline_row);
        }
        let row = members
            .get_mut(&key)
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| input_error(merge_id, "selected lock row is not a mapping"))?;
        let replacement = serde_yaml::to_value(typed)
            .map_err(|_| input_error(merge_id, "selected lock row cannot be serialized"))?;
        let replacement = replacement
            .as_mapping()
            .ok_or_else(|| input_error(merge_id, "selected lock row is not a mapping"))?;
        for field in [
            "path",
            "source_id",
            "source_kind",
            "commit",
            "branch",
            "detached",
            "upstream",
            "dirty",
            "materialized",
        ] {
            let field = Value::String(field.into());
            row.remove(&field);
            if let Some(value) = replacement.get(&field) {
                row.insert(field, value.clone());
            }
        }
    }
    let rendered = serde_yaml::to_string(&raw)
        .map_err(|_| input_error(merge_id, "complete lock cannot be serialized"))?;
    if parse_lock(merge_id, &rendered)? != *complete {
        return Err(input_error(
            merge_id,
            "complete lock YAML differs from its typed model",
        ));
    }
    Ok(rendered)
}

fn mapping_field_mut<'a>(
    value: &'a mut Value,
    field: &str,
    merge_id: &str,
) -> ModelResult<&'a mut serde_yaml::Mapping> {
    value
        .get_mut(field)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| input_error(merge_id, "accepted lock members are not a mapping"))
}

pub(super) fn require_workspace(
    workspace_id: &str,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    merge_id: &str,
) -> ModelResult<()> {
    if manifest.workspace.id == workspace_id && lock.workspace_id == workspace_id {
        Ok(())
    } else {
        Err(input_error(
            merge_id,
            "accepted metadata workspace identity changed",
        ))
    }
}

pub(super) fn required<'a>(
    value: Option<&'a str>,
    merge_id: &str,
    detail: &str,
) -> ModelResult<&'a str> {
    value.ok_or_else(|| input_error(merge_id, detail))
}

pub(super) fn require_digest(value: &str, expected: &str, merge_id: &str) -> ModelResult<()> {
    if digest(value) == expected {
        Ok(())
    } else {
        Err(input_error(
            merge_id,
            "operation baseline exact bytes do not match their digest",
        ))
    }
}

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn input_error(merge_id: &str, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::AcceptanceInputDrift,
        format!("merge record '{merge_id}' acceptance input is incomplete: {detail}"),
    )
}
