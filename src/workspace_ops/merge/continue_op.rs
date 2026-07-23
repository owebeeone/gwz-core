use std::collections::BTreeMap;
use std::path::Path;

use crate::git::{
    GitBackend, GitIntegrateResult, GitMergeAnalysisKind, GitPreparedCommit, GitPreparedMerge,
    GitPreparedSignature,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext, WorkspaceMutatorLock};

use super::{
    MergeOperationRecord, MergeParticipantRecord, MergeRecordError, MergeStore, MergeTargetKind,
    OperationState, ParticipantState, PendingCommitSpec, PendingGitSignature, PendingMergeAction,
    PendingMergeActionKind, PendingMergeExpectedResult,
};

/// Continue owns the workspace mutator lock. Its caller must resolve `root`
/// without parsing live workspace metadata when recovery discovery found it.
pub(crate) fn handle_continue<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    let Some(mut record) = store.discover_open(root)? else {
        return closed_or_missing(store, root, request.merge_id.as_deref(), context);
    };
    super::validate::validate_open_merge_id(request.merge_id.as_deref(), &record.merge_id)?;
    match record.state {
        OperationState::Finalizing | OperationState::Completed => {
            let completed =
                super::finalize::finalize(backend, store, root, &mut record, context, emitter)?;
            return if completed {
                record.to_response(context)
            } else {
                observed_response(backend, root, record, context)
            };
        }
        OperationState::Executing
        | OperationState::AwaitingResolution
        | OperationState::Halted
        | OperationState::RecoveryRequired => {}
        state => return Err(wrong_state(&record.merge_id, state)),
    }

    reconcile_pending_actions(backend, store, root, &mut record, emitter)?;
    let actions = preflight(backend, root, &record, context.attribution.as_ref())?;
    super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Executing,
        emitter,
    )?;

    for (position, action) in actions.iter().enumerate() {
        emitter.member_started(&action.target_id, &action.path);
        if !action.durable {
            set_pending_action(&mut record, action)?;
            super::persist_merge_record(store, root, &record, emitter)?;
        }
        let result = match action.kind {
            ContinueActionKind::Resolve => {
                resolve_conflict(backend, root, &record, action, context)
            }
            ContinueActionKind::Retry(kind) => {
                retry_merge(backend, root, &record, action, kind, context)
            }
        };
        match result {
            Ok(outcome) => {
                apply_outcome(&mut record, &action.target_id, outcome, None)?;
                super::persist_merge_record(store, root, &record, emitter)?;
                super::emit_merge_member_finished(emitter, &record, &action.target_id)?;
            }
            Err(error) => {
                let contextual = error.with_member(&action.target_id, &action.path);
                if action.durable {
                    let participant = participant(&record, &action.target_id)?;
                    emitter.merge_member_finished(
                        participant.to_protocol(&action.target_id, &record.source_ref),
                    );
                    return Err(contextual);
                }
                apply_failure(&mut record, &action.target_id, &contextual)?;
                super::persist_merge_record(store, root, &record, emitter)?;
                super::emit_merge_member_finished(emitter, &record, &action.target_id)?;
                mark_later_planned_unattempted(
                    store,
                    root,
                    &mut record,
                    &actions[position + 1..],
                    emitter,
                )?;
                super::persist_operation_transition(
                    store,
                    root,
                    &mut record,
                    OperationState::Halted,
                    emitter,
                )?;
                return observed_response(backend, root, record, context);
            }
        }
    }

    let snapshot = super::status::snapshot_status(backend, root, record.clone())?;
    if !snapshot.operation_drift.is_empty()
        || snapshot
            .participants
            .values()
            .any(|participant| !participant.drift.is_empty())
    {
        return observed_response(backend, root, record, context);
    }
    let next = remaining_state(&record);
    if next == OperationState::Finalizing {
        super::enter_finalizing(store, root, &mut record, emitter)?;
        let completed =
            super::finalize::finalize(backend, store, root, &mut record, context, emitter)?;
        return if completed {
            record.to_response(context)
        } else {
            observed_response(backend, root, record, context)
        };
    } else {
        super::persist_operation_transition(store, root, &mut record, next, emitter)?;
    }
    observed_response(backend, root, record, context)
}

#[derive(Clone, Copy)]
enum ContinueActionKind {
    Resolve,
    Retry(GitMergeAnalysisKind),
}

struct ContinueAction {
    target_id: String,
    path: String,
    kind: ContinueActionKind,
    prepared: ContinuePrepared,
    durable: bool,
}

enum ContinuePrepared {
    Merge(GitPreparedMerge),
    Resolution(GitPreparedCommit),
}

enum ReconciledPendingAction {
    NotStarted,
    ExpectedConflict(Vec<String>),
    Completed(String),
}

