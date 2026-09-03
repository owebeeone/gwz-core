//! The v1 reverse path's per-participant rollback observation and execution.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** Relocated out of
//! `merge/abort/participants.rs`, whose `rollback_participants` half was the
//! v0 abort engine and is deleted.

use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::{MergeParticipantRecord, ParticipantState};
use crate::workspace_ops::merge::model::v1::ParticipantRollbackKindV1;
use std::path::Path;

use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum V1ParticipantRollbackObservation {
    Before,
    After,
    Ambiguous,
}

pub(in crate::workspace_ops::merge) fn observe_v1_participant_rollback<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    member_id: &str,
    row: &MergeParticipantRecord,
    action: ParticipantRollbackKindV1,
) -> ModelResult<V1ParticipantRollbackObservation> {
    let path =
        crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?;
    let exclusions = checkout_exclusions(record, member_id, row)?;
    let after = clean_checkout(
        backend,
        &path,
        &row.target_branch,
        &row.before_commit,
        &exclusions,
    )
    .map_err(|error| attach(error, member_id, &row.path))?;
    let before = match action {
        ParticipantRollbackKindV1::AbortConflict => exact_native_conflict(backend, &path, row)?,
        ParticipantRollbackKindV1::ResetIntegrated => {
            let Some(result) = row.resulting_commit.as_deref() else {
                return Err(member_error(
                    member_id,
                    row,
                    "integrated rollback has no recorded result commit",
                ));
            };
            clean_checkout(backend, &path, &row.target_branch, result, &exclusions)
                .map_err(|error| attach(error, member_id, &row.path))?
        }
    };
    Ok(match (before, after) {
        (true, false) => V1ParticipantRollbackObservation::Before,
        (false, true) => V1ParticipantRollbackObservation::After,
        _ => V1ParticipantRollbackObservation::Ambiguous,
    })
}

pub(in crate::workspace_ops::merge) fn verify_v1_no_mutation_participant<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    member_id: &str,
    row: &MergeParticipantRecord,
) -> ModelResult<()> {
    let exclusions = checkout_exclusions(record, member_id, row)?;
    if !matches!(
        row.state,
        ParticipantState::Planned
            | ParticipantState::UpToDate
            | ParticipantState::Failed
            | ParticipantState::Unattempted
    ) || !clean_checkout(
        backend,
        &crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?,
        &row.target_branch,
        &row.before_commit,
        &exclusions,
    )
    .map_err(|error| attach(error, member_id, &row.path))?
    {
        return Err(member_error(
            member_id,
            row,
            "no-mutation rollback participant does not exactly match its recorded checkout",
        ));
    }
    Ok(())
}

pub(in crate::workspace_ops::merge) fn terminal_v1_participant_is_exact<B: GitBackend>(
    backend: &B,
    root: &Path,
    member_id: &str,
    row: &MergeParticipantRecord,
    overlay: &crate::git::GitCheckoutOverlay,
) -> ModelResult<bool> {
    if !matches!(
        row.state,
        ParticipantState::Aborted | ParticipantState::RolledBack
    ) {
        return Ok(false);
    }
    let path =
        crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?;
    if backend.repository_state(&path)? != GitRepositoryState::Clean {
        return Ok(false);
    }
    let head = backend.head(&path)?;
    let target = backend.read_ref(&path, &format!("refs/heads/{}", row.target_branch))?;
    Ok(!head.is_detached
        && head.branch.as_deref() == Some(row.target_branch.as_str())
        && head.commit.as_deref() == Some(row.before_commit.as_str())
        && target.as_deref() == Some(row.before_commit.as_str())
        && backend.checkout_matches_commit_with_overlay(&path, &row.before_commit, overlay)?)
}

