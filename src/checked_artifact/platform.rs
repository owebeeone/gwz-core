use std::ffi::OsStr;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError, ModelResult};

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_relative(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    let flags = if replace {
        rustix::fs::RenameFlags::empty()
    } else {
        rustix::fs::RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(dir, source, dir, destination, flags).map_err(|cause| {
        io_error(
            code,
            label,
            std::io::Error::from_raw_os_error(cause.raw_os_error()),
        )
    })
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn rename_relative(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if !replace {
        return Err(error(
            code,
            label,
            "atomic no-replace publication is unsupported on this Unix target",
        ));
    }
    dir.rename(source, dir, destination)
        .map_err(|cause| io_error(code, label, cause))
}

#[cfg(windows)]
pub(super) fn rename_relative(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;

    let mut options = OpenOptions::new();
    options
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH)
        .follow(FollowSymlinks::No)
        .maybe_dir(false);
    let source = dir
        .open_with(source, &options)
        .map_err(|cause| io_error(code, label, cause))?;
    let name = destination.encode_wide().collect::<Vec<_>>();
    let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = replace;
        (*info).RootDirectory = dir.as_raw_handle();
        (*info).FileNameLength = u32::try_from(name.len() * 2)
            .map_err(|_| error(code, label, "destination name is too long"))?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            u32::try_from(size).map_err(|_| error(code, label, "rename buffer is too large"))?,
        ) == 0
        {
            return Err(io_error(code, label, std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

#[cfg(all(not(unix), not(windows)))]
pub(super) fn rename_relative(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if !replace {
        return Err(error(
            code,
            label,
            "atomic no-replace publication is unsupported on this platform",
        ));
    }
    dir.rename(source, dir, destination)
        .map_err(|cause| io_error(code, label, cause))
}

pub(super) fn remove_relative(
    dir: &Dir,
    leaf: &OsStr,
    tombstone: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    #[cfg(windows)]
    {
        // A write-through, handle-relative no-replace rename makes the managed
        // leaf absence durable. Removing the private tombstone is cleanup; an
        // error remains recoverable because the managed leaf is already gone.
        rename_relative(dir, leaf, tombstone, false, code, label)?;
        return dir
            .remove_file(tombstone)
            .map_err(|cause| io_error(code, label, cause));
    }
    #[cfg(not(windows))]
    let _ = tombstone;
    dir.remove_file(leaf)
        .map_err(|cause| io_error(code, label, cause))
}

#[cfg(not(windows))]
pub(super) fn sync_parent(dir: &Dir) -> std::io::Result<()> {
    dir.try_clone()?.into_std_file().sync_all()
}

#[cfg(windows)]
pub(super) fn sync_parent(_dir: &Dir) -> std::io::Result<()> {
    // Relative replacement uses FILE_FLAG_WRITE_THROUGH. A normal directory
    // handle cannot be flushed portably on Windows.
    Ok(())
}

fn io_error(code: ErrorCode, label: &str, cause: std::io::Error) -> ModelError {
    error(code, label, cause)
}

fn error(code: ErrorCode, label: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(code, format!("checked {label}: {detail}"))
}
