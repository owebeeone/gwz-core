//! The native `FileSystem` implementation.

use super::*;

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
