#![forbid(clippy::disallowed_methods)]

use super::*;

enum FilesystemParent {
    Missing,
    Invalid,
    Open(FsDirectory),
}

pub(super) fn observe_filesystem_artifact_in(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    code: ErrorCode,
    label: &str,
) -> ModelResult<MergeArtifactFact> {
    let (parent_relative, leaf) = split_filesystem_relative(relative, code, label)?;
    let retained_root = filesystem
        .open_directory(root)
        .map_err(|cause| artifact_io(code, label, "open ambient artifact root", cause))?;
    let root_identity = super::identity::filesystem_directory_identity(&retained_root)
        .map_err(|_| reverse_door_identity_error(label))?;
    let parent =
        traverse_filesystem_parent(filesystem, &retained_root, parent_relative, code, label)?;
    let FilesystemParent::Open(parent) = parent else {
        return Ok(match parent {
            FilesystemParent::Missing => MergeArtifactFact::Missing,
            FilesystemParent::Invalid => MergeArtifactFact::Invalid,
            FilesystemParent::Open(_) => unreachable!(),
        });
    };
    let parent_identity = super::identity::filesystem_directory_identity(&parent)
        .map_err(|_| reverse_door_identity_error(label))?;
    let before = match filesystem.metadata_at(&parent, leaf) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MergeArtifactFact::Missing);
        }
        Err(cause) => {
            return Err(artifact_io(
                code,
                label,
                "read artifact leaf metadata",
                cause,
            ));
        }
    };
    if before.kind != FsKind::File || before.executable {
        return Ok(MergeArtifactFact::Invalid);
    }
    let file = match filesystem.open_file_at(&parent, leaf) {
        Ok(file) => file,
        Err(cause)
            if matches!(
                cause.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(MergeArtifactFact::Invalid);
        }
        Err(cause) => return Err(artifact_io(code, label, "open artifact no-follow", cause)),
    };
    let file_identity = super::identity::filesystem_file_identity(&file)
        .map_err(|_| reverse_door_identity_error(label))?;
    if !filesystem
        .file_entry_matches(&parent, leaf, &file)
        .map_err(|cause| {
            artifact_io(
                code,
                label,
                "bind opened artifact to directory entry",
                cause,
            )
        })?
    {
        return Ok(MergeArtifactFact::Invalid);
    }
    let bytes = filesystem
        .read_all(&file)
        .map_err(|cause| artifact_io(code, label, "read artifact bytes", cause))?;
    let after = match filesystem.metadata_at(&parent, leaf) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MergeArtifactFact::Invalid);
        }
        Err(cause) => {
            return Err(artifact_io(
                code,
                label,
                "reread artifact leaf metadata",
                cause,
            ));
        }
    };
    if before != after
        || !filesystem
            .file_entry_matches(&parent, leaf, &file)
            .map_err(|cause| {
                artifact_io(code, label, "rebind artifact to directory entry", cause)
            })?
        || super::identity::filesystem_file_identity(&file)
            .map_err(|_| reverse_door_identity_error(label))?
            != file_identity
        || !filesystem_parent_is_current(
            filesystem,
            &retained_root,
            parent_relative,
            &parent_identity,
            code,
            label,
        )?
    {
        return Ok(MergeArtifactFact::Invalid);
    }
    let reopened_root = filesystem
        .open_directory(root)
        .map_err(|cause| artifact_io(code, label, "reopen ambient artifact root", cause))?;
    if super::identity::filesystem_directory_identity(&reopened_root)
        .map_err(|_| reverse_door_identity_error(label))?
        != root_identity
    {
        return Ok(MergeArtifactFact::Invalid);
    }
    Ok(MergeArtifactFact::Bytes(bytes))
}

fn traverse_filesystem_parent(
    filesystem: &dyn FileSystem,
    root: &FsDirectory,
    relative: &Path,
    code: ErrorCode,
    label: &str,
) -> ModelResult<FilesystemParent> {
    let mut current = filesystem
        .clone_directory(root)
        .map_err(|cause| artifact_io(code, label, "retain artifact parent", cause))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Ok(FilesystemParent::Invalid);
        };
        let metadata = match filesystem.metadata_at(&current, name) {
            Ok(metadata) => metadata,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FilesystemParent::Missing);
            }
            Err(cause) => {
                return Err(artifact_io(
                    code,
                    label,
                    "traverse to artifact parent",
                    cause,
                ));
            }
        };
        if metadata.kind != FsKind::Directory {
            return Ok(FilesystemParent::Invalid);
        }
        let next = match filesystem.open_directory_at(&current, name) {
            Ok(next) => next,
            Err(cause) if cause.kind() != std::io::ErrorKind::PermissionDenied => {
                return Ok(FilesystemParent::Invalid);
            }
            Err(cause) => {
                return Err(artifact_io(
                    code,
                    label,
                    "traverse to artifact parent",
                    cause,
                ));
            }
        };
        if !filesystem
            .directory_entry_matches(&current, name, &next)
            .map_err(|cause| artifact_io(code, label, "bind artifact parent component", cause))?
        {
            return Ok(FilesystemParent::Invalid);
        }
        current = next;
    }
    Ok(FilesystemParent::Open(current))
}

fn filesystem_parent_is_current(
    filesystem: &dyn FileSystem,
    root: &FsDirectory,
    relative: &Path,
    expected: &super::identity::ObjectIdentity,
    code: ErrorCode,
    label: &str,
) -> ModelResult<bool> {
    let FilesystemParent::Open(parent) =
        traverse_filesystem_parent(filesystem, root, relative, code, label)?
    else {
        return Ok(false);
    };
    super::identity::filesystem_directory_identity(&parent)
        .map(|observed| observed == *expected)
        .map_err(|_| reverse_door_identity_error(label))
}

fn split_filesystem_relative<'path>(
    path: &'path Path,
    code: ErrorCode,
    label: &str,
) -> ModelResult<(&'path Path, &'path OsStr)> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ModelError::new(
            code,
            format!("checked {label}: path is not workspace-relative"),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| ModelError::new(code, format!("checked {label}: artifact has no parent")))?;
    let leaf = path
        .file_name()
        .ok_or_else(|| ModelError::new(code, format!("checked {label}: artifact has no leaf")))?;
    Ok((parent, leaf))
}

fn artifact_io(code: ErrorCode, label: &str, operation: &str, cause: std::io::Error) -> ModelError {
    ModelError::new(code, format!("checked {label}: {operation}: {cause}"))
}

fn reverse_door_identity_error(label: &str) -> ModelError {
    ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!(
            "checked {label}: {}",
            super::capability::HANDLE_FAIL_REVERSE_DOOR_ESCAPE
        ),
    )
}