pub(in crate::workspace_ops::merge) fn execute_v1_participant_rollback<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    member_id: &str,
    row: &MergeParticipantRecord,
    action: ParticipantRollbackKindV1,
) -> ModelResult<()> {
    if observe_v1_participant_rollback(backend, root, record, member_id, row, action)?
        != V1ParticipantRollbackObservation::Before
    {
        return Err(member_error(
            member_id,
            row,
            "participant rollback is not at its exact before state",
        ));
    }
    let path =
        crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?;
    match action {
        ParticipantRollbackKindV1::AbortConflict => backend.abort_merge(
            &path,
            &row.before_commit,
            row.expected_merge_head
                .as_deref()
                .unwrap_or(&row.source_commit),
        ),
        ParticipantRollbackKindV1::ResetIntegrated => backend
            .set_branch_target_checked(
                &path,
                &row.target_branch,
                row.resulting_commit.as_deref().ok_or_else(|| {
                    member_error(member_id, row, "integrated rollback has no result commit")
                })?,
                &row.before_commit,
            )
            .map(|_| ()),
    }
    .map_err(|error| attach(error, member_id, &row.path))
}

fn clean_checkout<B: GitBackend>(
    backend: &B,
    path: &Path,
    branch: &str,
    commit: &str,
    exclusions: &[String],
) -> ModelResult<bool> {
    if backend.repository_state(path)? != GitRepositoryState::Clean {
        return Ok(false);
    }
    let head = backend.head(path)?;
    let target = backend.read_ref(path, &format!("refs/heads/{branch}"))?;
    Ok(!head.is_detached
        && head.branch.as_deref() == Some(branch)
        && head.commit.as_deref() == Some(commit)
        && target.as_deref() == Some(commit)
        && backend.checkout_matches_commit_except(path, commit, exclusions)?)
}

fn checkout_exclusions(
    record: &MergeOperationRecordV1,
    member_id: &str,
    _row: &MergeParticipantRecord,
) -> ModelResult<Vec<String>> {
    if member_id != "@root" {
        return Ok(Vec::new());
    }
    let mut paths = if record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_some()
    {
        crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?
            .into_iter()
            .map(|file| file.path)
            .collect()
    } else {
        Vec::new()
    };
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn exact_native_conflict<B: GitBackend>(
    backend: &B,
    path: &Path,
    row: &MergeParticipantRecord,
) -> ModelResult<bool> {
    let merge_head = row
        .expected_merge_head
        .as_deref()
        .unwrap_or(&row.source_commit);
    if let Err(error) =
        backend.validate_merge_recovery_state(path, &row.before_commit, merge_head, false)
    {
        return if semantic_mismatch(&error) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let head = backend.head(path)?;
    let target = backend.read_ref(path, &format!("refs/heads/{}", row.target_branch))?;
    let native = backend.merge_state(path)?;
    let snapshot = backend.merge_conflict_snapshot(path, &row.before_commit, merge_head)?;
    let expected_snapshot = row
        .conflict_snapshot
        .iter()
        .map(|item| (item.path.as_str(), item.sha256.as_str()))
        .collect::<Vec<_>>();
    let observed_snapshot = snapshot
        .files
        .iter()
        .map(|item| (item.path.as_str(), item.sha256.as_str()))
        .collect::<Vec<_>>();
    Ok(!head.is_detached
        && head.branch.as_deref() == Some(row.target_branch.as_str())
        && head.commit.as_deref() == Some(row.before_commit.as_str())
        && target.as_deref() == Some(row.before_commit.as_str())
        && native.as_ref().is_some_and(|state| {
            state.merge_head == merge_head && state.conflict_paths == row.conflict_paths
        })
        && observed_snapshot == expected_snapshot)
}

fn member_error(
    member_id: &str,
    row: &MergeParticipantRecord,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into())
        .with_member(member_id, &row.path)
}

fn attach(mut error: ModelError, member_id: &str, path: &str) -> ModelError {
    error.member_id = Some(member_id.into());
    error.member_path = Some(path.into());
    error
}

fn semantic_mismatch(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::DirtyMember
            | ErrorCode::MergeDrift
            | ErrorCode::MergeRecoveryRequired
            | ErrorCode::RecoveryEvidenceMismatch
    )
}
