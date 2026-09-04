//! Opaque, bounded observations used to derive checked-artifact owners.

use super::{
    CanonicalMergeLocations, CanonicalRecordKind, CanonicalRecordLeaf, RecordDecodeError,
    decode_v0_parts,
};
use crate::workspace_ops::merge::OperationState;

pub(crate) const MAX_CHECKED_OWNER_RECORD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckedOwnerRecordVersion {
    V0,
    V1,
}

/// One exact durable-record read and the identity fields decoded from those
/// same bytes. Its fields and constructor remain inside `record_wire` so a
/// consumer cannot pair caller-selected IDs with an unrelated byte digest.
pub(crate) struct CheckedOwnerRecordObservation<'a> {
    version: CheckedOwnerRecordVersion,
    exact_bytes: &'a [u8],
    workspace_id: String,
    merge_id: String,
    operation_id: String,
}

/// An exact terminal record observed at the canonical open source while the
/// canonical archive destination was absent in the same stable acquisition.
/// Only record-wire arbitration can construct this value.
pub(crate) struct CheckedArchiveSourceObservation<'a> {
    owner: CheckedOwnerRecordObservation<'a>,
}

#[derive(Debug)]
pub(crate) enum CheckedOwnerObservationError {
    Absent,
    Bounds,
    Identity,
    NotTerminal,
    InvalidTerminal,
    DestinationPresent,
    Decode(RecordDecodeError),
}

impl CheckedOwnerRecordObservation<'_> {
    pub(crate) const fn version(&self) -> CheckedOwnerRecordVersion {
        self.version
    }

    pub(crate) const fn exact_bytes(&self) -> &[u8] {
        self.exact_bytes
    }

    pub(crate) fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub(crate) fn merge_id(&self) -> &str {
        &self.merge_id
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

impl CheckedArchiveSourceObservation<'_> {
    pub(crate) fn owner(&self) -> &CheckedOwnerRecordObservation<'_> {
        &self.owner
    }
}

fn decode_checked_owner_v0_bytes(
    exact_bytes: &[u8],
) -> Result<(CheckedOwnerRecordObservation<'_>, OperationState), CheckedOwnerObservationError> {
    require_bounded(exact_bytes)?;
    let (_, record) = decode_v0_parts(exact_bytes).map_err(CheckedOwnerObservationError::Decode)?;
    Ok((
        CheckedOwnerRecordObservation {
            version: CheckedOwnerRecordVersion::V0,
            exact_bytes,
            workspace_id: record.workspace_id,
            merge_id: record.merge_id,
            operation_id: record.operation_id,
        },
        record.state,
    ))
}

fn observe_checked_owner_v0_bytes(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    decode_checked_owner_v0_bytes(exact_bytes).map(|(observation, _)| observation)
}

pub(crate) fn observe_checked_owner_v0_from_canonical(
    leaf: &CanonicalRecordLeaf,
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    let Some((path, bytes, _)) = leaf.exact() else {
        return Err(CheckedOwnerObservationError::Absent);
    };
    let observation = observe_checked_owner_v0_bytes(bytes.as_slice())?;
    if path.as_path().file_stem().and_then(|value| value.to_str()) != Some(observation.merge_id()) {
        return Err(CheckedOwnerObservationError::Identity);
    }
    Ok(observation)
}

pub(crate) fn observe_checked_archive_source_v0(
    locations: &CanonicalMergeLocations,
) -> Result<CheckedArchiveSourceObservation<'_>, CheckedOwnerObservationError> {
    observe_checked_archive_source_v0_leaves(locations.open(), locations.archived())
}