fn reconcile_pending_actions<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let target_ids = record.selected_targets.clone();
    let mut reconciliations = Vec::new();
    for target_id in target_ids {
        let participant = participant(record, &target_id)?;
        if participant.pending_action.is_none() {
            continue;
        }
        let result =
            super::status::reconcile_pending_action(backend, root, &target_id, participant)?;
        let reconciliation = match result {
            super::status::PendingActionReconciliation::NotStarted => {
                ReconciledPendingAction::NotStarted
            }
            super::status::PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
                ReconciledPendingAction::ExpectedConflict(conflict_paths)
            }
            super::status::PendingActionReconciliation::Completed { resulting_commit } => {
                ReconciledPendingAction::Completed(resulting_commit)
            }
            super::status::PendingActionReconciliation::Ambiguous { reason, .. } => {
                return Err(ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    format!("pending merge action is ambiguous: {reason}"),
                )
                .with_member(&target_id, &participant.path));
            }
        };
        reconciliations.push((target_id, participant.path.clone(), reconciliation));
    }

    let adopted = reconciliations
        .iter()
        .filter(|(_, _, reconciliation)| {
            !matches!(reconciliation, ReconciledPendingAction::NotStarted)
        })
        .map(|(target_id, path, _)| (target_id.clone(), path.clone()))
        .collect::<Vec<_>>();
    for (target_id, path) in &adopted {
        emitter.member_started(target_id, path);
    }
    for (target_id, _, reconciliation) in reconciliations {
        if !matches!(reconciliation, ReconciledPendingAction::NotStarted) {
            apply_reconciled_pending(record, &target_id, reconciliation)?;
        }
    }
    if !adopted.is_empty() {
        super::persist_merge_record(store, root, record, emitter)?;
        for (target_id, _) in &adopted {
            super::emit_merge_member_finished(emitter, record, target_id)?;
        }
    }
    Ok(())
}

fn apply_reconciled_pending(
    record: &mut MergeOperationRecord,
    target_id: &str,
    reconciliation: ReconciledPendingAction,
) -> ModelResult<()> {
    let participant = participant(record, target_id)?;
    let pending = participant
        .pending_action
        .as_ref()
        .cloned()
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "pending-action reconciliation has no pending action",
            )
            .with_member(target_id, &participant.path)
        })?;
    match reconciliation {
        ReconciledPendingAction::NotStarted => Ok(()),
        ReconciledPendingAction::ExpectedConflict(paths) => {
            if pending.kind != PendingMergeActionKind::TrueMerge {
                return Err(invariant(
                    "only a pending true merge can reconcile to a native conflict",
                ));
            }
            apply_outcome(
                record,
                target_id,
                Outcome {
                    state: ParticipantState::Conflicted,
                    resulting_commit: None,
                    expected_merge_head: Some(pending.source_commit),
                    conflict_paths: paths,
                },
                None,
            )
        }
        ReconciledPendingAction::Completed(resulting_commit) => {
            let state = match pending.kind {
                PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
            };
            apply_outcome(
                record,
                target_id,
                Outcome::clean(state, resulting_commit),
                None,
            )
        }
    }
}

fn set_pending_action(
    record: &mut MergeOperationRecord,
    action: &ContinueAction,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&action.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", action.target_id),
            )
        })?;
    let kind = match action.kind {
        ContinueActionKind::Resolve => PendingMergeActionKind::ResolveConflict,
        ContinueActionKind::Retry(GitMergeAnalysisKind::UpToDate) => {
            PendingMergeActionKind::VerifyUpToDate
        }
        ContinueActionKind::Retry(GitMergeAnalysisKind::FastForward) => {
            PendingMergeActionKind::FastForward
        }
        ContinueActionKind::Retry(GitMergeAnalysisKind::TrueMerge) => {
            PendingMergeActionKind::TrueMerge
        }
    };
    participant.pending_action = Some(PendingMergeAction {
        kind,
        target_branch: participant.target_branch.clone(),
        before_commit: participant.before_commit.clone(),
        source_commit: participant.source_commit.clone(),
        commit_message: participant.commit_message.clone(),
        expected_result: Some(match &action.prepared {
            ContinuePrepared::Merge(prepared) => pending_expected_result(prepared),
            ContinuePrepared::Resolution(_) => PendingMergeExpectedResult::Commit,
        }),
        commit_spec: match &action.prepared {
            ContinuePrepared::Merge(GitPreparedMerge::Commit(spec))
            | ContinuePrepared::Resolution(spec) => Some(pending_commit_spec(spec)),
            _ => None,
        },
        extensions: BTreeMap::new(),
    });
    Ok(())
}

