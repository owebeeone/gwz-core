//! The host context, the session context and `open` (session plan CS1.4), for
//! the core session contract (gwz-dev `dev-docs/GwzCoreSessionDesign.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The driver creates a `HostContext` and passes it to every session it opens; they share it, and no static holds one | §2, §5.6 |
//! | Its supervisor thread starts with its first job, and stops once the host context has been dropped and its jobs have finished | §5.6 |
//! | A job whose poll panics is quarantined: kept, so its finish is never assumed, but never polled again, and dropped when the supervisor stops | §5.6; reuse design §7 |
//! | `open` validates the limits, refusing with `invalid_request` before any effect, then creates the session context on the calling thread | §1, §5.6, §9 |
//! | The session context holds the endpoint environment, the limits and the host context; a worker reaches it only through its gate | §5.6, O8, O9 |
//! | `open` returns the client end of the session's in-process channel | §9; the channel itself is CS1.2's |
//!
//! Later steps add members without changing these signatures. The host
//! context gains the SSH setup supervisor's jobs and budgets (CS3.5), the
//! HTTPS helper slots (CS3.6), the member lock manager (CS3.9), the workspace
//! registry (CS2.9), and TR1.4b's endpoint registry and bounded `shutdown`
//! (reuse design §7). The session context gains the transport timeouts
//! (CS3.8), the operation table (CS2.6) and the `diff.output` and `log.output`
//! registries (CS2.11). `SessionOptions` is `non_exhaustive`, so a later field,
//! such as the server design's `transport_off`, breaks no caller.

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use super::environment::EnvironmentSnapshot;
use super::limits::Limits;
use crate::model::ModelResult;

/// What the sessions of one driver share (§5.6): the driver creates it and
/// passes it to every session it opens.
///
/// A clone is another handle to the same host context. It is dropped when the
/// driver's handles and every session using it are gone. Dropping it stops its
/// supervisor thread once that thread's jobs have finished; the drop itself
/// does not wait. gwz-cli creates one at startup, and the Python bridge one per
/// process. Core keeps none in a static.
#[derive(Clone)]
pub struct HostContext {
    shared: Arc<HostShared>,
}

struct HostShared {
    supervisor: Arc<Supervisor>,
}

impl Drop for HostShared {
    fn drop(&mut self) {
        self.supervisor.release();
    }
}

/// A job the host context's supervisor owns until the job reports it has
/// finished (§5.6).
///
/// A job whose `poll` panics is quarantined, as the SSH reaper quarantines a
/// poisoned entry: it is kept, so its finish is never assumed while the host
/// context lives, but it is never polled again. It drops when the supervisor
/// stops, which quarantined jobs alone do not prevent.
pub(crate) trait SupervisedJob: Send {
    /// Advances the job without blocking. Returns true once it has finished
    /// and released what it owns.
    fn poll(&mut self) -> bool;
}

/// How often the supervisor polls while it has jobs.
const SUPERVISOR_POLL: Duration = Duration::from_millis(20);

#[derive(Default)]
struct Supervisor {
    state: Mutex<SupervisorState>,
    changed: Condvar,
}

#[derive(Default)]
struct SupervisorState {
    /// The jobs to poll.
    jobs: Vec<Box<dyn SupervisedJob>>,
    /// Jobs whose poll panicked: kept, never polled again.
    quarantined: Vec<Box<dyn SupervisedJob>>,
    /// The thread runs from the first job until the host context has been
    /// dropped and no job is left to poll.
    running: bool,
    /// The host context has been dropped.
    released: bool,
}

impl Supervisor {
    fn lock(&self) -> MutexGuard<'_, SupervisorState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn release(&self) {
        self.lock().released = true;
        self.changed.notify_all();
    }

    /// The supervisor thread: polls its jobs, quarantining any whose poll
    /// panics, and stops once none is left to poll and the host context has
    /// been dropped. The jobs are polled, and dropped, outside the lock.
    fn run(&self) {
        let mut state = self.lock();
        loop {
            let jobs = std::mem::take(&mut state.jobs);
            drop(state);
            let (mut active, mut quarantined) = (Vec::new(), Vec::new());
            for mut job in jobs {
                match catch_unwind(AssertUnwindSafe(|| job.poll())) {
                    Ok(true) => discard(job),
                    Ok(false) => active.push(job),
                    Err(_) => quarantined.push(job),
                }
            }
            state = self.lock();
            state.jobs.append(&mut active);
            state.quarantined.append(&mut quarantined);
            if !state.jobs.is_empty() {
                state = self
                    .changed
                    .wait_timeout(state, SUPERVISOR_POLL)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            } else if !state.released {
                // Nothing to poll, only quarantined jobs if any: wait for a
                // new job or the release, without a timeout.
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            } else {
                // Released, so no handle is left to add a job while the
                // quarantined ones drop.
                let quarantined = std::mem::take(&mut state.quarantined);
                drop(state);
                quarantined.into_iter().for_each(discard);
                self.lock().running = false;
                self.changed.notify_all();
                return;
            }
        }
    }
}

/// Drops a job the supervisor has done with. A panic in the job's `Drop` is
/// contained, so it cannot stop the supervisor.
fn discard(job: Box<dyn SupervisedJob>) {
    let _ = catch_unwind(AssertUnwindSafe(move || drop(job)));
}

