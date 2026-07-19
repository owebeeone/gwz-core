use super::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeOperationRecord, MergeParticipantPlan,
    MergeParticipantRecord, MergeRecordError, MergeStore, OperationState, ParticipantState,
    PendingMergeAction, PendingMergeActionKind, plan::plan_merge,
};
use crate::artifact;
use crate::git::{
    GitBackend, GitHeadState, GitIntegrateResult, GitMergeAnalysis, GitMergeAnalysisKind, GitStatus,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, EventEmitter, OperationContext};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;
use crate::{AggregateStatus, MergeOperationState as OpState, MergeParticipantState as PState};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn handle_start_durable<B, S, C, I>(
    dependencies: super::MergeDependencies<'_, B, S, C, I>,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    let super::MergeDependencies {
        backend,
        store,
        clock,
        ids,
        events: _,
    } = dependencies;
    if request.meta.dry_run.unwrap_or(false) {
        return handle_dry_run(backend, root, request, context);
    }

    if let Some(open) = store.discover_open(root)? {
        return Err(open_operation_error(&open.merge_id));
    }
    let mut plan = plan_merge(backend, root, request)?;
    let merge_id = ids.next_id("merge").to_string();
    freeze_merge_messages(&mut plan.participants, &plan.source_ref, &merge_id, context);
    let mut record = create_record(root, &plan, &merge_id, clock, context)?;
    super::persist_merge_record(store, root, &record, emitter)?;
    emitter.operation_state_changed(record.state.into());
    execute_durable(
        backend,
        store,
        root,
        &plan.participants,
        context.attribution.as_ref(),
        &mut record,
        emitter,
    )?;

    let next = if record
        .participants
        .values()
        .any(|participant| participant.state == ParticipantState::Failed)
    {
        OperationState::Halted
    } else if record
        .participants
        .values()
        .any(|participant| participant.state == ParticipantState::Conflicted)
    {
        OperationState::AwaitingResolution
    } else {
        OperationState::Finalizing
    };
    if next == OperationState::Finalizing {
        super::enter_finalizing(store, root, &mut record, emitter)?;
    } else {
        super::persist_operation_transition(store, root, &mut record, next, emitter)?;
    }
    start_response(&record, &plan.participants, context)
}

fn handle_dry_run<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let plan = plan_merge(backend, root, request)?;
    let repos = plan
        .participants
        .iter()
        .map(|participant| summary(Row::new(participant, PState::Planned), &plan.source_ref))
        .collect();
    merge_response(context, repos, Vec::new())
}

fn open_operation_error(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::OpenOperation,
        format!("merge '{merge_id}' is open; use merge status, merge continue, or merge abort"),
    )
}

fn freeze_merge_messages(
    participants: &mut [MergeParticipantPlan],
    source_ref: &str,
    merge_id: &str,
    context: &OperationContext,
) {
    for participant in participants {
        participant.commit_message = format!(
            "Merge '{source_ref}' into '{}'\n\nGWZ-Merge-ID: {merge_id}\nGWZ-Operation-ID: {}",
            participant.target_branch, context.operation_id
        );
    }
}

fn create_record<C: Clock>(
    root: &Path,
    plan: &super::MergePlan,
    merge_id: &str,
    clock: &C,
    context: &OperationContext,
) -> ModelResult<MergeOperationRecord> {
    let manifest = artifact::read_manifest(root)?;
    let participants = plan
        .participants
        .iter()
        .map(|participant| {
            (
                participant.target_id.clone(),
                MergeParticipantRecord {
                    path: participant.path.clone(),
                    target_kind: participant.target_kind,
                    target_branch: participant.target_branch.clone(),
                    before_commit: participant.before_commit.clone(),
                    source_commit: participant.source_commit.clone(),
                    commit_message: participant.commit_message.clone(),
                    state: ParticipantState::Planned,
                    resulting_commit: None,
                    expected_merge_head: None,
                    conflict_paths: Vec::new(),
                    error: None,
                    pending_action: None,
                    preservation: Vec::new(),
                    drift: Vec::new(),
                    extensions: BTreeMap::new(),
                },
            )
        })
        .collect();
    Ok(MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: crate::VERSION.to_owned(),
        workspace_id: manifest.workspace.id,
        merge_id: merge_id.to_owned(),
        operation_id: context.operation_id.clone(),
        state: OperationState::Executing,
        source_ref: plan.source_ref.clone(),
        created_at: clock.now_ms().0.to_string(),
        baseline: plan.baseline.clone(),
        selected_targets: plan
            .participants
            .iter()
            .map(|participant| participant.target_id.clone())
            .collect(),
        participants,
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    })
}

fn execute_durable<B: ExecutionBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    participants: &[MergeParticipantPlan],
    attribution: Option<&crate::model::OperationAttribution>,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for (index, participant) in participants.iter().enumerate() {
        emitter.member_started(&participant.target_id, &participant.path);
        let kind = match prepare_one(backend, root, participant) {
            Ok(kind) => kind,
            Err(error) => {
                persist_start_failure(store, root, record, participant, &error, emitter)?;
                mark_later_unattempted(store, root, record, &participants[index + 1..], emitter)?;
                break;
            }
        };
        set_pending_action(record, participant, kind)?;
        super::persist_merge_record(store, root, record, emitter)?;
        match execute_prepared(backend, root, participant, kind, attribution) {
            Ok(row) => {
                apply_row(record, participant, &row, None)?;
                record
                    .participants
                    .get_mut(&participant.target_id)
                    .expect("participant was validated before execution")
                    .pending_action = None;
                super::persist_merge_record(store, root, record, emitter)?;
                super::emit_merge_member_finished(emitter, record, &participant.target_id)?;
            }
            Err(error) => {
                persist_start_failure(store, root, record, participant, &error, emitter)?;
                mark_later_unattempted(store, root, record, &participants[index + 1..], emitter)?;
                break;
            }
        }
    }
    Ok(())
}

