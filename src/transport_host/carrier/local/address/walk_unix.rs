//! The walk of a socket path, Linux and macOS (parked server design §3 "The walk, on Linux and macOS" and "The walk and
//! the connect").
//!
//! The path is walked from `/` one component at a time. Each component is probed before the next is looked up, and
//! refused when it is an automount trigger, an automounter's mount or a network file system: looking up any name inside
//! an automounter's directory asks the automounter, whatever the flags, so the order matters. No probe follows a link or
//! triggers a mount, and a refused component's descriptor is never used for the next lookup.
//!
//! **Links** are read, not followed. A link's target is walked from the link's directory (or from `/` when it is
//! absolute) under the same rule; the walk holds the handle of every directory it walked, and the walked path holds no
//! link, so a `..` is taken when the walk reaches it, as the kernel's lookup takes it: it returns to the walked parent,
//! and is never cancelled against a component not yet walked (`d/../t` walks `d`, which may itself be a link). A walk
//! reads at most [`LINK_LIMIT`] links in all, and the next one refuses the address. Links are walked rather than refused
//! because `/var`, `/tmp`, `/etc` and `/home` are links on macOS.
//!
//! **A missing component** ends the walk without a refusal ([`Place::Ended`]): nothing answers there yet, and the walk
//! gives no connect or bind. A component that is not a directory, with path after it, refuses ([`Kind::NotDirectory`]) as
//! the kernel's `ENOTDIR` does: the path after it can be a name, a link target's `.` or `..`, or the `/` that ends a link
//! target (read as a `.` after its last component). An empty link target names nothing (the kernel's `ENOENT`). A path
//! that resolves to a directory through a final `.` or `..` names no socket and is refused.
//!
//! **The socket's directory** is judged ([`Probe::socket_directory`]) once the last component's lookup has given
//! something other than a link: a lookup inside a directory that passed the mount checks triggers nothing, and a link
//! there moves the socket to another directory.
//!
//! **What the walk holds** ([`Walked`], whose fields only [`walk_with`] sets): the walked directory of its last lookup
//! ([`Walked::directory`], for the operations relative to it that server design §4 names), the name looked up, that
//! component's own handle, and whether the final name was reached through a link. [`Walked::place`] is a path for rows
//! and messages only, never a connect or bind path: it can be longer than the address (`/var` reads `/private/var` on
//! macOS) and a component can be swapped after the walk. A connect or bind goes only through the walk, which borrows
//! what it holds for the whole call, so the descriptor a path names cannot be closed or reused meanwhile:
//! [`Walked::connect`] (one non-blocking attempt, as `local::unix::connect_once`, for a caller that retries a full accept
//! queue) and [`Walked::bind`], private to this module: the carrier reaches them only through a [`Walk`]. A connect
//! needs the walk to have ended holding the address's final component; a bind needs the final name's directory walked
//! and the final name not a link (Linux's own `bind` refuses one with `EADDRINUSE`; macOS's follows it, and the walk
//! refuses it there too, so that the socket is always the address's own entry); anything else is refused
//! ([`Kind::Unwalked`], [`Kind::NoSocket`], [`Kind::Directory`], [`Kind::FinalLink`]), so no walk gives a target that
//! the kernel's lookup of the address would not reach.
//! - Linux: `/proc/self/fd/<n>` of the socket's own `O_PATH` descriptor, and `/proc/self/fd/<n>/<name>` of the walked
//!   directory, which reach the walked inodes without resolving the path again; without procfs at `/proc` both refuse.
//!   The bind route spends 15 bytes and the digits of `n` before the name, so in a short directory a last component of
//!   more than about 90 bytes passes the grammar's 107 and is refused at bind ([`Kind::TooLong`] names both lengths).
//!   Linux records that bind path as the socket's address: a listener's `local_addr()` and its clients' `peer_addr()`
//!   read `/proc/self/fd/<n>/<name>`, a path that names another directory, or nothing, once the walk is dropped. It is
//!   never an address of record: [`Walk::listen`]'s caller reports [`Walked::address`], and the socket is removed
//!   through the walked directory ([`BoundSocket`]), never by a path (2.3e round 3, State S3-1).
//! - macOS: there is no `connectat`, so both go by the parsed address (which fits `sun_path`), under [`NoTrigger`], so
//!   that a component swapped meanwhile cannot make the process trigger a mount (the design's residual is a swap that
//!   moves the connect or bind elsewhere; [`Walk::listen`] then finds no socket of its own in the walked directory and
//!   refuses, and sets no mode by path).
//!
//! **The process's walk** ([`walk`]) takes the process's [`NoTriggerGate`], made once by the process owner and lent to
//! every walk, and gives a [`Walk`], which holds a [`NoTrigger`] from before the walk until it is dropped: the carrier's
//! connect (`local::unix::connect_stream`) and bind (`local::unix::LocalListener::bind`) take a `Walk`, a type only
//! [`walk`] builds, and drop it when their connect or bind returns (plan 2.3e2). On Linux the walk triggers no mount
//! (`O_PATH | O_NOFOLLOW`, `AT_NO_AUTOMOUNT`, the connect and bind through `/proc`), so its gate holds nothing.
//! `SocketCoreBridge`, inside an application's process, holds no trigger (design §3); its walk is plan step 3.2's.
//!
//! **The probes** ([`Probe`]; the host's is chosen below by platform, the rows pass their own):
//! - Linux: `openat(parent, name, O_PATH | O_NOFOLLOW | O_CLOEXEC)` of one component, which stops at a trigger without
//!   mounting it; `statx(AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT)` on it, refused when it carries
//!   `STATX_ATTR_AUTOMOUNT` (the kernel's own automount points: NFS, AFS, SMB DFS, FUSE submounts); `fstatfs` on it,
//!   refused when the type is in [`linux_refused_type`]'s table (only this catches autofs); a link on procfs is refused,
//!   any other is read with `readlinkat(fd, "")`.
//! - macOS: `getattrlistat(parent, name, FSOPT_NOFOLLOW)` first (a get-attributes lookup, which never mounts), judged by
//!   [`judge_probe`]: a trigger is refused, a directory whose mount status is not returned is refused, only a link is
//!   read (`readlinkat`), and only a directory that is not a trigger is opened (`openat(O_SEARCH | O_NOFOLLOW)`:
//!   `O_RDONLY` gets `EPERM` on auto_home's root); [`judge_opened`] then requires its `fgetattrlist` to show the
//!   file-system and file IDs the probe saw, and judges its mount (`fstatfs`'s flags and type name) by
//!   [`macos_refused_mount`]; the socket's own directory is refused on a mount that ignores ownership.
//! - Linux, links: a link that `fs.protected_symlinks` keeps the kernel from following (in a sticky directory others
//!   may write, owned by neither this user nor the directory's owner: [`protected_link`]) is refused, whatever the
//!   setting, as the kernel's lookup with the usual setting would refuse it (2.3e round 3, Code N3-3).
//!
//! A probe that cannot complete refuses ([`Kind::Unreadable`]).
use super::{super::unix::connect_once, Platform, SERVER_ADDRESS_REFUSED, SHOWN_ADDRESS, escape};
use rustix::fs::{AtFlags, FileType};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    fmt, io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    os::unix::{
        ffi::OsStrExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
};

