use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember,
    ResolvedMemberArtifact,
};
use crate::git::{GitHeadState, GitStatus};
use crate::model::ModelResult;

use super::response::io_error;

pub(crate) fn read_lock_or_empty(root: &Path, workspace_id: &str) -> ModelResult<LockArtifact> {
    read_lock_or_empty_in(&crate::filesystem::make_filesystem(), root, workspace_id)
}

pub(crate) fn read_lock_or_empty_in(
    filesystem: &dyn crate::filesystem::FileSystem,
    root: &Path,
    workspace_id: &str,
) -> ModelResult<LockArtifact> {
    match filesystem.metadata(&root.join(artifact::LOCK_PATH)) {
        Ok(_) => artifact::read_lock_in(filesystem, root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: workspace_id.to_owned(),
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            members: BTreeMap::new(),
        }),
        Err(error) => Err(io_error(error)),
    }
}

pub(crate) fn resolved_member(
    member: &ManifestMember,
    head: &GitHeadState,
    status: &GitStatus,
) -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: member.path.clone(),
        source_id: Some(member.source_id.clone()),
        source_kind: ArtifactSourceKind::Git,
        commit: head.commit.clone(),
        branch: head.branch.clone(),
        detached: Some(head.is_detached),
        upstream: None,
        dirty: Some(status.is_dirty),
        materialized: Some(true),
    }
}

pub(crate) fn members_with_source_id(manifest: &ManifestArtifact, source_id: &str) -> Vec<String> {
    manifest
        .members
        .iter()
        .filter(|member| member.source_id == source_id)
        .map(|member| member.id.clone())
        .collect()
}

pub(crate) fn default_source_id(member_id: &str) -> String {
    format!(
        "src_{}",
        member_id
            .strip_prefix("mem_")
            .expect("validated member id has mem_ prefix")
    )
}

pub(crate) fn now_marker() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("unix-ms:{millis}")
}