fn persist_start_failure<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    participant: &MergeParticipantPlan,
    error: &ModelError,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let contextual = error
        .clone()
        .with_member(&participant.target_id, &participant.path);
    apply_row(
        record,
        participant,
        &Row::new(participant, PState::Failed),
        Some(&contextual),
    )?;
    super::persist_merge_record(store, root, record, emitter)?;
    super::emit_merge_member_finished(emitter, record, &participant.target_id)
}

fn mark_later_unattempted<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    later: &[MergeParticipantPlan],
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for participant in later {
        apply_row(
            record,
            participant,
            &Row::new(participant, PState::Unattempted),
            None,
        )?;
        super::persist_merge_record(store, root, record, emitter)?;
        super::emit_merge_member_finished(emitter, record, &participant.target_id)?;
    }
    Ok(())
}

fn set_pending_action(
    record: &mut MergeOperationRecord,
    plan: &MergeParticipantPlan,
    kind: GitMergeAnalysisKind,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&plan.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", plan.target_id),
            )
        })?;
    participant.pending_action = Some(PendingMergeAction {
        kind: pending_kind(kind),
        target_branch: plan.target_branch.clone(),
        before_commit: plan.before_commit.clone(),
        source_commit: plan.source_commit.clone(),
        commit_message: plan.commit_message.clone(),
        extensions: BTreeMap::new(),
    });
    Ok(())
}

fn pending_kind(kind: GitMergeAnalysisKind) -> PendingMergeActionKind {
    match kind {
        GitMergeAnalysisKind::UpToDate => PendingMergeActionKind::VerifyUpToDate,
        GitMergeAnalysisKind::FastForward => PendingMergeActionKind::FastForward,
        GitMergeAnalysisKind::TrueMerge => PendingMergeActionKind::TrueMerge,
    }
}

fn apply_row(
    record: &mut MergeOperationRecord,
    plan: &MergeParticipantPlan,
    row: &Row<'_>,
    error: Option<&ModelError>,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&plan.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", plan.target_id),
            )
        })?;
    let next = match row.state {
        PState::UpToDate => ParticipantState::UpToDate,
        PState::FastForwarded => ParticipantState::FastForwarded,
        PState::Merged => ParticipantState::Merged,
        PState::Conflicted => ParticipantState::Conflicted,
        PState::Failed => ParticipantState::Failed,
        PState::Unattempted => ParticipantState::Unattempted,
        _ => {
            return Err(ModelError::new(
                ErrorCode::InternalError,
                "start produced an invalid durable participant state",
            ));
        }
    };
    participant.state = participant.state.transition(next)?;
    participant.resulting_commit.clone_from(&row.oid);
    participant.conflict_paths.clone_from(&row.paths);
    participant.expected_merge_head =
        (next == ParticipantState::Conflicted).then(|| plan.source_commit.clone());
    participant.error = error.map(|error| MergeRecordError {
        code: error.code,
        message: error.message.clone(),
        detail: None,
    });
    Ok(())
}

fn start_response(
    record: &MergeOperationRecord,
    plan: &[MergeParticipantPlan],
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let mut response = record.to_response(context)?;
    for (repo, participant) in response.repos.iter_mut().zip(plan) {
        repo.predicted = participant.analysis;
        repo.prediction_complete = Some(participant.prediction_complete);
        repo.live_commit = match repo.state {
            PState::UpToDate | PState::FastForwarded | PState::Merged => {
                repo.resulting_commit.clone()
            }
            PState::Conflicted => Some(participant.before_commit.clone()),
            _ => None,
        };
    }
    Ok(response)
}
type Inspection = (GitStatus, GitHeadState, GitMergeAnalysis);
trait ExecutionBackend {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection>;
    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitIntegrateResult>;
}
impl<B: GitBackend> ExecutionBackend for B {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
        Ok((
            self.status(path)?,
            self.head(path)?,
            self.merge_analysis(path, branch, source)?,
        ))
    }
    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitIntegrateResult> {
        GitBackend::merge_upstream_checked(
            self,
            path,
            branch,
            expected_before,
            source,
            message,
            attribution,
        )
    }
}
#[cfg(test)]
struct Execution<'a> {
    rows: Vec<Row<'a>>,
    errors: Vec<crate::GwzError>,
}
struct Row<'a> {
    plan: &'a MergeParticipantPlan,
    state: PState,
    oid: Option<String>,
    paths: Vec<String>,
    err: Option<crate::GwzError>,
}
impl<'a> Row<'a> {
    fn new(plan: &'a MergeParticipantPlan, state: PState) -> Self {
        Self {
            plan,
            state,
            oid: None,
            paths: Vec::new(),
            err: None,
        }
    }
}
#[cfg(test)]
fn execute_plan<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    participants: &'a [MergeParticipantPlan],
    attribution: Option<&crate::model::OperationAttribution>,
) -> Execution<'a> {
    let mut execution = Execution {
        rows: Vec::with_capacity(participants.len()),
        errors: Vec::new(),
    };
    for (index, participant) in participants.iter().enumerate() {
        match execute_one(backend, root, participant, attribution) {
            Ok(row) => {
                execution.rows.push(row);
            }
            Err(error) => {
                let wire_error = participant_error(participant, &error);
                execution.rows.push(Row {
                    err: Some(wire_error.clone()),
                    ..Row::new(participant, PState::Failed)
                });
                execution.errors.push(wire_error);
                execution.rows.extend(
                    participants[index + 1..]
                        .iter()
                        .map(|later| Row::new(later, PState::Unattempted)),
                );
                break;
            }
        }
    }
    execution
}
#[cfg(test)]
fn execute_one<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &'a MergeParticipantPlan,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<Row<'a>> {
    let kind = prepare_one(backend, root, plan)?;
    execute_prepared(backend, root, plan, kind, attribution)
}

