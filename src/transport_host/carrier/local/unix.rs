//! The Unix domain socket carrier (macOS, Linux). See the module documentation of [`super`] for the directory, the
//! path-length rule and the peer check.
use super::super::{
    CarrierBinding, CarrierId, CarrierLimits, Peer, Side,
    wire::{SERVER_PEER_REFUSED, session_error_frame},
};
use super::{
    Accepted, DirectoryFault, LocalRefusal, PeerIdentity, PeerPolicy, PeerRefusal, Principal,
    Relation, RuntimeEnv, StreamCarrier,
};
use gwz_session_channel::write_frame;
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
    time::Duration,
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

/// Which platform's rule picks the per-user directory. A value, not a build switch, so that the rows can state both
/// rules on one host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirFlavor {
    /// `confstr(_CS_DARWIN_USER_DIR)` and `gwz-<uid>`; the environment is not read.
    Darwin,
    /// `$XDG_RUNTIME_DIR/gwz`, else `$TMPDIR/gwz-<uid>`, else `/tmp/gwz-<uid>`.
    Linux,
}

cfg_if::cfg_if! {
    if #[cfg(target_vendor = "apple")] {
        impl DirFlavor {
            pub(crate) const HOST: DirFlavor = DirFlavor::Darwin;
        }
    } else {
        impl DirFlavor {
            pub(crate) const HOST: DirFlavor = DirFlavor::Linux;
        }
    }
}

/// The per-user directory's path under `flavor`'s rule: a pure function of the values passed in. `darwin_user_dir` is
/// what `confstr(_CS_DARWIN_USER_DIR)` returned (`None` when it failed).
pub(crate) fn resolve_directory(
    flavor: DirFlavor,
    env: &RuntimeEnv,
    uid: u32,
    darwin_user_dir: Option<&Path>,
) -> Result<PathBuf, LocalRefusal> {
    let absolute = |value: &std::ffi::OsString| {
        let path = Path::new(value);
        path.is_absolute()
            .then(|| path.to_path_buf())
            .ok_or(LocalRefusal::Directory(DirectoryFault::NotAbsolute))
    };
    let set = |value: &Option<std::ffi::OsString>| value.clone().filter(|v| !v.is_empty());
    match flavor {
        DirFlavor::Darwin => {
            let base =
                darwin_user_dir.ok_or(LocalRefusal::Directory(DirectoryFault::NoUserDirectory))?;
            Ok(base.join(format!("gwz-{uid}")))
        }
        DirFlavor::Linux => match (set(&env.xdg_runtime_dir), set(&env.tmpdir)) {
            (Some(xdg), _) => Ok(absolute(&xdg)?.join("gwz")),
            (None, Some(tmp)) => Ok(absolute(&tmp)?.join(format!("gwz-{uid}"))),
            (None, None) => Ok(PathBuf::from(format!("/tmp/gwz-{uid}"))),
        },
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_vendor = "apple")] {
        /// The system's per-user directory on macOS (`confstr(_CS_DARWIN_USER_DIR)`), which is not the per-user
        /// temporary directory and not `$TMPDIR`.
        fn darwin_user_dir() -> Option<PathBuf> {
            let mut buffer = vec![0u8; libc::PATH_MAX as usize];
            // SAFETY: the buffer is writable for its stated length; `confstr` NUL-terminates what it writes.
            let written = unsafe {
                libc::confstr(libc::_CS_DARWIN_USER_DIR, buffer.as_mut_ptr().cast(), buffer.len())
            };
            if written == 0 || written > buffer.len() {
                return None;
            }
            let path = CStr::from_bytes_until_nul(&buffer).ok()?;
            Some(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
        }
    } else {
        /// Only macOS has a `confstr` directory.
        fn darwin_user_dir() -> Option<PathBuf> {
            None
        }
    }
}

/// Every directory above `path` is not writable by group or other, unless it is sticky (as `/tmp` is). The parents are
/// resolved first, so a link in the chain (`/var` on macOS) is followed to the directories it leads through.
fn check_parents(path: &Path) -> Result<(), LocalRefusal> {
    let parent = path.parent().ok_or(LocalRefusal::NotOurSocket)?;
    let real = std::fs::canonicalize(parent).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => LocalRefusal::Directory(DirectoryFault::Missing),
        _ => LocalRefusal::os("canonicalize", &error),
    })?;
    for ancestor in real.ancestors() {
        let meta = std::fs::symlink_metadata(ancestor)
            .map_err(|error| LocalRefusal::os("lstat", &error))?;
        let mode = meta.mode();
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            return Err(LocalRefusal::Directory(DirectoryFault::ParentWritable));
        }
    }
    Ok(())
}

