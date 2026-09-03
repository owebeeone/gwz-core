use super::super::*;
use super::verify_finalization_recovery_origin;
use crate::git::{GitMergeAnalysisKind, GitPreparedMergeMode, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace_ops::merge::MergeExecutionMode;
use crate::workspace_ops::merge::integration::{IntegrationIntent, PreparedIntegration};
use crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1;
use crate::workspace_ops::merge::status::PendingActionReconciliation;
use crate::workspace_ops::merge::{
    ConflictFileEvidence, MergeParticipantRecord, MergeRecordError, ParticipantState,
    PendingMergeAction, PendingMergeActionKind,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_forward<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    let fact = match request.kind() {
        ObservationKind::ParticipantPreparation { member_id } => {
            prepare_participant(backend, context, current, member_id)?
        }
        ObservationKind::ParticipantAction { member_id } => {
            observe_participant_action(backend, current, member_id)?
        }
        ObservationKind::Recovery => observe_recovery(backend, current)?,
        _ => {
            return Err(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "v1 forward runtime received a non-forward observation",
            ));
        }
    };
    BoundExactObservation::issue(current, request, fact)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_participant_action<
    B: MergeAuthorityBackend,
>(
    backend: &B,
    current: &StoredV1Record,
    member_id: &str,
    expected: &PendingMergeAction,
) -> ModelResult<()> {
    let row = participant(current, member_id)?;
    if row.pending_action.as_ref() != Some(expected) {
        return Err(member_error(
            ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "persisted participant action no longer matches its execution authority",
            ),
            member_id,
            &row.path,
        ));
    }
    match crate::workspace_ops::merge::status::reconcile_pending_action(
        backend,
        current.location().root(),
        member_id,
        row,
    )? {
        PendingActionReconciliation::NotStarted => Ok(()),
        PendingActionReconciliation::ExpectedConflict { .. }
        | PendingActionReconciliation::Completed { .. } => Err(member_error(
            ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "participant action already has an observable result and must be reconciled",
            ),
            member_id,
            &row.path,
        )),
        PendingActionReconciliation::Ambiguous { reason, .. } => Err(member_error(
            ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("participant action is ambiguous: {reason}"),
            ),
            member_id,
            &row.path,
        )),
    }
}

/// Refuse a continue as a UNIT when any selected participant is not ready.
///
/// v0 parity (`continue_op::execution::preflight`, deleted in M5d(3) —
/// recover with `git show 57502e4:src/workspace_ops/merge/continue_op/execution.rs`):
/// a continue validated the continue-eligibility of EVERY selected participant
/// ONCE, up front, and propagated the first refusal with `?` — so the whole
/// operation refused before its execution loop made a single durable mutation,
/// and no participant was left half-advanced by a sibling's bad evidence.
///
/// v1's runtime is a step machine: it observes, commits, executes and observes
/// ONE participant at a time (`dispatcher::next_action`'s `Continue` arm picks
/// the first conflicted participant; `service::run_with_runtime` commits each
/// resolved observation immediately). Without this gate participant N is
/// durably merged and its native merge state cleared BEFORE participant N+1's
/// evidence is read at all, so a refusal that must be atomic across the
/// participant set instead leaves the earlier participants mutated.
///
/// This restores v0's atomicity at v0's exact point: after pending-action
/// reconciliation (v1 reconciles a durable `pending_action` through a
/// `ParticipantAction` observation, which `next_action` always dispatches
/// before any `ParticipantPreparation`) and before the first preparation,
/// carrying v0's exact code and member attribution.
///
/// SCOPED TO THE SIBLINGS. `owner` is the participant this continue's first
/// observation already selects, and v1 deliberately owns its outcome
/// differently from v0: the observation itself routes the owner's semantic
/// drift into a durable `RecoveryRequired` transition (`prepare_participant`'s
/// `semantic_drift` arm -> `finalization::ambiguity`) and its hard preparation
/// failure into a recorded `Halted` participant error (`preparation_failure`).
/// Those two are designed v1 behaviour with their own suites
/// (`v1_lifecycle::tests::forward::semantic_preparation_drift_enters_executing_recovery_before_owner_or_git_mutation`
/// and `::symlinked_member_directory_is_rejected_before_git_execution`) and
/// this gate must not preempt them. What was missing is the SIBLINGS' evidence:
/// v1 already refuses cross-participant drift on the recovery path
/// (`verify_forward_recovery_origin`'s `selected_targets` loop below, and
/// `tests::forward::recovery_with_an_exact_owner_rejects_drift_in_another_selected_participant`),
/// and this is that same property for the continue path, where it was absent.
pub(in crate::workspace_ops::merge::v1_lifecycle) fn preflight_continue_siblings<
    B: MergeAuthorityBackend,