mod judge;
pub(crate) use judge::*;

/// The most links a walk reads, links inside targets included (the kernels' own limits are 40 on Linux and 32 on macOS).
pub(crate) const LINK_LIMIT: usize = 8;

/// What a probe saw of one component.
#[derive(Debug)]
pub(crate) enum Seen<D, L> {
    Missing,
    /// A symbolic link, and its target.
    Link(Vec<u8>),
    /// A directory the walk may look the next component up in.
    Directory(D),
    /// Anything else (a socket, a file), and what the probe holds of it.
    Other(L),
}

/// What a walk holds of the final component, when it exists.
#[derive(Debug)]
pub(crate) enum Held<L> {
    /// A directory: it names no socket.
    Directory,
    /// A socket, or any other file: connecting to it is the kernel's to judge.
    Other(L),
}

/// Why a component is refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// Linux: `STATX_ATTR_AUTOMOUNT`.
    Automount,
    /// macOS: `DIR_MNTSTATUS_TRIGGER`.
    Trigger,
    /// A listed file-system type (its name, and on Linux its magic number).
    FileSystem(String),
    /// Linux: a link on procfs, whose text need not name what the kernel resolves.
    ProcLink,
    /// macOS: a mount that is not `MNT_LOCAL`.
    NotLocal,
    /// macOS: a mount that is `MNT_AUTOMOUNTED`.
    Automounted,
    /// macOS: the socket's directory is on a mount with `MNT_IGNORE_OWNERSHIP`, where owner checks mean nothing.
    IgnoresOwnership,
    /// macOS: the directory opened is not the one the probe saw.
    Changed,
    /// More than [`LINK_LIMIT`] links.
    TooManyLinks,
    /// The path resolves to a directory through a final `.` or `..`, or to `/`: it names no socket.
    Directory,
    /// The address goes on below a component that is not a directory (the kernel's `ENOTDIR`).
    NotDirectory,
    /// Linux: a link `fs.protected_symlinks` keeps the kernel from following ([`protected_link`]).
    ProtectedLink,
    /// A bind was asked of a walk whose final name is a link: the socket would not be the address's own entry.
    FinalLink,
    /// A connect or a bind was asked of a walk that ended before the socket's own name.
    Unwalked,
    /// A connect was asked of a walk whose last name is missing: nothing answers there.
    NoSocket,
    /// Linux: `/proc` is not procfs, so no `/proc/self/fd` path reaches the walked inode and nothing connects or binds.
    NoProc,
    /// The connect or bind path is longer than `sun_path` holds.
    TooLong { length: usize, limit: usize },
    /// A probe call failed, so the component could not be judged.
    Unreadable {
        call: &'static str,
        errno: Option<i32>,
    },
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Automount => f.write_str("an automount point"),
            Kind::Trigger => f.write_str("an automount trigger"),
            Kind::FileSystem(name) => write!(f, "on a file system of type {name}"),
            Kind::ProcLink => f.write_str("a link on procfs"),
            Kind::NotLocal => f.write_str("on a mount that is not local"),
            Kind::Automounted => f.write_str("on an automounted mount"),
            Kind::IgnoresOwnership => f.write_str("on a mount that ignores ownership"),
            Kind::Changed => f.write_str("not the directory the probe saw"),
            Kind::TooManyLinks => write!(f, "a link beyond the {LINK_LIMIT} a walk reads"),
            Kind::Directory => f.write_str("a directory, not a socket name"),
            Kind::NotDirectory => f.write_str("not a directory, and the address goes on below it"),
            Kind::ProtectedLink => {
                f.write_str("a link the kernel does not follow (fs.protected_symlinks)")
            }
            Kind::FinalLink => {
                f.write_str("a link, and a socket is bound at the address's own name only")
            }
            Kind::Unwalked => f.write_str("missing, so the socket's directory was not walked"),
            Kind::NoSocket => f.write_str("missing, so nothing answers there"),
            Kind::NoProc => f.write_str("not reachable by descriptor: /proc is not procfs"),
            Kind::TooLong { length, limit } => {
                write!(
                    f,
                    "reached by a path of {length} bytes, and sun_path holds {limit}"
                )
            }
            Kind::Unreadable { call, errno } => match errno {
                Some(errno) => write!(f, "not readable ({call} failed: errno {errno})"),
                None => write!(f, "not readable ({call} failed)"),
            },
        }
    }
}

