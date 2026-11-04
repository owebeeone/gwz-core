//! The local carrier: a [`StreamCarrier`](super::StreamCarrier) over a Unix domain socket (macOS, Linux) or a Windows
//! named pipe, between two processes of one user on one machine (plan 2.1; the parked server design's §4 listener and
//! peer rules, with the clauses §1.1 of the plan replaces left out).
//!
//! **Nothing listens on TCP.** The listener is an `AF_UNIX` socket or a local-only pipe; there is no other address.
//!
//! **The per-user directory** (Unix; `GwzSplitCommandChannel.md` §1 and §4.2). Sockets live in a directory only this
//! user can enter, chosen by [`resolve_directory`] from values passed in ([`RuntimeEnv`]: the resolver reads nothing):
//!
//! | Platform | Directory | Why |
//! | --- | --- | --- |
//! | macOS | `confstr(_CS_DARWIN_USER_DIR)` + `gwz-<uid>/`; `XDG_RUNTIME_DIR` and `TMPDIR` are not read; a failing `confstr` refuses | the system's per-user directory, owned by the user and not writable by group or other; not the per-user temporary directory, whose old files dirhelper removes, and no `/tmp` fallback |
//! | Linux | `$XDG_RUNTIME_DIR/gwz/`, else `$TMPDIR/gwz-<uid>/`, else `/tmp/gwz-<uid>/` | the parked design's order; an unset or empty variable moves to the next choice, a set value that is not an absolute path is refused, not skipped |
//!
//! The directory is created with mode 0700 below its parent without following a link, and every use checks it again:
//! a directory, not a link, owned by this user, with no group or other permission bit. The parent chain is walked too,
//! and a parent writable by group or other (without the sticky bit, as `/tmp` has) is refused. A directory that fails
//! a check is refused ([`DirectoryFault`]), never repaired. The environment values come from one named file outside
//! this module, `transport_host/runtime_env.rs`, so that no code under `carrier/` reads the environment.
//!
//! **The path-length rule** (Unix). A socket address is a path in a fixed-size field (`sun_path`: 104 bytes on macOS,
//! 108 on Linux, each counting the NUL). The directory path plus `/` plus the socket's name (`ws-` and 16 hex digits,
//! 19 bytes, the workspace's server key: `server_files`) must fit in the field less the NUL: 103 bytes on macOS, 107 on
//! Linux. A longer path is refused with
//! [`LocalRefusal::AddressTooLong`] before anything is created; it is never truncated and no shorter alias is made.
//!
//! **The peer check** runs on both sides, before either side sends a byte: the listener reads the client's
//! credentials after `accept`, and the client reads the listener's after `connect`
//! (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` and `LOCAL_PEERPID` on macOS; on Windows the pipe's client and server
//! process ids, then that process's token user, integrity level and AppContainer flag). The rule is
//! [`PeerPolicy::verify`], which also refuses any process below Medium integrity (sandboxed, absolutely) and holds the
//! caller to it. The peer's process is **pinned** before it is read and checked again after: a process that has gone, or
//! whose identity changed under the same process ID, is refused (Linux: the `pidfd` the kernel recorded for the connection,
//! `getsockopt(SO_PEERPIDFD)`, which names the process that connected or listened and never the one that holds its ID
//! later, and the `/proc` start time before and after; a kernel before 6.5 has no such descriptor (it answers
//! `ENOPROTOOPT`) and refuses every peer, while kernels 6.5 to 6.15 answer `EINVAL` for a peer that has gone, which is
//! refused as a process that is no longer there; macOS: the `LOCAL_PEERTOKEN` audit token and a live-process check; Windows: a process handle, its creation time
//! and its exit state). **The sandbox rule's platform probes** (`sandbox.rs`; seccomp and namespaces on Linux,
//! `sandbox_check` on macOS, the AppContainer flag on Windows) run on the same hold, are part of the peer's identity, and
//! refuse with [`PeerRefusal::Sandboxed`]; a probe that cannot complete refuses. A listener answers a refused peer with
//! exactly one `SessionError` (`server_peer_refused`) and closes, reading nothing of the peer before the check; a client
//! that refuses a listener sends nothing.
//!
//! **The Windows pipe** has a random name (`\\.\pipe\gwz-` and 32 hex digits), is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` (a squatter that already holds the name makes the creation fail), with
//! `PIPE_REJECT_REMOTE_CLIENTS`, and with a protected DACL that grants access to this user's SID alone. A client opens
//! it with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so a server can identify it but never impersonate it.
use super::{CarrierBinding, CarrierId, CarrierLimits, Peer, Side, StreamCarrier};
use std::{
    fmt,
    io::{self, Read, Write},
};

