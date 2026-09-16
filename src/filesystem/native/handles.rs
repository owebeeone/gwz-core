//! The native handle types and the small accessors that unwrap them.
//!
//! Handles are thin wrappers over `cap_std`; everything that reaches inside
//! one lives here so the contract implementation stays readable.

use super::*;

#[derive(Clone, Copy)]
pub(in crate::filesystem) struct NativeFileSystem;
pub(in crate::filesystem) struct Directory(pub(super) Dir);
pub(in crate::filesystem) struct File(pub(super) Arc<Mutex<cap_std::fs::File>>);
#[allow(dead_code, reason = "runtime-lock consumers are migrating")]
pub(in crate::filesystem) struct Lock(pub(super) Arc<Mutex<cap_std::fs::File>>);

pub(super) fn directory(value: &FsDirectory) -> io::Result<&Dir> {
    match &value.0 {
        DirectoryHandle::Native(dir) => Ok(&dir.0),
        #[cfg(test)]
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong filesystem backend",
        )),
    }
}

pub(super) fn with_file<T>(
    value: &FsFile,
    body: impl FnOnce(&cap_std::fs::File) -> T,
) -> io::Result<T> {
    let file = file(value)?
        .lock()
        .map_err(|_| io::Error::other("file lock poisoned"))?;
    Ok(body(&file))
}
pub(super) fn file(value: &FsFile) -> io::Result<&Arc<Mutex<cap_std::fs::File>>> {
    match &value.0 {
        FileHandle::Native(file) => Ok(&file.0),
        #[cfg(test)]
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong filesystem backend",
        )),
    }
}

pub(super) fn metadata_value(metadata: &cap_fs_ext::Metadata) -> FsMetadata {
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

#[allow(dead_code, reason = "retained identity consumers are migrating")]
pub(super) fn identity(metadata: &cap_fs_ext::Metadata) -> io::Result<FsIdentity> {
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

pub(super) fn durable_options(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
    }
    #[cfg(not(windows))]
    let _ = options;
}
