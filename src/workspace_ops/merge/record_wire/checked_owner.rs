//! Opaque, bounded observations used to derive checked-artifact owners.

use super::{CanonicalRecordLeaf, RecordDecodeError, decode_production_v0};

pub(crate) const MAX_CHECKED_OWNER_RECORD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckedOwnerRecordVersion {
    V0,
    #[cfg(test)]
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

#[derive(Debug)]
pub(crate) enum CheckedOwnerObservationError {
    Absent,
    Bounds,
    Identity,
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

fn observe_checked_owner_v0_bytes(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    require_bounded(exact_bytes)?;
    let decoded =
        decode_production_v0(exact_bytes).map_err(CheckedOwnerObservationError::Decode)?;
    let (_, _, record) = decoded.into_production_parts();
    Ok(CheckedOwnerRecordObservation {
        version: CheckedOwnerRecordVersion::V0,
        exact_bytes,
        workspace_id: record.workspace_id,
        merge_id: record.merge_id,
        operation_id: record.operation_id,
    })
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

#[cfg(test)]
pub(crate) fn observe_checked_owner_v0(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    observe_checked_owner_v0_bytes(exact_bytes)
}

#[cfg(test)]
pub(crate) fn observe_checked_owner_v1(
    exact_bytes: &[u8],
) -> Result<CheckedOwnerRecordObservation<'_>, CheckedOwnerObservationError> {
    require_bounded(exact_bytes)?;
    let decoded =
        super::decode_v1_for_r3_tests(exact_bytes).map_err(CheckedOwnerObservationError::Decode)?;
    Ok(CheckedOwnerRecordObservation {
        version: CheckedOwnerRecordVersion::V1,
        exact_bytes,
        workspace_id: decoded.record.workspace_id,
        merge_id: decoded.record.merge_id,
        operation_id: decoded.record.operation_id,
    })
}

#[cfg(test)]
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
