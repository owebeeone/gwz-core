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

pub fn acquire_workspace_mutation_guard(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<WorkspaceMutationGuard> {
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
    Ok(WorkspaceMutationGuard { root, _lock: lock })
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
    let guard = acquire_workspace_mutation_guard(start, workspace, command)?;
    let root = guard.root().to_path_buf();
    Ok((Some(guard), root))
}