/// A walk's refusal: the walked path up to and including the refused component, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WalkRefusal {
    pub(crate) component: Vec<u8>,
    pub(crate) kind: Kind,
}

impl WalkRefusal {
    pub(crate) fn code(&self) -> u64 {
        SERVER_ADDRESS_REFUSED
    }
}

impl fmt::Display for WalkRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the server address is refused: \"{}\" is {}",
            escape(&self.component, SHOWN_ADDRESS),
            self.kind
        )
    }
}

impl std::error::Error for WalkRefusal {}

/// Where a walk ended, as a path: for rows and messages only, never a connect or bind path (module documentation).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    /// The last component exists, at this walked path, which holds no link.
    Reached(Vec<u8>),
    /// A component is missing (or is a link with an empty target): the walked path to it.
    Ended(Vec<u8>),
}

/// A finished walk, holding what it judged. Its fields are private and only [`walk_with`] sets them, so no connect or
/// bind goes to what the walk did not judge (2.3e round 3, Code N3-2).
#[derive(Debug)]
pub(crate) struct Walked<D, L> {
    /// The address walked, as parsed.
    address: Vec<u8>,
    place: Place,
    /// The walked directory the last lookup was made in.
    parent: D,
    /// The name looked up there.
    name: Vec<u8>,
    /// `name` is the socket's own name: `parent` is the socket's directory, judged by [`Probe::socket_directory`].
    last: bool,
    /// The address's final name is a link: `name` was reached through it.
    final_link: bool,
    /// What the walk holds of `name` itself, when it exists.
    held: Option<Held<L>>,
}

