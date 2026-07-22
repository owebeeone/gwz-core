use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::{
    GitBackend, GitNativeMergeState, GitPreparedCommit, GitPreparedSignature, GitRepositoryState,
    GitStatus,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::{MemberPath, WORKSPACE_MANIFEST};

use super::{
    MergeOperationRecord, MergeParticipantObservation, MergeParticipantRecord, MergeStatusSnapshot,
    MergeStore, MergeTargetKind, OperationDrift, OperationDriftKind, ParticipantDrift,
    ParticipantDriftKind, ParticipantState, RetryEligibility, RollbackEligibility,
};

pub(crate) fn handle_status<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let Some(record) = store.discover_open(root)? else {
        return super::response::idle_status_response(context);
    };
    snapshot_status(backend, root, record)?.to_response(context)
}

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

fn reconcile_pending_action_from_live<B: GitBackend>(
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
    if pending.target_branch != participant.target_branch
        || pending.before_commit != participant.before_commit
        || pending.source_commit != participant.source_commit
        || pending.commit_message != participant.commit_message
    {
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
    if let Err(reason) = validate_pending_intent(pending) {
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

    let drift = classify_participant(target_id, participant, live).drift;
    let exact_branch = live.branch.as_deref() == Some(pending.target_branch.as_str())
        && live.head == live.target_ref;
    let clean = live.repository_state == GitRepositoryState::Clean
        && !live.status.is_dirty
        && live.missing_objects.is_empty();

    if exact_branch && clean {
        let live_commit = live.head.as_deref();
        match pending.kind {
            super::PendingMergeActionKind::VerifyUpToDate
                if live_commit == Some(pending.before_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            super::PendingMergeActionKind::FastForward
                if live_commit == Some(pending.source_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::Completed {
                    resulting_commit: pending.source_commit.clone(),
                });
            }
            super::PendingMergeActionKind::FastForward
            | super::PendingMergeActionKind::TrueMerge
                if live_commit == Some(pending.before_commit.as_str()) =>
            {
                return Ok(PendingActionReconciliation::NotStarted);
            }
            super::PendingMergeActionKind::TrueMerge
            | super::PendingMergeActionKind::ResolveConflict => {
                if let (Some(commit), Some(prepared)) =
                    (live_commit, pending_prepared_commit(pending))
                    && member_result(
                        backend.commit_matches_prepared_merge(
                            path,
                            commit,
                            &pending.before_commit,
                            &pending.source_commit,
                            &pending.commit_message,
                            &prepared,
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
        let require_resolved = pending.kind == super::PendingMergeActionKind::ResolveConflict;
        let native_intent_matches = require_resolved
            || (pending.kind == super::PendingMergeActionKind::TrueMerge
                && pending.expected_result
                    == Some(super::PendingMergeExpectedResult::ExpectedConflict));
        if native_intent_matches
            && backend
                .validate_merge_recovery_state(
                    path,
                    &pending.before_commit,
                    &pending.source_commit,
                    require_resolved,
                )
                .is_ok()
        {
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

fn validate_pending_intent(pending: &super::PendingMergeAction) -> Result<(), &'static str> {
    use super::{PendingMergeActionKind as Kind, PendingMergeExpectedResult as Result};
    let valid = matches!(
        (
            pending.kind,
            pending.expected_result,
            pending.commit_spec.is_some(),
        ),
        (Kind::VerifyUpToDate, None | Some(Result::Unchanged), false)
            | (Kind::FastForward, None | Some(Result::FastForward), false)
            | (Kind::TrueMerge, Some(Result::ExpectedConflict), false)
            | (
                Kind::TrueMerge | Kind::ResolveConflict,
                Some(Result::Commit),
                true
            )
    );
    valid.then_some(()).ok_or(
        "pending commit-producing action lacks a complete exact result and signature specification",
    )
}

fn pending_prepared_commit(pending: &super::PendingMergeAction) -> Option<GitPreparedCommit> {
    let spec = pending.commit_spec.as_ref()?;
    Some(GitPreparedCommit {
        tree_oid: spec.tree_oid.clone(),
        author: prepared_signature(&spec.author),
        committer: prepared_signature(&spec.committer),
    })
}

fn prepared_signature(signature: &super::PendingGitSignature) -> GitPreparedSignature {
    GitPreparedSignature {
        name: signature.name.clone(),
        email: signature.email.clone(),
        time_seconds: signature.time_seconds,
        timezone_offset_minutes: signature.timezone_offset_minutes,
    }
}

pub(crate) fn snapshot_status<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
) -> ModelResult<MergeStatusSnapshot> {
    // Validate the entire durable path set before the first repository access;
    // a corrupt unselected row must not become a later filesystem escape.
    for (target_id, participant) in &record.participants {
        validated_participant_path(root, target_id, participant)?;
    }
    let mut participants = BTreeMap::new();
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        participants.insert(
            target_id.clone(),
            observe_participant(backend, root, target_id, participant)?,
        );
    }

    let root_attempted = record.participants.values().any(|participant| {
        participant.target_kind == MergeTargetKind::Root
            && !matches!(
                participant.state,
                ParticipantState::Planned | ParticipantState::Unattempted
            )
    });
    let mut operation_drift = record.operation_drift.clone();
    if !root_attempted {
        compare_digest(
            root,
            artifact::LOCK_PATH,
            &record.baseline.lock_sha256,
            OperationDriftKind::BaselineLockChanged,
            &mut operation_drift,
        );
        compare_digest(
            root,
            WORKSPACE_MANIFEST,
            &record.baseline.manifest_sha256,
            OperationDriftKind::BaselineManifestChanged,
            &mut operation_drift,
        );
    }
    Ok(MergeStatusSnapshot {
        record,
        participants,
        operation_drift,
    })
}

pub(crate) fn observe_participant<B: GitBackend>(
    backend: &B,
    root: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<MergeParticipantObservation> {
    let path = validated_participant_path(root, target_id, participant)?;
    if !path.is_dir() || !member_result(backend.is_repository(&path), target_id, &participant.path)?
    {
        return Ok(missing_observation(target_id, participant));
    }
    let live = read_live_participant(backend, &path, target_id, participant)?;
    let mut observation = classify_participant(target_id, participant, &live);
    if participant.pending_action.is_some() {
        let reconciliation =
            reconcile_pending_action_from_live(backend, &path, target_id, participant, &live)?;
        apply_pending_observation(participant, reconciliation, &mut observation);
    } else if participant.state == ParticipantState::Conflicted && observation.drift.is_empty() {
        deepen_conflict_eligibility(
            backend,
            &path,
            target_id,
            participant,
            &live,
            &mut observation,
        );
    }
    Ok(observation)
}

fn apply_pending_observation(
    participant: &MergeParticipantRecord,
    reconciliation: PendingActionReconciliation,
    observation: &mut MergeParticipantObservation,
) {
    let kind = participant
        .pending_action
        .as_ref()
        .expect("pending observation requires durable pending action")
        .kind;
    let (state, message) = match reconciliation {
        PendingActionReconciliation::NotStarted => {
            observation.continue_eligibility = RetryEligibility {
                eligible: true,
                blockers: Vec::new(),
            };
            if kind == super::PendingMergeActionKind::ResolveConflict {
                observation.abort_eligibility = RollbackEligibility {
                    eligible: false,
                    blockers: vec![ParticipantDriftKind::IndexModified],
                };
            } else {
                observation.drift.clear();
                observation.abort_eligibility = RollbackEligibility {
                    eligible: true,
                    blockers: Vec::new(),
                };
            }
            (
                super::PendingActionObservationState::NotStarted,
                Some("live repository exactly matches the pending action's retry point".to_owned()),
            )
        }
        PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
            observation.conflict_paths = conflict_paths;
            observation.drift.clear();
            observation.continue_eligibility = RetryEligibility {
                eligible: false,
                blockers: vec![ParticipantDriftKind::IndexModified],
            };
            observation.abort_eligibility = RollbackEligibility {
                eligible: true,
                blockers: Vec::new(),
            };
            (
                super::PendingActionObservationState::ExpectedConflict,
                Some("pending true merge reached its exact expected native conflict".to_owned()),
            )
        }
        PendingActionReconciliation::Completed { resulting_commit } => {
            observation.live_commit = Some(resulting_commit);
            observation.drift.clear();
            observation.continue_eligibility = RetryEligibility {
                eligible: true,
                blockers: Vec::new(),
            };
            observation.abort_eligibility = RollbackEligibility {
                eligible: true,
                blockers: Vec::new(),
            };
            (
                super::PendingActionObservationState::CompletedExactly,
                Some("pending action completed exactly and can be adopted durably".to_owned()),
            )
        }
        PendingActionReconciliation::Ambiguous { reason, drift } => {
            for item in drift {
                if !observation
                    .drift
                    .iter()
                    .any(|existing| existing.kind == item.kind)
                {
                    observation.drift.push(item);
                }
            }
            observation.continue_eligibility.eligible = false;
            observation.abort_eligibility.eligible = false;
            (
                super::PendingActionObservationState::Ambiguous,
                Some(reason),
            )
        }
    };
    observation.pending_action = Some(super::PendingActionObservation {
        kind,
        state,
        message,
    });
}

fn read_live_participant<B: GitBackend>(
    backend: &B,
    path: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<ParticipantLiveState> {
    let expected_head = expected_head(participant)?;
    let head = member_result(backend.head(path), target_id, &participant.path)?;
    let target_ref = member_result(
        backend.read_ref(path, &format!("refs/heads/{}", participant.target_branch)),
        target_id,
        &participant.path,
    )?;
    let mut missing_objects = missing_recorded_objects(backend, path, target_id, participant)?;
    if let Some(live) = head.commit.as_deref()
        && !member_result(
            backend.commit_exists(path, live),
            target_id,
            &participant.path,
        )?
        && !missing_objects.iter().any(|missing| missing.oid == live)
    {
        missing_objects.push(MissingObject {
            role: "live HEAD".to_owned(),
            oid: live.to_owned(),
        });
    }
    let expected_exists = !missing_objects
        .iter()
        .any(|missing| missing.oid == expected_head);
    let live_exists = head
        .commit
        .as_deref()
        .is_none_or(|live| !missing_objects.iter().any(|missing| missing.oid == live));
    let relation = match head.commit.as_deref() {
        Some(live) if live == expected_head => HeadRelation::Equal,
        Some(_) if !expected_exists || !live_exists => HeadRelation::ObjectUnavailable,
        Some(live)
            if member_result(
                backend.is_ancestor(path, expected_head, live),
                target_id,
                &participant.path,
            )? =>
        {
            HeadRelation::Advanced
        }
        Some(live)
            if member_result(
                backend.is_ancestor(path, live, expected_head),
                target_id,
                &participant.path,
            )? =>
        {
            HeadRelation::Rewound
        }
        Some(_) => HeadRelation::Diverged,
        None => HeadRelation::Missing,
    };
    let repository_state =
        member_result(backend.repository_state(path), target_id, &participant.path)?;
    let (merge_state, native_detail_error) = if repository_state == GitRepositoryState::Merge {
        match backend.merge_state(path) {
            Ok(state) => (state, None),
            Err(error) => (None, Some(error.message)),
        }
    } else {
        (None, None)
    };
    Ok(ParticipantLiveState {
        branch: head.branch,
        head: head.commit,
        target_ref,
        status: member_result(backend.status(path), target_id, &participant.path)?,
        repository_state,
        merge_state,
        native_detail_error,
        missing_objects,
        head_relation: relation,
    })
}

fn validated_participant_path(
    root: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<PathBuf> {
    let valid = match participant.target_kind {
        MergeTargetKind::Root if participant.path == "." => return Ok(root.to_path_buf()),
        MergeTargetKind::Root => Err(ModelError::new(
            ErrorCode::PathEscape,
            "root participant path must be '.'",
        )),
        MergeTargetKind::Member => {
            MemberPath::parse(&participant.path).map(|path| path.to_string())
        }
    };
    valid.map(|path| root.join(path)).map_err(|error| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("invalid durable participant path: {}", error.message),
        )
        .with_member(target_id, &participant.path)
    })
}

fn member_result<T>(
    result: ModelResult<T>,
    target_id: &str,
    participant_path: &str,
) -> ModelResult<T> {
    result.map_err(|error| {
        if error.member_id.is_some() {
            error
        } else {
            error.with_member(target_id, participant_path)
        }
    })
}

fn missing_recorded_objects<B: GitBackend>(
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

fn deepen_conflict_eligibility<B: GitBackend>(
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
pub(crate) struct ParticipantLiveState {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub target_ref: Option<String>,
    pub status: GitStatus,
    pub repository_state: GitRepositoryState,
    pub merge_state: Option<GitNativeMergeState>,
    native_detail_error: Option<String>,
    missing_objects: Vec<MissingObject>,
    head_relation: HeadRelation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct MissingObject {
    role: String,
    oid: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeadRelation {
    Equal,
    Advanced,
    Rewound,
    Diverged,
    Missing,
    ObjectUnavailable,
}

pub(crate) fn classify_participant(
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

fn expected_head(participant: &MergeParticipantRecord) -> ModelResult<&str> {
    match participant.state {
        ParticipantState::UpToDate
        | ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued => participant.resulting_commit.as_deref().ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!(
                    "merge participant '{}' has no resulting commit",
                    participant.path
                ),
            )
        }),
        _ => Ok(&participant.before_commit),
    }
}

fn missing_observation(
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> MergeParticipantObservation {
    let repository_missing = ParticipantDrift {
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
    };
    let mut drift = vec![repository_missing];
    if participant.pending_action.is_some() {
        drift.push(ParticipantDrift {
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
        });
    }
    MergeParticipantObservation {
        live_commit: None,
        conflict_paths: Vec::new(),
        drift,
        continue_eligibility: RetryEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::RepositoryMissing],
        },
        abort_eligibility: RollbackEligibility {
            eligible: participant.pending_action.is_none()
                && does_not_require_rollback(participant.state),
            blockers: (participant.pending_action.is_some()
                || !does_not_require_rollback(participant.state))
            .then_some(ParticipantDriftKind::RepositoryMissing)
            .into_iter()
            .collect(),
        },
        pending_action: participant.pending_action.as_ref().map(|pending| {
            super::PendingActionObservation {
                kind: pending.kind,
                state: super::PendingActionObservationState::Ambiguous,
                message: Some("recorded participant repository is missing".to_owned()),
            }
        }),
    }
}

fn does_not_require_rollback(state: ParticipantState) -> bool {
    matches!(
        state,
        ParticipantState::UpToDate | ParticipantState::Unattempted
    )
}

fn participant_drift(
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

fn push_once(values: &mut Vec<ParticipantDriftKind>, value: ParticipantDriftKind) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn compare_digest(
    root: &Path,
    relative: &str,
    expected: &str,
    kind: OperationDriftKind,
    drift: &mut Vec<OperationDrift>,
) {
    let actual = fs::read(root.join(relative))
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    if actual.as_deref() != Some(expected) && !drift.iter().any(|item| item.kind == kind) {
        drift.push(OperationDrift {
            kind,
            message: format!(
                "workspace artifact '{relative}' changed from the recorded merge baseline"
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Git2Backend, GitBackend};
    use crate::workspace_ops::merge::{
        PendingCommitSpec, PendingGitSignature, PendingMergeAction, PendingMergeActionKind,
        PendingMergeExpectedResult,
    };

    fn test_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("gwz-status-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=GWZ",
                "-c",
                "user.email=gwz@example.invalid",
            ])
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit(repo: &Path, file: &str, content: &str, message: &str) -> String {
        fs::write(repo.join(file), content).unwrap();
        run_git(repo, &["add", file]);
        run_git(repo, &["commit", "-m", message]);
        run_git(repo, &["rev-parse", "HEAD"])
    }

    fn seed_divergence(repo: &Path) -> (String, String) {
        Git2Backend::new().create_repo(repo).unwrap();
        commit(repo, "base.txt", "base\n", "base");
        run_git(repo, &["branch", "feature"]);
        run_git(repo, &["checkout", "feature"]);
        let source = commit(repo, "source.txt", "source\n", "source");
        run_git(repo, &["checkout", "main"]);
        let before = commit(repo, "main.txt", "main\n", "main");
        (before, source)
    }

    fn pending_record(
        state: ParticipantState,
        before: &str,
        source: &str,
        message: &str,
        kind: PendingMergeActionKind,
    ) -> MergeParticipantRecord {
        let mut record = participant(state);
        record.before_commit = before.to_owned();
        record.source_commit = source.to_owned();
        record.commit_message = message.to_owned();
        record.pending_action = Some(PendingMergeAction {
            kind,
            target_branch: "main".to_owned(),
            before_commit: before.to_owned(),
            source_commit: source.to_owned(),
            commit_message: message.to_owned(),
            expected_result: None,
            commit_spec: None,
            extensions: BTreeMap::new(),
        });
        record
    }

    fn set_expected_conflict(record: &mut MergeParticipantRecord) {
        let pending = record.pending_action.as_mut().unwrap();
        pending.expected_result = Some(PendingMergeExpectedResult::ExpectedConflict);
        pending.commit_spec = None;
    }

    fn set_prepared_commit(record: &mut MergeParticipantRecord, prepared: &GitPreparedCommit) {
        let pending = record.pending_action.as_mut().unwrap();
        pending.expected_result = Some(PendingMergeExpectedResult::Commit);
        pending.commit_spec = Some(PendingCommitSpec {
            tree_oid: prepared.tree_oid.clone(),
            author: pending_signature(&prepared.author),
            committer: pending_signature(&prepared.committer),
            extensions: BTreeMap::new(),
        });
    }

    fn pending_signature(signature: &GitPreparedSignature) -> PendingGitSignature {
        PendingGitSignature {
            name: signature.name.clone(),
            email: signature.email.clone(),
            time_seconds: signature.time_seconds,
            timezone_offset_minutes: signature.timezone_offset_minutes,
            extensions: BTreeMap::new(),
        }
    }

    #[derive(Clone, Copy)]
    enum CandidateDifference {
        Tree,
        AuthorTime,
        CommitterTime,
    }

    fn alternate_merge_commit(
        repo_path: &Path,
        before: &str,
        source: &str,
        message: &str,
        prepared: &GitPreparedCommit,
        difference: CandidateDifference,
    ) -> String {
        let repo = git2::Repository::open(repo_path).unwrap();
        let expected_tree = repo
            .find_tree(git2::Oid::from_str(&prepared.tree_oid).unwrap())
            .unwrap();
        let tree_oid = if matches!(difference, CandidateDifference::Tree) {
            let blob = repo.blob(b"post-intent content\n").unwrap();
            let mut builder = repo.treebuilder(Some(&expected_tree)).unwrap();
            builder.insert("post-intent.txt", blob, 0o100644).unwrap();
            builder.write().unwrap()
        } else {
            expected_tree.id()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        let first = repo
            .find_commit(git2::Oid::from_str(before).unwrap())
            .unwrap();
        let second = repo
            .find_commit(git2::Oid::from_str(source).unwrap())
            .unwrap();
        let signature = |value: &GitPreparedSignature, delta: i64| {
            git2::Signature::new(
                &value.name,
                &value.email,
                &git2::Time::new(value.time_seconds + delta, value.timezone_offset_minutes),
            )
            .unwrap()
        };
        let author = signature(
            &prepared.author,
            i64::from(matches!(difference, CandidateDifference::AuthorTime)),
        );
        let committer = signature(
            &prepared.committer,
            i64::from(matches!(difference, CandidateDifference::CommitterTime)),
        );
        repo.commit(
            None,
            &author,
            &committer,
            message,
            &tree,
            &[&first, &second],
        )
        .unwrap()
        .to_string()
    }

    fn participant(state: ParticipantState) -> MergeParticipantRecord {
        let yaml = format!(
            "path: repos/app\ntarget_kind: member\ntarget_branch: main\nbefore_commit: before\nsource_commit: source\ncommit_message: merge\nstate: {}\n",
            serde_yaml::to_string(&state).unwrap().trim()
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    #[test]
    fn unattempted_post_plan_work_is_structured_and_blocks_recovery() {
        let record = participant(ParticipantState::Unattempted);
        let live = ParticipantLiveState {
            branch: Some("main".into()),
            head: Some("later".into()),
            target_ref: Some("later".into()),
            status: GitStatus {
                is_dirty: true,
                unstaged: 1,
                ..GitStatus::clean()
            },
            repository_state: GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Advanced,
        };
        let observed = classify_participant("mem_app", &record, &live);
        let kinds: Vec<_> = observed.drift.iter().map(|item| item.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ParticipantDriftKind::TargetRefChanged,
                ParticipantDriftKind::HeadAdvanced,
                ParticipantDriftKind::WorktreeModified,
            ]
        );
        assert!(!observed.continue_eligibility.eligible);
        assert!(observed.abort_eligibility.eligible);
        assert!(observed.drift[1].message.contains("or abort"));
    }

    #[test]
    fn digest_comparison_reports_change_without_rewriting_the_file() {
        let root = std::env::temp_dir().join(format!("gwz-status-{}", std::process::id()));
        let path = root.join("baseline");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, b"live").unwrap();
        let mut drift = Vec::new();
        compare_digest(
            &root,
            "baseline",
            "recorded",
            OperationDriftKind::BaselineLockChanged,
            &mut drift,
        );
        assert_eq!(drift[0].kind, OperationDriftKind::BaselineLockChanged);
        assert_eq!(fs::read(&path).unwrap(), b"live");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_unattempted_repo_is_visible_but_does_not_block_abort() {
        let observed = missing_observation("mem_app", &participant(ParticipantState::Unattempted));
        assert_eq!(
            observed.drift[0].kind,
            ParticipantDriftKind::RepositoryMissing
        );
        assert!(observed.abort_eligibility.eligible);
    }

    #[test]
    fn planned_drift_fails_closed_after_an_ambiguous_crash_window() {
        let record = participant(ParticipantState::Planned);
        let live = ParticipantLiveState {
            branch: Some("main".into()),
            head: Some("later".into()),
            target_ref: Some("later".into()),
            status: GitStatus::clean(),
            repository_state: GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Advanced,
        };
        let observed = classify_participant("mem_app", &record, &live);
        assert!(!observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);
        assert!(
            observed
                .abort_eligibility
                .blockers
                .contains(&ParticipantDriftKind::HeadAdvanced)
        );
    }

    #[test]
    fn divergent_head_has_its_own_structured_drift() {
        let record = participant(ParticipantState::Merged);
        let live = ParticipantLiveState {
            branch: Some("main".into()),
            head: Some("other-line".into()),
            target_ref: Some("other-line".into()),
            status: GitStatus::clean(),
            repository_state: crate::git::GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Diverged,
        };
        let observed = classify_participant("mem_app", &record, &live);

        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::HeadDiverged)
        );
        assert!(!observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);
    }

    #[test]
    fn foreign_native_state_blocks_rows_that_require_mutation() {
        let record = participant(ParticipantState::Merged);
        let live = ParticipantLiveState {
            branch: Some("main".into()),
            head: None,
            target_ref: None,
            status: GitStatus::clean(),
            repository_state: crate::git::GitRepositoryState::CherryPick,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Missing,
        };
        let observed = classify_participant("mem_app", &record, &live);

        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::ForeignIntegrationState)
        );
        assert!(!observed.abort_eligibility.eligible);
    }

    #[test]
    fn externally_restored_conflict_is_abort_eligible() {
        let record = participant(ParticipantState::Conflicted);
        let live = ParticipantLiveState {
            branch: Some("main".into()),
            head: Some("before".into()),
            target_ref: Some("before".into()),
            status: GitStatus::clean(),
            repository_state: crate::git::GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Equal,
        };
        let observed = classify_participant("mem_app", &record, &live);

        assert!(!observed.continue_eligibility.eligible);
        assert!(observed.abort_eligibility.eligible);
        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::MergeStateMissing)
        );
    }

    #[test]
    fn durably_restored_row_ignores_later_worktree_dirt_for_abort() {
        let record = participant(ParticipantState::RolledBack);
        let live = ParticipantLiveState {
            branch: Some("other".into()),
            head: Some("later".into()),
            target_ref: Some("before".into()),
            status: GitStatus {
                is_dirty: true,
                staged: 1,
                untracked: 1,
                ..GitStatus::clean()
            },
            repository_state: crate::git::GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: HeadRelation::Advanced,
        };
        let observed = classify_participant("mem_app", &record, &live);

        assert!(observed.abort_eligibility.eligible);
        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::WorktreeModified)
        );
    }

    #[test]
    fn invalid_record_path_is_rejected_before_repository_observation() {
        let root =
            std::env::temp_dir().join(format!("gwz-status-invalid-path-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let backend = crate::git::Git2Backend::new();
        for invalid in ["../outside", "/tmp/outside"] {
            let mut record = participant(ParticipantState::Unattempted);
            record.path = invalid.to_owned();
            let error = observe_participant(&backend, &root, "mem_app", &record).unwrap_err();
            assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
            assert_eq!(error.member_id.as_deref(), Some("mem_app"));
            assert_eq!(error.member_path.as_deref(), Some(invalid));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_expected_and_resulting_commits_are_member_scoped_object_drift() {
        let root = test_root("missing-object");
        let repo = root.join("repos/app");
        Git2Backend::new().create_repo(&repo).unwrap();
        let head = commit(&repo, "tracked.txt", "one\n", "initial");
        let missing = "0000000000000000000000000000000000000000";
        let mut expected = participant(ParticipantState::Unattempted);
        expected.before_commit = missing.to_owned();
        expected.source_commit = head.clone();
        let mut resulting = participant(ParticipantState::Merged);
        resulting.before_commit = head.clone();
        resulting.source_commit = head;
        resulting.resulting_commit = Some(missing.to_owned());

        for record in [expected, resulting] {
            let observed =
                observe_participant(&Git2Backend::new(), &root, "mem_app", &record).unwrap();
            assert!(
                observed
                    .drift
                    .iter()
                    .any(|drift| drift.kind == ParticipantDriftKind::ObjectMissing)
            );
            assert!(!observed.continue_eligibility.eligible);
        }
        assert!(repo.join("tracked.txt").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewound_detached_and_missing_heads_have_distinct_evidence() {
        let record = participant(ParticipantState::Unattempted);
        let cases = [
            (
                Some("main"),
                Some("older"),
                HeadRelation::Rewound,
                ParticipantDriftKind::HeadRewound,
            ),
            (
                None,
                Some("other"),
                HeadRelation::Advanced,
                ParticipantDriftKind::HeadAdvanced,
            ),
            (
                Some("main"),
                None,
                HeadRelation::Missing,
                ParticipantDriftKind::ObjectMissing,
            ),
        ];
        for (branch, head, relation, expected) in cases {
            let live = ParticipantLiveState {
                branch: branch.map(str::to_owned),
                head: head.map(str::to_owned),
                target_ref: head.map(str::to_owned),
                status: GitStatus::clean(),
                repository_state: GitRepositoryState::Clean,
                merge_state: None,
                native_detail_error: None,
                missing_objects: Vec::new(),
                head_relation: relation,
            };
            let observed = classify_participant("mem_app", &record, &live);
            assert!(observed.drift.iter().any(|drift| drift.kind == expected));
            if branch.is_none() {
                assert!(
                    observed
                        .drift
                        .iter()
                        .any(|drift| drift.kind == ParticipantDriftKind::BranchChanged)
                );
            }
        }
    }

    #[test]
    fn actual_foreign_sequencer_state_is_not_optimistically_accepted() {
        let root = test_root("foreign-state");
        let repo = root.join("repos/app");
        Git2Backend::new().create_repo(&repo).unwrap();
        let head = commit(&repo, "tracked.txt", "one\n", "initial");
        fs::write(repo.join(".git/CHERRY_PICK_HEAD"), format!("{head}\n")).unwrap();
        let mut record = participant(ParticipantState::Merged);
        record.before_commit = head.clone();
        record.source_commit = head.clone();
        record.resulting_commit = Some(head);

        let observed = observe_participant(&Git2Backend::new(), &root, "mem_app", &record).unwrap();

        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::ForeignIntegrationState)
        );
        assert!(!observed.abort_eligibility.eligible);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_true_merge_completion_is_exact_and_read_only() {
        let root = test_root("reconcile-complete");
        let repo = root.join("repos/app");
        let (before, source) = seed_divergence(&repo);
        let backend = Git2Backend::new();
        let message = "frozen message";
        let prepared = backend
            .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
            .unwrap();
        let result = backend
            .execute_prepared_merge_upstream_checked(
                &repo, "main", &before, &source, message, &prepared,
            )
            .unwrap();
        let mut record = pending_record(
            ParticipantState::Planned,
            &before,
            &source,
            message,
            PendingMergeActionKind::TrueMerge,
        );
        let crate::git::GitPreparedMerge::Commit(prepared) = prepared else {
            panic!("fixture must produce a clean merge")
        };
        set_prepared_commit(&mut record, &prepared);
        let index_before = fs::read(repo.join(".git/index")).unwrap();
        let head_before = backend.head(&repo).unwrap();

        let reconciled = reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap();

        assert_eq!(
            reconciled,
            PendingActionReconciliation::Completed {
                resulting_commit: result.commit.unwrap()
            }
        );
        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
        assert_eq!(
            observed
                .pending_action
                .as_ref()
                .map(|pending| pending.state),
            Some(super::super::PendingActionObservationState::CompletedExactly)
        );
        assert!(observed.drift.is_empty());
        assert!(observed.continue_eligibility.eligible);
        assert!(observed.abort_eligibility.eligible);
        assert_eq!(backend.head(&repo).unwrap(), head_before);
        assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
        assert_eq!(backend.status(&repo).unwrap(), GitStatus::clean());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn different_tree_or_signature_commit_is_ambiguous_and_status_is_read_only() {
        for (case, difference) in [
            ("tree", CandidateDifference::Tree),
            ("author", CandidateDifference::AuthorTime),
            ("committer", CandidateDifference::CommitterTime),
        ] {
            let root = test_root(&format!("reconcile-different-{case}"));
            let repo = root.join("repos/app");
            let (before, source) = seed_divergence(&repo);
            let backend = Git2Backend::new();
            let message = "frozen message";
            let prepared = backend
                .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
                .unwrap();
            let crate::git::GitPreparedMerge::Commit(prepared) = prepared else {
                panic!("fixture must prepare a commit")
            };
            let candidate =
                alternate_merge_commit(&repo, &before, &source, message, &prepared, difference);
            run_git(&repo, &["reset", "--hard", &candidate]);
            let mut record = pending_record(
                ParticipantState::Planned,
                &before,
                &source,
                message,
                PendingMergeActionKind::TrueMerge,
            );
            set_prepared_commit(&mut record, &prepared);
            let index_before = fs::read(repo.join(".git/index")).unwrap();
            let status_before = backend.status(&repo).unwrap();

            let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

            assert_eq!(
                observed.pending_action.unwrap().state,
                super::super::PendingActionObservationState::Ambiguous,
                "case={case}"
            );
            assert!(!observed.continue_eligibility.eligible, "case={case}");
            assert!(!observed.abort_eligibility.eligible, "case={case}");
            assert_eq!(
                backend.head(&repo).unwrap().commit.as_deref(),
                Some(candidate.as_str())
            );
            assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
            assert_eq!(backend.status(&repo).unwrap(), status_before);
            if matches!(difference, CandidateDifference::Tree) {
                assert!(repo.join("post-intent.txt").is_file());
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn old_commit_producing_pending_record_is_ambiguous_but_old_fast_forward_is_classifiable() {
        let root = test_root("reconcile-old-record");
        let repo = root.join("repos/app");
        let (before, source) = seed_divergence(&repo);
        let backend = Git2Backend::new();
        let old_merge = pending_record(
            ParticipantState::Planned,
            &before,
            &source,
            "old merge",
            PendingMergeActionKind::TrueMerge,
        );
        let observed = observe_participant(&backend, &root, "mem_app", &old_merge).unwrap();
        assert_eq!(
            observed.pending_action.unwrap().state,
            super::super::PendingActionObservationState::Ambiguous
        );
        assert!(!observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);

        let mut old_fast_forward = pending_record(
            ParticipantState::Planned,
            &before,
            &source,
            "old fast-forward",
            PendingMergeActionKind::FastForward,
        );
        old_fast_forward.source_commit = before.clone();
        old_fast_forward
            .pending_action
            .as_mut()
            .unwrap()
            .source_commit = before;
        assert_eq!(
            reconcile_pending_action(&backend, &root, "mem_app", &old_fast_forward).unwrap(),
            PendingActionReconciliation::Completed {
                resulting_commit: old_fast_forward.source_commit.clone()
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_conflict_and_resolved_native_state_are_distinguished() {
        let root = test_root("reconcile-conflict");
        let repo = root.join("repos/app");
        let backend = Git2Backend::new();
        backend.create_repo(&repo).unwrap();
        commit(&repo, "conflict.txt", "base\n", "base");
        run_git(&repo, &["branch", "feature"]);
        run_git(&repo, &["checkout", "feature"]);
        let source = commit(&repo, "conflict.txt", "source\n", "source");
        run_git(&repo, &["checkout", "main"]);
        let before = commit(&repo, "conflict.txt", "main\n", "main");
        let message = "frozen conflict message";
        let prepared = backend
            .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
            .unwrap();
        assert_eq!(prepared, crate::git::GitPreparedMerge::ExpectedConflict);
        let result = backend
            .execute_prepared_merge_upstream_checked(
                &repo, "main", &before, &source, message, &prepared,
            )
            .unwrap();
        assert_eq!(result.conflicts, vec!["conflict.txt"]);
        let mut record = pending_record(
            ParticipantState::Planned,
            &before,
            &source,
            message,
            PendingMergeActionKind::TrueMerge,
        );
        set_expected_conflict(&mut record);

        assert_eq!(
            reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
            PendingActionReconciliation::ExpectedConflict {
                conflict_paths: vec!["conflict.txt".to_owned()]
            }
        );
        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
        assert_eq!(
            observed
                .pending_action
                .as_ref()
                .map(|pending| pending.state),
            Some(super::super::PendingActionObservationState::ExpectedConflict)
        );
        assert!(observed.abort_eligibility.eligible);
        assert_eq!(observed.conflict_paths, vec!["conflict.txt"]);

        fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
        run_git(&repo, &["add", "conflict.txt"]);
        record.state = ParticipantState::Conflicted;
        record.expected_merge_head = Some(source.clone());
        record.pending_action.as_mut().unwrap().kind = PendingMergeActionKind::ResolveConflict;
        let prepared = backend
            .prepare_merge_resolution_checked(&repo, &before, &source, None)
            .unwrap();
        set_prepared_commit(&mut record, &prepared);
        assert_eq!(
            reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
            PendingActionReconciliation::NotStarted
        );
        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
        assert_eq!(
            observed
                .pending_action
                .as_ref()
                .map(|pending| pending.state),
            Some(super::super::PendingActionObservationState::NotStarted)
        );
        assert!(observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);

        let committed = backend
            .commit_prepared_merge_resolution_checked(&repo, &before, &source, message, &prepared)
            .unwrap();
        assert_eq!(
            reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
            PendingActionReconciliation::Completed {
                resulting_commit: committed.commit
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolution_candidate_with_different_tree_is_never_adopted_or_rollback_eligible() {
        let root = test_root("reconcile-resolution-different-tree");
        let repo = root.join("repos/app");
        let backend = Git2Backend::new();
        backend.create_repo(&repo).unwrap();
        commit(&repo, "conflict.txt", "base\n", "base");
        run_git(&repo, &["branch", "feature"]);
        run_git(&repo, &["checkout", "feature"]);
        let source = commit(&repo, "conflict.txt", "source\n", "source");
        run_git(&repo, &["checkout", "main"]);
        let before = commit(&repo, "conflict.txt", "main\n", "main");
        let conflict = backend
            .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
            .unwrap();
        backend
            .execute_prepared_merge_upstream_checked(
                &repo,
                "main",
                &before,
                &source,
                "resolution message",
                &conflict,
            )
            .unwrap();
        fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
        run_git(&repo, &["add", "conflict.txt"]);
        let prepared = backend
            .prepare_merge_resolution_checked(&repo, &before, &source, None)
            .unwrap();
        let candidate = alternate_merge_commit(
            &repo,
            &before,
            &source,
            "resolution message",
            &prepared,
            CandidateDifference::Tree,
        );
        run_git(&repo, &["reset", "--hard", &candidate]);
        let mut record = pending_record(
            ParticipantState::Conflicted,
            &before,
            &source,
            "resolution message",
            PendingMergeActionKind::ResolveConflict,
        );
        record.expected_merge_head = Some(source);
        set_prepared_commit(&mut record, &prepared);
        let index_before = fs::read(repo.join(".git/index")).unwrap();

        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

        assert_eq!(
            observed.pending_action.unwrap().state,
            super::super::PendingActionObservationState::Ambiguous
        );
        assert!(!observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);
        assert_eq!(backend.head(&repo).unwrap().commit, Some(candidate));
        assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
        assert!(repo.join("post-intent.txt").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ambiguous_pending_inputs_are_structured_and_block_recovery() {
        let root = test_root("reconcile-ambiguous");
        let repo = root.join("repos/app");
        let backend = Git2Backend::new();
        backend.create_repo(&repo).unwrap();
        let before = commit(&repo, "tracked.txt", "one\n", "initial");
        let mut record = pending_record(
            ParticipantState::Planned,
            &before,
            &before,
            "frozen",
            PendingMergeActionKind::FastForward,
        );
        record.pending_action.as_mut().unwrap().target_branch = "other".to_owned();

        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

        let pending = observed.pending_action.unwrap();
        assert_eq!(
            pending.state,
            super::super::PendingActionObservationState::Ambiguous
        );
        assert!(pending.message.unwrap().contains("do not match"));
        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::PendingActionAmbiguous)
        );
        assert!(!observed.continue_eligibility.eligible);
        assert!(!observed.abort_eligibility.eligible);
        fs::remove_dir_all(root).unwrap();
    }
}
