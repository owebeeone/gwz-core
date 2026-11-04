//! The runtime's connection host (plan step 1.3b; contract sections 4.1 and 12 of dev-docs/GwzConnectionApiContract.md):
//! the `gwz_connection` host a `TransportRuntime` builds in `build_native` and closes in `shutdown`, and the
//! composition's ports it is built on.
//!
//! The ports: one [`DriverThread`] is the host's `Clock` (milliseconds from its own origin), its `Progress` (it runs
//! the host's driver task and its close jobs) and its cleanup domain's `RetentionPort` (it steps retained disposals
//! until they are confirmed); the `OwnerSeed` is 128 random bits drawn once at build. The thread starts the first time
//! any of them is used, so a runtime whose host never attaches an endpoint starts none, and it ends when the last
//! port reference goes (the host, its domain and everything they retain). Nothing here is a global.
//!
//! The clock's epoch (contract section 10) is the wall clock against the monotonic origin: the epoch moves when the two
//! have drifted apart by more than the idle interval since it last moved, as they do over a suspend, which the host
//! then answers by retiring its idle connections. The wall clock is a parameter of the thread ([`WallClock`]); the
//! cleanup domain's budgets are a parameter of the host ([`HostParts`]).
use crate::git::endpoint::ssh_key_snapshot::Registry;
use gwz_connection::{
    CleanupDomainOwner, CleanupStatus, Clock, DomainConfig, DriverWork, EndpointFactory,
    HostConfig, HostHandle, HostLimits, HostNamespace, HostOwner, HostPorts, OwnerSeed, Progress,
    ProgressUnavailable, RetainedWork, RetentionPort, RetentionRefused, SharedProcessSafety, Sleep,
    Tick,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The wall clock, in milliseconds since the Unix epoch (negative before it). A suspend moves it and not the monotonic
/// origin, which is how the driver sees one.
pub(crate) type WallClock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// The operating system's wall clock.
pub(crate) fn system_wall() -> WallClock {
    Arc::new(|| match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    })
}

/// The suspend and wall-clock-jump detector behind `Clock::epoch`: the counter moves when the wall clock and the
/// monotonic origin have drifted apart by more than `threshold_ms` since the counter last moved (or since the origin).
/// The baseline is re-set only when the counter moves, so several drifts that are each under the threshold add up, as
/// several short suspends age an idle connection together.
struct Epoch {
    wall: WallClock,
    threshold_ms: u64,
    state: Mutex<EpochState>,
}
struct EpochState {
    counter: u64,
    /// The monotonic and wall readings at the origin or at the last move.
    base_mono_ms: u64,
    base_wall_ms: i64,
}
impl Epoch {
    fn new(wall: WallClock, threshold_ms: u64, mono_ms: u64) -> Self {
        let base_wall_ms = wall();
        Self {
            wall,
            threshold_ms,
            state: Mutex::new(EpochState {
                counter: 0,
                base_mono_ms: mono_ms,
                base_wall_ms,
            }),
        }
    }
    /// The counter at `mono_ms` on the monotonic origin, moved first if the drift calls for it. The wall clock is read
    /// before the lock is taken.
    fn read(&self, mono_ms: u64) -> u64 {
        let wall_ms = (self.wall)();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mono_elapsed = i128::from(mono_ms) - i128::from(state.base_mono_ms);
        let wall_elapsed = i128::from(wall_ms) - i128::from(state.base_wall_ms);
        if (wall_elapsed - mono_elapsed).unsigned_abs() > u128::from(self.threshold_ms) {
            state.counter += 1;
            state.base_mono_ms = mono_ms;
            state.base_wall_ms = wall_ms;
        }
        state.counter
    }
}

/// The longest the thread parks with nothing due: every wake source unparks it sooner.
const PARK: Duration = Duration::from_secs(1);
/// How often retained disposals are stepped.
const RETAIN_STEP: Duration = Duration::from_millis(50);

struct Task {
    work: DriverWork,
    wake: Arc<TaskWake>,
}
struct TaskWake {
    woken: AtomicBool,
    inner: Weak<Inner>,
}
impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
        if let Some(inner) = self.inner.upgrade() {
            inner.unpark();
        }
    }
}
#[derive(Default)]
struct State {
    tasks: Vec<Task>,
    /// (deadline, sleep, waker) of every pending sleep.
    timers: Vec<(u64, u64, Waker)>,
    next_sleep: u64,
    retained: Vec<RetainedWork>,
    thread: Option<Thread>,
    stopped: bool,
}
struct Inner {
    origin: Instant,
    epoch: Epoch,
    state: Mutex<State>,
}
impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn now(&self) -> u64 {
        self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn unpark(&self) {
        if let Some(thread) = &self.lock().thread {
            thread.unpark();
        }
    }
}

