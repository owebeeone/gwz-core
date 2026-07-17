use super::{MergeParticipantPlan, plan::plan_merge};
use crate::artifact;
use crate::git::{
    GitBackend, GitHeadState, GitIntegrateResult, GitMergeAnalysis, GitMergeAnalysisKind, GitStatus,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, OperationContext, WorkspaceMutatorLock};
use crate::workspace_ops::{resolve_workspace_root, sync_workspace_boundary};
use crate::{AggregateStatus, MergeOperationState as OpState, MergeParticipantState as PState};
use std::path::Path;
pub(super) fn handle_start<B: GitBackend>(
    backend: &B,
    start: &Path,
    request: &crate::MergeRequest,
    context: OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    if request.meta.dry_run.unwrap_or(false) {
        let plan = plan_merge(backend, &root, request)?;
        let repos = plan
            .participants
            .iter()
            .map(|participant| summary(Row::new(participant, PState::Planned), &plan.source_ref))
            .collect();
        return merge_response(&context, repos, Vec::new());
    }
    let _guard = WorkspaceMutatorLock::acquire(&root)?;
    let plan = plan_merge(backend, &root, request)?;
    let mut execution = execute_plan(backend, &root, &plan.participants);
    if !execution.observed.is_empty()
        && let Err(error) = advance_m0_lock(backend, &root, &execution.observed)
    {
        execution.errors.push((&error).into());
    }
    merge_response(
        &context,
        execution
            .rows
            .into_iter()
            .map(|row| summary(row, &plan.source_ref))
            .collect(),
        execution.errors,
    )
}
type Inspection = (GitStatus, GitHeadState, GitMergeAnalysis);
trait ExecutionBackend {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection>;
    fn merge(&self, path: &Path, branch: &str, source: &str) -> ModelResult<GitIntegrateResult>;
}
impl<B: GitBackend> ExecutionBackend for B {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
        Ok((
            self.status(path)?,
            self.head(path)?,
            self.merge_analysis(path, branch, source)?,
        ))
    }
    fn merge(&self, path: &Path, branch: &str, source: &str) -> ModelResult<GitIntegrateResult> {
        GitBackend::merge_upstream(self, path, branch, source)
    }
}
struct Execution<'a> {
    rows: Vec<Row<'a>>,
    observed: Vec<Observed>,
    errors: Vec<crate::GwzError>,
}
struct Observed {
    id: String,
    oid: String,
    branch: String,
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
fn execute_plan<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    participants: &'a [MergeParticipantPlan],
) -> Execution<'a> {
    let mut execution = Execution {
        rows: Vec::with_capacity(participants.len()),
        observed: Vec::new(),
        errors: Vec::new(),
    };
    for (index, participant) in participants.iter().enumerate() {
        match execute_one(backend, root, participant) {
            Ok((summary, observed)) => {
                execution.rows.push(summary);
                execution.observed.extend(observed);
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
fn execute_one<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &'a MergeParticipantPlan,
) -> ModelResult<(Row<'a>, Option<Observed>)> {
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
    let result = backend.merge(&path, &plan.target_branch, &plan.source_commit)?;
    if !result.conflicts.is_empty() {
        if kind != GitMergeAnalysisKind::TrueMerge || result.commit.is_some() {
            return Err(invariant(
                plan,
                "backend returned an invalid conflict result",
            ));
        }
        return Ok((
            Row {
                paths: result.conflicts,
                ..Row::new(plan, PState::Conflicted)
            },
            None,
        ));
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
    Ok((
        Row {
            oid: Some(resulting.clone()),
            ..Row::new(plan, state)
        },
        Some(Observed {
            id: plan.target_id.clone(),
            oid: resulting,
            branch: plan.target_branch.clone(),
        }),
    ))
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
fn advance_m0_lock<B: GitBackend>(
    backend: &B,
    root: &Path,
    observed: &[Observed],
) -> ModelResult<()> {
    let manifest = artifact::read_manifest(root)?;
    let mut lock = artifact::read_lock(root)?;
    for observed in observed {
        let state = lock.members.get_mut(&observed.id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{}'", observed.id),
            )
        })?;
        state.commit = Some(observed.oid.clone());
        state.branch = Some(observed.branch.clone());
        state.detached = Some(false);
        state.dirty = Some(false);
        state.materialized = Some(true);
    }
    artifact::write_lock(root, &lock)?;
    sync_workspace_boundary(backend, root, &manifest, &lock)
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
        live_commit: Some(
            row.oid
                .clone()
                .unwrap_or_else(|| plan.before_commit.clone()),
        ),
        resulting_commit: row.oid,
        state: row.state,
        predicted: plan.analysis,
        prediction_complete: Some(plan.prediction_complete),
        conflict_paths: row.paths,
        continue_eligible: None,
        abort_eligible: None,
        drift: Vec::new(),
        error: row.err,
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
        (OpState::Executing, AggregateStatus::Accepted)
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
        open: false,
        participant_counts: counts,
        repos,
        operation_drift: Vec::new(),
        preservation: None,
        publication_step: None,
    })
}
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
    use std::cell::RefCell;
    #[derive(Default)]
    struct Fake(RefCell<Vec<String>>);
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
        fn merge(&self, path: &Path, _: &str, source: &str) -> ModelResult<GitIntegrateResult> {
            let key = key(path);
            self.0.borrow_mut().push(format!("{key}:{source}"));
            if key == "fail" {
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
    #[test]
    fn conflict_continues_with_frozen_oids_and_maps_response() {
        let fake = Fake::default();
        let plans = plans(&["conflict", "next"]);
        let run = execute_plan(&fake, Path::new("."), &plans);
        assert_eq!(run.rows[0].state, PState::Conflicted);
        assert_eq!(run.rows[1].state, PState::Merged);
        assert_eq!(fake.0.borrow()[1], "next:source-next");
        let repos = run.rows.into_iter().map(|r| summary(r, "x")).collect();
        let response = merge_response(&context(false), repos, run.errors).unwrap();
        assert_eq!(response.state, OpState::AwaitingResolution);
        assert_eq!(response.participant_counts.conflicted, 1);
        assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    }
    #[test]
    fn unexpected_failure_stops_and_marks_later_unattempted() {
        let fake = Fake::default();
        let plans = plans(&["first", "fail", "later"]);
        let run = execute_plan(&fake, Path::new("."), &plans);
        assert_eq!(run.rows[0].state, PState::Merged);
        assert_eq!(run.rows[1].state, PState::Failed);
        assert_eq!(run.rows[2].state, PState::Unattempted);
        assert_eq!(*fake.0.borrow(), ["first:source-first", "fail:source-fail"]);
        assert_eq!(run.errors.len(), 1);
    }
}