fn pending_expected_result(result: &GitPreparedMerge) -> PendingMergeExpectedResult {
    match result {
        GitPreparedMerge::Unchanged => PendingMergeExpectedResult::Unchanged,
        GitPreparedMerge::FastForward => PendingMergeExpectedResult::FastForward,
        GitPreparedMerge::ExpectedConflict => PendingMergeExpectedResult::ExpectedConflict,
        GitPreparedMerge::Commit(_) => PendingMergeExpectedResult::Commit,
    }
}

fn pending_commit_spec(spec: &GitPreparedCommit) -> PendingCommitSpec {
    PendingCommitSpec {
        tree_oid: spec.tree_oid.clone(),
        author: pending_signature(&spec.author),
        committer: pending_signature(&spec.committer),
        extensions: BTreeMap::new(),
    }
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

fn preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<Vec<ContinueAction>> {
    if let Some((target_id, _)) = record
        .participants
        .iter()
        .find(|(_, participant)| participant.target_kind == MergeTargetKind::Root)
    {
        return Err(ModelError::new(
            ErrorCode::RootMergeNotYetSupported,
            format!("merge participant '{target_id}' targets the workspace root"),
        ));
    }
    let snapshot = super::status::snapshot_status(backend, root, record.clone())?;
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
            let prepared =
                super::pending::decode_durable_prepared_action(pending).map_err(|reason| {
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
        match participant.state {
            ParticipantState::Conflicted => {
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
            ParticipantState::Planned
            | ParticipantState::Failed
            | ParticipantState::Unattempted => {
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
            ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Continued => {}
            ParticipantState::Aborted | ParticipantState::RolledBack => {
                return Err(wrong_participant_state(target_id, participant));
            }
        }
    }
    Ok(actions)
}

fn durable_continue_action(
    participant: &MergeParticipantRecord,
    prepared: super::pending::DurablePreparedAction,
) -> ModelResult<(ContinueActionKind, ContinuePrepared)> {
    match (participant.state, prepared) {
        (
            ParticipantState::Conflicted,
            super::pending::DurablePreparedAction::Resolution(prepared),
        ) => Ok((
            ContinueActionKind::Resolve,
            ContinuePrepared::Resolution(prepared),
        )),
        (
            ParticipantState::Planned | ParticipantState::Failed | ParticipantState::Unattempted,
            super::pending::DurablePreparedAction::Merge(prepared),
        ) => {
            let kind = match prepared {
                GitPreparedMerge::Unchanged => GitMergeAnalysisKind::UpToDate,
                GitPreparedMerge::FastForward => GitMergeAnalysisKind::FastForward,
                GitPreparedMerge::ExpectedConflict | GitPreparedMerge::Commit(_) => {
                    GitMergeAnalysisKind::TrueMerge
                }
            };
            Ok((
                ContinueActionKind::Retry(kind),
                ContinuePrepared::Merge(prepared),
            ))
        }
        _ => Err(invariant(
            "pending action kind does not match the participant recovery state",
        )),
    }
}

fn resolve_conflict<B: GitBackend>(
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

fn retry_merge<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    action: &ContinueAction,
    kind: GitMergeAnalysisKind,
    _context: &OperationContext,
) -> ModelResult<Outcome> {
    let participant = participant(record, &action.target_id)?;
    let ContinuePrepared::Merge(prepared) = &action.prepared else {
        return Err(invariant("retry action has no prepared merge"));
    };
    let result = backend.execute_prepared_merge_upstream_checked(
        &root.join(&participant.path),
        &participant.target_branch,
        &participant.before_commit,
        &participant.source_commit,
        &participant.commit_message,
        prepared,
    )?;
    classify_retry(participant, kind, result)
}

fn classify_retry(
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
struct Outcome {
    state: ParticipantState,
    resulting_commit: Option<String>,
    expected_merge_head: Option<String>,
    conflict_paths: Vec<String>,
}

impl Outcome {
    fn clean(state: ParticipantState, commit: String) -> Self {
        Self {
            state,
            resulting_commit: Some(commit),
            expected_merge_head: None,
            conflict_paths: Vec::new(),
        }
    }
}

fn apply_outcome(
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
    participant.error = error;
    participant.pending_action = None;
    Ok(())
}

fn apply_failure(
    record: &mut MergeOperationRecord,
    target_id: &str,
    error: &ModelError,
) -> ModelResult<()> {
    let current = participant(record, target_id)?.state;
    let state = if current == ParticipantState::Conflicted {
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

fn mark_later_planned_unattempted<S: MergeStore>(
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
            super::persist_merge_record(store, root, record, emitter)?;
            super::emit_merge_member_finished(emitter, record, &action.target_id)?;
        }
    }
    Ok(())
}

fn remaining_state(record: &MergeOperationRecord) -> OperationState {
    if record
        .participants
        .values()
        .any(|participant| participant.state == ParticipantState::Failed)
    {
        OperationState::Halted
    } else if record.participants.values().any(|participant| {
        matches!(
            participant.state,
            ParticipantState::Planned
                | ParticipantState::Unattempted
                | ParticipantState::Conflicted
        )
    }) {
        OperationState::AwaitingResolution
    } else {
        OperationState::Finalizing
    }
}

fn observed_response<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    super::status::snapshot_status(backend, root, record)?.to_response(context)
}

fn participant<'a>(
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

fn participant_mut<'a>(
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

fn closed_or_missing<S: MergeStore>(
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

fn wrong_state(merge_id: &str, state: OperationState) -> ModelError {
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

fn invariant(message: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn participant(state: ParticipantState) -> MergeParticipantRecord {
        serde_yaml::from_str(&format!(
            "path: repos/app\ntarget_kind: member\ntarget_branch: main\nbefore_commit: before\nsource_commit: source\ncommit_message: exact message\nstate: {}\n",
            serde_yaml::to_string(&state).unwrap().trim()
        ))
        .unwrap()
    }

    fn record(states: &[(&str, ParticipantState)]) -> MergeOperationRecord {
        let mut record: MergeOperationRecord = serde_yaml::from_str(
            r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_1, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#,
        )
        .unwrap();
        record.selected_targets = states.iter().map(|(id, _)| (*id).to_owned()).collect();
        record.participants = states
            .iter()
            .map(|(id, state)| ((*id).to_owned(), participant(*state)))
            .collect::<BTreeMap<_, _>>();
        record
    }

    #[test]
    fn failed_retry_can_become_a_new_conflict_without_closing_the_batch() {
        let mut record = record(&[("mem_app", ParticipantState::Failed)]);
        let source = record.participants["mem_app"].clone();
        let outcome = classify_retry(
            &source,
            GitMergeAnalysisKind::TrueMerge,
            GitIntegrateResult {
                commit: None,
                conflicts: vec!["README.md".into()],
            },
        )
        .unwrap();
        apply_outcome(&mut record, "mem_app", outcome, None).unwrap();
        assert_eq!(
            record.participants["mem_app"].state,
            ParticipantState::Conflicted
        );
        assert_eq!(remaining_state(&record), OperationState::AwaitingResolution);
    }

    #[test]
    fn invalid_retry_result_is_rejected_before_the_record_is_changed() {
        let record = record(&[("mem_app", ParticipantState::Unattempted)]);
        let unchanged = record.clone();
        let error = classify_retry(
            &record.participants["mem_app"],
            GitMergeAnalysisKind::FastForward,
            GitIntegrateResult::clean("wrong".into()),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
        assert_eq!(record, unchanged);
    }

    #[test]
    fn retry_intent_freezes_inputs_and_is_cleared_only_with_the_outcome() {
        let mut record = record(&[("mem_app", ParticipantState::Failed)]);
        let action = ContinueAction {
            target_id: "mem_app".to_owned(),
            path: "repos/app".to_owned(),
            kind: ContinueActionKind::Retry(GitMergeAnalysisKind::FastForward),
            prepared: ContinuePrepared::Merge(GitPreparedMerge::FastForward),
            durable: false,
        };

        set_pending_action(&mut record, &action).unwrap();
        let pending = record.participants["mem_app"]
            .pending_action
            .as_ref()
            .unwrap();
        assert_eq!(pending.kind, PendingMergeActionKind::FastForward);
        assert_eq!(pending.before_commit, "before");
        assert_eq!(pending.source_commit, "source");
        assert_eq!(pending.commit_message, "exact message");

        apply_outcome(
            &mut record,
            "mem_app",
            Outcome::clean(ParticipantState::FastForwarded, "source".to_owned()),
            None,
        )
        .unwrap();
        assert!(record.participants["mem_app"].pending_action.is_none());
    }

    #[test]
    fn mixed_results_enter_only_the_safe_next_operation_state() {
        let mut record = record(&[
            ("app", ParticipantState::UpToDate),
            ("lib", ParticipantState::Merged),
            ("docs", ParticipantState::Continued),
        ]);
        assert_eq!(remaining_state(&record), OperationState::Finalizing);
        record.participants.get_mut("docs").unwrap().state = ParticipantState::Conflicted;
        assert_eq!(remaining_state(&record), OperationState::AwaitingResolution);
        record.participants.get_mut("lib").unwrap().state = ParticipantState::Failed;
        assert_eq!(remaining_state(&record), OperationState::Halted);
    }
}
