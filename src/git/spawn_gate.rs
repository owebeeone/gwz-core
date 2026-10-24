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
//! The gate is the process's (operator decision D-E; contract 7.3 of the Connection API). The composition
//! (`transport_host::composition`) makes it once and hands it to every host it builds, whose helper slots, native
//! route and request backends hold the same gate; no static holds one. On Windows it is one named kernel object with a
//! name of this process's own, so a window held for one host's launch excludes every other host's spawn; if that object
//! cannot be made, or another account owns it, the gate is unavailable and native authentication refuses (`kernel.rs`).
//! A backend with no host takes part in no launch's window and has no gate.
//!
//! On every platform other than Windows the gate does not exclude: close-on-exec keeps a spawn from inheriting
//! another thread's descriptors, so a hold there costs nothing and waits for nothing. The interface is the same.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod kernel;
    }
}
use std::{
    io,
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Condvar, Mutex, PoisonError},
};

/// Why the process's gate is not available (contract 7.3): the Windows object could not be made, or another account owns it.
#[allow(
    dead_code,
    reason = "only the transport build's composition makes, or asks about, the process gate"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GateFault {
    NotCreated,
    ForeignOwner,
}

// The process gate's physical part. On Windows a named kernel object shared by every host in the process; elsewhere a
// spawn needs no window, so there is none.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[allow(
            dead_code,
            reason = "only the transport build's composition makes, or asks about, the process gate"
        )]
        struct ProcessLock {
            /// `None` for a gate that is not the process's; `Some(Err)` when the object is unavailable.
            kernel: Option<Result<kernel::Kernel, GateFault>>,
        }
        #[allow(
            dead_code,
            reason = "only the transport build's composition makes, or asks about, the process gate"
        )]
        impl ProcessLock {
            fn none() -> Self {
                Self { kernel: None }
            }
            fn open() -> Self {
                Self { kernel: Some(kernel::Kernel::open_process()) }
            }
            fn active(&self) -> bool {
                matches!(self.kernel, Some(Ok(_)))
            }
            fn fault(&self) -> Option<GateFault> {
                match self.kernel {
                    Some(Err(fault)) => Some(fault),
                    _ => None,
                }
            }
            fn acquire(&self) -> bool {
                matches!(&self.kernel, Some(Ok(kernel)) if kernel.acquire())
            }
            fn try_acquire(&self) -> bool {
                matches!(&self.kernel, Some(Ok(kernel)) if kernel.try_acquire())
            }
            fn release(&self) {
                if let Some(Ok(kernel)) = &self.kernel {
                    kernel.release();
                }
            }
            cfg_if::cfg_if! {
                if #[cfg(test)] {
                    fn faulted(fault: GateFault) -> Self {
                        Self { kernel: Some(Err(fault)) }
                    }
                }
            }
        }
    } else {
        #[allow(
            dead_code,
            reason = "only the transport build's composition makes, or asks about, the process gate"
        )]
        struct ProcessLock {
            /// Never set by the product: nothing here can fail. A test sets it to show the refusals.
            fault: Option<GateFault>,
        }
        #[allow(
            dead_code,
            reason = "only the transport build's composition makes, or asks about, the process gate"
        )]
        impl ProcessLock {
            fn none() -> Self {
                Self { fault: None }
            }
            fn open() -> Self {
                Self { fault: None }
            }
            fn active(&self) -> bool {
                false
            }
            fn fault(&self) -> Option<GateFault> {
                self.fault
            }
            fn acquire(&self) -> bool {
                false
            }
            fn try_acquire(&self) -> bool {
                false
            }
            fn release(&self) {}
            cfg_if::cfg_if! {
                if #[cfg(test)] {
                    fn faulted(fault: GateFault) -> Self {
                        Self { fault: Some(fault) }
                    }
                }
            }
        }
    }
}

/// A spawn gate. A clone is another handle to the same gate. A gate is made in one of three ways: [`Self::process`], by
/// the composition (`transport_host::composition`) alone, for the gate every host of the process shares;
/// [`Self::ungated`], for a backend with no host, which takes part in no launch window and so excludes nothing; and, in
/// tests, `new` and `with_exclusion`.
#[derive(Clone)]
pub(crate) struct SpawnGate {
    shared: Arc<Shared>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Local,
    #[allow(
        dead_code,
        reason = "only the transport build's composition makes the process gate"
    )]
    /// The process's own gate: any two of them are the same gate.
    Process,
}

struct Shared {
    scope: Scope,
    /// Whether a hold on the in-process lock excludes another. Fixed when the gate is made.
    excludes: bool,
    held: Mutex<bool>,
    released: Condvar,
    process: ProcessLock,
}

/// The gate, held. The next hold waits until this is dropped. It owns its gate's state, so it can be moved to the
/// thread that launches.
pub(crate) struct SpawnHold {
    release: Release,
}
enum Release {
    Nothing,
    Local(Arc<Shared>),
    Process(Arc<Shared>),
}

