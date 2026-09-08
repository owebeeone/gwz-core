//! The only native I/O implementation of the filesystem contract.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "native filesystem adapter"
)]
use super::*;
mod facts;
mod legacy_identity;
mod publication;
#[cfg(not(windows))]
use cap_fs_ext::DirExt;
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::PermissionsExt;
use cap_std::fs::{Dir, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(super) struct NativeFileSystem;
pub(super) struct Directory(Dir);
pub(super) struct File(Arc<Mutex<cap_std::fs::File>>);
#[allow(dead_code, reason = "runtime-lock consumers are migrating")]
pub(super) struct Lock(Arc<Mutex<cap_std::fs::File>>);

fn directory(value: &FsDirectory) -> io::Result<&Dir> {
    match &value.0 {
        DirectoryHandle::Native(dir) => Ok(&dir.0),
        #[cfg(test)]
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong filesystem backend",
        )),
    }
}

fn with_file<T>(value: &FsFile, body: impl FnOnce(&cap_std::fs::File) -> T) -> io::Result<T> {
    let file = file(value)?
        .lock()
        .map_err(|_| io::Error::other("file lock poisoned"))?;
    Ok(body(&file))
}
fn file(value: &FsFile) -> io::Result<&Arc<Mutex<cap_std::fs::File>>> {
    match &value.0 {
        FileHandle::Native(file) => Ok(&file.0),
        #[cfg(test)]
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong filesystem backend",
        )),
    }
}

fn metadata_value(metadata: &cap_fs_ext::Metadata) -> FsMetadata {
    let kind = metadata.file_type();
    let kind = if kind.is_file() {
        FsKind::File
    } else if kind.is_dir() {
        FsKind::Directory
    } else if kind.is_symlink() {
        FsKind::Symlink
    } else {
        FsKind::Other
    };
    #[cfg(unix)]
    let executable = metadata.permissions().mode() & 0o111 != 0;
    #[cfg(not(unix))]
    let executable = false;
    FsMetadata {
        kind,
        executable,
        identity: FsIdentity {
            namespace: metadata.dev(),
            object: metadata.ino(),
        },
        length: metadata.len(),
    }
}

impl FileSystem for NativeFileSystem {
    fn legacy_directory_identity(&self, value: &FsDirectory) -> io::Result<FsLegacyObjectIdentity> {
        legacy_identity::dir_object_identity(directory(value)?)
    }
    fn legacy_file_identity(&self, value: &FsFile) -> io::Result<FsLegacyObjectIdentity> {
        with_file(value, legacy_identity::file_object_identity)?
    }
    fn legacy_rename_domain(&self, value: &FsDirectory) -> io::Result<FsLegacyRenameDomain> {
        legacy_identity::rename_domain(directory(value)?)
    }
    fn legacy_path_identity(&self, value: &FsDirectory, relative: &Path) -> io::Result<Vec<u8>> {
        legacy_identity::path_identity(directory(value)?, relative)
    }
    fn directory_names(&self, value: &FsDirectory) -> io::Result<FsDirectoryNames> {
        Ok(Box::new(
            directory(value)?
                .entries()?
                .map(|entry| entry.map(|entry| entry.file_name())),
        ))
    }

