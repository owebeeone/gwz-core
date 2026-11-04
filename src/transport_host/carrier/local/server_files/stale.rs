//! The stale-socket test (Linux, macOS; parked server design §4 "A stale socket", split doc :240) and the link check
//! (every platform; split doc :241).
//!
//! **Stale socket.** Holding the lock, a start removes the entry at its socket's name, and binds, only when all three
//! hold: (1) the entry is a socket this user owns with no group or other bit, and not a link; (2) a connection to it,
//! made without blocking and within [`STALE_CONNECT_BOUND`], with nothing sent, is refused (`ECONNREFUSED`); (3) the
//! record the previous holder left names exactly this socket's address. A refused connection alone never shows a socket
//! stale: macOS refuses a connection to a full accept queue just as it refuses one to a dead socket. Anything else
//! refuses the start and deletes nothing. A Windows pipe disappears with its server, so nothing is left over there.
//!
//! **Link check.** A running server calls [`check_links`] every [`LINK_CHECK_PERIOD`]: the directory's path must still
//! name the directory it opened (a directory renamed away and made again leaves the old one reachable only through the
//! server's descriptor), and its socket (Linux, macOS), its lock file and its identity lock file must still be linked at their names and be the
//! files it created. A system cleaner, logind removing `/run/user/<uid>`, or a user's `rm` or `mv` makes it report which
//! one went, so the server can stop instead of holding a lock no published address reaches while a second start takes a
//! new one; a check that could not be made is reported apart (the loop that calls it is step 2.3i's).
use super::super::LocalRefusal;
use super::lock::{FileId, FilesDir};
use std::{fmt, io, time::Duration};

/// How long the stale test's connection may take (server design §3's 5 seconds).
pub(crate) const STALE_CONNECT_BOUND: Duration = Duration::from_secs(5);
/// How often a running server checks its links.
pub(crate) const LINK_CHECK_PERIOD: Duration = Duration::from_secs(30);

/// Why an entry at the socket's name was not removed. Nothing was deleted.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum StaleRefusal {
    /// The entry is not a socket this user owns with no group or other bit (or it is a link, or it changed while it
    /// was judged).
    NotOurs,
    /// A connection to it succeeded: something listens there. The process ID is the listener's, from the peer
    /// credentials.
    Answering {
        pid: Option<u32>,
    },
    /// The connection did not complete within its bound (a full accept queue on Linux).
    Timeout,
    /// The connection was refused, but the lock file's record does not name this socket: gwz did not make it, as far as
    /// the record shows. Remove it if nothing uses it.
    Unrecorded,
    Local(LocalRefusal),
}

impl fmt::Display for StaleRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StaleRefusal::NotOurs => f.write_str(
                "it is not a socket of this user with mode 0600 (or it changed while it was checked)",
            ),
            StaleRefusal::Answering { pid: Some(pid) } => write!(f, "process {pid} answers on it"),
            StaleRefusal::Answering { pid: None } => f.write_str("a process answers on it"),
            StaleRefusal::Timeout => f.write_str(
                "a connection to it did not complete in time, so a busy server may be listening there",
            ),
            StaleRefusal::Unrecorded => f.write_str(
                "nothing answers on it, but the lock file's record does not name it, so gwz did not make it as far as \
                 the record shows; remove it if nothing uses it",
            ),
            StaleRefusal::Local(refusal) => write!(f, "it cannot be checked: {refusal}"),
        }
    }
}

/// What the link check found gone, or that it could not look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkLost {
    /// The directory's path no longer names the directory the server opened: it was renamed, removed or replaced.
    Directory,
    /// The socket is no longer linked at its name, or another file is.
    Socket,
    /// The lock file is no longer linked at its name, or another file is.
    Lock,
    /// The identity lock file (`id-<key>.lock`) is no longer linked at its name, or another file is: a second server
    /// for this directory could start.
    IdentityLock,
    /// The check could not be made (the system refused a `stat` or an open); nothing is known to be gone.
    Unchecked(io::ErrorKind),
}