/// Why a connect or a bind through a walk did not happen.
#[derive(Debug)]
pub(crate) enum UseError {
    /// The walk did not hold what the call needs: nothing was connected or bound.
    Refused(WalkRefusal),
    /// The operating system's answer to the connect or bind.
    Io(io::Error),
}

impl From<WalkRefusal> for UseError {
    fn from(refusal: WalkRefusal) -> Self {
        UseError::Refused(refusal)
    }
}

impl fmt::Display for UseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UseError::Refused(refusal) => refusal.fmt(f),
            UseError::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for UseError {}

impl<D, L> Walked<D, L> {
    /// The address walked, as parsed: the address of record, in messages and in a listener's address.
    pub(crate) fn address(&self) -> &[u8] {
        &self.address
    }

    /// Where the walk ended (module documentation: for rows and messages only).
    pub(crate) fn place(&self) -> &Place {
        &self.place
    }

    /// The name the last lookup looked up.
    pub(crate) fn name(&self) -> &[u8] {
        &self.name
    }

    /// The name is the socket's own name, in the socket's directory.
    pub(crate) fn last(&self) -> bool {
        self.last
    }

    /// What the walk holds of the name, when it exists.
    pub(crate) fn held(&self) -> Option<&Held<L>> {
        self.held.as_ref()
    }

    /// The address's final name is a link (a bind refuses it, [`Kind::FinalLink`]).
    pub(crate) fn final_link(&self) -> bool {
        self.final_link
    }

    /// A refusal naming where the walk ended.
    fn refusal(&self, kind: Kind) -> WalkRefusal {
        let (Place::Reached(component) | Place::Ended(component)) = &self.place;
        WalkRefusal {
            component: component.clone(),
            kind,
        }
    }

    /// A bind needs the socket's own directory walked: refused when the walk ended before the socket's name.
    fn unwalked(&self) -> Result<(), WalkRefusal> {
        if self.last {
            return Ok(());
        }
        Err(self.refusal(Kind::Unwalked))
    }

    /// A bind needs the socket's own directory walked and the address's own final name, not a link to another.
    fn bindable(&self) -> Result<(), WalkRefusal> {
        self.unwalked()?;
        if self.final_link {
            return Err(self.refusal(Kind::FinalLink));
        }
        Ok(())
    }

    /// A connect needs the walk to have ended holding the address's final component, which is not a directory: the
    /// handle of the socket, or a refusal. No other walk (a missing component, a missing name, a directory) gives a
    /// connect target.
    fn socket(&self) -> Result<&L, WalkRefusal> {
        self.unwalked()?;
        match &self.held {
            Some(Held::Other(leaf)) => Ok(leaf),
            Some(Held::Directory) => Err(self.refusal(Kind::Directory)),
            None => Err(self.refusal(Kind::NoSocket)),
        }
    }

    /// `path`, a connect or bind path, checked against `sun_path`.
    fn sized(&self, path: Vec<u8>) -> Result<Vec<u8>, WalkRefusal> {
        let limit = HOST.socket_path_max().unwrap_or(0);
        if path.len() > limit {
            return Err(WalkRefusal {
                component: self.address.clone(),
                kind: Kind::TooLong {
                    length: path.len(),
                    limit,
                },
            });
        }
        Ok(path)
    }

