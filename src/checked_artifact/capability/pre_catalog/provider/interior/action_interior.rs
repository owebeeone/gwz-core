use super::super::RawCatalogBytesV1;
use super::super::filesystem::PlatformProviderV1;
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::CatalogNameBudgetV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionSlotV1, BaseActionSlotV1, MAX_ACTION_SLOTS,
    ObservedActionDirectoryV1, ProtocolRecordKindV1, RecordObservationV1, ScratchBytesV1,
    classify_expected_prefix,
};
use crate::filesystem::FsKind;
use crate::filesystem::FsOpenMode;
use std::ffi::OsStr;

use super::*;

/// Bounded observation of one action directory through the frozen
/// [`ActionSlotV1`] grammar.
///
/// It lives in this file because `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1
/// records that the verification a recheck arm drives "lives in a different
/// file of the same owner" — `publication.rs` decides, `interior.rs` verifies.
/// It returns the frozen R1 observation type, so neither the admission driver
/// nor the sealed primitive receives a handle or a raw row.
pub(in crate::checked_artifact::capability::pre_catalog::provider) fn observe_action_directory(
    parent: &crate::filesystem::FsDirectory,
    name: &OsStr,
    expected: &ActionCapacityReservationV1,
    platform: &impl PlatformProviderV1,
) -> Result<ObservedActionDirectoryV1, CheckedFsError> {
    let metadata = match parent.entry_metadata(name) {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ObservedActionDirectoryV1::Missing);
        }
        Err(source) => return Err(CheckedFsError::io("observe action directory", source)),
    };
    if metadata.kind != FsKind::Directory || metadata.kind == FsKind::Symlink {
        return Ok(ObservedActionDirectoryV1::Other);
    }
    let directory = parent
        .retained_child(name)
        .map_err(|source| CheckedFsError::io("open action directory", source))?;
    let identity = platform.dir_identity(&directory)?;
    let observed = observe_action_interior(&directory, expected)?;
    Ok(ObservedActionDirectoryV1::exact(
        identity.durable().clone(),
        observed.reservation,
        observed.extra_children,
    ))
}

/// Bounded interior of an already-open action directory, so the sealed
/// publication primitive can re-verify its retained source handle without
/// reopening the name it is about to consume.
pub(in crate::checked_artifact::capability::pre_catalog::provider) struct ActionInteriorObservationV1
{
    pub(in crate::checked_artifact::capability::pre_catalog::provider) reservation:
        RecordObservationV1<ActionCapacityReservationV1>,
    pub(in crate::checked_artifact::capability::pre_catalog::provider) extra_children: usize,
}

impl ActionInteriorObservationV1 {
    /// The §7 (:220-221) exactness predicate: the deterministic resident
    /// reservation and no extra children.
    pub(in crate::checked_artifact::capability::pre_catalog::provider) fn is_exact(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> bool {
        self.extra_children == 0
            && matches!(&self.reservation, RecordObservationV1::Exact(value) if value == expected)
    }

    /// R2-E E3.1's terminal source-interior predicate: the resident reservation
    /// is still this exact one.
    ///
    /// It is [`Self::is_exact`] without the `extra_children == 0` clause,
    /// which holds of a freshly staged action directory and never of one that
    /// has run an action — the directory a terminal retirement moves carries
    /// its authority, payload, worklist and retired-alias rows by construction.
    /// The bounded enumeration and the frozen `MAX_ACTION_SLOTS` refusal are
    /// the same for both; only this clause differs. "`publication.rs` decides,
    /// `interior.rs` verifies" (freeze §4.4 Class 1).
    pub(in crate::checked_artifact::capability::pre_catalog::provider) fn is_reservation_exact(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> bool {
        matches!(&self.reservation, RecordObservationV1::Exact(value) if value == expected)
    }
}

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn observe_action_interior(
    directory: &crate::filesystem::FsDirectory,
    expected: &ActionCapacityReservationV1,
) -> Result<ActionInteriorObservationV1, CheckedFsError> {
    let reservation_name =
        ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(expected.action_digest());
    let reservation_name = OsStr::new(reservation_name.as_str());
    let expected_bytes = expected.encode_canonical().map_err(|_| {
        CheckedFsError::ambiguous(
            "action capacity reservation",
            "expected capacity record is not canonically encodable",
        )
    })?;
    let mut budget = CatalogNameBudgetV1::new();
    let mut reservation = RecordObservationV1::Missing;
    let mut extra_children = 0_usize;
    let mut seen = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate action directory", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read action directory", source))?;
        let child = entry;
        budget.charge_os_str(&child)?;
        seen += 1;
        if seen > MAX_ACTION_SLOTS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "action directory exceeds the frozen action-slot bound",
            ));
        }
        if child == reservation_name {
            reservation = observe_reservation(directory, &child, &expected_bytes, expected)?;
            continue;
        }
        extra_children += 1;
    }
    Ok(ActionInteriorObservationV1 {
        reservation,
        extra_children,
    })
}

pub(crate) fn observe_reservation(
    directory: &crate::filesystem::FsDirectory,
    name: &OsStr,
    expected_bytes: &[u8],
    expected: &ActionCapacityReservationV1,
) -> Result<RecordObservationV1<ActionCapacityReservationV1>, CheckedFsError> {
    let metadata = directory
        .entry_metadata(name)
        .map_err(|source| CheckedFsError::io("observe resident reservation", source))?;
    if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
        return Ok(RecordObservationV1::Other);
    }
    let options = FsOpenMode::Read;
    let mut file = directory
        .open_file(name, &options)
        .map_err(|source| CheckedFsError::io("open resident reservation", source))?;
    let RawCatalogBytesV1::Bounded(bytes) =
        read_bounded_with(&mut file, ProtocolRecordKindV1::Capacity.max_bytes())?
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(match classify_expected_prefix(&bytes, expected_bytes) {
        ScratchBytesV1::Exact => RecordObservationV1::Exact(expected.clone()),
        ScratchBytesV1::PartialExpectedPrefix => RecordObservationV1::PartialExpectedPrefix,
        ScratchBytesV1::Missing | ScratchBytesV1::Other => RecordObservationV1::Other,
    })
}