>(
    backend: &B,
    current: &StoredV1Record,
    owner: &str,
) -> ModelResult<()> {
    let record = current.record();
    for member_id in &record.selected_targets {
        if member_id == owner {
            continue;
        }
        let row = participant(current, member_id)?;
        let observed = crate::workspace_ops::merge::status::observe_participant(
            backend,
            current.location().root(),
            member_id,
            row,
        )?;
        if !observed.continue_eligibility.eligible {
            return Err(member_error(
                ModelError::new(
                    ErrorCode::MergeDrift,
                    format!(
                        "participant is not ready to continue; blockers: {:?}",
                        observed.continue_eligibility.blockers
                    ),
                ),
                member_id,
                &row.path,
            ));
        }
    }
    Ok(())
}

fn prepare_participant<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    member_id: &str,
) -> ModelResult<ExactObservationFact> {
    let row = participant(current, member_id)?;
    match prepare_pending(backend, context, current, member_id, row) {
        Ok(pending) => {
            let mut prepared = row.clone();
            prepared.pending_action = Some(pending);
            let proof = PreparedParticipantAction::issue(
                &AuthorityIssuer::for_observer(current),
                member_id,
                "prepare_participant",
                "prepared",
                ParticipantActionPayload {
                    member_id: member_id.into(),
                    row: prepared,
                },
            )?;
            Ok(completed(CompletedObservation::Participant(
                ParticipantObservation::Prepared(Box::new(proof)),
            )))
        }
        Err(error) if semantic_drift(&error) => super::finalization::ambiguity(current),
        Err(error) if row.state == ParticipantState::Conflicted => {
            Err(member_error(error, member_id, &row.path))
        }
        Err(error) => preparation_failure(current, member_id, row, error),
    }
}

fn prepare_pending<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    member_id: &str,
    row: &MergeParticipantRecord,
) -> ModelResult<PendingMergeAction> {
    let observation = crate::workspace_ops::merge::status::observe_participant(
        backend,
        current.location().root(),
        member_id,
        row,
    )?;
    if row.state == ParticipantState::Conflicted
        && observation.drift.is_empty()
        && !observation.continue_eligibility.eligible
    {
        return Err(ModelError::new(
            ErrorCode::MergeValidationFailed,
            "conflict resolution is not ready; resolve and stage every conflict before continuing",
        ));
    }
    if !observation.drift.is_empty() || !observation.continue_eligibility.eligible {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "participant is not ready for its exact forward action; blockers: {:?}",
                observation.continue_eligibility.blockers
            ),
        ));
    }
    let path = crate::workspace_ops::merge::status::validated_participant_path(
        current.location().root(),
        member_id,
        row,
    )?;
    let intent = IntegrationIntent::from_record(row);
    let prepared = if row.state == ParticipantState::Conflicted {
        let merge_head = row
            .expected_merge_head
            .as_deref()
            .unwrap_or(&row.source_commit);
        let commit = backend.prepare_merge_resolution_checked(
            &path,
            &row.target_branch,
            &row.before_commit,
            merge_head,
            context.attribution.as_ref(),
        )?;
        PreparedIntegration::resolution(intent, &commit)
    } else if matches!(
        row.state,
        ParticipantState::Planned | ParticipantState::Failed | ParticipantState::Unattempted
    ) {
        if !backend.commit_exists(&path, &row.source_commit)? {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "recorded merge source commit is not available locally",
            ));
        }
        let analysis = backend.merge_analysis(&path, &row.target_branch, &row.source_commit)?;
        if analysis.target_branch != row.target_branch
            || analysis.target_commit != row.before_commit
            || analysis.source_commit != row.source_commit
        {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "recorded merge inputs no longer match the live repository",
            ));
        }
        validate_mode(current.record().mode, analysis.kind)?;
        let mode = prepared_mode(current.record().mode);
        let result = backend.prepare_merge_upstream_mode_checked(
            &path,
            &row.target_branch,
            &row.before_commit,
            &row.source_commit,
            mode,
            context.attribution.as_ref(),
        )?;
        let effective_kind = if mode == GitPreparedMergeMode::ForceMergeCommit
            && analysis.kind == GitMergeAnalysisKind::FastForward
        {
            GitMergeAnalysisKind::TrueMerge
        } else {
            analysis.kind
        };
        PreparedIntegration::from_merge(intent, effective_kind, &result).map_err(|reason| {
            ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("prepared merge result is inconsistent: {reason}"),
            )
        })?
    } else {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("participant state {:?} has no forward action", row.state),
        ));
    };
    Ok(prepared.to_pending())
}

