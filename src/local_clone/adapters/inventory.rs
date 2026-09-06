//! The included-repository inventory and the source snapshot behind
//! `InstallPorts::{snapshot_source, recheck_source}` and disposal's fresh
//! evidence.
//!
//! Design §4.0: "Inventory every included Git repository before copying:
//! workspace root, materialized members, and unmanaged/ignored nested
//! repositories." `gwz-repo-inspect` answers for one repository path; the
//! traversal that finds them is core's, and it is the same traversal the
//! copier makes -- excluded entries are not included, symbolic links are
//! never followed, and a repository's own `.git` is never descended into.
//! Every repository found is inspected; the hazards of all of them are
//! aggregated into one refusal (design §4 step 1).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Exclusion;
use gwz_repo_contract::{HeadState, LayoutError, ObjectId, RepoInspector, RepoKey, RepositoryInfo};
use gwz_workspace_install::{CapturedRepository, InstallPortError, SourceSnapshot};

use crate::artifact::{self, LOCK_PATH};
use crate::workspace::WORKSPACE_MANIFEST;

/// One repository of a workspace tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludedRepository {
    /// The workspace root, a manifest member by id, or an unmanaged nested
    /// repository keyed `nested:<relative path>` -- it is copied and
    /// inspected like the others but is never paired across the family.
    pub key: RepoKey,
    /// Workspace-relative; empty for the root.
    pub relative: PathBuf,
    /// The repository path (its `.git` entry's parent, or the Git directory
    /// itself when `bare`).
    pub path: PathBuf,
    /// The directory *is* a Git directory (no `.git` entry): a bare
    /// repository, or a nested `.git` moved out of its worktree. Found by
    /// Git's own test -- a `HEAD` file beside `objects/` and `refs/` -- and,
    /// when nested, never descended into (design §4.0 "unmanaged/ignored
    /// nested repositories"; LCM2.1, closing §13.8's recorded gap).
    pub bare: bool,
}

impl IncludedRepository {
    /// The Git directory, workspace-relative: the `.git` entry, or the
    /// directory itself for a bare repository.
    pub fn relative_git_dir(&self) -> PathBuf {
        if self.bare {
            self.relative.clone()
        } else {
            self.relative.join(".git")
        }
    }

    /// `@root`, the member id, or `nested:<path>`.
    pub fn label(&self) -> String {
        self.key.to_string()
    }
}

/// Every repository under `workspace`: the root first (when it is one),
/// then members and nested repositories in path order. `exclusions` are
/// the copy-time exclusions, so an excluded subtree is not inventoried.
pub fn included_repositories(
    workspace: &Path,
    exclusions: &[Exclusion],
) -> Result<Vec<IncludedRepository>, String> {
    let members: BTreeMap<PathBuf, String> = match artifact::read_manifest(workspace) {
        Ok(manifest) => manifest
            .members
            .iter()
            .map(|member| (PathBuf::from(&member.path), member.id.clone()))
            .collect(),
        // No manifest, or one this build cannot read: nothing is keyed as a
        // member, and every repository found is inventoried as nested. The
        // snapshot reads the manifest separately and refuses there.
        Err(_) => BTreeMap::new(),
    };
    let mut found: Vec<IncludedRepository> = Vec::new();
    let mut pending: Vec<PathBuf> = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let directory = workspace.join(&relative);
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        let mut names: Vec<std::ffi::OsString> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
            names.push(entry.file_name());
        }
        names.sort();
        let mut is_repository = false;
        let mut children: Vec<PathBuf> = Vec::new();
        for name in names {
            let child = relative.join(&name);
            if exclusions.iter().any(|exclusion| exclusion.matches(&child)) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(workspace.join(&child))
                .map_err(|error| format!("{}: {error}", workspace.join(&child).display()))?;
            if name == ".git" {
                // A directory or a gitfile: either way this directory is a
                // repository; the inspector decides whether it is copyable.
                is_repository = true;
                continue;
            }
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            children.push(child);
        }
        // A directory that is itself a Git directory is a repository too,
        // and its contents (objects, refs) are not a tree to walk.
        let bare = !is_repository && is_git_directory(&directory);
        let at_root = relative.as_os_str().is_empty();
        if is_repository || bare {
            let key = if at_root {
                RepoKey::Root
            } else if let Some(id) = members.get(&relative) {
                RepoKey::Member { id: id.clone() }
            } else {
                RepoKey::Member {
                    id: format!("nested:{}", relative.display()),
                }
            };
            found.push(IncludedRepository {
                key,
                path: workspace.join(&relative),
                relative,
                bare,
            });
        }
        // A nested bare repository's contents are objects and refs, not a
        // tree to walk; the root is walked whatever it is, because a bare
        // hub root (design §4.3) still holds its members beneath it.
        if bare && !at_root {
            continue;
        }
        // Depth-first in sorted order: push in reverse so the first child
        // is visited first.
        pending.extend(children.into_iter().rev());
    }
    found.sort_by(|left, right| {
        (left.key != RepoKey::Root)
            .cmp(&(right.key != RepoKey::Root))
            .then_with(|| left.relative.cmp(&right.relative))
    });
    Ok(found)
}

