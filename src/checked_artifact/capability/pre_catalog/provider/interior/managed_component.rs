use super::super::RawCatalogBytesV1;
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::CatalogNameBudgetV1;
use crate::checked_artifact::protocol::{
    OwnershipMarkerV1, ProtocolRecordKindV1, RecordObservationV1, ScratchBytesV1,
    classify_expected_prefix, managed_marker_name,
};
use crate::filesystem::FsKind;
use crate::filesystem::FsOpenMode;
use std::ffi::OsStr;

use super::*;

/// The scan bound for a staged or installed managed component interior.
///
/// A staged component holds exactly the ownership marker; an installed one
/// holds the marker until it retires and nothing after. Two is therefore one
/// row of headroom over every shape this owner accepts, and it is a *refusal
/// threshold* for the bounded enumeration (§4.1 family P4), not durable
/// vocabulary: no record, slot, purpose or phase is minted by it.
pub(crate) const MAX_MANAGED_COMPONENT_ENTRIES: usize = 2;

/// Bounded interior of an already-open staged managed component directory, so
/// the sealed publication primitive can re-verify its retained source handle
/// inside the acquisition window without reopening the name it is about to
/// consume.
///
/// This is the verification half of `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1's
/// **managed source-interior** arm (edge E15) — the row whose definition is "a
/// staged managed component's interior is neither record type". It lives here
/// rather than in `publication.rs` for the reason §4.4 records: "`publication.rs`
/// decides, `interior.rs` verifies".
pub(in crate::checked_artifact::capability::pre_catalog::provider) struct ManagedComponentInteriorObservationV1
{
    pub(crate) marker: RecordObservationV1<()>,
    pub(crate) extra_children: usize,
}

impl ManagedComponentInteriorObservationV1 {
    /// The exactness predicate: the deterministic resident ownership marker,
    /// byte-exact against the marker the intent issued, and no extra children.
    pub(in crate::checked_artifact::capability::pre_catalog::provider) const fn is_exact(
        &self,
    ) -> bool {
        self.extra_children == 0 && matches!(self.marker, RecordObservationV1::Exact(()))
    }
}

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn observe_managed_component_interior(
    directory: &crate::filesystem::FsDirectory,
    expected: &OwnershipMarkerV1,
) -> Result<ManagedComponentInteriorObservationV1, CheckedFsError> {
    let marker_name = managed_marker_name();
    let marker_name = OsStr::new(
        std::str::from_utf8(marker_name.as_bytes())
            .expect("the frozen managed marker name is ASCII"),
    );
    let expected_bytes = expected.encode_canonical();
    let mut budget = CatalogNameBudgetV1::new();
    let mut marker = RecordObservationV1::Missing;
    let mut extra_children = 0_usize;
    let mut seen = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate managed component", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read managed component", source))?;
        let child = entry;
        budget.charge_os_str(&child)?;
        seen += 1;
        if seen > MAX_MANAGED_COMPONENT_ENTRIES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "managed component exceeds its frozen interior bound",
            ));
        }
        if child == marker_name {
            marker = observe_managed_marker(directory, &child, &expected_bytes)?;
            continue;
        }
        extra_children += 1;
    }
    Ok(ManagedComponentInteriorObservationV1 {
        marker,
        extra_children,
    })
}

/// The resident marker, read bounded against the frozen `Marker` record bound
/// and compared byte-exact. The comparison is in-memory only: no handle, no row
/// and no path leaves this owner.
pub(crate) fn observe_managed_marker(
    directory: &crate::filesystem::FsDirectory,
    name: &OsStr,
    expected_bytes: &[u8],
) -> Result<RecordObservationV1<()>, CheckedFsError> {
    let metadata = directory
        .entry_metadata(name)
        .map_err(|source| CheckedFsError::io("observe ownership marker", source))?;
    if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
        return Ok(RecordObservationV1::Other);
    }
    let options = FsOpenMode::Read;
    let mut file = directory
        .open_file(name, &options)
        .map_err(|source| CheckedFsError::io("open ownership marker", source))?;
    let RawCatalogBytesV1::Bounded(bytes) =
        read_bounded_with(&mut file, ProtocolRecordKindV1::Marker.max_bytes())?
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(match classify_expected_prefix(&bytes, expected_bytes) {
        ScratchBytesV1::Exact => RecordObservationV1::Exact(()),
        ScratchBytesV1::PartialExpectedPrefix => RecordObservationV1::PartialExpectedPrefix,
        ScratchBytesV1::Missing | ScratchBytesV1::Other => RecordObservationV1::Other,
    })
}