pub(crate) mod address;
mod peer;
pub(crate) mod sandbox;
pub(crate) mod server_files;

use std::ffi::OsString;

pub(crate) use peer::{Integrity, PeerIdentity, PeerPolicy, PeerRefusal, Principal, Relation};

/// The environment values the per-user directory depends on, passed in by the caller. This module reads none: the
/// values are captured in one named file outside it (`transport_host/runtime_env.rs`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeEnv {
    pub(crate) xdg_runtime_dir: Option<OsString>,
    pub(crate) tmpdir: Option<OsString>,
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub(crate) use unix::{LocalAddress, LocalListener, RuntimeDir, connect_stream, own_identity};
        pub(crate) use unix::{DirFlavor, resolve_directory};

        /// What a client connects to (plan 2.3e2): a walk of the address, made under the process's trigger hold.
        pub(crate) type Target<'g> = address::Walk<'g>;
    } else if #[cfg(windows)] {
        mod windows;
        pub(crate) use windows::{LocalAddress, LocalListener, RuntimeDir, connect_stream, own_identity};

        /// What a client connects to (plan 2.3e2): a pipe name the pipe checks passed.
        pub(crate) type Target<'g> = address::CheckedPipe;
    }
}
// What the rows reach into: the path rule and the pipe's construction, which production code uses through the items above.
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        pub(crate) use unix::{Hold, SOCKET_NAME_BYTES, SUN_PATH_MAX, connect_once, connect_until, peer_of, socket_path};

        /// How long [`connect`] may take: the handshake's 30 seconds (server design §3).
        const CONNECT_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

        /// What a row's walk is lent: a gate of the row's own (a test process has no process owner; the rows that set
        /// the process's trigger policy are one row, `address_walk_rows`).
        pub(crate) type RowGate = address::NoTriggerGate;

        pub(crate) fn row_gate() -> RowGate {
            address::NoTriggerGate::new()
        }

        impl LocalAddress {
            /// An address the rows name: they reach it through the walk ([`walk_to`]); the carrier's connect and bind take
            /// only a walk.
            pub(crate) fn from_path(path: &std::path::Path) -> Self {
                Self(path.to_path_buf())
            }
        }

        /// The rows' walk of `address` under `gate`. A walk refusal is the carrier's [`LocalRefusal::Address`].
        pub(crate) fn walk_to<'g>(address: &LocalAddress, gate: &'g RowGate) -> Result<Target<'g>, LocalRefusal> {
            use std::os::unix::ffi::OsStrExt;
            address::walk(address.path().as_os_str().as_bytes(), gate).map_err(|refusal| LocalRefusal::Address {
                unavailable: false,
                text: refusal.to_string(),
            })
        }

        /// Connects and builds the carrier at once, with no handshake: [`connect_stream`] and
        /// [`RawConnection::into_carrier`] together, which is how the rows that exercise the carrier alone reach one.
        /// Test builds only: every byte stream a client opens uses the handshake (`handshake::dial`, server design §3).
        /// The same function stands in each platform's test arm.
        pub(crate) fn connect(
            address: &LocalAddress,
            policy: &PeerPolicy,
            limits: CarrierLimits,
        ) -> Accepted {
            let deadline = std::time::Instant::now() + CONNECT_BOUND;
            let gate = row_gate();
            connect_stream(walk_to(address, &gate)?, policy, deadline)?.into_carrier(limits)
        }
    } else if #[cfg(all(test, windows))] {
        pub(crate) use windows::{PIPE_MODE, PIPE_OPEN_MODE, ProcessHold, create_first_instance, create_first_instance_with, created_pipe_dacl_sids, judge, pipe_sddl, squat};

        /// How long [`connect`] may take: the handshake's 30 seconds (server design §3).
        const CONNECT_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

        /// What a row's walk is lent: Windows has no walk and no gate.
        pub(crate) struct RowGate;

        pub(crate) fn row_gate() -> RowGate {
            RowGate
        }

        /// The rows' pipe checks of `address`. A refusal is the carrier's [`LocalRefusal::Address`].
        pub(crate) fn walk_to<'g>(address: &LocalAddress, _: &'g RowGate) -> Result<Target<'g>, LocalRefusal> {
            address::check_name(address.pipe_name())
        }

        /// Connects and builds the carrier at once, with no handshake: [`connect_stream`] and
        /// [`RawConnection::into_carrier`] together, which is how the rows that exercise the carrier alone reach one.
        /// Test builds only: every byte stream a client opens uses the handshake (`handshake::dial`, server design §3).
        /// The same function stands in each platform's test arm.
        pub(crate) fn connect(
            address: &LocalAddress,
            policy: &PeerPolicy,
            limits: CarrierLimits,
        ) -> Accepted {
            let deadline = std::time::Instant::now() + CONNECT_BOUND;
            connect_stream(walk_to(address, &row_gate())?, policy, deadline)?.into_carrier(limits)
        }
    }
}

