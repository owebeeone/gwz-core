//! The sandbox rule's findings and the platform probes that fill them (plan 2.3a; parked server design §4, "The sandbox
//! rule"). A process of this user that is sandboxed is refused absolutely, whatever sandbox the other side is in: Git runs
//! commands named in a workspace's own configuration (hooks, `core.fsmonitor`, `core.sshCommand`), so a sandboxed process
//! that can write a workspace would otherwise run code outside its sandbox through a server.
//!
//! A probe reads a process only through the hold [`super::PeerPolicy`]'s platform check has taken on it, and a probe
//! that cannot complete refuses ([`PeerRefusal::Unreadable`]); none of them falls back to "not sandboxed". The findings
//! become part of the process's [`PeerIdentity`](super::PeerIdentity), so that the rule, `PeerPolicy::verify`, stays a
//! pure function of identities.
//!
//! | Platform | Sandboxed when | Read through |
//! | --- | --- | --- |
//! | Linux | `Seccomp:` of any thread in `/proc/<pid>/task/*/status` is not 0, or `/proc/<pid>/ns/user` is not the initial user namespace's inode; refused too when the mount or user namespace differs from this process's | the hold: the `pidfd` the kernel recorded for the connection (`SO_PEERPIDFD`, Linux 6.5 or later, else refused), and the start-time bracket |
//! | macOS | `sandbox_check_by_audit_token(token, NULL, 0)` (else `sandbox_check(pid, NULL, 0)`) is not 0 | the `LOCAL_PEERTOKEN` token of the hold |
//! | Windows | the token's AppContainer flag, or an integrity level below Medium (`peer.rs`) | the process handle of the hold |
//!
//! **What the probes cannot see.** A sandbox that is none of these: a Landlock ruleset, an LSM profile (AppArmor,
//! SELinux), a cgroup or capability restriction. Such a peer is not refused and the rule does not claim it.
//!
//! **Sandboxed after the check.** A check judges the process as it is when the hold is taken and read; a process that
//! enters a sandbox afterwards is not refused, on any platform: on Linux a process that applies seccomp or `unshare`s
//! after it connected (or listened), or one that hands the connected descriptor (or the listening socket) to a sandboxed
//! child it forks; on macOS one that enters its sandbox after the check. Linux reads every thread, so a filter on any
//! thread counts at the time of the check. A connection whose connector (or listener) has gone, whatever process holds
//! the descriptor now, is refused: the hold names the process that made it.
//!
//! **Kernel requirement (Linux).** The hold is the descriptor `getsockopt(SO_PEERPIDFD)` returns (Linux 6.5 or later). A
//! kernel without it cannot say which process is on the other end of a connection, so every peer is refused
//! ([`PeerRefusal::Unreadable`], naming the requirement); there is no weaker fallback. A peer in a process-ID namespace
//! that this process's `/proc` does not show has no ID there, and is refused the same way.
use super::PeerRefusal;
use std::fmt;

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        pub(crate) mod linux;
        pub(crate) use linux::{of_peer, of_self};
    } else if #[cfg(target_vendor = "apple")] {
        pub(crate) mod macos;
        pub(crate) use macos::{of_peer, of_self};
    } else {
        /// Windows has no probe of this kind: its token flags are read with the identity (`windows.rs`).
        pub(crate) fn of_self() -> Result<SandboxFindings, PeerRefusal> {
            Ok(SandboxFindings::default())
        }
    }
}

/// `PROC_USER_INIT_INO`: the inode number the kernel gives the initial user namespace (`stat /proc/<pid>/ns/user`).
pub(crate) const INIT_USER_NAMESPACE: u64 = 0xEFFF_FFFD;

/// Why a process counts as sandboxed. Closed: a refusal names one of these and nothing else about the process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cause {
    /// Linux: a seccomp filter or strict mode.
    Seccomp,
    /// Linux: a user namespace other than the initial one.
    UserNamespace,
    /// Linux: a mount or user namespace other than the checking process's.
    OtherNamespace,
    /// macOS: the sandbox check reports a profile.
    Profile,
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Cause::Seccomp => "a seccomp filter",
            Cause::UserNamespace => "a user namespace",
            Cause::OtherNamespace => "a namespace other than this process's",
            Cause::Profile => "a sandbox profile",
        })
    }
}

/// What the probes found about one process. The default is "nothing found", which is what a platform without the probe
/// (Windows) and an identity built by hand carry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SandboxFindings {
    /// Linux: the `Seccomp:` mode (0 off, 1 strict, 2 filter).
    pub(crate) seccomp: u32,
    /// Linux: the inode of `/proc/<pid>/ns/user`.
    pub(crate) user_namespace: Option<u64>,
    /// Linux: the inode of `/proc/<pid>/ns/mnt`.
    pub(crate) mount_namespace: Option<u64>,
    /// macOS: the sandbox check reported a sandbox.
    pub(crate) profile: bool,
}

impl SandboxFindings {
    /// The reason this process is sandboxed, if it is.
    pub(crate) fn cause(&self) -> Option<Cause> {
        if self.seccomp != 0 {
            Some(Cause::Seccomp)
        } else if self
            .user_namespace
            .is_some_and(|inode| inode != INIT_USER_NAMESPACE)
        {
            Some(Cause::UserNamespace)
        } else if self.profile {
            Some(Cause::Profile)
        } else {
            None
        }
    }

    /// Whether both processes are in the same mount and user namespace. A namespace read for one and not the other is
    /// not the same.
    pub(crate) fn same_namespaces(&self, other: &Self) -> bool {
        self.user_namespace == other.user_namespace && self.mount_namespace == other.mount_namespace
    }
}
