//! The job budget, its waiters and the reaper of one transport host's SSH and
//! HTTPS setups (adaptive concurrency design §7.5, OQ9).
//!
//! A [`Supervisor`] is an explicit context, created by the host that owns the
//! endpoints and passed to every job it starts: the budget of [`LIMIT`] jobs,
//! the wakers of setups waiting for a place, the cleanup records, and the one
//! reaper thread that joins and disposes of finished jobs. Nothing of it is
//! process-wide. Endpoints that share a host share its budget; hosts do not
//! share one.
use super::{LIMIT, Reap};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Waker,
    thread::{self, JoinHandle, Thread},
    time::Duration,
};

pub(super) type Spawn<'a> =
    &'a mut dyn FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>;

struct Reaper {
    thread: Thread,
    _join: JoinHandle<()>,
}
pub(super) struct Shared {
    count: AtomicUsize,
    cleanups: AtomicUsize,
    pub(super) entries: Mutex<Vec<Box<dyn Reap>>>,
    /// Wakers of the setups waiting for a place in the job budget.
    waiters: Mutex<Vec<Waker>>,
    reaper: Mutex<Option<Reaper>>,
    closed: AtomicBool,
}
/// Dropped with the last handle: the reaper then ends once it has disposed of
/// every job it still holds.
struct Closer(Arc<Shared>);
impl Drop for Closer {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        if let Some(reaper) = self
            .0
            .reaper
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            reaper.thread.unpark();
        }
    }
}
/// The context of one host's setup jobs. Clones share one budget.
#[derive(Clone)]
pub(crate) struct Supervisor {
    pub(super) shared: Arc<Shared>,
    _closer: Arc<Closer>,
}
impl Supervisor {
    pub(crate) fn new() -> Self {
        let shared = Arc::new(Shared {
            count: AtomicUsize::new(0),
            cleanups: AtomicUsize::new(0),
            entries: Mutex::new(Vec::new()),
            waiters: Mutex::new(Vec::new()),
            reaper: Mutex::new(None),
            closed: AtomicBool::new(false),
        });
        Self {
            _closer: Arc::new(Closer(shared.clone())),
            shared,
        }
    }
    /// The reaper's thread, started by the first job or cleanup that needs it.
    pub(super) fn reaper(&self, spawn: Spawn<'_>) -> io::Result<Thread> {
        let mut reaper = self.shared.reaper.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reaper) = reaper.as_ref() {
            return Ok(reaper.thread.clone());
        }
        let shared = self.shared.clone();
        let join = spawn(
            "gwz-setup-reaper",
            Box::new(move || {
                loop {
                    let mut batch = std::mem::take(
                        &mut *shared.entries.lock().unwrap_or_else(|e| e.into_inner()),
                    );
                    batch.retain_mut(|entry| !entry.reap());
                    let empty = {
                        let mut entries = shared.entries.lock().unwrap_or_else(|e| e.into_inner());
                        entries.append(&mut batch);
                        entries.is_empty()
                    };
                    if empty && shared.closed.load(Ordering::Acquire) {
                        return;
                    }
                    if empty {
                        thread::park();
                    } else {
                        thread::park_timeout(Duration::from_millis(20));
                    }
                }
            }),
        )?;
        let thread = join.thread().clone();
        *reaper = Some(Reaper {
            thread: thread.clone(),
            _join: join,
        });
        Ok(thread)
    }
    pub(super) fn reaper_spawn(&self) -> io::Result<Thread> {
        self.reaper(&mut |name, body| thread::Builder::new().name(name.into()).spawn(body))
    }
    /// A place now, or `WouldBlock` with `waker` woken when one is returned.
    pub(crate) fn reserve(&self, waker: &Waker) -> io::Result<Reservation> {
        if let Ok(permit) = self.take() {
            return Ok(Reservation(permit));
        }
        {
            let mut waiters = self
                .shared
                .waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !waiters.iter().any(|queued| queued.will_wake(waker)) {
                waiters.push(waker.clone());
            }
        }
        // A place may have been returned between the refusal and the
        // registration, whose wake would then never come.
        self.take().map(Reservation)
    }
    /// `WouldBlock` when every place is taken.
    pub(super) fn take(&self) -> io::Result<Permit> {
        self.shared
            .count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        Ok(Permit(self.clone()))
    }
    pub(super) fn take_cleanup(&self) -> io::Result<CleanupPermit> {
        self.shared
            .cleanups
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        Ok(CleanupPermit(self.clone()))
    }
    fn wake_waiters(&self) {
        let waiters = std::mem::take(
            &mut *self
                .shared
                .waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for waiter in waiters {
            waiter.wake();
        }
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        /// The places taken now.
        pub(crate) fn taken(&self) -> usize {
            self.shared.count.load(Ordering::Acquire)
        }
    } }
}
/// A place in the job budget. Returning it wakes whoever waits for one.
pub(super) struct Permit(Supervisor);
impl Permit {
    pub(super) fn supervisor(&self) -> &Supervisor {
        &self.0
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.shared.count.fetch_sub(1, Ordering::AcqRel);
        self.0.wake_waiters();
    }
}
/// A place in the job budget taken ahead of the setup that will use it, so a
/// setup that finds the budget full can wait for a place with its work still
/// in hand, and start it with no further refusal. A full budget is
/// backpressure, never a failure (adaptive concurrency design §7.5).
pub(crate) struct Reservation(pub(super) Permit);
impl Reservation {
    pub(crate) fn supervisor(&self) -> &Supervisor {
        self.0.supervisor()
    }
}
/// Each active endpoint reserves its eventual cleanup record before spawning.
pub(super) struct CleanupPermit(Supervisor);
impl Drop for CleanupPermit {
    fn drop(&mut self) {
        self.0.shared.cleanups.fetch_sub(1, Ordering::AcqRel);
    }
}
