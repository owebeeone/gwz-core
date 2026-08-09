use std::io;
use std::path::Path;

/// Atomically publish `source` at `destination`, optionally replacing an existing file.
///
/// Callers must finish writing and syncing `source` before calling this function.
pub(crate) fn rename_durable(
    source: &Path,
    destination: &Path,
    replace_existing: bool,
) -> io::Result<()> {
    platform::rename_durable(source, destination, replace_existing)
}

/// Atomically move `source` to an absent `destination` without replacement.
#[allow(
    dead_code,
    reason = "v1 lifecycle archive is production-disabled until A1"
)]
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    platform::rename_noreplace(source, destination)
}

/// Flush directory-entry changes where the platform exposes that operation.
///
/// Windows supplies the persistence barrier through `MOVEFILE_WRITE_THROUGH`;
/// opening a directory as a normal `File` and syncing it fails with access denied.
#[cfg(not(windows))]
pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
pub(crate) fn sync_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
mod platform {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    use super::*;

    pub(super) fn rename_durable(
        source: &Path,
        destination: &Path,
        _replace_existing: bool,
    ) -> io::Result<()> {
        std::fs::rename(source, destination)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
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
        // SAFETY: both owned C strings are NUL-terminated and remain alive for
        // the duration of the synchronous call.
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
    pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
        use std::ffi::{c_char, c_int, c_uint};

        const RENAME_EXCL: c_uint = 0x0000_0004;
        unsafe extern "C" {
            fn renamex_np(oldpath: *const c_char, newpath: *const c_char, flags: c_uint) -> c_int;
        }
        let source = c_path(source)?;
        let destination = c_path(destination)?;
        // SAFETY: both owned C strings are NUL-terminated and remain alive for
        // the duration of the synchronous call.
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
    pub(super) fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
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
mod platform {
    use super::*;

    pub(super) fn rename_durable(
        source: &Path,
        destination: &Path,
        _replace_existing: bool,
    ) -> io::Result<()> {
        std::fs::rename(source, destination)
    }

    pub(super) fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace rename is unavailable on this platform",
        ))
    }
}

#[cfg(windows)]
mod platform {
    use std::iter;
    use std::os::windows::ffi::OsStrExt as _;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    use super::*;

    pub(super) fn rename_durable(
        source: &Path,
        destination: &Path,
        replace_existing: bool,
    ) -> io::Result<()> {
        let source = wide_path(source)?;
        let destination = wide_path(destination)?;
        let mut flags = MOVEFILE_WRITE_THROUGH;
        if replace_existing {
            flags |= MOVEFILE_REPLACE_EXISTING;
        }
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and remain
        // alive for the duration of the synchronous Windows API call.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
        rename_durable(source, destination, false)
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
