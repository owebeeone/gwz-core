//! The Unix domain socket carrier (macOS, Linux). See the module documentation of [`super`] for the directory, the
//! path-length rule and the peer check.
use super::super::{CarrierBinding, CarrierId, CarrierLimits, Peer, Side};
use super::{
    Accepted, DirectoryFault, LocalRefusal, PeerIdentity, PeerPolicy, Principal, Relation,
    StreamCarrier,
};
use std::{
    ffi::{CStr, OsStr},
    io::{self, Read, Write},
    mem,
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
};

/// Bytes of a socket's file name: `s-` and 16 hex digits.
pub(crate) const SOCKET_NAME_BYTES: usize = 18;
/// Bytes of the address field, and so of the path including its NUL.
const SUN_PATH_LEN: usize =
    mem::size_of::<libc::sockaddr_un>() - mem::offset_of!(libc::sockaddr_un, sun_path);
/// The longest socket path: the field less the NUL.
pub(crate) const SUN_PATH_MAX: usize = SUN_PATH_LEN - 1;

/// How many times `bind` draws a new name when the one drawn exists.
const NAME_DRAWS: usize = 3;

fn euid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The path of the socket `name` in `dir`, if it fits the address field. A longer one is refused, not shortened.
pub(crate) fn socket_path(dir: &Path, name: &str) -> Result<PathBuf, LocalRefusal> {
    let path = dir.join(name);
    let length = path.as_os_str().as_bytes().len();
    if length > SUN_PATH_MAX {
        return Err(LocalRefusal::AddressTooLong {
            length,
            limit: SUN_PATH_MAX,
        });
    }
    Ok(path)
}

pub(crate) fn own_identity() -> Result<PeerIdentity, LocalRefusal> {
    Ok(PeerIdentity {
        principal: Principal::Uid(euid()),
        pid: Some(std::process::id()),
        integrity: None,
        app_container: false,
    })
}

/// A directory only this user can enter, for sockets.
#[derive(Debug)]
pub(crate) struct RuntimeDir(PathBuf);

/// The directory's check: the leaf is a directory and not a link, owned by this user, with no group or other bit. The
/// path's parents are walked by the operating system as they are (`/var` is a link on macOS).
fn check_directory(path: &Path) -> Result<(), LocalRefusal> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => LocalRefusal::Directory(DirectoryFault::Missing),
        _ => LocalRefusal::os("lstat", &error),
    })?;
    let fault = if meta.file_type().is_symlink() {
        Some(DirectoryFault::Link)
    } else if !meta.is_dir() {
        Some(DirectoryFault::NotADirectory)
    } else if meta.uid() != euid() {
        Some(DirectoryFault::OtherOwner)
    } else if meta.mode() & 0o077 != 0 {
        Some(DirectoryFault::GroupOrOtherAccess)
    } else {
        None
    };
    fault.map_or(Ok(()), |fault| Err(LocalRefusal::Directory(fault)))
}

/// Creates `path` with mode 0700 if nothing is there, then checks it. `mkdir` does not follow a link at the leaf.
fn create_and_check(path: &Path) -> Result<(), LocalRefusal> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(LocalRefusal::os("mkdir", &error)),
    }
    check_directory(path)
}

cfg_if::cfg_if! {
    if #[cfg(target_vendor = "apple")] {
        /// The operating system's per-user temporary directory on macOS (`confstr`), which is not `$TMPDIR`.
        fn user_parent() -> Option<PathBuf> {
            let mut buffer = vec![0u8; libc::PATH_MAX as usize];
            // SAFETY: the buffer is writable for its stated length; `confstr` NUL-terminates what it writes.
            let written = unsafe {
                libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buffer.as_mut_ptr().cast(), buffer.len())
            };
            if written == 0 || written > buffer.len() {
                return None;
            }
            let path = CStr::from_bytes_until_nul(&buffer).ok()?;
            Some(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
        }
    } else {
        /// The runtime directory systemd makes for the user, where it exists.
        fn user_parent() -> Option<PathBuf> {
            let path = PathBuf::from(format!("/run/user/{}", euid()));
            check_directory(&path).ok().map(|()| path)
        }
    }
}

impl RuntimeDir {
    /// The user's directory (the module documentation says where), created with mode 0700 on first use and checked on
    /// every use.
    pub(crate) fn for_current_user() -> Result<Self, LocalRefusal> {
        let path = match user_parent() {
            Some(parent) => parent.join("gwz"),
            None => PathBuf::from(format!("/tmp/gwz-{}", euid())),
        };
        create_and_check(&path)?;
        Ok(Self(path))
    }