/// Git's own test for a Git directory (`is_git_directory` in `setup.c`): a
/// `HEAD` file beside an `objects` directory and a `refs` directory. Only
/// real entries count -- a symbolic link is never followed.
fn is_git_directory(directory: &Path) -> bool {
    let real = |name: &str, want_dir: bool| {
        std::fs::symlink_metadata(directory.join(name)).is_ok_and(|metadata| {
            !metadata.file_type().is_symlink() && metadata.is_dir() == want_dir
        })
    };
    real("HEAD", false) && real("objects", true) && real("refs", true)
}

/// Capture the source (design §4 step 2): inspect every included
/// repository, aggregate their hazards, and freeze HEADs, branches and
/// remotes plus the manifest/lock digest and the open-merge observation.
pub fn snapshot(
    inspector: &dyn RepoInspector,
    workspace: &Path,
    repositories: &[IncludedRepository],
    open_gwz_merge: Option<String>,
) -> Result<SourceSnapshot, InstallPortError> {
    let mut hazards = Vec::new();
    let mut captured = Vec::new();
    for repository in repositories {
        match inspector.inspect_layout(&repository.path) {
            Ok(info) => {
                let (branches, remotes) = branches_and_remotes(&info).map_err(|detail| {
                    InstallPortError::Layout(LayoutError::ReadFailed {
                        path: repository.path.clone(),
                        detail,
                    })
                })?;
                captured.push(CapturedRepository {
                    key: repository.key.clone(),
                    head: head_of(&info),
                    info,
                    branches,
                    remotes,
                });
            }
            Err(LayoutError::Unsupported { hazards: found, .. }) => hazards.extend(found),
            Err(error) => return Err(InstallPortError::Layout(error)),
        }
    }
    if !hazards.is_empty() {
        return Err(InstallPortError::Layout(LayoutError::Unsupported {
            path: workspace.to_path_buf(),
            hazards,
        }));
    }
    Ok(SourceSnapshot {
        repositories: captured,
        configuration_digest: configuration_digest(workspace)?,
        open_gwz_merge,
    })
}

fn head_of(info: &RepositoryInfo) -> Option<ObjectId> {
    match &info.head {
        HeadState::Attached { target, .. } | HeadState::Detached { target } => Some(target.clone()),
        HeadState::Unborn { .. } => None,
    }
}

fn branches_and_remotes(info: &RepositoryInfo) -> Result<(Vec<String>, Vec<String>), String> {
    let repository = git2::Repository::open_ext(
        &info.git_dir,
        git2::RepositoryOpenFlags::NO_SEARCH | git2::RepositoryOpenFlags::NO_DOTGIT,
        std::iter::empty::<&std::ffi::OsStr>(),
    )
    .map_err(|error| error.message().to_owned())?;
    let mut branches = Vec::new();
    for branch in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|error| error.message().to_owned())?
    {
        let (branch, _) = branch.map_err(|error| error.message().to_owned())?;
        if let Some(name) = branch.name().map_err(|error| error.message().to_owned())? {
            branches.push(name.to_owned());
        }
    }
    branches.sort();
    let mut remotes: Vec<String> = Vec::new();
    for remote in repository
        .remotes()
        .map_err(|error| error.message().to_owned())?
        .iter()
    {
        if let Some(name) = remote.map_err(|error| error.message().to_owned())? {
            remotes.push(name.to_owned());
        }
    }
    remotes.sort();
    Ok((branches, remotes))
}

/// SHA-256 over the manifest and lock bytes as they sit; the lock may be
/// absent (a workspace with nothing materialised yet).
fn configuration_digest(workspace: &Path) -> Result<String, InstallPortError> {
    let manifest = std::fs::read(workspace.join(WORKSPACE_MANIFEST)).map_err(|error| {
        InstallPortError::Configuration {
            detail: format!("{}: {error}", workspace.join(WORKSPACE_MANIFEST).display()),
        }
    })?;
    let lock = match std::fs::read(workspace.join(LOCK_PATH)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(InstallPortError::Configuration {
                detail: format!("{}: {error}", workspace.join(LOCK_PATH).display()),
            });
        }
    };
    let mut bytes = manifest;
    bytes.push(0);
    bytes.extend(lock);
    Ok(format!("sha256:{}", artifact::sha256_hex(&bytes)))
}

/// Design §4 step 3, "recheck the source observations": the fresh snapshot
/// must equal the frozen one; the first difference is the drift detail.
pub fn recheck(frozen: &SourceSnapshot, fresh: &SourceSnapshot) -> Result<(), InstallPortError> {
    if frozen == fresh {
        return Ok(());
    }
    let detail = if frozen.configuration_digest != fresh.configuration_digest {
        "the source manifest or lock changed".to_owned()
    } else if frozen.open_gwz_merge != fresh.open_gwz_merge {
        "the source's open gwz merge state changed".to_owned()
    } else {
        let mut detail = None;
        for repository in &frozen.repositories {
            match fresh
                .repositories
                .iter()
                .find(|fresh| fresh.key == repository.key)
            {
                None => {
                    detail = Some(format!("repository {} is gone", repository.key));
                    break;
                }
                Some(fresh) if fresh != repository => {
                    detail = Some(format!("repository {} changed", repository.key));
                    break;
                }
                Some(_) => {}
            }
        }
        detail.unwrap_or_else(|| {
            fresh
                .repositories
                .iter()
                .find(|fresh| !frozen.repositories.iter().any(|repo| repo.key == fresh.key))
                .map_or_else(
                    || "the source changed".to_owned(),
                    |added| format!("repository {} appeared", added.key),
                )
        })
    };
    Err(InstallPortError::Drift { detail })
}