    /// One non-blocking connect to `path`, which names what the walk holds.
    fn connect_by(&self, path: Vec<u8>) -> Result<UnixStream, UseError> {
        let path = self.sized(path)?;
        connect_once(Path::new(OsStr::from_bytes(&path))).map_err(UseError::Io)
    }

    /// A bind to `path`, which names what the walk holds.
    fn bind_by(&self, path: Vec<u8>) -> Result<UnixListener, UseError> {
        let path = self.sized(path)?;
        UnixListener::bind(OsStr::from_bytes(&path)).map_err(UseError::Io)
    }
}

impl<D: AsFd, L> Walked<D, L> {
    /// The walked directory the last lookup was made in (the socket's, once [`Self::last`]): the descriptor the operations
    /// relative to it use (server design §4: the lock, the log, the stale test, the removal; 2.3e round 3, State S3-3).
    pub(crate) fn directory(&self) -> BorrowedFd<'_> {
        self.parent.as_fd()
    }
}

/// One probe of the file system, for [`walk_with`].
pub(crate) trait Probe {
    /// A directory the walk holds.
    type Dir;
    /// What the probe holds of a component that is neither a directory nor a link.
    type Leaf;
    /// `/`.
    fn root(&mut self) -> Result<Self::Dir, Kind>;
    /// The component `name` of `dir`, judged.
    fn look(&mut self, dir: &Self::Dir, name: &[u8]) -> Result<Seen<Self::Dir, Self::Leaf>, Kind>;
    /// `dir` is the directory the socket sits in.
    fn socket_directory(&mut self, dir: &Self::Dir) -> Result<(), Kind>;
}

/// A walk of this host's file system made under the process's trigger hold, which it keeps until it is dropped: the one
/// input of the carrier's connect and bind (module documentation). Only [`walk`] makes one, and it gives no other
/// connect or bind than its own ([`Walk::connect`], [`Walk::listen`]): the walk's raw operations are private to this
/// module (2.3e2 round 1, Code F2 / State F4).
#[derive(Debug)]
pub(crate) struct Walk<'g> {
    walked: HostWalked,
    _hold: NoTrigger<'g>,
}

/// Walks the socket path `path` (one that passed the grammar) with this host's probe, holding a [`NoTrigger`] from
/// `gate`, the process's, from before the walk.
pub(crate) fn walk<'g>(path: &[u8], gate: &'g NoTriggerGate) -> Result<Walk<'g>, WalkRefusal> {
    let hold = gate.hold().map_err(|error| WalkRefusal {
        component: b"/".to_vec(),
        kind: unreadable("NoTrigger", &error),
    })?;
    let walked = walk_with(path, &mut HostProbe)?;
    Ok(Walk {
        walked,
        _hold: hold,
    })
}

fn euid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// What a bind through a [`Walk`] leaves its listener: the walked directory, the socket's name in it and the socket
/// file's identity as bound. The socket is removed through it ([`Self::remove`]), never by a path.
#[derive(Debug)]
pub(crate) struct BoundSocket {
    dir: OwnedFd,
    name: Vec<u8>,
    id: (libc::dev_t, libc::ino_t),
}

impl BoundSocket {
    /// The entry `name` in `dir`, which must be a socket this user owns: anything else is not the bind's.
    fn judged(dir: OwnedFd, name: Vec<u8>) -> Option<Self> {
        let stat =
            rustix::fs::statat(&dir, OsStr::from_bytes(&name), AtFlags::SYMLINK_NOFOLLOW).ok()?;
        let socket =
            FileType::from_raw_mode(stat.st_mode) == FileType::Socket && stat.st_uid == euid();
        socket.then_some(Self {
            dir,
            name,
            id: (stat.st_dev, stat.st_ino),
        })
    }

    /// The socket file's identity as bound: device and inode.
    pub(crate) fn id(&self) -> (libc::dev_t, libc::ino_t) {
        self.id
    }

