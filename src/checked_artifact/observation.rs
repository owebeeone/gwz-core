use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use cap_fs_ext::OsMetadataExt;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, OpenOptions};

use super::{CheckedArtifact, CheckedArtifactFact, ParentState, error, io_error};
use crate::model::{ErrorCode, ModelResult};

impl CheckedArtifact {
    /// Prepare a canonical no-follow directory hierarchy before an operation
    /// can persist an action that depends on it.
    pub(crate) fn prepare_parent(
        root: &Path,
        relative: &Path,
        code: ErrorCode,
        label: impl Into<String>,
    ) -> ModelResult<()> {
        let label = label.into();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(error(code, &label, "parent path is noncanonical"));
        }
        let mut current = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|cause| io_error(code, &label, cause))?;
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(error(code, &label, "parent path is noncanonical"));
            };
            let metadata = match current.symlink_metadata(component) {
                Ok(metadata) => metadata,
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                    current
                        .create_dir(component)
                        .map_err(|cause| io_error(code, &label, cause))?;
                    super::platform::sync_parent(&current)
                        .map_err(|cause| io_error(code, &label, cause))?;
                    current
                        .symlink_metadata(component)
                        .map_err(|cause| io_error(code, &label, cause))?
                }
                Err(cause) => return Err(io_error(code, &label, cause)),
            };
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(error(code, &label, "parent component is noncanonical"));
            }
            let next = current
                .open_dir_nofollow(component)
                .map_err(|cause| io_error(code, &label, cause))?;
            if identity(&metadata)
                != identity(
                    &next
                        .dir_metadata()
                        .map_err(|cause| io_error(code, &label, cause))?,
                )
            {
                return Err(error(
                    code,
                    &label,
                    "parent component changed while opening",
                ));
            }
            current = next;
        }
        Ok(())
    }

    pub(crate) fn acquire(
        root: &Path,
        relative: &Path,
        code: ErrorCode,
        label: impl Into<String>,
    ) -> ModelResult<Self> {
        let label = label.into();
        let relative = relative.to_path_buf();
        let (parent_relative, leaf) =
            split_relative(&relative).map_err(|detail| error(code, &label, detail))?;
        let (private_root, quarantine_parent) = git2::Repository::open(root).map_or_else(
            |_| (root.to_path_buf(), PathBuf::from(".gwz/checked-artifacts")),
            |repo| {
                (
                    repo.path().to_path_buf(),
                    PathBuf::from("gwz/checked-artifacts"),
                )
            },
        );
        let root = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|cause| io_error(code, &label, cause))?;
        let parent = match traverse(&root, &parent_relative)
            .map_err(|cause| io_error(code, &label, cause))?
        {
            Traversal::Missing => ParentState::Missing,
            Traversal::Invalid => ParentState::Invalid,
            Traversal::Open(dir) => {
                let identity = identity(
                    &dir.dir_metadata()
                        .map_err(|cause| io_error(code, &label, cause))?,
                );
                ParentState::Open { dir, identity }
            }
        };
        Ok(Self {
            root,
            relative,
            parent_relative,
            parent,
            leaf,
            private_root,
            quarantine_parent,
            code,
            label,
        })
    }

    pub(crate) fn observe(&self) -> ModelResult<CheckedArtifactFact> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return Ok(match self.parent {
                ParentState::Missing => CheckedArtifactFact::Missing,
                ParentState::Invalid => CheckedArtifactFact::Invalid,
                ParentState::Open { .. } => unreachable!(),
            });
        };
        if !self.parent_is_current(*identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        observe_leaf(dir, &self.leaf, self.code, &self.label)
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "v1 bundle lifecycle remains disabled until A1")
    )]
    pub(crate) fn parent_is_canonical(&self) -> ModelResult<bool> {
        match &self.parent {
            ParentState::Open { identity, .. } => self.parent_is_current(*identity),
            ParentState::Missing | ParentState::Invalid => Ok(false),
        }
    }

    pub(super) fn parent_is_current(&self, expected: (u64, u64)) -> ModelResult<bool> {
        let current = traverse(&self.root, &self.parent_relative)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        let Traversal::Open(current) = current else {
            return Ok(false);
        };
        Ok(identity(
            &current
                .dir_metadata()
                .map_err(|cause| io_error(self.code, &self.label, cause))?,
        ) == expected)
    }
}

