//! The server's listener files (step 2.3f of `dev-docs/GwzServerCore-Step2.3-Plan.md`; split doc §1 and :229-241;
//! parked server design §4 and §11): the `ws-<key>` names, the lock file and its record, the stale-socket test, the
//! clean exit order and the link check.
//!
//! | Step | What happens | Where |
//! | --- | --- | --- |
//! | Keys | `ws-<key>`: the first 16 hex digits of SHA-256 of the canonical root path (an address names one workspace path); `id-<key>`: the same of the root's filesystem identity (one server per directory, whatever its spelling) | [`key`] |
//! | Directory | the per-user directory ([`RuntimeDir`]) opened once and checked through its descriptor | [`lock`] |
//! | Identity lock | `id-<key>.lock` opened under the file rule and locked exclusively without waiting, before the workspace lock and held for the server's whole life; a held lock refuses the start with the holder's record ([`FilesRefusal::Held`]), which names the first server's address, so a second spelling of one directory cannot start a second server | [`claim`] |
//! | Lock | `ws-<key>.lock`, the same rule; a held lock refuses the start with the holder's record | [`claim`] |
//! | Previous record | read once the lock is held, for the stale test | [`Claim::previous`] |
//! | Stale socket (Linux, macOS) | the three-part test on `ws-<key>`; anything but a pass refuses and deletes nothing | [`stale`] |
//! | Record, then bind (Linux, macOS) | the record (pid, address, root, log) written through both locks' descriptors, then the socket bound at `ws-<key>`, in [`record_then_bind`]'s one order: its bind step takes the [`Recorded`] proof that only a record write makes | [`Claim::bind`] |
//! | Pipe, then record (Windows) | the pipe created under a fresh random name (`FILE_FLAG_FIRST_PIPE_INSTANCE`), then recorded in both, so no reader sees a name that does not exist | [`Claim::bind`] |
//! | Log | `ws-<key>.log` beside the lock, or the path given, opened append-only under the file rule | [`Claim::open_log`] |
//! | Link check | every 30 s: the directory's path still names the directory opened, the socket and both lock files still linked and still the files created | [`ServerListener::check_links`] |
//! | Clean exit | the socket removed (only while it is still the one bound), then the workspace record cleared and lock unlocked and closed, then the identity record cleared and the identity lock unlocked and closed last (the reverse of the order taken), in [`exit_in_order`]'s one order, on `close` and on every drop; the lock files stay | [`ServerListener::close`] |
//!
//! A lock on Unix belongs to the open file description, and a child forked (or `vfork`ed, or `posix_spawn`ed) by
//! another thread while the lock is held keeps a copy of the descriptor until it executes. The clean exit therefore
//! releases each lock explicitly before it closes the file (`flock(LOCK_UN)`, which releases it for every copy;
//! `UnlockFileEx` on Windows), so a child in its fork-to-exec window does not keep a lock the server gave up: a start
//! after a clean exit succeeds, and nothing is left to answer, because the socket file was removed first. Two
//! releases do not unlock and so keep the window: a crash (the process is gone and the record still names it), and a
//! [`Claim`] dropped before it bound. In either, a child forked while the lock was held keeps it until the child
//! executes or exits; a start in that window is refused as held ([`FilesRefusal::Held`]), with [`Record::None`] when the
//! record was cleared and with the dead server's complete record after a crash, and the refusal's text says to start
//! again once the holder has executed or exited. A process that forks and never executes (a reaper) holds every
//! inherited lock for its whole life unless it closes the descriptors first; that obligation belongs to the step that
//! forks such a process (2.3r-2).
//!
//! A start the stale test refuses writes no record. The record is a claim only: any process of the user may write the
//! lock file, so its reader treats it as data, and a message that quotes it says it comes from the lock file.
mod key;
mod lock;
mod record;
mod stale;

pub(crate) use key::{IdentityKey, Keys, ServerKey};
pub(crate) use lock::{FileFault, FilesDir, Recorded, Use, write_record};
pub(crate) use record::{Record, ServerRecord};
pub(crate) use stale::{LINK_CHECK_PERIOD, LinkLost, STALE_CONNECT_BOUND, StaleRefusal};