fn prepare_one<B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &MergeParticipantPlan,
) -> ModelResult<GitMergeAnalysisKind> {
    let path = root.join(&plan.path);
    let (status, head, analysis) =
        backend.inspect(&path, &plan.target_branch, &plan.source_commit)?;
    let kind = planned_kind(plan)?;
    if status.is_dirty
        || head.branch.as_deref() != Some(plan.target_branch.as_str())
        || head.commit.as_deref() != Some(plan.before_commit.as_str())
        || analysis.target_branch != plan.target_branch
        || analysis.target_commit != plan.before_commit
        || analysis.source_commit != plan.source_commit
        || analysis.kind != kind
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!("member '{}' changed after merge planning", plan.target_id),
        ));
    }
    Ok(kind)
}

fn execute_prepared<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &'a MergeParticipantPlan,
    kind: GitMergeAnalysisKind,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<Row<'a>> {
    let result = backend.merge(
        &root.join(&plan.path),
        &plan.target_branch,
        &plan.before_commit,
        &plan.source_commit,
        &plan.commit_message,
        attribution,
    )?;
    if !result.conflicts.is_empty() {
        if kind != GitMergeAnalysisKind::TrueMerge || result.commit.is_some() {
            return Err(invariant(
                plan,
                "backend returned an invalid conflict result",
            ));
        }
        return Ok(Row {
            paths: result.conflicts,
            ..Row::new(plan, PState::Conflicted)
        });
    }
    let resulting = result
        .commit
        .ok_or_else(|| invariant(plan, "clean merge result omitted its commit"))?;
    if (kind == GitMergeAnalysisKind::UpToDate && resulting != plan.before_commit)
        || (kind == GitMergeAnalysisKind::FastForward && resulting != plan.source_commit)
    {
        return Err(invariant(
            plan,
            "backend returned the wrong clean result commit",
        ));
    }
    let state = match kind {
        GitMergeAnalysisKind::UpToDate => PState::UpToDate,
        GitMergeAnalysisKind::FastForward => PState::FastForwarded,
        GitMergeAnalysisKind::TrueMerge => PState::Merged,
    };
    Ok(Row {
        oid: Some(resulting),
        ..Row::new(plan, state)
    })
}
fn planned_kind(plan: &MergeParticipantPlan) -> ModelResult<GitMergeAnalysisKind> {
    match plan.analysis {
        Some(crate::MergeAnalysisKind::UpToDate) => Ok(GitMergeAnalysisKind::UpToDate),
        Some(crate::MergeAnalysisKind::FastForward) => Ok(GitMergeAnalysisKind::FastForward),
        Some(crate::MergeAnalysisKind::TrueMerge) => Ok(GitMergeAnalysisKind::TrueMerge),
        _ => Err(invariant(
            plan,
            "frozen plan has no executable merge analysis",
        )),
    }
}
fn summary(row: Row<'_>, source_ref: &str) -> crate::MergeRepoSummary {
    let plan = row.plan;
    crate::MergeRepoSummary {
        target_id: plan.target_id.clone(),
        target_kind: crate::TargetKind::Member,
        path: plan.path.clone(),
        source_ref: source_ref.to_owned(),
        source_commit: plan.source_commit.clone(),
        target_branch: plan.target_branch.clone(),
        before_commit: plan.before_commit.clone(),
        live_commit: (!matches!(row.state, PState::Failed | PState::Unattempted)).then(|| {
            row.oid
                .clone()
                .unwrap_or_else(|| plan.before_commit.clone())
        }),
        resulting_commit: row.oid,
        state: row.state,
        predicted: plan.analysis,
        prediction_complete: Some(plan.prediction_complete),
        conflict_paths: row.paths,
        continue_eligible: None,
        abort_eligible: None,
        drift: Vec::new(),
        error: row.err,
        pending_action: None,
    }
}
fn merge_response(
    context: &OperationContext,
    repos: Vec<crate::MergeRepoSummary>,
    errors: Vec<crate::GwzError>,
) -> ModelResult<crate::MergeResponse> {
    let mut counts = crate::MergeParticipantCounts {
        total: repos.len() as i64,
        ..Default::default()
    };
    for repo in &repos {
        match repo.state {
            PState::Planned => counts.planned += 1,
            PState::UpToDate => counts.up_to_date += 1,
            PState::FastForwarded => counts.fast_forwarded += 1,
            PState::Merged => counts.merged += 1,
            PState::Conflicted => counts.conflicted += 1,
            PState::Failed => counts.failed += 1,
            PState::Unattempted => counts.unattempted += 1,
            _ => {}
        }
    }
    let (state, aggregate) = if context.dry_run {
        (OpState::Completed, AggregateStatus::Accepted)
    } else if !errors.is_empty() {
        (OpState::Halted, AggregateStatus::Failed)
    } else if counts.conflicted > 0 {
        (OpState::AwaitingResolution, AggregateStatus::Conflicted)
    } else if counts.up_to_date == counts.total {
        (OpState::Completed, AggregateStatus::Noop)
    } else {
        (OpState::Completed, AggregateStatus::Ok)
    };
    let meta = crate::RequestMeta {
        request_id: context.request_id.clone(),
        schema_version: context.schema_version.clone(),
        attribution: context.attribution.as_ref().map(Into::into),
        ..Default::default()
    };
    Ok(crate::MergeResponse {
        response: crate::operation::response_envelope_for(
            &meta,
            ActionKind::Merge,
            context.operation_id.clone(),
            aggregate,
            errors,
        )?,
        merge_id: None,
        state,
        open: !matches!(state, OpState::Completed | OpState::Aborted),
        participant_counts: counts,
        repos,
        operation_drift: Vec::new(),
        preservation: None,
        publication_step: None,
    })
}
#[cfg(test)]
fn participant_error(plan: &MergeParticipantPlan, error: &ModelError) -> crate::GwzError {
    let mut wire: crate::GwzError = error.into();
    wire.member_id = Some(plan.target_id.clone());
    wire.member_path = Some(plan.path.clone());
    wire.target_kind = Some(crate::TargetKind::Member);
    wire
}
fn invariant(plan: &MergeParticipantPlan, message: &str) -> ModelError {
    ModelError::new(
        ErrorCode::GitCommandFailed,
        format!("member '{}': {message}", plan.target_id),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Git2Backend;
    use crate::operation::EventSink;
    use crate::runtime::clock::{FixedClock, TimestampMs};
    use crate::workspace_ops::tests::{TempDir, commit_file, request_meta, test_member_state};
    use std::cell::RefCell;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        mutated_before_failure: RefCell<Vec<String>>,
    }
    impl ExecutionBackend for Fake {
        fn inspect(&self, path: &Path, _: &str, source: &str) -> ModelResult<Inspection> {
            let key = key(path);
            Ok((
                GitStatus::clean(),
                GitHeadState {
                    branch: Some("main".into()),
                    commit: Some(format!("before-{key}")),
                    is_detached: false,
                },
                GitMergeAnalysis {
                    target_branch: "main".into(),
                    target_commit: format!("before-{key}"),
                    source_commit: source.into(),
                    kind: GitMergeAnalysisKind::TrueMerge,
                    commit_identity_required: true,
                    prediction_complete: false,
                },
            ))
        }
        fn merge(
            &self,
            path: &Path,
            _: &str,
            expected_before: &str,
            source: &str,
            message: &str,
            _: Option<&crate::model::OperationAttribution>,
        ) -> ModelResult<GitIntegrateResult> {
            let key = key(path);
            self.calls
                .borrow_mut()
                .push(format!("{key}:{expected_before}:{source}:{message}"));
            if key == "fail" {
                self.mutated_before_failure.borrow_mut().push(key.into());
                return Err(ModelError::new(ErrorCode::GitCommandFailed, "boom"));
            }
            Ok(if key == "conflict" {
                GitIntegrateResult {
                    commit: None,
                    conflicts: vec!["x".into()],
                }
            } else {
                GitIntegrateResult::clean(format!("result-{key}"))
            })
        }
    }
    fn key(path: &Path) -> &str {
        path.file_name().unwrap().to_str().unwrap()
    }
    fn plans(names: &[&str]) -> Vec<MergeParticipantPlan> {
        names
            .iter()
            .map(|name| MergeParticipantPlan {
                target_id: format!("mem_{name}"),
                target_kind: super::super::MergeTargetKind::Member,
                path: (*name).into(),
                target_branch: "main".into(),
                before_commit: format!("before-{name}"),
                source_commit: format!("source-{name}"),
                analysis: Some(crate::MergeAnalysisKind::TrueMerge),
                prediction_complete: false,
                commit_message: "merge".into(),
            })
            .collect()
    }
    fn context(dry_run: bool) -> OperationContext {
        OperationContext {
            operation_id: "op".into(),
            request_id: "req".into(),
            schema_version: "gwz.v0".into(),
            action: ActionKind::Merge,
            dry_run,
            attribution: None,
        }
    }

    fn attributed_context() -> OperationContext {
        let mut context = context(false);
        context.attribution = Some(crate::model::OperationAttribution {
            git_author: Some(crate::model::GitObjectIdentity {
                name: "Merge Request Author".to_owned(),
                email: "merge-author@example.invalid".to_owned(),
                time_ms: Some(TimestampMs(1_700_000_000_000)),
                timezone_offset_minutes: Some(600),
            }),
            git_committer: Some(crate::model::GitObjectIdentity {
                name: "Merge Request Committer".to_owned(),
                email: "merge-committer@example.invalid".to_owned(),
                time_ms: Some(TimestampMs(1_700_000_100_000)),
                timezone_offset_minutes: Some(-300),
            }),
            ..Default::default()
        });
        context
    }

    #[derive(Default)]
    struct MemoryStore {
        records: Mutex<Vec<MergeOperationRecord>>,
        trace: Mutex<Vec<String>>,
        events: Mutex<Vec<crate::OperationEvent>>,
        writes: Mutex<usize>,
        fail_write_at: Mutex<Option<usize>>,
    }

    impl MergeStore for MemoryStore {
        fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
            Ok(self.records.lock().unwrap().last().cloned())
        }

        fn write_open(&self, _root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
            let mut writes = self.writes.lock().unwrap();
            *writes += 1;
            if self.fail_write_at.lock().unwrap().as_ref() == Some(&*writes) {
                return Err(ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    "injected record write failure",
                ));
            }
            self.trace
                .lock()
                .unwrap()
                .push(format!("write:{:?}", record.state));
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    struct TraceSink<'a>(&'a MemoryStore);

    impl EventSink for TraceSink<'_> {
        fn deliver(&self, event: crate::OperationEvent) {
            self.0.events.lock().unwrap().push(event.clone());
            self.0
                .trace
                .lock()
                .unwrap()
                .push(format!("event:{:?}", event.kind));
        }
    }

    struct DurableSpy<'a> {
        store: &'a MemoryStore,
        fake: Fake,
    }

    impl ExecutionBackend for DurableSpy<'_> {
        fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
            self.fake.inspect(path, branch, source)
        }

        fn merge(
            &self,
            path: &Path,
            branch: &str,
            expected_before: &str,
            source: &str,
            message: &str,
            attribution: Option<&crate::model::OperationAttribution>,
        ) -> ModelResult<GitIntegrateResult> {
            let records = self.store.records.lock().unwrap();
            let target_id = format!("mem_{}", key(path));
            assert!(
                records
                    .last()
                    .and_then(|record| record.participants.get(&target_id))
                    .and_then(|participant| participant.pending_action.as_ref())
                    .is_some(),
                "the exact participant action must be durable before Git mutation"
            );
            drop(records);
            self.store
                .trace
                .lock()
                .unwrap()
                .push(format!("git:{}", key(path)));
            self.fake
                .merge(path, branch, expected_before, source, message, attribution)
        }
    }

    fn durable_record(root: &Path, plan: &super::super::MergePlan) -> MergeOperationRecord {
        create_record(
            root,
            plan,
            "merge_test",
            &FixedClock::new(TimestampMs(42)),
            &context(false),
        )
        .unwrap()
    }

    struct DriftOnInspection {
        backend: Git2Backend,
        drift_path: std::path::PathBuf,
        inspected: RefCell<Vec<String>>,
    }

    impl ExecutionBackend for DriftOnInspection {
        fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
            self.inspected.borrow_mut().push(key(path).to_owned());
            if path == self.drift_path {
                let before = self.backend.head(path)?.commit.unwrap();
                commit_file(
                    path,
                    "drift.txt",
                    "branch moved after planning\n",
                    "external branch move",
                    &[git2::Oid::from_str(&before).unwrap()],
                )
                .unwrap();
            }
            ExecutionBackend::inspect(&self.backend, path, branch, source)
        }

        fn merge(
            &self,
            path: &Path,
            branch: &str,
            expected_before: &str,
            source: &str,
            message: &str,
            attribution: Option<&crate::model::OperationAttribution>,
        ) -> ModelResult<GitIntegrateResult> {
            ExecutionBackend::merge(
                &self.backend,
                path,
                branch,
                expected_before,
                source,
                message,
                attribution,
            )
        }
    }

    fn real_three_member_plan(
        root: &Path,
        backend: &Git2Backend,
    ) -> (super::super::MergePlan, Vec<String>, Vec<String>) {
        backend.create_repo(root).unwrap();
        let mut lock = artifact::LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_ops".to_owned(),
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            members: Default::default(),
        };
        let mut bases = Vec::new();
        let mut sources = Vec::new();
        for path in ["app", "lib", "tool"] {
            let repo = root.join(path);
            backend.create_repo(&repo).unwrap();
            let before = commit_file(&repo, "README.md", "base\n", "base", &[]).unwrap();
            backend
                .branch_create(&repo, "feature/source", "HEAD")
                .unwrap();
            backend.switch_branch(&repo, "feature/source").unwrap();
            let source = commit_file(
                &repo,
                "source.txt",
                "source\n",
                "source",
                &[git2::Oid::from_str(&before).unwrap()],
            )
            .unwrap();
            backend.switch_branch(&repo, "main").unwrap();
            let mut state = test_member_state(path, Some(before.clone()), false);
            state.source_id = Some(format!("src_{path}"));
            lock.members.insert(format!("mem_{path}"), state);
            bases.push(before);
            sources.push(source);
        }
        artifact::write_manifest(
            root,
            &artifact::ManifestArtifact {
                schema: artifact::WORKSPACE_SCHEMA.to_owned(),
                workspace: artifact::WorkspaceHeader {
                    id: "ws_ops".to_owned(),
                },
                members: ["app", "lib", "tool"]
                    .into_iter()
                    .map(|path| artifact::ManifestMember {
                        id: format!("mem_{path}"),
                        path: path.to_owned(),
                        source_kind: artifact::ArtifactSourceKind::Git,
                        source_id: format!("src_{path}"),
                        active: true,
                        desired: None,
                        remotes: Vec::new(),
                    })
                    .collect(),
            },
        )
        .unwrap();
        artifact::write_lock(root, &lock).unwrap();
        let request = crate::MergeRequest {
            meta: request_meta(),
            op: crate::MergeOp::Start,
            source_ref: Some("feature/source".to_owned()),
            ..Default::default()
        };
        let plan = plan_merge(backend, root, &request).unwrap();
        (plan, bases, sources)
    }

    #[derive(Clone, Copy)]
    enum ActionFixture {
        FastForward,
        TrueMerge,
        Conflict,
    }

    fn single_real_plan(
        root: &Path,
        backend: &Git2Backend,
        fixture: ActionFixture,
    ) -> super::super::MergePlan {
        let (mut plan, bases, sources) = real_three_member_plan(root, backend);
        let app = root.join("app");
        match fixture {
            ActionFixture::FastForward => {}
            ActionFixture::TrueMerge => {
                commit_file(
                    &app,
                    "local.txt",
                    "local\n",
                    "local",
                    &[git2::Oid::from_str(&bases[0]).unwrap()],
                )
                .unwrap();
            }
            ActionFixture::Conflict => {
                backend.switch_branch(&app, "feature/source").unwrap();
                commit_file(
                    &app,
                    "README.md",
                    "source\n",
                    "source conflict",
                    &[git2::Oid::from_str(&sources[0]).unwrap()],
                )
                .unwrap();
                backend.switch_branch(&app, "main").unwrap();
                commit_file(
                    &app,
                    "README.md",
                    "local\n",
                    "local conflict",
                    &[git2::Oid::from_str(&bases[0]).unwrap()],
                )
                .unwrap();
            }
        }
        if !matches!(fixture, ActionFixture::FastForward) {
            plan = plan_merge(
                backend,
                root,
                &crate::MergeRequest {
                    meta: request_meta(),
                    op: crate::MergeOp::Start,
                    source_ref: Some("feature/source".to_owned()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        plan.participants.truncate(1);
        plan
    }

    fn resume_request() -> crate::MergeRequest {
        crate::MergeRequest {
            meta: request_meta(),
            op: crate::MergeOp::Resume,
            merge_id: Some("merge_test".to_owned()),
            ..Default::default()
        }
    }

    fn resume(
        backend: &Git2Backend,
        store: &MemoryStore,
        root: &Path,
    ) -> ModelResult<crate::MergeResponse> {
        let context = context(false);
        resume_with_context(backend, store, root, &context)
    }

    fn resume_with_context(
        backend: &Git2Backend,
        store: &MemoryStore,
        root: &Path,
        context: &OperationContext,
    ) -> ModelResult<crate::MergeResponse> {
        let sink = crate::operation::NullSink;
        let emitter = EventEmitter::new(context, &sink, 0);
        super::super::continue_op::handle_continue(
            backend,
            store,
            root,
            &resume_request(),
            context,
            &emitter,
        )
    }

    fn start_with_outcome_fault(
        backend: &Git2Backend,
        root: &Path,
        plan: &super::super::MergePlan,
    ) -> MemoryStore {
        let store = MemoryStore::default();
        let mut record = durable_record(root, plan);
        store.write_open(root, &record).unwrap();
        *store.fail_write_at.lock().unwrap() = Some(3);
        execute_durable(
            backend,
            &store,
            root,
            &plan.participants,
            None,
            &mut record,
            &EventEmitter::new(&context(false), &crate::operation::NullSink, 0),
        )
        .unwrap_err();
        *store.fail_write_at.lock().unwrap() = None;
        store
    }
    #[test]
    fn conflict_continues_with_frozen_oids_and_maps_response() {
        let fake = Fake::default();
        let plans = plans(&["conflict", "next"]);
        let run = execute_plan(&fake, Path::new("."), &plans, None);
        assert_eq!(run.rows[0].state, PState::Conflicted);
        assert_eq!(run.rows[1].state, PState::Merged);
        assert_eq!(fake.calls.borrow()[1], "next:before-next:source-next:merge");
        let repos = run.rows.into_iter().map(|r| summary(r, "x")).collect();
        let response = merge_response(&context(false), repos, run.errors).unwrap();
        assert_eq!(response.state, OpState::AwaitingResolution);
        assert!(response.open);
        assert_eq!(response.participant_counts.conflicted, 1);
        assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
        let repos = plans
            .iter()
            .map(|plan| summary(Row::new(plan, PState::Planned), "x"))
            .collect();
        let response = merge_response(&context(true), repos, Vec::new()).unwrap();
        assert_eq!(response.state, OpState::Completed);
        assert!(!response.open);
    }
    #[test]
    fn unexpected_failure_stops_and_marks_later_unattempted() {
        let fake = Fake::default();
        let plans = plans(&["first", "fail", "later"]);
        let run = execute_plan(&fake, Path::new("."), &plans, None);
        assert_eq!(run.rows[0].state, PState::Merged);
        assert_eq!(run.rows[1].state, PState::Failed);
        assert_eq!(run.rows[2].state, PState::Unattempted);
        assert_eq!(
            *fake.calls.borrow(),
            [
                "first:before-first:source-first:merge",
                "fail:before-fail:source-fail:merge"
            ]
        );
        assert_eq!(*fake.mutated_before_failure.borrow(), ["fail"]);
        let repos = run
            .rows
            .into_iter()
            .map(|r| summary(r, "x"))
            .collect::<Vec<_>>();
        assert_eq!(repos[1].live_commit, None);
        assert_eq!(repos[2].live_commit, None);
        let response = merge_response(&context(false), repos, run.errors).unwrap();
        assert_eq!(response.state, OpState::Halted);
        assert!(response.open);
    }

    #[test]
    fn durable_execution_persists_before_git_and_emits_only_after_writes() {
        let root = TempDir::new("merge-durable-order");
        let backend = Git2Backend::new();
        let (mut plan, _, _) = real_three_member_plan(root.path(), &backend);
        plan.participants = plans(&["conflict", "next"]);
        let store = MemoryStore::default();
        let mut record = durable_record(root.path(), &plan);
        store.write_open(root.path(), &record).unwrap();
        let sink = TraceSink(&store);
        let emitter = EventEmitter::new(&context(false), &sink, 0);
        emitter.operation_state_changed(record.state.into());
        let spy = DurableSpy {
            store: &store,
            fake: Fake::default(),
        };

        execute_durable(
            &spy,
            &store,
            root.path(),
            &plan.participants,
            None,
            &mut record,
            &emitter,
        )
        .unwrap();
        super::super::persist_operation_transition(
            &store,
            root.path(),
            &mut record,
            OperationState::AwaitingResolution,
            &emitter,
        )
        .unwrap();

        let trace = store.trace.lock().unwrap().clone();
        assert_eq!(
            trace,
            [
                "write:Executing",
                "event:OperationStateChanged",
                "event:MemberStarted",
                "write:Executing",
                "event:ArtifactWritten",
                "git:conflict",
                "write:Executing",
                "event:ArtifactWritten",
                "event:MemberFinished",
                "event:MemberStarted",
                "write:Executing",
                "event:ArtifactWritten",
                "git:next",
                "write:Executing",
                "event:ArtifactWritten",
                "event:MemberFinished",
                "write:AwaitingResolution",
                "event:ArtifactWritten",
                "event:OperationStateChanged",
            ]
        );
        let events = store.events.lock().unwrap();
        let artifacts = events
            .iter()
            .filter(|event| event.kind == crate::EventKind::ArtifactWritten)
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 5);
        assert!(
            artifacts.iter().all(|event| {
                event.artifact_path.as_deref() == Some(".gwz/merge/merge_test.yaml")
            })
        );
        let outcomes = events
            .iter()
            .filter_map(|event| event.merge_member.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].state, crate::MergeParticipantState::Conflicted);
        assert_eq!(outcomes[0].conflict_paths, ["x"]);
        assert_eq!(outcomes[1].state, crate::MergeParticipantState::Merged);
        assert_eq!(
            store.records.lock().unwrap().last().unwrap().state,
            OperationState::AwaitingResolution
        );
        assert!(
            store
                .records
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .participants
                .values()
                .all(|participant| participant.pending_action.is_none())
        );
    }

    #[test]
    fn clean_start_store_failures_adopt_exact_results_without_duplicate_git() {
        for (name, fixture, expected) in [
            (
                "ff",
                ActionFixture::FastForward,
                crate::MergeParticipantState::FastForwarded,
            ),
            (
                "true",
                ActionFixture::TrueMerge,
                crate::MergeParticipantState::Merged,
            ),
        ] {
            let root = TempDir::new(&format!("merge-{name}-action-recovery"));
            let backend = Git2Backend::new();
            let plan = single_real_plan(root.path(), &backend, fixture);
            let store = start_with_outcome_fault(&backend, root.path(), &plan);
            let app = root.path().join("app");
            let result = backend.head(&app).unwrap().commit.unwrap();
            let response = resume(&backend, &store, root.path()).unwrap();

            assert_eq!(response.repos[0].state, expected);
            assert_eq!(
                response.repos[0].resulting_commit.as_deref(),
                Some(&*result)
            );
            assert_eq!(backend.head(&app).unwrap().commit, Some(result));
            assert!(
                store.records.lock().unwrap().last().unwrap().participants["mem_app"]
                    .pending_action
                    .is_none()
            );
        }
    }

    #[test]
    fn conflict_and_resolution_store_failures_reconcile_after_reload() {
        let root = TempDir::new("merge-conflict-action-recovery");
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
        let app = root.path().join("app");
        let store = start_with_outcome_fault(&backend, root.path(), &plan);
        assert!(backend.merge_state(&app).unwrap().is_some());

        let unresolved = resume(&backend, &store, root.path()).unwrap_err();
        assert_eq!(unresolved.code, ErrorCode::MergeDrift);
        assert_eq!(
            store.records.lock().unwrap().last().unwrap().participants["mem_app"].state,
            ParticipantState::Conflicted
        );
        assert!(
            store.records.lock().unwrap().last().unwrap().participants["mem_app"]
                .pending_action
                .is_none()
        );

        std::fs::write(app.join("README.md"), "resolved\n").unwrap();
        backend.stage_paths(&app, &["README.md"]).unwrap();
        let fail_resolution_outcome = *store.writes.lock().unwrap() + 3;
        *store.fail_write_at.lock().unwrap() = Some(fail_resolution_outcome);
        let attribution = attributed_context();
        resume_with_context(&backend, &store, root.path(), &attribution).unwrap_err();
        let resolution_commit = backend.head(&app).unwrap().commit.unwrap();
        assert!(backend.merge_state(&app).unwrap().is_none());
        assert_eq!(
            store.records.lock().unwrap().last().unwrap().participants["mem_app"]
                .pending_action
                .as_ref()
                .unwrap()
                .kind,
            PendingMergeActionKind::ResolveConflict
        );

        *store.fail_write_at.lock().unwrap() = None;
        let response = resume(&backend, &store, root.path()).unwrap();
        assert_eq!(
            response.repos[0].state,
            crate::MergeParticipantState::Continued
        );
        assert_eq!(
            response.repos[0].resulting_commit.as_deref(),
            Some(resolution_commit.as_str())
        );
        assert_eq!(backend.head(&app).unwrap().commit, Some(resolution_commit));
        let repository = git2::Repository::open(&app).unwrap();
        let commit = repository
            .find_commit(
                git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(commit.author().name(), Ok("Merge Request Author"));
        assert_eq!(commit.committer().name(), Ok("Merge Request Committer"));
    }

    #[test]
    fn retry_true_merge_uses_request_author_and_committer() {
        let root = TempDir::new("merge-retry-attribution");
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, ActionFixture::TrueMerge);
        let store = MemoryStore::default();
        let mut record = durable_record(root.path(), &plan);
        record.state = OperationState::RecoveryRequired;
        record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
        store.write_open(root.path(), &record).unwrap();

        let response =
            resume_with_context(&backend, &store, root.path(), &attributed_context()).unwrap();
        let repository = git2::Repository::open(root.path().join("app")).unwrap();
        let commit = repository
            .find_commit(
                git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(commit.author().name(), Ok("Merge Request Author"));
        assert_eq!(commit.author().email(), Ok("merge-author@example.invalid"));
        assert_eq!(commit.committer().name(), Ok("Merge Request Committer"));
        assert_eq!(
            commit.committer().email(),
            Ok("merge-committer@example.invalid")
        );
    }

    #[test]
    fn recovery_required_retry_store_failure_adopts_without_repeating_git() {
        let root = TempDir::new("merge-retry-action-recovery");
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, ActionFixture::FastForward);
        let source = plan.participants[0].source_commit.clone();
        let store = MemoryStore::default();
        let mut record = durable_record(root.path(), &plan);
        record.state = OperationState::RecoveryRequired;
        record.participants.get_mut("mem_app").unwrap().state = ParticipantState::Failed;
        store.write_open(root.path(), &record).unwrap();
        *store.fail_write_at.lock().unwrap() = Some(4);
        resume(&backend, &store, root.path()).unwrap_err();
        assert_eq!(
            backend.head(&root.path().join("app")).unwrap().commit,
            Some(source.clone())
        );
        assert_eq!(
            store.records.lock().unwrap().last().unwrap().participants["mem_app"]
                .pending_action
                .as_ref()
                .unwrap()
                .kind,
            PendingMergeActionKind::FastForward
        );

        *store.fail_write_at.lock().unwrap() = None;
        let response = resume(&backend, &store, root.path()).unwrap();
        assert_eq!(
            response.repos[0].state,
            crate::MergeParticipantState::FastForwarded
        );
        assert_eq!(
            response.repos[0].resulting_commit.as_deref(),
            Some(source.as_str())
        );
        assert_eq!(
            backend.head(&root.path().join("app")).unwrap().commit,
            Some(source)
        );
    }

    #[test]
    fn real_git_drift_halts_with_durable_rows_and_keeps_baseline_lock() {
        let root = TempDir::new("merge-real-halt");
        let backend = Git2Backend::new();
        let (plan, bases, sources) = real_three_member_plan(root.path(), &backend);
        let drifting = DriftOnInspection {
            backend: backend.clone(),
            drift_path: root.path().join("lib"),
            inspected: RefCell::new(Vec::new()),
        };

        let store = MemoryStore::default();
        let mut record = durable_record(root.path(), &plan);
        store.write_open(root.path(), &record).unwrap();
        let sink = TraceSink(&store);
        let emitter = EventEmitter::new(&context(false), &sink, 0);
        execute_durable(
            &drifting,
            &store,
            root.path(),
            &plan.participants,
            None,
            &mut record,
            &emitter,
        )
        .unwrap();
        super::super::persist_operation_transition(
            &store,
            root.path(),
            &mut record,
            OperationState::Halted,
            &emitter,
        )
        .unwrap();
        assert_eq!(*drifting.inspected.borrow(), ["app", "lib"]);
        let response = start_response(&record, &plan.participants, &context(false)).unwrap();

        assert_eq!(response.state, OpState::Halted);
        assert_eq!(
            response.response.meta.aggregate_status,
            AggregateStatus::Failed
        );
        assert_eq!(response.participant_counts.fast_forwarded, 1);
        assert_eq!(response.participant_counts.failed, 1);
        assert_eq!(response.participant_counts.unattempted, 1);
        let ids = response
            .repos
            .iter()
            .map(|repo| repo.target_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["mem_app", "mem_lib", "mem_tool"]);
        assert_eq!(response.repos[0].state, PState::FastForwarded);
        assert_eq!(response.repos[1].state, PState::Failed);
        assert_eq!(response.repos[2].state, PState::Unattempted);
        assert_eq!(
            response.repos[0].live_commit.as_deref(),
            Some(sources[0].as_str())
        );
        assert_eq!(response.repos[1].live_commit, None);
        assert_eq!(response.repos[2].live_commit, None);
        let error = response.repos[1].error.as_ref().unwrap();
        assert_eq!(error.code, ErrorCode::MergeDrift.into());
        assert_eq!(error.member_id.as_deref(), Some("mem_lib"));

        let live = |path| {
            backend
                .head(&root.path().join(path))
                .unwrap()
                .commit
                .unwrap()
        };
        let moved_lib = live("lib");
        assert_ne!(moved_lib, bases[1]);
        assert_eq!(
            (live("app"), live("tool")),
            (sources[0].clone(), bases[2].clone())
        );
        let lock = artifact::read_lock(root.path()).unwrap();
        assert_eq!(
            ["mem_app", "mem_lib", "mem_tool"].map(|id| lock.members[id].commit.as_deref()),
            [
                Some(bases[0].as_str()),
                Some(bases[1].as_str()),
                Some(bases[2].as_str())
            ]
        );
        assert_eq!(record.state, OperationState::Halted);
        assert_eq!(
            record.participants["mem_app"].state,
            ParticipantState::FastForwarded
        );
        assert_eq!(
            record.participants["mem_lib"].state,
            ParticipantState::Failed
        );
        assert_eq!(
            record.participants["mem_tool"].state,
            ParticipantState::Unattempted
        );
        assert!(["app", "lib", "tool"].into_iter().all(|path| {
            backend
                .merge_state(&root.path().join(path))
                .unwrap()
                .is_none()
        }));
    }
}
