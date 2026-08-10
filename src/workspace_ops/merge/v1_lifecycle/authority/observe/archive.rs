use std::path::{Path, PathBuf};

use super::super::super::checked::RecordDigest;
use super::super::*;
use crate::model::ModelResult;
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::record_wire::{
    CanonicalRecordKind, CanonicalRecordLeaf, acquire_canonical_merge_locations,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckedArchivePhysical {
    DestinationAbsent,
    ExactTerminalCopy,
}

/// Opaque authority-local result of observing both canonical archive leaves.
/// Construction is deliberately private: callers can request an archive
/// observation, but cannot turn caller-selected bytes into lifecycle authority.
struct CheckedArchiveObservation {
    workspace_id: String,
    merge_id: String,
    operation_id: String,
    terminal_state: OperationState,
    source_path: PathBuf,
    source_digest: RecordDigest,
    destination_path: PathBuf,
    destination_digest: Option<[u8; 32]>,
    physical: CheckedArchivePhysical,
}

impl CheckedArchiveObservation {
    fn acquire(current: &StoredV1Record, request: &BoundObservationRequest) -> ModelResult<Self> {
        let record = current.record();
        if request.lifecycle() != V1LifecycleRequest::Archive
            || request.kind() != &ObservationKind::Archive
            || !request.matches(current, V1LifecycleRequest::Archive)
            || !matches!(
                record.state,
                OperationState::Completed | OperationState::Aborted
            )
        {
            return Err(authority_error(
                "archive observation requires the exact bound terminal archive request",
            ));
        }

        let locations =
            acquire_canonical_merge_locations(current.location().root(), &record.merge_id)?;
        let (source_path, source_bytes, source_digest) =
            locations.open().exact().ok_or_else(|| {
                authority_error("checked archive source is absent from its canonical open path")
            })?;
        require_kind_and_path(
            source_path.kind(),
            CanonicalRecordKind::Open,
            source_path.as_path(),
            current.location().path(),
        )?;
        let reopened = StoredV1Record::from_open_bytes(
            current.location().root(),
            source_path.as_path(),
            source_bytes.as_slice(),
        )?;
        if !current.same_source_as(&reopened) {
            return Err(authority_error("checked archive source lineage changed"));
        }

        let destination_path = current
            .location()
            .root()
            .join(".gwz/merge/done")
            .join(format!("{}.yaml", record.merge_id));
        let (physical, destination_digest) = match locations.archived() {
            CanonicalRecordLeaf::Absent => (CheckedArchivePhysical::DestinationAbsent, None),
            CanonicalRecordLeaf::Exact {
                path,
                bytes,
                digest,
            } => {
                require_kind_and_path(
                    path.kind(),
                    CanonicalRecordKind::Archived,
                    path.as_path(),
                    &destination_path,
                )?;
                crate::workspace_ops::merge::record_wire::decode_archived_for_r3_tests(
                    bytes.as_slice(),
                    &record.merge_id,
                )?;
                if bytes.as_slice() != source_bytes.as_slice() || digest != &source_digest {
                    return Err(authority_error(
                        "checked archive destination is not an exact terminal source copy",
                    ));
                }
                (
                    CheckedArchivePhysical::ExactTerminalCopy,
                    Some(*digest.as_bytes()),
                )
            }
        };

        Ok(Self {
            workspace_id: record.workspace_id.clone(),
            merge_id: record.merge_id.clone(),
            operation_id: record.operation_id.clone(),
            terminal_state: record.state,
            source_path: source_path.as_path().to_owned(),
            source_digest: current.source_digest(),
            destination_path,
            destination_digest,
            physical,
        })
    }

    fn still_matches(&self, current: &StoredV1Record, request: &BoundObservationRequest) -> bool {
        let record = current.record();
        request.lifecycle() == V1LifecycleRequest::Archive
            && request.kind() == &ObservationKind::Archive
            && request.matches(current, V1LifecycleRequest::Archive)
            && self.workspace_id == record.workspace_id
            && self.merge_id == record.merge_id
            && self.operation_id == record.operation_id
            && self.terminal_state == record.state
            && self.source_path == current.location().path()
            && self.source_digest == current.source_digest()
            && self.destination_path
                == current
                    .location()
                    .root()
                    .join(".gwz/merge/done")
                    .join(format!("{}.yaml", record.merge_id))
            && matches!(
                (self.physical, self.destination_digest),
                (CheckedArchivePhysical::DestinationAbsent, None)
                    | (CheckedArchivePhysical::ExactTerminalCopy, Some(_))
            )
    }

    fn into_fact(self) -> ExactObservationFact {
        match self.physical {
            CheckedArchivePhysical::DestinationAbsent => {
                ExactObservationFact::NotStarted(NotStartedObservation::Archive)
            }
            CheckedArchivePhysical::ExactTerminalCopy => {
                ExactObservationFact::Completed(CompletedObservation::Archive)
            }
        }
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn observe(
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    let checked = CheckedArchiveObservation::acquire(current, request)?;
    if !checked.still_matches(current, request) {
        return Err(authority_error(
            "checked archive observation does not match its exact source or request",
        ));
    }
    BoundExactObservation::issue(current, request, checked.into_fact())
}

fn require_kind_and_path(
    actual_kind: CanonicalRecordKind,
    expected_kind: CanonicalRecordKind,
    actual_path: &Path,
    expected_path: &Path,
) -> ModelResult<()> {
    if actual_kind == expected_kind && actual_path == expected_path {
        Ok(())
    } else {
        Err(authority_error(
            "canonical archive observation returned the wrong record location",
        ))
    }
}
