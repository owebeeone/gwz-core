use std::io;

use cap_std::fs::File;

pub(super) struct AdvisoryLock {
    file: File,
}

impl AdvisoryLock {
    pub(super) fn try_acquire(file: File) -> io::Result<Option<Self>> {
        if try_lock_exclusive(&file)? {
            Ok(Some(Self { file }))
        } else {
            Ok(None)
        }
    }

    pub(super) fn file(&self) -> &File {
        &self.file
    }
}

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file);
    }
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if rc == 0 {
        Ok(true)
    } else {
        let error = io::Error::last_os_error();
        if matches!(error.kind(), io::ErrorKind::WouldBlock) || raw_os_error_is_lock_busy(&error) {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
fn unlock(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    const LOCK_UN: i32 = 8;
    let rc = unsafe { flock(file.as_raw_fd(), LOCK_UN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn raw_os_error_is_lock_busy(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(11) | Some(35))
}

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;

    const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x00000001;
    const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x00000002;

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
        if raw_os_error_is_windows_lock_busy(&error) {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

#[cfg(windows)]
fn unlock(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    let mut overlapped = Overlapped::default();
    let rc =
        unsafe { unlock_file_ex(file.as_raw_handle(), 0, u32::MAX, u32::MAX, &mut overlapped) };
    if rc != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn raw_os_error_is_windows_lock_busy(error: &io::Error) -> bool {
    const ERROR_LOCK_VIOLATION: i32 = 33;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    matches!(
        error.raw_os_error(),
        Some(ERROR_LOCK_VIOLATION | ERROR_SHARING_VIOLATION)
    )
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
fn try_lock_exclusive(_file: &File) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "workspace mutator lock requires OS advisory file locks on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock(_file: &File) -> io::Result<()> {
    Ok(())
}
