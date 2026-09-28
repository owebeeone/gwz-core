//! The host context, the session context and `open` (session plan CS1.4,
//! extended by CS1.9), for the core session contract (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md`) as the reuse and server designs amend it.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The driver creates a `HostContext` and passes it to every session it opens; they share it, and no static holds one | §2, §5.6 |
//! | Its supervisor thread starts with its first job, and stops once the host context has been shut down or dropped and its jobs have finished | §5.6; reuse design §7 |
//! | A job whose poll panics is quarantined: kept, so its finish is never assumed, but never polled again, and dropped when the supervisor stops | §5.6; reuse design §7 |
//! | `shutdown` disposes what the members hold within one cleanup bound, 5 s, and reports what remains: the jobs still running at the bound, which the supervisor polls to their end, and the quarantined jobs | reuse design §7 "Host context and server shutdown", and §13's §5.6 bullet (CS:308) |
//! | The report's `peer_cleanup_confirmed` is false: no member has a peer before CS3.7's endpoint registry, so no peer cleanup occurred | §8's `(0, false)` |
//! | A later `shutdown`, from any handle, returns the first one's report and disposes nothing again; a concurrent one waits for the first, so it too returns within the bound | reuse design §7; §8's repeated close |
//! | Once `shutdown` has begun, `supervise` refuses a job, dropping it unpolled, and `open` refuses the host context with `invalid_request`, before any effect | reuse design §7; §9 |
//! | Dropping a host context without `shutdown` disposes the same way, without waiting, and reports nothing; after `shutdown`, the drop disposes nothing more | reuse design §7 and §13 (CS:308) |
//! | `open` validates the limits, refusing with `invalid_request` before any effect, then creates the session context on the calling thread | §1, §5.6, §9 |
//! | The session context holds the endpoint environment, the limits, the off switch's value and the host context; a worker reaches it only through its gate | §5.6, O8, O9 |
//! | `transport_off` comes only from `open`'s options: nothing derives it from the snapshot or from the process's own environment or configuration | §5.6 as server design §8 amends it; server design §5 "The off switch's value" |
//! | `open` returns the client end of the session's in-process channel | §9; the channel itself is CS1.2's |
//!
//! Later steps add members without changing these signatures. The host
//! context gains the SSH setup supervisor's jobs and budgets (CS3.5), the
//! HTTPS helper slots (CS3.6), the member lock manager (CS3.9), the workspace
//! registry (CS2.9), and the endpoint registry (CS3.7), whose disposal joins
//! `shutdown`'s one bound. The session context gains the transport timeouts
//! (CS3.8), the operation table (CS2.6) and the `diff.output` and `log.output`
//! registries (CS2.11). `SessionOptions` is `non_exhaustive`, so a later
//! field breaks no caller.

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::environment::EnvironmentSnapshot;
use super::limits::Limits;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// What the sessions of one driver share (§5.6): the driver creates it and
/// passes it to every session it opens.
///
/// A clone is another handle to the same host context. It is dropped when the
/// driver's handles and every session using it are gone. At its end the
/// driver calls `shutdown`, which disposes what it holds within the cleanup
/// bound and reports what remains; dropping it without that disposes the same
/// way, without waiting or reporting. gwz-cli creates one at startup, and the
/// Python bridge one per process. Core keeps none in a static.
#[derive(Clone)]
pub struct HostContext {
    shared: Arc<HostShared>,
}

struct HostShared {
    supervisor: Arc<Supervisor>,
    /// Set as `shutdown` begins; `open` then refuses the host context.
    shut_down: AtomicBool,
    /// The first `shutdown`'s report, which later calls return. That call
    /// holds the lock while it disposes, so a concurrent call waits for it.
    report: Mutex<Option<ShutdownReport>>,
}

impl Drop for HostShared {
    fn drop(&mut self) {
        // The disposal `shutdown` makes, without its wait or its report.
        // After a `shutdown` the supervisor is released already.
        self.supervisor.release();
    }
}

/// The cleanup bound within which `shutdown` disposes the host context's
/// members (reuse design §7: "within the 5 s cleanup bound").
const CLEANUP_BOUND: Duration = Duration::from_secs(5);

