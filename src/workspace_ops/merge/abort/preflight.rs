use super::{
    super::{
        MergeOperationRecord, MergeStatusSnapshot,
        participant_semantics::rollback::{
            AbortPreflightDecision, RollbackClass, abort_preflight_decision, rollback_class,
        },
        status::PendingActionReconciliation,
    },
    reconciliation::pending_reconciliation,
};
use crate::artifact;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Default)]
pub(super) struct AbortPreflight {
    pub(super) no_op_targets: BTreeSet<String>,
    pub(super) pending: BTreeMap<String, PendingActionReconciliation>,
}

pub(super) fn preflight(snapshot: &MergeStatusSnapshot) -> ModelResult<AbortPreflight> {
    if let Some(drift) = snapshot.operation_drift.first() {
        return Err(ModelError::new(ErrorCode::MergeDrift, &drift.message));
    }
    let mut preflight = AbortPreflight::default();
    for target_id in &snapshot.record.selected_targets {
        let observation = snapshot.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge status is missing participant '{target_id}'"),
            )
        })?;
        let participant = snapshot.record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "status participant is missing",
            )
        })?;
        if participant.pending_action.is_some() {
            let reconciliation = pending_reconciliation(target_id, participant, observation)?;
            preflight.pending.insert(target_id.clone(), reconciliation);
            continue;
        }
        match abort_preflight_decision(snapshot.record.state, participant, observation) {
            AbortPreflightDecision::Reject => {
                let message = observation
                    .drift
                    .first()
                    .map(|drift| drift.message.clone())
                    .unwrap_or_else(|| {
                        "participant is not eligible for coordinated abort".to_owned()
                    });
                let mut error = ModelError::new(ErrorCode::MergeDrift, message);
                error.member_id = Some(target_id.clone());
                error.member_path = Some(participant.path.clone());
                return Err(error);
            }
            AbortPreflightDecision::Proceed => {}
            AbortPreflightDecision::AlreadyApplied => {
                preflight.no_op_targets.insert(target_id.clone());
            }
        }
    }
    Ok(preflight)
}

pub(super) fn verify_baseline(root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
    for (relative, expected) in [
        (artifact::LOCK_PATH, record.baseline.lock_sha256.as_str()),
        (WORKSPACE_MANIFEST, record.baseline.manifest_sha256.as_str()),
    ] {
        let actual = fs::read(root.join(relative))
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        if actual.as_deref() != Some(expected) {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("workspace artifact '{relative}' does not match the abort baseline"),
            ));
        }
    }
    Ok(())
}

pub(super) fn restore_baseline(root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
    let root_selected = record
        .selected_targets
        .iter()
        .any(|target| target == "@root");
    if !root_selected {
        return Ok(());
    }
    let root_participant = record.participants.get("@root");
    let participant = root_participant.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "selected root participant is missing from the merge record",
        )
    })?;
    if participant.target_kind != super::super::MergeTargetKind::Root
        || participant.path != "."
        || rollback_class(participant.state) != RollbackClass::Complete
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root participant is not at an exact rolled-back state",
        ));
    }
    let (Some(manifest_yaml), Some(lock_yaml)) = (
        record.baseline.manifest_yaml.as_deref(),
        record.baseline.lock_yaml.as_deref(),
    ) else {
        if record.baseline.manifest_yaml.is_none() && record.baseline.lock_yaml.is_none() {
            return Ok(());
        }
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "merge baseline contains only one of the exact workspace artifacts",
        ));
    };
    let manifest = artifact::ManifestArtifact::from_yaml(manifest_yaml)?;
    let lock = artifact::LockArtifact::from_yaml(lock_yaml)?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "exact merge baseline identifies a different workspace",
        ));
    }
    for (contents, expected, relative) in [
        (
            manifest_yaml,
            record.baseline.manifest_sha256.as_str(),
            WORKSPACE_MANIFEST,
        ),
        (
            lock_yaml,
            record.baseline.lock_sha256.as_str(),
            artifact::LOCK_PATH,
        ),
    ] {
        if format!("{:x}", Sha256::digest(contents.as_bytes())) != expected {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("exact merge baseline for '{relative}' does not match its digest"),
            ));
        }
    }
    artifact::write_atomic(&root.join(WORKSPACE_MANIFEST), manifest_yaml)?;
    artifact::write_atomic(&root.join(artifact::LOCK_PATH), lock_yaml)
}
