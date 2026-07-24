use std::path::Path;

use crate::git::{GitBackend, GitRepositoryState};
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
    if !pending_inputs_match_participant(pending, participant) {
        let reason = "pending action inputs do not match the frozen participant record";
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
    let prepared = match super::super::pending::decode_durable_prepared_action(pending) {
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
    let drift = classify_participant(target_id, participant, live).drift;
    let exact_branch = live.branch.as_deref() == Some(pending.target_branch.as_str())
        && live.head == live.target_ref;
    let clean = live.repository_state == GitRepositoryState::Clean
        && !live.status.is_dirty
        && live.missing_objects.is_empty();

    if exact_branch && clean {
        let live_commit = live.head.as_deref();
        match pending.kind {
            super::super::PendingMergeActionKind::VerifyUpToDate
                if live_commit == Some(pending.before_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            super::super::PendingMergeActionKind::FastForward
                if live_commit == Some(pending.source_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::Completed {
                    resulting_commit: pending.source_commit.clone(),
                });
            }
            super::super::PendingMergeActionKind::FastForward
                if live_commit == Some(pending.before_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            super::super::PendingMergeActionKind::TrueMerge
                if live_commit == Some(pending.before_commit.as_str())
                    && member_result(
                        backend.validate_prepared_merge_upstream_state(
                            path,
                            &pending.target_branch,
                            &pending.before_commit,
                            &pending.source_commit,
                            match &prepared {
                                super::super::pending::DurablePreparedAction::Merge(prepared) => {
                                    prepared
                                }
                                super::super::pending::DurablePreparedAction::Resolution(_) => {
                                    unreachable!(
                                        "true-merge pending action decoded as a resolution"
                                    )
                                }
                            },
                        ),
                        target_id,
                        &participant.path,
                    )
                    .is_ok() =>
            {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            super::super::PendingMergeActionKind::TrueMerge
            | super::super::PendingMergeActionKind::ResolveConflict => {
                if let (
                    Some(commit),
                    super::super::pending::DurablePreparedAction::Merge(
                        crate::git::GitPreparedMerge::Commit(prepared),
                    )
                    | super::super::pending::DurablePreparedAction::Resolution(prepared),
                ) = (live_commit, &prepared)
                    && member_result(
                        backend.commit_matches_prepared_merge(
                            path,
                            commit,
                            &pending.before_commit,
                            &pending.source_commit,
                            &pending.commit_message,
                            prepared,
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
            _ => {}
        }
    }

    let native_matches = exact_branch
        && live.head.as_deref() == Some(pending.before_commit.as_str())
        && live.repository_state == GitRepositoryState::Merge
        && live.missing_objects.is_empty()
        && live
            .merge_state
            .as_ref()
            .is_some_and(|state| state.merge_head == pending.source_commit);
    if native_matches {
        let require_resolved =
            pending.kind == super::super::PendingMergeActionKind::ResolveConflict;
        let native_intent_matches = require_resolved
            || (pending.kind == super::super::PendingMergeActionKind::TrueMerge
                && pending.expected_result
                    == Some(super::super::PendingMergeExpectedResult::ExpectedConflict));
        let native_state_valid = match &prepared {
            super::super::pending::DurablePreparedAction::Resolution(prepared)
                if require_resolved =>
            {
                backend
                    .validate_prepared_merge_resolution_state(
                        path,
                        &pending.target_branch,
                        &pending.before_commit,
                        &pending.source_commit,
                        prepared,
                    )
                    .is_ok()
            }
            super::super::pending::DurablePreparedAction::Merge(
                crate::git::GitPreparedMerge::ExpectedConflict,
            ) if !require_resolved => backend
                .validate_merge_recovery_state(
                    path,
                    &pending.before_commit,
                    &pending.source_commit,
                    false,
                )
                .is_ok(),
            _ => false,
        };
        if native_intent_matches && native_state_valid {
            if require_resolved {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            return Ok(PendingActionReconciliation::ExpectedConflict {
                conflict_paths: live
                    .merge_state
                    .as_ref()
                    .map(|state| state.conflict_paths.clone())
                    .unwrap_or_default(),
            });
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

pub(super) fn pending_inputs_match_participant(
    pending: &super::super::PendingMergeAction,
    participant: &MergeParticipantRecord,
) -> bool {
    pending.target_branch == participant.target_branch
        && pending.before_commit == participant.before_commit
        && pending.source_commit == participant.source_commit
        && pending.commit_message == participant.commit_message
}
