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
//! **What the walk holds** ([`Walked`]): the walked directory of its last lookup, the name looked up and that
//! component's own handle. [`Walked::place`] is a path for rows and messages only, never a connect or bind path: it can
//! be longer than the address (`/var` reads `/private/var` on macOS) and a component can be swapped after the walk. A
//! connect or bind goes only through the walk, which borrows what it holds for the whole call, so the descriptor a
//! path names cannot be closed or reused meanwhile: [`Walked::connect`] (one non-blocking attempt, as
//! `local::unix::connect_once`, for a caller that retries a full accept queue) and [`Walked::bind`]. A connect needs the
//! walk to have ended holding the address's final component; a bind needs the final name's directory walked; anything
//! else is refused ([`Kind::Unwalked`], [`Kind::NoSocket`], [`Kind::Directory`]), so no walk gives a target that the
//! kernel's lookup of the address would not reach.
//! - Linux: `/proc/self/fd/<n>` of the socket's own `O_PATH` descriptor, and `/proc/self/fd/<n>/<name>` of the walked
//!   directory, which reach the walked inodes without resolving the path again; without procfs at `/proc` both refuse.
//!   The bind route spends 15 bytes and the digits of `n` before the name, so in a short directory a last component of
//!   more than about 90 bytes passes the grammar's 107 and is refused at bind ([`Kind::TooLong`] names both lengths).
//! - macOS: there is no `connectat`, so both go by the parsed address (which fits `sun_path`). A process that is its
//!   own holds [`NoTrigger`] from before the walk until `connect` or `bind` returns, so that a component swapped
//!   meanwhile cannot make it trigger a mount: the CLI around `Walked::connect` (plan 2.3k) and the serving CLI around
//!   `Walked::bind` (plan 2.3i), never `SocketCoreBridge`, inside an application's process, which keeps the design's
//!   residual.
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
//!
//! A probe that cannot complete refuses ([`Kind::Unreadable`]).
use super::{super::unix::connect_once, Platform, SERVER_ADDRESS_REFUSED, SHOWN_ADDRESS, escape};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    fmt, io,
    os::unix::{
        ffi::OsStrExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
};

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

/// A finished walk, holding what it judged.
#[derive(Debug)]
pub(crate) struct Walked<D, L> {
    /// The address walked, as parsed.
    pub(crate) address: Vec<u8>,
    pub(crate) place: Place,
    /// The walked directory the last lookup was made in.
    pub(crate) parent: D,
    /// The name looked up there.
    pub(crate) name: Vec<u8>,
    /// `name` is the socket's own name: `parent` is the socket's directory, judged by [`Probe::socket_directory`].
    pub(crate) last: bool,
    /// What the walk holds of `name` itself, when it exists.
    pub(crate) held: Option<Held<L>>,
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

/// A walk with this host's probe.
pub(crate) type HostWalked = Walked<<HostProbe as Probe>::Dir, <HostProbe as Probe>::Leaf>;

/// Walks the socket path `path` (one that passed the grammar) with this host's probe.
pub(crate) fn walk(path: &[u8]) -> Result<HostWalked, WalkRefusal> {
    walk_with(path, &mut HostProbe)
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
            held,
        });
    }
    Err(WalkRefusal {
        component: joined(&walked),
        kind: Kind::Directory,
    })
}

/// Linux `PROC_SUPER_MAGIC`.
pub(crate) const PROC_SUPER_MAGIC: u32 = 0x9FA0;

/// The Linux file-system types a walk refuses (`f_type`, magic.h v7.2 and coreutils' out-of-tree list): every
/// automounter, network and cluster file system, and every FUSE file system, whose subtype whoever mounts it chooses.
const LINUX_REFUSED: [(u32, &str); 25] = [
    (0x0187, "autofs"),
    (0x6969, "nfs"),
    (0x517B, "smbfs"),
    (0xFF53_4D42, "cifs"),
    (0xFE53_4D42, "smb2"),
    (0x5346_414F, "openafs"),
    (0x6B41_4653, "kafs"),
    (0x00C3_6400, "ceph"),
    (0x0102_1997, "9p"),
    (0x7375_7245, "coda"),
    (0x564C, "ncp"),
    (0x7461_636F, "ocfs2"),
    (0x0116_1970, "gfs2"),
    (0x2003_0528, "orangefs"),
    (0x786F_4256, "vboxsf"),
    (0x6573_5546, "fuse"),
    (0x0BD0_0BD0, "lustre"),
    (0x4750_4653, "gpfs"),
    (0x1983_0326, "beegfs"),
    (0xAAD7_AAEA, "panfs"),
    (0x0131_11A8, "ibrix"),
    (0xBEEF_DEAD, "stornext"),
    (0x6163_6673, "acfs"),
    (0x7C7C_6673, "prl_fs"),
    (0xBACB_ACBC, "vmhgfs"),
];

