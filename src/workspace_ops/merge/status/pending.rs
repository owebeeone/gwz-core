use std::path::Path;

use crate::git::{GitBackend, GitPreparedMerge, GitRepositoryState};
use crate::model::ModelResult;

use super::super::{MergeParticipantRecord, ParticipantDrift, ParticipantDriftKind};
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingActionReconciliation {
    NotStarted,
    ExpectedConflict {
        conflict_paths: Vec<String>,
    },
    Completed {
        resulting_commit: String,
    },
    Ambiguous {
        reason: String,
        drift: Vec<ParticipantDrift>,
    },
}

/// Reconcile a durable participant action against live Git state without
/// writing either the repository or the operation record.
pub(crate) fn reconcile_pending_action<B: GitBackend>(
    backend: &B,
    root: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<PendingActionReconciliation> {
    let path = validated_participant_path(root, target_id, participant)?;
    if !path.is_dir() || !member_result(backend.is_repository(&path), target_id, &participant.path)?
    {
        return Ok(PendingActionReconciliation::Ambiguous {
            reason: "recorded participant repository is missing".to_owned(),
            drift: missing_observation(target_id, participant).drift,
        });
    }
    let live = read_live_participant(backend, &path, target_id, participant)?;
    reconcile_pending_action_from_live(backend, &path, target_id, participant, &live)
}

pub(super) fn reconcile_pending_action_from_live<B: GitBackend>(
    backend: &B,
    path: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
) -> ModelResult<PendingActionReconciliation> {
    let Some(pending) = participant.pending_action.as_ref() else {
        return Ok(PendingActionReconciliation::Ambiguous {
            reason: "participant has no pending action to reconcile".to_owned(),
            drift: Vec::new(),
        });
    };
    let prepared = match super::super::integration::decode_for_participant(pending, participant) {
        Ok(prepared) => prepared,
        Err(reason) => {
            return Ok(PendingActionReconciliation::Ambiguous {
                reason: reason.to_owned(),
                drift: vec![participant_drift(
                    ParticipantDriftKind::PendingActionAmbiguous,
                    target_id,
                    participant,
                    live,
                    reason,
                )],
            });
        }
    };
    let intent = &prepared.intent;
    let drift = classify_participant(target_id, participant, live).drift;
    let exact_branch = live.branch.as_deref() == Some(intent.target_branch.as_str())
        && live.head == live.target_ref;
    let clean = live.repository_state == GitRepositoryState::Clean
        && !live.status.is_dirty
        && live.missing_objects.is_empty();

    if exact_branch && clean {
        let live_commit = live.head.as_deref();
        use super::super::integration::PreparedIntegrationAction as Action;
        match &prepared.action {
            Action::VerifyUpToDate => {
                if live_commit == Some(intent.before_commit.as_str()) {
                    return Ok(PendingActionReconciliation::NotStarted);
                }
            }
            Action::FastForward => {
                if live_commit == Some(intent.source_commit.as_str()) {
                    return Ok(PendingActionReconciliation::Completed {
                        resulting_commit: intent.source_commit.clone(),
                    });
                }
                if live_commit == Some(intent.before_commit.as_str()) {
                    return Ok(PendingActionReconciliation::NotStarted);
                }
            }
            Action::TrueMergeExpectedConflict => {
                if live_commit == Some(intent.before_commit.as_str())
                    && member_result(
                        backend.validate_prepared_merge_upstream_state(
                            path,
                            &intent.target_branch,
                            &intent.before_commit,
                            &intent.source_commit,
                            &GitPreparedMerge::ExpectedConflict,
                        ),
                        target_id,
                        &participant.path,
                    )
                    .is_ok()
                {
                    return Ok(PendingActionReconciliation::NotStarted);
                }
            }
            Action::TrueMergeCommit(spec) => {
                if live_commit == Some(intent.before_commit.as_str())
                    && member_result(
                        backend.validate_prepared_merge_upstream_state(
                            path,
                            &intent.target_branch,
                            &intent.before_commit,
                            &intent.source_commit,
                            &GitPreparedMerge::Commit(spec.clone()),
                        ),
                        target_id,
                        &participant.path,
                    )
                    .is_ok()
                {
                    return Ok(PendingActionReconciliation::NotStarted);
                }
                if let Some(commit) = live_commit
                    && member_result(
                        backend.commit_matches_prepared_merge(
                            path,
                            commit,
                            &intent.before_commit,
                            &intent.source_commit,
                            &intent.commit_message,
                            spec,
                        ),
                        target_id,
                        &participant.path,
                    )?
                {
                    return Ok(PendingActionReconciliation::Completed {
                        resulting_commit: commit.to_owned(),
                    });
                }
            }
            Action::ResolveConflict(spec) => {
                if let Some(commit) = live_commit
                    && member_result(
                        backend.commit_matches_prepared_merge(
                            path,
                            commit,
                            &intent.before_commit,
                            &intent.source_commit,
                            &intent.commit_message,
                            spec,
                        ),
                        target_id,
                        &participant.path,
                    )?
                {
                    return Ok(PendingActionReconciliation::Completed {
                        resulting_commit: commit.to_owned(),
                    });
                }
            }
        }
    }

    let native_matches = exact_branch
        && live.head.as_deref() == Some(intent.before_commit.as_str())
        && live.repository_state == GitRepositoryState::Merge
        && live.missing_objects.is_empty()
        && live
            .merge_state
            .as_ref()
            .is_some_and(|state| state.merge_head == intent.source_commit);
    if native_matches {
        use super::super::integration::PreparedIntegrationAction as Action;
        match &prepared.action {
            Action::ResolveConflict(spec) => {
                if backend
                    .validate_prepared_merge_resolution_state(
                        path,
                        &intent.target_branch,
                        &intent.before_commit,
                        &intent.source_commit,
                        spec,
                    )
                    .is_ok()
                {
                    return Ok(PendingActionReconciliation::NotStarted);
                }
            }
            Action::TrueMergeExpectedConflict => {
                if backend
                    .validate_merge_recovery_state(
                        path,
                        &intent.before_commit,
                        &intent.source_commit,
                        false,
                    )
                    .is_ok()
                {
                    return Ok(PendingActionReconciliation::ExpectedConflict {
                        conflict_paths: live
                            .merge_state
                            .as_ref()
                            .map(|state| state.conflict_paths.clone())
                            .unwrap_or_default(),
                    });
                }
            }
            Action::VerifyUpToDate | Action::FastForward | Action::TrueMergeCommit(_) => {}
        }
    }

    let reason = "live repository does not exactly match a pending-action recovery point";
    let mut drift = drift;
    drift.push(participant_drift(
        ParticipantDriftKind::PendingActionAmbiguous,
        target_id,
        participant,
        live,
        reason,
    ));
    Ok(PendingActionReconciliation::Ambiguous {
        reason: reason.to_owned(),
        drift,
    })
}
