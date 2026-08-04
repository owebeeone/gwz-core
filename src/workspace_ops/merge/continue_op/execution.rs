use super::super::participant_semantics::continue_eligibility::{
    ContinueDisposition, continue_disposition, post_continue_state,
};
use super::*;

pub(super) fn preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<Vec<ContinueAction>> {
    let snapshot = super::super::status::snapshot_status(backend, root, record.clone())?;
    if let Some(drift) = snapshot.operation_drift.first() {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            drift.message.clone(),
        ));
    }

    let mut actions = Vec::new();
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        let observed = snapshot.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge status is missing participant '{target_id}'"),
            )
        })?;
        if !observed.continue_eligibility.eligible {
            let reason = format!(
                "participant is not ready to continue; blockers: {:?}",
                observed.continue_eligibility.blockers
            );
            return Err(ModelError::new(ErrorCode::MergeDrift, reason)
                .with_member(target_id, &participant.path));
        }
        if let Some(pending) = participant.pending_action.as_ref() {
            let prepared = super::super::integration::decode_for_participant(pending, participant)
                .map_err(|reason| {
                    ModelError::new(ErrorCode::MergeRecoveryRequired, reason)
                        .with_member(target_id, &participant.path)
                })?;
            let (kind, prepared) = durable_continue_action(participant, prepared)
                .map_err(|error| error.with_member(target_id, &participant.path))?;
            actions.push(ContinueAction {
                target_id: target_id.clone(),
                path: participant.path.clone(),
                kind,
                prepared,
                durable: true,
            });
            continue;
        }
        match continue_disposition(participant.state) {
            ContinueDisposition::ResolveConflict => {
                let merge_head = participant
                    .expected_merge_head
                    .as_deref()
                    .unwrap_or(&participant.source_commit);
                let prepared = backend
                    .prepare_merge_resolution_checked(
                        &root.join(&participant.path),
                        &participant.target_branch,
                        &participant.before_commit,
                        merge_head,
                        attribution,
                    )
                    .map_err(|error| error.with_member(target_id, &participant.path))?;
                actions.push(ContinueAction {
                    target_id: target_id.clone(),
                    path: participant.path.clone(),
                    kind: ContinueActionKind::Resolve,
                    prepared: ContinuePrepared::Resolution(prepared),
                    durable: false,
                });
            }
            ContinueDisposition::RetryIntegration => {
                let path = root.join(&participant.path);
                if !backend
                    .commit_exists(&path, &participant.source_commit)
                    .map_err(|error| error.with_member(target_id, &participant.path))?
                {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        "recorded merge source commit is not available locally",
                    )
                    .with_member(target_id, &participant.path));
                }
                let analysis = backend
                    .merge_analysis(
                        &path,
                        &participant.target_branch,
                        &participant.source_commit,
                    )
                    .map_err(|error| error.with_member(target_id, &participant.path))?;
                if analysis.target_branch != participant.target_branch
                    || analysis.target_commit != participant.before_commit
                    || analysis.source_commit != participant.source_commit
                {
                    return Err(ModelError::new(
                        ErrorCode::MergeDrift,
                        "recorded merge plan no longer matches the repository",
                    )
                    .with_member(target_id, &participant.path));
                }
                actions.push(ContinueAction {
                    target_id: target_id.clone(),
                    path: participant.path.clone(),
                    kind: ContinueActionKind::Retry(analysis.kind),
                    prepared: ContinuePrepared::Merge(
                        backend
                            .prepare_merge_upstream_checked(
                                &path,
                                &participant.target_branch,
                                &participant.before_commit,
                                &participant.source_commit,
                                attribution,
                            )
                            .map_err(|error| error.with_member(target_id, &participant.path))?,
                    ),
                    durable: false,
                });
            }
            ContinueDisposition::Settled => {}
            ContinueDisposition::RejectedTerminal => {
                return Err(wrong_participant_state(target_id, participant));
            }
        }
    }
    Ok(actions)
}

