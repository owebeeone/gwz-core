use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::RUNTIME_DIR;

pub const WORKSPACE_MUTATOR_LOCK_NAME: &str = "workspace-mutator.lock";

pub struct WorkspaceMutatorLock {
    file: File,
    path: PathBuf,
}

impl WorkspaceMutatorLock {
    /// Acquire the workspace mutation lock or return the standard busy error.
    pub fn acquire(root: &Path) -> ModelResult<Self> {
        Self::try_acquire(root)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::UnsupportedOperation,
                "workspace mutator lock is already held",
            )
        })
    }

    /// Try to acquire the workspace-wide mutation lock.
    ///
    /// The lock is an OS advisory exclusive lock on `.gwz/locks/workspace-mutator.lock`.
    /// The file itself is stable runtime state and may remain after a process exits. A
    /// remaining unlocked file is not stale. If a process dies while holding the lock,
    /// the OS releases the file lock with that process' file descriptor. This lock is
    /// intentionally workspace-wide, so stash and branch mutators in separate processes
    /// serialize before changing native Git state or `.gwz/` registry files.
    ///
    /// Advisory file locking must be reliable on the workspace filesystem. Network
    /// filesystems with broken advisory-lock semantics are unsupported for concurrent
    /// GWZ mutators; run mutating operations serially there.
    pub fn try_acquire(root: &Path) -> ModelResult<Option<Self>> {
        let path = lock_path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error)?;

        match try_lock_exclusive(&file) {
            Ok(true) => Ok(Some(Self { file, path })),
            Ok(false) => Ok(None),
            Err(error) => Err(io_error(error)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceMutatorLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file);
    }
}

pub fn lock_path(root: &Path) -> PathBuf {
    root.join(RUNTIME_DIR)
        .join("locks")
        .join(WORKSPACE_MUTATOR_LOCK_NAME)
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

fn io_error(err: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, err.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const CHILD_ENV: &str = "GWZ_WORKSPACE_MUTATOR_LOCK_CHILD_ROOT";

    #[test]
    fn lock_file_may_remain_and_be_reacquired_after_release() {
        let temp = TempDir::new("mutator-lock-reacquire");
        let first = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("first lock acquired");
        let path = first.path().to_path_buf();
        assert_eq!(path, lock_path(temp.path()));
        drop(first);

        assert!(path.is_file(), "lock file remains as runtime state");
        let second = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("released lock can be reacquired");
        drop(second);
    }

    #[test]
    fn separate_process_cannot_acquire_held_workspace_mutator_lock() {
        let temp = TempDir::new("mutator-lock-process");
        let _held = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("parent lock acquired");

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--ignored")
            .arg("--exact")
            .arg("operation::workspace_mutator_lock::tests::child_process_observes_lock_contention")
            .env(CHILD_ENV, temp.path())
            .status()
            .unwrap();

        assert!(status.success(), "child test process failed: {status}");
    }

    #[test]
    #[ignore]
    fn child_process_observes_lock_contention() {
        let Some(root) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        assert!(
            WorkspaceMutatorLock::try_acquire(Path::new(&root))
                .unwrap()
                .is_none()
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gwz-core-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
