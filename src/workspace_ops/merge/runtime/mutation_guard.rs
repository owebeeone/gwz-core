use std::path::{Path, PathBuf};

use super::super::{FileMergeStore, MergeStore, discover_open_before_manifest};
use super::open_gate::enforce_workspace_open_merge_gate;
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

pub fn acquire_workspace_mutation_guard(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<WorkspaceMutationAccess> {
    let root = if command == crate::operation::OpenMergeCommand::StageConflictResolution
        && workspace
            .and_then(|workspace| workspace.root.as_ref())
            .is_none()
    {
        discover_open_before_manifest(&FileMergeStore, start)?
            .map(|recovery| recovery.root)
            .map_or_else(
                || crate::workspace_ops::resolve_workspace_root(start, workspace),
                Ok,
            )?
    } else {
        crate::workspace_ops::resolve_workspace_root(start, workspace)?
    };
    let lock = WorkspaceMutatorLock::acquire(&root)?;
    let store = FileMergeStore;
    let open = store.discover_open(&root)?;
    crate::operation::enforce_open_merge_gate(
        open.as_ref().map(|record| record.merge_id.as_str()),
        command,
    )?;
    let guard = WorkspaceMutationGuard { root, _lock: lock };
    Ok(if dry_run {
        WorkspaceMutationAccess::PlanOnly(guard)
    } else {
        WorkspaceMutationAccess::Mutating(guard)
    })
}

/// Resolve and enforce a gated dry-run without taking the mutator lock, or
/// retain the authoritative guard for a real mutation.
pub(crate) fn guarded_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<(Option<WorkspaceMutationGuard>, PathBuf)> {
    if dry_run {
        enforce_workspace_open_merge_gate(start, workspace, command)?;
        return Ok((
            None,
            crate::workspace_ops::resolve_workspace_root(start, workspace)?,
        ));
    }
    let guard = acquire_workspace_mutation_guard(start, workspace, command, false)?
        .into_guard()
        .expect("a non-dry-run acquisition always yields the mutating arm");
    let root = guard.root().to_path_buf();
    Ok((Some(guard), root))
}
