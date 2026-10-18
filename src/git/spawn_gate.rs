//! The spawn gate (WH2 contract C9 and section 6; operator decision D25).
//!
//! On Windows a child created with `bInheritHandles = TRUE` inherits every handle that is inheritable at that
//! moment. A launch that must hand a child exactly its own pipe ends makes them inheritable for the length of one
//! `CreateProcessW` (the creation window) and no longer, but a standard `Command` spawn on another thread uses
//! `bInheritHandles = TRUE` with no handle list, so one that overlaps the window inherits the ends.
//!
//! The gate serializes the two. A launch holds it for its window, and so does every gwz spawn that names no
//! handle list: it is held for the spawn only, and a spawn's wait comes after the hold is dropped (a caller waits
//! for a `git commit` and its hooks without it). The one call that cannot be split is libgit2's credential
//! helper, which spawns and waits inside one call of git2-rs's; that call is held whole (D25).
//!
//! The gate belongs to the host that owns the launches: [`crate::git::endpoint`]'s helper slots create it, and the
//! host hands the same handle to the backends of its requests. No static holds one. Two hosts have two gates, so a
//! spawn in one host does not wait for a window in another: that scope is the host's, not the process's.
//!
//! On every platform other than Windows the gate does not exclude: close-on-exec keeps a spawn from inheriting
//! another thread's descriptors, so a hold there costs nothing and waits for nothing. The interface is the same.
use std::{
    io,
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Condvar, Mutex, PoisonError},
};

/// One host's spawn gate. A clone is another handle to the same gate.
#[derive(Clone)]
pub(crate) struct SpawnGate {
    shared: Arc<Shared>,
}

struct Shared {
    /// Whether a hold excludes another. Fixed when the gate is made.
    excludes: bool,
    held: Mutex<bool>,
    released: Condvar,
}

/// The gate, held. The next hold waits until this is dropped. It owns its gate's state, so it can be moved to the
/// thread that launches.
pub(crate) struct SpawnHold {
    held: Option<Arc<Shared>>,
}

impl SpawnGate {
    /// A gate that excludes where inheritance needs it to (Windows) and is a no-op elsewhere.
    pub(crate) fn new() -> Self {
        Self::with_exclusion(cfg!(windows))
    }

    /// A gate that excludes, or one that does not, whatever the platform: the platform's choice is [`Self::new`]'s,
    /// and a test makes the other.
    pub(crate) fn with_exclusion(excludes: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                excludes,
                held: Mutex::new(false),
                released: Condvar::new(),
            }),
        }
    }

    /// Whether `other` is a handle to this same gate, not another host's.
    #[allow(
        dead_code,
        reason = "only the transport build's NativeHost asks whether a backend is on its gate"
    )]
    pub(crate) fn same_gate(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Waits until no other hold exists, then holds the gate. A thread that holds it must not ask again: the gate
    /// is not re-entrant.
    pub(crate) fn hold(&self) -> SpawnHold {
        if !self.shared.excludes {
            return SpawnHold { held: None };
        }
        let mut held = self
            .shared
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while *held {
            held = self
                .shared
                .released
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *held = true;
        SpawnHold {
            held: Some(self.shared.clone()),
        }
    }

    /// Holds the gate if no other hold exists, without waiting. A launch uses it to wait on its own clock; only the
    /// transport build has launches.
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    pub(crate) fn try_hold(&self) -> Option<SpawnHold> {
        if !self.shared.excludes {
            return Some(SpawnHold { held: None });
        }
        let mut held = self
            .shared
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *held {
            return None;
        }
        *held = true;
        Some(SpawnHold {
            held: Some(self.shared.clone()),
        })
    }

    /// Spawns `command` under the gate and releases it as soon as the child exists. The caller sets every
    /// standard stream it needs (a spawn inherits those it does not set; [`Self::output`] sets them like
    /// `output()`) and waits for the child afterwards.
    pub(crate) fn spawn(&self, command: &mut Command) -> io::Result<Child> {
        let hold = self.hold();
        let child = command.spawn();
        drop(hold);
        child
    }

    /// `command.output()` with the spawn under the gate: the child's input is null and its output and diagnostics
    /// are piped, as `output()` makes them, and the gate is released as soon as the child exists, before the wait.
    pub(crate) fn output(&self, command: &mut Command) -> io::Result<Output> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.spawn(command)?.wait_with_output()
    }
}

impl Default for SpawnGate {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SpawnHold {
    fn drop(&mut self) {
        if let Some(shared) = self.held.take() {
            *shared.held.lock().unwrap_or_else(PoisonError::into_inner) = false;
            shared.released.notify_one();
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
