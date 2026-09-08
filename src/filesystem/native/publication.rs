use super::*;

#[cfg(not(windows))]
pub(super) fn open(source_dir: &Dir, source: &OsStr) -> io::Result<cap_std::fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_std::fs::OpenOptions;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .maybe_dir(true);
    let file = source_dir.open_with(source, &options)?;
    Ok(file)
}

#[cfg(windows)]
pub(super) fn open(source_dir: &Dir, source: &OsStr) -> io::Result<cap_std::fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::*;

    let mut options = OpenOptions::new();
    options
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH | FILE_FLAG_BACKUP_SEMANTICS,
        )
        .follow(FollowSymlinks::No);
    let file = source_dir.open_with(source, &options)?;
    Ok(file)
}

#[cfg(not(windows))]
pub(super) fn publish(
    file: &cap_std::fs::File,
    source_dir: &Dir,
    source_name: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    mode: RenameMode,
    acquired: &dyn Fn() -> io::Result<()>,
) -> io::Result<()> {
    let _ = file;
    acquired()?;
    retained::rename(source_dir, source_name, destination_dir, destination, mode)
}

#[cfg(windows)]
pub(super) fn publish(
    file: &cap_std::fs::File,
    source_dir: &Dir,
    source_name: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    mode: RenameMode,
    acquired: &dyn Fn() -> io::Result<()>,
) -> io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;

    let _ = (source_dir, source_name);

    let destination_path = retained::windows_destination_path(destination_dir, destination)?;
    acquired()?;
    let name = destination_path.encode_wide().collect::<Vec<_>>();
    // Windows requires at least the fixed structure size plus the variable
    // name bytes, even though the fixed structure already contains its
    // one-element FileName placeholder.
    let size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = mode == RenameMode::Replace;
        // SetFileInformationByHandle rejects a non-null RootDirectory on
        // supported Windows runners, so the destination is an absolute path
        // derived from the retained directory handle immediately before the
        // rename. The handle does NOT prevent a same-user process from
        // renaming the destination directory or a path ancestor inside this
        // window (directory opens share FILE_SHARE_DELETE); that residual is
        // assigned to the amendment's cooperating-same-user boundary, and
        // the mandatory post-publish verification through the retained
        // destination handle detects a redirect read-only (§4.1 erratum
        // 2026-08-15; native window test executes at R2-F).
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = u32::try_from(name.len() * 2)
            .map_err(|_| io::Error::other("destination name is too long"))?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            u32::try_from(size).map_err(|_| io::Error::other("rename buffer is too large"))?,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