    fn create_private_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        durable_options(&mut options);
        directory(parent)?.open_with(name, &options).map(|file| {
            FsFile(
                FileHandle::Native(File(Arc::new(Mutex::new(file)))),
                0,
                Arc::new(*self),
            )
        })
    }
    fn open_publication_source(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let file = publication::open(directory(parent)?, name)?;
        Ok(FsFile(
            FileHandle::Native(File(Arc::new(Mutex::new(file)))),
            0,
            Arc::new(*self),
        ))
    }
    fn publish_source(
        &self,
        source: FsPublicationSource<'_>,
        destination: &FsDirectory,
        target: &OsStr,
        mode: RenameMode,
        acquired: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        component(source.name)?;
        component(target)?;
        with_file(source.file, |file| {
            publication::publish(
                file,
                directory(source.parent)?,
                source.name,
                directory(destination)?,
                target,
                mode,
                acquired,
            )
        })?
    }
    fn file_metadata(&self, value: &FsFile) -> io::Result<FsMetadata> {
        with_file(value, |file| {
            file.metadata().map(|value| metadata_value(&value))
        })?
    }
    fn support_profile(&self) -> FsSupportProfile {
        facts::support_profile()
    }
    fn persistent_directory_identity(
        &self,
        value: &FsDirectory,
    ) -> Result<FsObjectIdentity, FsProbeError> {
        facts::dir_identity(
            directory(value)
                .map_err(|e| FsProbeError::io("access native retained directory", e))?,
        )
    }
    fn persistent_file_identity(&self, value: &FsFile) -> Result<FsObjectIdentity, FsProbeError> {
        with_file(value, facts::file_identity)
            .map_err(|e| FsProbeError::io("access native retained file", e))?
    }
    fn lookup_mode(&self, value: &FsDirectory) -> Result<FsLookupMode, FsProbeError> {
        facts::parent_mode(
            directory(value)
                .map_err(|e| FsProbeError::io("access native retained directory", e))?,
        )
    }
    fn rename_domain(&self, value: &FsDirectory) -> Result<Vec<u8>, FsProbeError> {
        facts::rename_domain(
            directory(value)
                .map_err(|e| FsProbeError::io("access native retained directory", e))?,
        )
    }
    fn describe_volume(&self, value: &FsDirectory) -> Result<FsVolumeDescription, FsProbeError> {
        facts::describe_volume(
            directory(value)
                .map_err(|e| FsProbeError::io("access native retained directory", e))?,
        )
    }
    fn canonical_path(&self, path: &Path) -> io::Result<std::path::PathBuf> {
        std::fs::canonicalize(path)
    }
    #[cfg(windows)]
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::*;

        // Windows path-only metadata omits the volume and file identity.
        // Observe the entry itself through one handle, including dangling
        // reparse points and directory roots, without opening its target.
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let metadata = cap_std::fs::File::from_std(file).metadata()?;
        Ok(metadata_value(&metadata))
    }
    #[cfg(not(windows))]
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        let metadata = std::fs::symlink_metadata(path)?;
        let kind = metadata.file_type();
        let kind = if kind.is_file() {
            FsKind::File
        } else if kind.is_dir() {
            FsKind::Directory
        } else if kind.is_symlink() {
            FsKind::Symlink
        } else {
            FsKind::Other
        };
        #[cfg(unix)]
        let executable =
            std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o111 != 0;
        #[cfg(not(unix))]
        let executable = false;
        let metadata = cap_std::fs::Metadata::from_just_metadata(metadata);
        Ok(FsMetadata {
            kind,
            executable,
            identity: FsIdentity {
                namespace: metadata.dev(),
                object: metadata.ino(),
            },
            length: metadata.len(),
        })
    }
    fn metadata_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsMetadata> {
        component(name)?;
        let metadata = directory(parent)?.symlink_metadata(name)?;
        Ok(metadata_value(&metadata))
    }
    fn link_target(&self, path: &Path) -> io::Result<std::path::PathBuf> {
        std::fs::read_link(path)
    }
    fn remove_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        directory(parent)?.remove_file(name)
    }
    fn remove_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        directory(parent)?.remove_dir(name)
    }

    fn open_directory(&self, path: &Path) -> io::Result<FsDirectory> {
        let dir = Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
        #[cfg(windows)]
        let dir = retained::open_directory(&dir, OsStr::new("."))?;
        Ok(FsDirectory(
            DirectoryHandle::Native(Directory(dir)),
            Arc::new(*self),
        ))
    }
    fn clone_directory(&self, value: &FsDirectory) -> io::Result<FsDirectory> {
        directory(value)?
            .try_clone()
            .map(|dir| FsDirectory(DirectoryHandle::Native(Directory(dir)), Arc::new(*self)))
    }
    fn open_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsDirectory> {
        component(name)?;
        retained::open_directory(directory(parent)?, name)
            .map(|dir| FsDirectory(DirectoryHandle::Native(Directory(dir)), Arc::new(*self)))
    }
    fn create_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        directory(parent)?.create_dir(name)
    }
    fn open_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory(parent)?.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(FsFile(
            FileHandle::Native(File(Arc::new(Mutex::new(file)))),
            0,
            Arc::new(*self),
        ))
    }
    fn open_lock_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).follow(FollowSymlinks::No);
        durable_options(&mut options);
        let file = directory(parent)?.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(FsFile(
            FileHandle::Native(File(Arc::new(Mutex::new(file)))),
            0,
            Arc::new(*self),
        ))
    }
    fn open_file_for_write_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).follow(FollowSymlinks::No);
        durable_options(&mut options);
        let file = directory(parent)?.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(FsFile(
            FileHandle::Native(File(Arc::new(Mutex::new(file)))),
            0,
            Arc::new(*self),
        ))
    }
    fn create_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        durable_options(&mut options);
        directory(parent)?.open_with(name, &options).map(|file| {
            FsFile(
                FileHandle::Native(File(Arc::new(Mutex::new(file)))),
                0,
                Arc::new(*self),
            )
        })
    }
    fn read_at(&self, handle: &FsFile, offset: u64, bytes: &mut [u8]) -> io::Result<usize> {
        let mut file = file(handle)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read(bytes)
    }
    fn file_len(&self, handle: &FsFile) -> io::Result<u64> {
        file(handle)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?
            .metadata()
            .map(|metadata| metadata.len())
    }
    fn write_at(&self, handle: &FsFile, offset: u64, bytes: &[u8]) -> io::Result<usize> {
        let mut file = file(handle)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?;
        file.seek(SeekFrom::Start(offset))?;
        file.write(bytes)
    }
    fn set_len(&self, handle: &FsFile, length: u64) -> io::Result<()> {
        file(handle)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?
            .set_len(length)
    }
    fn sync_file(&self, handle: &FsFile) -> io::Result<()> {
        file(handle)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?
            .sync_all()
    }
    fn directory_identity(&self, value: &FsDirectory) -> io::Result<FsIdentity> {
        identity(&directory(value)?.dir_metadata()?)
    }
    fn file_identity(&self, value: &FsFile) -> io::Result<FsIdentity> {
        identity(
            &file(value)?
                .lock()
                .map_err(|_| io::Error::other("file lock poisoned"))?
                .metadata()?,
        )
    }
    fn rename(&self, source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()> {
        platform_rename::rename(source, destination, mode)
    }
    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        #[cfg(not(windows))]
        {
            std::fs::File::open(path)?.sync_all()
        }
        #[cfg(windows)]
        {
            self.open_directory(path).map(|_| ())
        }
    }
    fn sync_directory_at(&self, value: &FsDirectory) -> io::Result<()> {
        retained::sync_directory(directory(value)?)
    }
    fn read_directory(&self, path: &Path) -> io::Result<Vec<FsDirectoryEntry>> {
        self.read_directory_at(&self.open_directory(path)?)
    }
    fn read_directory_at(&self, value: &FsDirectory) -> io::Result<Vec<FsDirectoryEntry>> {
        let mut entries = directory(value)?
            .entries()?
            .map(|entry| {
                let entry = entry?;
                let kind = entry.file_type()?;
                let kind = if kind.is_file() {
                    FsKind::File
                } else if kind.is_dir() {
                    FsKind::Directory
                } else if kind.is_symlink() {
                    FsKind::Symlink
                } else {
                    FsKind::Other
                };
                Ok(FsDirectoryEntry {
                    name: entry.file_name(),
                    kind,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }
    fn directory_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsDirectory,
    ) -> io::Result<bool> {
        component(name)?;
        let metadata = match directory(parent)?.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || metadata.is_symlink() {
            return Ok(false);
        }
        let retained = directory(child)?.dir_metadata()?;
        Ok((metadata.dev(), metadata.ino()) == (retained.dev(), retained.ino()))
    }
    fn file_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsFile,
    ) -> io::Result<bool> {
        component(name)?;
        let metadata = match directory(parent)?.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.is_symlink() {
            return Ok(false);
        }
        let retained = file(child)?
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?
            .metadata()?;
        Ok((metadata.dev(), metadata.ino()) == (retained.dev(), retained.ino()))
    }
    #[cfg(test)]
    fn test_create_symlink_at(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        target: &Path,
    ) -> io::Result<()> {
        component(name)?;
        #[cfg(windows)]
        {
            let directory = directory(parent)?;
            match directory.metadata(target) {
                Ok(metadata) if metadata.is_dir() => directory.symlink_dir(target, name),
                Ok(_) => directory.symlink_file(target, name),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    directory.symlink_file(target, name)
                }
                Err(error) => Err(error),
            }
        }
        #[cfg(not(windows))]
        directory(parent)?.symlink(target, name)
    }
    fn try_lock_file(&self, value: &FsFile) -> io::Result<Option<FsLockGuard>> {
        let file = file(value)?.clone();
        let acquired = {
            let retained = file
                .lock()
                .map_err(|_| io::Error::other("file lock poisoned"))?;
            platform_lock::try_lock_exclusive(&retained)?
        };
        Ok(acquired.then(|| FsLockGuard {
            _handle: LockHandle::Native { _lock: Lock(file) },
        }))
    }
    fn rename_at(
        &self,
        source: &FsDirectory,
        name: &OsStr,
        destination: &FsDirectory,
        target: &OsStr,
        mode: RenameMode,
    ) -> io::Result<()> {
        component(name)?;
        component(target)?;
        retained::rename(
            directory(source)?,
            name,
            directory(destination)?,
            target,
            mode,
        )
    }
    #[cfg(test)]
    fn test_workspace(&self) -> io::Result<TestFsWorkspace> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "gwz-fs-contract-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    let path = std::fs::canonicalize(path)?;
                    let cleanup_path = path.clone();
                    return Ok(TestFsWorkspace {
                        path,
                        cleanup: Some(Box::new(move || {
                            let _ = std::fs::remove_dir_all(cleanup_path);
                        })),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

#[allow(dead_code, reason = "retained identity consumers are migrating")]
fn identity(metadata: &cap_fs_ext::Metadata) -> io::Result<FsIdentity> {
    Ok(FsIdentity {
        namespace: metadata.dev(),
        object: metadata.ino(),
    })
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Ok(file) = self.0.lock() {
            let _ = platform_lock::unlock(&file);
        }
    }
}

#[allow(dead_code, reason = "runtime-lock consumers are migrating")]
mod platform_lock {
    use super::*;

    #[cfg(unix)]
    pub(super) fn try_lock_exclusive(file: &cap_std::fs::File) -> io::Result<bool> {
        use std::os::fd::AsRawFd;

        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
        if rc == 0 {
            Ok(true)
        } else {
            let error = io::Error::last_os_error();
            if matches!(error.kind(), io::ErrorKind::WouldBlock)
                || matches!(error.raw_os_error(), Some(11) | Some(35))
            {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn unlock(file: &cap_std::fs::File) -> io::Result<()> {
        use std::os::fd::AsRawFd;

        const LOCK_UN: i32 = 8;
        if unsafe { flock(file.as_raw_fd(), LOCK_UN) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(unix)]
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }

    #[cfg(windows)]
    pub(super) fn try_lock_exclusive(file: &cap_std::fs::File) -> io::Result<bool> {
        use std::os::windows::io::AsRawHandle;

        const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
        const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;
        let mut overlapped = Overlapped::default();
        let rc = unsafe {
            lock_file_ex(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if rc != 0 {
            Ok(true)
        } else {
            let error = io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(32) | Some(33)) {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }

    #[cfg(windows)]
    pub(super) fn unlock(file: &cap_std::fs::File) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;

        let mut overlapped = Overlapped::default();
        if unsafe { unlock_file_ex(file.as_raw_handle(), 0, u32::MAX, u32::MAX, &mut overlapped) }
            != 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(windows)]
    #[repr(C)]
    #[derive(Default)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        h_event: *mut std::ffi::c_void,
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "LockFileEx"]
        fn lock_file_ex(
            h_file: *mut std::ffi::c_void,
            dw_flags: u32,
            dw_reserved: u32,
            number_of_bytes_to_lock_low: u32,
            number_of_bytes_to_lock_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;

        #[link_name = "UnlockFileEx"]
        fn unlock_file_ex(
            h_file: *mut std::ffi::c_void,
            dw_reserved: u32,
            number_of_bytes_to_unlock_low: u32,
            number_of_bytes_to_unlock_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
    }

    #[cfg(not(any(unix, windows)))]
    pub(super) fn try_lock_exclusive(_file: &cap_std::fs::File) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem advisory locks are unsupported on this platform",
        ))
    }

    #[cfg(not(any(unix, windows)))]
    pub(super) fn unlock(_file: &cap_std::fs::File) -> io::Result<()> {
        Ok(())
    }
}

mod retained {
    use super::*;

    #[cfg(not(windows))]
    pub(super) fn open_directory(parent: &Dir, name: &OsStr) -> io::Result<Dir> {
        parent.open_dir_nofollow(name)
    }

    #[cfg(windows)]
    pub(super) fn open_directory(parent: &Dir, name: &OsStr) -> io::Result<Dir> {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Foundation::GENERIC_READ;
        use windows_sys::Win32::Storage::FileSystem::*;

        // Retain the object across renames, rather than preventing namespace changes.
        let mut options = OpenOptions::new();
        options
            .access_mode(GENERIC_READ)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .follow(FollowSymlinks::No);
        let file = parent.open_with(name, &options)?;
        if !file.metadata()?.is_dir() {
            return Err(io::ErrorKind::NotADirectory.into());
        }
        Ok(Dir::from_std_file(file.into_std()))
    }

    pub(super) fn rename(
        source: &Dir,
        name: &OsStr,
        destination: &Dir,
        target: &OsStr,
        mode: RenameMode,
    ) -> io::Result<()> {
        match mode {
            RenameMode::Replace => source.rename(name, destination, target),
            RenameMode::NoReplace => rename_no_replace(source, name, destination, target),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn rename_no_replace(
        source: &Dir,
        name: &OsStr,
        destination: &Dir,
        target: &OsStr,
    ) -> io::Result<()> {
        rustix::fs::renameat_with(
            source,
            name,
            destination,
            target,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
    }

    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn rename_no_replace(
        _source: &Dir,
        _name: &OsStr,
        _destination: &Dir,
        _target: &OsStr,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace directory publication is unsupported on this Unix target",
        ))
    }

    #[cfg(windows)]
    fn rename_no_replace(
        source: &Dir,
        name: &OsStr,
        destination: &Dir,
        target: &OsStr,
    ) -> io::Result<()> {
        let file = publication::open(source, name)?;
        publication::publish(
            &file,
            source,
            name,
            destination,
            target,
            RenameMode::NoReplace,
            &|| Ok(()),
        )
    }

    #[cfg(windows)]
    pub(super) fn windows_destination_path(dir: &Dir, destination: &OsStr) -> io::Result<OsString> {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
        };

        const MAX_PATH_UNITS: usize = 32_768;
        let mut buffer = vec![0; 512];
        loop {
            let capacity = u32::try_from(buffer.len())
                .map_err(|_| io::Error::other("destination path buffer is too large"))?;
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    dir.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    capacity,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            if length == 0 {
                return Err(io::Error::last_os_error());
            }
            let length = usize::try_from(length)
                .map_err(|_| io::Error::other("destination path length is invalid"))?;
            if length < buffer.len() {
                buffer.truncate(length);
                let mut path = std::path::PathBuf::from(OsString::from_wide(&buffer));
                path.push(destination);
                return Ok(path.into_os_string());
            }
            let required = length
                .checked_add(1)
                .ok_or_else(|| io::Error::other("destination path length overflowed"))?;
            if required > MAX_PATH_UNITS {
                return Err(io::Error::other(
                    "destination path exceeds the platform bound",
                ));
            }
            buffer.resize(required, 0);
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn sync_directory(dir: &Dir) -> io::Result<()> {
        let flushable = rustix::fs::openat(
            dir,
            c".",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?;
        rustix::fs::fsync(&flushable).map_err(io::Error::from)
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    pub(super) fn sync_directory(dir: &Dir) -> io::Result<()> {
        dir.try_clone()?.into_std_file().sync_all()
    }

    #[cfg(windows)]
    pub(super) fn sync_directory(dir: &Dir) -> io::Result<()> {
        dir.dir_metadata().map(|_| ())
    }
}

#[cfg(unix)]
mod platform_rename {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    use super::*;

    pub(super) fn rename(source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()> {
        match mode {
            RenameMode::Replace => std::fs::rename(source, destination),
            RenameMode::NoReplace => rename_noreplace(source, destination),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
        use std::ffi::{c_char, c_int, c_uint};

        const AT_FDCWD: c_int = -100;
        const RENAME_NOREPLACE: c_uint = 1;
        unsafe extern "C" {
            fn renameat2(
                olddirfd: c_int,
                oldpath: *const c_char,
                newdirfd: c_int,
                newpath: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }
        let source = c_path(source)?;
        let destination = c_path(destination)?;
        // SAFETY: both owned C strings are NUL-terminated and live through the call.
        if unsafe {
            renameat2(
                AT_FDCWD,
                source.as_ptr(),
                AT_FDCWD,
                destination.as_ptr(),
                RENAME_NOREPLACE,
            )
        } == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
        use std::ffi::{c_char, c_int, c_uint};

        const RENAME_EXCL: c_uint = 0x0000_0004;
        unsafe extern "C" {
            fn renamex_np(oldpath: *const c_char, newpath: *const c_char, flags: c_uint) -> c_int;
        }
        let source = c_path(source)?;
        let destination = c_path(destination)?;
        // SAFETY: both owned C strings are NUL-terminated and live through the call.
        if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace rename is unavailable on this platform",
        ))
    }

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "path contains an embedded NUL")
        })
    }
}

#[cfg(all(not(unix), not(windows)))]
mod platform_rename {
    use super::*;

    pub(super) fn rename(source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()> {
        match mode {
            RenameMode::Replace => std::fs::rename(source, destination),
            RenameMode::NoReplace => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "atomic no-replace rename is unavailable on this platform",
            )),
        }
    }
}

#[cfg(windows)]
mod platform_rename {
    use std::iter;
    use std::os::windows::ffi::OsStrExt as _;

    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    use super::*;

    pub(super) fn rename(source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()> {
        if mode == RenameMode::Replace {
            // Rust's handle-based rename supports replacing an open destination.
            return std::fs::rename(source, destination);
        }
        let source = wide_path(source)?;
        let destination = wide_path(destination)?;
        let flags = MOVEFILE_WRITE_THROUGH;
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and live through the call.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains an embedded NUL",
            ));
        }
        Ok(encoded.into_iter().chain(iter::once(0)).collect())
    }
}

fn durable_options(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
    }
    #[cfg(not(windows))]
    let _ = options;
}
