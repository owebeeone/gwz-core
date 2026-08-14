//! Pure ownership contracts for the three checked-artifact bootstrap layers.

use std::path::Path;

use super::capability::{CheckedFsError, RevalidatedPreCatalogPermitV1};

mod managed;
mod runtime;

pub(super) use managed::*;
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

/// Durable first-catalog bootstrap. Only the pre-catalog owner can construct
/// the lifetime-bound value accepted here, immediately after revalidation.
pub(super) trait CatalogBootstrapV1<RetainedRoot> {
    type Catalog;

    fn recover_or_create(
        &self,
        permit: RevalidatedPreCatalogPermitV1<'_, RetainedRoot>,
    ) -> Result<Self::Catalog, CheckedFsError>;
}
