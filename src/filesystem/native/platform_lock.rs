//! Advisory whole-file locking, per platform.
#![allow(dead_code, reason = "runtime-lock consumers are migrating")]

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
    if unsafe { unlock_file_ex(file.as_raw_handle(), 0, u32::MAX, u32::MAX, &mut overlapped) } != 0
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