fn validate_mode(mode: MergeExecutionMode, kind: GitMergeAnalysisKind) -> ModelResult<()> {
    match (mode, kind) {
        (MergeExecutionMode::FfOnly, GitMergeAnalysisKind::TrueMerge) => Err(ModelError::new(
            ErrorCode::MergeValidationFailed,
            "ff_only cannot prepare a true merge",
        )),
        _ => Ok(()),
    }
}

fn prepared_mode(mode: MergeExecutionMode) -> GitPreparedMergeMode {
    if mode == MergeExecutionMode::NoFf {
        GitPreparedMergeMode::ForceMergeCommit
    } else {
        GitPreparedMergeMode::AllowFastForward
    }
}

fn preparation_failure(
    current: &StoredV1Record,
    member_id: &str,
    row: &MergeParticipantRecord,
    error: ModelError,
) -> ModelResult<ExactObservationFact> {
    let contextual = member_error(error, member_id, &row.path);
    let mut failed = row.clone();
    failed.state = ParticipantState::Failed;
    failed.resulting_commit = None;
    failed.expected_merge_head = None;
    failed.conflict_paths.clear();
    failed.conflict_snapshot.clear();
    failed.pending_action = None;
    failed.error = Some(MergeRecordError {
        code: contextual.code,
        message: contextual.message,
        detail: None,
    });
    let later_unattempted = later_planned(current, member_id);
    let proof = PreparedFailureHaltBatch::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "preparation_failure",
        "verified",
        ParticipantFailurePayload {
            member_id: member_id.into(),
            row: failed,
            later_unattempted,
        },
    )?;
    Ok(completed(CompletedObservation::Participant(
        ParticipantObservation::PreparationFailed(Box::new(proof)),
    )))
}