use super::{
    Accepted, CarrierLimits, LocalAddress, LocalListener, LocalRefusal, PeerPolicy, RuntimeDir,
};
use lock::{FileId, clear_record, file_id, open_named, read_record, release, try_lock};
use record::NotRecordable;
use std::{
    fmt,
    fs::File,
    io,
    mem::ManuallyDrop,
    path::{Path, PathBuf},
    time::Duration,
};

/// Why a start was refused, or a file could not be opened.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum FilesRefusal {
    /// The directory failed its check, or there is none (Windows without an explicit directory).
    Directory(LocalRefusal),
    /// The workspace root has no filesystem identity, so it has no key; `kind` is why the system gave none.
    NoIdentity {
        root: PathBuf,
        kind: io::ErrorKind,
    },
    /// A file failed the file rule (`server_address_refused`): it was not followed, truncated or written.
    File {
        path: PathBuf,
        fault: FileFault,
    },
    /// Another start holds the lock; `record` is what the lock file says about it.
    Held {
        lock: PathBuf,
        record: Record,
    },
    /// The entry at the socket's name was not shown stale (`server_unavailable`); nothing was deleted.
    Stale {
        path: PathBuf,
        refusal: StaleRefusal,
    },
    /// The socket or pipe could not be bound or created.
    Bind(LocalRefusal),
    /// A path cannot be recorded.
    Record(NotRecordable),
    Os(LocalRefusal),
}

impl fmt::Display for FilesRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilesRefusal::Directory(refusal)
            | FilesRefusal::Bind(refusal)
            | FilesRefusal::Os(refusal) => {
                write!(f, "{refusal}")
            }
            FilesRefusal::NoIdentity { root, kind } => {
                write!(
                    f,
                    "the workspace root {} cannot be identified: {kind}",
                    root.display()
                )
            }
            FilesRefusal::File { path, fault } => {
                write!(f, "{} is refused by the file rule: {fault}", path.display())
            }
            FilesRefusal::Held { lock, record } => match record {
                Record::Complete(record) => write!(
                    f,
                    "another server holds {}; the lock file says process {} at {}, log {}",
                    lock.display(),
                    record.pid,
                    record.address.display(),
                    record.log.display()
                ),
                Record::Incomplete => {
                    write!(f, "another server holds {} and is starting", lock.display())
                }
                Record::None => write!(
                    f,
                    "another process holds {} and its file names no server; if a start of this workspace has just \
                     ended without binding, a child it started can still hold the lock until that child executes \
                     or exits, so start again once it has",
                    lock.display()
                ),
            },
            FilesRefusal::Stale { path, refusal } => {
                write!(f, "{} is not removed: {refusal}", path.display())
            }
            FilesRefusal::Record(NotRecordable(path)) => {
                write!(f, "{} cannot be recorded", path.display())
            }
        }
    }
}

impl std::error::Error for FilesRefusal {}

/// A start that holds both locks and has passed the stale test, before it binds.
pub(crate) struct Claim {
    dir: FilesDir,
    keys: Keys,
    /// `ws-<key>.lock`.
    lock: File,
    /// `id-<key>.lock`, taken before `lock`.
    identity_lock: File,
    log: PathBuf,
    previous: Record,
}

/// Takes the locks of the workspace at `root` in `runtime`, reads the previous record and, on Linux and macOS, clears
/// a stale socket; `log` is the log's path, `ws-<key>.log` beside the lock when `None`.
///
/// # Errors
///
/// See [`FilesRefusal`].
pub(crate) fn claim(
    runtime: &RuntimeDir,
    root: &Path,
    log: Option<&Path>,
) -> Result<Claim, FilesRefusal> {
    claim_within(runtime, root, log, STALE_CONNECT_BOUND)
}

