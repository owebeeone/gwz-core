//! The host context's supervisor (session plan CS1.4, extended by CS1.9),
//! moved from gwz-core's `session_host/context.rs`, where the host context
//! stays a core composite that holds one (crate map §2 and §7).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Its thread starts with its first job, and stops once the supervisor has been shut down or dropped and its jobs have finished | §5.6; reuse design §7 |
//! | A job whose poll panics is quarantined: kept, so its finish is never assumed, but never polled again, and dropped when the thread stops | §5.6; reuse design §7 |
//! | `shutdown(bound)` closes it to new jobs, waits at most `bound` for its jobs to finish, and returns how many remain: those still running at the bound, which the thread polls to their end, and the quarantined ones | reuse design §7 |
//! | A later `shutdown` returns the first one's count and disposes nothing again; a concurrent one waits for the first, so each returns within its bound | reuse design §7; §8's repeated close |
//! | Once `shutdown` has begun, `supervise` refuses a job, dropping it unpolled | reuse design §7 |
//! | Dropping the supervisor disposes the same way, without waiting or reporting; after `shutdown`, the drop disposes nothing more | reuse design §7 and §13 |
//!
//! gwz-core's host context holds one and passes it what remains of the host
//! context's one cleanup bound, after the members that dispose first (CS1.9;
//! CS3.7 puts the endpoint registry there). CS3.5 moves the SSH setup jobs
//! onto it: one supervisor on the session path (crate map §2).

use std::error::Error;
use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// A job the supervisor owns until the job reports it has finished (§5.6).
///
/// A job whose `poll` panics is quarantined, as the SSH reaper quarantines a
/// poisoned entry: it is kept, so its finish is never assumed, but it is never
/// polled again. It drops when the thread stops, after the supervisor's
/// shutdown or drop, which quarantined jobs alone do not delay.
pub trait SupervisedJob: Send {
    /// Advances the job without blocking. Returns true once it has finished
    /// and released what it owns.
    fn poll(&mut self) -> bool;
}

/// How often the thread polls while it has jobs.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The supervisor of one host context: a thread that polls the jobs handed to
/// it until each finishes. It is not `Clone`: its holder, gwz-core's host
/// context, is shared, and dropping it releases the thread.
pub struct Supervisor {
    pub(crate) shared: Arc<Shared>,
}

pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    pub(crate) changed: Condvar,
    /// The first shutdown's count, which later calls return. That call holds
    /// the lock while it disposes, so a concurrent call waits for it.
    report: Mutex<Option<usize>>,
}

#[derive(Default)]
pub(crate) struct State {
    /// The jobs waiting for their next poll.
    jobs: Vec<Box<dyn SupervisedJob>>,
    /// The jobs the thread has taken out of `jobs` to poll outside the lock.
    polling: usize,
    /// Jobs whose poll panicked: kept, never polled again.
    quarantined: Vec<Box<dyn SupervisedJob>>,
    /// The thread runs from the first job until it has been released and no
    /// job is left to poll.
    pub(crate) running: bool,
    /// No job is taken: the supervisor has been shut down or dropped.
    closed: bool,
    /// The thread may stop once no job is left to poll: the supervisor has
    /// been dropped, or its shutdown has returned.
    pub(crate) released: bool,
}

impl State {
    /// The jobs still to finish, apart from the quarantined ones.
    fn active(&self) -> usize {
        self.jobs.len() + self.polling
    }
}

/// Why [`Supervisor::supervise`] did not take a job. Either way the job was
/// dropped unpolled.
#[derive(Debug)]
pub enum SuperviseError {
    /// The supervisor has been shut down.
    ShutDown,
    /// Its thread could not be created.
    Spawn(io::Error),
}

impl fmt::Display for SuperviseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SuperviseError::ShutDown => formatter.write_str("the supervisor has been shut down"),
            SuperviseError::Spawn(error) => {
                write!(
                    formatter,
                    "the supervisor's thread could not be created: {error}"
                )
            }
        }
    }
}

impl Error for SuperviseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            SuperviseError::ShutDown => None,
            SuperviseError::Spawn(error) => Some(error),
        }
    }
}

impl Supervisor {
    /// A supervisor. It starts no thread until it has a job.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::default(),
                changed: Condvar::new(),
                report: Mutex::new(None),
            }),
        }
    }

    /// Hands `job` to the supervisor, starting its thread with the first job.
    /// A caller registers a job before starting the work it owns, so no work
    /// starts that nothing cleans up.
    ///
    /// # Errors
    ///
    /// [`SuperviseError::ShutDown`] once `shutdown` has begun, and
    /// [`SuperviseError::Spawn`] if the thread cannot be created; the job is
    /// dropped unpolled either way.
    pub fn supervise(&self, job: Box<dyn SupervisedJob>) -> Result<(), SuperviseError> {
        let shared = &self.shared;
        let mut state = shared.lock();
        if state.closed {
            return Err(SuperviseError::ShutDown);
        }
        if !state.running {
            let thread_shared = Arc::clone(shared);
            thread::Builder::new()
                .name("gwz-host-supervisor".into())
                .spawn(move || thread_shared.run())
                .map_err(SuperviseError::Spawn)?;
            state.running = true;
        }
        state.jobs.push(job);
        shared.changed.notify_all();
        Ok(())
    }

    /// Shuts the supervisor down within `bound` (reuse design §7): no new
    /// job; a wait of at most `bound` for its jobs to finish; then the thread
    /// polls what remains to its end, drops the quarantined jobs and stops.
    /// Returns what remained at the bound: the jobs still running and the
    /// quarantined ones. It returns within the bound even when a job never
    /// finishes.
    ///
    /// A later call returns the first call's count and disposes nothing
    /// again; a concurrent call waits for the first.
    pub fn shutdown(&self, bound: Duration) -> usize {
        let mut report = self
            .shared
            .report
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(remaining) = *report {
            return remaining;
        }
        let remaining = self.shared.dispose(Instant::now() + bound);
        *report = Some(remaining);
        remaining
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        // The disposal `shutdown` makes, without its wait or its report.
        // After a `shutdown` the thread is released already.
        self.shared.release();
    }
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Supervisor").finish_non_exhaustive()
    }
}

impl Shared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The disposal when the supervisor drops: no new job, and the thread
    /// polls the rest to their end, then drops the quarantined ones and
    /// stops. It does not wait.
    fn release(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.released = true;
        drop(state);
        self.changed.notify_all();
    }

    /// `shutdown`'s disposal: the same, after waiting until `deadline` for
    /// the jobs to finish. The wait frees the lock the thread needs. Returns
    /// the jobs that remain, still running or quarantined, counted before the
    /// release lets the thread drop any.
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

    /// The thread: polls its jobs, quarantining any whose poll panics, and
    /// stops once it has been released and none is left to poll. The jobs
    /// are polled, and dropped, outside the lock.
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
                    .wait_timeout(state, POLL_INTERVAL)
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
/// contained, so it cannot stop the thread.
fn discard(job: Box<dyn SupervisedJob>) {
    let _ = catch_unwind(AssertUnwindSafe(move || drop(job)));
}

mod tests;
