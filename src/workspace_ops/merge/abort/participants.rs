use super::{
    super::{
        MergeOperationRecord, MergeStore,
        participant_semantics::rollback::{RollbackGitAction, participant_rollback_decision},
    },
    preflight::AbortPreflight,
    runtime::AbortRuntime,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;
use std::path::Path;

pub(super) fn rollback_participants<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    preflight: &AbortPreflight,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for target_id in record.selected_targets.clone().into_iter().rev() {
        let (prior, next) = {
            let participant = record.participants.get(&target_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("merge record is missing participant '{target_id}'"),
                )
            })?;
            let path =
                super::super::status::validated_participant_path(root, &target_id, participant)?;
            let prior = participant.state;
            let decision =
                participant_rollback_decision(prior, preflight.no_op_targets.contains(&target_id));
            let Some(next) = decision.terminal_state else {
                continue;
            };
            emitter.member_started(&target_id, &participant.path);
            match decision.git_action {
                RollbackGitAction::None => {}
                RollbackGitAction::AbortConflict => runtime.abort_merge(
                    &path,
                    &participant.before_commit,
                    participant
                        .expected_merge_head
                        .as_deref()
                        .unwrap_or(&participant.source_commit),
                )?,
                RollbackGitAction::ResetIntegrated => runtime.reset_branch(
                    &path,
                    &participant.target_branch,
                    participant.resulting_commit.as_deref().ok_or_else(|| {
                        ModelError::new(
                            ErrorCode::MergeRecordUnreadable,
                            format!("merge participant '{target_id}' has no resulting commit"),
                        )
                    })?,
                    &participant.before_commit,
                )?,
            }
            (prior, next)
        };
        record.participants.get_mut(&target_id).unwrap().state = prior.transition(next)?;
        super::super::persist_merge_record(store, root, record, emitter)?;
        super::super::emit_merge_member_finished(emitter, record, &target_id)?;
    }
    Ok(())
}

#[cfg(test)]
use crate::git::{GitBackend, GitRepositoryState};
#[cfg(test)]
use crate::workspace_ops::merge::model::v1::ParticipantRollbackKindV1;
#[cfg(test)]
use crate::workspace_ops::merge::{MergeParticipantRecord, ParticipantState};

#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1_rollback::{
    V1ParticipantRollbackObservation, execute_v1_participant_rollback,
    observe_v1_participant_rollback, verify_v1_no_mutation_participant,
};

#[cfg(test)]
mod v1_rollback {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(in crate::workspace_ops::merge) enum V1ParticipantRollbackObservation {
        Before,
        After,
        Ambiguous,
    }

    pub(in crate::workspace_ops::merge) fn observe_v1_participant_rollback<B: GitBackend>(
        backend: &B,
        root: &Path,
        member_id: &str,
        row: &MergeParticipantRecord,
        action: ParticipantRollbackKindV1,
    ) -> ModelResult<V1ParticipantRollbackObservation> {
        let path =
            crate::workspace_ops::merge::status::validated_participant_path(root, member_id, row)?;
        let after = clean_checkout(backend, &path, &row.target_branch, &row.before_commit)?;
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
                clean_checkout(backend, &path, &row.target_branch, result)?
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
        member_id: &str,
        row: &MergeParticipantRecord,
    ) -> ModelResult<()> {
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
        )? {
            return Err(member_error(
                member_id,
                row,
                "no-mutation rollback participant does not exactly match its recorded checkout",
            ));
        }
        Ok(())
    }

    pub(in crate::workspace_ops::merge) fn execute_v1_participant_rollback<B: GitBackend>(
        backend: &B,
        root: &Path,
        member_id: &str,
        row: &MergeParticipantRecord,
        action: ParticipantRollbackKindV1,
    ) -> ModelResult<()> {
        if observe_v1_participant_rollback(backend, root, member_id, row, action)?
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
    ) -> ModelResult<bool> {
        if backend.repository_state(path)? != GitRepositoryState::Clean {
            return Ok(false);
        }
        let head = backend.head(path)?;
        let status = backend.status(path)?;
        let target = backend.read_ref(path, &format!("refs/heads/{branch}"))?;
        Ok(!head.is_detached
            && head.branch.as_deref() == Some(branch)
            && head.commit.as_deref() == Some(commit)
            && target.as_deref() == Some(commit)
            && !status.is_dirty
            && status.unresolved == 0)
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
}
