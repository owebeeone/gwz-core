//! `--dry-run`: predict the merge, write nothing.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §4, "Dry-run"; parity inventory §1).** This
//! code was `merge/start/response.rs`, inside the v0 start engine's own
//! module. The engine is deleted; the dry run is not, and it is deliberately
//! re-homed rather than deleted with its neighbours — the failure class this
//! guards against is exactly the one that shipped in 0.8.0, where a behaviour
//! present on the old path went silently missing on the new one.
//!
//! The guarantee is structural, and the structure is one early return:
//! `handle_start_durable` returns here BEFORE `classify_open_record`, before
//! id minting, before `create_record`, and before `select_record_version` —
//! so raising `ACTIVE_WRITER_FLOOR` to `V1` changes nothing about a dry run.
//! Everything reachable from here is an observer: `plan_merge` is
//! parameterised over `PlanningBackend`, a trait with eight methods and no
//! write verb, and `guarded_workspace_root`'s dry-run arm returns before it
//! takes the workspace mutator lock. `merge/v1_lifecycle/` contains no
//! mention of `dry_run` at all, because it is unreachable from one.
//!
//! A dry run answers `state: Completed`, `aggregate: Accepted`, `open: false`
//! regardless of predicted conflicts, with `merge_id`, `record`,
//! `preservation`, `publication_step` and `crash_recovery` all absent
//! (`gwz-cli/docs/MachineOutput.md:238-239` pins `record: null`).

use super::super::plan::plan_merge;
use super::super::MergeParticipantPlan;
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::operation::{ActionKind, OperationContext};
use crate::{AggregateStatus, MergeOperationState as OpState, MergeParticipantState as PState};
use std::path::Path;

/// One predicted participant row.
///
/// The v0 engine's `Row` carried an executed participant's outcome as well;
/// the executed half left with the engine, so what remains is the planned
/// half — which is all a dry run ever produced.
struct PlannedRow<'a> {
    plan: &'a MergeParticipantPlan,
}

pub(crate) fn handle_dry_run<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let plan = plan_merge(backend, root, request)?;
    let repos = plan
        .participants
        .iter()
        .map(|participant| summary(PlannedRow { plan: participant }, &plan.source_ref))
        .collect();
    merge_response(context, repos, Vec::new())
}

/// One planned row's protocol projection.
///
/// A dry run's rows are all `Planned`, so the executed-outcome fields the v0
/// engine's `Row` carried — `oid`, `paths`, `err` — are constant here:
/// `live_commit` is the participant's `before_commit` (a `Planned` row is
/// neither `Failed` nor `Unattempted` and has no result yet),
/// `resulting_commit` is absent, and `conflict_paths` is the PREDICTION.
fn summary(row: PlannedRow<'_>, source_ref: &str) -> crate::MergeRepoSummary {
    let plan = row.plan;
    crate::MergeRepoSummary {
        target_id: plan.target_id.clone(),
        target_kind: plan.target_kind.into(),
        path: plan.path.clone(),
        source_ref: source_ref.to_owned(),
        source_commit: plan.source_commit.clone(),
        target_branch: plan.target_branch.clone(),
        before_commit: plan.before_commit.clone(),
        live_commit: Some(plan.before_commit.clone()),
        resulting_commit: None,
        state: PState::Planned,
        predicted: plan.analysis,
        prediction_complete: Some(plan.prediction_complete),
        conflict_paths: plan.predicted_conflict_paths.clone(),
        continue_eligible: None,
        abort_eligible: None,
        drift: Vec::new(),
        error: None,
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
        record: None,
        crash_recovery: None,
    })
}