/// What a host context's `shutdown` leaves behind (reuse design §7). It
/// carries the two facts of the contract's §13 `CleanupReport`, so a driver
/// can add it to its session's close report (§8), as gwz-cli does at command
/// end (session plan CS6.6).
///
/// `non_exhaustive`: core creates it, and a driver reads its fields.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// The local work the host context still owned when `shutdown` returned:
    /// its jobs still running at the cleanup bound, which the supervisor
    /// polls to their end, and its quarantined jobs, whose finish is never
    /// assumed. Zero when everything was disposed within the bound.
    pub pending_local_work: u32,
    /// Whether a peer confirmed its cleanup. False until the endpoint
    /// registry (CS3.7) gives the host context a peer: like the close report
    /// of a session that ran no network operation, `(0, false)` says that no
    /// peer cleanup occurred (§8).
    pub peer_cleanup_confirmed: bool,
}

/// A job the host context's supervisor owns until the job reports it has
/// finished (§5.6).
///
/// A job whose `poll` panics is quarantined, as the SSH reaper quarantines a
/// poisoned entry: it is kept, so its finish is never assumed, but it is never
/// polled again. It drops when the supervisor stops, after the host context's
/// shutdown or drop, which quarantined jobs alone do not delay.
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
    /// The jobs waiting for their next poll.
    jobs: Vec<Box<dyn SupervisedJob>>,
    /// The jobs the thread has taken out of `jobs` to poll outside the lock.
    polling: usize,
    /// Jobs whose poll panicked: kept, never polled again.
    quarantined: Vec<Box<dyn SupervisedJob>>,
    /// The thread runs from the first job until it has been released and no
    /// job is left to poll.
    running: bool,
    /// No job is taken: the host context has been shut down or dropped.
    closed: bool,
    /// The thread may stop once no job is left to poll: the host context has
    /// been dropped, or its shutdown has reported.
    released: bool,
}

impl SupervisorState {
    /// The jobs still to finish, apart from the quarantined ones.
    fn active(&self) -> usize {
        self.jobs.len() + self.polling
    }
}

impl Supervisor {
    fn lock(&self) -> MutexGuard<'_, SupervisorState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The disposal when the host context drops: no new job, and the thread
    /// polls the rest to their end, then drops the quarantined ones and
    /// stops. It does not wait.
    fn release(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.released = true;
        drop(state);
        self.changed.notify_all();
    }

    /// The supervisor's part of `shutdown`: the same disposal, after waiting
    /// until `deadline` for its jobs to finish. The wait frees the lock the
    /// thread needs. Returns the jobs that remain, still running or
    /// quarantined, counted before the release lets the thread drop any.
    fn dispose(&self, deadline: Instant) -> usize {
        let mut state = self.lock();
        state.closed = true;
        let wait = deadline.saturating_duration_since(Instant::now());
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, wait, |state| state.active() > 0)
            .unwrap_or_else(PoisonError::into_inner);
        let remaining = state.active() + state.quarantined.len();
        state.released = true;
        drop(state);
        self.changed.notify_all();
        remaining
    }

    /// The supervisor thread: polls its jobs, quarantining any whose poll
    /// panics, and stops once it has been released and none is left to poll.
    /// The jobs are polled, and dropped, outside the lock.
    fn run(&self) {
        let mut state = self.lock();
        loop {
            let jobs = std::mem::take(&mut state.jobs);
            state.polling = jobs.len();
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
            let settled = state.polling > active.len();
            state.polling = 0;
            state.jobs.append(&mut active);
            state.quarantined.append(&mut quarantined);
            if settled {
                // A shutdown waiting for the jobs to finish counts them again.
                self.changed.notify_all();
            }
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
                // Released, and so closed: no job can be added while the
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
                shut_down: AtomicBool::new(false),
                report: Mutex::new(None),
            }),
        }
    }

    /// Shuts the host context down at the driver's end (reuse design §7). It
    /// disposes what the host context's members hold within one cleanup
    /// bound of 5 seconds and returns a report of what remains. It returns
    /// within the bound even when a job never finishes: that job stays with
    /// the supervisor, which polls it to its end.
    ///
    /// Once it has begun, `open` refuses the host context and no new job
    /// starts. A later call, from any handle, returns the same report and
    /// disposes nothing again, and a concurrent call waits for the first, so
    /// each call returns within the bound. Dropping the host context without
    /// `shutdown` disposes the same way, without waiting or reporting; after
    /// `shutdown`, the drop disposes nothing more.
    pub fn shutdown(&self) -> ShutdownReport {
        self.shared.shutdown(CLEANUP_BOUND)
    }

    /// Hands `job` to the supervisor, starting its thread with the first job.
    /// If the thread cannot be created, or the host context has been shut
    /// down, the job is dropped unpolled, so a caller registers a job before
    /// starting the work it owns, and no work starts that nothing cleans up.
    #[allow(dead_code, reason = "CS3.5 moves the SSH setup jobs onto it")]
    pub(crate) fn supervise(&self, job: Box<dyn SupervisedJob>) -> io::Result<()> {
        let supervisor = &self.shared.supervisor;
        let mut state = supervisor.lock();
        if state.closed {
            return Err(io::Error::other("the host context has been shut down"));
        }
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

impl HostShared {
    /// `shutdown` within `bound`.
    fn shutdown(&self, bound: Duration) -> ShutdownReport {
        let mut report = self.report.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(done) = report.as_ref() {
            return done.clone();
        }
        self.shut_down.store(true, Ordering::Release);
        // Every member disposes within the one deadline. CS3.7 adds the
        // endpoint registry's disposal before the supervisor's, whose jobs
        // then hold what a disposal leaves unfinished, and adds what remains.
        let remaining = self.supervisor.dispose(Instant::now() + bound);
        let disposed = ShutdownReport {
            pending_local_work: u32::try_from(remaining).unwrap_or(u32::MAX),
            peer_cleanup_confirmed: false,
        };
        *report = Some(disposed.clone());
        disposed
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
    /// The off switch's resolved value, from `open`'s options only.
    transport_off: bool,
}

#[allow(
    dead_code,
    reason = "reached through the gate by CS1.6, Phase 2 and CS3.10"
)]
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

    /// The off switch's resolved value, as `open`'s options gave it. With it
    /// on, CS3.10 runs every transport-scope request on the native path.
    pub(crate) fn transport_off(&self) -> bool {
        self.transport_off
    }
}

