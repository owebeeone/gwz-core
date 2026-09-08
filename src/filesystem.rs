//! Filesystem operations shared by production and test backends.
//!
//! Journal file operations and preservation observations use this boundary.
//! Retained-handle operations grow as the checked-artifact callers migrate.
#![deny(clippy::disallowed_types)]

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path};

mod factory;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod filesystem_contract_tests;
pub(crate) mod native;

pub(crate) use factory::make_filesystem;

#[cfg(test)]
pub(crate) fn remove_file_for_test(path: &Path) -> io::Result<()> {
    make_filesystem().remove_file(path)
}

#[cfg(test)]
pub(crate) fn write_atomic_for_test(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let filesystem = make_filesystem();
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    filesystem.create_directories(parent)?;
    let temporary = loop {
        let candidate = path.with_extension(format!(
            "gwz-memory-{}.tmp",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match filesystem.create_file(&candidate) {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    filesystem.write_all(&temporary.1, bytes)?;
    filesystem.sync_file(&temporary.1)?;
    drop(temporary.1);
    filesystem.rename(&temporary.0, path, RenameMode::Replace)?;
    filesystem.sync_directory(parent)
}

#[cfg(test)]
pub(crate) fn write_for_test(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let filesystem = make_filesystem();
    if let Some(parent) = path.parent() {
        filesystem.create_directories(parent)?;
    }
    let (parent, name) = split(path)?;
    let parent = filesystem.open_directory(parent)?;
    let file = match filesystem.open_file_for_write_at(&parent, name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            filesystem.create_file_at(&parent, name)?
        }
        Err(error) => return Err(error),
    };
    filesystem.set_len(&file, 0)?;
    filesystem.write_all(&file, bytes)
}

pub(crate) struct FsDirectory(DirectoryHandle);
pub(crate) struct FsFile(FileHandle);

impl FsDirectory {
    #[cfg(test)]
    pub(crate) fn is_memory(&self) -> bool {
        matches!(self.0, DirectoryHandle::Memory(_))
    }

    #[cfg(not(test))]
    pub(crate) const fn is_memory(&self) -> bool {
        false
    }
}

impl FsFile {
    #[cfg(test)]
    pub(crate) fn is_memory(&self) -> bool {
        matches!(self.0, FileHandle::Memory(_))
    }

    #[cfg(not(test))]
    pub(crate) const fn is_memory(&self) -> bool {
        false
    }
}
#[allow(dead_code, reason = "runtime-lock consumers are migrating")]
pub(crate) struct FsLockGuard {
    _handle: LockHandle,
}
enum DirectoryHandle {
    Native(native::Directory),
    #[cfg(test)]
    Memory(fake::Handle),
}
enum FileHandle {
    Native(native::File),
    #[cfg(test)]
    Memory(fake::Handle),
}
#[allow(dead_code, reason = "runtime-lock consumers are migrating")]
enum LockHandle {
    Native {
        _lock: native::Lock,
    },
    #[cfg(test)]
    Memory {
        _lock: fake::Lock,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code, reason = "retained identity consumers are migrating")]
pub(crate) struct FsIdentity {
    namespace: u64,
    object: u64,
}

impl FsIdentity {
    pub(crate) fn encode(self) -> [u8; 16] {
        let mut encoded = [0; 16];
        encoded[..8].copy_from_slice(&self.namespace.to_be_bytes());
        encoded[8..].copy_from_slice(&self.object.to_be_bytes());
        encoded
    }

    pub(crate) const fn namespace(self) -> u64 {
        self.namespace
    }
    pub(crate) const fn object(self) -> u64 {
        self.object
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FsKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FsMetadata {
    pub(crate) kind: FsKind,
    pub(crate) executable: bool,
}

#[allow(
    dead_code,
    reason = "directory consumers are migrating; fake Git already exercises this contract"
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FsDirectoryEntry {
    pub(crate) name: OsString,
    pub(crate) kind: FsKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenameMode {
    Replace,
    NoReplace,
}

pub(crate) trait FileSystem: Send + Sync {
    fn canonical_path(&self, path: &Path) -> io::Result<std::path::PathBuf>;
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata>;
    fn metadata_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsMetadata>;
    fn link_target(&self, path: &Path) -> io::Result<std::path::PathBuf>;
    fn kind(&self, path: &Path) -> io::Result<FsKind> {
        self.metadata(path).map(|value| value.kind)
    }
    fn remove_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()>;
    #[allow(
        dead_code,
        reason = "test fixtures are the first migrated directory-removal consumer"
    )]
    fn remove_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()>;
    fn create_file(&self, path: &Path) -> io::Result<FsFile> {
        let (parent, name) = split(path)?;
        self.create_file_at(&self.open_directory(parent)?, name)
    }
    #[allow(dead_code, reason = "runtime-lock consumers are migrating")]
    fn open_file(&self, path: &Path) -> io::Result<FsFile> {
        let (parent, name) = split(path)?;
        self.open_file_at(&self.open_directory(parent)?, name)
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let (parent, name) = split(path)?;
        self.read_all(&self.open_file_at(&self.open_directory(parent)?, name)?)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let (parent, name) = split(path)?;
        self.remove_file_at(&self.open_directory(parent)?, name)
    }
    #[allow(
        dead_code,
        reason = "test fixtures are the first migrated directory-removal consumer"
    )]
    fn remove_directory(&self, path: &Path) -> io::Result<()> {
        let (parent, name) = split(path)?;
        self.remove_directory_at(&self.open_directory(parent)?, name)
    }
    fn create_directories(&self, path: &Path) -> io::Result<()> {
        match self.open_directory(path) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let (parent, name) = split(path)?;
        self.create_directories(parent)?;
        match self.create_directory_at(&self.open_directory(parent)?, name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.open_directory(path).map(|_| ())
            }
            Err(error) => Err(error),
        }
    }
    fn open_directory(&self, path: &Path) -> io::Result<FsDirectory>;
    fn clone_directory(&self, directory: &FsDirectory) -> io::Result<FsDirectory>;
    #[allow(
        dead_code,
        reason = "retained-directory consumers are migrating; shared contract covers this operation"
    )]
    fn open_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsDirectory>;
    fn create_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()>;
    fn open_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>;
    fn open_lock_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>;
    fn open_file_for_write_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>;
    fn create_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>;
    fn read_at(&self, file: &FsFile, offset: u64, bytes: &mut [u8]) -> io::Result<usize>;
    fn file_len(&self, file: &FsFile) -> io::Result<u64>;
    fn write_at(&self, file: &FsFile, offset: u64, bytes: &[u8]) -> io::Result<usize>;
    #[allow(dead_code, reason = "test fixtures are the first truncation consumer")]
    fn set_len(&self, file: &FsFile, length: u64) -> io::Result<()>;
    fn sync_file(&self, file: &FsFile) -> io::Result<()>;
    #[allow(dead_code, reason = "retained identity consumers are migrating")]
    fn directory_identity(&self, directory: &FsDirectory) -> io::Result<FsIdentity>;
    #[allow(dead_code, reason = "retained identity consumers are migrating")]
    fn file_identity(&self, file: &FsFile) -> io::Result<FsIdentity>;
    fn rename(&self, source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()>;
    fn sync_directory(&self, path: &Path) -> io::Result<()>;
    fn sync_directory_at(&self, directory: &FsDirectory) -> io::Result<()>;
    #[allow(
        dead_code,
        reason = "directory consumers are migrating; fake Git already exercises this contract"
    )]
    fn read_directory(&self, path: &Path) -> io::Result<Vec<FsDirectoryEntry>>;
    fn read_directory_at(&self, directory: &FsDirectory) -> io::Result<Vec<FsDirectoryEntry>>;
    fn directory_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsDirectory,
    ) -> io::Result<bool>;
    fn file_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsFile,
    ) -> io::Result<bool>;
    #[cfg(test)]
    fn test_create_symlink_at(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        target: &Path,
    ) -> io::Result<()>;
    #[allow(dead_code, reason = "runtime-lock consumers are migrating")]
    fn try_lock_file(&self, file: &FsFile) -> io::Result<Option<FsLockGuard>>;
    #[allow(
        dead_code,
        reason = "retained publication consumers are migrating; shared contract covers this operation"
    )]
    fn rename_at(
        &self,
        source: &FsDirectory,
        name: &OsStr,
        destination: &FsDirectory,
        target: &OsStr,
        mode: RenameMode,
    ) -> io::Result<()>;

    fn read_all(&self, file: &FsFile) -> io::Result<Vec<u8>> {
        let mut result = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            match self.read_at(file, result.len() as u64, &mut chunk) {
                Ok(0) => return Ok(result),
                Ok(count) => result.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
    fn write_all(&self, file: &FsFile, bytes: &[u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            match self.write_at(file, offset as u64, &bytes[offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => offset += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn test_workspace(&self) -> io::Result<TestFsWorkspace>;
}

fn split(path: &Path) -> io::Result<(&Path, &OsStr)> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    component(name)?;
    Ok((
        if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        },
        name,
    ))
}

fn component(name: &OsStr) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if matches!(parts.next(), Some(Component::Normal(part)) if part == name)
        && parts.next().is_none()
        && !name.as_encoded_bytes().contains(&0)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one relative filesystem component",
        ))
    }
}

#[cfg(test)]
pub(crate) struct TestFsWorkspace {
    path: std::path::PathBuf,
    cleanup: Option<Box<dyn FnOnce() + Send + Sync>>,
}
#[cfg(test)]
impl TestFsWorkspace {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
#[cfg(test)]
impl Drop for TestFsWorkspace {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}