fn observe_participant_action<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    member_id: &str,
) -> ModelResult<ExactObservationFact> {
    let row = participant(current, member_id)?;
    let pending = row.pending_action.as_ref().ok_or_else(|| {
        member_error(
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "participant action observation has no persisted action",
            ),
            member_id,
            &row.path,
        )
    })?;
    match crate::workspace_ops::merge::status::reconcile_pending_action(
        backend,
        current.location().root(),
        member_id,
        row,
    )? {
        PendingActionReconciliation::NotStarted
            if pending.kind == PendingMergeActionKind::VerifyUpToDate =>
        {
            participant_outcome(
                current,
                member_id,
                outcome_row(
                    row,
                    ParticipantState::UpToDate,
                    Some(row.before_commit.clone()),
                    Vec::new(),
                    Vec::new(),
                ),
            )
        }
        PendingActionReconciliation::NotStarted => Ok(ExactObservationFact::NotStarted(
            NotStartedObservation::Participant {
                member_id: member_id.into(),
                action: Box::new(pending.clone()),
            },
        )),
        PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
            if pending.kind != PendingMergeActionKind::TrueMerge {
                return super::finalization::ambiguity(current);
            }
            let path = crate::workspace_ops::merge::status::validated_participant_path(
                current.location().root(),
                member_id,
                row,
            )?;
            let snapshot = match backend.merge_conflict_snapshot(
                &path,
                &row.before_commit,
                &row.source_commit,
            ) {
                Ok(snapshot) => snapshot
                    .files
                    .into_iter()
                    .map(|file| ConflictFileEvidence {
                        path: file.path,
                        sha256: file.sha256,
                    })
                    .collect(),
                Err(error) if semantic_drift(&error) => {
                    return super::finalization::ambiguity(current);
                }
                Err(error) => return Err(member_error(error, member_id, &row.path)),
            };
            participant_outcome(
                current,
                member_id,
                outcome_row(
                    row,
                    ParticipantState::Conflicted,
                    None,
                    conflict_paths,
                    snapshot,
                ),
            )
        }
        PendingActionReconciliation::Completed { resulting_commit } => {
            let state = match pending.kind {
                PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
            };
            participant_outcome(
                current,
                member_id,
                outcome_row(row, state, Some(resulting_commit), Vec::new(), Vec::new()),
            )
        }
        PendingActionReconciliation::Ambiguous { .. } => super::finalization::ambiguity(current),
    }
}

fn participant_outcome(
    current: &StoredV1Record,
    member_id: &str,
    row: MergeParticipantRecord,
) -> ModelResult<ExactObservationFact> {
    let proof = VerifiedParticipantOutcome::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: member_id.into(),
            row,
        },
    )?;
    Ok(completed(CompletedObservation::Participant(
        ParticipantObservation::Outcome(Box::new(proof), EntryFact::None),
    )))
}

fn outcome_row(
    prior: &MergeParticipantRecord,
    state: ParticipantState,
    resulting_commit: Option<String>,
    conflict_paths: Vec<String>,
    conflict_snapshot: Vec<ConflictFileEvidence>,
) -> MergeParticipantRecord {
    let mut row = prior.clone();
    row.state = state;
    row.resulting_commit = resulting_commit;
    row.expected_merge_head =
        (state == ParticipantState::Conflicted).then(|| prior.source_commit.clone());
    row.conflict_paths = conflict_paths;
    row.conflict_snapshot = conflict_snapshot;
    row.error = None;
    row.pending_action = None;
    row
}

fn semantic_drift(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::DirtyMember
            | ErrorCode::MergeDrift
            | ErrorCode::MergeRecoveryRequired
            | ErrorCode::AcceptanceInputDrift
    )
}

fn observe_recovery<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let context = current.record().recovery_context.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "recovery-required record has no exact origin context",
        )
    })?;
    match context.origin_state {
        RecoveryOriginStateV1::Executing
        | RecoveryOriginStateV1::AwaitingResolution
        | RecoveryOriginStateV1::Halted => {
            verify_forward_recovery_origin(backend, current, context.origin_state)?;
        }
        RecoveryOriginStateV1::Finalizing => {
            verify_finalization_recovery_origin(backend, current)?;
        }
        RecoveryOriginStateV1::Preserving | RecoveryOriginStateV1::RollingBack => {
            return Err(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "reverse-lifecycle recovery verification is owned by the preservation/rollback phase",
            ));
        }
    }
    let proof = VerifiedRecoveryOrigin::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "resume_recovery",
        "verified",
        context.origin_state,
    )?;
    Ok(completed(CompletedObservation::Recovery(proof)))
}

