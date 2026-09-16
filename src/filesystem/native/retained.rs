//! Retained-handle operations on an already-opened directory.

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
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
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