/// `open`'s options (§9): the host context to use, the endpoint environment
/// the driver captured (§5.6), the limits of §1, and the off switch's
/// resolved value (§5.6 as the server design amends it).
///
/// `non_exhaustive`: build it with `SessionOptions::new`, then change
/// `limits` or `transport_off`.
#[derive(Debug)]
#[non_exhaustive]
pub struct SessionOptions {
    /// The host context the session shares with the driver's other sessions.
    pub host: HostContext,
    /// The endpoint environment the driver captured, once, at its edge (§5.6).
    pub environment: EnvironmentSnapshot,
    /// The session's limits (§1). `open` validates them.
    pub limits: Limits,
    /// The off switch's resolved value (the server design's §5): the driver
    /// resolves the switch's forms into one value before `open`. The snapshot
    /// never carries it, and core never derives it from the snapshot or from
    /// the process's own environment or configuration. False from `new`.
    pub transport_off: bool,
}

impl SessionOptions {
    /// Options with the contract's default limits and the off switch off.
    pub fn new(host: HostContext, environment: EnvironmentSnapshot) -> Self {
        Self {
            host,
            environment,
            limits: Limits::default(),
            transport_off: false,
        }
    }
}

/// Opens a session (§9). It validates `options.limits` (§1) and refuses a
/// host context that has been shut down, each with `invalid_request` before
/// any effect. It then creates the session context on the calling thread
/// (§5.6), and returns the client end of the session's in-process channel.
pub fn open(options: SessionOptions) -> ModelResult<ClientChannel> {
    options.limits.validate()?;
    if options.host.shared.shut_down.load(Ordering::Acquire) {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "the host context has been shut down",
        ));
    }
    let SessionOptions {
        host,
        environment,
        limits,
        transport_off,
    } = options;
    let session = Arc::new(SessionContext {
        host,
        environment,
        limits,
        transport_off,
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
/// snapshot drop with it, the snapshot overwriting its buffers as it drops
/// (§8's channel closure, before any worker exists).
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

            /// `shutdown` within a short bound, so no test waits 5 seconds.
            pub(crate) fn shutdown_within(&self, bound: Duration) -> ShutdownReport {
                self.shared.shutdown(bound)
            }
        }

        /// A test's view of a supervisor that can outlive its host context.
        pub(crate) struct SupervisorWatch(Arc<Supervisor>);

        impl SupervisorWatch {
            pub(crate) fn running(&self) -> bool {
                self.0.lock().running
            }

            /// Waits until the supervisor has been released, by the host
            /// context's drop or its shutdown, and its thread, if it ever
            /// started, has stopped.
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