fn verify_forward_recovery_origin<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    origin: RecoveryOriginStateV1,
) -> ModelResult<()> {
    let record = current.record();
    let pending = record
        .selected_targets
        .iter()
        .find_map(|id| record.participants[id].pending_action.as_ref().map(|_| id));
    if (pending.is_some() && origin == RecoveryOriginStateV1::AwaitingResolution)
        || (origin == RecoveryOriginStateV1::Halted && !has_halt_cause(record))
    {
        return Err(ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "durable participant owner does not match the recorded recovery origin",
        ));
    }
    if let Some(member_id) = pending {
        let row = &record.participants[member_id];
        match crate::workspace_ops::merge::status::reconcile_pending_action(
            backend,
            current.location().root(),
            member_id,
            row,
        )? {
            PendingActionReconciliation::NotStarted
            | PendingActionReconciliation::Completed { .. } => Ok(()),
            PendingActionReconciliation::ExpectedConflict { .. } => {
                let path = crate::workspace_ops::merge::status::validated_participant_path(
                    current.location().root(),
                    member_id,
                    row,
                )?;
                backend
                    .merge_conflict_snapshot(&path, &row.before_commit, &row.source_commit)
                    .map(|_| ())
                    .map_err(|error| recovery_mismatch(error, member_id, &row.path))
            }
            PendingActionReconciliation::Ambiguous { reason, .. } => Err(member_error(
                ModelError::new(ErrorCode::RecoveryEvidenceMismatch, reason),
                member_id,
                &row.path,
            )),
        }?;
    }

    let shape_matches = match origin {
        RecoveryOriginStateV1::Executing => true,
        RecoveryOriginStateV1::AwaitingResolution => record.participants.values().any(|row| {
            row.state == ParticipantState::Conflicted
                && row.pending_action.is_none()
                && row.error.is_none()
        }),
        RecoveryOriginStateV1::Halted => has_halt_cause(record),
        RecoveryOriginStateV1::Finalizing
        | RecoveryOriginStateV1::Preserving
        | RecoveryOriginStateV1::RollingBack => false,
    };
    if !shape_matches {
        return Err(ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "durable participant state does not match the recorded recovery origin",
        ));
    }
    for member_id in &record.selected_targets {
        if pending == Some(member_id) {
            continue;
        }
        let row = &record.participants[member_id];
        let observed = crate::workspace_ops::merge::status::observe_participant(
            backend,
            current.location().root(),
            member_id,
            row,
        )?;
        let ready = observed.drift.is_empty()
            && match row.state {
                ParticipantState::Planned
                | ParticipantState::Failed
                | ParticipantState::Unattempted => observed.continue_eligibility.eligible,
                ParticipantState::Conflicted => observed.abort_eligibility.eligible,
                ParticipantState::UpToDate
                | ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued => true,
                ParticipantState::Aborted | ParticipantState::RolledBack => false,
            };
        if !ready {
            return Err(member_error(
                ModelError::new(
                    ErrorCode::RecoveryEvidenceMismatch,
                    "live participant state does not exactly match the recorded recovery origin",
                ),
                member_id,
                &row.path,
            ));
        }
    }
    Ok(())
}

fn recovery_mismatch(error: ModelError, member_id: &str, path: &str) -> ModelError {
    if semantic_drift(&error) {
        member_error(
            ModelError::new(ErrorCode::RecoveryEvidenceMismatch, error.message),
            member_id,
            path,
        )
    } else {
        member_error(error, member_id, path)
    }
}

fn later_planned(current: &StoredV1Record, member_id: &str) -> Vec<String> {
    let record = current.record();
    let start = record
        .selected_targets
        .iter()
        .position(|id| id == member_id)
        .map_or(record.selected_targets.len(), |index| index + 1);
    record.selected_targets[start..]
        .iter()
        .filter(|id| {
            record
                .participants
                .get(*id)
                .is_some_and(|row| row.state == ParticipantState::Planned)
        })
        .cloned()
        .collect()
}

fn participant<'a>(
    current: &'a StoredV1Record,
    member_id: &str,
) -> ModelResult<&'a MergeParticipantRecord> {
    current.record().participants.get(member_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{member_id}'"),
        )
    })
}

fn completed(value: CompletedObservation) -> ExactObservationFact {
    ExactObservationFact::Completed(value)
}

fn member_error(mut error: ModelError, member_id: &str, path: &str) -> ModelError {
    if error.member_id.is_none() {
        error = error.with_member(member_id, path);
    }
    error
}