/// [`claim`], the stale test's connection bounded by `bound`.
pub(crate) fn claim_within(
    runtime: &RuntimeDir,
    root: &Path,
    log: Option<&Path>,
    bound: Duration,
) -> Result<Claim, FilesRefusal> {
    let keys = Keys::of_root(root).map_err(|error| FilesRefusal::NoIdentity {
        root: root.to_path_buf(),
        kind: error.kind(),
    })?;
    claim_keyed(runtime, keys, log, bound)
}

/// [`claim_within`] for `keys` already read: the one body of a start's claim, and the seam a row uses to give two
/// directories one identity (a bind mount) or one reused identity to a new root, without a real mount or a real
/// inode-reuse race.
///
/// The identity lock is taken first, so a second server for one directory through another spelling (another
/// `ws-<key>`) refuses as held and names the first server; the workspace lock second, so a server for the same
/// canonical path whose directory was replaced (another identity) refuses too.
pub(crate) fn claim_keyed(
    runtime: &RuntimeDir,
    keys: Keys,
    log: Option<&Path>,
    bound: Duration,
) -> Result<Claim, FilesRefusal> {
    let dir = FilesDir::open(files_path(runtime)?).map_err(FilesRefusal::Directory)?;
    let identity_lock = lock_named(&dir, &keys.identity.lock_name())?;
    let lock = lock_named(&dir, &keys.server.lock_name())?;
    let previous = read_record(&lock)
        .map_err(|error| FilesRefusal::Os(LocalRefusal::os("read the record", &error)))?;
    clear_stale_socket(&dir, &keys.server, &previous, bound)?;
    let log = log.map_or_else(
        || dir.path().join(keys.server.log_name()),
        Path::to_path_buf,
    );
    Ok(Claim {
        dir,
        keys,
        lock,
        identity_lock,
        log,
        previous,
    })
}

/// Opens the lock file `name` in `dir` under the file rule and takes its exclusive lock without waiting; a held lock
/// refuses with the holder's record.
fn lock_named(dir: &FilesDir, name: &str) -> Result<File, FilesRefusal> {
    let file = open_named(dir, name, Use::Lock)?;
    let os = |call, error: io::Error| FilesRefusal::Os(LocalRefusal::os(call, &error));
    if !try_lock(&file).map_err(|error| os("lock", error))? {
        let record = read_record(&file).map_err(|error| os("read the record", error))?;
        return Err(FilesRefusal::Held {
            lock: dir.path().join(name),
            record,
        });
    }
    Ok(file)
}

/// Reads the record of the lock file `name` in the directory at `dir`, under the file rule, without locking: what a
/// starter that found the lock held, `server list` and a client read.
///
/// # Errors
///
/// The directory's or the file's refusal, or the read's error.
pub(crate) fn read_lock_record(dir: &Path, name: &str) -> Result<Record, FilesRefusal> {
    let dir = FilesDir::open(dir).map_err(FilesRefusal::Directory)?;
    let file = open_named(&dir, name, Use::Read)?;
    read_record(&file)
        .map_err(|error| FilesRefusal::Os(LocalRefusal::os("read the record", &error)))
}

impl Claim {
    /// The record the lock file held when the lock was taken.
    pub(crate) fn previous(&self) -> &Record {
        &self.previous
    }

    pub(crate) fn key(&self) -> &ServerKey {
        &self.keys.server
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.dir.path().join(self.keys.server.lock_name())
    }

    pub(crate) fn identity_lock_path(&self) -> PathBuf {
        self.dir.path().join(self.keys.identity.lock_name())
    }

