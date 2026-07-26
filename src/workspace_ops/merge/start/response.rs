use super::super::plan::plan_merge;
use super::super::{MergeOperationRecord, MergeParticipantPlan};
use super::prepared::Row;
use crate::git::GitBackend;
#[cfg(test)]
use crate::model::ModelError;
use crate::model::ModelResult;
use crate::operation::{ActionKind, OperationContext};
use crate::{AggregateStatus, MergeOperationState as OpState, MergeParticipantState as PState};
use std::path::Path;

pub(super) fn handle_dry_run<B: GitBackend>(
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

pub(super) fn start_response(
    record: &MergeOperationRecord,
    plan: &[MergeParticipantPlan],
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    decorate_start_response(record.to_response(context)?, plan)
}

pub(super) fn decorate_start_response(
    mut response: crate::MergeResponse,
    plan: &[MergeParticipantPlan],
) -> ModelResult<crate::MergeResponse> {
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
pub(super) fn summary(row: Row<'_>, source_ref: &str) -> crate::MergeRepoSummary {
    let plan = row.plan;
    crate::MergeRepoSummary {
        target_id: plan.target_id.clone(),
        target_kind: plan.target_kind.into(),
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
        conflict_paths: if row.state == PState::Planned {
            plan.predicted_conflict_paths.clone()
        } else {
            row.paths
        },
        continue_eligible: None,
        abort_eligible: None,
        drift: Vec::new(),
        error: row.err,
        pending_action: None,
    }
}
pub(super) fn merge_response(
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
pub(super) fn participant_error(
    plan: &MergeParticipantPlan,
    error: &ModelError,
) -> crate::GwzError {
    let mut wire: crate::GwzError = error.into();
    wire.member_id = Some(plan.target_id.clone());
    wire.member_path = Some(plan.path.clone());
    wire.target_kind = Some(plan.target_kind.into());
    wire
}