fn durable_continue_action(
    participant: &MergeParticipantRecord,
    prepared: super::super::integration::PreparedIntegration,
) -> ModelResult<(ContinueActionKind, ContinuePrepared)> {
    use super::super::integration::PreparedIntegrationAction as Action;

    match continue_disposition(participant.state) {
        ContinueDisposition::ResolveConflict => match prepared.action {
            Action::ResolveConflict(prepared) => Ok((
                ContinueActionKind::Resolve,
                ContinuePrepared::Resolution(prepared),
            )),
            Action::VerifyUpToDate
            | Action::FastForward
            | Action::TrueMergeExpectedConflict
            | Action::TrueMergeCommit(_) => Err(invariant(
                "pending action kind does not match the participant recovery state",
            )),
        },
        ContinueDisposition::RetryIntegration => {
            let (kind, prepared) = match prepared.action {
                Action::VerifyUpToDate => {
                    (GitMergeAnalysisKind::UpToDate, GitPreparedMerge::Unchanged)
                }
                Action::FastForward => (
                    GitMergeAnalysisKind::FastForward,
                    GitPreparedMerge::FastForward,
                ),
                Action::TrueMergeExpectedConflict => (
                    GitMergeAnalysisKind::TrueMerge,
                    GitPreparedMerge::ExpectedConflict,
                ),
                Action::TrueMergeCommit(prepared) => (
                    GitMergeAnalysisKind::TrueMerge,
                    GitPreparedMerge::Commit(prepared),
                ),
                Action::ResolveConflict(_) => {
                    return Err(invariant(
                        "pending action kind does not match the participant recovery state",
                    ));
                }
            };
            Ok((
                ContinueActionKind::Retry(kind),
                ContinuePrepared::Merge(prepared),
            ))
        }
        ContinueDisposition::Settled | ContinueDisposition::RejectedTerminal => Err(invariant(
            "pending action kind does not match the participant recovery state",
        )),
    }
}

pub(super) fn resolve_conflict<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    action: &ContinueAction,
    _context: &OperationContext,
) -> ModelResult<Outcome> {
    let participant = participant(record, &action.target_id)?;
    let merge_head = participant
        .expected_merge_head
        .as_deref()
        .unwrap_or(&participant.source_commit);
    let ContinuePrepared::Resolution(prepared) = &action.prepared else {
        return Err(invariant("resolution action has no prepared commit"));
    };
    let commit = backend.commit_prepared_merge_resolution_checked(
        &root.join(&participant.path),
        &participant.target_branch,
        &participant.before_commit,
        merge_head,
        &participant.commit_message,
        prepared,
    )?;
    Ok(Outcome::clean(ParticipantState::Continued, commit.commit))
}

pub(super) fn retry_merge<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    action: &ContinueAction,
    kind: GitMergeAnalysisKind,
    _context: &OperationContext,
) -> Result<Outcome, ActionFailure> {
    let participant = participant(record, &action.target_id).map_err(ActionFailure::Ordinary)?;
    let ContinuePrepared::Merge(prepared) = &action.prepared else {
        return Err(ActionFailure::Ordinary(invariant(
            "retry action has no prepared merge",
        )));
    };
    let result = backend
        .execute_prepared_merge_upstream_checked(
            &root.join(&participant.path),
            &participant.target_branch,
            &participant.before_commit,
            &participant.source_commit,
            &participant.commit_message,
            prepared,
        )
        .map_err(ActionFailure::Ordinary)?;
    let mut outcome = classify_retry(participant, kind, result).map_err(ActionFailure::Ordinary)?;
    if outcome.state == ParticipantState::Conflicted {
        outcome.conflict_snapshot = backend
            .merge_conflict_snapshot(
                &root.join(&participant.path),
                &participant.before_commit,
                &participant.source_commit,
            )
            .map_err(ActionFailure::RecoveryRequired)?
            .files
            .into_iter()
            .map(|file| ConflictFileEvidence {
                path: file.path,
                sha256: file.sha256,
            })
            .collect();
    }
    Ok(outcome)
}

pub(super) fn classify_retry(
    participant: &MergeParticipantRecord,
    kind: GitMergeAnalysisKind,
    result: GitIntegrateResult,
) -> ModelResult<Outcome> {
    if !result.conflicts.is_empty() {
        if kind != GitMergeAnalysisKind::TrueMerge || result.commit.is_some() {
            return Err(invariant("backend returned an invalid conflict result"));
        }
        return Ok(Outcome {
            state: ParticipantState::Conflicted,
            resulting_commit: None,
            expected_merge_head: Some(participant.source_commit.clone()),
            conflict_paths: result.conflicts,
            conflict_snapshot: Vec::new(),
        });
    }
    let commit = result
        .commit
        .ok_or_else(|| invariant("clean retry omitted its resulting commit"))?;
    let state = match kind {
        GitMergeAnalysisKind::UpToDate if commit == participant.before_commit => {
            ParticipantState::UpToDate
        }
        GitMergeAnalysisKind::FastForward if commit == participant.source_commit => {
            ParticipantState::FastForwarded
        }
        GitMergeAnalysisKind::TrueMerge => ParticipantState::Merged,
        _ => return Err(invariant("retry produced the wrong resulting commit")),
    };
    Ok(Outcome::clean(state, commit))
}

