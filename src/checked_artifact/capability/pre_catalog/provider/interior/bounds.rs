use super::super::RawCatalogBytesV1;
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::protocol::{MAX_ROOT_ENTRIES, ProtocolRecordKindV1};
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

/// The catalog root's bound, widened from the ten infrastructure slots to the
/// already-frozen `MAX_ROOT_ENTRIES` (= 74 = 10 infrastructure + 64 active
/// action directories, `protocol/bounds.rs:21-23`) so a published
/// `RootEntryNameV1::ActiveAction` row survives reobservation.
///
/// `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2 (C-3) fact 3: "Ten is exactly
/// `|InfrastructureSlotV1::ALL|`, so a fully-populated catalog root has zero
/// headroom: the cap and the grammar have to move together." Both move here,
/// and both move only onto vocabulary R1+C0 already froze. Widening only: the
/// per-family caps below keep every previously-accepted interior accepted.
pub(crate) const MAX_INTERIOR_ENTRIES: usize = MAX_ROOT_ENTRIES;

pub(crate) fn interior_bound_exceeded() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::PrivateNamespaceCollisionScan,
        "catalog interior exceeds the frozen root-entry bound",
    )
}

pub(crate) fn reserve_one<T>(values: &mut Vec<T>) -> Result<(), CheckedFsError> {
    values.try_reserve_exact(1).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior row allocation failed",
        )
    })
}

pub(crate) fn read_bounded(
    file: &mut crate::filesystem::FsFile,
) -> Result<RawCatalogBytesV1, CheckedFsError> {
    read_bounded_with(
        file,
        ProtocolRecordKindV1::Infrastructure
            .max_bytes()
            .max(ProtocolRecordKindV1::CatalogBootstrap.max_bytes()),
    )
}

pub(crate) fn read_bounded_with(
    file: &mut crate::filesystem::FsFile,
    limit: usize,
) -> Result<RawCatalogBytesV1, CheckedFsError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog interior file", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog interior file", source))?;
    Ok(if bytes.len() > limit {
        RawCatalogBytesV1::Oversize
    } else {
        RawCatalogBytesV1::Bounded(bytes)
    })
}

/// The canonical ASCII spelling of a native name, or `None` when the platform
/// name is not ASCII. An action row's grammar is ASCII-only
/// (`protocol/slots.rs:385-400`), so a non-ASCII child can never be one and
/// falls through to the unchanged unowned-child refusal.
pub(crate) fn native_ascii_bytes(name: &OsStr) -> Option<&[u8]> {
    let bytes = name.to_str()?.as_bytes();
    bytes.is_ascii().then_some(bytes)
}
