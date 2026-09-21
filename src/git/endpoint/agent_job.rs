//! Bounded process-local ownership of setup threads, including abandoned jobs.
use std::{
    io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};
const LIMIT: usize = 64;
static COUNT: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        COUNT.fetch_sub(1, Ordering::AcqRel);
    }
}
struct State {
    failure: Option<io::ErrorKind>,
    cancelled_at: Option<Instant>,
    joined: bool,
    consumed: bool,
    waker: Option<Waker>,
}
pub(crate) struct Control {
    state: Mutex<State>,
    deadline: Option<Instant>,
    cleanup: Duration,
}
impl Control {
    fn update(&self, state: &mut State) {
        if !state.consumed
            && state.failure.is_none()
            && self.deadline.is_some_and(|at| Instant::now() >= at)
        {
            state.failure = Some(io::ErrorKind::TimedOut);
            state.cancelled_at = Some(Instant::now());
        }
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        state.failure.map_or(Ok(()), |kind| Err(kind.into()))
    }
    pub(crate) fn quantum(&self) -> io::Result<Duration> {
        self.check()?;
        Ok(self.deadline.map_or(Duration::from_millis(20), |at| {
            at.saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20))
        }))
    }
    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if !state.consumed && state.failure.is_none() {
            state.failure = Some(io::ErrorKind::ConnectionAborted);
            state.cancelled_at = Some(Instant::now());
        }
    }
}
struct Cell<T> {
    control: Arc<Control>,
    result: Mutex<Option<io::Result<T>>>,
}
trait Reap: Send {
    fn reap(&mut self) -> bool;
}
struct Entry<T> {
    cell: Arc<Cell<T>>,
    join: Option<JoinHandle<()>>,
    _permit: Permit,
}
impl<T: Send + 'static> Reap for Entry<T> {
    fn reap(&mut self) -> bool {
        if self.join.as_ref().is_some_and(|join| !join.is_finished()) {
            let control = &self.cell.control;
            let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
            control.update(&mut state);
            let wake = if state
                .cancelled_at
                .is_some_and(|at| at.elapsed() >= control.cleanup)
            {
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
        let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
        control.update(&mut state);
        if panic && state.failure.is_none() {
            state.failure = Some(io::ErrorKind::Other);
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
        if let Some(wake) = wake {
            wake.wake();
        }
        done
    }
}
struct Hub {
    entries: Arc<Mutex<Vec<Box<dyn Reap>>>>,
    worker: Thread,
    _join: JoinHandle<()>,
}
impl Hub {
    fn global() -> io::Result<&'static Self> {
        static HUB: OnceLock<Result<Hub, io::ErrorKind>> = OnceLock::new();
        HUB.get_or_init(|| {
            let entries = Arc::new(Mutex::new(Vec::<Box<dyn Reap>>::new()));
            let shared = entries.clone();
            let join = thread::Builder::new()
                .name("gwz-setup-reaper".into())
                .spawn(move || {
                    loop {
                        let mut batch =
                            std::mem::take(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()));
                        batch.retain_mut(|entry| !entry.reap());
                        shared
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .append(&mut batch);
                        if COUNT.load(Ordering::Acquire) == 0 {
                            thread::park();
                        } else {
                            thread::park_timeout(Duration::from_millis(20));
                        }
                    }
                })
                .map_err(|e| e.kind())?;
            Ok(Self {
                entries,
                worker: join.thread().clone(),
                _join: join,
            })
        })
        .as_ref()
        .map_err(|kind| (*kind).into())
    }
}
/// T must have bounded, non-panicking destruction (the native connection owner
/// terminates its socket before destruction). No native session clones allowed.
pub(crate) struct Job<T: Send + 'static> {
    cell: Arc<Cell<T>>,
    hub: &'static Hub,
}
impl<T: Send + 'static> Job<T> {
    pub(crate) fn start(
        deadline: Option<Instant>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_with(deadline, cleanup, work, |body| {
            thread::Builder::new()
                .name("gwz-agent-setup".into())
                .spawn(body)
        })
    }
    // Private injection seam for deterministic thread-creation failure tests.
    pub(crate) fn start_with(
        deadline: Option<Instant>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<Self> {
        let hub = Hub::global()?;
        COUNT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        let permit = Permit;
        let control = Arc::new(Control {
            deadline,
            cleanup,
            state: Mutex::new(State {
                failure: None,
                cancelled_at: None,
                joined: false,
                consumed: false,
                waker: None,
            }),
        });
        let cell = Arc::new(Cell {
            control: control.clone(),
            result: Mutex::new(None),
        });
        let target = cell.clone();
        let wake = hub.worker.clone();
        let join = spawn(Box::new(move || {
            let result = control.check().and_then(|_| work(control.clone()));
            let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
            control.update(&mut state);
            // Even cancelled results stay owned until the supervisor joins and disposes.
            *target.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            drop(state);
            wake.unpark();
        }))?;
        hub.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(Entry {
                cell: cell.clone(),
                join: Some(join),
                _permit: permit,
            }));
        hub.worker.unpark();
        Ok(Self { cell, hub })
    }
    pub(crate) fn cancel(&self) {
        self.cell.control.cancel();
        self.hub.worker.unpark();
    }
    pub(crate) fn poll_result(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        let control = &self.cell.control;
        let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
        control.update(&mut state);
        state.waker = Some(cx.waker().clone());
        if !state.joined || (state.failure.is_some() && !state.consumed) {
            self.hub.worker.unpark();
            return Poll::Pending;
        }
        if let Some(error) = state.failure {
            return Poll::Ready(Err(error.into()));
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
        self.hub.worker.unpark();
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
            .is_some_and(|at| at.elapsed() >= self.cell.control.cleanup)
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