    /// An explicit directory, which must already exist and pass the check. It is never created.
    pub(crate) fn at(path: &Path) -> Result<Self, LocalRefusal> {
        check_directory(path)?;
        Ok(Self(path.to_path_buf()))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

/// Where a listener listens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalAddress(PathBuf);

impl LocalAddress {
    /// An address given explicitly. It is checked when it is connected to, never trusted before.
    pub(crate) fn from_path(path: &Path) -> Self {
        Self(path.to_path_buf())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for LocalAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

/// A stream half that shuts the socket down when it is dropped, so that the peer reads the stream's end even while
/// another clone of the socket is alive.
struct Half(UnixStream);

impl Read for Half {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for Half {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Drop for Half {
    fn drop(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        /// The credentials of the process on the other end of a connected socket (`SO_PEERCRED`).
        fn peer_of(fd: RawFd) -> Result<PeerIdentity, LocalRefusal> {
            // SAFETY: `ucred` is plain data; `getsockopt` writes at most `length` bytes into it.
            let mut credentials: libc::ucred = unsafe { mem::zeroed() };
            let mut length = mem::size_of::<libc::ucred>() as libc::socklen_t;
            let result = unsafe {
                libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut credentials).cast(), &mut length)
            };
            if result != 0 {
                return Err(LocalRefusal::os("getsockopt(SO_PEERCRED)", &io::Error::last_os_error()));
            }
            Ok(PeerIdentity {
                principal: Principal::Uid(credentials.uid),
                pid: u32::try_from(credentials.pid).ok(),
                integrity: None,
                app_container: false,
            })
        }
    } else {
        /// The credentials of the process on the other end of a connected socket (`getpeereid`, `LOCAL_PEERPID`).
        fn peer_of(fd: RawFd) -> Result<PeerIdentity, LocalRefusal> {
            let (mut uid, mut gid) = (0 as libc::uid_t, 0 as libc::gid_t);
            // SAFETY: both out-parameters are valid for writes.
            if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
                return Err(LocalRefusal::os("getpeereid", &io::Error::last_os_error()));
            }
            let mut pid: libc::pid_t = 0;
            let mut length = mem::size_of::<libc::pid_t>() as libc::socklen_t;
            // SAFETY: `pid` is valid for `length` bytes.
            let result = unsafe {
                libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&raw mut pid).cast(), &mut length)
            };
            let pid = (result == 0).then(|| u32::try_from(pid).ok()).flatten();
            Ok(PeerIdentity {
                principal: Principal::Uid(uid),
                pid,
                integrity: None,
                app_container: false,
            })
        }
    }
}

/// Reads and judges the peer of `stream`; a refused connection is shut with nothing written.
fn check_peer(
    stream: &UnixStream,
    policy: &PeerPolicy,
    relation: Relation,
) -> Result<PeerIdentity, LocalRefusal> {
    let refuse = |refusal| {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        LocalRefusal::Peer(refusal)
    };
    let peer = peer_of(stream.as_raw_fd())
        .map_err(|_| refuse(super::PeerRefusal::Unreadable("the peer's credentials")))?;
    policy.verify(&peer, relation).map_err(refuse)?;
    Ok(peer)
}

fn carrier_over(
    stream: UnixStream,
    side: Side,
    peer: PeerIdentity,
    limits: CarrierLimits,
) -> Accepted {
    let os = |call, error: io::Error| LocalRefusal::os(call, &error);
    let reader = stream.try_clone().map_err(|e| os("dup", e))?;
    let killer = stream.try_clone().map_err(|e| os("dup", e))?;
    let id = CarrierId::random().map_err(|e| os("getrandom", e))?;
    StreamCarrier::new(
        Box::new(Half(reader)),
        Box::new(Half(stream)),
        Box::new(move || {
            let _ = killer.shutdown(std::net::Shutdown::Both);
        }),
        CarrierBinding::new(id, side, Peer::Verified(peer)),
        limits,
    )
    .map_err(|e| os("spawn", e))
}

/// A listening socket in a per-user directory, with the rule it applies to whoever connects.
pub(crate) struct LocalListener {
    listener: UnixListener,
    address: LocalAddress,
    policy: PeerPolicy,
    limits: CarrierLimits,
}

impl LocalListener {
    /// Listens on a new socket with a random name in `dir`. Nothing is created if the path would not fit.
    pub(crate) fn bind(
        dir: &RuntimeDir,
        policy: PeerPolicy,
        limits: CarrierLimits,
    ) -> Result<Self, LocalRefusal> {
        check_directory(dir.path())?;
        for _ in 0..NAME_DRAWS {
            let mut random = [0u8; 8];
            getrandom::fill(&mut random)
                .map_err(|_| LocalRefusal::os("getrandom", &io::Error::other("no random bytes")))?;
            let name = format!("s-{:016x}", u64::from_le_bytes(random));
            let path = socket_path(dir.path(), &name)?;
            match UnixListener::bind(&path) {
                Ok(listener) => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .map_err(|error| LocalRefusal::os("chmod", &error))?;
                    return Ok(Self {
                        listener,
                        address: LocalAddress(path),
                        policy,
                        limits,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
                Err(error) => return Err(LocalRefusal::os("bind", &error)),
            }
        }
        Err(LocalRefusal::NameTaken)
    }

    pub(crate) fn address(&self) -> &LocalAddress {
        &self.address
    }

    /// Waits for a connection and judges its peer before anything is read or written. A refused connection is closed
    /// and reported; the listener goes on.
    pub(crate) fn accept(&self) -> Accepted {
        let (stream, _) = self
            .listener
            .accept()
            .map_err(|error| LocalRefusal::os("accept", &error))?;
        let peer = check_peer(&stream, &self.policy, Relation::ListenerChecksClient)?;
        carrier_over(stream, Side::Core, peer, self.limits)
    }
}

impl AsRawFd for LocalListener {
    fn as_raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }
}

impl Drop for LocalListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.address.path());
    }
}

/// Connects to `address`, verifying the listener before a byte is sent: the socket's directory and file, then the
/// process that holds it.
pub(crate) fn connect(
    address: &LocalAddress,
    policy: &PeerPolicy,
    limits: CarrierLimits,
) -> Accepted {
    let path = address.path();
    let parent = path.parent().ok_or(LocalRefusal::NotOurSocket)?;
    check_directory(parent)?;
    let meta =
        std::fs::symlink_metadata(path).map_err(|error| LocalRefusal::os("lstat", &error))?;
    let ours = meta.file_type().is_socket() && meta.uid() == euid() && meta.mode() & 0o077 == 0;
    if !ours {
        return Err(LocalRefusal::NotOurSocket);
    }
    let stream = UnixStream::connect(path).map_err(|error| LocalRefusal::os("connect", &error))?;
    let peer = check_peer(&stream, policy, Relation::ClientChecksListener)?;
    carrier_over(stream, Side::Client, peer, limits)
}
