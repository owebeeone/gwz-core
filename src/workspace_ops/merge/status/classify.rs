use std::path::Path;

use crate::git::{GitBackend, GitNativeMergeState, GitRepositoryState, GitStatus};
use crate::model::ModelResult;

use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, ParticipantDriftKind, ParticipantState,
    RetryEligibility, RollbackEligibility,
};
use super::*;

pub(super) fn missing_recorded_objects<B: GitBackend>(
    backend: &B,
    path: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<Vec<MissingObject>> {
    let mut required = vec![
        ("before commit", participant.before_commit.as_str()),
        ("source commit", participant.source_commit.as_str()),
    ];
    if let Some(result) = participant.resulting_commit.as_deref() {
        required.push(("resulting commit", result));
    }
    if let Some(merge_head) = participant.expected_merge_head.as_deref() {
        required.push(("expected merge head", merge_head));
    }
    if let Some(pending) = participant.pending_action.as_ref() {
        required.extend([
            ("pending before commit", pending.before_commit.as_str()),
            ("pending source commit", pending.source_commit.as_str()),
        ]);
    }

    let mut missing = Vec::new();
    let mut checked = Vec::new();
    for (role, oid) in required {
        if checked.contains(&oid) {
            continue;
        }
        checked.push(oid);
        if !member_result(
            backend.commit_exists(path, oid),
            target_id,
            &participant.path,
        )? {
            missing.push(MissingObject {
                role: role.to_owned(),
                oid: oid.to_owned(),
            });
        }
    }
    Ok(missing)
}

pub(super) fn deepen_conflict_eligibility<B: GitBackend>(
    backend: &B,
    path: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
    observation: &mut MergeParticipantObservation,
) {
    let merge_head = participant
        .expected_merge_head
        .as_deref()
        .unwrap_or(&participant.source_commit);
    if let Err(error) =
        backend.validate_merge_recovery_state(path, &participant.before_commit, merge_head, false)
    {
        observation.drift.push(participant_drift(
            ParticipantDriftKind::IndexModified,
            target_id,
            participant,
            live,
            &format!(
                "restore the recorded merge index and worktree before recovery ({})",
                error.message
            ),
        ));
        observation.continue_eligibility.eligible = false;
        observation.abort_eligibility.eligible = false;
        push_once(
            &mut observation.continue_eligibility.blockers,
            ParticipantDriftKind::IndexModified,
        );
        push_once(
            &mut observation.abort_eligibility.blockers,
            ParticipantDriftKind::IndexModified,
        );
    } else if observation.continue_eligibility.eligible
        && let Err(error) = backend.validate_merge_recovery_state(
            path,
            &participant.before_commit,
            merge_head,
            true,
        )
    {
        observation.drift.push(participant_drift(
            ParticipantDriftKind::IndexModified,
            target_id,
            participant,
            live,
            &format!(
                "finish staging the recorded merge resolution ({})",
                error.message
            ),
        ));
        observation.continue_eligibility.eligible = false;
        push_once(
            &mut observation.continue_eligibility.blockers,
            ParticipantDriftKind::IndexModified,
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ParticipantLiveState {
    pub(super) branch: Option<String>,
    pub(super) head: Option<String>,
    pub(super) target_ref: Option<String>,
    pub(super) status: GitStatus,
    pub(super) repository_state: GitRepositoryState,
    pub(super) merge_state: Option<GitNativeMergeState>,
    pub(super) native_detail_error: Option<String>,
    pub(super) missing_objects: Vec<MissingObject>,
    pub(super) head_relation: HeadRelation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MissingObject {
    pub(super) role: String,
    pub(super) oid: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HeadRelation {
    Equal,
    Advanced,
    Rewound,
    Diverged,
    Missing,
    ObjectUnavailable,
}

pub(super) fn classify_participant(
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
) -> MergeParticipantObservation {
    let expected_head = expected_head(participant).unwrap_or(&participant.before_commit);
    let mut drift = Vec::new();
    let conflicted = participant.state == ParticipantState::Conflicted;
    {
        let mut add = |kind: ParticipantDriftKind, guidance: &str| {
            drift.push(participant_drift(
                kind,
                target_id,
                participant,
                live,
                guidance,
            ));
        };
        for missing in &live.missing_objects {
            let guidance = format!(
                "recorded {} object {} is missing; restore the object before recovery",
                missing.role, missing.oid
            );
            add(ParticipantDriftKind::ObjectMissing, &guidance);
        }
        if live.branch.as_deref() != Some(participant.target_branch.as_str()) {
            add(
                ParticipantDriftKind::BranchChanged,
                "restore the recorded target branch before continuing or aborting",
            );
        }
        if live.target_ref.as_deref() != Some(expected_head) {
            add(
                ParticipantDriftKind::TargetRefChanged,
                "restore the target ref to its recorded commit before continuing or aborting",
            );
        }
        if live.head_relation != HeadRelation::Equal {
            let kind = match live.head_relation {
                HeadRelation::Advanced => ParticipantDriftKind::HeadAdvanced,
                HeadRelation::Rewound => ParticipantDriftKind::HeadRewound,
                HeadRelation::Diverged => ParticipantDriftKind::HeadDiverged,
                HeadRelation::Missing | HeadRelation::ObjectUnavailable => {
                    ParticipantDriftKind::ObjectMissing
                }
                HeadRelation::Equal => unreachable!(),
            };
            let guidance = if matches!(
                participant.state,
                ParticipantState::Planned
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
            ) {
                "restore this repository to its recorded before commit and clean state, or abort"
            } else {
                "preserve or remove post-merge work and restore the recorded result before recovery"
            };
            add(kind, guidance);
        }
        match live.repository_state {
            GitRepositoryState::Clean => {
                if conflicted {
                    add(
                        ParticipantDriftKind::MergeStateMissing,
                        "the recorded native merge is no longer active; an exact clean before state remains abortable",
                    );
                }
            }
            GitRepositoryState::Merge => {
                if conflicted {
                    match &live.merge_state {
                        None => add(
                            ParticipantDriftKind::MergeStateMissing,
                            live.native_detail_error.as_deref().unwrap_or(
                                "restore the recorded native merge metadata before recovery",
                            ),
                        ),
                        Some(state)
                            if state.merge_head
                                != participant
                                    .expected_merge_head
                                    .as_deref()
                                    .unwrap_or(&participant.source_commit) =>
                        {
                            add(
                                ParticipantDriftKind::MergeHeadChanged,
                                "restore the expected MERGE_HEAD before recovery",
                            );
                        }
                        Some(_) => {}
                    }
                } else {
                    add(
                        ParticipantDriftKind::NewIntegrationState,
                        "finish or abort the unrelated merge before merge recovery",
                    );
                }
            }
            foreign => {
                let guidance = format!(
                    "finish or abort the unrelated {} operation before merge recovery",
                    foreign.as_str()
                );
                add(ParticipantDriftKind::ForeignIntegrationState, &guidance);
            }
        }
        if !conflicted && (live.status.staged > 0 || live.status.unresolved > 0) {
            add(
                ParticipantDriftKind::IndexModified,
                "restore the recorded clean index before recovery",
            );
        }
        if live.status.untracked > 0 || (!conflicted && live.status.unstaged > 0) {
            add(
                ParticipantDriftKind::WorktreeModified,
                "preserve or remove unrelated worktree changes before recovery",
            );
        }
    }

    let drift_blockers: Vec<_> = drift.iter().map(|item| item.kind).collect();
    let native_matches = conflicted
        && live.repository_state == GitRepositoryState::Merge
        && live.merge_state.as_ref().is_some_and(|state| {
            state.merge_head
                == participant
                    .expected_merge_head
                    .as_deref()
                    .unwrap_or(&participant.source_commit)
        });
    let continue_extra = conflicted && (live.status.unresolved > 0 || live.status.unstaged > 0);
    let mut continue_blockers = drift_blockers.clone();
    if continue_extra {
        push_once(&mut continue_blockers, ParticipantDriftKind::IndexModified);
    }
    let continue_eligible = if conflicted {
        drift.is_empty() && native_matches && !continue_extra
    } else {
        drift.is_empty()
            && live.repository_state == GitRepositoryState::Clean
            && !live.status.is_dirty
    };
    let no_abort_action = does_not_require_rollback(participant.state);
    let exact_before_clean = live.branch.as_deref() == Some(participant.target_branch.as_str())
        && live.head.as_deref() == Some(participant.before_commit.as_str())
        && live.target_ref.as_deref() == Some(participant.before_commit.as_str())
        && live.repository_state == GitRepositoryState::Clean
        && !live.status.is_dirty
        && live.missing_objects.is_empty();
    let externally_restored_conflict = conflicted && exact_before_clean;
    let restored_mutation = matches!(
        participant.state,
        ParticipantState::FastForwarded | ParticipantState::Merged | ParticipantState::Continued
    ) && exact_before_clean;
    let durable_restore_verified = matches!(
        participant.state,
        ParticipantState::Aborted | ParticipantState::RolledBack
    ) && live.target_ref.as_deref()
        == Some(participant.before_commit.as_str());
    let abort_eligible = no_abort_action
        || durable_restore_verified
        || externally_restored_conflict
        || restored_mutation
        || if conflicted {
            native_matches && drift.is_empty()
        } else {
            drift.is_empty()
                && live.repository_state == GitRepositoryState::Clean
                && !live.status.is_dirty
        };
    let abort_blockers = if abort_eligible {
        Vec::new()
    } else {
        drift_blockers
    };
    MergeParticipantObservation {
        live_commit: live.head.clone(),
        conflict_paths: live
            .merge_state
            .as_ref()
            .map(|state| state.conflict_paths.clone())
            .unwrap_or_default(),
        drift,
        continue_eligibility: RetryEligibility {
            eligible: continue_eligible,
            blockers: continue_blockers,
        },
        abort_eligibility: RollbackEligibility {
            eligible: abort_eligible,
            blockers: abort_blockers,
        },
        pending_action: None,
    }
}