/// The directory part of the link check, every platform.
fn check_directory(dir: &FilesDir) -> Result<(), LinkLost> {
    match dir.still_published() {
        Ok(true) => Ok(()),
        Ok(false) => Err(LinkLost::Directory),
        Err(error) => Err(LinkLost::Unchecked(error.kind())),
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::super::unix::{connect_until, peer_of, socket_path};
        use super::lock::is_own_socket;
        use super::record::Record;
        use std::{os::fd::AsRawFd, time::Instant};

        /// The stale-socket test on the entry `name` in `dir`, removing it when it passes; `Ok` too when nothing is
        /// there. `previous` is the record the lock file held when the lock was taken.
        pub(crate) fn clear_stale(
            dir: &FilesDir,
            name: &str,
            previous: &Record,
            bound: Duration,
        ) -> Result<(), StaleRefusal> {
            let os = |call, error: std::io::Error| StaleRefusal::Local(LocalRefusal::os(call, &error));
            let Some((judged, stat)) = dir.entry(name).map_err(|error| os("fstatat", error))? else {
                return Ok(());
            };
            if !is_own_socket(&stat) {
                return Err(StaleRefusal::NotOurs);
            }
            let address = socket_path(dir.path(), name).map_err(StaleRefusal::Local)?;
            match connect_until(&address, Instant::now() + bound) {
                Ok(stream) => {
                    let pid = peer_of(stream.as_raw_fd()).ok().and_then(|peer| peer.pid);
                    return Err(StaleRefusal::Answering { pid });
                }
                Err(LocalRefusal::Os { code: Some(libc::ECONNREFUSED), .. }) => {}
                Err(LocalRefusal::Timeout) => return Err(StaleRefusal::Timeout),
                Err(other) => return Err(StaleRefusal::Local(other)),
            }
            if !previous.names(&address) {
                return Err(StaleRefusal::Unrecorded);
            }
            // Removed only while it is still the entry judged.
            match dir.entry(name).map_err(|error| os("fstatat", error))? {
                Some((now, _)) if now == judged => dir.remove(name).map_err(|error| os("unlinkat", error)),
                _ => Err(StaleRefusal::NotOurs),
            }
        }

        /// Whether the entry `name` in `dir` is still the file `id`; `lost` when it is not.
        fn still(dir: &FilesDir, name: &str, id: FileId, lost: LinkLost) -> Result<(), LinkLost> {
            match dir.entry(name) {
                Ok(Some((now, _))) if now == id => Ok(()),
                Ok(_) => Err(lost),
                Err(error) => Err(LinkLost::Unchecked(error.kind())),
            }
        }

        /// The link check: `dir`'s path still names the directory opened, and the socket `(name, id)`, when there is
        /// one, the lock file `(name, id)` and the identity lock file `(name, id)` are still linked in it and still
        /// the files created.
        pub(crate) fn check_links(
            dir: &FilesDir,
            socket: Option<(&str, FileId)>,
            lock: (&str, FileId),
            identity: (&str, FileId),
        ) -> Result<(), LinkLost> {
            check_directory(dir)?;
            if let Some((name, id)) = socket {
                still(dir, name, id, LinkLost::Socket)?;
            }
            still(dir, lock.0, lock.1, LinkLost::Lock)?;
            still(dir, identity.0, identity.1, LinkLost::IdentityLock)
        }
    } else if #[cfg(windows)] {
        use super::lock::{Use, file_id, open_file};

        use super::lock::FileFault;

        /// Whether the lock file `name` in `dir` is still the file `id`; `lost` when it is not. An open the system
        /// refused for another reason (a backup tool's exclusive open) is `Unchecked`, not lost.
        fn still(dir: &FilesDir, name: &str, id: FileId, lost: LinkLost) -> Result<(), LinkLost> {
            let file = match open_file(dir, name, Use::Read) {
                Ok(file) => file,
                Err(FileFault::Os { kind, .. }) => return Err(LinkLost::Unchecked(kind)),
                Err(_) => return Err(lost),
            };
            match file_id(&file) {
                Ok(now) if now == id => Ok(()),
                Ok(_) => Err(lost),
                Err(error) => Err(LinkLost::Unchecked(error.kind())),
            }
        }

        /// The link check on Windows: `dir`'s path still opens the directory opened; there is no socket file; the
        /// lock file `(name, id)` and the identity lock file `(name, id)` must still be linked in `dir` and still be
        /// the files created.
        pub(crate) fn check_links(
            dir: &FilesDir,
            _socket: Option<(&str, FileId)>,
            lock: (&str, FileId),
            identity: (&str, FileId),
        ) -> Result<(), LinkLost> {
            check_directory(dir)?;
            still(dir, lock.0, lock.1, LinkLost::Lock)?;
            still(dir, identity.0, identity.1, LinkLost::IdentityLock)
        }
    }
}
