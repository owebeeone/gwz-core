//! The Linux probe and the connect and bind through `/proc/self/fd` (movement-only split of `walk_unix.rs`, step 2.3e2;
//! the module documentation of [`super`] says what they do).
use super::*;
use rustix::fs::{Mode, OFlags, StatxAttributes, StatxFlags};
use std::{marker::PhantomData, os::fd::AsRawFd};

/// The grammar this host applies.
pub(crate) const HOST: Platform = Platform::Linux;

/// A walk with this host's probe.
pub(crate) type HostWalked = Walked<OwnedFd, OwnedFd>;

/// Linux: the walk triggers no mount (`O_PATH | O_NOFOLLOW`, `AT_NO_AUTOMOUNT`, the connect and bind through
/// `/proc`), so the process's gate holds nothing and counts nothing; a walk takes one, as on macOS.
#[derive(Debug)]
pub(crate) struct NoTriggerGate(());

impl NoTriggerGate {
    /// The process's gate, made once by the process owner, as on macOS.
    pub(crate) fn new() -> Self {
        NoTriggerGate(())
    }

    pub(crate) fn hold(&self) -> io::Result<NoTrigger<'_>> {
        Ok(NoTrigger(PhantomData))
    }
}

/// Linux: nothing to hold (see [`NoTriggerGate`]).
#[derive(Debug)]
pub(crate) struct NoTrigger<'gate>(PhantomData<&'gate NoTriggerGate>);

/// `<proc>/self/fd/<fd>`, once `<proc>/self/fd` is shown to be on procfs.
fn proc_fd(proc: &[u8], fd: &OwnedFd) -> Option<Vec<u8>> {
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
    on_proc.then(|| {
        table.extend_from_slice(format!("/{}", fd.as_raw_fd()).as_bytes());
        table
    })
}

/// Sets the bound socket's mode through the walked directory: its entry opened `O_PATH | O_NOFOLLOW` there, shown
/// to be the socket bound, and changed through `/proc/self/fd/<n>` of that descriptor (`fchmod` refuses an
/// `O_PATH` descriptor, and `fchmodat` follows a final link).
pub(super) fn set_mode(bound: &BoundSocket, mode: Mode) -> Result<(), UseError> {
    let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let os = |error: rustix::io::Errno| UseError::Io(error.into());
    let fd = rustix::fs::openat(
        &bound.dir,
        OsStr::from_bytes(&bound.name),
        flags,
        Mode::empty(),
    )
    .map_err(os)?;
    if !bound.is_bound(&rustix::fs::fstat(&fd).map_err(os)?) {
        return Err(UseError::Io(io::Error::from(io::ErrorKind::NotFound)));
    }
    let path = proc_fd(b"/proc", &fd)
        .ok_or_else(|| UseError::Io(io::Error::from(io::ErrorKind::Unsupported)))?;
    rustix::fs::chmod(OsStr::from_bytes(&path), mode).map_err(os)
}

fn failed(call: &'static str, error: rustix::io::Errno) -> Kind {
    unreadable(call, &error.into())
}

impl Probe for HostProbe {
    type Dir = OwnedFd;
    type Leaf = OwnedFd;

    fn root(&mut self) -> Result<OwnedFd, Kind> {
        rustix::fs::open(
            "/",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
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
        let wanted = StatxFlags::TYPE | StatxFlags::MODE | StatxFlags::UID;
        let stat =
            rustix::fs::statx(&fd, "", at, wanted).map_err(|error| failed("statx", error))?;
        if stat.stx_attributes.contains(StatxAttributes::AUTOMOUNT) {
            return Err(Kind::Automount);
        }
        // `f_type` is a signed word on some targets; the magic numbers are its low 32 bits.
        let f_type = rustix::fs::fstatfs(&fd)
            .map_err(|error| failed("fstatfs", error))?
            .f_type as u32;
        if let Some(type_name) = linux_refused_type(f_type) {
            return Err(Kind::FileSystem(format!("{type_name} (0x{f_type:x})")));
        }
        match FileType::from_raw_mode(stat.stx_mode.into()) {
            FileType::Symlink => {
                if f_type == PROC_SUPER_MAGIC {
                    return Err(Kind::ProcLink);
                }
                let parent = rustix::fs::fstat(dir).map_err(|error| failed("fstat", error))?;
                if protected_link(parent.st_mode, parent.st_uid, stat.stx_uid, euid()) {
                    return Err(Kind::ProtectedLink);
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
    pub(super) fn connect(&self) -> Result<UnixStream, UseError> {
        self.connect_in(b"/proc")
    }

    /// [`Self::connect`] with procfs expected at `proc`.
    pub(super) fn connect_in(&self, proc: &[u8]) -> Result<UnixStream, UseError> {
        let path = self.through(proc, self.socket()?, None)?;
        self.connect_by(path)
    }

    /// Binds the socket's name in the walked directory, through `/proc/self/fd/<n>/<name>` of its descriptor.
    /// Refused when the walk ended before the socket's own name, when that name is a link, and when `/proc` is
    /// not procfs. [`Walk::listen`] sets the socket's mode; the listener's own address is the descriptor path
    /// (module documentation).
    pub(super) fn bind(&self) -> Result<UnixListener, UseError> {
        self.bind_in(b"/proc")
    }

    /// [`Self::bind`] with procfs expected at `proc`.
    pub(super) fn bind_in(&self, proc: &[u8]) -> Result<UnixListener, UseError> {
        self.bindable()?;
        let path = self.through(proc, &self.parent, Some(&self.name))?;
        self.bind_by(path)
    }

    /// `<proc>/self/fd/<fd>[/<name>]`, once `<proc>/self/fd` is shown to be on procfs.
    fn through(
        &self,
        proc: &[u8],
        fd: &OwnedFd,
        name: Option<&[u8]>,
    ) -> Result<Vec<u8>, WalkRefusal> {
        let Some(mut path) = proc_fd(proc, fd) else {
            return Err(WalkRefusal {
                component: self.address.clone(),
                kind: Kind::NoProc,
            });
        };
        if let Some(name) = name {
            path.push(b'/');
            path.extend_from_slice(name);
        }
        Ok(path)
    }

    /// The socket file the walk holds, from its own descriptor (refused as [`Self::connect`] is).
    pub(crate) fn socket_stat(&self) -> Result<rustix::fs::Stat, UseError> {
        rustix::fs::fstat(self.socket()?).map_err(|error| UseError::Io(error.into()))
    }

    /// The walked directory and the name looked up in it.
    pub(super) fn into_directory(self) -> (OwnedFd, Vec<u8>) {
        (self.parent, self.name)
    }
}

// The rows' reach into the descriptor route with procfs expected elsewhere (`walk_unix`'s `RowUse` holds the rest).
cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// [`Walked::connect_in`] and [`Walked::bind_in`], for the rows.
        pub(crate) trait RowProc {
            fn connect_in(&self, proc: &[u8]) -> Result<UnixStream, UseError>;
            fn bind_in(&self, proc: &[u8]) -> Result<UnixListener, UseError>;
        }

        impl RowProc for HostWalked {
            fn connect_in(&self, proc: &[u8]) -> Result<UnixStream, UseError> {
                HostWalked::connect_in(self, proc)
            }

            fn bind_in(&self, proc: &[u8]) -> Result<UnixListener, UseError> {
                HostWalked::bind_in(self, proc)
            }
        }
    }
}
