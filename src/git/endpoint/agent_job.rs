//! Bounded process-local ownership of setup threads, including abandoned jobs.
use gwz_transport::protocol::SetupFailureCause;
use std::{
    fmt, io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};
pub(super) const LIMIT: usize = 64;
static COUNT: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        COUNT.fetch_sub(1, Ordering::AcqRel);
    }
}
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
#[derive(Clone)]
pub(crate) struct ManualClock {
    now: Arc<Mutex<Instant>>,
}
impl ManualClock {
    pub(crate) fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(Instant::now())),
        }
    }
    pub(crate) fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *now += by;
    }
    pub(crate) fn clock(&self) -> Arc<dyn Fn() -> Instant + Send + Sync> {
        let now = self.now.clone();
        Arc::new(move || *now.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
#[derive(Clone, Copy)]
struct Fail {
    kind: io::ErrorKind,
    reason: Option<TimeoutReason>,
}
impl Fail {
    fn into_io(self) -> io::Error {
        match self.reason {
            Some(reason) => io::Error::new(self.kind, SetupTimeout { reason }),
            None => self.kind.into(),
        }
    }
}
struct State {
    failure: Option<Fail>,
    cancelled_at: Option<Instant>,
    joined: bool,
    consumed: bool,
    waker: Option<Waker>,
    aggregate: Option<Instant>,
    wait_started: Option<Instant>,
    interacting: bool,
    interaction_started: Option<Instant>,
    paused_elapsed: Option<Duration>,
}
pub(crate) struct Control {
    state: Mutex<State>,
    stall: Duration,
    cleanup: Duration,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl Control {
    pub(crate) fn scripted(
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self::new(aggregate, stall, cleanup, clock))
    }
    fn new(
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                failure: None,
                cancelled_at: None,
                joined: false,
                consumed: false,
                waker: None,
                aggregate,
                wait_started: None,
                interacting: false,
                interaction_started: None,
                paused_elapsed: None,
            }),
            stall,
            cleanup,
            clock,
        }
    }
    fn now(&self) -> Instant {
        (self.clock)()
    }
    fn fail(&self, state: &mut State, reason: TimeoutReason) {
        state.failure = Some(Fail {
            kind: io::ErrorKind::TimedOut,
            reason: Some(reason),
        });
        state.cancelled_at = Some(self.now());
    }
    fn update(&self, state: &mut State) {
        if state.consumed || state.failure.is_some() || state.interacting {
            return;
        }
        let now = self.now();
        let stall_at = if self.stall > Duration::ZERO {
            state
                .wait_started
                .and_then(|start| start.checked_add(self.stall))
        } else {
            None
        };
        let aggregate_at = state.aggregate;
        let stall_due = stall_at.is_some_and(|at| now >= at);
        let aggregate_due = aggregate_at.is_some_and(|at| now >= at);
        let reason = match (stall_due, aggregate_due) {
            (true, true) => {
                if aggregate_at <= stall_at {
                    TimeoutReason::Aggregate
                } else {
                    TimeoutReason::Stall
                }
            }
            (false, true) => TimeoutReason::Aggregate,
            (true, false) => TimeoutReason::Stall,
            (false, false) => return,
        };
        self.fail(state, reason);
    }
    pub(crate) fn begin_wait(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if let Some(failure) = state.failure {
            return Err(failure.into_io());
        }
        if self.stall > Duration::ZERO && !state.interacting && state.wait_started.is_none() {
            state.wait_started = Some(self.now());
        }
        Ok(())
    }
    pub(crate) fn complete_wait(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if let Some(failure) = state.failure {
            return Err(failure.into_io());
        }
        let now = self.now();
        if self.stall > Duration::ZERO {
            if state.interacting {
                state.paused_elapsed = Some(Duration::ZERO);
            } else {
                state.wait_started = Some(now);
            }
        }
        Ok(())
    }
    pub(crate) fn begin_slice(&self) -> io::Result<Duration> {
        self.begin_wait()?;
        self.quantum()
    }
    pub(crate) fn end_slice(&self, ready: bool) -> io::Result<()> {
        if ready {
            self.complete_wait()?;
        }
        self.check()
    }
    pub(crate) fn wait_step(
        &self,
        poll: impl FnOnce(Duration) -> io::Result<bool>,
    ) -> io::Result<()> {
        let duration = self.begin_slice()?;
        let outcome = poll(duration);
        self.check()?;
        let ready = outcome?;
        self.end_slice(ready)
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        match state.failure {
            Some(failure) => Err(failure.into_io()),
            None => Ok(()),
        }
    }
    pub(crate) fn quantum(&self) -> io::Result<Duration> {
        self.check()?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut limit = Duration::from_millis(20);
        if let Some(left) = self.aggregate_remaining(&state) {
            limit = limit.min(left);
        }
        if let Some(left) = self.stall_remaining(&state) {
            limit = limit.min(left);
        }
        Ok(limit)
    }
    fn aggregate_remaining(&self, state: &State) -> Option<Duration> {
        let at = state.aggregate?;
        if state.interacting {
            let origin = state.interaction_started.unwrap_or(at);
            Some(at.saturating_duration_since(origin))
        } else {
            Some(at.saturating_duration_since(self.now()))
        }
    }
    fn stall_remaining(&self, state: &State) -> Option<Duration> {
        if self.stall == Duration::ZERO {
            return None;
        }
        if state.interacting {
            let elapsed = state.paused_elapsed.unwrap_or(Duration::ZERO);
            return Some(self.stall.saturating_sub(elapsed));
        }
        let start = state.wait_started?;
        let elapsed = self.now().saturating_duration_since(start);
        Some(self.stall.saturating_sub(elapsed))
    }
    pub(crate) fn begin_interaction(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if state.failure.is_some() || state.interacting {
            return;
        }
        let now = self.now();
        if let Some(start) = state.wait_started.take() {
            state.paused_elapsed = Some(now.saturating_duration_since(start));
        }
        state.interaction_started = Some(now);
        state.interacting = true;
    }
    pub(crate) fn end_interaction(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.interacting {
            return;
        }
        let now = self.now();
        if let Some(started) = state.interaction_started.take() {
            let paused = now.saturating_duration_since(started);
            if let Some(at) = state.aggregate {
                state.aggregate = at.checked_add(paused).or(Some(at));
            }
        }
        if let Some(elapsed) = state.paused_elapsed.take() {
            state.wait_started = now.checked_sub(elapsed);
        }
        state.interacting = false;
    }
    pub(crate) fn disposal_due(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cancelled_at.is_some_and(|at| self.cleanup_due(at))
    }
    fn cleanup_due(&self, at: Instant) -> bool {
        match at.checked_add(self.cleanup) {
            Some(due) => self.now() >= due,
            None => true,
        }
    }
    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if !state.consumed && state.failure.is_none() {
            state.failure = Some(Fail {
                kind: io::ErrorKind::ConnectionAborted,
                reason: None,
            });
            state.cancelled_at = Some(self.now());
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
        let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
        control.update(&mut state);
        if panic && state.failure.is_none() {
            state.failure = Some(Fail {
                kind: io::ErrorKind::Other,
                reason: None,
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
    fn global(
        spawn: &mut impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<&'static Self> {
        static HUB: OnceLock<Hub> = OnceLock::new();
        static INIT: Mutex<()> = Mutex::new(());
        if let Some(hub) = HUB.get() {
            return Ok(hub);
        }
        let _init = INIT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hub) = HUB.get() {
            return Ok(hub);
        }
        let hub = {
            let entries = Arc::new(Mutex::new(Vec::<Box<dyn Reap>>::new()));
            let shared = entries.clone();
            let join = spawn(
                "gwz-setup-reaper",
                Box::new(move || {
                    loop {
                        let mut batch =
                            std::mem::take(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()));
                        batch.retain_mut(|entry| !entry.reap());
                        shared
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .append(&mut batch);
                        if shared.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                            thread::park();
                        } else {
                            thread::park_timeout(Duration::from_millis(20));
                        }
                    }
                }),
            )?;
            Self {
                entries,
                worker: join.thread().clone(),
                _join: join,
            }
        };
        Ok(HUB.get_or_init(|| hub))
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
        Self::start_timed(deadline, Duration::ZERO, cleanup, wall_clock(), work)
    }
    pub(crate) fn start_timed(
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_inner(aggregate, stall, cleanup, clock, work, |name, body| {
            thread::Builder::new().name(name.into()).spawn(body)
        })
    }
    // Private injection seam for deterministic thread-creation failure tests.
    pub(crate) fn start_with(
        deadline: Option<Instant>,
        cleanup: Duration,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
        spawn: impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<Self> {
        Self::start_inner(deadline, Duration::ZERO, cleanup, wall_clock(), work, spawn)
    }
    fn start_inner(
        aggregate: Option<Instant>,
        stall: Duration,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
        mut spawn: impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<Self> {
        let hub = Hub::global(&mut spawn)?;
        COUNT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        let permit = Permit;
        let control = Arc::new(Control::new(aggregate, stall, cleanup, clock));
        let cell = Arc::new(Cell {
            control: control.clone(),
            result: Mutex::new(None),
        });
        let target = cell.clone();
        let wake = hub.worker.clone();
        let join = spawn(
            "gwz-agent-setup",
            Box::new(move || {
                let result = control.check().and_then(|_| work(control.clone()));
                let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
                control.update(&mut state);
                // Even cancelled results stay owned until the supervisor joins and disposes.
                *target.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                drop(state);
                wake.unpark();
            }),
        )?;
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
    pub(crate) fn begin_interaction(&self) {
        self.cell.control.begin_interaction();
    }
    pub(crate) fn end_interaction(&self) {
        self.cell.control.end_interaction();
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
static CLEANUPS: AtomicUsize = AtomicUsize::new(0);
struct CleanupPermit;
impl Drop for CleanupPermit {
    fn drop(&mut self) {
        CLEANUPS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) struct Cleanup {
    hub: &'static Hub,
    permit: CleanupPermit,
}
impl Cleanup {
    pub(crate) fn reserve() -> io::Result<Self> {
        let hub =
            Hub::global(&mut |name, body| thread::Builder::new().name(name.into()).spawn(body))?;
        CLEANUPS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        Ok(Self {
            hub,
            permit: CleanupPermit,
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
        self.hub
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(Retained {
                poll,
                _permit: self.permit,
                poisoned: false,
            }));
        self.hub.worker.unpark();
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        fn scripted(stall: Duration, aggregate: Duration) -> (ManualClock, Arc<Control>) {
            let clock = ManualClock::new();
            let aggregate = (aggregate > Duration::ZERO).then(|| clock.now() + aggregate);
            let control = Control::scripted(aggregate, stall, Duration::from_secs(5), clock.clock());
            (clock, control)
        }

        #[test]
        fn progressing_waits_outlive_one_stall_allowance() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            for _ in 0..4 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(600));
                control.end_slice(true).unwrap();
            }
            control.check().unwrap();
        }

        #[test]
        fn idle_slices_expire_as_stall_while_aggregate_remains() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            control.begin_wait().unwrap();
            let _ = control.quantum().unwrap();
            let _ = control.quantum().unwrap();
            clock.advance(Duration::from_millis(1_000));
            let error = control.check().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn completed_native_attempt_after_stall_is_rejected() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            control.begin_wait().unwrap();
            clock.advance(Duration::from_millis(1_001));
            let error = control.complete_wait().unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn terminal_stall_wins_over_a_poll_error() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            let error = control
                .wait_step(|_| {
                    clock.advance(Duration::from_millis(1_000));
                    Err(io::ErrorKind::ConnectionRefused.into())
                })
                .unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn handshake_completions_reset_the_stall() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            for _ in 0..3 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(700));
                control.end_slice(true).unwrap();
            }
            control.check().unwrap();
        }

        #[test]
        fn interaction_spends_neither_clock() {
            let (clock, control) = scripted(Duration::from_secs(1), Duration::from_secs(2));
            control.begin_interaction();
            clock.advance(Duration::from_secs(5));
            control.check().unwrap();
            control.end_interaction();
            control.begin_wait().unwrap();
            clock.advance(Duration::from_secs(1));
            let error = control.check().unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn disabled_stall_does_not_invent_a_deadline() {
            let (clock, control) = scripted(Duration::ZERO, Duration::from_secs(10));
            control.begin_wait().unwrap();
            clock.advance(Duration::from_secs(3));
            control.check().unwrap();
            clock.advance(Duration::from_millis(6_999));
            control.check().unwrap();
            clock.advance(Duration::from_millis(1));
            let error = control.check().unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
        }

        #[test]
        fn zero_aggregate_has_no_network_deadline() {
            let clock = ManualClock::new();
            let control = Control::scripted(None, Duration::ZERO, Duration::from_secs(5), clock.clock());
            control.begin_wait().unwrap();
            clock.advance(Duration::from_secs(30));
            control.complete_wait().unwrap();
            control.check().unwrap();
            control.cancel();
            assert!(!control.disposal_due());
            clock.advance(Duration::from_secs(5));
            assert!(control.disposal_due());
        }

        #[test]
        fn short_waits_fail_when_their_sum_passes_the_aggregate() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(2_500));
            for _ in 0..2 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(800));
                control.end_slice(true).unwrap();
            }
            control.begin_slice().unwrap();
            clock.advance(Duration::from_millis(900));
            let error = control.end_slice(true).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
        }

        #[test]
        fn cancel_drops_a_success_inside_the_aggregate() {
            let (clock, control) = scripted(Duration::from_secs(3), Duration::from_secs(10));
            for _ in 0..4 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(1_250));
                control.end_slice(true).unwrap();
            }
            control.cancel();
            let error = control.check().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
            assert!(timeout_reason(&error).is_none());
        }
    }
}