pub(super) fn observe_leaf(
    dir: &Dir,
    leaf: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<CheckedArtifactFact> {
    Ok(observe_leaf_exact(dir, leaf, code, label)?.fact)
}

pub(super) struct LeafObservation {
    pub(super) fact: CheckedArtifactFact,
    pub(super) identity: Option<(u64, u64)>,
}

pub(super) fn observe_leaf_exact(
    dir: &Dir,
    leaf: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<LeafObservation> {
    let metadata = match dir.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Missing,
                identity: None,
            });
        }
        Err(cause) => return Err(io_error(code, label, cause)),
    };
    if !metadata.is_file() || metadata.is_symlink() || executable(&metadata) {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = match dir.open_with(leaf, &options) {
        Ok(file) => file,
        Err(cause)
            if matches!(
                cause.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Invalid,
                identity: None,
            });
        }
        Err(cause) => return Err(io_error(code, label, cause)),
    };
    let opened = file
        .metadata()
        .map_err(|cause| io_error(code, label, cause))?;
    if identity(&opened) != identity(&metadata) {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|cause| io_error(code, label, cause))?;
    let after = match dir.symlink_metadata(leaf) {
        Ok(after) => after,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Invalid,
                identity: None,
            });
        }
        Err(cause) => return Err(io_error(code, label, cause)),
    };
    if !after.is_file()
        || after.is_symlink()
        || executable(&after)
        || identity(&after) != identity(&metadata)
        || identity(&after) != identity(&opened)
        || opened.len() != bytes.len() as u64
    {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    Ok(LeafObservation {
        fact: CheckedArtifactFact::Bytes(bytes),
        identity: Some(identity(&opened)),
    })
}

pub(super) fn identity(metadata: &cap_fs_ext::Metadata) -> (u64, u64) {
    (MetadataExt::dev(metadata), MetadataExt::ino(metadata))
}

enum Traversal {
    Missing,
    Invalid,
    Open(Dir),
}

fn traverse(root: &Dir, relative: &Path) -> std::io::Result<Traversal> {
    let mut current = root.try_clone()?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Ok(Traversal::Invalid);
        };
        let metadata = match current.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Traversal::Missing);
            }
            Err(cause) => return Err(cause),
        };
        if !metadata.is_dir() || metadata.is_symlink() {
            return Ok(Traversal::Invalid);
        }
        let next = match current.open_dir_nofollow(component) {
            Ok(next) => next,
            Err(cause) if cause.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(cause);
            }
            Err(_) => return Ok(Traversal::Invalid),
        };
        if identity(&metadata) != identity(&next.dir_metadata()?) {
            return Ok(Traversal::Invalid);
        }
        current = next;
    }
    Ok(Traversal::Open(current))
}

fn split_relative(path: &Path) -> Result<(PathBuf, OsString), &'static str> {
    if path.is_absolute() {
        return Err("path is not workspace-relative");
    }
    let mut components = path.components().collect::<Vec<_>>();
    let leaf = match components.pop() {
        Some(Component::Normal(leaf)) => leaf.to_owned(),
        _ => return Err("leaf name is noncanonical"),
    };
    let mut parent = PathBuf::new();
    for component in components {
        let Component::Normal(component) = component else {
            return Err("parent path is noncanonical");
        };
        parent.push(component);
    }
    Ok((parent, leaf))
}

#[cfg(unix)]
fn executable(metadata: &cap_fs_ext::Metadata) -> bool {
    OsMetadataExt::mode(metadata) & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &cap_fs_ext::Metadata) -> bool {
    false
}