    /// Opens the log, appended to only, under the file rule: `ws-<key>.log` in the held directory, or the path given,
    /// opened in its own directory. That directory is the caller's to walk (`--log` is walked as an address is, step
    /// 2.3e's walk at 2.3i's call site); the file rule applies to the file, not the directory's mode.
    ///
    /// # Errors
    ///
    /// The file rule's refusal.
    pub(crate) fn open_log(&self) -> Result<File, FilesRefusal> {
        let name = self
            .log
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| FilesRefusal::File {
                path: self.log.clone(),
                fault: FileFault::NotRegular,
            })?;
        if self.log.parent() == Some(self.dir.path()) {
            return open_named(&self.dir, name, Use::Log);
        }
        let parent = self.log.parent().unwrap_or(Path::new("."));
        let dir = FilesDir::open_given(parent).map_err(FilesRefusal::Directory)?;
        open_named(&dir, name, Use::Log)
    }

    /// Writes this server's record into both locks, the workspace lock first: the proof that a record names `address`.
    /// A write that fails clears what was written.
    ///
    /// # Errors
    ///
    /// The record's refusal.
    fn record(&self, address: PathBuf) -> Result<Recorded, FilesRefusal> {
        let record = ServerRecord {
            pid: std::process::id(),
            address,
            root: self.keys.canonical.clone(),
            log: self.log.clone(),
        };
        write_record(&self.lock, &record)?;
        write_record(&self.identity_lock, &record).inspect_err(|_| self.unrecord())
    }

    /// Clears both records (a bind that failed after them).
    fn unrecord(&self) {
        let _ = clear_record(&self.lock);
        let _ = clear_record(&self.identity_lock);
    }

    /// The bound server: `listener` with this claim's locks and records.
    fn into_server(
        self,
        listener: LocalListener,
        socket: Option<FileId>,
    ) -> Result<ServerListener, FilesRefusal> {
        let os = |error: io::Error| FilesRefusal::Os(LocalRefusal::os("fstat", &error));
        let lock_id = file_id(&self.lock).map_err(os)?;
        let identity_lock_id = file_id(&self.identity_lock).map_err(os)?;
        let held = Held {
            dir: self.dir,
            keys: self.keys,
            lock: WorkspaceLock(self.lock),
            lock_id,
            identity_lock: IdentityLock(self.identity_lock),
            identity_lock_id,
            socket,
        };
        Ok(ServerListener {
            listener: ManuallyDrop::new(listener),
            held: ManuallyDrop::new(held),
        })
    }
}

/// `ws-<key>.lock`, held.
struct WorkspaceLock(File);

/// `id-<key>.lock`, held.
struct IdentityLock(File);

/// The locks and the records a bound server holds. The clean exit clears each record, unlocks the lock and then closes
/// its file ([`exit_in_order`], [`release`]).
struct Held {
    dir: FilesDir,
    keys: Keys,
    lock: WorkspaceLock,
    lock_id: FileId,
    identity_lock: IdentityLock,
    identity_lock_id: FileId,
    socket: Option<FileId>,
}

/// The clean exit's one order (server design §4, split doc §4.2): `socket` dropped (its file removed while it is still
/// the one bound), then `clear_workspace` run on the workspace lock's holder (it clears the record and unlocks, see
/// [`release`]) and that lock dropped, then `clear_identity` run on the identity lock's holder and that lock dropped
/// last: the reverse of the order the locks were taken. The five arguments have different types at the one call site, so a swap there does not compile, and the
/// rows pin this body's order.
pub(crate) fn exit_in_order<S, W, I>(
    socket: S,
    clear_workspace: impl FnOnce(&W),
    workspace_lock: W,
    clear_identity: impl FnOnce(&I),
    identity_lock: I,
) {
    drop(socket);
    clear_workspace(&workspace_lock);
    drop(workspace_lock);
    clear_identity(&identity_lock);
    drop(identity_lock);
}

/// A server's listener with the files that make it the workspace's one server. Its drop, and so [`Self::close`], is
/// the clean exit ([`exit_in_order`]).
pub(crate) struct ServerListener {
    listener: ManuallyDrop<LocalListener>,
    held: ManuallyDrop<Held>,
}

impl Drop for ServerListener {
    fn drop(&mut self) {
        // SAFETY: each field is taken once, here, and the struct is not used after its drop.
        let (listener, held) = unsafe {
            (
                ManuallyDrop::take(&mut self.listener),
                ManuallyDrop::take(&mut self.held),
            )
        };
        let Held {
            lock,
            identity_lock,
            dir,
            ..
        } = held;
        exit_in_order(
            listener,
            |lock: &WorkspaceLock| release(&lock.0),
            lock,
            |lock: &IdentityLock| release(&lock.0),
            identity_lock,
        );
        drop(dir);
    }
}

