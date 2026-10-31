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
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
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
                    /// The kernel object of a name a test chooses: a row that saturates the gate must not hold the
                    /// process's own object, which the other rows of the process use at the same time.
                    fn named(name: &str) -> Self {
                        Self { kernel: Some(kernel::Kernel::open_named(name, kernel::Expect::Current)) }
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
                    /// There is no kernel object here, whatever the name.
                    fn named(_name: &str) -> Self {
                        Self { fault: None }
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
    state: Mutex<State>,
    changed: Condvar,
    process: ProcessLock,
}

/// What the in-process lock keeps. It is the entry to the gate on every excluding path, the process gate's kernel object
/// included, so that at most one waiting hold is ever queued in the kernel and a launch is not overtaken by a queue of them.
struct State {
    held: bool,
    /// Launches that are waiting for the gate ([`Launching`]). A blocking hold defers to them, within
    /// [`SpawnGate::LAUNCHES_AHEAD`].
    launches: usize,
    /// Blocking holds that are waiting in [`SpawnGate::hold`].
    waiting_holds: usize,
    /// Launches admitted since a blocking hold last was, counted only while one waits. At [`SpawnGate::LAUNCHES_AHEAD`] the
    /// next admission is the blocking hold's.
    launch_streak: usize,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a hold takes the in-process lock: the gate excludes in process, or the kernel object is the process's.
    fn guards(&self) -> bool {
        self.excludes || self.process.active()
    }

    /// Gives the in-process lock back and wakes the waiting holds.
    fn unlock(&self) {
        self.lock().held = false;
        self.changed.notify_one();
    }
}

/// The gate, held. The next hold waits until this is dropped. It owns its gate's state, so it can be moved to the
/// thread that launches.
pub(crate) struct SpawnHold {
    release: Release,
}
enum Release {
    Nothing,
    /// The in-process lock alone.
    Local(Arc<Shared>),
    /// The in-process lock and the process gate's kernel object.
    Process(Arc<Shared>),
}

/// A launch that is waiting for the gate (the HTTPS helper's, which waits on its own clock by polling [`SpawnGate::try_hold`]).
/// While one exists a blocking [`SpawnGate::hold`] defers to it, so a launch that polls is not lost to a queue of blocking
/// holds for as long as the queue is refilled (the spawn-gate fairness probe, `https_auth/runner/gate_fairness_tests.rs`).
/// The precedence is bounded, the other way: once [`SpawnGate::LAUNCHES_AHEAD`] launches have been admitted since a
/// blocking hold last was, while one waits, the next admission is a blocking hold's and the launch polls on (a launch is
/// refused by [`Self::try_hold`] until it is). Without the bound a stream of overlapping launches, each registered before
/// the last is dropped, keeps a blocking hold out for as long as the stream lasts (`spawn_gate/bounded_precedence_tests.rs`).
/// Counting launches rather than time keeps the gate free of a clock, and the bound holds however long a launch's window is.
/// Dropping it, with the launch holding the gate or having given up, lets the blocking holds start again.
pub(crate) struct Launching {
    gate: SpawnGate,
}

impl SpawnGate {
    /// Whether the process gate excludes across handles: where a spawn needs a creation window (Windows). Elsewhere it
    /// never waits.
    #[allow(
        dead_code,
        reason = "only the transport build's composition makes, or asks about, the process gate"
    )]
    pub(crate) const PROCESS_EXCLUDES: bool = cfg!(windows);

    /// The most launches that are admitted while a blocking hold waits (see [`Launching`]).
    pub(crate) const LAUNCHES_AHEAD: usize = 4;

    fn make(scope: Scope, excludes: bool, process: ProcessLock) -> Self {
        Self {
            shared: Arc::new(Shared {
                scope,
                excludes,
                state: Mutex::new(State {
                    held: false,
                    launches: 0,
                    waiting_holds: 0,
                    launch_streak: 0,
                }),
                changed: Condvar::new(),
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

            /// The process gate on the kernel object `name` (Windows; elsewhere the process gate): the gate of a row that
            /// saturates it, which must not hold the object the other rows of the process share.
            #[allow(dead_code, reason = "only the transport build's fairness probe (https_auth::runner) saturates a gate")]
            pub(crate) fn process_named(name: &str) -> Self {
                let process = ProcessLock::named(name);
                let excludes = Self::PROCESS_EXCLUDES && !process.active();
                Self::make(Scope::Process, excludes, process)
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
    /// is not re-entrant. It defers to a launch that is waiting ([`Self::launching`]), but to no more than
    /// [`Self::LAUNCHES_AHEAD`] of them.
    pub(crate) fn hold(&self) -> SpawnHold {
        if !self.shared.guards() {
            return SpawnHold {
                release: Release::Nothing,
            };
        }
        let mut state = self.shared.lock();
        state.waiting_holds += 1;
        while state.held || (state.launches > 0 && state.launch_streak < Self::LAUNCHES_AHEAD) {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.waiting_holds -= 1;
        state.launch_streak = 0;
        state.held = true;
        drop(state);
        // The process gate's kernel object, with the in-process lock held: the one thread of this gate that is let to wait
        // for it is first in line there.
        SpawnHold {
            release: self.release(self.shared.process.acquire()),
        }
    }

    /// Registers a launch that is about to wait for the gate with [`Self::try_hold`] (see [`Launching`]).
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    pub(crate) fn launching(&self) -> Launching {
        self.shared.lock().launches += 1;
        Launching { gate: self.clone() }
    }

    /// Holds the gate if no other hold exists, without waiting. A launch uses it to wait on its own clock, registered
    /// with [`Self::launching`]; only the transport build has launches.
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    pub(crate) fn try_hold(&self) -> Option<SpawnHold> {
        self.take(false)
    }

    /// [`Self::try_hold`] for a registered launch (`launch`), whose admission is counted against the blocking holds it
    /// takes precedence over, and is refused when the blocking hold waiting is due: [`Self::LAUNCHES_AHEAD`] launches
    /// have been admitted since a blocking hold last was.
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    fn take(&self, launch: bool) -> Option<SpawnHold> {
        if !self.shared.guards() {
            return Some(SpawnHold {
                release: Release::Nothing,
            });
        }
        let mut state = self.shared.lock();
        let contended = launch && state.waiting_holds > 0;
        if state.held || (contended && state.launch_streak >= Self::LAUNCHES_AHEAD) {
            return None;
        }
        state.held = true;
        if contended {
            state.launch_streak += 1;
        }
        drop(state);
        if self.shared.process.active() && !self.shared.process.try_acquire() {
            // Another handle of the process gate, or another thread outside this one, has the kernel object.
            if contended {
                self.shared.lock().launch_streak -= 1;
            }
            self.shared.unlock();
            return None;
        }
        Some(SpawnHold {
            release: self.release(self.shared.process.active()),
        })
    }

    /// The release of a hold that has the in-process lock, and the kernel object if `kernel`.
    fn release(&self, kernel: bool) -> Release {
        if kernel {
            Release::Process(self.shared.clone())
        } else {
            Release::Local(self.shared.clone())
        }
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
            Release::Local(shared) => shared.unlock(),
            Release::Process(shared) => {
                // The kernel object first, so the next thread of this gate to take the in-process lock finds it free.
                shared.process.release();
                shared.unlock();
            }
        }
    }
}

impl Launching {
    /// [`SpawnGate::try_hold`] for the registered launch.
    #[allow(
        dead_code,
        reason = "only the transport build's launches wait on their own clock"
    )]
    pub(crate) fn try_hold(&self) -> Option<SpawnHold> {
        self.gate.take(true)
    }
}

impl Drop for Launching {
    fn drop(&mut self) {
        let mut state = self.gate.shared.lock();
        state.launches -= 1;
        if state.launches == 0 {
            drop(state);
            self.gate.shared.changed.notify_all();
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
        mod bounded_precedence_tests;
    }
}
