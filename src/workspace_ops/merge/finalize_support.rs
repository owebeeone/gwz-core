use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;

use super::marker::VerifiedMergeParticipant;
use super::{
    MergeOperationRecord, MergeStore, OperationDrift, OperationDriftKind, OperationState,
    PublicationCandidate, PublicationProgress, PublicationStep,
};

pub(super) fn verified_participants<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<Option<Vec<VerifiedMergeParticipant>>> {
    let mut observed_record = record.clone();
    clear_root_recovery_drift(&mut observed_record);
    let snapshot = super::status::snapshot_status(backend, root, observed_record)?;
    let root_finalization_exact = super::root::root_finalization_is_exact(backend, root, record)?;
    if !snapshot.operation_drift.is_empty()
        || snapshot
            .participants
            .iter()
            .any(|(target_id, participant)| {
                !(participant.drift.is_empty() || target_id == "@root" && root_finalization_exact)
            })
    {
        return Ok(None);
    }
    if record
        .operation_drift
        .iter()
        .any(|drift| drift.kind == OperationDriftKind::RootCandidateStateChanged)
    {
        clear_root_drift(record);
        super::persist_merge_record(store, root, record, emitter)?;
    }
    record
        .selected_targets
        .iter()
        .map(|target_id| {
            let durable = record
                .participants
                .get(target_id)
                .ok_or_else(|| unreadable(format!("merge participant '{target_id}' is missing")))?;
            let observed = snapshot
                .participants
                .get(target_id)
                .ok_or_else(|| unreadable(format!("merge observation '{target_id}' is missing")))?;
            let resulting_commit = if target_id == "@root" && root_finalization_exact {
                durable.resulting_commit.clone()
            } else {
                observed.live_commit.clone()
            }
            .ok_or_else(|| {
                recovery(format!(
                    "verified participant '{target_id}' has no live commit"
                ))
            })?;
            Ok(VerifiedMergeParticipant {
                target_id: target_id.clone(),
                target_branch: durable.target_branch.clone(),
                resulting_commit,
            })
        })
        .collect::<ModelResult<Vec<_>>>()
        .map(Some)
}

pub(super) fn set_step<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    next: PublicationStep,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let publication = progress_mut(record)?;
    if publication.step < next {
        publication.step = publication.step.transition(next)?;
        super::persist_merge_record(store, root, record, emitter)?;
    }
    Ok(())
}

pub(super) fn complete_and_archive<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    super::persist_operation_transition(store, root, record, OperationState::Completed, emitter)?;
    super::archive_merge_record(store, root, &record.merge_id, emitter)
}

pub(super) fn block_root<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
    message: &str,
) -> ModelResult<bool> {
    clear_root_drift(record);
    record.operation_drift.push(OperationDrift {
        kind: OperationDriftKind::RootCandidateStateChanged,
        message: message.to_owned(),
    });
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(false)
}

pub(super) fn record_root_metadata_invalid<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
    message: &str,
) -> ModelResult<()> {
    clear_root_metadata_drift(record);
    record.operation_drift.push(OperationDrift {
        kind: OperationDriftKind::RootCandidateMetadataInvalid,
        message: message.to_owned(),
    });
    super::persist_merge_record(store, root, record, emitter)
}

pub(super) fn clear_root_drift(record: &mut MergeOperationRecord) {
    record
        .operation_drift
        .retain(|drift| drift.kind != OperationDriftKind::RootCandidateStateChanged);
}

pub(super) fn clear_root_metadata_drift(record: &mut MergeOperationRecord) {
    record
        .operation_drift
        .retain(|drift| drift.kind != OperationDriftKind::RootCandidateMetadataInvalid);
}

fn clear_root_recovery_drift(record: &mut MergeOperationRecord) {
    clear_root_drift(record);
    clear_root_metadata_drift(record);
}

pub(super) fn progress(record: &MergeOperationRecord) -> ModelResult<&PublicationProgress> {
    record
        .publication
        .as_ref()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

pub(super) fn progress_mut(
    record: &mut MergeOperationRecord,
) -> ModelResult<&mut PublicationProgress> {
    record
        .publication
        .as_mut()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

pub(super) fn candidate(record: &MergeOperationRecord) -> ModelResult<&PublicationCandidate> {
    progress(record)?
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))
}

pub(super) fn file_sha256(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| sha256(&bytes))
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

pub(super) fn recovery(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

pub(super) fn root_drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message).with_member("@root", ".")
}