    /// The entry is still the socket bound.
    fn is_bound(&self, stat: &rustix::fs::Stat) -> bool {
        FileType::from_raw_mode(stat.st_mode) == FileType::Socket
            && (stat.st_dev, stat.st_ino) == self.id
    }

    /// Removes the socket while its name in the walked directory is still the one bound; anything bound there since is
    /// left.
    pub(crate) fn remove(&self) {
        let name = OsStr::from_bytes(&self.name);
        if let Ok(stat) = rustix::fs::statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW)
            && self.is_bound(&stat)
        {
            let _ = rustix::fs::unlinkat(&self.dir, name, AtFlags::empty());
        }
    }
}

impl Walk<'_> {
    /// The address walked, as parsed ([`Walked::address`]).
    pub(crate) fn address(&self) -> &[u8] {
        self.walked.address()
    }

    /// Where the walk ended ([`Walked::place`]: for rows and messages only).
    pub(crate) fn place(&self) -> &Place {
        self.walked.place()
    }

    /// The address's final name is a link ([`Walked::final_link`]).
    pub(crate) fn final_link(&self) -> bool {
        self.walked.final_link()
    }

    /// The walk reached the socket's own directory ([`Walked::last`]).
    pub(crate) fn last(&self) -> bool {
        self.walked.last()
    }

    /// The walked directory the last lookup was made in ([`Walked::directory`]).
    pub(crate) fn directory(&self) -> BorrowedFd<'_> {
        self.walked.directory()
    }

    /// The socket file the walk holds ([`Walked::socket_stat`]): a walk that did not end holding the address's final
    /// component, which is not a directory, is refused (`Unwalked`, `NoSocket`, `Directory`).
    pub(crate) fn socket_stat(&self) -> Result<rustix::fs::Stat, UseError> {
        self.walked.socket_stat()
    }

    /// One non-blocking connect to the socket the walk holds ([`Walked::connect`]), under the walk's hold.
    pub(crate) fn connect(&self) -> Result<UnixStream, UseError> {
        self.walked.connect()
    }

    /// Binds the socket's name in the walked directory ([`Walked::bind`]) and sets its `mode` through that directory's
    /// descriptor, never by path; the hold ends when this returns. A bind that left no socket of this user's at the name
    /// in the walked directory (macOS: a component swapped after the walk moved the bind) is refused
    /// ([`Kind::Changed`]), and no mode is set anywhere; a mode that cannot be set removes the socket.
    pub(crate) fn listen(
        self,
        mode: rustix::fs::Mode,
    ) -> Result<(UnixListener, BoundSocket), UseError> {
        let listener = self.walked.bind()?;
        let changed = self.walked.refusal(Kind::Changed);
        let Walk { walked, _hold } = self;
        let (dir, name) = walked.into_directory();
        let bound = BoundSocket::judged(dir, name).ok_or(changed)?;
        if let Err(error) = set_mode(&bound, mode) {
            bound.remove();
            return Err(error);
        }
        Ok((listener, bound))
    }
}

/// `/` and `parts` joined.
fn joined(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut path = Vec::new();
    for part in parts {
        path.push(b'/');
        path.extend_from_slice(part);
    }
    if path.is_empty() {
        path.push(b'/');
    }
    path
}

/// The components of `path`, `.` and `..` included: they are taken when the walk reaches them.
fn components(path: &[u8]) -> impl DoubleEndedIterator<Item = Vec<u8>> + '_ {
    path.split(|&byte| byte == b'/')
        .filter(|part| !part.is_empty())
        .map(<[u8]>::to_vec)
}

