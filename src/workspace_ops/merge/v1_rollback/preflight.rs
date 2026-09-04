//! The v1 reverse path's rollback preflight.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** Relocated out of
//! `merge/abort/preflight.rs`, whose `verify_baseline` / `restore_baseline`
//! half was the v0 abort engine and is deleted.

use super::super::root::artifact_facts;
use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, ParticipantRollbackKindV1};
use crate::workspace_ops::merge::{OperationDriftKind, ParticipantState};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(in crate::workspace_ops::merge) fn preflight_v1_rollback<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<()> {
    // The two root-candidate rows are diagnostics about the workspace root's
    // *future* metadata; they say nothing about whether this operation can be
    // reversed, so they must never be able to block or complicate an abort.
    // v0 encoded that by stripping both kinds unconditionally at the top of
    // abort, ahead of the evidence preflight, and again if the record passed
    // through `RollingBack` with evidence present
    // (`git show 57502e4:src/workspace_ops/merge/abort/mod.rs`, lines 116-137).
    // v1 keeps the diagnostic in the record rather than erasing it -- an
    // aborted merge should still be able to say why -- so the exemption lives
    // at the gates, here and in the preservation entry's twin.
    if record.operation_drift.iter().any(|drift| {
        !matches!(
            drift.kind,
            OperationDriftKind::RootCandidateMetadataInvalid
                | OperationDriftKind::RootCandidateStateChanged
        )
    }) {
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
                crate::workspace_ops::merge::v1_rollback::observe_v1_participant_rollback(
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
                crate::workspace_ops::merge::v1_rollback::observe_v1_participant_rollback(
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
                crate::workspace_ops::merge::v1_rollback::verify_v1_no_mutation_participant(
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

    super::evidence::preflight_v1_evidence(backend, root, record)?;
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
    let allowed = crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
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
    observed: crate::workspace_ops::merge::v1_rollback::V1ParticipantRollbackObservation,
    member_id: &str,
    path: &str,
) -> ModelResult<()> {
    if observed
        == crate::workspace_ops::merge::v1_rollback::V1ParticipantRollbackObservation::Ambiguous
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