#[derive(Debug)]
pub(super) struct Outcome {
    pub(super) state: ParticipantState,
    pub(super) resulting_commit: Option<String>,
    pub(super) expected_merge_head: Option<String>,
    pub(super) conflict_paths: Vec<String>,
    pub(super) conflict_snapshot: Vec<ConflictFileEvidence>,
}

impl Outcome {
    pub(super) fn clean(state: ParticipantState, commit: String) -> Self {
        Self {
            state,
            resulting_commit: Some(commit),
            expected_merge_head: None,
            conflict_paths: Vec::new(),
            conflict_snapshot: Vec::new(),
        }
    }
}

pub(super) fn apply_outcome(
    record: &mut MergeOperationRecord,
    target_id: &str,
    outcome: Outcome,
    error: Option<MergeRecordError>,
) -> ModelResult<()> {
    let participant = record.participants.get_mut(target_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{target_id}'"),
        )
    })?;
    participant.state = participant.state.transition(outcome.state)?;
    participant.resulting_commit = outcome.resulting_commit;
    participant.expected_merge_head = outcome.expected_merge_head;
    participant.conflict_paths = outcome.conflict_paths;
    participant.conflict_snapshot = outcome.conflict_snapshot;
    participant.error = error;
    participant.pending_action = None;
    Ok(())
}

pub(super) fn apply_failure(
    record: &mut MergeOperationRecord,
    target_id: &str,
    error: &ModelError,
) -> ModelResult<()> {
    let current = participant(record, target_id)?.state;
    let state = if continue_disposition(current) == ContinueDisposition::ResolveConflict {
        ParticipantState::Conflicted
    } else {
        ParticipantState::Failed
    };
    let prior = participant(record, target_id)?.clone();
    let pending_action = prior.pending_action.clone();
    apply_outcome(
        record,
        target_id,
        Outcome {
            state,
            resulting_commit: prior.resulting_commit,
            expected_merge_head: prior.expected_merge_head,
            conflict_paths: prior.conflict_paths,
            conflict_snapshot: prior.conflict_snapshot,
        },
        Some(MergeRecordError {
            code: error.code,
            message: error.message.clone(),
            detail: None,
        }),
    )?;
    participant_mut(record, target_id)?.pending_action = pending_action;
    Ok(())
}

pub(super) fn apply_recovery_failure(
    record: &mut MergeOperationRecord,
    target_id: &str,
    _error: &ModelError,
) -> ModelResult<()> {
    let participant = participant(record, target_id)?;
    if participant.pending_action.is_none() {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "post-mutation observation failure lost its durable pending action",
        )
        .with_member(target_id, &participant.path));
    }
    Ok(())
}

pub(super) fn mark_later_planned_unattempted<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    later: &[ContinueAction],
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for action in later {
        let participant = record.participants.get_mut(&action.target_id).unwrap();
        if participant.state == ParticipantState::Planned {
            participant.state = participant
                .state
                .transition(ParticipantState::Unattempted)?;
            super::super::persist_merge_record(store, root, record, emitter)?;
            super::super::emit_merge_member_finished(emitter, record, &action.target_id)?;
        }
    }
    Ok(())
}

pub(super) fn remaining_state(record: &MergeOperationRecord) -> OperationState {
    post_continue_state(
        record
            .participants
            .values()
            .map(|participant| participant.state),
    )
}

pub(super) fn observed_response<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    super::super::status::snapshot_status(backend, root, record)?.to_response(context)
}

pub(super) fn participant<'a>(
    record: &'a MergeOperationRecord,
    target_id: &str,
) -> ModelResult<&'a MergeParticipantRecord> {
    record.participants.get(target_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{target_id}'"),
        )
    })
}

pub(super) fn participant_mut<'a>(
    record: &'a mut MergeOperationRecord,
    target_id: &str,
) -> ModelResult<&'a mut MergeParticipantRecord> {
    record.participants.get_mut(target_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{target_id}'"),
        )
    })
}

pub(super) fn closed_or_missing<S: MergeStore>(
    store: &S,
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let Some(merge_id) = merge_id else {
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            "there is no open merge to continue",
        ));
    };
    let record = store.load(root, merge_id)?;
    if record.state == OperationState::Completed {
        record.to_response(context)
    } else {
        Err(wrong_state(merge_id, record.state))
    }
}

pub(super) fn wrong_state(merge_id: &str, state: OperationState) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("merge '{merge_id}' in state {state:?} cannot be continued"),
    )
}

fn wrong_participant_state(target_id: &str, participant: &MergeParticipantRecord) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("participant is in state {:?}", participant.state),
    )
    .with_member(target_id, &participant.path)
}

pub(super) fn invariant(message: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}
