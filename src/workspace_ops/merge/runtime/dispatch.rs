use std::path::Path;

use super::super::{
    FileMergeStore, MergeStore, abort, continue_op, discover_open_before_manifest, gc, start,
    status, validate_merge_request,
};
use super::mutation_guard::guarded_workspace_root;
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::operation::{EventSink, OperationRequest};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;

/// All environmental dependencies used by the merge lifecycle are explicit.
pub(crate) struct MergeDependencies<'a, B, S, C, I> {
    pub backend: &'a B,
    pub store: &'a S,
    pub clock: &'a C,
    pub ids: &'a mut I,
    pub events: &'a dyn EventSink,
}

/// First-class merge service entry. I0 validates and dispatches only; feature
/// milestones replace typed phase errors without changing this public signature.
pub fn handle_merge<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
{
    handle_merge_with_events(
        backend,
        start,
        request,
        operation_id,
        &crate::operation::NullSink,
    )
}

pub fn handle_merge_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
{
    let operation_id = operation_id.into();
    let store = FileMergeStore;
    let clock = SystemClock;
    let mut ids = OperationScopedIds::new(&operation_id);
    handle_merge_invocation(
        MergeDependencies {
            backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events,
        },
        start,
        request,
        operation_id,
        true,
    )
}

/// Dependency-injected lifecycle seam used by the persistence milestones.
#[cfg(test)]
pub(crate) fn handle_merge_with_dependencies<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    handle_merge_invocation(dependencies, start, request, operation_id.into(), false)
}

/// Run one accepted merge invocation under its single lifecycle owner.
///
/// Public entry points request the authoritative start gate. Dependency-
/// injected tests use the same lifecycle owner but deliberately bypass that
/// filesystem-owned guard so their supplied store remains authoritative.
fn handle_merge_invocation<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: String,
    enforce_start_gate: bool,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    let emitter = crate::operation::EventEmitter::from_request_meta(
        operation_id.clone(),
        &request.meta,
        dependencies.events,
        0,
    );
    emitter.operation_started();
    let result = (|| {
        let context = OperationRequest::Merge(request.clone()).context(operation_id)?;
        let (_start_guard, effective_start) =
            if enforce_start_gate && request.op == crate::MergeOp::Start {
                guarded_workspace_root(
                    start,
                    request.meta.workspace.as_ref(),
                    crate::operation::OpenMergeCommand::MergeStart,
                    request.meta.dry_run.unwrap_or(false),
                )?
            } else {
                (None, start.to_path_buf())
            };
        validate_merge_request(&request)?;
        dispatch_merge(dependencies, &effective_start, request, context, &emitter)
    })();
    emitter.operation_finished();
    result
}

fn dispatch_merge<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    context: crate::operation::OperationContext,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    match request.op {
        crate::MergeOp::Start => {
            start::handle_start_durable(dependencies, start, &request, &context, emitter)
        }
        crate::MergeOp::Status => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            status::handle_status(
                dependencies.backend,
                dependencies.store,
                &root,
                request.merge_id.as_deref(),
                &context,
            )
        }
        crate::MergeOp::Resume => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            continue_op::handle_continue(
                dependencies.backend,
                dependencies.store,
                &root,
                &request,
                &context,
                emitter,
            )
        }
        crate::MergeOp::Abort => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            abort::handle_abort(
                dependencies.backend,
                dependencies.store,
                &root,
                &request,
                &context,
                emitter,
            )
        }
        crate::MergeOp::Gc => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            gc::handle_gc(
                dependencies.backend,
                dependencies.store,
                &root,
                request.merge_id.as_deref(),
                &context,
            )
        }
    }
}

fn resolve_recovery_root<S: MergeStore>(
    store: &S,
    start: &Path,
    request: &crate::MergeRequest,
) -> ModelResult<std::path::PathBuf> {
    if let Some(root) = request
        .meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.root.as_ref())
    {
        return Ok(std::path::PathBuf::from(root));
    }
    if let Some(recovery) = discover_open_before_manifest(store, start)? {
        return Ok(recovery.root);
    }
    crate::workspace_ops::resolve_workspace_root(start, request.meta.workspace.as_ref())
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> crate::runtime::clock::TimestampMs {
        crate::operation::now_ms()
    }
}

struct OperationScopedIds {
    suffix: String,
    next: u64,
}

impl OperationScopedIds {
    fn new(operation_id: &str) -> Self {
        let suffix = operation_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        Self { suffix, next: 0 }
    }
}

impl IdProvider for OperationScopedIds {
    fn next_id(&mut self, prefix: &str) -> crate::runtime::ids::GeneratedId {
        self.next += 1;
        crate::runtime::ids::GeneratedId::new(format!("{prefix}_{}_{:04}", self.suffix, self.next))
    }
}
