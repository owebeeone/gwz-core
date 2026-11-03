//! macOS: a process that resolves no automount trigger while it walks and connects or binds (parked server design §3
//! "The walk and the connect", macOS), for the process that is its own: the CLI around its walk and
//! [`Walked::connect`](super::Walked::connect) (plan 2.3k), the serving CLI around its walk and
//! [`Walked::bind`](super::Walked::bind) (plan 2.3i), never `SocketCoreBridge`, inside an application's process.
//!
//! A [`NoTrigger`] holds `/dev/autofs_notrigger` open, or, where the device cannot be opened, the process's
//! `IOPOL_TYPE_VFS_TRIGGER_RESOLVE` policy set off. The device is held by each holder's own descriptor. The policy is
//! the process's, shared by every holder, so it is counted by a [`NoTriggerGate`]: set off by the first holder and
//! restored when the last is dropped, in whatever order they are. The gate belongs to the process owner (there is no
//! global one), which makes one and lends it; two gates in a process do not count each other.
use super::open_at;
use std::{
    ffi::{CStr, c_int},
    io,
    os::fd::OwnedFd,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// `IOPOL_TYPE_VFS_TRIGGER_RESOLVE`, `IOPOL_SCOPE_PROCESS` and `IOPOL_VFS_TRIGGER_RESOLVE_OFF`
/// (`<sys/resource.h>`; not in the libc crate).
const IOPOL_TYPE_VFS_TRIGGER_RESOLVE: c_int = 5;
const IOPOL_SCOPE_PROCESS: c_int = 0;
const IOPOL_VFS_TRIGGER_RESOLVE_OFF: c_int = 1;

unsafe extern "C" {
    fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
    fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
}

/// The holders of the process policy: how many, and the policy the first found.
#[derive(Debug)]
struct PolicyHolds {
    count: usize,
    previous: c_int,
}

/// The process owner's count of the holders of the trigger policy (module documentation).
#[derive(Debug)]
pub(crate) struct NoTriggerGate {
    holds: Mutex<PolicyHolds>,
}

impl NoTriggerGate {
    pub(crate) fn new() -> Self {
        NoTriggerGate {
            holds: Mutex::new(PolicyHolds {
                count: 0,
                previous: 0,
            }),
        }
    }

    fn holds(&self) -> MutexGuard<'_, PolicyHolds> {
        // The count is plain data written under the lock, whole or not at all: a poisoned lock is still right.
        self.holds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds `/dev/autofs_notrigger` open, else the policy off, until the returned guard is dropped.
    pub(crate) fn hold(&self) -> io::Result<NoTrigger<'_>> {
        self.hold_with(c"/dev/autofs_notrigger")
    }

    /// [`Self::hold`] with the device at `device`.
    pub(crate) fn hold_with(&self, device: &CStr) -> io::Result<NoTrigger<'_>> {
        if let Ok(fd) = open_at(libc::AT_FDCWD, device, libc::O_RDONLY | libc::O_CLOEXEC) {
            return Ok(NoTrigger {
                device: Some(fd),
                gate: None,
            });
        }
        let mut holds = self.holds();
        if holds.count == 0 {
            // SAFETY: plain calls on the process's own policy.
            let previous =
                unsafe { getiopolicy_np(IOPOL_TYPE_VFS_TRIGGER_RESOLVE, IOPOL_SCOPE_PROCESS) };
            if previous < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: as above.
            let set = unsafe {
                setiopolicy_np(
                    IOPOL_TYPE_VFS_TRIGGER_RESOLVE,
                    IOPOL_SCOPE_PROCESS,
                    IOPOL_VFS_TRIGGER_RESOLVE_OFF,
                )
            };
            if set != 0 {
                return Err(io::Error::last_os_error());
            }
            holds.previous = previous;
        }
        holds.count += 1;
        Ok(NoTrigger {
            device: None,
            gate: Some(self),
        })
    }

    /// One holder of the policy is gone: the last one restores what the first found.
    fn release(&self) {
        let mut holds = self.holds();
        holds.count = holds.count.saturating_sub(1);
        if holds.count == 0 {
            // SAFETY: restores the policy `hold_with` read.
            unsafe {
                setiopolicy_np(
                    IOPOL_TYPE_VFS_TRIGGER_RESOLVE,
                    IOPOL_SCOPE_PROCESS,
                    holds.previous,
                );
            }
        }
    }
}

/// While held, this process resolves no automount trigger (module documentation).
#[derive(Debug)]
pub(crate) struct NoTrigger<'gate> {
    device: Option<OwnedFd>,
    gate: Option<&'gate NoTriggerGate>,
}

impl NoTrigger<'_> {
    /// The device is held (rather than the process policy).
    pub(crate) fn holds_device(&self) -> bool {
        self.device.is_some()
    }

    /// The process's trigger policy is off.
    pub(crate) fn policy_is_off() -> bool {
        // SAFETY: a plain query of the process's own policy.
        let policy = unsafe { getiopolicy_np(IOPOL_TYPE_VFS_TRIGGER_RESOLVE, IOPOL_SCOPE_PROCESS) };
        policy == IOPOL_VFS_TRIGGER_RESOLVE_OFF
    }
}

impl Drop for NoTrigger<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.gate {
            gate.release();
        }
    }
}
