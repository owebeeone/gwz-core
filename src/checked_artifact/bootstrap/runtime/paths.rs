use crate::filesystem::{FileSystem, FsDirectory, FsFile, FsIdentity, FsKind};
use crate::operation_context::OperationContext;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use super::super::super::capability::CheckedFsError;

pub(super) struct RetainedDirectory {
    dir: FsDirectory,
    identity: FsIdentity,
}

impl RetainedDirectory {
    pub(super) fn handle(&self) -> &FsDirectory {
        &self.dir
    }
    pub(super) fn identity(&self) -> FsIdentity {
        self.identity
    }
}

pub(super) struct ResolvedWorkspacePaths {
    pub(super) workspace_root: PathBuf,
    pub(super) workspace_git_dir: PathBuf,
}

pub(super) fn resolve_workspace_paths_in(
    context: &OperationContext,
    root: &Path,
) -> Result<ResolvedWorkspacePaths, CheckedFsError> {
    let filesystem = context.filesystem();
    reject_non_directory_or_symlink(filesystem, root, "workspace root")?;
    let workspace_root = filesystem
        .canonical_path(root)
        .map_err(|source| CheckedFsError::io("canonicalize workspace root", source))?;
    let repository = context
        .repository()
        .repository_paths(&workspace_root)
        .map_err(|error| {
            CheckedFsError::io(
                "open workspace Git repository",
                io::Error::other(error.to_string()),
            )
        })?;
    let workdir = repository.worktree.ok_or_else(|| {
        CheckedFsError::ambiguous("workspace root", "bare Git repositories are not workspaces")
    })?;
    let observed_workdir = filesystem
        .canonical_path(&workdir)
        .map_err(|source| CheckedFsError::io("canonicalize Git worktree", source))?;
    if observed_workdir != workspace_root {
        return Err(CheckedFsError::ambiguous(
            "workspace root",
            "path is not the Git worktree root",
        ));
    }
    let workspace_git_dir = filesystem
        .canonical_path(&repository.git_dir)
        .map_err(|source| CheckedFsError::io("canonicalize workspace Git directory", source))?;
    reject_non_directory_or_symlink(filesystem, &workspace_git_dir, "workspace Git directory")?;
    Ok(ResolvedWorkspacePaths {
        workspace_root,
        workspace_git_dir,
    })
}

pub(super) fn retain_ambient_directory_in(
    filesystem: &dyn FileSystem,
    path: &Path,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    reject_non_directory_or_symlink(filesystem, path, label)?;
    let dir = filesystem
        .open_directory(path)
        .map_err(|source| CheckedFsError::io("open retained runtime directory", source))?;
    let identity = filesystem
        .directory_identity(&dir)
        .map_err(|source| CheckedFsError::io("identify retained runtime directory", source))?;
    let retained = RetainedDirectory { dir, identity };
    revalidate_ambient_directory(path, &retained, label)?;
    Ok(retained)
}

pub(super) fn revalidate_ambient_directory(
    path: &Path,
    expected: &RetainedDirectory,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let filesystem = expected.dir.filesystem();
    reject_non_directory_or_symlink(filesystem, path, label)?;
    let current = filesystem
        .open_directory(path)
        .map_err(|source| CheckedFsError::io("reopen retained runtime directory", source))?;
    let current_identity = filesystem
        .directory_identity(&current)
        .map_err(|source| CheckedFsError::io("reidentify runtime directory", source))?;
    if current_identity != expected.identity {
        return Err(CheckedFsError::ambiguous(
            label,
            "directory identity changed",
        ));
    }
    Ok(())
}

pub(super) fn revalidate_workspace_repository_in(
    context: &OperationContext,
    workspace_root: &Path,
    workspace_git_dir: &Path,
) -> Result<(), CheckedFsError> {
    let filesystem = context.filesystem();
    let repository = context
        .repository()
        .repository_paths(workspace_root)
        .map_err(|error| {
            CheckedFsError::io(
                "reopen workspace Git repository",
                io::Error::other(error.to_string()),
            )
        })?;
    let workdir = repository.worktree.ok_or_else(|| {
        CheckedFsError::ambiguous("workspace root", "bare Git repositories are not workspaces")
    })?;
    let observed_workdir = filesystem
        .canonical_path(&workdir)
        .map_err(|source| CheckedFsError::io("recanonicalize Git worktree", source))?;
    let observed_git_dir = filesystem
        .canonical_path(&repository.git_dir)
        .map_err(|source| CheckedFsError::io("recanonicalize workspace Git directory", source))?;
    if observed_workdir != workspace_root || observed_git_dir != workspace_git_dir {
        return Err(CheckedFsError::ambiguous(
            "workspace Git relationship",
            "worktree or Git-directory binding changed",
        ));
    }
    Ok(())
}

