use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::{GitBackend, GitNativeMergeState, GitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::WORKSPACE_MANIFEST;

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

pub(crate) fn snapshot_status<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
) -> ModelResult<MergeStatusSnapshot> {
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
    let path = root.join(&participant.path);
    if !path.is_dir() || !backend.is_repository(&path)? {
        return Ok(missing_observation(target_id, participant));
    }
    let expected_head = expected_head(participant)?;
    let head = backend.head(&path)?;
    let target_ref =
        backend.read_ref(&path, &format!("refs/heads/{}", participant.target_branch))?;
    let relation = match head.commit.as_deref() {
        Some(live) if live == expected_head => HeadRelation::Equal,
        Some(live) if backend.is_ancestor(&path, expected_head, live)? => HeadRelation::Advanced,
        Some(live) if backend.is_ancestor(&path, live, expected_head)? => HeadRelation::Rewound,
        Some(_) => HeadRelation::Diverged,
        None => HeadRelation::Missing,
    };
    let live = ParticipantLiveState {
        branch: head.branch,
        head: head.commit,
        target_ref,
        status: backend.status(&path)?,
        merge_state: backend.merge_state(&path)?,
        head_relation: relation,
    };
    let mut observation = classify_participant(target_id, participant, &live);
    if participant.state == ParticipantState::Conflicted && observation.drift.is_empty() {
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
    pub merge_state: Option<GitNativeMergeState>,
    head_relation: HeadRelation,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeadRelation {
    Equal,
    Advanced,
    Rewound,
    Diverged,
    Missing,
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
        let mut add = |kind, guidance| {
            drift.push(participant_drift(
                kind,
                target_id,
                participant,
                live,
                guidance,
            ));
        };
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
                HeadRelation::Rewound | HeadRelation::Missing => ParticipantDriftKind::HeadRewound,
                _ => ParticipantDriftKind::HeadAdvanced,
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
        if conflicted {
            match &live.merge_state {
                None => add(
                    ParticipantDriftKind::MergeStateMissing,
                    "restore the recorded native merge state or follow abort recovery guidance",
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
        } else if live.merge_state.is_some() {
            add(
                ParticipantDriftKind::NewIntegrationState,
                "finish or abort the unrelated integration before merge recovery",
            );
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
        drift.is_empty() && live.merge_state.is_none() && !live.status.is_dirty
    };
    let no_abort_action = does_not_require_rollback(participant.state);
    let abort_eligible = no_abort_action
        || if conflicted {
            native_matches && drift.is_empty()
        } else {
            drift.is_empty() && live.merge_state.is_none() && !live.status.is_dirty
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
    let drift = ParticipantDrift {
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
    MergeParticipantObservation {
        live_commit: None,
        conflict_paths: Vec::new(),
        drift: vec![drift],
        continue_eligibility: RetryEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::RepositoryMissing],
        },
        abort_eligibility: RollbackEligibility {
            eligible: does_not_require_rollback(participant.state),
            blockers: (!does_not_require_rollback(participant.state))
                .then_some(ParticipantDriftKind::RepositoryMissing)
                .into_iter()
                .collect(),
        },
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
            merge_state: None,
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
            merge_state: None,
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
}
