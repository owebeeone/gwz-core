use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::capability::{CheckedFsError, PlatformCapability};
use super::{WorkspaceRuntimeBootstrapV1, WorkspaceRuntimePaths};

mod advisory;
mod paths;

#[cfg(test)]
mod fault;
#[cfg(test)]
mod tests;

use advisory::AdvisoryLock;
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
    _lock: AdvisoryLock,
    path: PathBuf,
    _workspace_root: RetainedDirectory,
    _workspace_git_dir: RetainedDirectory,
    _runtime_dir: RetainedDirectory,
    _locks_dir: RetainedDirectory,
}

impl WorkspaceRuntimeLease {
    pub(crate) fn path(&self) -> &Path {
        &self.path
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
        fault::run(fault::RuntimeBootstrapFault::AfterFinalLeaseOpen);
        let Some(lock) = try_advisory_lock(lease_file)? else {
            return Ok(None);
        };
        #[cfg(test)]
        fault::run(fault::RuntimeBootstrapFault::AfterFinalLeaseLock);

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
            _lock: lock,
            path: paths
                .workspace_root()
                .join(crate::workspace::RUNTIME_DIR)
                .join(LOCKS_DIRECTORY_NAME)
                .join(WORKSPACE_MUTATOR_LOCK_NAME),
            _workspace_root: workspace_root,
            _workspace_git_dir: workspace_git_dir,
            _runtime_dir: runtime_dir,
            _locks_dir: locks_dir,
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
    revalidate_ambient_directory(paths.workspace_root(), workspace_root, "workspace root")?;
    revalidate_ambient_directory(
        paths.workspace_git_dir(),
        workspace_git_dir,
        "workspace Git directory",
    )?;
    revalidate_workspace_repository(paths.workspace_root(), paths.workspace_git_dir())?;
    revalidate_file(
        workspace_git_dir.handle(),
        OsStr::new(BOOTSTRAP_GUARD_NAME),
        guard.file(),
        "runtime bootstrap guard",
    )?;
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