/// The composition's executor, clock and retainer for one connection host. See the module doc.
pub(crate) struct DriverThread {
    inner: Arc<Inner>,
}
impl DriverThread {
    /// A thread whose epoch moves when `wall` drifts from the monotonic origin by more than `epoch_threshold_ms`.
    pub(crate) fn new(wall: WallClock, epoch_threshold_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                origin: Instant::now(),
                epoch: Epoch::new(wall, epoch_threshold_ms, 0),
                state: Mutex::new(State::default()),
            }),
        })
    }
    /// Runs `change` on the state, starting the thread first if it has not started; `false` once stopped.
    fn with_started(&self, change: impl FnOnce(&mut State)) -> Result<(), ProgressUnavailable> {
        let mut state = self.inner.lock();
        if state.stopped {
            return Err(ProgressUnavailable);
        }
        if state.thread.is_none() {
            let inner = self.inner.clone();
            let join = thread::Builder::new()
                .name("gwz-connection-driver".into())
                .spawn(move || run(inner))
                .map_err(|_| ProgressUnavailable)?;
            state.thread = Some(join.thread().clone());
        }
        change(&mut state);
        if let Some(thread) = &state.thread {
            thread.unpark();
        }
        Ok(())
    }
}
impl Drop for DriverThread {
    fn drop(&mut self) {
        let mut state = self.inner.lock();
        state.stopped = true;
        if let Some(thread) = &state.thread {
            thread.unpark();
        }
    }
}

fn run(inner: Arc<Inner>) {
    loop {
        let now = inner.now();
        let (ready, due, next, retained) = {
            let mut state = inner.lock();
            if state.stopped {
                return;
            }
            let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut state.tasks)
                .into_iter()
                .partition(|task| task.wake.woken.swap(false, Ordering::AcqRel));
            state.tasks = waiting;
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut state.timers)
                .into_iter()
                .partition(|(at, _, _)| *at <= now);
            state.timers = later;
            let next = state.timers.iter().map(|(at, _, _)| *at).min();
            (ready, due, next, std::mem::take(&mut state.retained))
        };
        for (_, _, waker) in due {
            waker.wake();
        }
        let mut pending = Vec::new();
        for mut task in ready {
            let waker = Waker::from(task.wake.clone());
            if task
                .work
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
            {
                pending.push(task);
            }
        }
        let mut retained = retained;
        retained.retain_mut(|work| !work.step());
        let park = {
            let mut state = inner.lock();
            state.tasks.extend(pending);
            let stepping = !retained.is_empty();
            state.retained.extend(retained);
            let ready = state
                .tasks
                .iter()
                .any(|task| task.wake.woken.load(Ordering::Acquire));
            let until = next.map_or(PARK, |at| Duration::from_millis(at.saturating_sub(now)));
            if ready {
                Duration::ZERO
            } else if stepping {
                until.min(RETAIN_STEP)
            } else {
                until.min(PARK)
            }
        };
        if !park.is_zero() {
            thread::park_timeout(park);
        }
    }
}

struct DriverSleep {
    inner: Arc<Inner>,
    deadline: u64,
    id: Option<u64>,
    owner: Weak<DriverThread>,
}
impl Future for DriverSleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.inner.now() >= self.deadline {
            return Poll::Ready(());
        }
        let Some(owner) = self.owner.upgrade() else {
            // No clock thread is left to wake this: it would never end.
            return Poll::Ready(());
        };
        let (deadline, id) = (self.deadline, self.id);
        let mut assigned = id;
        let registered = owner.with_started(|state| {
            let id = id.unwrap_or_else(|| {
                state.next_sleep += 1;
                state.next_sleep
            });
            state.timers.retain(|(_, sleep, _)| *sleep != id);
            state.timers.push((deadline, id, cx.waker().clone()));
            assigned = Some(id);
        });
        self.id = assigned;
        if registered.is_err() {
            return Poll::Ready(());
        }
        Poll::Pending
    }
}
impl Drop for DriverSleep {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.inner
                .lock()
                .timers
                .retain(|(_, sleep, _)| *sleep != id);
        }
    }
}

/// The clock, the executor and the retainer share one thread; the clock's port needs the `Arc` it is held by.
pub(crate) struct DriverPorts(Arc<DriverThread>);
impl Clock for DriverPorts {
    fn now(&self) -> Tick {
        Tick::new(self.0.inner.now())
    }
    fn sleep_until(&self, deadline: Tick) -> Sleep {
        Box::pin(DriverSleep {
            inner: self.0.inner.clone(),
            deadline: deadline.get(),
            id: None,
            owner: Arc::downgrade(&self.0),
        })
    }
    fn epoch(&self) -> u64 {
        self.0.inner.epoch.read(self.0.inner.now())
    }
}
impl Progress for DriverPorts {
    fn check(&self) -> Result<(), ProgressUnavailable> {
        if self.0.inner.lock().stopped {
            Err(ProgressUnavailable)
        } else {
            Ok(())
        }
    }
    fn spawn(&self, work: DriverWork) -> Result<(), ProgressUnavailable> {
        let wake = Arc::new(TaskWake {
            woken: AtomicBool::new(true),
            inner: Arc::downgrade(&self.0.inner),
        });
        self.0
            .with_started(|state| state.tasks.push(Task { work, wake }))
    }
}
impl RetentionPort for DriverPorts {
    fn retain(&self, work: RetainedWork) -> Result<(), RetentionRefused> {
        let mut work = Some(work);
        let taken = self.0.with_started(|state| {
            if let Some(work) = work.take() {
                state.retained.push(work);
            }
        });
        match (taken, work) {
            (Err(_), Some(work)) => Err(RetentionRefused::new(work)),
            _ => Ok(()),
        }
    }
}

