use std::path::{Path, PathBuf};

use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::MemberPath;

use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, MergeTargetKind, ParticipantDriftKind,
    RetryEligibility, RollbackEligibility, participant_semantics,
};
use super::*;

pub(in crate::workspace_ops::merge) fn observe_participant<B: GitBackend>(
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
    } else if participant_semantics::status::status_policy(participant.state).conflict_role
        == participant_semantics::status::ConflictRole::NativeMerge
        && observation.drift.is_empty()
    {
        let merge_head = participant
            .expected_merge_head
            .as_deref()
            .unwrap_or(&participant.source_commit);
        let abort = match backend.validate_merge_recovery_state(
            &path,
            &participant.before_commit,
            merge_head,
            false,
        ) {
            Ok(()) => participant_semantics::status::ConflictValidationOutcome::Valid,
            Err(error) => {
                participant_semantics::status::ConflictValidationOutcome::Invalid(error.message)
            }
        };
        let resolution = if abort == participant_semantics::status::ConflictValidationOutcome::Valid
            && observation.continue_eligibility.eligible
        {
            match backend.validate_merge_recovery_state(
                &path,
                &participant.before_commit,
                merge_head,
                true,
            ) {
                Ok(()) => participant_semantics::status::ConflictValidationOutcome::Valid,
                Err(error) => {
                    participant_semantics::status::ConflictValidationOutcome::Invalid(error.message)
                }
            }
        } else {
            participant_semantics::status::ConflictValidationOutcome::NotChecked
        };
        participant_semantics::status::apply_conflict_validation(
            target_id,
            participant,
            &live,
            participant_semantics::status::ConflictValidationOutcomes { abort, resolution },
            &mut observation,
        );
    }
    Ok(observation)
}

pub(super) fn apply_pending_observation(
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
            if kind == super::super::PendingMergeActionKind::ResolveConflict {
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
                super::super::PendingActionObservationState::NotStarted,
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
                super::super::PendingActionObservationState::ExpectedConflict,
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
                super::super::PendingActionObservationState::CompletedExactly,
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
                super::super::PendingActionObservationState::Ambiguous,
                Some(reason),
            )
        }
    };
    observation.pending_action = Some(super::super::PendingActionObservation {
        kind,
        state,
        message,
    });
}

pub(super) fn read_live_participant<B: GitBackend>(
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

pub(in crate::workspace_ops::merge) fn validated_participant_path(
    root: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<PathBuf> {
    let path = match participant.target_kind {
        MergeTargetKind::Root if participant.path == "." => return Ok(root.to_path_buf()),
        MergeTargetKind::Root => Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root participant path must be '.'",
        )),
        MergeTargetKind::Member => MemberPath::parse(&participant.path)
            .map(|path| path.to_string())
            .map_err(|error| ModelError::new(ErrorCode::MergeRecordUnreadable, error.message)),
    }
    .map_err(|error| {
        ModelError::new(
            error.code,
            format!("invalid durable participant path: {}", error.message),
        )
        .with_member(target_id, &participant.path)
    })?;
    let mut candidate = root.to_path_buf();
    for component in Path::new(&path).components() {
        candidate.push(component);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(ModelError::new(
                    ErrorCode::PathEscape,
                    "invalid durable participant path: durable participant path crosses a symlink or non-directory",
                )
                .with_member(target_id, &participant.path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(ModelError::new(
                    ErrorCode::IoError,
                    format!("invalid durable participant path: {error}"),
                )
                .with_member(target_id, &participant.path));
            }
        }
    }
    Ok(root.join(path))
}

pub(super) fn member_result<T>(
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