impl HostContext {
    /// Creates a host context. It starts no thread until it has a job.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(HostShared {
                supervisor: Arc::default(),
            }),
        }
    }

    /// Hands `job` to the supervisor, starting its thread with the first job.
    /// If the thread cannot be created, the job is dropped unpolled, so a
    /// caller registers a job before starting the work it owns.
    #[allow(dead_code, reason = "CS3.5 moves the SSH setup jobs onto it")]
    pub(crate) fn supervise(&self, job: Box<dyn SupervisedJob>) -> io::Result<()> {
        let supervisor = &self.shared.supervisor;
        let mut state = supervisor.lock();
        if !state.running {
            let thread_supervisor = Arc::clone(supervisor);
            thread::Builder::new()
                .name("gwz-host-supervisor".into())
                .spawn(move || thread_supervisor.run())?;
            state.running = true;
        }
        state.jobs.push(job);
        supervisor.changed.notify_all();
        Ok(())
    }
}

impl Default for HostContext {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for HostContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostContext").finish_non_exhaustive()
    }
}

/// A session's context (§5.6), created by `open` on the calling thread and
/// dropped when the session ends. Every operation reaches it only through its
/// gate. Code on the session path reads session-relevant input from here, or
/// from the host context, never from the process (O9).
pub(crate) struct SessionContext {
    host: HostContext,
    environment: EnvironmentSnapshot,
    limits: Limits,
}

#[allow(dead_code, reason = "reached through the gate by CS1.6 and Phase 2")]
impl SessionContext {
    pub(crate) fn host(&self) -> &HostContext {
        &self.host
    }

    pub(crate) fn environment(&self) -> &EnvironmentSnapshot {
        &self.environment
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }
}

/// `open`'s options (§9): the host context to use, the endpoint environment
/// the driver captured (§5.6), and the limits of §1.
///
/// `non_exhaustive`: build it with `SessionOptions::new`, then change `limits`.
#[derive(Debug)]
#[non_exhaustive]
pub struct SessionOptions {
    /// The host context the session shares with the driver's other sessions.
    pub host: HostContext,
    /// The endpoint environment the driver captured, once, at its edge (§5.6).
    pub environment: EnvironmentSnapshot,
    /// The session's limits (§1). `open` validates them.
    pub limits: Limits,
}

impl SessionOptions {
    /// Options with the contract's default limits.
    pub fn new(host: HostContext, environment: EnvironmentSnapshot) -> Self {
        Self {
            host,
            environment,
            limits: Limits::default(),
        }
    }
}

/// Opens a session (§9). It validates `options.limits` (§1), refusing invalid
/// ones with `invalid_request` before any effect, then creates the session
/// context on the calling thread (§5.6), and returns the client end of the
/// session's in-process channel.
pub fn open(options: SessionOptions) -> ModelResult<ClientChannel> {
    options.limits.validate()?;
    let SessionOptions {
        host,
        environment,
        limits,
    } = options;
    let session = Arc::new(SessionContext {
        host,
        environment,
        limits,
    });
    Ok(ClientChannel { session })
}

/// The client end of a session's in-process channel (§3, §9).
///
/// This is the seam CS1.2 fills. TODO(CS1.2): the channel contract and the
/// in-process adapter give it `send(frame)`, which never blocks and fails with
/// `transport_session_full` on a full queue, `recv()`, which blocks until a
/// frame arrives or the session has ended, and `close()`, over two bounded
/// queues of the outstanding calls plus the control reserve (§3). CS2.2's
/// session host then owns the session context. Until then this end owns the
/// session: dropping it ends the session, and the session context and its
/// snapshot drop with it (§8's channel closure, before any worker exists).
pub struct ClientChannel {
    #[allow(dead_code, reason = "held for its drop until CS1.2 and CS2.2")]
    session: Arc<SessionContext>,
}

impl fmt::Debug for ClientChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientChannel").finish_non_exhaustive()
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl ClientChannel {
            pub(crate) fn session(&self) -> &Arc<SessionContext> {
                &self.session
            }
        }

        impl HostContext {
            pub(crate) fn same_as(&self, other: &HostContext) -> bool {
                Arc::ptr_eq(&self.shared, &other.shared)
            }

            pub(crate) fn supervisor_watch(&self) -> SupervisorWatch {
                SupervisorWatch(Arc::clone(&self.shared.supervisor))
            }
        }

        /// A test's view of a supervisor that can outlive its host context.
        pub(crate) struct SupervisorWatch(Arc<Supervisor>);

        impl SupervisorWatch {
            pub(crate) fn running(&self) -> bool {
                self.0.lock().running
            }

            /// Waits until the host context has been dropped and the thread,
            /// if it ever started, has stopped.
            pub(crate) fn wait_ended(&self, timeout: Duration) -> bool {
                let ended = |state: &SupervisorState| state.released && !state.running;
                let state = self.0.lock();
                let (state, _) = self
                    .0
                    .changed
                    .wait_timeout_while(state, timeout, |state| !ended(state))
                    .unwrap_or_else(PoisonError::into_inner);
                ended(&state)
            }
        }

        /// A session with the default limits and an empty snapshot.
        pub(crate) fn test_session() -> ClientChannel {
            let environment = EnvironmentSnapshot::from_byte_pairs(Vec::new()).unwrap();
            open(SessionOptions::new(HostContext::new(), environment)).unwrap()
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
