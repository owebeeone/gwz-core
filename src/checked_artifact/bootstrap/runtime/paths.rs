use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, File, OpenOptions};

use super::super::super::capability::CheckedFsError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InvocationIdentity {
    device: u64,
    inode: u64,
}

pub(super) struct RetainedDirectory {
    dir: Dir,
    identity: InvocationIdentity,
}

impl RetainedDirectory {
    pub(super) fn handle(&self) -> &Dir {
        &self.dir
    }

    pub(super) fn identity(&self) -> InvocationIdentity {
        self.identity
    }
}

pub(super) struct ResolvedWorkspacePaths {
    pub(super) workspace_root: PathBuf,
    pub(super) workspace_git_dir: PathBuf,
}

pub(super) fn resolve_workspace_paths(
    root: &Path,
) -> Result<ResolvedWorkspacePaths, CheckedFsError> {
    reject_non_directory_or_symlink(root, "workspace root")?;
    let workspace_root = std::fs::canonicalize(root)
        .map_err(|source| CheckedFsError::io("canonicalize workspace root", source))?;
    let repository = git2::Repository::open(&workspace_root).map_err(|error| {
        CheckedFsError::io(
            "open workspace Git repository",
            io::Error::other(error.message().to_owned()),
        )
    })?;
    let workdir = repository.workdir().ok_or_else(|| {
        CheckedFsError::ambiguous("workspace root", "bare Git repositories are not workspaces")
    })?;
    let observed_workdir = std::fs::canonicalize(workdir)
        .map_err(|source| CheckedFsError::io("canonicalize Git worktree", source))?;
    if observed_workdir != workspace_root {
        return Err(CheckedFsError::ambiguous(
            "workspace root",
            "path is not the Git worktree root",
        ));
    }
    let workspace_git_dir = std::fs::canonicalize(repository.path())
        .map_err(|source| CheckedFsError::io("canonicalize workspace Git directory", source))?;
    reject_non_directory_or_symlink(&workspace_git_dir, "workspace Git directory")?;

    Ok(ResolvedWorkspacePaths {
        workspace_root,
        workspace_git_dir,
    })
}

pub(super) fn retain_ambient_directory(
    path: &Path,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    reject_non_directory_or_symlink(path, label)?;
    let dir = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|source| CheckedFsError::io("open retained runtime directory", source))?;
    let identity = identity(
        &dir.dir_metadata()
            .map_err(|source| CheckedFsError::io("identify retained runtime directory", source))?,
    );
    let retained = RetainedDirectory { dir, identity };
    revalidate_ambient_directory(path, &retained, label)?;
    Ok(retained)
}

pub(super) fn revalidate_ambient_directory(
    path: &Path,
    expected: &RetainedDirectory,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    reject_non_directory_or_symlink(path, label)?;
    let current = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|source| CheckedFsError::io("reopen retained runtime directory", source))?;
    let current_identity = identity(
        &current
            .dir_metadata()
            .map_err(|source| CheckedFsError::io("reidentify runtime directory", source))?,
    );
    if current_identity != expected.identity {
        return Err(CheckedFsError::ambiguous(
            label,
            "directory identity changed",
        ));
    }
    Ok(())
}

pub(super) fn revalidate_workspace_repository(
    workspace_root: &Path,
    workspace_git_dir: &Path,
) -> Result<(), CheckedFsError> {
    let repository = git2::Repository::open(workspace_root).map_err(|error| {
        CheckedFsError::io(
            "reopen workspace Git repository",
            io::Error::other(error.message().to_owned()),
        )
    })?;
    let workdir = repository.workdir().ok_or_else(|| {
        CheckedFsError::ambiguous("workspace root", "bare Git repositories are not workspaces")
    })?;
    let observed_workdir = std::fs::canonicalize(workdir)
        .map_err(|source| CheckedFsError::io("recanonicalize Git worktree", source))?;
    let observed_git_dir = std::fs::canonicalize(repository.path())
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
    parent: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    match parent.symlink_metadata(name) {
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            if let Err(source) = parent.create_dir(name)
                && source.kind() != io::ErrorKind::AlreadyExists
            {
                return Err(CheckedFsError::io("create runtime directory", source));
            }
        }
        Err(source) => return Err(CheckedFsError::io("observe runtime directory", source)),
    }
    open_child_directory(parent, name, label)
}

pub(super) fn revalidate_child_directory(
    parent: &Dir,
    name: &OsStr,
    expected: InvocationIdentity,
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
    parent: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    let observed = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe runtime directory", source))?;
    if !observed.is_dir() || observed.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    let dir = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("open runtime directory no-follow", source))?;
    let opened = dir
        .dir_metadata()
        .map_err(|source| CheckedFsError::io("identify runtime directory", source))?;
    let after = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("reobserve runtime directory", source))?;
    if !after.is_dir()
        || after.is_symlink()
        || identity(&observed) != identity(&opened)
        || identity(&after) != identity(&opened)
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "directory changed while opening",
        ));
    }
    Ok(RetainedDirectory {
        dir,
        identity: identity(&opened),
    })
}

pub(super) fn open_or_create_file(
    parent: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<File, CheckedFsError> {
    const MAX_WINNER_REOPENS: usize = 16;

    for attempt in 0..MAX_WINNER_REOPENS {
        match parent.symlink_metadata(name) {
            Ok(observed) if !observed.is_file() || observed.is_symlink() => {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "expected a no-follow regular file",
                ));
            }
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(CheckedFsError::io("observe runtime file", source)),
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .follow(FollowSymlinks::No);
        match parent.open_with(name, &options) {
            Ok(file) => {
                revalidate_file(parent, name, &file, label)?;
                return Ok(file);
            }
            Err(source)
                if source.kind() == io::ErrorKind::NotFound && attempt + 1 < MAX_WINNER_REOPENS =>
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
    parent: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<File, CheckedFsError> {
    let observed = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe existing runtime file", source))?;
    if !observed.is_file() || observed.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected an existing no-follow regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    let file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open existing runtime file no-follow", source))?;
    revalidate_file(parent, name, &file, label)?;
    Ok(file)
}

pub(super) fn revalidate_file(
    parent: &Dir,
    name: &OsStr,
    file: &File,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let observed = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe runtime file", source))?;
    let opened = file
        .metadata()
        .map_err(|source| CheckedFsError::io("identify runtime file", source))?;
    if !observed.is_file()
        || observed.is_symlink()
        || !opened.is_file()
        || opened.is_symlink()
        || identity(&observed) != identity(&opened)
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "file changed while opening or locking",
        ));
    }
    Ok(())
}

fn reject_non_directory_or_symlink(path: &Path, label: &'static str) -> Result<(), CheckedFsError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| CheckedFsError::io("observe ambient runtime directory", source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    Ok(())
}

fn identity(metadata: &impl MetadataExt) -> InvocationIdentity {
    InvocationIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}
