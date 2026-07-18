use std::path::Path;

use crate::git::{GitBackend, GitIntegrateResult, GitMergeAnalysisKind};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationContext, WorkspaceMutatorLock};

use super::{
    MergeOperationRecord, MergeParticipantRecord, MergeRecordError, MergeStore, MergeTargetKind,
    OperationState, ParticipantState,
};

/// Continue owns the workspace mutator lock. Its caller must resolve `root`
/// without parsing live workspace metadata when recovery discovery found it.
pub(crate) fn handle_continue<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    let Some(mut record) = store.discover_open(root)? else {
        return closed_or_missing(store, root, request.merge_id.as_deref(), context);
    };
    super::validate::validate_open_merge_id(request.merge_id.as_deref(), &record.merge_id)?;
    match record.state {
        OperationState::Finalizing => return record.to_response(context),
        OperationState::Executing | OperationState::AwaitingResolution | OperationState::Halted => {
        }
        state => return Err(wrong_state(&record.merge_id, state)),
    }

    let actions = preflight(backend, root, &record)?;
    let emitter = EventEmitter::new(context, events, 0);
    super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Executing,
        &emitter,
    )?;

    for (position, action) in actions.iter().enumerate() {
        let result = match action.kind {
            ContinueActionKind::Resolve => resolve_conflict(backend, root, &record, action),
            ContinueActionKind::Retry(kind) => {
                retry_merge(backend, root, &record, action, kind, context)
            }
        };
        match result {
            Ok(outcome) => {
                apply_outcome(&mut record, &action.target_id, outcome, None)?;
                store.write_open(root, &record)?;
                emitter.member_finished(&action.target_id, &action.path);
            }
            Err(error) => {
                let contextual = error.with_member(&action.target_id, &action.path);
                apply_failure(&mut record, &action.target_id, &contextual)?;
                store.write_open(root, &record)?;
                emitter.member_finished(&action.target_id, &action.path);
                mark_later_planned_unattempted(store, root, &mut record, &actions[position + 1..])?;
                super::persist_operation_transition(
                    store,
                    root,
                    &mut record,
                    OperationState::Halted,
                    &emitter,
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
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::RecoveryRequired,
            &emitter,
        )?;
        return observed_response(backend, root, record, context);
    }
    let next = remaining_state(&record);
    if next == OperationState::Finalizing {
        super::enter_finalizing(store, root, &mut record, &emitter)?;
    } else {
        super::persist_operation_transition(store, root, &mut record, next, &emitter)?;
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
}

fn preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
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
        match participant.state {
            ParticipantState::Conflicted => actions.push(ContinueAction {
                target_id: target_id.clone(),
                path: participant.path.clone(),
                kind: ContinueActionKind::Resolve,
            }),
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

fn resolve_conflict<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    action: &ContinueAction,
) -> ModelResult<Outcome> {
    let participant = participant(record, &action.target_id)?;
    let merge_head = participant
        .expected_merge_head
        .as_deref()
        .unwrap_or(&participant.source_commit);
    let commit = backend.commit_merge_resolution_checked(
        &root.join(&participant.path),
        &participant.before_commit,
        merge_head,
        &participant.commit_message,
    )?;
    Ok(Outcome::clean(ParticipantState::Continued, commit.commit))
}

fn retry_merge<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    action: &ContinueAction,
    kind: GitMergeAnalysisKind,
    context: &OperationContext,
) -> ModelResult<Outcome> {
    let participant = participant(record, &action.target_id)?;
    let result = backend.merge_upstream_checked(
        &root.join(&participant.path),
        &participant.target_branch,
        &participant.before_commit,
        &participant.source_commit,
        &participant.commit_message,
        context.attribution.as_ref(),
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
    )
}

fn mark_later_planned_unattempted<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    later: &[ContinueAction],
) -> ModelResult<()> {
    for action in later {
        let participant = record.participants.get_mut(&action.target_id).unwrap();
        if participant.state == ParticipantState::Planned {
            participant.state = participant
                .state
                .transition(ParticipantState::Unattempted)?;
            store.write_open(root, record)?;
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
