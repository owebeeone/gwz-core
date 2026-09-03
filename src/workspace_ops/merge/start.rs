use super::plan::plan_merge;
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::operation::{EventEmitter, OperationContext};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;
use std::path::Path;

mod decorations;
mod dry_run;
mod record;

pub(super) use decorations::decorate_start_response;
pub(super) use dry_run::handle_dry_run;
use record::{create_record, freeze_merge_messages};

/// Start one merge.
///
/// **M5d (`GwzM5-8M5d-Charter.md` §1).** There is one lifecycle below this
/// function. Ordinary, `--ff-only`, custom-message and `--no-ff` starts all
/// select `V1` — `ACTIVE_WRITER_FLOOR` is `V1` and selection is
/// `max(floor, requested)` — so the version fork the v0 engine hung off is
/// gone with it. What survives here is what was always shared: the dry-run
/// interception, the open-merge refusal, planning, message freezing, record
/// creation and the start decorations.
pub(super) fn handle_start_durable<B, S, C, I>(
    dependencies: super::MergeDependencies<'_, B, S, C, I>,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
    v1: &dyn super::runtime::V1Router,
    start_guard: Option<super::WorkspaceMutationGuard>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: super::MergeStore,
    C: Clock,
    I: IdProvider,
{
    let super::MergeDependencies {
        backend,
        store: _,
        clock,
        ids,
        events: _,
    } = dependencies;
    // Above everything: a dry run plans and answers, and reaches no lock, no
    // record and no lifecycle. See `start/dry_run.rs`.
    if request.meta.dry_run.unwrap_or(false) {
        return handle_dry_run(backend, root, request, context);
    }

    // The occupancy gate, by envelope. On the authoritative path
    // `guarded_workspace_root` has already refused (`runtime/dispatch.rs`),
    // so this is the dependency-injected seam's own gate — but it is also the
    // site charter §2/§10.3 names for the pre-0.14 sentence, and it must not
    // print the v1 remedy for a v0 envelope.
    if let Some(open) = super::classify_open_record(root)? {
        open.refuse_if_pre_014()?;
        return Err(super::runtime::open_merge_start_error(&open.merge_id));
    }
    let mut plan = plan_merge(backend, root, request)?;
    let merge_id = ids.next_id("merge").to_string();
    freeze_merge_messages(
        &mut plan.participants,
        request.message.as_deref(),
        &plan.source_ref,
        &merge_id,
        context,
    )?;
    let record = create_record(root, &plan, &merge_id, clock, context)?;
    // The v1 lifecycle owns the workspace mutator lock for its whole
    // operation — `service::run` takes its own `V1MutationLease`, and that
    // lock is a workspace-wide OS advisory exclusive lock, not a re-entrant
    // one. Release the start gate's guard here, after it has already enforced
    // the open-merge policy. `create_open` re-checks that no record exists, so
    // the handoff cannot publish a second record.
    drop(start_guard);
    // DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1,
    // 2026-09-03): `--filesystem-strict` is a START-only flag, and this is
    // the only place a start's request meets the v1 owner that decides.
    //
    // M5d charter §4 ("Responses"): the prediction is the PLAN's, not the
    // record's — no record version stores it — so the decoration is applied
    // here, where the plan is still in hand, rather than inside
    // `v1_lifecycle/`, which never sees a plan. `selected_targets` is built
    // from `plan.participants` in creation order (`start/record.rs`), and
    // every response arm orders its rows by `selected_targets`, so the zip is
    // exact.
    let response = v1.start(
        root,
        record,
        request.filesystem_strict.unwrap_or(false),
        context,
        emitter,
    )?;
    decorate_start_response(response, &plan.participants)
}
