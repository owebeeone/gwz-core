use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::filesystem::{FileSystem, FsDirectory, FsIdentity, make_filesystem};
use crate::model::{ErrorCode, ModelError, ModelResult};

const MERGE_DIR: &str = ".gwz/merge";
const DONE_DIR: &str = ".gwz/merge/done";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalRecordKind {
    Open,
    Archived,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalRecordPath {
    kind: CanonicalRecordKind,
    path: PathBuf,
    identity: FsIdentity,
}

impl CanonicalRecordPath {
    pub(crate) fn kind(&self) -> CanonicalRecordKind {
        self.kind
    }

    pub(crate) fn as_path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImmutableBytes(Arc<[u8]>);

impl ImmutableBytes {
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    #[allow(
        dead_code,
        reason = "the v1 archive authority consumes the opaque digest"
    )]
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalRecordLeaf {
    Absent,
    Exact {
        path: CanonicalRecordPath,
        bytes: ImmutableBytes,
        digest: Sha256Digest,
    },
}

impl CanonicalRecordLeaf {
    #[allow(dead_code, reason = "focused location tests assert absence explicitly")]
    pub(crate) fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub(crate) fn exact(&self) -> Option<(&CanonicalRecordPath, &ImmutableBytes, Sha256Digest)> {
        match self {
            Self::Absent => None,
            Self::Exact {
                path,
                bytes,
                digest,
            } => Some((path, bytes, *digest)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalMergeLocations {
    open: CanonicalRecordLeaf,
    archived: CanonicalRecordLeaf,
}

impl CanonicalMergeLocations {
    pub(crate) fn open(&self) -> &CanonicalRecordLeaf {
        &self.open
    }

    pub(crate) fn archived(&self) -> &CanonicalRecordLeaf {
        &self.archived
    }
}

/// Read the canonical open and archived leaves without following any named
/// parent or leaf symlink. The bytes and digest in each `Exact` result come
/// from the same checked file handle.
pub(crate) fn acquire_canonical_merge_locations(
    root: &Path,
    merge_id: &str,
) -> ModelResult<CanonicalMergeLocations> {
    validate_merge_id(merge_id)?;
    acquire_filesystem_merge_locations(root, merge_id)
}

fn absent_locations() -> CanonicalMergeLocations {
    CanonicalMergeLocations {
        open: CanonicalRecordLeaf::Absent,
        archived: CanonicalRecordLeaf::Absent,
    }
}

fn acquire_filesystem_merge_locations(
    root_path: &Path,
    merge_id: &str,
) -> ModelResult<CanonicalMergeLocations> {
    let filesystem = make_filesystem();
    let root_path = filesystem
        .canonical_path(root_path)
        .map_err(|error| location_error(root_path, error))?;
    let root = filesystem
        .open_directory(&root_path)
        .map_err(|error| location_error(&root_path, error))?;
    let root_identity = filesystem
        .directory_identity(&root)
        .map_err(|error| location_error(&root_path, error))?;
    let gwz_path = root_path.join(".gwz");
    let Some((gwz, gwz_identity)) = optional_directory(
        &filesystem,
        &root,
        ".gwz".as_ref(),
        &gwz_path,
        ErrorCode::MergeRecordUnreadable,
    )?
    else {
        return Ok(absent_locations());
    };
    let merge_path = root_path.join(MERGE_DIR);
    let Some((merge, merge_identity)) = optional_directory(
        &filesystem,
        &gwz,
        "merge".as_ref(),
        &merge_path,
        ErrorCode::MergeRecordUnreadable,
    )?
    else {
        return Ok(absent_locations());
    };
    let leaf_name = format!("{merge_id}.yaml");
    let open_path = merge_path.join(&leaf_name);
    let open = read_leaf(
        &filesystem,
        &merge,
        leaf_name.as_ref(),
        &open_path,
        CanonicalRecordKind::Open,
    )?;
    let done_path = root_path.join(DONE_DIR);
    let done = optional_directory(
        &filesystem,
        &merge,
        "done".as_ref(),
        &done_path,
        ErrorCode::ArchivedRecordUnreadable,
    )?;
    let archived = match &done {
        Some((done, _)) => read_leaf(
            &filesystem,
            done,
            leaf_name.as_ref(),
            &done_path.join(&leaf_name),
            CanonicalRecordKind::Archived,
        )?,
        None => CanonicalRecordLeaf::Absent,
    };

    #[cfg(test)]
    inject_location_fault(&merge_path, merge_id);

    let reopened_root = filesystem
        .open_directory(&root_path)
        .map_err(|error| location_error(&root_path, error))?;
    if filesystem
        .directory_identity(&reopened_root)
        .map_err(|error| location_error(&root_path, error))?
        != root_identity
        || !filesystem
            .directory_entry_matches(&root, ".gwz".as_ref(), &gwz)
            .map_err(|error| location_error(&gwz_path, error))?
        || filesystem
            .directory_identity(&gwz)
            .map_err(|error| location_error(&gwz_path, error))?
            != gwz_identity
        || !filesystem
            .directory_entry_matches(&gwz, "merge".as_ref(), &merge)
            .map_err(|error| location_error(&merge_path, error))?
        || filesystem
            .directory_identity(&merge)
            .map_err(|error| location_error(&merge_path, error))?
            != merge_identity
    {
        return Err(changed_parent(&merge_path));
    }
    let final_done = optional_directory(
        &filesystem,
        &merge,
        "done".as_ref(),
        &done_path,
        ErrorCode::ArchivedRecordUnreadable,
    )?;
    if done.as_ref().map(|(_, identity)| identity)
        != final_done.as_ref().map(|(_, identity)| identity)
    {
        return Err(changed_parent(&done_path));
    }
    let final_open = read_leaf(
        &filesystem,
        &merge,
        leaf_name.as_ref(),
        &open_path,
        CanonicalRecordKind::Open,
    )?;
    let final_archived = match final_done {
        Some((done, _)) => read_leaf(
            &filesystem,
            &done,
            leaf_name.as_ref(),
            &done_path.join(&leaf_name),
            CanonicalRecordKind::Archived,
        )?,
        None => CanonicalRecordLeaf::Absent,
    };
    if open != final_open || archived != final_archived {
        return Err(contention_error(format!(
            "canonical merge record leaves for '{merge_id}' changed during observation"
        )));
    }
    Ok(CanonicalMergeLocations { open, archived })
}

fn optional_directory(
    filesystem: &impl FileSystem,
    parent: &FsDirectory,
    name: &std::ffi::OsStr,
    path: &Path,
    code: ErrorCode,
) -> ModelResult<Option<(FsDirectory, FsIdentity)>> {
    match filesystem.open_directory_at(parent, name) {
        Ok(directory) => {
            let identity = filesystem
                .directory_identity(&directory)
                .map_err(|error| location_error_with_code(path, error, code))?;
            Ok(Some((directory, identity)))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(location_error_with_code(path, error, code)),
    }
}

fn read_leaf(
    filesystem: &impl FileSystem,
    parent: &FsDirectory,
    name: &std::ffi::OsStr,
    path: &Path,
    kind: CanonicalRecordKind,
) -> ModelResult<CanonicalRecordLeaf> {
    let file = match filesystem.open_file_at(parent, name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CanonicalRecordLeaf::Absent);
        }
        Err(error) => return Err(leaf_error(path, kind, error)),
    };
    let identity = filesystem
        .file_identity(&file)
        .map_err(|error| leaf_error(path, kind, error))?;
    if !filesystem
        .file_entry_matches(parent, name, &file)
        .map_err(|error| leaf_error(path, kind, error))?
    {
        return Err(changed_leaf(path));
    }
    let bytes = filesystem
        .read_all(&file)
        .map_err(|error| leaf_error(path, kind, error))?;
    let reopened = filesystem
        .open_file_at(parent, name)
        .map_err(|error| leaf_error(path, kind, error))?;
    if !filesystem
        .file_entry_matches(parent, name, &file)
        .map_err(|error| leaf_error(path, kind, error))?
        || filesystem
            .file_identity(&reopened)
            .map_err(|error| leaf_error(path, kind, error))?
            != identity
        || !filesystem
            .file_entry_matches(parent, name, &reopened)
            .map_err(|error| leaf_error(path, kind, error))?
        || filesystem
            .read_all(&reopened)
            .map_err(|error| leaf_error(path, kind, error))?
            != bytes
    {
        return Err(changed_leaf(path));
    }
    let digest = Sha256Digest(Sha256::digest(&bytes).into());
    Ok(CanonicalRecordLeaf::Exact {
        path: CanonicalRecordPath {
            kind,
            path: path.into(),
            identity,
        },
        bytes: ImmutableBytes(Arc::from(bytes)),
        digest,
    })
}

fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
    if merge_id.is_empty()
        || matches!(merge_id, "." | "..")
        || !merge_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("invalid merge record id '{merge_id}'"),
        ));
    }
    Ok(())
}