pub(super) fn ensure_child_directory(
    parent: &FsDirectory,
    name: &OsStr,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    let filesystem = parent.filesystem();
    match filesystem.open_directory_at(parent, name) {
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            if let Err(source) = filesystem.create_directory_at(parent, name)
                && source.kind() != io::ErrorKind::AlreadyExists
            {
                return Err(CheckedFsError::io("create runtime directory", source));
            }
        }
        Err(source)
            if matches!(
                source.kind(),
                io::ErrorKind::NotADirectory | io::ErrorKind::InvalidInput
            ) =>
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "expected a no-follow directory",
            ));
        }
        Err(source) => return Err(CheckedFsError::io("observe runtime directory", source)),
    }
    open_child_directory(parent, name, label)
}

pub(super) fn revalidate_child_directory(
    parent: &FsDirectory,
    name: &OsStr,
    expected: FsIdentity,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let current = open_child_directory(parent, name, label)?;
    if current.identity != expected {
        return Err(CheckedFsError::ambiguous(
            label,
            "directory identity changed",
        ));
    }
    Ok(())
}

pub(super) fn open_child_directory(
    parent: &FsDirectory,
    name: &OsStr,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    let filesystem = parent.filesystem();
    let dir = filesystem
        .open_directory_at(parent, name)
        .map_err(|source| match source.kind() {
            io::ErrorKind::NotADirectory | io::ErrorKind::InvalidInput => {
                CheckedFsError::ambiguous(label, "expected a no-follow directory")
            }
            _ => CheckedFsError::io("open runtime directory no-follow", source),
        })?;
    if !filesystem
        .directory_entry_matches(parent, name, &dir)
        .map_err(|source| CheckedFsError::io("reobserve runtime directory", source))?
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "directory changed while opening",
        ));
    }
    let identity = filesystem
        .directory_identity(&dir)
        .map_err(|source| CheckedFsError::io("identify runtime directory", source))?;
    Ok(RetainedDirectory { dir, identity })
}

pub(super) fn open_or_create_file(
    parent: &FsDirectory,
    name: &OsStr,
    label: &'static str,
) -> Result<FsFile, CheckedFsError> {
    const MAX_WINNER_REOPENS: usize = 16;
    let filesystem = parent.filesystem();
    for attempt in 0..MAX_WINNER_REOPENS {
        match filesystem.open_lock_file_at(parent, name) {
            Ok(file) => {
                revalidate_file(parent, name, &file, label)?;
                return Ok(file);
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::IsADirectory | io::ErrorKind::InvalidInput
                ) =>
            {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "expected a no-follow regular file",
                ));
            }
            Err(source) => return Err(CheckedFsError::io("observe runtime file", source)),
        }
        match filesystem.create_file_at(parent, name) {
            Ok(file) => {
                revalidate_file(parent, name, &file, label)?;
                return Ok(file);
            }
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                ) && attempt + 1 < MAX_WINNER_REOPENS =>
            {
                std::thread::yield_now();
            }
            Err(source) => {
                return Err(CheckedFsError::io(
                    "open or create runtime file no-follow",
                    source,
                ));
            }
        }
    }
    unreachable!("bounded runtime file reopen loop returns on its final attempt")
}

pub(super) fn open_existing_file(
    parent: &FsDirectory,
    name: &OsStr,
    label: &'static str,
) -> Result<FsFile, CheckedFsError> {
    let filesystem = parent.filesystem();
    let file = filesystem
        .open_lock_file_at(parent, name)
        .map_err(|source| {
            if matches!(
                source.kind(),
                io::ErrorKind::IsADirectory | io::ErrorKind::InvalidInput
            ) {
                CheckedFsError::ambiguous(label, "expected an existing no-follow regular file")
            } else {
                CheckedFsError::io("open existing runtime file no-follow", source)
            }
        })?;
    revalidate_file(parent, name, &file, label)?;
    Ok(file)
}

pub(super) fn revalidate_file(
    parent: &FsDirectory,
    name: &OsStr,
    file: &FsFile,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let filesystem = parent.filesystem();
    if !filesystem
        .file_entry_matches(parent, name, file)
        .map_err(|source| CheckedFsError::io("observe runtime file", source))?
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "file changed while opening or locking",
        ));
    }
    Ok(())
}

fn reject_non_directory_or_symlink(
    filesystem: &dyn FileSystem,
    path: &Path,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let metadata = filesystem
        .metadata(path)
        .map_err(|source| CheckedFsError::io("observe ambient runtime directory", source))?;
    if metadata.kind != FsKind::Directory {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn retain_ambient_directory(
    path: &Path,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    retain_ambient_directory_in(OperationContext::existing().filesystem(), path, label)
}
