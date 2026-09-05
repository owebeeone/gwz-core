//! The family's one OS advisory try-lock.
//!
//! Design §3.2: local create, dispose, disband and family exchange take the
//! one root family lock and refuse busy; the lock is held through the
//! command and released with its handle. This is an *advisory* OS lock on
//! `root/.gwz/local-family.lock` — `flock` on Unix, `LockFileEx` on Windows
//! — reached through `std::fs::File::try_lock`, which is exactly those two
//! calls and keeps this crate free of `unsafe` (see `#![forbid(unsafe_code)]`
//! in the crate root). It is not a create-file-as-lock protocol, has no
//! stale-lock recovery, and never touches gwz-core's private
//! checked-artifact locks.
//!
//! Because the lock lives on the open file description rather than the
//! process, two handles on the same path conflict *within* one process as
//! well as across processes: a second `try_lock` in the same program is
//! `Busy`, which is what the conformance suite measures.

// Ordinary file I/O in the family metadata publisher: this crate is outside
// gwz-core's merge-writer boundary (gwz-core/clippy.toml), whose disallowed
// writers exist to route *merge artifact* mutation through checked entries.
// The boundaries doc requires the opposite of that here — "ordinary
// temporary-write/rename with checked available flushes ... do not reach into
// private checked-artifact locks or the single-caller pinned `verified_write`
// helper" (GwzLocalCloneLibraryBoundaries.md §3).
#![allow(clippy::disallowed_methods)]

use std::fs::{File, TryLockError};
use std::io;
use std::path::Path;

/// What one try-lock attempt produced.
#[derive(Debug)]
pub(crate) enum Attempt {
    Acquired(FamilyLock),
    /// Another family operation holds it. A refusal, never a wait.
    Busy,
    /// The platform offers no supported advisory try-lock, so family
    /// mutation refuses rather than proceeding unserialised.
    Unsupported(String),
}

/// A held advisory lock. Dropping it releases the lock with the handle.
#[derive(Debug)]
pub(crate) struct FamilyLock {
    file: File,
}

impl FamilyLock {
    /// Take the lock at `path` without waiting. The lock file is created if
    /// it is missing and is never truncated: it carries no state, only the
    /// lock.
    pub(crate) fn try_acquire(path: &Path) -> io::Result<Attempt> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Attempt::Acquired(Self { file })),
            Err(TryLockError::WouldBlock) => Ok(Attempt::Busy),
            Err(TryLockError::Error(error)) if error.kind() == io::ErrorKind::Unsupported => {
                Ok(Attempt::Unsupported(error.to_string()))
            }
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for FamilyLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock; unlocking first makes the
        // release explicit and is checked only in the sense that a failure
        // here cannot be reported from `Drop`.
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_handle_is_busy_in_this_process_until_the_first_is_dropped() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("local-family.lock");
        let first = match FamilyLock::try_acquire(&path).unwrap() {
            Attempt::Acquired(lock) => lock,
            other => panic!("the first attempt takes the lock, got {other:?}"),
        };
        // A second *handle* on the same path, in this same process: the
        // lock lives on the open file description, so this is Busy.
        assert!(
            matches!(FamilyLock::try_acquire(&path).unwrap(), Attempt::Busy),
            "a second handle must be busy while the first is held"
        );
        drop(first);
        assert!(
            matches!(
                FamilyLock::try_acquire(&path).unwrap(),
                Attempt::Acquired(_)
            ),
            "the lock is released with its handle"
        );
    }

    #[test]
    fn acquiring_creates_the_lock_file_without_truncating_it() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("local-family.lock");
        assert!(!path.exists());
        let held = FamilyLock::try_acquire(&path).unwrap();
        assert!(
            path.is_file(),
            "the lock file is created by locking, not by reading"
        );
        drop(held);
        std::fs::write(&path, b"marker").unwrap();
        let held = FamilyLock::try_acquire(&path).unwrap();
        assert!(matches!(held, Attempt::Acquired(_)));
        drop(held);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"marker",
            "the file is not truncated"
        );
    }

    #[test]
    fn a_missing_directory_is_an_io_error_not_a_refusal() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("absent").join("local-family.lock");
        let error = FamilyLock::try_acquire(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
