use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use cap_fs_ext::OsMetadataExt;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, OpenOptions};

use crate::model::{ErrorCode, ModelError, ModelResult};

mod fault;
mod platform;

pub(crate) use fault::CheckedArtifactFault;
use fault::fault;
#[cfg(test)]
pub(crate) use fault::{fail_next_checked_artifact_at, run_next_checked_artifact_at};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CheckedArtifactFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

enum ParentState {
    Missing,
    Invalid,
    Open { dir: Dir, identity: (u64, u64) },
}

/// A no-follow capability for one workspace-relative regular-file artifact.
///
/// Acquisition never creates a parent. Mutations remain bound to the retained
/// parent and reobserve the exact expected leaf immediately before their
/// handle-relative linearization point.
pub(crate) struct CheckedArtifact {
    root: Dir,
    parent_relative: PathBuf,
    parent: ParentState,
    leaf: OsString,
    code: ErrorCode,
    label: String,
}

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
                    platform::sync_parent(&current)
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
        let (parent_relative, leaf) =
            split_relative(relative).map_err(|detail| error(code, &label, detail))?;
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
            parent_relative,
            parent,
            leaf,
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

    pub(crate) fn replace_exact(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        require_source(expected, self.code, &self.label)?;
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        let (temporary, mut file) = self.create_temp(dir, "tmp")?;
        let result = (|| {
            file.write_all(goal)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            file.sync_all()
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            drop(file);
            fault(
                CheckedArtifactFault::BeforeFinalCheck,
                self.code,
                &self.label,
            )?;
            if !self.parent_is_current(*identity)? || self.observe()? != *expected {
                return Err(error(
                    self.code,
                    &self.label,
                    "source changed before checked replacement",
                ));
            }
            platform::rename_relative(
                dir,
                &temporary,
                &self.leaf,
                !matches!(expected, CheckedArtifactFact::Missing),
                self.code,
                &self.label,
            )?;
            fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
            self.sync_parent(dir)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = dir.remove_file(&temporary);
        }
        result
    }

    pub(crate) fn remove_exact(&self, expected: &CheckedArtifactFact) -> ModelResult<()> {
        if !matches!(expected, CheckedArtifactFact::Bytes(_)) {
            return Err(error(
                self.code,
                &self.label,
                "checked removal requires exact existing source bytes",
            ));
        }
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        fault(
            CheckedArtifactFault::BeforeFinalCheck,
            self.code,
            &self.label,
        )?;
        if !self.parent_is_current(*identity)? || self.observe()? != *expected {
            return Err(error(
                self.code,
                &self.label,
                "source changed before checked removal",
            ));
        }
        let tombstone = self.unique_name(dir, "removed")?;
        platform::remove_relative(dir, &self.leaf, &tombstone, self.code, &self.label)?;
        fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
        self.sync_parent(dir)
    }

    fn parent_is_current(&self, expected: (u64, u64)) -> ModelResult<bool> {
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

    fn create_temp(&self, dir: &Dir, suffix: &str) -> ModelResult<(OsString, cap_std::fs::File)> {
        let stem = self.leaf.to_string_lossy();
        let mut last_error = None;
        for _ in 0..64 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = OsString::from(format!(
                ".{stem}.gwz-{}-{sequence}.{suffix}",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            #[cfg(unix)]
            cap_std::fs::OpenOptionsExt::mode(&mut options, 0o644);
            match dir.open_with(&name, &options) {
                Ok(file) => return Ok((name, file)),
                Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(cause);
                }
                Err(cause) => return Err(io_error(self.code, &self.label, cause)),
            }
        }
        Err(io_error(
            self.code,
            &self.label,
            last_error.unwrap_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "checked-artifact temporary name collision",
                )
            }),
        ))
    }

    fn unique_name(&self, dir: &Dir, suffix: &str) -> ModelResult<OsString> {
        let stem = self.leaf.to_string_lossy();
        for _ in 0..64 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = OsString::from(format!(
                ".{stem}.gwz-{}-{sequence}.{suffix}",
                std::process::id()
            ));
            match dir.symlink_metadata(&name) {
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(name),
                Ok(_) => {}
                Err(cause) => return Err(io_error(self.code, &self.label, cause)),
            }
        }
        Err(error(
            self.code,
            &self.label,
            "checked-artifact temporary name collision",
        ))
    }

    fn sync_parent(&self, dir: &Dir) -> ModelResult<()> {
        fault(
            CheckedArtifactFault::BeforeDurability,
            self.code,
            &self.label,
        )?;
        platform::sync_parent(dir).map_err(|cause| io_error(self.code, &self.label, cause))?;
        fault(
            CheckedArtifactFault::AfterDurability,
            self.code,
            &self.label,
        )
    }
}

fn require_source(expected: &CheckedArtifactFact, code: ErrorCode, label: &str) -> ModelResult<()> {
    if matches!(
        expected,
        CheckedArtifactFact::Missing | CheckedArtifactFact::Bytes(_)
    ) {
        Ok(())
    } else {
        Err(error(
            code,
            label,
            "invalid source cannot authorize mutation",
        ))
    }
}

fn observe_leaf(
    dir: &Dir,
    leaf: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<CheckedArtifactFact> {
    let metadata = match dir.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CheckedArtifactFact::Missing);
        }
        Err(cause) => return Err(io_error(code, label, cause)),
    };
    if !metadata.is_file() || metadata.is_symlink() || executable(&metadata) {
        return Ok(CheckedArtifactFact::Invalid);
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
            return Ok(CheckedArtifactFact::Invalid);
        }
        Err(cause) => return Err(io_error(code, label, cause)),
    };
    let opened = file
        .metadata()
        .map_err(|cause| io_error(code, label, cause))?;
    if identity(&opened) != identity(&metadata) {
        return Ok(CheckedArtifactFact::Invalid);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|cause| io_error(code, label, cause))?;
    let after = match dir.symlink_metadata(leaf) {
        Ok(after) => after,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CheckedArtifactFact::Invalid);
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
        return Ok(CheckedArtifactFact::Invalid);
    }
    Ok(CheckedArtifactFact::Bytes(bytes))
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

fn identity(metadata: &cap_fs_ext::Metadata) -> (u64, u64) {
    (MetadataExt::dev(metadata), MetadataExt::ino(metadata))
}

#[cfg(unix)]
fn executable(metadata: &cap_fs_ext::Metadata) -> bool {
    OsMetadataExt::mode(metadata) & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &cap_fs_ext::Metadata) -> bool {
    false
}

fn io_error(code: ErrorCode, label: &str, cause: std::io::Error) -> ModelError {
    error(code, label, cause)
}

fn error(code: ErrorCode, label: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(code, format!("checked {label}: {detail}"))
}

#[cfg(test)]
#[path = "checked_artifact/tests.rs"]
mod tests;