/// What is wrong with the per-user directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirectoryFault {
    Missing,
    NotADirectory,
    Link,
    OtherOwner,
    GroupOrOtherAccess,
    /// macOS: `confstr(_CS_DARWIN_USER_DIR)` failed. There is no fallback.
    NoUserDirectory,
    /// A directory above the per-user directory is writable by group or other and not sticky.
    ParentWritable,
    /// A set `XDG_RUNTIME_DIR` or `TMPDIR` that is not an absolute path.
    NotAbsolute,
}

/// Why the local carrier did not connect, listen or accept.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum LocalRefusal {
    /// The per-user directory failed its check.
    Directory(DirectoryFault),
    /// The socket's path does not fit the address field. Nothing was created.
    AddressTooLong { length: usize, limit: usize },
    /// The address names something that is not a socket this user owns.
    NotOurSocket,
    /// The walk, or the pipe's checks, did not give what the connect or bind needs, and nothing was connected or bound:
    /// `server_address_refused`, or, when nothing answers at the address (a missing name), `server_unavailable`.
    Address { unavailable: bool, text: String },
    /// The peer check refused the other end. Nothing was sent to it.
    Peer(PeerRefusal),
    /// This process was refused a policy (it is itself sandboxed, or the check on it could not be made): there is no
    /// peer, and the text says so.
    Own(PeerRefusal),
    /// The pipe name is already taken: someone else created it first (Windows `FILE_FLAG_FIRST_PIPE_INSTANCE`).
    NameTaken,
    /// The connect did not complete before its bound: a socket whose accept queue stayed full, a pipe that stayed busy.
    Timeout,
    /// An operating-system call failed.
    Os {
        call: &'static str,
        kind: io::ErrorKind,
        code: Option<i32>,
    },
}

impl LocalRefusal {
    pub(crate) fn os(call: &'static str, error: &io::Error) -> Self {
        LocalRefusal::Os {
            call,
            kind: error.kind(),
            code: error.raw_os_error(),
        }
    }
}

impl fmt::Display for LocalRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocalRefusal::Directory(fault) => {
                write!(f, "the per-user directory is not usable: {fault:?}")
            }
            LocalRefusal::AddressTooLong { length, limit } => {
                write!(
                    f,
                    "the socket path is {length} bytes and the address field holds {limit}"
                )
            }
            LocalRefusal::NotOurSocket => f.write_str("the address is not a socket this user owns"),
            LocalRefusal::Address { text, .. } => f.write_str(text),
            LocalRefusal::Peer(refusal) => write!(f, "the peer was refused: {refusal}"),
            LocalRefusal::Own(refusal) => f.write_str(&refusal.about_caller()),
            LocalRefusal::NameTaken => f.write_str("the pipe name is already taken"),
            LocalRefusal::Timeout => f.write_str("the connect did not complete within its bound"),
            LocalRefusal::Os { call, kind, code } => write!(f, "{call} failed: {kind} ({code:?})"),
        }
    }
}

impl std::error::Error for LocalRefusal {}

/// The result of an accept: a connected carrier, or why this connection was refused. A refusal closes only that
/// connection; the listener goes on.
pub(crate) type Accepted = Result<StreamCarrier, LocalRefusal>;

/// A connected stream whose peer the check has passed, before any carrier is built over it: what the handshake's client
/// reads and writes first. It becomes a carrier with [`RawConnection::into_carrier`] once the handshake is done, and
/// not before, so a host's first bytes are judged as bytes and not as frames (`handshake`).
pub(crate) struct RawConnection {
    pub(crate) read: Box<dyn Read + Send>,
    pub(crate) write: Box<dyn Write + Send>,
    /// Shuts the stream so that a blocked read or write returns.
    pub(crate) abort: Box<dyn Fn() + Send + Sync>,
    pub(crate) peer: PeerIdentity,
    pub(crate) side: Side,
}

impl RawConnection {
    pub(crate) fn into_carrier(self, limits: CarrierLimits) -> Accepted {
        let id = CarrierId::random().map_err(|error| LocalRefusal::os("getrandom", &error))?;
        StreamCarrier::new(
            self.read,
            self.write,
            self.abort,
            CarrierBinding::new(id, self.side, Peer::Verified(self.peer)),
            limits,
        )
        .map_err(|error| LocalRefusal::os("spawn", &error))
    }
}
