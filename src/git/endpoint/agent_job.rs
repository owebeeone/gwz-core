//! Bounded ownership of setup threads, including abandoned jobs, by one host's `Supervisor`.
use gwz_transport::protocol::SetupFailureCause;
use std::{
    fmt, io,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};
pub(super) const LIMIT: usize = 64;
mod supervisor;
use supervisor::{CleanupPermit, Permit};
pub(crate) use supervisor::{Reservation, Supervisor};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeoutReason {
    Stall,
    Aggregate,
}
impl TimeoutReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            TimeoutReason::Stall => "stall",
            TimeoutReason::Aggregate => "aggregate",
        }
    }
    pub(crate) fn setup_cause(self) -> SetupFailureCause {
        match self {
            TimeoutReason::Stall => SetupFailureCause::Stall,
            TimeoutReason::Aggregate => SetupFailureCause::Aggregate,
        }
    }
}
#[derive(Debug)]
pub(crate) struct SetupTimeout {
    pub(crate) reason: TimeoutReason,
}
impl fmt::Display for SetupTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason.label())
    }
}
impl std::error::Error for SetupTimeout {}
pub(crate) fn timeout_reason(error: &io::Error) -> Option<TimeoutReason> {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<SetupTimeout>())
        .map(|timeout| timeout.reason)
}
pub(crate) fn wall_clock() -> Arc<dyn Fn() -> Instant + Send + Sync> {
    Arc::new(Instant::now)
}
struct Cell<T> {
    control: Arc<Control>,
    result: Mutex<Option<io::Result<T>>>,
    /// The job's place in the budget. Its thread has been joined once its
    /// result is taken or disposed of, so either frees the permit at once.
    permit: Mutex<Option<Permit>>,
}
impl<T> Cell<T> {
    fn release(&self) {
        let permit = self.permit.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(permit);
    }
}
trait Reap: Send {
    fn reap(&mut self) -> bool;
}
struct Entry<T> {
    cell: Arc<Cell<T>>,
    join: Option<JoinHandle<()>>,
}
impl<T: Send + 'static> Reap for Entry<T> {
    fn reap(&mut self) -> bool {
        if self.join.as_ref().is_some_and(|join| !join.is_finished()) {
            let control = &self.cell.control;
            control.sync_shared();
            let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
            control.update(&mut state);
            let wake = if state.cancelled_at.is_some_and(|at| control.cleanup_due(at)) {
                state.waker.take()
            } else {
                None
            };
            drop(state);
            if let Some(wake) = wake {
                wake.wake();
            }
            return false;
        }
        let panic = self.join.take().is_some_and(|join| join.join().is_err());
        let control = &self.cell.control;
        control.sync_shared();
        let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
        control.update(&mut state);
        if panic && state.failure.is_none() {
            state.failure = Some(Fail {
                kind: io::ErrorKind::Other,
                reason: None,
                terminal: None,
            });
        }
        if state.failure.is_some() && !state.consumed {
            let discarded = self
                .cell
                .result
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            // Never run native result destructors with the arbitration lock held.
            drop(state);
            drop(discarded);
            state = control.state.lock().unwrap_or_else(|e| e.into_inner());
            state.consumed = true;
        }
        state.joined = true;
        let wake = state.waker.take();
        let done = state.consumed;
        drop(state);
        if done {
            self.cell.release();
        }
        if let Some(wake) = wake {
            wake.wake();
        }
        done
    }
}
/// T must have bounded, non-panicking destruction (the native connection owner
/// terminates its socket before destruction). No native session clones allowed.
pub(crate) struct Job<T: Send + 'static> {
    cell: Arc<Cell<T>>,
    worker: Thread,
}
/// Where a job's place in the budget comes from.
pub(crate) enum Place<'a> {
    /// Take one now, `WouldBlock` when the supervisor's budget is full.
    Take(&'a Supervisor),
    /// A place taken ahead (`Supervisor::reserve`).
    Reserved(Reservation),
}
impl<T: Send + 'static> Job<T> {
    pub(crate) fn start(
        supervisor: &Supervisor,
        deadline: Option<Instant>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_in(Place::Take(supervisor), deadline, cleanup, work)
    }
    /// `start` on a given place.
    pub(crate) fn start_in(
        place: Place<'_>,
        deadline: Option<Instant>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_timed(place, deadline, Duration::ZERO, cleanup, wall_clock(), work)
    }
    pub(crate) fn start_timed(
        place: Place<'_>,
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_inner(
            place,
            aggregate,
            stall,
            cleanup,
            clock,
            work,
            |name, body| thread::Builder::new().name(name.into()).spawn(body),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn start_inner(
        place: Place<'_>,
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
        spawn: impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<Self> {
        Self::start_control(
            place,
            Arc::new(Control::new(aggregate, stall, cleanup, clock)),
            work,
            spawn,
        )
    }
    pub(crate) fn start_setup(
        place: Place<'_>,
        setup: Arc<super::ssh_setup_context::SetupContext>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        let control = Arc::new(Control::new_shared(setup.clone(), cleanup, wall_clock()));
        Self::start_control(
            place,
            control,
            move |control| {
                work(control).map_err(|error| {
                    if error
                        .get_ref()
                        .is_some_and(|cause| cause.is::<super::ssh_setup_context::SetupEnded>())
                    {
                        error
                    } else {
                        setup.terminate_failure(super::ssh_setup::failure_from_io(&error))
                    }
                })
            },
            |name, body| thread::Builder::new().name(name.into()).spawn(body),
        )
    }
    fn start_control(
        place: Place<'_>,
        control: Arc<Control>,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
        mut spawn: impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<Self> {
        let (supervisor, reservation) = match place {
            Place::Take(supervisor) => (supervisor.clone(), None),
            Place::Reserved(reservation) => (reservation.supervisor().clone(), Some(reservation)),
        };
        let worker = supervisor.reaper(&mut spawn)?;
        let permit = match reservation {
            Some(Reservation(permit)) => permit,
            None => supervisor.take()?,
        };
        let cell = Arc::new(Cell {
            control: control.clone(),
            result: Mutex::new(None),
            permit: Mutex::new(Some(permit)),
        });
        let target = cell.clone();
        let wake = worker.clone();
        let join = spawn(
            "gwz-agent-setup",
            Box::new(move || {
                let result = control.check().and_then(|_| work(control.clone()));
                control.sync_shared();
                let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
                control.update(&mut state);
                // Even cancelled results stay owned until the supervisor joins and disposes.
                *target.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                drop(state);
                wake.unpark();
            }),
        )?;
        supervisor
            .shared
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(Entry {
                cell: cell.clone(),
                join: Some(join),
            }));
        worker.unpark();
        Ok(Self { cell, worker })
    }
    /// How long the job's socket connect took, once it has completed.
    pub(crate) fn tcp_connect_ms(&self) -> Option<u64> {
        self.cell.control.tcp_connect_ms()
    }
    pub(crate) fn cancel(&self) {
        self.cell.control.cancel();
        self.worker.unpark();
    }
    pub(crate) fn poll_result(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        let control = &self.cell.control;
        control.sync_shared();
        let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
        control.update(&mut state);
        state.waker = Some(cx.waker().clone());
        if !state.joined || (state.failure.is_some() && !state.consumed) {
            self.worker.unpark();
            return Poll::Pending;
        }
        if let Some(error) = state.failure {
            return Poll::Ready(Err(error.into_io()));
        }
        let result = self
            .cell
            .result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| Err(io::ErrorKind::BrokenPipe.into()));
        state.consumed = true;
        drop(state);
        self.cell.release();
        self.worker.unpark();
        Poll::Ready(result)
    }
    /// Error means cleanup is overdue and still owned, never a disposal ack.
    pub(crate) fn poll_disposed(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.cancel();
        let mut state = self
            .cell
            .control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.joined && state.consumed {
            return Poll::Ready(Ok(()));
        }
        state.waker = Some(cx.waker().clone());
        if state
            .cancelled_at
            .is_some_and(|at| self.cell.control.cleanup_due(at))
        {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        Poll::Pending
    }
}
impl<T: Send + 'static> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

// Each active endpoint reserves its eventual cleanup record before spawning.
// Retaining a stopped pool never allocates another thread or another permit.
pub(crate) struct Cleanup {
    supervisor: Supervisor,
    worker: Thread,
    permit: CleanupPermit,
}
impl Cleanup {
    pub(crate) fn reserve(supervisor: &Supervisor) -> io::Result<Self> {
        let worker = supervisor.reaper_spawn()?;
        let permit = supervisor.take_cleanup()?;
        Ok(Self {
            supervisor: supervisor.clone(),
            worker,
            permit,
        })
    }
    /// Poll must be bounded; true means physical cleanup and ledger closure.
    pub(crate) fn retain(self, poll: impl FnMut() -> bool + Send + 'static) {
        struct Retained<F> {
            poll: F,
            _permit: CleanupPermit,
            poisoned: bool,
        }
        impl<F: FnMut() -> bool + Send> Reap for Retained<F> {
            fn reap(&mut self) -> bool {
                if self.poisoned {
                    return false;
                }
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut self.poll)) {
                    Ok(done) => done,
                    Err(_) => {
                        self.poisoned = true;
                        false
                    } // retain ownership, never claim disposal
                }
            }
        }
        self.supervisor
            .shared
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(Retained {
                poll,
                _permit: self.permit,
                poisoned: false,
            }));
        self.worker.unpark();
    }
}

mod control;
pub(crate) use control::Control;
use control::Fail;
cfg_if::cfg_if! { if #[cfg(test)] { mod tests; pub(crate) use tests::ManualClock; } }
