use crate::git::{GitNativeMergeState, GitRepositoryState, GitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, MergeStatusSnapshot, OperationDriftKind,
    ParticipantDrift, ParticipantDriftKind, ParticipantState, PendingActionObservation,
    PendingActionObservationState,
};
use super::continue_eligibility::{
    ContinueEligibilityFacts, continue_eligibility, missing_repository_continue_eligibility,
};
use super::rollback::{
    RollbackEligibilityFacts, missing_repository_abort_eligibility, rollback_eligibility,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum ExpectedHeadSource {
    BeforeCommit,
    ResultingCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum ConflictRole {
    None,
    NativeMerge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum HeadDriftGuidance {
    RestoreBeforeOrAbort,
    RestoreRecordedResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RootAttemptedRole {
    NotAttempted,
    Attempted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct StatusStatePolicy {
    pub(in crate::workspace_ops::merge) expected_head_source: ExpectedHeadSource,
    pub(in crate::workspace_ops::merge) conflict_role: ConflictRole,
    pub(in crate::workspace_ops::merge) head_drift_guidance: HeadDriftGuidance,
    pub(in crate::workspace_ops::merge) root_attempted_role: RootAttemptedRole,
}

pub(in crate::workspace_ops::merge) const fn status_policy(
    state: ParticipantState,
) -> StatusStatePolicy {
    use ConflictRole::{NativeMerge as Conflict, None as Ordinary};
    use ExpectedHeadSource::{BeforeCommit as Before, ResultingCommit as Result};
    use HeadDriftGuidance::{
        RestoreBeforeOrAbort as RestoreBefore, RestoreRecordedResult as RestoreResult,
    };
    use RootAttemptedRole::{Attempted, NotAttempted};
    match state {
        ParticipantState::Planned => policy(Before, Ordinary, RestoreBefore, NotAttempted),
        ParticipantState::UpToDate => policy(Result, Ordinary, RestoreResult, Attempted),
        ParticipantState::FastForwarded => policy(Result, Ordinary, RestoreResult, Attempted),
        ParticipantState::Merged => policy(Result, Ordinary, RestoreResult, Attempted),
        ParticipantState::Conflicted => policy(Before, Conflict, RestoreResult, Attempted),
        ParticipantState::Failed => policy(Before, Ordinary, RestoreBefore, Attempted),
        ParticipantState::Unattempted => policy(Before, Ordinary, RestoreBefore, NotAttempted),
        ParticipantState::Continued => policy(Result, Ordinary, RestoreResult, Attempted),
        ParticipantState::Aborted => policy(Before, Ordinary, RestoreResult, Attempted),
        ParticipantState::RolledBack => policy(Before, Ordinary, RestoreResult, Attempted),
    }
}

const fn policy(
    expected_head_source: ExpectedHeadSource,
    conflict_role: ConflictRole,
    head_drift_guidance: HeadDriftGuidance,
    root_attempted_role: RootAttemptedRole,
) -> StatusStatePolicy {
    StatusStatePolicy {
        expected_head_source,
        conflict_role,
        head_drift_guidance,
        root_attempted_role,
    }
}

pub(in crate::workspace_ops::merge) fn expected_head(
    participant: &MergeParticipantRecord,
) -> ModelResult<&str> {
    match status_policy(participant.state).expected_head_source {
        ExpectedHeadSource::BeforeCommit => Ok(&participant.before_commit),
        ExpectedHeadSource::ResultingCommit => {
            participant.resulting_commit.as_deref().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!(
                        "merge participant '{}' has no resulting commit",
                        participant.path
                    ),
                )
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct ParticipantLiveState {
    pub(in crate::workspace_ops::merge) branch: Option<String>,
    pub(in crate::workspace_ops::merge) head: Option<String>,
    pub(in crate::workspace_ops::merge) target_ref: Option<String>,
    pub(in crate::workspace_ops::merge) status: GitStatus,
    pub(in crate::workspace_ops::merge) repository_state: GitRepositoryState,
    pub(in crate::workspace_ops::merge) merge_state: Option<GitNativeMergeState>,
    pub(in crate::workspace_ops::merge) native_detail_error: Option<String>,
    pub(in crate::workspace_ops::merge) missing_objects: Vec<MissingObject>,
    pub(in crate::workspace_ops::merge) head_relation: HeadRelation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct MissingObject {
    pub(in crate::workspace_ops::merge) role: String,
    pub(in crate::workspace_ops::merge) oid: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum HeadRelation {
    Equal,
    Advanced,
    Rewound,
    Diverged,
    Missing,
    ObjectUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct StatusFacts {
    conflicted: bool,
    native_merge_matches: bool,
    exact_before_clean: bool,
    target_ref_matches_before: bool,
    repository_is_clean: bool,
    worktree_is_clean: bool,
    conflict_has_unresolved_or_unstaged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct ParticipantDriftProjection {
    pub(in crate::workspace_ops::merge) live_commit: Option<String>,
    pub(in crate::workspace_ops::merge) conflict_paths: Vec<String>,
    pub(in crate::workspace_ops::merge) drift: Vec<ParticipantDrift>,
    pub(in crate::workspace_ops::merge) facts: StatusFacts,
}

pub(in crate::workspace_ops::merge) fn project_participant_drift(
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
) -> ParticipantDriftProjection {
    let expected_head = expected_head(participant).unwrap_or(&participant.before_commit);
    let policy = status_policy(participant.state);
    let conflicted = policy.conflict_role == ConflictRole::NativeMerge;
    let mut drift = Vec::new();
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
        add(
            ParticipantDriftKind::ObjectMissing,
            &format!(
                "recorded {} object {} is missing; restore the object before recovery",
                missing.role, missing.oid
            ),
        );
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
        let guidance = match policy.head_drift_guidance {
            HeadDriftGuidance::RestoreBeforeOrAbort => {
                "restore this repository to its recorded before commit and clean state, or abort"
            }
            HeadDriftGuidance::RestoreRecordedResult => {
                "preserve or remove post-merge work and restore the recorded result before recovery"
            }
        };
        add(kind, guidance);
    }
    match live.repository_state {
        GitRepositoryState::Clean if conflicted => add(
            ParticipantDriftKind::MergeStateMissing,
            "the recorded native merge is no longer active; an exact clean before state remains abortable",
        ),
        GitRepositoryState::Clean => {}
        GitRepositoryState::Merge if conflicted => match &live.merge_state {
            None => add(
                ParticipantDriftKind::MergeStateMissing,
                live.native_detail_error
                    .as_deref()
                    .unwrap_or("restore the recorded native merge metadata before recovery"),
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
        },
        GitRepositoryState::Merge => add(
            ParticipantDriftKind::NewIntegrationState,
            "finish or abort the unrelated merge before merge recovery",
        ),
        foreign => add(
            ParticipantDriftKind::ForeignIntegrationState,
            &format!(
                "finish or abort the unrelated {} operation before merge recovery",
                foreign.as_str()
            ),
        ),
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

    let native_merge_matches = conflicted
        && live.repository_state == GitRepositoryState::Merge
        && live.merge_state.as_ref().is_some_and(|state| {
            state.merge_head
                == participant
                    .expected_merge_head
                    .as_deref()
                    .unwrap_or(&participant.source_commit)
        });
    ParticipantDriftProjection {
        live_commit: live.head.clone(),
        conflict_paths: live
            .merge_state
            .as_ref()
            .map(|state| state.conflict_paths.clone())
            .unwrap_or_default(),
        drift,
        facts: StatusFacts {
            conflicted,
            native_merge_matches,
            exact_before_clean: live.branch.as_deref() == Some(participant.target_branch.as_str())
                && live.head.as_deref() == Some(participant.before_commit.as_str())
                && live.target_ref.as_deref() == Some(participant.before_commit.as_str())
                && live.repository_state == GitRepositoryState::Clean
                && !live.status.is_dirty
                && live.missing_objects.is_empty(),
            target_ref_matches_before: live.target_ref.as_deref()
                == Some(participant.before_commit.as_str()),
            repository_is_clean: live.repository_state == GitRepositoryState::Clean,
            worktree_is_clean: !live.status.is_dirty,
            conflict_has_unresolved_or_unstaged: conflicted
                && (live.status.unresolved > 0 || live.status.unstaged > 0),
        },
    }
}

pub(in crate::workspace_ops::merge) fn observation_from_projection(
    participant: &MergeParticipantRecord,
    projection: ParticipantDriftProjection,
) -> MergeParticipantObservation {
    let drift_blockers = projection
        .drift
        .iter()
        .map(|item| item.kind)
        .collect::<Vec<_>>();
    let facts = projection.facts;
    MergeParticipantObservation {
        live_commit: projection.live_commit,
        conflict_paths: projection.conflict_paths,
        drift: projection.drift,
        continue_eligibility: continue_eligibility(
            ContinueEligibilityFacts {
                conflicted: facts.conflicted,
                native_merge_matches: facts.native_merge_matches,
                conflict_has_unresolved_or_unstaged: facts.conflict_has_unresolved_or_unstaged,
                repository_is_clean: facts.repository_is_clean,
                worktree_is_clean: facts.worktree_is_clean,
            },
            drift_blockers.clone(),
        ),
        abort_eligibility: rollback_eligibility(
            participant.state,
            RollbackEligibilityFacts {
                native_merge_matches: facts.native_merge_matches,
                exact_before_clean: facts.exact_before_clean,
                target_ref_matches_before: facts.target_ref_matches_before,
                repository_is_clean: facts.repository_is_clean,
                worktree_is_clean: facts.worktree_is_clean,
            },
            drift_blockers,
        ),
        pending_action: None,
    }
}

pub(in crate::workspace_ops::merge) fn missing_repository_observation(
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> MergeParticipantObservation {
    let drift = vec![missing_repository_drift(target_id, participant)]
        .into_iter()
        .chain(participant.pending_action.as_ref().map(|_| ParticipantDrift {
            kind: ParticipantDriftKind::PendingActionAmbiguous,
            message: format!(
                "participant '{target_id}' at '{}': pending action cannot be reconciled because the repository is missing",
                participant.path
            ),
            expected_branch: Some(participant.target_branch.clone()),
            live_branch: None,
            expected_head: Some(participant.before_commit.clone()),
            live_head: None,
            expected_merge_head: participant.expected_merge_head.clone(),
            live_merge_head: None,
        }))
        .collect();
    MergeParticipantObservation {
        live_commit: None,
        conflict_paths: Vec::new(),
        drift,
        continue_eligibility: missing_repository_continue_eligibility(),
        abort_eligibility: missing_repository_abort_eligibility(
            participant.state,
            participant.pending_action.is_some(),
        ),
        pending_action: participant.pending_action.as_ref().map(|pending| {
            PendingActionObservation {
                kind: pending.kind,
                state: PendingActionObservationState::Ambiguous,
                message: Some("recorded participant repository is missing".to_owned()),
            }
        }),
    }
}

fn missing_repository_drift(
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ParticipantDrift {
    ParticipantDrift {
        kind: ParticipantDriftKind::RepositoryMissing,
        message: format!(
            "participant '{target_id}' at '{}' is missing; restore it at the recorded path before recovery",
            participant.path
        ),
        expected_branch: Some(participant.target_branch.clone()),
        live_branch: None,
        expected_head: Some(participant.before_commit.clone()),
        live_head: None,
        expected_merge_head: participant.expected_merge_head.clone(),
        live_merge_head: None,
    }
}

pub(in crate::workspace_ops::merge) fn participant_drift(
    kind: ParticipantDriftKind,
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
    guidance: &str,
) -> ParticipantDrift {
    ParticipantDrift {
        kind,
        message: format!(
            "participant '{target_id}' at '{}': {guidance}",
            participant.path
        ),
        expected_branch: Some(participant.target_branch.clone()),
        live_branch: live.branch.clone(),
        expected_head: expected_head(participant).ok().map(str::to_owned),
        live_head: live.head.clone(),
        expected_merge_head: participant.expected_merge_head.clone(),
        live_merge_head: live
            .merge_state
            .as_ref()
            .map(|state| state.merge_head.clone()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum ConflictValidationOutcome {
    NotChecked,
    Valid,
    Invalid(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct ConflictValidationOutcomes {
    pub(in crate::workspace_ops::merge) abort: ConflictValidationOutcome,
    pub(in crate::workspace_ops::merge) resolution: ConflictValidationOutcome,
}

pub(in crate::workspace_ops::merge) fn apply_conflict_validation(
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
    outcomes: ConflictValidationOutcomes,
    observation: &mut MergeParticipantObservation,
) {
    match outcomes.abort {
        ConflictValidationOutcome::Invalid(error) => {
            observation.drift.push(participant_drift(
                ParticipantDriftKind::IndexModified,
                target_id,
                participant,
                live,
                &format!("restore the recorded merge index and worktree before recovery ({error})"),
            ));
            super::continue_eligibility::block_continue(
                &mut observation.continue_eligibility,
                ParticipantDriftKind::IndexModified,
            );
            super::rollback::block_abort(
                &mut observation.abort_eligibility,
                ParticipantDriftKind::IndexModified,
            );
        }
        ConflictValidationOutcome::NotChecked | ConflictValidationOutcome::Valid => {}
    }
    if let ConflictValidationOutcome::Invalid(error) = outcomes.resolution {
        observation.drift.push(participant_drift(
            ParticipantDriftKind::IndexModified,
            target_id,
            participant,
            live,
            &format!("finish staging the recorded merge resolution ({error})"),
        ));
        super::continue_eligibility::block_continue(
            &mut observation.continue_eligibility,
            ParticipantDriftKind::IndexModified,
        );
    }
}

pub(in crate::workspace_ops::merge) fn apply_exact_root_finalization_override(
    observation: &mut MergeParticipantObservation,
) {
    observation.conflict_paths.clear();
    observation.drift.clear();
    observation.continue_eligibility.eligible = true;
    observation.continue_eligibility.blockers.clear();
    observation.abort_eligibility.eligible = true;
    observation.abort_eligibility.blockers.clear();
}

pub(in crate::workspace_ops::merge) fn apply_interrupted_root_rollback_override(
    snapshot: &mut MergeStatusSnapshot,
) -> ModelResult<()> {
    let participant = snapshot.record.participants.get("@root").ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a durable root participant",
        )
    })?;
    if participant.target_kind != super::super::MergeTargetKind::Root || participant.path != "." {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence participant identity is inconsistent",
        ));
    }
    let observation = snapshot.participants.get_mut("@root").ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a root status observation",
        )
    })?;
    snapshot
        .operation_drift
        .retain(|drift| drift.kind != OperationDriftKind::RootCandidateStateChanged);
    observation.live_commit = participant.resulting_commit.clone();
    observation.conflict_paths.clear();
    observation.drift.clear();
    observation.abort_eligibility.eligible = true;
    observation.abort_eligibility.blockers.clear();
    Ok(())
}

#[cfg(test)]
#[path = "status_tests/mod.rs"]
mod tests;