impl RuntimeDir {
    /// The user's directory (the module documentation says where), created with mode 0700 on first use and checked on
    /// every use, its parents walked. `env` is the values the caller captured; see [`resolve_directory`].
    pub(crate) fn for_current_user(env: &RuntimeEnv) -> Result<Self, LocalRefusal> {
        Self::in_resolved(DirFlavor::HOST, env, darwin_user_dir().as_deref())
    }

    /// [`for_current_user`](Self::for_current_user) under `flavor`'s rule and a stated `confstr` result: the whole
    /// resolve, parent walk, create and check, with the platform's two inputs passed in so that the rows can state
    /// either rule on one host.
    pub(crate) fn in_resolved(
        flavor: DirFlavor,
        env: &RuntimeEnv,
        darwin_user_dir: Option<&Path>,
    ) -> Result<Self, LocalRefusal> {
        let path = resolve_directory(flavor, env, euid(), darwin_user_dir)?;
        check_parents(&path)?;
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
        pub(crate) fn peer_of(fd: RawFd) -> Result<PeerIdentity, LocalRefusal> {
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
        pub(crate) fn peer_of(fd: RawFd) -> Result<PeerIdentity, LocalRefusal> {
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

/// Reads the peer of `stream` through a hold on its process, judges it, and confirms the hold: a peer that has gone, or
/// whose process ID now names another process, is refused. Nothing is written or read on the stream.
fn check_peer(
    stream: &UnixStream,
    policy: &PeerPolicy,
    relation: Relation,
) -> Result<PeerIdentity, PeerRefusal> {
    let fd = stream.as_raw_fd();
    let peer = peer_of(fd).map_err(|_| PeerRefusal::Unreadable("the peer's credentials"))?;
    let pid = peer
        .pid
        .ok_or(PeerRefusal::Unreadable("the peer's process id"))?;
    let Principal::Uid(uid) = peer.principal else {
        return Err(PeerRefusal::Unreadable("the peer's user"));
    };
    let hold = Hold::take(fd, pid, uid)?;
    policy.verify(&peer, relation)?;
    hold.confirm(fd)?;
    Ok(peer)
}

/// A refused client of a listener is owed one answer (parked server design §4): a `SessionError` for call ID 0 with
/// `server_peer_refused`, naming the cause, and then the end of the stream. Nothing of the client was read before the
/// check; what it sent meanwhile is drained (bounded) after the answer, so that closing does not reset the connection
/// and lose the answer.
fn turn_away(stream: &UnixStream, refusal: &PeerRefusal) {
    let frame = session_error_frame(SERVER_PEER_REFUSED, &format!("peer refused: {refusal}"));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = write_frame(&mut &*stream, &frame);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut sink = [0u8; 4096];
    let mut drained = 0usize;
    while drained < 64 * 1024 {
        match (&*stream).read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(read) => drained += read,
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        use std::os::fd::{FromRawFd, OwnedFd};

        /// A hold on the peer's process: a `pidfd` for it, and the start time `/proc` gave before and after the
        /// descriptor was taken. Without a hold a process ID can name another process by the time it is read.
        pub(crate) struct Hold {
            pid: u32,
            pidfd: OwnedFd,
            started: u64,
        }

        /// A process's start time in clock ticks (field 22 of `/proc/<pid>/stat`) and whether it is a zombie or dead.
        fn process_start(pid: u32) -> Option<(u64, bool)> {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // The command name is in parentheses and may hold anything; the fields after its last `)` are fixed.
            let rest = &stat[stat.rfind(')')? + 1..];
            let mut fields = rest.split_whitespace();
            let state = fields.next()?;
            // Field 22 is the 20th after the command name's field 2.
            let started = fields.nth(18)?.parse().ok()?;
            Some((started, matches!(state, "Z" | "X")))
        }

        impl Hold {
            pub(crate) fn take(_fd: RawFd, pid: u32, _uid: u32) -> Result<Self, PeerRefusal> {
                let gone = PeerRefusal::Unreadable("a live peer process");
                let (before, dead) = process_start(pid).ok_or(gone.clone())?;
                if dead {
                    return Err(gone);
                }
                // SAFETY: `pidfd_open` takes a process ID and flags and returns a descriptor or -1.
                let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
                if fd < 0 {
                    return Err(PeerRefusal::Unreadable("a hold on the peer's process"));
                }
                // SAFETY: `fd` is a new descriptor this value owns.
                let pidfd = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
                let (after, dead) = process_start(pid).ok_or(gone.clone())?;
                if dead || after != before {
                    return Err(gone);
                }
                Ok(Self { pid, pidfd, started: before })
            }

            pub(crate) fn confirm(&self, _fd: RawFd) -> Result<(), PeerRefusal> {
                // SAFETY: signal 0 through the descriptor checks that the process is there and sends nothing.
                let alive = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        self.pidfd.as_raw_fd(),
                        0,
                        std::ptr::null::<libc::siginfo_t>(),
                        0u32,
                    )
                } == 0;
                let same = process_start(self.pid) == Some((self.started, false));
                (alive && same)
                    .then_some(())
                    .ok_or(PeerRefusal::Unreadable("a live peer process"))
            }
        }
    } else {
        /// A hold on the peer's process: the audit token the kernel recorded for the connection (`LOCAL_PEERTOKEN`),
        /// which carries the process's version as well as its ID, and a check that the process is there.
        pub(crate) struct Hold {
            pid: u32,
            token: [u32; 8],
        }

        /// `audit_token_t`: eight words; word 1 is the effective user, 5 the process ID, 7 its version.
        fn peer_token(fd: RawFd) -> Option<[u32; 8]> {
            let mut token = [0u32; 8];
            let mut length = mem::size_of_val(&token) as libc::socklen_t;
            // SAFETY: `token` is valid for `length` bytes.
            let result = unsafe {
                libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERTOKEN, (&raw mut token).cast(), &mut length)
            };
            (result == 0 && length as usize == mem::size_of_val(&token)).then_some(token)
        }

        /// Whether `pid` is a process that is there and not a zombie.
        fn process_alive(pid: u32) -> bool {
            // SAFETY: `info` is plain data and `proc_pidinfo` writes at most its stated size into it.
            let mut info: libc::proc_bsdshortinfo = unsafe { mem::zeroed() };
            let size = mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
            let written = unsafe {
                libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDT_SHORTBSDINFO, 0, (&raw mut info).cast(), size)
            };
            // 5 is `SZOMB`.
            written == size && info.pbsi_status != 5
        }

        impl Hold {
            pub(crate) fn take(fd: RawFd, pid: u32, uid: u32) -> Result<Self, PeerRefusal> {
                let token = peer_token(fd).ok_or(PeerRefusal::Unreadable("the peer's audit token"))?;
                if token[5] != pid || token[1] != uid {
                    return Err(PeerRefusal::Unreadable("a peer token that names the peer"));
                }
                if !process_alive(pid) {
                    return Err(PeerRefusal::Unreadable("a live peer process"));
                }
                Ok(Self { pid, token })
            }

            pub(crate) fn confirm(&self, fd: RawFd) -> Result<(), PeerRefusal> {
                let same = peer_token(fd) == Some(self.token);
                (same && process_alive(self.pid))
                    .then_some(())
                    .ok_or(PeerRefusal::Unreadable("a live peer process"))
            }
        }
    }
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
        match check_peer(&stream, &self.policy, Relation::ListenerChecksClient) {
            Ok(peer) => carrier_over(stream, Side::Core, peer, self.limits),
            Err(refusal) => {
                turn_away(&stream, &refusal);
                Err(LocalRefusal::Peer(refusal))
            }
        }
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
    let peer = match check_peer(&stream, policy, Relation::ClientChecksListener) {
        Ok(peer) => peer,
        Err(refusal) => {
            // A client that refuses its listener sends nothing.
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return Err(LocalRefusal::Peer(refusal));
        }
    };
    carrier_over(stream, Side::Client, peer, limits)
}
