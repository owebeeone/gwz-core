use std::path::{Path, PathBuf};

use super::super::{classify_open_record, discover_open_envelope_before_manifest};
use crate::model::ModelResult;
use crate::operation::WorkspaceMutatorLock;

/// Authoritative guard for an existing-workspace mutation.
///
/// The effective request workspace is resolved before locking; the open-merge
/// policy is then checked while the same lock remains held for the caller's
/// mutation. Public mutating handlers migrate to this seam during the M2a
/// remediation wave so direct core callers cannot bypass driver checks.
pub struct WorkspaceMutationGuard {
    root: PathBuf,
    _lock: WorkspaceMutatorLock,
}

impl WorkspaceMutationGuard {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The outcome of asking a workspace for permission to mutate it.
///
/// The seam takes `dry_run` so that NO CALLER CAN COMPILE without stating an answer —
/// that is its guarantee. The arm is the record of the answer: [`Self::Mutating`]
/// hands back the guard that authorizes the conf-gate reconcile, [`Self::PlanOnly`]
/// hands back only a root to read. It does not by itself stop a handler from writing
/// to the filesystem through other paths; each handler still gates its own writes on
/// the answer (review P3-1, 2026-09-02). The mutator lock is held in both arms: a
/// plan must observe the same workspace a real run would have mutated, and the
/// pre-existing dry-run behaviour of the handlers on this seam
/// (add/commit/snapshot/capture/tag) held it too.
pub enum WorkspaceMutationAccess {
    /// A real mutation: the inner guard authorizes the writes.
    Mutating(WorkspaceMutationGuard),
    /// A dry run: the workspace root is resolved and locked, but nothing may be written.
    PlanOnly(WorkspaceMutationGuard),
}

impl WorkspaceMutationAccess {
    pub fn root(&self) -> &Path {
        match self {
            Self::Mutating(guard) | Self::PlanOnly(guard) => guard.root(),
        }
    }

    /// The guard that authorizes a write, or `None` for a dry run. Every write a
    /// handler performs must be reachable only through this `Option`.
    pub fn writes(&self) -> Option<&WorkspaceMutationGuard> {
        match self {
            Self::Mutating(guard) => Some(guard),
            Self::PlanOnly(_) => None,
        }
    }

    pub fn is_dry_run(&self) -> bool {
        matches!(self, Self::PlanOnly(_))
    }

    /// Consume the access, yielding the write-authorizing guard for a real
    /// mutation and `None` for a dry run.
    pub fn into_guard(self) -> Option<WorkspaceMutationGuard> {
        match self {
            Self::Mutating(guard) => Some(guard),
            Self::PlanOnly(_) => None,
        }
    }
}

/// Acquire the mutation guard using the services composed for this invocation.
///
/// This legacy adapter accepts an explicit process-independent start directory.
/// New serialized requests must use
/// [`acquire_workspace_mutation_guard_for_request_with_services`].
pub fn acquire_workspace_mutation_guard_with_services(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    let root = resolve_guard_root_from_legacy_start(start, workspace, command)?;
    acquire_workspace_mutation_guard_at_root(services, root, command, dry_run)
}

/// Acquire the mutation guard for a request with its serialized caller context.
///
/// The context is resolved here, at the locking boundary, rather than inherited
/// from the executor's process directory.
pub fn acquire_workspace_mutation_guard_for_request_with_services(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    meta: &crate::RequestMeta,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    let root = resolve_guard_root_from_request(start, meta, command)?;
    acquire_workspace_mutation_guard_at_root(services, root, command, dry_run)
}

fn acquire_workspace_mutation_guard_at_root(
    services: &crate::operation_context::OperationServices,
    root: PathBuf,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    let lock = WorkspaceMutatorLock::acquire_in(services, &root)?;
    // A1: by envelope, for the reason `enforce_workspace_open_merge_gate`
    // states — the v0 store's decoder cannot read an open v1 record, and a
    // version error here replaced the open-merge remedy with misdirection.
    let open = classify_open_record(&root)?;
    super::open_gate::enforce_open_merge_gate_for_envelope(open.as_ref(), command)?;
    let guard = WorkspaceMutationGuard { root, _lock: lock };
    Ok(if dry_run {
        WorkspaceMutationAccess::PlanOnly(guard)
    } else {
        WorkspaceMutationAccess::Mutating(guard)
    })
}

fn resolve_guard_root_from_legacy_start(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<PathBuf> {
    if command == crate::operation::OpenMergeCommand::StageConflictResolution
        && workspace
            .and_then(|workspace| workspace.root.as_ref())
            .is_none()
    {
        return discover_open_envelope_before_manifest(start)?
            .map(|envelope| envelope.root)
            .map_or_else(
                || crate::workspace_ops::resolve_workspace_root(start, workspace),
                Ok,
            );
    }
    crate::workspace_ops::resolve_workspace_root(start, workspace)
}

fn resolve_guard_root_from_request(
    start: &Path,
    meta: &crate::RequestMeta,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<PathBuf> {
    let caller_start = crate::workspace_ops::invocation_start(start, meta)?;
    if command == crate::operation::OpenMergeCommand::StageConflictResolution
        && meta
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.root.as_ref())
            .is_none()
    {
        return discover_open_envelope_before_manifest(&caller_start)?
            .map(|envelope| envelope.root)
            .map_or_else(
                || crate::workspace_ops::resolve_request_workspace_root(start, meta),
                Ok,
            );
    }
    crate::workspace_ops::resolve_request_workspace_root(start, meta)
}

#[allow(dead_code)]
pub(crate) fn acquire_workspace_mutation_guard_in(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    acquire_workspace_mutation_guard_with_services(services, start, workspace, command, dry_run)
}

pub(crate) fn acquire_workspace_mutation_guard_for_request_in(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    meta: &crate::RequestMeta,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    acquire_workspace_mutation_guard_for_request_with_services(
        services, start, meta, command, dry_run,
    )
}

/// Resolve and enforce a gated dry-run without taking the mutator lock, or
/// retain the authoritative guard for a real mutation.
#[allow(dead_code)]
pub(crate) fn guarded_workspace_root_in(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<(Option<WorkspaceMutationGuard>, PathBuf)> {
    let root = resolve_guard_root_from_legacy_start(start, workspace, command)?;
    guarded_workspace_root_at_root(services, root, command, dry_run)
}

pub(crate) fn guarded_workspace_root_for_request_in(
    services: &crate::operation_context::OperationServices,
    start: &Path,
    meta: &crate::RequestMeta,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<(Option<WorkspaceMutationGuard>, PathBuf)> {
    let root = resolve_guard_root_from_request(start, meta, command)?;
    guarded_workspace_root_at_root(services, root, command, dry_run)
}

fn guarded_workspace_root_at_root(
    services: &crate::operation_context::OperationServices,
    root: PathBuf,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<(Option<WorkspaceMutationGuard>, PathBuf)> {
    if dry_run {
        let open = classify_open_record(&root)?;
        super::open_gate::enforce_open_merge_gate_for_envelope(open.as_ref(), command)?;
        return Ok((None, root));
    }
    let guard = acquire_workspace_mutation_guard_at_root(services, root, command, false)?
        .into_guard()
        .expect("a non-dry-run acquisition always yields the mutating arm");
    let root = guard.root().to_path_buf();
    Ok((Some(guard), root))
}
