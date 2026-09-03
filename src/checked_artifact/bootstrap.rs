//! Pure ownership contracts for the three checked-artifact bootstrap layers.

use std::path::Path;

use super::capability::CheckedFsError;

mod managed;
mod runtime;

pub(super) use managed::*;
pub(super) use runtime::CatalogLeaseTargetWitnessV1;
pub(crate) use runtime::CatalogMutationLeaseV1;
/// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §2,
/// 2026-09-03): the catalog's admission answer without its lease.
pub(super) use runtime::probe_workspace_admission;
#[allow(
    unused_imports,
    reason = "R2-C0 freezes catalog lease interfaces before the C1 owner consumes them"
)]
pub(in crate::checked_artifact) use runtime::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
};
pub(crate) use runtime::{WorkspaceRuntimeLease, try_acquire_workspace_runtime};

pub(super) struct WorkspaceRuntimePaths<'a> {
    workspace_root: &'a Path,
    workspace_git_dir: &'a Path,
}

impl<'a> WorkspaceRuntimePaths<'a> {
    pub(super) fn new(workspace_root: &'a Path, workspace_git_dir: &'a Path) -> Self {
        Self {
            workspace_root,
            workspace_git_dir,
        }
    }

    pub(super) fn workspace_root(&self) -> &Path {
        self.workspace_root
    }

    pub(super) fn workspace_git_dir(&self) -> &Path {
        self.workspace_git_dir
    }
}

/// Capability-neutral live-process coordination. Implementors may create only
/// the fixed runtime guard, GWZ and lock directories, and the final lease file.
pub(super) trait WorkspaceRuntimeBootstrapV1 {
    type Lease;

    fn try_acquire(
        &self,
        paths: WorkspaceRuntimePaths<'_>,
    ) -> Result<Option<Self::Lease>, CheckedFsError>;
}