impl ServerListener {
    pub(crate) fn address(&self) -> &LocalAddress {
        self.listener.address()
    }

    pub(crate) fn accept(&self) -> Accepted {
        self.listener.accept()
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.held.dir.path().join(self.held.keys.server.lock_name())
    }

    pub(crate) fn identity_lock_path(&self) -> PathBuf {
        self.held
            .dir
            .path()
            .join(self.held.keys.identity.lock_name())
    }

    /// The link check ([`LINK_CHECK_PERIOD`]): the directory's path still names the directory opened, and the socket
    /// (Linux, macOS), the lock file and the identity lock file are still linked and still the files this server
    /// created.
    ///
    /// # Errors
    ///
    /// Which of them went, or [`LinkLost::Unchecked`] when the check could not be made.
    pub(crate) fn check_links(&self) -> Result<(), LinkLost> {
        let socket_name = self.held.keys.server.socket_name();
        let lock_name = self.held.keys.server.lock_name();
        let identity_name = self.held.keys.identity.lock_name();
        stale::check_links(
            &self.held.dir,
            self.held.socket.map(|id| (socket_name.as_str(), id)),
            (&lock_name, self.held.lock_id),
            (&identity_name, self.held.identity_lock_id),
        )
    }

    /// The clean exit: the socket removed (while it is still the one bound), then the workspace record cleared and its
    /// lock unlocked and closed, then the identity record cleared and its lock unlocked and closed. The lock files
    /// stay.
    pub(crate) fn close(self) {
        drop(self);
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::address::{Kind, NoTriggerGate, WalkRefusal, walk};

        fn files_path(runtime: &RuntimeDir) -> Result<&Path, FilesRefusal> {
            Ok(runtime.path())
        }

        fn clear_stale_socket(dir: &FilesDir, key: &ServerKey, previous: &Record, bound: Duration) -> Result<(), FilesRefusal> {
            let name = key.socket_name();
            stale::clear_stale(dir, &name, previous, bound).map_err(|refusal| FilesRefusal::Stale {
                path: dir.path().join(&name),
                refusal,
            })
        }

        /// The start's one order on Linux and macOS (server design §4, plan 2.3f): `record` first, then `bind`, which
        /// takes the [`Recorded`] proof `record` returns; a bind that fails runs `unrecord` (its listener, if any, has
        /// dropped and removed its socket by then). Every socket gwz binds therefore has a record naming it before it
        /// exists, so a crash after the bind leaves a socket the next start recovers. Only a record write makes a
        /// [`Recorded`], so a call site cannot skip the record (`|| Ok(())` does not compile) or write it inside the
        /// bind step after binding (the bind step has no record to write: it has the proof, not the means), and the
        /// rows pin this body's order.
        ///
        /// # Errors
        ///
        /// The record's refusal (nothing bound) or the bind's (the record cleared).
        pub(crate) fn record_then_bind<T>(
            record: impl FnOnce() -> Result<Recorded, FilesRefusal>,
            bind: impl FnOnce(Recorded) -> Result<T, FilesRefusal>,
            unrecord: impl FnOnce(),
        ) -> Result<T, FilesRefusal> {
            let recorded = record()?;
            bind(recorded).inspect_err(|_| unrecord())
        }

        impl Claim {
            /// Writes this server's records, then walks `ws-<key>` under `gate`, the process's, and binds the socket
            /// through the walk ([`record_then_bind`]; plan 2.3e2: the server binds only through the walk, holding
            /// `NoTrigger` from before the walk until the bind returns).
            ///
            /// # Errors
            ///
            /// The record's, the walk's or the bind's refusal.
            pub(crate) fn bind(
                self,
                gate: &NoTriggerGate,
                policy: PeerPolicy,
                limits: CarrierLimits,
            ) -> Result<ServerListener, FilesRefusal> {
                let name = self.keys.server.socket_name();
                let address = super::unix::socket_path(self.dir.path(), &name).map_err(FilesRefusal::Bind)?;
                let (listener, socket) = record_then_bind(
                    || self.record(address.clone()),
                    |recorded| self.bind_recorded(&recorded, &address, &name, gate, policy, limits),
                    || self.unrecord(),
                )?;
                self.into_server(listener, Some(socket))
            }

            /// Walks `address`, the socket `name` in the directory, and binds it through the walk; `_recorded` is the
            /// proof that the records naming it are written. The walked directory must be the claimed one (both
            /// descriptors name one directory), and the entry at `name` there after the bind must be the listener's own
            /// socket; anything else is refused before or after the bind (2.3e2 round 1, State F3 / Code F3).
            fn bind_recorded(
                &self,
                _recorded: &Recorded,
                address: &Path,
                name: &str,
                gate: &NoTriggerGate,
                policy: PeerPolicy,
                limits: CarrierLimits,
            ) -> Result<(LocalListener, FileId), FilesRefusal> {
                use std::os::unix::ffi::OsStrExt;
                let walk = walk(address.as_os_str().as_bytes(), gate).map_err(|refusal| {
                    FilesRefusal::Bind(LocalRefusal::Address { unavailable: false, text: refusal.to_string() })
                })?;
                let claimed = self.dir.entry(".").map_err(|error| FilesRefusal::Os(LocalRefusal::os("fstatat", &error)))?;
                let walked = rustix::fs::fstat(walk.directory())
                    .map_err(|error| FilesRefusal::Os(LocalRefusal::os("fstat", &error.into())))?;
                if claimed.is_none_or(|(_, stat)| (stat.st_dev, stat.st_ino) != (walked.st_dev, walked.st_ino)) {
                    // The claimed directory's path reaches another directory now (renamed or replaced since the claim).
                    let changed = WalkRefusal { component: self.dir.path().as_os_str().as_bytes().to_vec(), kind: Kind::Changed };
                    return Err(FilesRefusal::Bind(LocalRefusal::Address { unavailable: false, text: changed.to_string() }));
                }
                let listener = LocalListener::bind(walk, policy, limits).map_err(FilesRefusal::Bind)?;
                match self.dir.entry(name) {
                    Ok(Some((id, stat))) if (stat.st_dev, stat.st_ino) == listener.bound_id() => Ok((listener, id)),
                    Ok(_) => Err(FilesRefusal::Bind(LocalRefusal::NotOurSocket)),
                    Err(error) => Err(FilesRefusal::Os(LocalRefusal::os("fstatat", &error))),
                }
            }
        }
    } else if #[cfg(windows)] {
        fn files_path(runtime: &RuntimeDir) -> Result<&Path, FilesRefusal> {
            runtime
                .path()
                .ok_or(FilesRefusal::Directory(LocalRefusal::Directory(super::DirectoryFault::Missing)))
        }

        /// A pipe disappears with its server: there is no stale socket on Windows.
        fn clear_stale_socket(_: &FilesDir, _: &ServerKey, _: &Record, _: Duration) -> Result<(), FilesRefusal> {
            Ok(())
        }

        impl Claim {
            /// Creates the pipe under a fresh random name, then records it in both locks: no reader of a lock file sees a
            /// pipe name before the pipe exists. A record that fails closes the pipe.
            ///
            /// # Errors
            ///
            /// The pipe's or the record's refusal.
            pub(crate) fn bind(self, policy: PeerPolicy, limits: CarrierLimits) -> Result<ServerListener, FilesRefusal> {
                let runtime = RuntimeDir::at(self.dir.path()).map_err(FilesRefusal::Directory)?;
                let listener = LocalListener::bind(&runtime, policy, limits).map_err(FilesRefusal::Bind)?;
                let address = PathBuf::from(listener.address().pipe_name());
                self.record(address)?;
                self.into_server(listener, None)
            }
        }
    }
}
