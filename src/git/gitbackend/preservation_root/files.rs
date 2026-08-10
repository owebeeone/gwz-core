use super::super::*;
use super::{FaultBoundary, fault};

use cap_fs_ext::{
    DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OsMetadataExt, ambient_authority,
};
use cap_std::fs::{Dir, OpenOptions};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn observe_relative(
    root: &Path,
    expected: &Option<GitCandidateFile>,
    path: &str,
) -> ModelResult<bool> {
    observe_in_root(
        root,
        Path::new(path),
        expected.as_ref().map(|file| file.bytes.as_slice()),
    )
}

pub(super) fn observe_required(root: &Path, expected: &GitCandidateFile) -> ModelResult<bool> {
    observe_in_root(root, Path::new(&expected.path), Some(&expected.bytes))
}

pub(super) fn observe_boundary(root: &Path, expected: &[u8]) -> ModelResult<bool> {
    let repo = open_repo(root)?;
    observe_in_root(repo.path(), Path::new("info/exclude"), Some(expected))
}

pub(super) fn replace_relative(
    root: &Path,
    path: &str,
    source: Option<&GitCandidateFile>,
    goal: Option<&GitCandidateFile>,
) -> ModelResult<()> {
    let Some(parent) = AnchoredParent::open(root, Path::new(path))? else {
        return Err(evidence_error("managed leaf parent is missing or replaced"));
    };
    let expected = source.map(|file| file.bytes.as_slice());
    match goal {
        Some(file) => parent.replace(expected, &file.bytes),
        None => parent.remove(expected),
    }
}

fn observe_in_root(root: &Path, path: &Path, expected: Option<&[u8]>) -> ModelResult<bool> {
    let Some(parent) = AnchoredParent::open(root, path)? else {
        return Ok(expected.is_none());
    };
    if !parent.is_current()? {
        return Ok(false);
    }
    parent.observe(expected)
}

struct AnchoredParent {
    root: Dir,
    relative: PathBuf,
    dir: Dir,
    identity: (u64, u64),
    leaf: OsString,
}

impl AnchoredParent {
    fn open(root: &Path, path: &Path) -> ModelResult<Option<Self>> {
        let (parent_path, leaf) = split_relative(path)?;
        let root =
            Dir::open_ambient_dir(root, ambient_authority()).map_err(crate::git::io_error)?;
        let Some(dir) = traverse(&root, &parent_path)? else {
            return Ok(None);
        };
        let identity = identity(&dir.dir_metadata().map_err(crate::git::io_error)?);
        Ok(Some(Self {
            root,
            relative: parent_path,
            dir,
            identity,
            leaf,
        }))
    }

    fn is_current(&self) -> ModelResult<bool> {
        let Some(current) = traverse(&self.root, &self.relative)? else {
            return Ok(false);
        };
        Ok(identity(&current.dir_metadata().map_err(crate::git::io_error)?) == self.identity)
    }

