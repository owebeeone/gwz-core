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
#[cfg(test)]
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
#[cfg(test)]
use crate::workspace_ops::merge::ParticipantState;
#[cfg(test)]
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, ParticipantRollbackKindV1};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[cfg(test)]
use super::super::root::artifact_facts;

#[cfg(test)]
mod v1_rollback {
    use super::*;

    pub(in crate::workspace_ops::merge) fn preflight_v1_rollback<B: GitBackend>(
        backend: &B,
        root: &Path,
        record: &MergeOperationRecordV1,
    ) -> ModelResult<()> {
        if !record.operation_drift.is_empty() {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "operation drift prevents coordinated rollback entry",
            ));
        }
        validate_v1_baseline(record)?;
        for member_id in &record.selected_targets {
            let row = record.participants.get(member_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("selected rollback participant '{member_id}' is missing"),
                )
            })?;
            if member_id == "@root"
                && record.publication.as_ref().is_some_and(|publication| {
                    publication.candidate.is_some()
                        && publication.composition_commit.is_some()
                        && !publication.evidence_rolled_back
                })
            {
                // Publication evidence owns the root checkout first. Its complete
                // classifier is run by the caller before rollback entry; the root
                // participant becomes observable at its result commit only after
                // that evidence owner is durably completed.
                require_virtual_selected_root_after_evidence(backend, root, record, row)?;
                continue;
            }
            match row.state {
                ParticipantState::Conflicted => require_rollback_form(
                    crate::workspace_ops::merge::abort::observe_v1_participant_rollback(
                        backend,
                        root,
                        record,
                        member_id,
                        row,
                        ParticipantRollbackKindV1::AbortConflict,
                    )?,
                    member_id,
                    &row.path,
                )?,
                ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued => require_rollback_form(
                    crate::workspace_ops::merge::abort::observe_v1_participant_rollback(
                        backend,
                        root,
                        record,
                        member_id,
                        row,
                        ParticipantRollbackKindV1::ResetIntegrated,
                    )?,
                    member_id,
                    &row.path,
                )?,
                ParticipantState::Planned
                | ParticipantState::UpToDate
                | ParticipantState::Failed
                | ParticipantState::Unattempted => {
                    crate::workspace_ops::merge::abort::verify_v1_no_mutation_participant(
                        backend, root, record, member_id, row,
                    )?;
                }
                ParticipantState::Aborted | ParticipantState::RolledBack => {
                    if !exact_clean_before(backend, root, member_id, row)? {
                        return Err(member_preflight_error(
                            member_id,
                            &row.path,
                            "completed rollback participant no longer matches its before commit",
                        ));
                    }
                }
            }
        }
        if record.selected_targets.iter().any(|id| id == "@root") {
            for relative in [WORKSPACE_MANIFEST, artifact::LOCK_PATH] {
                if !matches!(
                    artifact_facts::observe(root, relative)?,
                    artifact_facts::RegularFileFact::Bytes(_)
                ) {
                    return Err(ModelError::new(
                        ErrorCode::MergeRecoveryRequired,
                        format!(
                            "selected-root artifact '{relative}' is not a canonical regular file"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn require_virtual_selected_root_after_evidence<B: GitBackend>(
        backend: &B,
        root: &Path,
        record: &MergeOperationRecordV1,
        row: &crate::workspace_ops::merge::MergeParticipantRecord,
    ) -> ModelResult<()> {
        use crate::workspace_ops::merge::model::v1::AcceptedRootBaseV1;

        super::super::evidence::preflight_v1_evidence(backend, root, record)?;
        let publication = record.publication.as_ref().ok_or_else(|| {
            member_preflight_error("@root", ".", "publication evidence disappeared")
        })?;
        let candidate = publication.candidate.as_ref().ok_or_else(|| {
            member_preflight_error("@root", ".", "publication candidate disappeared")
        })?;
        let result_commit = row.resulting_commit.as_deref().ok_or_else(|| {
            member_preflight_error(
                "@root",
                ".",
                "selected-root publication handoff has no participant result commit",
            )
        })?;
        let accepted = record.accepted_workspace.as_ref().ok_or_else(|| {
            member_preflight_error(
                "@root",
                ".",
                "selected-root publication handoff has no accepted root base",
            )
        })?;
        if !matches!(
            &accepted.root.base,
            AcceptedRootBaseV1::BornAttached { symbolic_branch, commit }
                if symbolic_branch == &candidate.root_branch && commit == result_commit
        ) {
            return Err(member_preflight_error(
                "@root",
                ".",
                "publication evidence does not retire to the selected-root participant result",
            ));
        }
        crate::workspace_ops::merge::root::selected_root_result_artifacts(backend, root, record)?;
        if backend.repository_state(root)? != crate::git::GitRepositoryState::Clean {
            return Err(member_preflight_error(
                "@root",
                ".",
                "selected-root publication handoff has a foreign native Git state",
            ));
        }
        let mut allowed = crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?
            .into_iter()
            .map(|file| file.path)
            .collect::<Vec<_>>();
        allowed.push(crate::workspace::RUNTIME_DIR.into());
        allowed.push(format!("{}/.tmp", crate::workspace::WORKSPACE_DIR));
        let manifest = crate::artifact::ManifestArtifact::from_yaml(
            record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
                member_preflight_error("@root", ".", "rollback baseline has no manifest bytes")
            })?,
        )?;
        allowed.extend(manifest.members.into_iter().map(|member| member.path));
        if !backend
            .checkout_matches_commit_except(root, result_commit, &allowed)
            .map_err(|mut error| {
                error.member_id = Some("@root".into());
                error.member_path = Some(".".into());
                error
            })?
        {
            return Err(member_preflight_error(
                "@root",
                ".",
                "selected-root checkout has unrelated dirt outside publication-owned paths",
            ));
        }
        Ok(())
    }

    fn require_rollback_form(
        observed: crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation,
        member_id: &str,
        path: &str,
    ) -> ModelResult<()> {
        if observed
            == crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::Ambiguous
        {
            Err(member_preflight_error(
                member_id,
                path,
                "participant is neither at the exact rollback before nor after state",
            ))
        } else {
            Ok(())
        }
    }

    fn exact_clean_before<B: GitBackend>(
        backend: &B,
        root: &Path,
        member_id: &str,
        row: &crate::workspace_ops::merge::MergeParticipantRecord,
    ) -> ModelResult<bool> {
        let path =
            crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?;
        let head = backend.head(&path)?;
        let status = backend.status(&path)?;
        Ok(
            backend.repository_state(&path)? == crate::git::GitRepositoryState::Clean
                && !head.is_detached
                && head.branch.as_deref() == Some(row.target_branch.as_str())
                && head.commit.as_deref() == Some(row.before_commit.as_str())
                && !status.is_dirty
                && status.unresolved == 0,
        )
    }

    fn validate_v1_baseline(record: &MergeOperationRecordV1) -> ModelResult<()> {
        let selected_root = record.selected_targets.iter().any(|id| id == "@root");
        let (manifest, lock) = (
            record.baseline.manifest_yaml.as_deref(),
            record.baseline.lock_yaml.as_deref(),
        );
        if selected_root && (manifest.is_none() || lock.is_none()) {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "selected-root rollback requires exact operation-baseline manifest and lock bytes",
            ));
        }
        for (relative, value, digest) in [
            (
                WORKSPACE_MANIFEST,
                manifest,
                record.baseline.manifest_sha256.as_str(),
            ),
            (
                artifact::LOCK_PATH,
                lock,
                record.baseline.lock_sha256.as_str(),
            ),
        ] {
            if let Some(value) = value
                && format!("{:x}", Sha256::digest(value.as_bytes())) != digest
            {
                return Err(ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("operation-baseline '{relative}' bytes do not match their digest"),
                ));
            }
        }
        Ok(())
    }

    fn member_preflight_error(member_id: &str, path: &str, detail: &str) -> ModelError {
        ModelError::new(ErrorCode::MergeRecoveryRequired, detail).with_member(member_id, path)
    }
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1_rollback::preflight_v1_rollback;

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
