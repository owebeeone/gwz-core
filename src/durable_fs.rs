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

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn rename_durable(
        source: &Path,
        destination: &Path,
        _replace_existing: bool,
    ) -> io::Result<()> {
        std::fs::rename(source, destination)
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