    fn observe(&self, expected: Option<&[u8]>) -> ModelResult<bool> {
        let metadata = match self.dir.symlink_metadata(&self.leaf) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(expected.is_none());
            }
            Err(error) => return Err(crate::git::io_error(error)),
        };
        if !metadata.is_file() || metadata.is_symlink() || executable(&metadata) {
            return Ok(false);
        }
        let Some(expected) = expected else {
            return Ok(false);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = match self.dir.open_with(&self.leaf, &options) {
            Ok(file) => file,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(crate::git::io_error(error)),
        };
        if identity(&file.metadata().map_err(crate::git::io_error)?) != identity(&metadata) {
            return Ok(false);
        }
        let mut observed = Vec::new();
        file.read_to_end(&mut observed)
            .map_err(crate::git::io_error)?;
        Ok(observed == expected)
    }

    fn replace(&self, source: Option<&[u8]>, bytes: &[u8]) -> ModelResult<()> {
        let (temporary, mut file) = self.create_temp()?;
        let result = (|| {
            file.write_all(bytes).map_err(crate::git::io_error)?;
            file.sync_all().map_err(crate::git::io_error)?;
            drop(file);
            if !self.is_current()? || !self.observe(source)? {
                return Err(evidence_error(
                    "managed leaf changed before atomic replacement",
                ));
            }
            fault(FaultBoundary::BeforeLeafRename)?;
            self.dir
                .rename(&temporary, &self.dir, &self.leaf)
                .map_err(crate::git::io_error)?;
            self.sync()?;
            fault(FaultBoundary::AfterLeafRename)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = self.dir.remove_file(&temporary);
        }
        result
    }

    fn remove(&self, source: Option<&[u8]>) -> ModelResult<()> {
        if !self.is_current()? || !self.observe(source)? {
            return Err(evidence_error("managed leaf changed before atomic removal"));
        }
        fault(FaultBoundary::BeforeLeafUnlink)?;
        self.dir
            .remove_file(&self.leaf)
            .map_err(crate::git::io_error)?;
        self.sync()?;
        fault(FaultBoundary::AfterLeafUnlink)
    }

    fn create_temp(&self) -> ModelResult<(OsString, cap_std::fs::File)> {
        let stem = self.leaf.to_string_lossy();
        let mut last_error = None;
        for _ in 0..32 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = OsString::from(format!(".{stem}.gwz-{sequence}.tmp"));
            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            #[cfg(unix)]
            cap_std::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match self.dir.open_with(&name, &options) {
                Ok(file) => return Ok((name, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(error);
                }
                Err(error) => return Err(crate::git::io_error(error)),
            }
        }
        Err(crate::git::io_error(last_error.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "temporary name collision",
            )
        })))
    }

    fn sync(&self) -> ModelResult<()> {
        self.dir
            .try_clone()
            .and_then(|dir| dir.into_std_file().sync_all())
            .map_err(crate::git::io_error)
    }
}

pub(super) fn split_relative(path: &Path) -> ModelResult<(PathBuf, OsString)> {
    if path.is_absolute() {
        return Err(evidence_error("managed path is not relative"));
    }
    let mut components = path.components().collect::<Vec<_>>();
    let leaf = match components.pop() {
        Some(Component::Normal(leaf)) => leaf.to_owned(),
        _ => return Err(evidence_error("managed leaf name is noncanonical")),
    };
    let mut parent = PathBuf::new();
    for component in components {
        let Component::Normal(component) = component else {
            return Err(evidence_error("managed parent path is noncanonical"));
        };
        parent.push(component);
    }
    Ok((parent, leaf))
}

fn traverse(root: &Dir, relative: &Path) -> ModelResult<Option<Dir>> {
    let mut current = root.try_clone().map_err(crate::git::io_error)?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Ok(None);
        };
        let metadata = match current.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(crate::git::io_error(error)),
        };
        if !metadata.is_dir() || metadata.is_symlink() {
            return Ok(None);
        }
        let next = match current.open_dir_nofollow(component) {
            Ok(next) => next,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(crate::git::io_error(error));
            }
            Err(_) => return Ok(None),
        };
        if identity(&metadata) != identity(&next.dir_metadata().map_err(crate::git::io_error)?) {
            return Ok(None);
        }
        current = next;
    }
    Ok(Some(current))
}

pub(super) fn identity(metadata: &cap_fs_ext::Metadata) -> (u64, u64) {
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

fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}

#[cfg(unix)]
pub(in crate::git::gitbackend) fn raw_path_to_path(value: &[u8]) -> ModelResult<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(value.to_vec()).into())
}

#[cfg(not(unix))]
pub(in crate::git::gitbackend) fn raw_path_to_path(value: &[u8]) -> ModelResult<PathBuf> {
    String::from_utf8(value.to_vec())
        .map(Into::into)
        .map_err(|_| evidence_error("non-UTF-8 Git path is unsupported on this platform"))
}

#[cfg(unix)]
pub(in crate::git::gitbackend) fn path_to_raw(value: &Path) -> ModelResult<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Ok(value.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
pub(in crate::git::gitbackend) fn path_to_raw(value: &Path) -> ModelResult<Vec<u8>> {
    value
        .to_str()
        .map(|value| value.as_bytes().to_vec())
        .ok_or_else(|| evidence_error("non-UTF-8 path is unsupported on this platform"))
}
