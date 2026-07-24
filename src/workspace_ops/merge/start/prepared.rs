use super::super::MergeParticipantPlan;
use super::execution::ExecutionBackend;
#[cfg(test)]
use super::response::participant_error;
use crate::MergeParticipantState as PState;
use crate::git::{GitMergeAnalysisKind, GitPreparedMerge};
use crate::model::{ErrorCode, ModelError, ModelResult};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreparedAction {
    pub(super) kind: GitMergeAnalysisKind,
    pub(super) result: GitPreparedMerge,
}

#[cfg(test)]
pub(super) struct Execution<'a> {
    pub(super) rows: Vec<Row<'a>>,
    pub(super) errors: Vec<crate::GwzError>,
}
pub(super) struct Row<'a> {
    pub(super) plan: &'a MergeParticipantPlan,
    pub(super) state: PState,
    pub(super) oid: Option<String>,
    pub(super) paths: Vec<String>,
    pub(super) err: Option<crate::GwzError>,
}
impl<'a> Row<'a> {
    pub(super) fn new(plan: &'a MergeParticipantPlan, state: PState) -> Self {
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
pub(super) fn execute_plan<'a, B: ExecutionBackend>(
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
    let prepared = prepare_one(backend, root, plan, attribution)?;
    execute_prepared(backend, root, plan, &prepared)
}

pub(super) fn prepare_one<B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &MergeParticipantPlan,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<PreparedAction> {
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
    let result = backend.prepare_merge(
        &path,
        &plan.target_branch,
        &plan.before_commit,
        &plan.source_commit,
        attribution,
    )?;
    if prepared_kind(&result) != kind {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "member '{}' merge result changed during preparation",
                plan.target_id
            ),
        ));
    }
    Ok(PreparedAction { kind, result })
}

pub(super) fn execute_prepared<'a, B: ExecutionBackend>(
    backend: &B,
    root: &Path,
    plan: &'a MergeParticipantPlan,
    prepared: &PreparedAction,
) -> ModelResult<Row<'a>> {
    let result = backend.merge(
        &root.join(&plan.path),
        &plan.target_branch,
        &plan.before_commit,
        &plan.source_commit,
        &plan.commit_message,
        &prepared.result,
    )?;
    if !result.conflicts.is_empty() {
        if prepared.kind != GitMergeAnalysisKind::TrueMerge || result.commit.is_some() {
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
    if (prepared.kind == GitMergeAnalysisKind::UpToDate && resulting != plan.before_commit)
        || (prepared.kind == GitMergeAnalysisKind::FastForward && resulting != plan.source_commit)
    {
        return Err(invariant(
            plan,
            "backend returned the wrong clean result commit",
        ));
    }
    let state = match prepared.kind {
        GitMergeAnalysisKind::UpToDate => PState::UpToDate,
        GitMergeAnalysisKind::FastForward => PState::FastForwarded,
        GitMergeAnalysisKind::TrueMerge => PState::Merged,
    };
    Ok(Row {
        oid: Some(resulting),
        ..Row::new(plan, state)
    })
}

fn prepared_kind(prepared: &GitPreparedMerge) -> GitMergeAnalysisKind {
    match prepared {
        GitPreparedMerge::Unchanged => GitMergeAnalysisKind::UpToDate,
        GitPreparedMerge::FastForward => GitMergeAnalysisKind::FastForward,
        GitPreparedMerge::ExpectedConflict | GitPreparedMerge::Commit(_) => {
            GitMergeAnalysisKind::TrueMerge
        }
    }
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
fn invariant(plan: &MergeParticipantPlan, message: &str) -> ModelError {
    ModelError::new(
        ErrorCode::GitCommandFailed,
        format!("member '{}': {message}", plan.target_id),
    )
}