/// Walks `path` with `probe` (module documentation).
pub(crate) fn walk_with<P: Probe>(
    path: &[u8],
    probe: &mut P,
) -> Result<Walked<P::Dir, P::Leaf>, WalkRefusal> {
    let root = probe.root().map_err(|kind| WalkRefusal {
        component: b"/".to_vec(),
        kind,
    })?;
    // `dirs[i]` is the handle of the directory `walked[..=i]` names: as many as `walked` holds, below `root`.
    let mut dirs: Vec<P::Dir> = Vec::new();
    let mut walked: Vec<Vec<u8>> = Vec::new();
    let mut pending: VecDeque<Vec<u8>> = components(path).collect();
    let mut links = 0;
    let mut final_link = false;
    while let Some(name) = pending.pop_front() {
        if name == b"." {
            continue;
        }
        if name == b".." {
            // The walked path holds no link, so its parent is the kernel's `..`; `/..` is `/`.
            walked.pop();
            dirs.pop();
            continue;
        }
        let last = pending.is_empty();
        walked.push(name.clone());
        let here = joined(&walked);
        let seen = probe
            .look(dirs.last().unwrap_or(&root), &name)
            .map_err(|kind| WalkRefusal {
                component: here.clone(),
                kind,
            })?;
        // A link with no target names nothing (the kernel's `ENOENT`); it is not a `.`.
        let seen = match seen {
            Seen::Link(target) if target.is_empty() => Seen::Missing,
            seen => seen,
        };
        let (place, held) = match seen {
            Seen::Link(target) => {
                final_link |= last;
                links += 1;
                if links > LINK_LIMIT {
                    return Err(WalkRefusal {
                        component: here,
                        kind: Kind::TooManyLinks,
                    });
                }
                walked.pop();
                if target.starts_with(b"/") {
                    walked.clear();
                    dirs.clear();
                }
                // A target's `/` at its end requires a directory, as a `.` after its last component does.
                let mut parts: Vec<Vec<u8>> = components(&target).collect();
                if target.ends_with(b"/") {
                    parts.push(b".".to_vec());
                }
                for part in parts.into_iter().rev() {
                    pending.push_front(part);
                }
                continue;
            }
            Seen::Directory(next) if !last => {
                dirs.push(next);
                continue;
            }
            Seen::Other(_) if !last => {
                return Err(WalkRefusal {
                    component: here,
                    kind: Kind::NotDirectory,
                });
            }
            Seen::Missing => (Place::Ended(here), None),
            Seen::Directory(_) => (Place::Reached(here), Some(Held::Directory)),
            Seen::Other(leaf) => (Place::Reached(here), Some(Held::Other(leaf))),
        };
        let parent = dirs.pop().unwrap_or(root);
        if last {
            walked.pop();
            probe
                .socket_directory(&parent)
                .map_err(|kind| WalkRefusal {
                    component: joined(&walked),
                    kind,
                })?;
        }
        return Ok(Walked {
            address: path.to_vec(),
            place,
            parent,
            name,
            last,
            final_link,
            held,
        });
    }
    Err(WalkRefusal {
        component: joined(&walked),
        kind: Kind::Directory,
    })
}

/// The probe of this host's file system: private to the walk, so that only [`walk`], under a gate, walks the host
/// (rows: `walk_host`).
struct HostProbe;

fn unreadable(call: &'static str, error: &std::io::Error) -> Kind {
    Kind::Unreadable {
        call,
        errno: error.raw_os_error(),
    }
}

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod linux;
        pub(crate) use linux::*;
    } else if #[cfg(target_vendor = "apple")] {
        mod macos;
        mod no_trigger;
        pub(crate) use macos::*;
        pub(crate) use no_trigger::{NoTrigger, NoTriggerGate};
    } else {
        compile_error!("the socket-path walk has a probe for Linux and macOS only (parked server design §3)");
    }
}

// The rows' reach into the walk: the host's walk without a gate or a hold, and the walk's own connect and bind, which
// production code reaches only through a `Walk`.
cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// The host's walk of `path` without a trigger hold: what the walk judged and holds.
        pub(crate) fn walk_host(path: &[u8]) -> Result<HostWalked, WalkRefusal> {
            walk_with(path, &mut HostProbe)
        }

        /// The walk's own connect and bind ([`Walked::connect`], [`Walked::bind`]), for the rows.
        pub(crate) trait RowUse {
            fn connect(&self) -> Result<UnixStream, UseError>;
            fn bind(&self) -> Result<UnixListener, UseError>;
        }

        impl RowUse for HostWalked {
            fn connect(&self) -> Result<UnixStream, UseError> {
                HostWalked::connect(self)
            }

            fn bind(&self) -> Result<UnixListener, UseError> {
                HostWalked::bind(self)
            }
        }
    }
}
