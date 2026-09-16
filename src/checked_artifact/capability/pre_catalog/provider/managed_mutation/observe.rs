use super::super::retained::encode_identity;
use crate::checked_artifact::capability::{
    CanonicalPathIdentityV1, CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1,
    PathComponentMode, PlatformCapability,
};
use crate::filesystem::FsKind;
use crate::filesystem::{FsDirectory as Dir, FsOpenMode};
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

use super::*;

/// One exact managed namespace object, observed and closed.
///
/// The observation handle is dropped before it is returned for the reason
/// `namespace_mutation.rs:97-105` records: on Windows the sealed primitive
/// reopens the source with `DELETE` access, which a surviving caller handle
/// opened without `FILE_SHARE_DELETE` would refuse.
pub(in crate::checked_artifact) struct ObservedManagedObjectV1 {
    pub(crate) identity: DurableObjectIdentityV1,
    pub(crate) encoded_identity: Vec<u8>,
    pub(crate) bytes: Vec<u8>,
}

impl ObservedManagedObjectV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }
}

/// The durable facts one managed component installation observed.
pub(in crate::checked_artifact) struct ManagedInstalledFactsV1 {
    pub(in crate::checked_artifact) marker_object_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_mode: PathComponentMode,
    pub(in crate::checked_artifact) installed_path: CanonicalPathIdentityV1,
}

/// The durable facts one ownership-marker retirement observed.
pub(in crate::checked_artifact) struct ManagedRetiredFactsV1 {
    pub(in crate::checked_artifact) marker_bytes: Vec<u8>,
    pub(in crate::checked_artifact) retired_marker_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_parent_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_parent_mode: PathComponentMode,
    pub(in crate::checked_artifact) installed_parent_path: CanonicalPathIdentityV1,
}

/// One exact regular-file managed object, read bounded against the frozen bound
/// of `kind` — never against the file's own length (ConsumerCheckpoint §8
/// :236-237). R2-D Step 3.1b adds the `kind` parameter because the managed
/// intent record is a `BootstrapIntent`, not a `Marker`.
pub(crate) fn observe_regular_file(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
    kind: ProtocolRecordKindV1,
) -> Result<ObservedManagedObjectV1, CheckedFsError> {
    let metadata = directory
        .entry_metadata(name)
        .map_err(|source| CheckedFsError::io("observe managed object", source))?;
    if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
        return Err(CheckedFsError::ambiguous(
            label,
            "managed object is not a canonical regular file",
        ));
    }
    let options = FsOpenMode::Read;
    let mut file = directory
        .open_file(name, &options)
        .map_err(|source| CheckedFsError::io("open managed object no-follow", source))?;
    let fact = super::super::HostPlatform.file_identity(&file)?;
    let limit = kind.max_bytes();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit + 1).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "managed object read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind managed object", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read managed object", source))?;
    if bytes.len() > limit {
        return Err(CheckedFsError::ambiguous(
            label,
            "managed object exceeds its frozen record bound",
        ));
    }
    Ok(ObservedManagedObjectV1 {
        identity: fact.durable().clone(),
        encoded_identity: encode_identity(&fact),
        bytes,
    })
}

pub(crate) fn observe_marker(
    installed: &Dir,
    label: &'static str,
) -> Result<ObservedManagedObjectV1, CheckedFsError> {
    observe_regular_file(
        installed,
        &os_name(&managed_marker_name()),
        label,
        ProtocolRecordKindV1::Marker,
    )
}