fn observe_checked_archive_source_v0_leaves<'a>(
    open: &'a CanonicalRecordLeaf,
    archived: &CanonicalRecordLeaf,
) -> Result<CheckedArchiveSourceObservation<'a>, CheckedOwnerObservationError> {
    if !archived.is_absent() {
        return Err(CheckedOwnerObservationError::DestinationPresent);
    }
    let Some((path, bytes, _)) = open.exact() else {
        return Err(CheckedOwnerObservationError::Absent);
    };
    if path.kind() != CanonicalRecordKind::Open {
        return Err(CheckedOwnerObservationError::Identity);
    }
    let (owner, state) = decode_checked_owner_v0_bytes(bytes.as_slice())?;
    if state.is_open() {
        return Err(CheckedOwnerObservationError::NotTerminal);
    }
    super::archive::decode_archived(bytes.as_slice(), owner.merge_id())
        .map_err(|_| CheckedOwnerObservationError::InvalidTerminal)?;
    if path.as_path().file_stem().and_then(|value| value.to_str()) != Some(owner.merge_id()) {
        return Err(CheckedOwnerObservationError::Identity);
    }
    Ok(CheckedArchiveSourceObservation { owner })
}

#[cfg(test)]
pub(crate) fn observe_checked_archive_source_v0_leaves_for_test<'a>(
    open: &'a CanonicalRecordLeaf,
    archived: &CanonicalRecordLeaf,
) -> Result<CheckedArchiveSourceObservation<'a>, CheckedOwnerObservationError> {
    observe_checked_archive_source_v0_leaves(open, archived)
}

pub(crate) fn observe_checked_archive_source_v1(
    locations: &CanonicalMergeLocations,
) -> Result<CheckedArchiveSourceObservation<'_>, CheckedOwnerObservationError> {
    if !locations.archived().is_absent() {
        return Err(CheckedOwnerObservationError::DestinationPresent);
    }
    let Some((path, bytes, _)) = locations.open().exact() else {
        return Err(CheckedOwnerObservationError::Absent);
    };
    if path.kind() != CanonicalRecordKind::Open {
        return Err(CheckedOwnerObservationError::Identity);
    }
    require_bounded(bytes.as_slice())?;
    let decoded = super::decode_production_v1(bytes.as_slice())
        .map_err(CheckedOwnerObservationError::Decode)?;
    if decoded.record.state.is_open() {
        return Err(CheckedOwnerObservationError::NotTerminal);
    }
    super::archive::decode_archived(bytes.as_slice(), &decoded.record.merge_id)
        .map_err(|_| CheckedOwnerObservationError::InvalidTerminal)?;
    let owner = CheckedOwnerRecordObservation {
        version: CheckedOwnerRecordVersion::V1,
        exact_bytes: bytes.as_slice(),
        workspace_id: decoded.record.workspace_id,
        merge_id: decoded.record.merge_id,
        operation_id: decoded.record.operation_id,
    };
    if path.as_path().file_stem().and_then(|value| value.to_str()) != Some(owner.merge_id()) {
        return Err(CheckedOwnerObservationError::Identity);
    }
    Ok(CheckedArchiveSourceObservation { owner })
}

pub(crate) fn observe_checked_owner_v0(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    observe_checked_owner_v0_bytes(exact_bytes)
}

pub(crate) fn observe_checked_owner_v1(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    require_bounded(exact_bytes)?;
    let decoded =
        super::decode_production_v1(exact_bytes).map_err(CheckedOwnerObservationError::Decode)?;
    Ok(CheckedOwnerRecordObservation {
        version: CheckedOwnerRecordVersion::V1,
        exact_bytes,
        workspace_id: decoded.record.workspace_id,
        merge_id: decoded.record.merge_id,
        operation_id: decoded.record.operation_id,
    })
}

pub(crate) fn observe_checked_owner_v1_from_canonical(
    leaf: &CanonicalRecordLeaf,
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    let Some((path, bytes, _)) = leaf.exact() else {
        return Err(CheckedOwnerObservationError::Absent);
    };
    let observation = observe_checked_owner_v1(bytes.as_slice())?;
    if path.as_path().file_stem().and_then(|value| value.to_str()) != Some(observation.merge_id()) {
        return Err(CheckedOwnerObservationError::Identity);
    }
    Ok(observation)
}

fn require_bounded(exact_bytes: &[u8]) -> Result<(), CheckedOwnerObservationError> {
    if exact_bytes.is_empty() || exact_bytes.len() > MAX_CHECKED_OWNER_RECORD_BYTES {
        return Err(CheckedOwnerObservationError::Bounds);
    }
    Ok(())
}