impl SpawnGate {
    /// Whether the process gate excludes across handles: where a spawn needs a creation window (Windows). Elsewhere it
    /// never waits.
    #[allow(
        dead_code,
        reason = "only the transport build's composition makes, or asks about, the process gate"
    )]
    pub(crate) const PROCESS_EXCLUDES: bool = cfg!(windows);

    fn make(scope: Scope, excludes: bool, process: ProcessLock) -> Self {
        Self {
            shared: Arc::new(Shared {
                scope,
                excludes,
                held: Mutex::new(false),
                released: Condvar::new(),
                process,
            }),
        }
    }

    /// The process's gate, which every host of the process shares (operator decision D-E). On Windows it is the named
    /// kernel object of [`kernel`](self), and if the object is unavailable the gate says so ([`Self::fault`]) and holds
    /// in this handle alone, so the plain spawns still serialize while native authentication, which needs the window,
    /// refuses. Elsewhere a spawn needs no window and the gate excludes nothing. Only the composition calls it.
    #[allow(
        dead_code,
        reason = "only the transport build's composition makes, or asks about, the process gate"
    )]
    pub(crate) fn process() -> Self {
        let process = ProcessLock::open();
        let excludes = Self::PROCESS_EXCLUDES && !process.active();
        Self::make(Scope::Process, excludes, process)
    }

    /// A gate for a backend that has no host: it takes part in no launch's window, so it excludes nothing, and it is
    /// the same gate as no other.
    pub(crate) fn ungated() -> Self {
        Self::make(Scope::Local, false, ProcessLock::none())
    }

    cfg_if::cfg_if! {
        if #[cfg(test)] {
            /// A gate that excludes where inheritance needs it to (Windows) and is a no-op elsewhere.
            pub(crate) fn new() -> Self {
                Self::with_exclusion(cfg!(windows))
            }

            /// A gate that excludes, or one that does not, whatever the platform: the platform's choice is
            /// [`Self::new`]'s, and a test makes the other.
            pub(crate) fn with_exclusion(excludes: bool) -> Self {
                Self::make(Scope::Local, excludes, ProcessLock::none())
            }

            /// The process gate whose object is unavailable for `fault`: it holds in this handle alone, as the product's
            /// does, and says why it is unavailable.
            pub(crate) fn faulted(fault: GateFault) -> Self {
                Self::make(Scope::Process, true, ProcessLock::faulted(fault))
            }
        }
    }

    /// Why this gate cannot give a launch its window, if it cannot: the process gate whose object is unavailable.
    #[allow(
        dead_code,
        reason = "only the transport build's composition makes, or asks about, the process gate"
    )]
    pub(crate) fn fault(&self) -> Option<GateFault> {
        self.shared.process.fault()
    }

    /// Whether `other` is a handle to this same gate, not another host's. Any two process gates are the same gate.
    #[allow(
        dead_code,
        reason = "only the transport build's NativeHost asks whether a backend is on its gate"
    )]
    pub(crate) fn same_gate(&self, other: &Self) -> bool {
        match (self.shared.scope, other.shared.scope) {
            (Scope::Process, Scope::Process) => true,
            _ => Arc::ptr_eq(&self.shared, &other.shared),
        }
    }

    /// Waits until no other hold exists, then holds the gate. A thread that holds it must not ask again: the gate
    /// is not re-entrant.
    pub(crate) fn hold(&self) -> SpawnHold {
        if self.shared.process.acquire() {
            return SpawnHold {
                release: Release::Process(self.shared.clone()),
            };
        }
        if !self.shared.excludes {
            return SpawnHold {
                release: Release::Nothing,
            };
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
            release: Release::Local(self.shared.clone()),
        }
    }

    /// Holds the gate if no other hold exists, without waiting. A launch uses it to wait on its own clock; only the
    /// transport build has launches.
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    pub(crate) fn try_hold(&self) -> Option<SpawnHold> {
        if self.shared.process.try_acquire() {
            return Some(SpawnHold {
                release: Release::Process(self.shared.clone()),
            });
        }
        if self.shared.process.active() {
            return None;
        }
        if !self.shared.excludes {
            return Some(SpawnHold {
                release: Release::Nothing,
            });
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
            release: Release::Local(self.shared.clone()),
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

/// gwz-git launches the Git CLI route's child through the host's gate.
impl gwz_git::native_commit::ProcessGate for SpawnGate {
    fn output(&self, command: &mut Command) -> io::Result<Output> {
        SpawnGate::output(self, command)
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Default for SpawnGate {
            fn default() -> Self {
                Self::new()
            }
        }
    }
}

impl Drop for SpawnHold {
    fn drop(&mut self) {
        match std::mem::replace(&mut self.release, Release::Nothing) {
            Release::Nothing => {}
            Release::Local(shared) => {
                *shared.held.lock().unwrap_or_else(PoisonError::into_inner) = false;
                shared.released.notify_one();
            }
            Release::Process(shared) => shared.process.release(),
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
