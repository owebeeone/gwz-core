//! Ordinary metadata publication: same-directory temporary file, rename,
//! checked flushes, and the no-follow reads that precede them.
//!
//! This is design §3.1's "best-effort metadata publication, not a
//! power-loss-safe multi-file transaction": every step's error is checked
//! and returned, nothing is retried or repaired, and a failure leaves the
//! previous file in place for inspection. There is no durability catalog,
//! journal or replay here, and this module never reaches into gwz-core's
//! private checked-artifact writers.

// Ordinary file I/O in the family metadata publisher: this crate is outside
// gwz-core's merge-writer boundary (gwz-core/clippy.toml), whose disallowed
// writers exist to route *merge artifact* mutation through checked entries.
// The boundaries doc requires the opposite of that here — "ordinary
// temporary-write/rename with checked available flushes ... do not reach into
// private checked-artifact locks or the single-caller pinned `verified_write`
// helper" (GwzLocalCloneLibraryBoundaries.md §3).
#![allow(clippy::disallowed_methods)]

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What stands at a metadata path. A path that is not a regular file is
/// never followed or overwritten: it is reported so the caller can refuse
/// and retain it for inspection (design §3.1, "no-follow checks where
/// available").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileState {
    Absent,
    Regular,
    /// A symlink, directory or other node occupies the metadata path.
    Irregular,
}

impl FileState {
    pub(crate) fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// Classify `path` without following a final symlink.
pub(crate) fn file_state(path: &Path) -> io::Result<FileState> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(FileState::Regular),
        Ok(_) => Ok(FileState::Irregular),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileState::Absent),
        Err(error) => Err(error),
    }
}

/// Read a metadata file whose size the caller has already admitted.
pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// The encoded size on disk, for the caller's own limit decision.
pub(crate) fn encoded_size(path: &Path) -> io::Result<u64> {
    Ok(fs::metadata(path)?.len())
}

/// Create `directory` if it is missing, creating nothing above it: the
/// orchestrator allocates the workspace, the store owns only `.gwz/`.
/// Something already at that path which is not a directory is an error, not
/// a success: the store neither replaces nor writes through it.
pub(crate) fn ensure_metadata_directory(directory: &Path) -> io::Result<()> {
    match fs::create_dir(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(directory)?.is_dir() {
                return Ok(());
            }
            Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!(
                    "{} is not a directory, so the family metadata cannot live there",
                    directory.display()
                ),
            ))
        }
        Err(error) => Err(error),
    }
}

/// Distinct temporary names within a process, so two concurrent
/// publications in one directory never collide.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `target` through a same-directory temporary file:
/// write, flush the file, rename, then flush the directory where the
/// platform offers it. Every step is checked; a failure removes the
/// temporary file and leaves any previous `target` untouched.
pub(crate) fn publish(target: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a metadata file always has a parent directory",
        )
    })?;
    let temporary = temporary_path(target)?;
    let outcome = write_then_rename(&temporary, target, contents, directory);
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

fn write_then_rename(
    temporary: &Path,
    target: &Path,
    contents: &[u8],
    directory: &Path,
) -> io::Result<()> {
    let mut file = File::create(temporary)?;
    file.write_all(contents)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, target)?;
    flush_directory(directory)
}

fn temporary_path(target: &Path) -> io::Result<PathBuf> {
    let directory = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a metadata file always has a parent directory",
        )
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "a metadata file always has a UTF-8 name",
            )
        })?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(directory.join(format!(
        ".{name}.tmp.{process}.{sequence}",
        process = std::process::id(),
    )))
}

/// Flush the directory entry itself where the platform offers it. Windows
/// has no directory handle to flush through `std::fs`, so the rename is the
/// publication there; this is the "checked ordinary flush operations
/// available on the platform" of design §3.1, not a durability promise.
fn flush_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

/// Remove `path` if it is there. An absent file is not an error: removal is
/// repeatable (contract `remove_pointer`).
pub(crate) fn remove(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary directory")
    }

    #[test]
    fn publication_replaces_the_target_and_leaves_no_temporary_behind() {
        let temp = temp();
        let target = temp.path().join("local-family.yml");
        publish(&target, b"first").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"first");
        publish(&target, b"second").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"second");
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name != "local-family.yml")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_failed_publication_keeps_the_previous_file_and_removes_the_temporary() {
        let temp = temp();
        let target = temp.path().join("family-root");
        publish(&target, b"good").unwrap();
        // A directory at the target makes the rename fail with a real OS
        // error; the previous bytes are what a reread still sees.
        let blocked = temp.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        let error = publish(&blocked, b"never").unwrap_err();
        assert!(error.kind() != io::ErrorKind::NotFound, "{error:?}");
        assert_eq!(fs::read(&target).unwrap(), b"good");
        let temporaries: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(temporaries.is_empty(), "{temporaries:?}");
    }

    #[test]
    fn temporary_names_are_distinct_within_a_process() {
        let target = Path::new("/roots/one/.gwz/local-family.yml");
        let first = temporary_path(target).unwrap();
        let second = temporary_path(target).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), target.parent(), "same directory");
    }

    #[test]
    fn the_metadata_directory_is_created_but_nothing_above_it() {
        let temp = temp();
        let workspace = temp.path().join("ws-A");
        let metadata = workspace.join(".gwz");
        let error = ensure_metadata_directory(&metadata).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert!(
            !workspace.exists(),
            "the workspace above `.gwz/` is not created"
        );
        fs::create_dir(&workspace).unwrap();
        ensure_metadata_directory(&metadata).unwrap();
        ensure_metadata_directory(&metadata).unwrap();
        assert!(metadata.is_dir());
    }

    #[test]
    fn something_that_is_not_a_directory_where_the_metadata_belongs_is_an_error() {
        let temp = temp();
        let occupied = temp.path().join(".gwz");
        publish(&occupied, b"not a directory").unwrap();
        let error = ensure_metadata_directory(&occupied).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{error:?}");
        assert_eq!(
            fs::read(&occupied).unwrap(),
            b"not a directory",
            "and it is retained, not replaced"
        );
    }

    #[test]
    fn states_distinguish_absent_regular_and_irregular_without_following() {
        let temp = temp();
        let file = temp.path().join("family-root");
        assert_eq!(file_state(&file).unwrap(), FileState::Absent);
        publish(&file, b"x").unwrap();
        assert_eq!(file_state(&file).unwrap(), FileState::Regular);
        let directory = temp.path().join("as-a-directory");
        fs::create_dir(&directory).unwrap();
        assert_eq!(file_state(&directory).unwrap(), FileState::Irregular);
        assert!(FileState::Irregular.is_present());
        assert!(!FileState::Absent.is_present());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_a_metadata_path_is_irregular_and_never_followed() {
        let temp = temp();
        let real = temp.path().join("elsewhere");
        publish(&real, b"x").unwrap();
        let link = temp.path().join("family-root");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(file_state(&link).unwrap(), FileState::Irregular);
    }

    #[test]
    fn removal_is_repeatable() {
        let temp = temp();
        let file = temp.path().join("family-root");
        publish(&file, b"x").unwrap();
        assert!(remove(&file).unwrap());
        assert!(!remove(&file).unwrap());
    }

    #[test]
    fn the_encoded_size_is_the_size_on_disk() {
        let temp = temp();
        let file = temp.path().join("local-family.yml");
        publish(&file, b"12345").unwrap();
        assert_eq!(encoded_size(&file).unwrap(), 5);
        assert_eq!(read(&file).unwrap(), b"12345");
    }
}
