use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

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
    identity: FileIdentity,
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
    let root = root
        .canonicalize()
        .map_err(|error| location_error(root, error))?;
    let root_identity = require_real_directory(&root)?;

    let gwz = root.join(".gwz");
    let Some(gwz_identity) = optional_real_directory(&gwz)? else {
        return Ok(absent_locations());
    };
    let merge = root.join(MERGE_DIR);
    let Some(merge_identity) = optional_real_directory(&merge)? else {
        return Ok(absent_locations());
    };

    let open_path = merge.join(format!("{merge_id}.yaml"));
    let open = read_leaf(&open_path, CanonicalRecordKind::Open)?;
    let done = root.join(DONE_DIR);
    let done_identity = optional_archived_directory(&done)?;
    let archived = match done_identity.as_ref() {
        Some(_) => read_leaf(
            &done.join(format!("{merge_id}.yaml")),
            CanonicalRecordKind::Archived,
        )?,
        None => CanonicalRecordLeaf::Absent,
    };

    #[cfg(test)]
    inject_location_fault(&merge, merge_id);

    // A parent replacement after either read invalidates both observations.
    require_same_directory(&root, &root_identity)?;
    require_same_directory(&gwz, &gwz_identity)?;
    require_same_directory(&merge, &merge_identity)?;
    let final_done_identity = optional_archived_directory(&done)?;
    match (done_identity.as_ref(), final_done_identity.as_ref()) {
        (Some(before), Some(after)) if before == after => {}
        (None, None) => {}
        _ => return Err(changed_parent(&done)),
    }
    let final_open = read_leaf(&open_path, CanonicalRecordKind::Open)?;
    let final_archived = match final_done_identity {
        Some(_) => read_leaf(
            &done.join(format!("{merge_id}.yaml")),
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

fn absent_locations() -> CanonicalMergeLocations {
    CanonicalMergeLocations {
        open: CanonicalRecordLeaf::Absent,
        archived: CanonicalRecordLeaf::Absent,
    }
}

fn optional_real_directory(path: &Path) -> ModelResult<Option<FileIdentity>> {
    optional_directory(path, ErrorCode::MergeRecordUnreadable)
}

fn optional_archived_directory(path: &Path) -> ModelResult<Option<FileIdentity>> {
    optional_directory(path, ErrorCode::ArchivedRecordUnreadable)
}

fn optional_directory(path: &Path, code: ErrorCode) -> ModelResult<Option<FileIdentity>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            match identity_at_named_path(path, &metadata)
                .map_err(|error| location_error_with_code(path, error, code))?
            {
                Some(identity) => Ok(Some(identity)),
                None => Err(changed_parent(path)),
            }
        }
        Ok(_) => Err(location_error_with_code(
            path,
            "path is not a real directory",
            code,
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(location_error_with_code(path, error, code)),
    }
}

fn require_real_directory(path: &Path) -> ModelResult<FileIdentity> {
    optional_real_directory(path)?.ok_or_else(|| changed_parent(path))
}

fn require_same_directory(path: &Path, expected: &FileIdentity) -> ModelResult<()> {
    let current = require_real_directory(path)?;
    if expected == &current {
        Ok(())
    } else {
        Err(changed_parent(path))
    }
}

fn read_leaf(path: &Path, kind: CanonicalRecordKind) -> ModelResult<CanonicalRecordLeaf> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Err(leaf_error(path, kind, "record leaf is not a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CanonicalRecordLeaf::Absent);
        }
        Err(reason) => return Err(leaf_error(path, kind, reason)),
    };
    let Some(before_identity) =
        identity_at_named_path(path, &before).map_err(|reason| leaf_error(path, kind, reason))?
    else {
        return Err(changed_leaf(path));
    };
    let mut file = open_named_path(path).map_err(|reason| leaf_error(path, kind, reason))?;
    let opened = file
        .metadata()
        .map_err(|reason| leaf_error(path, kind, reason))?;
    let opened_identity =
        identity_from_file(&file, &opened).map_err(|reason| leaf_error(path, kind, reason))?;
    if !opened.file_type().is_file() || before_identity != opened_identity {
        return Err(changed_leaf(path));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|reason| leaf_error(path, kind, reason))?;
    let after = fs::symlink_metadata(path).map_err(|reason| {
        if reason.kind() == io::ErrorKind::NotFound {
            changed_leaf(path)
        } else {
            leaf_error(path, kind, reason)
        }
    })?;
    let Some(after_identity) =
        identity_at_named_path(path, &after).map_err(|reason| leaf_error(path, kind, reason))?
    else {
        return Err(changed_leaf(path));
    };
    if !after.file_type().is_file()
        || before_identity != after_identity
        || opened_identity != after_identity
        || opened.len() != bytes.len() as u64
    {
        return Err(changed_leaf(path));
    }
    let digest = Sha256Digest(Sha256::digest(&bytes).into());
    Ok(CanonicalRecordLeaf::Exact {
        path: CanonicalRecordPath {
            kind,
            path: path.to_owned(),
            identity: opened_identity,
        },
        bytes: ImmutableBytes(Arc::from(bytes)),
        digest,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume: u32,
    #[cfg(windows)]
    index: u64,
}

#[cfg(unix)]
pub(in crate::workspace_ops::merge) fn identity_at_named_path(
    _path: &Path,
    metadata: &Metadata,
) -> io::Result<Option<FileIdentity>> {
    Ok(Some(identity_from_metadata(metadata)))
}

#[cfg(windows)]
pub(in crate::workspace_ops::merge) fn identity_at_named_path(
    path: &Path,
    metadata: &Metadata,
) -> io::Result<Option<FileIdentity>> {
    let file = open_named_path(path)?;
    let opened = file.metadata()?;
    if metadata.file_type() != opened.file_type() {
        return Ok(None);
    }
    identity_from_file(&file, &opened).map(Some)
}

#[cfg(unix)]
pub(in crate::workspace_ops::merge) fn open_named_path(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
pub(in crate::workspace_ops::merge) fn open_named_path(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path)
}

#[cfg(unix)]
pub(in crate::workspace_ops::merge) fn identity_from_file(
    _file: &File,
    metadata: &Metadata,
) -> io::Result<FileIdentity> {
    Ok(identity_from_metadata(metadata))
}

#[cfg(unix)]
fn identity_from_metadata(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(windows)]
pub(in crate::workspace_ops::merge) fn identity_from_file(
    file: &File,
    _metadata: &Metadata,
) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a valid handle for the duration of the synchronous
    // call and `information` is a writable value of the required Win32 type.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity {
        volume: information.dwVolumeSerialNumber,
        index: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
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
            let old = merge.with_extension("observed-old");
            fs::rename(merge, old).expect("test parent rename succeeds");
            fs::create_dir(merge).expect("test replacement parent is created");
        }
        LocationFault::AppearOpen => {
            fs::write(open, b"appeared").expect("test open leaf appears");
        }
        LocationFault::ReplaceOpen => {
            let bytes = fs::read(&open).expect("test open leaf exists");
            fs::rename(&open, open.with_extension("old")).expect("test open leaf rename succeeds");
            fs::write(open, bytes).expect("test replacement open leaf is written");
        }
        LocationFault::AppearArchived => {
            let done = merge.join("done");
            fs::create_dir(&done).expect("test archive parent appears");
            fs::write(done.join(format!("{merge_id}.yaml")), b"appeared")
                .expect("test archived leaf appears");
        }
    }
}
