use std::path::Path;

use crate::artifact::ManifestArtifact;
use crate::filesystem::{FileSystem, FsKind};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::{MemberPath, validate_member_path_set};

use super::response::io_error;

pub(crate) fn assert_workspace_id(
    manifest: &ManifestArtifact,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<()> {
    if let Some(expected) = workspace.and_then(|workspace| workspace.workspace_id.as_ref())
        && expected != &manifest.workspace.id
    {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            "workspace id does not match manifest",
        ));
    }
    Ok(())
}

pub(crate) fn reject_existing_active_member_path_overlap(
    manifest: &ManifestArtifact,
    path: &MemberPath,
) -> ModelResult<()> {
    let mut active_paths = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| MemberPath::parse(&member.path))
        .collect::<ModelResult<Vec<_>>>()?;
    active_paths.push(path.clone());
    validate_member_path_set(&active_paths)
}

pub(crate) fn reject_duplicate_member_id(
    manifest: &ManifestArtifact,
    member_id: &str,
) -> ModelResult<()> {
    if manifest.members.iter().any(|member| member.id == member_id) {
        Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "member id is already registered",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn ensure_member_target_available_in(
    filesystem: &dyn FileSystem,
    path: &Path,
) -> ModelResult<()> {
    let kind = match filesystem.kind(path) {
        Ok(kind) => kind,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(error)),
    };
    if kind != FsKind::Directory {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "member path exists and is not a directory",
        ));
    }
    if !filesystem
        .read_directory(path)
        .map_err(io_error)?
        .is_empty()
    {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "member path is not empty",
        ));
    }
    Ok(())
}