/// A host namespace drawn once from the operating system's random source.
struct RandomSeed(HostNamespace);
impl OwnerSeed for RandomSeed {
    fn namespace(&self) -> HostNamespace {
        self.0
    }
}

/// The runtime's connection host: its owner, its handle and its cleanup domain's owner.
pub(crate) struct ConnectionHost {
    #[allow(
        dead_code,
        reason = "requests reach the host through its handle from plan phase 3; step 1.3b's rows use it now"
    )]
    handle: HostHandle,
    owner: Option<HostOwner>,
    domain: Option<CleanupDomainOwner>,
    #[allow(
        dead_code,
        reason = "the selected-key admission of plan step 1.5 admits keys here; step 1.3b's rows admit theirs through it"
    )]
    ssh_keys: Option<Registry>,
}
/// What the composition chooses for a host besides its factory and gate: its cleanup domain's budgets (contract section
/// 10: setup jobs, retained cleanup records and helper slots, which its adapters draw on) and the wall clock its epoch
/// is read from.
pub(crate) struct HostParts {
    pub(crate) domain: DomainConfig,
    pub(crate) wall: WallClock,
}
impl HostParts {
    /// The product's: the contract's 64 / 64 / 8 and the operating system's wall clock.
    pub(crate) fn product() -> Self {
        Self {
            domain: DomainConfig::default(),
            wall: system_wall(),
        }
    }
}
impl ConnectionHost {
    /// Builds the host over `factory` on the process's gate. No thread starts and nothing physical is made.
    /// `ssh_keys` is the registry the factory's SSH adapters look selected keys up in.
    pub(crate) fn build(
        factory: Arc<dyn EndpointFactory>,
        ssh_keys: Option<Registry>,
        safety: SharedProcessSafety,
    ) -> Result<Self, String> {
        Self::build_on(factory, ssh_keys, safety, HostParts::product())
    }
    /// [`ConnectionHost::build`] with the cleanup domain's budgets and the wall clock `parts` gives.
    pub(crate) fn build_on(
        factory: Arc<dyn EndpointFactory>,
        ssh_keys: Option<Registry>,
        safety: SharedProcessSafety,
        parts: HostParts,
    ) -> Result<Self, String> {
        let mut seed = [0u8; 16];
        getrandom::fill(&mut seed).map_err(|error| format!("host namespace: {error}"))?;
        // The idle interval is the instances' pools' (the crate's default), the discrepancy a suspend must exceed.
        let idle_ms = gwz_transport::pool::Config::default().idle_timeout_ms;
        let ports = Arc::new(DriverPorts(DriverThread::new(parts.wall, idle_ms)));
        let (domain_owner, domain) = gwz_connection::cleanup_domain(parts.domain, ports.clone())
            .map_err(|error| error.to_string())?;
        let (owner, handle) = gwz_connection::build(
            HostConfig {
                domain,
                safety,
                limits: HostLimits::default(),
            },
            HostPorts::new(
                ports.clone(),
                Arc::new(RandomSeed(HostNamespace::new(u128::from_le_bytes(seed)))),
                ports,
                factory,
            ),
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            handle,
            owner: Some(owner),
            domain: Some(domain_owner),
            ssh_keys,
        })
    }
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            pub(crate) fn handle(&self) -> HostHandle {
                self.handle.clone()
            }
            pub(crate) fn ssh_keys(&self) -> Option<Registry> {
                self.ssh_keys.clone()
            }
        }
    }
    /// Closes the host, then its domain: whether every disposal was confirmed within the bound.
    pub(crate) async fn close(&mut self) -> bool {
        let mut complete = true;
        if let Some(owner) = self.owner.take() {
            complete &= matches!(owner.close().await, CleanupStatus::Complete);
        }
        if let Some(domain) = self.domain.take() {
            complete &= matches!(domain.close().await, CleanupStatus::Complete);
        }
        complete
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod connect_deadline_tests;
        mod counting_adapter;
        mod domain_ports_tests;
        mod factory_profile_tests;
        mod https_auth_tests;
        mod https_idle_lease_tests;
        mod https_bound_tests;
        mod https_needs_tests;
        mod https_tests;
        mod idle_lease_tests;
        mod sleep_poll_tests;
        mod stream_wake_tests;
        mod tests;
        // Real SSPI through the host's HTTPS adapter: Windows only.
        cfg_if::cfg_if! { if #[cfg(windows)] { mod https_sspi_tests; } }
    }
}
