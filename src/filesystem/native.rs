//! The only native I/O implementation of the filesystem contract.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "native filesystem adapter"
)]
use super::*;
mod facts;
mod filesystem_impl;
mod handles;
mod legacy_identity;
mod platform_lock;
mod publication;
mod retained;

pub(super) use handles::{Directory, File, Lock, NativeFileSystem};
use handles::{directory, durable_options, file, identity, metadata_value, with_file};

#[cfg(not(windows))]
use cap_fs_ext::DirExt;
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::PermissionsExt;
use cap_std::fs::{Dir, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

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
