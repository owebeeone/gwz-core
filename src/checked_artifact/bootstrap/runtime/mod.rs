use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::capability::{CheckedFsError, PlatformCapability};
use super::{WorkspaceRuntimeBootstrapV1, WorkspaceRuntimePaths};

mod advisory;
mod catalog_lease;
mod paths;

#[cfg(test)]
mod fault;
#[cfg(test)]
mod tests;

use advisory::AdvisoryLock;
pub(in crate::checked_artifact) use catalog_lease::probe_workspace_admission;
pub(in crate::checked_artifact) use catalog_lease::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
};
pub(crate) use catalog_lease::{CatalogLeaseTargetWitnessV1, CatalogMutationLeaseV1};
use paths::{
    RetainedDirectory, ensure_child_directory, open_or_create_file, resolve_workspace_paths,
    retain_ambient_directory, revalidate_ambient_directory, revalidate_child_directory,
    revalidate_file, revalidate_workspace_repository,
};

const BOOTSTRAP_GUARD_NAME: &str = "gwz-runtime-bootstrap-v1.lock";
const LOCKS_DIRECTORY_NAME: &str = "locks";
const WORKSPACE_MUTATOR_LOCK_NAME: &str = "workspace-mutator.lock";

struct RuntimeBootstrap;

pub(crate) struct WorkspaceRuntimeLease {
    lock: AdvisoryLock,
    path: PathBuf,
    workspace_root: RetainedDirectory,
    workspace_git_dir: RetainedDirectory,
    runtime_dir: RetainedDirectory,
    locks_dir: RetainedDirectory,
    workspace_root_path: PathBuf,
    workspace_git_dir_path: PathBuf,
}

impl WorkspaceRuntimeLease {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn catalog_mutation_lease(&self) -> CatalogMutationLeaseV1<'_> {
        CatalogMutationLeaseV1::from_workspace_runtime(self)
    }

    fn workspace_root_path(&self) -> &Path {
        &self.workspace_root_path
    }

    fn workspace_git_dir_path(&self) -> &Path {
        &self.workspace_git_dir_path
    }

    fn workspace_root_handle(&self) -> &cap_std::fs::Dir {
        self.workspace_root.handle()
    }

    fn workspace_git_dir_handle(&self) -> &cap_std::fs::Dir {
        self.workspace_git_dir.handle()
    }

    fn revalidate_catalog_target(&self) -> Result<(), CheckedFsError> {
        revalidate_workspace_catalog_target(
            &WorkspaceRuntimePaths::new(&self.workspace_root_path, &self.workspace_git_dir_path),
            &self.workspace_root,
            &self.workspace_git_dir,
            &self.runtime_dir,
            &self.locks_dir,
            &self.lock,
        )
    }
}

pub(crate) fn try_acquire_workspace_runtime(
    root: &Path,
) -> ModelResult<Option<WorkspaceRuntimeLease>> {
    let resolved = resolve_workspace_paths(root).map_err(runtime_error)?;
    let lease = RuntimeBootstrap
        .try_acquire(WorkspaceRuntimePaths::new(
            &resolved.workspace_root,
            &resolved.workspace_git_dir,
        ))
        .map_err(runtime_error)?;
    Ok(lease.map(|mut lease| {
        lease.path = root
            .join(crate::workspace::RUNTIME_DIR)
            .join(LOCKS_DIRECTORY_NAME)
            .join(WORKSPACE_MUTATOR_LOCK_NAME);
        lease
    }))
}

impl WorkspaceRuntimeBootstrapV1 for RuntimeBootstrap {
    type Lease = WorkspaceRuntimeLease;

