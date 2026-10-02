use super::*;
#[derive(Clone, Copy)]
pub(super) struct Fail {
    pub(super) kind: io::ErrorKind,
    pub(super) reason: Option<TimeoutReason>,
    pub(super) terminal: Option<gwz_transport::pool::SetupTerminal>,
}
impl Fail {
    pub(super) fn into_io(self) -> io::Error {
        if let Some(record) = self.terminal {
            return io::Error::new(
                self.kind,
                super::super::ssh_setup_context::SetupEnded(record),
            );
        }
        match self.reason {
            Some(reason) => io::Error::new(self.kind, SetupTimeout { reason }),
            None => self.kind.into(),
        }
    }
}
pub(super) struct State {
    pub(super) failure: Option<Fail>,
    pub(super) cancelled_at: Option<Instant>,
    pub(super) joined: bool,
    pub(super) consumed: bool,
    pub(super) waker: Option<Waker>,
    pub(super) aggregate: Option<Instant>,
    pub(super) wait_started: Option<Instant>,
}
pub(crate) struct Control {
    pub(super) state: Mutex<State>,
    pub(super) stall: Duration,
    pub(super) cleanup: Duration,
    pub(super) clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    pub(super) setup: Option<Arc<super::super::ssh_setup_context::SetupContext>>,
}
impl Control {
    pub(super) fn new(
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
            }),
            stall,
            cleanup,
            clock,
            setup: None,
        }
    }
    pub(super) fn now(&self) -> Instant {
        (self.clock)()
    }
    fn fail(&self, state: &mut State, reason: TimeoutReason) {
        state.failure = Some(Fail {
            kind: io::ErrorKind::TimedOut,
            reason: Some(reason),
            terminal: None,
        });
        state.cancelled_at = Some(self.now());
    }
    pub(super) fn update(&self, state: &mut State) {
        if self.setup.is_some() {
            return;
        }
        if state.consumed || state.failure.is_some() {
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
        if let Some(setup) = &self.setup {
            setup
                .clock
                .begin_network_wait()
                .deliver()
                .map_err(|error| self.shared_publication_error(error))?;
            return self.check_shared();
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if let Some(failure) = state.failure {
            return Err(failure.into_io());
        }
        if self.stall > Duration::ZERO && state.wait_started.is_none() {
            state.wait_started = Some(self.now());
        }
        Ok(())
    }
    pub(crate) fn complete_wait(&self) -> io::Result<()> {
        if let Some(setup) = &self.setup {
            setup
                .clock
                .network_progress()
                .deliver()
                .map_err(|error| self.shared_publication_error(error))?;
            return self.check_shared();
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if let Some(failure) = state.failure {
            return Err(failure.into_io());
        }
        if self.stall > Duration::ZERO {
            state.wait_started = Some(self.now());
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
        if self.setup.is_some() {
            return self.check_shared();
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        match state.failure {
            Some(failure) => Err(failure.into_io()),
            None => Ok(()),
        }
    }
    pub(crate) fn quantum(&self) -> io::Result<Duration> {
        self.check()?;
        if let Some(setup) = &self.setup {
            return Ok(Duration::from_millis(
                setup.clock.remaining().deliver().unwrap_or(20).min(20),
            ));
        }
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
        Some(at.saturating_duration_since(self.now()))
    }
    fn stall_remaining(&self, state: &State) -> Option<Duration> {
        if self.stall == Duration::ZERO {
            return None;
        }
        let start = state.wait_started?;
        let elapsed = self.now().saturating_duration_since(start);
        Some(self.stall.saturating_sub(elapsed))
    }
    pub(super) fn cleanup_due(&self, at: Instant) -> bool {
        match at.checked_add(self.cleanup) {
            Some(due) => self.now() >= due,
            None => true,
        }
    }
    pub(super) fn cancel(&self) {
        if let Some(setup) = &self.setup {
            if self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .consumed
            {
                return;
            }
            setup
                .clock
                .terminate(gwz_transport::pool::SetupCause::Cancelled)
                .deliver();
            self.sync_shared();
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.update(&mut state);
        if !state.consumed && state.failure.is_none() {
            state.failure = Some(Fail {
                kind: io::ErrorKind::ConnectionAborted,
                reason: None,
                terminal: None,
            });
            state.cancelled_at = Some(self.now());
        }
    }
}

mod shared;