fn changed_leaf(path: &Path) -> ModelError {
    contention_error(format!(
        "canonical merge record leaf '{}' changed during observation",
        path.display()
    ))
}

fn changed_parent(path: &Path) -> ModelError {
    contention_error(format!(
        "canonical merge record parent '{}' changed during observation",
        path.display()
    ))
}

fn contention_error(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

fn location_error(path: &Path, reason: impl std::fmt::Display) -> ModelError {
    location_error_with_code(path, reason, ErrorCode::MergeRecordUnreadable)
}

fn leaf_error(
    path: &Path,
    kind: CanonicalRecordKind,
    reason: impl std::fmt::Display,
) -> ModelError {
    let code = match kind {
        CanonicalRecordKind::Open => ErrorCode::MergeRecordUnreadable,
        CanonicalRecordKind::Archived => ErrorCode::ArchivedRecordUnreadable,
    };
    location_error_with_code(path, reason, code)
}

fn location_error_with_code(
    path: &Path,
    reason: impl std::fmt::Display,
    code: ErrorCode,
) -> ModelError {
    ModelError::new(
        code,
        format!(
            "canonical merge record location '{}' is unreadable: {reason}",
            path.display()
        ),
    )
}

#[cfg(test)]
thread_local! {
    static LOCATION_FAULT: std::cell::Cell<LocationFault> = const {
        std::cell::Cell::new(LocationFault::None)
    };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocationFault {
    None,
    ReplaceParent,
    AppearOpen,
    ReplaceOpen,
    AppearArchived,
}

#[cfg(test)]
pub(crate) fn replace_parent_before_final_check_for_test() {
    LOCATION_FAULT.with(|fault| fault.set(LocationFault::ReplaceParent));
}

#[cfg(test)]
pub(crate) fn appear_open_before_final_check_for_test() {
    LOCATION_FAULT.with(|fault| fault.set(LocationFault::AppearOpen));
}

#[cfg(test)]
pub(crate) fn replace_open_before_final_check_for_test() {
    LOCATION_FAULT.with(|fault| fault.set(LocationFault::ReplaceOpen));
}

#[cfg(test)]
pub(crate) fn appear_archived_before_final_check_for_test() {
    LOCATION_FAULT.with(|fault| fault.set(LocationFault::AppearArchived));
}

#[cfg(test)]
fn inject_location_fault(merge: &Path, merge_id: &str) {
    let fault = LOCATION_FAULT.with(|slot| slot.replace(LocationFault::None));
    let open = merge.join(format!("{merge_id}.yaml"));
    match fault {
        LocationFault::None => {}
        LocationFault::ReplaceParent => {
            make_filesystem()
                .rename(
                    merge,
                    &merge.with_extension("observed-old"),
                    crate::filesystem::RenameMode::Replace,
                )
                .expect("test parent rename succeeds");
            make_filesystem()
                .create_directories(merge)
                .expect("test replacement parent is created");
        }
        LocationFault::AppearOpen => crate::filesystem::write_atomic_for_test(&open, b"appeared")
            .expect("test open leaf appears"),
        LocationFault::ReplaceOpen => {
            let bytes = make_filesystem()
                .read(&open)
                .expect("test open leaf exists");
            make_filesystem()
                .rename(
                    &open,
                    &open.with_extension("old"),
                    crate::filesystem::RenameMode::Replace,
                )
                .expect("test open leaf rename succeeds");
            crate::filesystem::write_atomic_for_test(&open, &bytes)
                .expect("test replacement open leaf is written");
        }
        LocationFault::AppearArchived => {
            let done = merge.join("done");
            make_filesystem()
                .create_directories(&done)
                .expect("test archive parent appears");
            crate::filesystem::write_atomic_for_test(
                &done.join(format!("{merge_id}.yaml")),
                b"appeared",
            )
            .expect("test archived leaf appears");
        }
    }
}