/// The name of a refused Linux file-system type, or `None` for one a socket may sit on.
pub(crate) fn linux_refused_type(f_type: u32) -> Option<&'static str> {
    LINUX_REFUSED
        .iter()
        .find(|(magic, _)| *magic == f_type)
        .map(|(_, name)| *name)
}

/// macOS `MNT_LOCAL`, `MNT_IGNORE_OWNERSHIP` and `MNT_AUTOMOUNTED` (`<sys/mount.h>`), so the table is judged on any host.
pub(crate) const MNT_LOCAL: u32 = 0x0000_1000;
pub(crate) const MNT_IGNORE_OWNERSHIP: u32 = 0x0020_0000;
pub(crate) const MNT_AUTOMOUNTED: u32 = 0x0040_0000;
/// macOS `DIR_MNTSTATUS_TRIGGER` (`<sys/attr.h>`) and the vnode types `VDIR` and `VLNK` (`<sys/vnode.h>`), so the
/// probe's decisions are judged on any host.
pub(crate) const DIR_MNTSTATUS_TRIGGER: u32 = 0x0000_0002;
pub(crate) const VDIR: u32 = 2;
pub(crate) const VLNK: u32 = 5;

/// The macOS file-system type names a walk refuses, and the prefixes macFUSE's `fstypename=NAME` gives. The names are
/// checked as well as the flags because macFUSE's `-o local` sets `MNT_LOCAL`.
const MACOS_REFUSED: [&[u8]; 12] = [
    b"autofs",
    b"nfs",
    b"smbfs",
    b"afpfs",
    b"webdav",
    b"ftp",
    b"acfs",
    b"virtiofs",
    b"fusefs",
    b"osxfusefs",
    b"osxfuse",
    b"macfuse",
];
const MACOS_REFUSED_PREFIXES: [&[u8]; 2] = [b"macfuse_", b"osxfuse_"];

/// Why a macOS mount with `flags` and type name `name` is refused, or `None` for one a socket may sit on.
pub(crate) fn macos_refused_mount(flags: u32, name: &[u8]) -> Option<Kind> {
    if MACOS_REFUSED.contains(&name)
        || MACOS_REFUSED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
    {
        return Some(Kind::FileSystem(String::from_utf8_lossy(name).into_owned()));
    }
    if flags & MNT_LOCAL == 0 {
        return Some(Kind::NotLocal);
    }
    if flags & MNT_AUTOMOUNTED != 0 {
        return Some(Kind::Automounted);
    }
    None
}

/// What macOS's `getattrlistat` showed of one component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacAttributes {
    pub(crate) fsid: [u8; 8],
    pub(crate) fileid: u64,
    pub(crate) objtype: u32,
    /// `ATTR_DIR_MOUNTSTATUS`, when the call returned it.
    pub(crate) mount_status: Option<u32>,
}

/// What the macOS probe may do next with a component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacNext {
    /// A link: read it.
    ReadLink,
    /// Neither a link nor a directory: nothing is opened.
    Other,
    /// A directory that is not a trigger: open it.
    Open,
}

/// Design §3 macOS steps 1 to 3, before anything is opened: a trigger is refused, and so is a directory whose mount
/// status was not returned, since any `open` of a trigger mounts it.
pub(crate) fn judge_probe(seen: &MacAttributes) -> Result<MacNext, Kind> {
    if seen
        .mount_status
        .is_some_and(|status| status & DIR_MNTSTATUS_TRIGGER != 0)
    {
        return Err(Kind::Trigger);
    }
    match seen.objtype {
        VLNK => Ok(MacNext::ReadLink),
        VDIR if seen.mount_status.is_none() => Err(Kind::Unreadable {
            call: "getattrlistat",
            errno: None,
        }),
        VDIR => Ok(MacNext::Open),
        _ => Ok(MacNext::Other),
    }
}

/// Design §3 macOS step 3, after the open: the descriptor's file-system and file IDs (`opened`) must be the probe's,
/// and its mount (`flags`, `type_name`) must pass [`macos_refused_mount`].
pub(crate) fn judge_opened(
    seen: &MacAttributes,
    opened: ([u8; 8], u64),
    flags: u32,
    type_name: &[u8],
) -> Result<(), Kind> {
    if opened != (seen.fsid, seen.fileid) {
        return Err(Kind::Changed);
    }
    match macos_refused_mount(flags, type_name) {
        Some(kind) => Err(kind),
        None => Ok(()),
    }
}