    fn try_acquire(
        &self,
        paths: WorkspaceRuntimePaths<'_>,
    ) -> Result<Option<Self::Lease>, CheckedFsError> {
        let workspace_root = retain_ambient_directory(paths.workspace_root(), "workspace root")?;
        let workspace_git_dir =
            retain_ambient_directory(paths.workspace_git_dir(), "workspace Git directory")?;
        revalidate_workspace_repository(paths.workspace_root(), paths.workspace_git_dir())?;

        let guard_file = open_or_create_file(
            workspace_git_dir.handle(),
            OsStr::new(BOOTSTRAP_GUARD_NAME),
            "runtime bootstrap guard",
        )?;
        let Some(guard) = try_advisory_lock(guard_file)? else {
            return Ok(None);
        };

        let runtime_dir = ensure_child_directory(
            workspace_root.handle(),
            OsStr::new(crate::workspace::RUNTIME_DIR),
            "workspace runtime directory",
        )?;
        let locks_dir = ensure_child_directory(
            runtime_dir.handle(),
            OsStr::new(LOCKS_DIRECTORY_NAME),
            "workspace locks directory",
        )?;
        let lease_file = open_or_create_file(
            locks_dir.handle(),
            OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
            "workspace mutator lease",
        )?;
        #[cfg(test)]
        fault::run(fault::RuntimeBootstrapFault::FinalLeaseOpen);
        let Some(lock) = try_advisory_lock(lease_file)? else {
            return Ok(None);
        };
        #[cfg(test)]
        fault::run(fault::RuntimeBootstrapFault::FinalLeaseLock);

        revalidate_runtime_tree(
            &paths,
            &workspace_root,
            &workspace_git_dir,
            &runtime_dir,
            &locks_dir,
            &guard,
            &lock,
        )?;

        drop(guard);
        Ok(Some(WorkspaceRuntimeLease {
            lock,
            path: paths
                .workspace_root()
                .join(crate::workspace::RUNTIME_DIR)
                .join(LOCKS_DIRECTORY_NAME)
                .join(WORKSPACE_MUTATOR_LOCK_NAME),
            workspace_root,
            workspace_git_dir,
            runtime_dir,
            locks_dir,
            workspace_root_path: paths.workspace_root().to_path_buf(),
            workspace_git_dir_path: paths.workspace_git_dir().to_path_buf(),
        }))
    }
}

#[allow(clippy::too_many_arguments)]
fn revalidate_runtime_tree(
    paths: &WorkspaceRuntimePaths<'_>,
    workspace_root: &RetainedDirectory,
    workspace_git_dir: &RetainedDirectory,
    runtime_dir: &RetainedDirectory,
    locks_dir: &RetainedDirectory,
    guard: &AdvisoryLock,
    lock: &AdvisoryLock,
) -> Result<(), CheckedFsError> {
    revalidate_workspace_catalog_target(
        paths,
        workspace_root,
        workspace_git_dir,
        runtime_dir,
        locks_dir,
        lock,
    )?;
    revalidate_file(
        workspace_git_dir.handle(),
        OsStr::new(BOOTSTRAP_GUARD_NAME),
        guard.file(),
        "runtime bootstrap guard",
    )
}

#[allow(clippy::too_many_arguments)]
fn revalidate_workspace_catalog_target(
    paths: &WorkspaceRuntimePaths<'_>,
    workspace_root: &RetainedDirectory,
    workspace_git_dir: &RetainedDirectory,
    runtime_dir: &RetainedDirectory,
    locks_dir: &RetainedDirectory,
    lock: &AdvisoryLock,
) -> Result<(), CheckedFsError> {
    revalidate_ambient_directory(paths.workspace_root(), workspace_root, "workspace root")?;
    revalidate_ambient_directory(
        paths.workspace_git_dir(),
        workspace_git_dir,
        "workspace Git directory",
    )?;
    revalidate_workspace_repository(paths.workspace_root(), paths.workspace_git_dir())?;
    revalidate_child_directory(
        workspace_root.handle(),
        OsStr::new(crate::workspace::RUNTIME_DIR),
        runtime_dir.identity(),
        "workspace runtime directory",
    )?;
    revalidate_child_directory(
        runtime_dir.handle(),
        OsStr::new(LOCKS_DIRECTORY_NAME),
        locks_dir.identity(),
        "workspace locks directory",
    )?;
    revalidate_file(
        locks_dir.handle(),
        OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
        lock.file(),
        "workspace mutator lease",
    )
}

fn try_advisory_lock(file: cap_std::fs::File) -> Result<Option<AdvisoryLock>, CheckedFsError> {
    AdvisoryLock::try_acquire(file).map_err(|source| {
        if source.kind() == io::ErrorKind::Unsupported {
            CheckedFsError::unsupported(PlatformCapability::RuntimeAdvisoryLock, source.to_string())
        } else {
            CheckedFsError::io("acquire runtime advisory lock", source)
        }
    })
}

fn runtime_error(error: CheckedFsError) -> ModelError {
    match error {
        CheckedFsError::Unsupported { detail, .. } => {
            ModelError::new(ErrorCode::UnsupportedOperation, detail)
        }
        CheckedFsError::Io { operation, source } => ModelError::new(
            ErrorCode::IoError,
            format!("workspace runtime bootstrap {operation}: {source}"),
        ),
        CheckedFsError::Ambiguous { fact, detail } => ModelError::new(
            ErrorCode::IoError,
            format!("workspace runtime bootstrap rejected {fact}: {detail}"),
        ),
    }
}
