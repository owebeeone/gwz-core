//! The local carrier: a [`StreamCarrier`](super::StreamCarrier) over a Unix domain socket (macOS, Linux) or a Windows
//! named pipe, between two processes of one user on one machine (plan 2.1; the parked server design's §4 listener and
//! peer rules, with the clauses §1.1 of the plan replaces left out).
//!
//! **Nothing listens on TCP.** The listener is an `AF_UNIX` socket or a local-only pipe; there is no other address.
//!
//! **The per-user directory** (Unix). Sockets live in a directory only this user can enter:
//!
//! | Platform | Directory | Why |
//! | --- | --- | --- |
//! | macOS | `confstr(_CS_DARWIN_USER_TEMP_DIR)` + `gwz/` (a path like `/var/folders/ab/.../T/gwz/`) | the operating system's per-user temporary directory: created for the user at login with mode 0700, not `$TMPDIR`, which a process may set to anywhere, and not `/tmp`, which every user can write |
//! | Linux | `/run/user/<uid>/gwz/` | the runtime directory systemd makes (the value `XDG_RUNTIME_DIR` has), without reading the environment |
//! | either, when that parent is missing | `/tmp/gwz-<uid>/` | the parked design's fallback; refused unless it exists with this user as owner and mode 0700 |
//!
//! The directory is created with mode 0700 below its parent without following a link, and every use checks it again:
//! a directory, not a link, owned by this user, with no group or other permission bit. A directory that fails the
//! check is refused ([`DirectoryFault`]), never repaired.
//!
//! **The path-length rule** (Unix). A socket address is a path in a fixed-size field (`sun_path`: 104 bytes on macOS,
//! 108 on Linux, each counting the NUL). The directory path plus `/` plus the socket's name (`s-` and 16 hex digits,
//! 18 bytes) must fit in the field less the NUL: 103 bytes on macOS, 107 on Linux. A longer path is refused with
//! [`LocalRefusal::AddressTooLong`] before anything is created; it is never truncated and no shorter alias is made.
//!
//! **The peer check** runs on both sides, before either side sends a byte: the listener reads the client's
//! credentials after `accept`, and the client reads the listener's after `connect`
//! (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` and `LOCAL_PEERPID` on macOS; on Windows the pipe's client and server
//! process ids, then that process's token user, integrity level and AppContainer flag). The rule is
//! [`PeerPolicy::verify`]. A refused connection is closed with nothing written to it.
//!
//! **The Windows pipe** has a random name (`\\.\pipe\gwz-` and 32 hex digits), is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` (a squatter that already holds the name makes the creation fail), with
//! `PIPE_REJECT_REMOTE_CLIENTS`, and with a protected DACL that grants access to this user's SID alone. A client opens
//! it with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so a server can identify it but never impersonate it.
use super::StreamCarrier;
use std::{fmt, io};

mod peer;

pub(crate) use peer::{Integrity, PeerIdentity, PeerPolicy, PeerRefusal, Principal, Relation};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub(crate) use unix::{LocalAddress, LocalListener, RuntimeDir, connect, own_identity};
    } else if #[cfg(windows)] {
        mod windows;
        pub(crate) use windows::{LocalAddress, LocalListener, RuntimeDir, connect, own_identity};
    }
}
// What the rows reach into: the path rule and the pipe's construction, which production code uses through the items above.
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        pub(crate) use unix::{SOCKET_NAME_BYTES, SUN_PATH_MAX, socket_path};
    } else if #[cfg(all(test, windows))] {
        pub(crate) use windows::{PIPE_MODE, PIPE_OPEN_MODE, create_first_instance, created_pipe_dacl_sids, pipe_sddl};
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
    /// The peer check refused the other end. Nothing was sent to it.
    Peer(PeerRefusal),
    /// The pipe name is already taken: someone else created it first (Windows `FILE_FLAG_FIRST_PIPE_INSTANCE`).
    NameTaken,
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
            LocalRefusal::Peer(refusal) => write!(f, "the peer was refused: {refusal}"),
            LocalRefusal::NameTaken => f.write_str("the pipe name is already taken"),
            LocalRefusal::Os { call, kind, code } => write!(f, "{call} failed: {kind} ({code:?})"),
        }
    }
}

impl std::error::Error for LocalRefusal {}

/// The result of an accept: a connected carrier, or why this connection was refused. A refusal closes only that
/// connection; the listener goes on.
pub(crate) type Accepted = Result<StreamCarrier, LocalRefusal>;