/// The probe of this host's file system.
pub(crate) struct HostProbe;

fn unreadable(call: &'static str, error: &std::io::Error) -> Kind {
    Kind::Unreadable {
        call,
        errno: error.raw_os_error(),
    }
}

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        use rustix::fs::{AtFlags, FileType, Mode, OFlags, StatxAttributes, StatxFlags};
        use std::os::fd::{AsRawFd, OwnedFd};

        /// The grammar this host applies.
        pub(crate) const HOST: Platform = Platform::Linux;

        fn failed(call: &'static str, error: rustix::io::Errno) -> Kind {
            unreadable(call, &error.into())
        }

        impl Probe for HostProbe {
            type Dir = OwnedFd;
            type Leaf = OwnedFd;

            fn root(&mut self) -> Result<OwnedFd, Kind> {
                rustix::fs::open("/", OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
                    .map_err(|error| failed("open", error))
            }

            fn look(&mut self, dir: &OwnedFd, name: &[u8]) -> Result<Seen<OwnedFd, OwnedFd>, Kind> {
                let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
                let fd = match rustix::fs::openat(dir, OsStr::from_bytes(name), flags, Mode::empty()) {
                    Ok(fd) => fd,
                    Err(rustix::io::Errno::NOENT) => {
                        return Ok(Seen::Missing);
                    }
                    Err(error) => {
                        return Err(failed("openat", error));
                    }
                };
                let at = AtFlags::EMPTY_PATH | AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT;
                let stat = rustix::fs::statx(&fd, "", at, StatxFlags::TYPE).map_err(|error| failed("statx", error))?;
                if stat.stx_attributes.contains(StatxAttributes::AUTOMOUNT) {
                    return Err(Kind::Automount);
                }
                // `f_type` is a signed word on some targets; the magic numbers are its low 32 bits.
                let f_type = rustix::fs::fstatfs(&fd).map_err(|error| failed("fstatfs", error))?.f_type as u32;
                if let Some(type_name) = linux_refused_type(f_type) {
                    return Err(Kind::FileSystem(format!("{type_name} (0x{f_type:x})")));
                }
                match FileType::from_raw_mode(stat.stx_mode.into()) {
                    FileType::Symlink => {
                        if f_type == PROC_SUPER_MAGIC {
                            return Err(Kind::ProcLink);
                        }
                        let target = rustix::fs::readlinkat(&fd, "", Vec::new())
                            .map_err(|error| failed("readlinkat", error))?;
                        Ok(Seen::Link(target.into_bytes()))
                    }
                    FileType::Directory => Ok(Seen::Directory(fd)),
                    _ => Ok(Seen::Other(fd)),
                }
            }

            fn socket_directory(&mut self, _dir: &OwnedFd) -> Result<(), Kind> {
                Ok(())
            }
        }

        impl Walked<OwnedFd, OwnedFd> {
            /// One non-blocking connect to the socket the walk holds, through `/proc/self/fd/<n>` of its descriptor,
            /// which reaches that inode without resolving the address again. Refused before any connect when the walk
            /// did not end holding the final component (the module documentation) and when `/proc` is not procfs.
            pub(crate) fn connect(&self) -> Result<UnixStream, UseError> {
                self.connect_in(b"/proc")
            }

            /// [`Self::connect`] with procfs expected at `proc`.
            pub(crate) fn connect_in(&self, proc: &[u8]) -> Result<UnixStream, UseError> {
                let path = self.through(proc, self.socket()?, None)?;
                self.connect_by(path)
            }

            /// Binds the socket's name in the walked directory, through `/proc/self/fd/<n>/<name>` of its descriptor.
            /// Refused when the walk ended before the socket's own name, and when `/proc` is not procfs. The socket's
            /// mode is the caller's.
            pub(crate) fn bind(&self) -> Result<UnixListener, UseError> {
                self.bind_in(b"/proc")
            }

            /// [`Self::bind`] with procfs expected at `proc`.
            pub(crate) fn bind_in(&self, proc: &[u8]) -> Result<UnixListener, UseError> {
                self.unwalked()?;
                let path = self.through(proc, &self.parent, Some(&self.name))?;
                self.bind_by(path)
            }

            /// `<proc>/self/fd/<fd>[/<name>]`, once `<proc>/self/fd` is shown to be on procfs.
            fn through(&self, proc: &[u8], fd: &OwnedFd, name: Option<&[u8]>) -> Result<Vec<u8>, WalkRefusal> {
                let mut table = proc.to_vec();
                table.extend_from_slice(b"/self/fd");
                let opened = rustix::fs::open(
                    OsStr::from_bytes(&table),
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                );
                let on_proc = opened
                    .and_then(|fd| rustix::fs::fstatfs(&fd))
                    .is_ok_and(|stat| stat.f_type as u32 == PROC_SUPER_MAGIC);
                if !on_proc {
                    return Err(WalkRefusal { component: self.address.clone(), kind: Kind::NoProc });
                }
                let mut path = table;
                path.extend_from_slice(format!("/{}", fd.as_raw_fd()).as_bytes());
                if let Some(name) = name {
                    path.push(b'/');
                    path.extend_from_slice(name);
                }
                Ok(path)
            }
        }
    } else if #[cfg(target_vendor = "apple")] {
        use std::{
            ffi::{CStr, CString, c_char, c_int},
            os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        };

        mod no_trigger;
        pub(crate) use no_trigger::{NoTrigger, NoTriggerGate};

        /// The grammar this host applies.
        pub(crate) const HOST: Platform = Platform::Macos;

        /// A walked directory, and the flags of the mount it is on.
        #[derive(Debug)]
        pub(crate) struct MacDir {
            fd: OwnedFd,
            flags: u32,
        }

        fn word(buffer: &[u8], at: usize) -> u32 {
            u32::from_ne_bytes([buffer[at], buffer[at + 1], buffer[at + 2], buffer[at + 3]])
        }

        fn long(buffer: &[u8], at: usize) -> u64 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&buffer[at..at + 8]);
            u64::from_ne_bytes(bytes)
        }

        fn name_of(name: &[u8]) -> Result<CString, Kind> {
            CString::new(name).map_err(|_| Kind::Unreadable { call: "getattrlistat", errno: None })
        }

        /// The attributes of `name` in `dir`, never following a link or crossing a trigger. The buffer is packed in
        /// bit order, invalid attributes included (`FSOPT_PACK_INVAL_ATTRS`): its length, the returned set (20 bytes),
        /// `ATTR_CMN_FSID` (8), `ATTR_CMN_OBJTYPE` (4), `ATTR_CMN_FILEID` (8) and `ATTR_DIR_MOUNTSTATUS` (4).
        fn attributes(dir: RawFd, name: &CString) -> io::Result<MacAttributes> {
            let mut list = libc::attrlist {
                bitmapcount: libc::ATTR_BIT_MAP_COUNT,
                reserved: 0,
                commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_FSID | libc::ATTR_CMN_OBJTYPE
                    | libc::ATTR_CMN_FILEID,
                volattr: 0,
                dirattr: libc::ATTR_DIR_MOUNTSTATUS,
                fileattr: 0,
                forkattr: 0,
            };
            let mut buffer = [0u8; 64];
            let options = libc::FSOPT_NOFOLLOW | libc::FSOPT_PACK_INVAL_ATTRS;
            // SAFETY: `list` and `buffer` are valid for the call; the kernel writes at most `buffer.len()` bytes.
            let result = unsafe {
                libc::getattrlistat(dir, name.as_ptr(), (&raw mut list).cast(), buffer.as_mut_ptr().cast(), buffer.len(),
                    libc::c_ulong::from(options))
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            let common = libc::ATTR_CMN_FSID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_FILEID;
            if word(&buffer, 4) & common != common {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            let mut fsid = [0u8; 8];
            fsid.copy_from_slice(&buffer[24..32]);
            Ok(MacAttributes {
                fsid,
                objtype: word(&buffer, 32),
                fileid: long(&buffer, 36),
                mount_status: (word(&buffer, 12) & libc::ATTR_DIR_MOUNTSTATUS != 0).then(|| word(&buffer, 44)),
            })
        }

        /// The file-system and file IDs of an open descriptor (`fgetattrlist`).
        fn identity(fd: RawFd) -> io::Result<([u8; 8], u64)> {
            let mut list = libc::attrlist {
                bitmapcount: libc::ATTR_BIT_MAP_COUNT,
                reserved: 0,
                commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_FSID | libc::ATTR_CMN_FILEID,
                volattr: 0,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            };
            let mut buffer = [0u8; 64];
            // SAFETY: as in `attributes`.
            let result = unsafe {
                libc::fgetattrlist(fd, (&raw mut list).cast(), buffer.as_mut_ptr().cast(), buffer.len(),
                    libc::FSOPT_PACK_INVAL_ATTRS)
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut fsid = [0u8; 8];
            fsid.copy_from_slice(&buffer[24..32]);
            Ok((fsid, long(&buffer, 32)))
        }

        /// The mount `fd` is on: its flags and its type name. Its `f_fsid` is not compared with the probe's file-system ID:
        /// on the sealed system volume the two differ (`/System`: `f_fsid` is the data volume's), as `st_dev` does.
        fn mount_of(fd: RawFd) -> io::Result<(u32, Vec<u8>)> {
            // SAFETY: `statfs` is plain data, written by the call.
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            // SAFETY: `stat` is valid for writes.
            if unsafe { libc::fstatfs(fd, &mut stat) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let name: Vec<u8> = stat.f_fstypename.iter().take_while(|&&c| c != 0).map(|&c: &c_char| c as u8).collect();
            Ok((stat.f_flags, name))
        }

        fn open_at(dir: RawFd, name: &CStr, flags: c_int) -> io::Result<OwnedFd> {
            // SAFETY: `name` is NUL-terminated; the descriptor returned is owned by the caller.
            let fd = unsafe { libc::openat(dir, name.as_ptr(), flags) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `fd` was just opened and is owned by no one else.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }

        impl Probe for HostProbe {
            type Dir = MacDir;
            type Leaf = ();

            fn root(&mut self) -> Result<MacDir, Kind> {
                let fd = open_at(libc::AT_FDCWD, c"/", libc::O_SEARCH | libc::O_CLOEXEC)
                    .map_err(|error| unreadable("open", &error))?;
                let (flags, _) = mount_of(fd.as_raw_fd()).map_err(|error| unreadable("fstatfs", &error))?;
                Ok(MacDir { fd, flags })
            }

            fn look(&mut self, dir: &MacDir, name: &[u8]) -> Result<Seen<MacDir, ()>, Kind> {
                let name = name_of(name)?;
                let seen = match attributes(dir.fd.as_raw_fd(), &name) {
                    Ok(seen) => seen,
                    Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                        return Ok(Seen::Missing);
                    }
                    Err(error) => {
                        return Err(unreadable("getattrlistat", &error));
                    }
                };
                match judge_probe(&seen)? {
                    MacNext::ReadLink => {
                        let mut target = vec![0u8; libc::PATH_MAX as usize + 1];
                        // SAFETY: `target` is valid for `target.len()` bytes.
                        let length = unsafe {
                            libc::readlinkat(dir.fd.as_raw_fd(), name.as_ptr(), target.as_mut_ptr().cast(), target.len())
                        };
                        let Ok(length) = usize::try_from(length) else {
                            return Err(unreadable("readlinkat", &io::Error::last_os_error()));
                        };
                        if length >= target.len() {
                            return Err(Kind::Unreadable { call: "readlinkat", errno: None });
                        }
                        target.truncate(length);
                        Ok(Seen::Link(target))
                    }
                    MacNext::Other => Ok(Seen::Other(())),
                    MacNext::Open => {
                        let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                        let fd = match open_at(dir.fd.as_raw_fd(), &name, flags) {
                            Ok(fd) => fd,
                            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                                return Ok(Seen::Missing);
                            }
                            Err(error) => {
                                return Err(unreadable("openat", &error));
                            }
                        };
                        let opened = identity(fd.as_raw_fd()).map_err(|error| unreadable("fgetattrlist", &error))?;
                        let (flags, type_name) =
                            mount_of(fd.as_raw_fd()).map_err(|error| unreadable("fstatfs", &error))?;
                        judge_opened(&seen, opened, flags, &type_name)?;
                        Ok(Seen::Directory(MacDir { fd, flags }))
                    }
                }
            }

            fn socket_directory(&mut self, dir: &MacDir) -> Result<(), Kind> {
                if dir.flags & MNT_IGNORE_OWNERSHIP != 0 {
                    return Err(Kind::IgnoresOwnership);
                }
                Ok(())
            }
        }

        impl Walked<MacDir, ()> {
            /// One non-blocking connect to the parsed address, which fits `sun_path` (there is no `connectat`), once
            /// the walk has ended holding the address's final component. A process that is its own holds
            /// [`NoTrigger`] from before the walk until `connect` returns.
            pub(crate) fn connect(&self) -> Result<UnixStream, UseError> {
                self.socket()?;
                self.connect_by(self.address.clone())
            }

            /// Binds the parsed address, once the walk has judged the socket's own directory. The serving CLI holds
            /// [`NoTrigger`] from before the walk until `bind` returns. The socket's mode is the caller's.
            pub(crate) fn bind(&self) -> Result<UnixListener, UseError> {
                self.unwalked()?;
                self.bind_by(self.address.clone())
            }
        }
    } else {
        compile_error!("the socket-path walk has a probe for Linux and macOS only (parked server design §3)");
    }
}
