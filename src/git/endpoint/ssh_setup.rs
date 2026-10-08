//! Bounded setup ownership for one authenticated SSH connection.
use super::{
    agent_job::{self, Job, Place, Supervisor, TimeoutReason, timeout_reason},
    idle_watch::{IdleReactor, IdleSocket},
    ssh_channel::{GitService, SshChannel},
    ssh_connection::SshConnection,
    ssh_key_auth::Verified,
    ssh_key_snapshot::Entry,
    ssh_pool::{Connector, Opening, Progress, Resource},
    ssh_pump::SshPump,
    ssh_setup_context::SetupContext,
    ssh_worker::ChannelResource,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{AuthMethod, Effect, ErrorCode, Facts, Failure, SetupFailureCause},
    stream::{MessageEndpoint, Stream},
};
use std::{
    io, mem,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

pub(crate) type Setup =
    Box<dyn FnOnce(Arc<agent_job::Control>) -> io::Result<Authenticated> + Send>;
pub(crate) struct Authenticated {
    connection: SshConnection,
    identity: Identity,
    facts: Facts,
    authority: Option<Arc<Entry>>,
}
impl Authenticated {
    pub(crate) fn selected(verified: Verified) -> io::Result<Self> {
        let (connection, entry) = verified.into_parts();
        Ok(Self {
            connection,
            identity: entry.identity(),
            facts: Facts {
                method: AuthMethod::SshKey,
                authenticated: Some(true),
                credential_offered: true,
                ..Facts::default()
            },
            authority: Some(entry),
        })
    }

    /// The same connection, pooled under `identity`: a URL password's open's
    /// own identity, whichever credential authenticated it (TR2.18).
    pub(crate) fn under(mut self, identity: Identity) -> Self {
        self.identity = identity;
        self
    }

    pub(crate) fn new(
        mut connection: SshConnection,
        identity: Identity,
        facts: Facts,
    ) -> io::Result<Self> {
        if !connection.session().authenticated() || facts.authenticated != Some(true) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        connection.set_nonblocking()?;
        Ok(Self {
            connection,
            identity,
            facts,
            authority: None,
        })
    }
}
type Factory = Box<dyn FnMut(&Key, &Identity, Opening) -> io::Result<Setup> + Send>;
pub(crate) struct SetupConnector {
    origin: Instant,
    cleanup: Duration,
    /// The host's job budget, which this connector's setups draw on.
    supervisor: Supervisor,
    stall_ms: u64,
    factory: Factory,
    /// Watches this connector's idle sessions; started by the first connect.
    idle: Option<Arc<IdleReactor>>,
}
impl SetupConnector {
    pub(crate) fn reported(
        origin: Instant,
        cleanup: Duration,
        supervisor: Supervisor,
        factory: impl FnMut(&Key, &Identity, Opening) -> io::Result<Setup> + Send + 'static,
    ) -> Self {
        Self {
            origin,
            cleanup,
            supervisor,
            stall_ms: 0,
            factory: Box::new(factory),
            idle: None,
        }
    }

    fn idle_reactor(&mut self) -> io::Result<Arc<IdleReactor>> {
        if let Some(reactor) = &self.idle {
            return Ok(reactor.clone());
        }
        let reactor = IdleReactor::start()?;
        self.idle = Some(reactor.clone());
        Ok(reactor)
    }
}
cfg_if::cfg_if! { if #[cfg(test)] {
impl SetupConnector {
    /// A connector on a job budget of its own, for a test that is not about
    /// the budget.
    pub(crate) fn isolated(
        origin: Instant,
        cleanup: Duration,
        factory: impl FnMut(&Key, &Identity, Opening) -> io::Result<Setup> + Send + 'static,
    ) -> Self {
        Self::reported(origin, cleanup, Supervisor::new(), factory)
    }
}
} }
impl Connector for SetupConnector {
    type Resource = NativeResource;
    fn setup_clock_source(&self) -> Option<(Instant, Arc<dyn Fn() -> u64 + Send + Sync>)> {
        let origin = self.origin;
        Some((
            origin,
            Arc::new(move || origin.elapsed().as_millis().min(u64::MAX as u128) as u64),
        ))
    }

    fn set_stall_ms(&mut self, stall_ms: u64) {
        self.stall_ms = stall_ms;
    }
    fn now_ms(&self) -> Option<u64> {
        Some(self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64)
    }
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        self.start_reported(key, identity, deadline, Opening::default())
    }
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        opening: Opening,
    ) -> Result<Self::Resource, Failure> {
        let idle = self.idle_reactor().map_err(|error| failure(error.kind()))?;
        let progress = opening.progress.clone();
        let setup_context = opening.setup.clone();
        let deadline = deadline
            .filter(|_| setup_context.is_none())
            .map(|milliseconds| {
                self.origin
                    .checked_add(Duration::from_millis(milliseconds))
                    .ok_or_else(|| failure(io::ErrorKind::InvalidInput))
            })
            .transpose()?;
        let setup =
            (self.factory)(key, identity, opening).map_err(|error| failure(error.kind()))?;
        let mut resource = NativeResource {
            state: State::Waiting(Box::new(Deferred {
                setup,
                setup_context: setup_context.clone(),
                stall: Duration::from_millis(self.stall_ms),
                cleanup: self.cleanup,
                supervisor: self.supervisor.clone(),
            })),
            requested: identity.clone(),
            exchanges: 0,
            network_budget: deadline.map(|at| at.saturating_duration_since(Instant::now())),
            waited: false,
            deadline,
            authority: None,
            progress,
            setup_context,
            idle,
            watch: None,
        };
        // The setup starts at once when the host's job budget has a place.
        // When it is full the setup waits for one, with its work in hand: a
        // full budget is backpressure, never the open's failure.
        if let Poll::Ready(Err(failed)) =
            resource.poll_start(&mut Context::from_waker(Waker::noop()))
        {
            return Err(failed);
        }
        Ok(resource)
    }
}
/// A setup that has not started, for want of a place in the job budget.
struct Deferred {
    setup: Setup,
    setup_context: Option<Arc<SetupContext>>,
    stall: Duration,
    cleanup: Duration,
    supervisor: Supervisor,
}
enum State {
    Waiting(Box<Deferred>),
    Connecting(Job<Authenticated>),
    Idle(Authenticated),
    Active {
        pump: SshPump<SshChannel>,
        identity: Identity,
        facts: Facts,
    },
    Disposed,
}
pub(crate) struct NativeResource {
    state: State,
    requested: Identity,
    exchanges: u64,
    deadline: Option<Instant>,
    /// The network time the setup has once it starts: the time spent waiting
    /// on the job budget was not the network's.
    network_budget: Option<Duration>,
    waited: bool,
    authority: Option<Arc<Entry>>,
    progress: Progress,
    setup_context: Option<Arc<SetupContext>>,
    idle: Arc<IdleReactor>,
    /// The idle session's socket watch, only while `State::Idle`.
    watch: Option<IdleSocket>,
}
impl NativeResource {
    /// Starts the setup's job once the job budget has a place for it. Until
    /// then the setup reports that it waits locally (`waiting_locally`): the
    /// pool leaves the wait to the open's allocation, and the connect clock
    /// runs from what it had left when the wait ends.
    fn poll_start(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Failure>> {
        let State::Waiting(_) = &self.state else {
            return Poll::Ready(Ok(()));
        };
        let State::Waiting(deferred) = &self.state else {
            unreachable!("checked above");
        };
        let reservation = match deferred.supervisor.reserve(cx.waker()) {
            Ok(reservation) => reservation,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.waited = true;
                return Poll::Pending;
            }
            Err(error) => return Poll::Ready(Err(failure(error.kind()))),
        };
        let State::Waiting(deferred) = mem::replace(&mut self.state, State::Disposed) else {
            unreachable!("checked above");
        };
        if self.waited {
            self.deadline = self.network_budget.map(|budget| Instant::now() + budget);
        }
        let Deferred {
            setup,
            setup_context,
            stall,
            cleanup,
            supervisor: _,
        } = *deferred;
        let job = if let Some(context) = &setup_context {
            Job::start_setup(
                Place::Reserved(reservation),
                context.clone(),
                cleanup,
                setup,
            )
        } else {
            Job::start_timed(
                Place::Reserved(reservation),
                self.deadline,
                stall,
                cleanup,
                agent_job::wall_clock(),
                setup,
            )
        }
        .map_err(|error| failure(error.kind()))?;
        self.state = State::Connecting(job);
        Poll::Ready(Ok(()))
    }
}
impl Resource for NativeResource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        match self.poll_start(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(failed)) => return Poll::Ready(Err(failed)),
            Poll::Ready(Ok(())) => {}
        }
        let state = mem::replace(&mut self.state, State::Disposed);
        match state {
            State::Connecting(mut job) => match job.poll_result(cx) {
                Poll::Pending => {
                    self.state = State::Connecting(job);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(self.setup_failure(&error))),
                Poll::Ready(Ok(mut authenticated)) => {
                    let identity_matches = authenticated.identity == self.requested;
                    let session_ok = authenticated.connection.session().authenticated()
                        && authenticated.facts.authenticated == Some(true);
                    let decision = self
                        .setup_context
                        .as_ref()
                        .map_or(Ok(()), |context| {
                            context.check().map_err(|error| self.setup_failure(&error))
                        })
                        .and_then(|_| {
                            classify_setup_result(
                                Instant::now(),
                                self.deadline,
                                identity_matches,
                                session_ok,
                            )
                        });
                    if setup_is_reusable(&decision) {
                        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) =
                            authenticated.facts.clone();
                        self.authority = authenticated.authority.take();
                        let identity = authenticated.identity.clone();
                        self.state = State::Idle(authenticated);
                        Poll::Ready(Ok(Some(identity)))
                    } else {
                        let failure = decision.expect_err("rejected setup");
                        drop(authenticated);
                        Poll::Ready(Err(failure))
                    }
                }
            },
            other => {
                self.state = other;
                Poll::Ready(Err(failure(io::ErrorKind::BrokenPipe)))
            }
        }
    }
    fn waiting_locally(&self) -> bool {
        matches!(self.state, State::Waiting(_))
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        let state = mem::replace(&mut self.state, State::Disposed);
        match state {
            State::Connecting(mut job) => match job.poll_disposed(cx) {
                Poll::Pending => {
                    self.state = State::Connecting(job);
                    Poll::Pending
                }
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(error)) => {
                    self.state = State::Connecting(job);
                    Poll::Ready(Err(error))
                }
            },
            // Nothing was started, so nothing is owed.
            State::Waiting(_) => Poll::Ready(Ok(())),
            State::Idle(authenticated) => {
                drop(authenticated);
                Poll::Ready(Ok(()))
            }
            State::Active {
                mut pump,
                identity,
                facts,
            } => {
                let result = if force {
                    pump.force_dispose()
                } else {
                    pump.poll_dispose()
                };
                match result {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(error) if !force && error.kind() == io::ErrorKind::WouldBlock => {
                        self.state = State::Active {
                            pump,
                            identity,
                            facts,
                        };
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Err(error) => {
                        self.state = State::Active {
                            pump,
                            identity,
                            facts,
                        };
                        Poll::Ready(Err(error))
                    }
                }
            }
            State::Disposed => Poll::Ready(Ok(())),
        }
    }
    fn reusable(&self) -> bool {
        matches!(self.state, State::Idle(_))
    }
    fn poll_idle_lost(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let State::Idle(authenticated) = &self.state else {
            self.watch = None;
            return Poll::Pending;
        };
        if self.watch.is_none() {
            let watch = authenticated
                .connection
                .watch_socket()
                .and_then(|socket| IdleSocket::watch(&self.idle, socket));
            match watch {
                Ok(watch) => self.watch = Some(watch),
                // A session that cannot be watched is not kept idle.
                Err(_) => return Poll::Ready(()),
            }
        }
        self.watch.as_ref().expect("idle watch").poll_lost(cx)
    }
}
impl ChannelResource for NativeResource {
    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: GitService,
        path: &str,
    ) -> io::Result<()> {
        // The exchange owns the socket's reads from here.
        self.watch = None;
        let state = mem::replace(&mut self.state, State::Disposed);
        match state {
            State::Idle(authenticated) => {
                let Authenticated {
                    connection,
                    identity,
                    facts,
                    authority: _,
                } = authenticated;
                let channel = SshChannel::new(connection, service, path)?;
                let mut pump = SshPump::new(stream, endpoint, channel, 65_536, 65_536);
                pump.set_service(service);
                let exchange_facts = if self.exchanges == 0 {
                    facts.clone()
                } else {
                    let mut exchange_facts = facts.clone();
                    exchange_facts.credential_offered = false;
                    exchange_facts
                };
                pump.set_facts(exchange_facts);
                self.exchanges = self.exchanges.saturating_add(1);
                self.state = State::Active {
                    pump,
                    identity,
                    facts,
                };
                Ok(())
            }
            other => {
                self.state = other;
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "SSH resource is not idle",
                ))
            }
        }
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        match &mut self.state {
            State::Active { pump, .. } => Some(pump),
            _ => None,
        }
    }
    fn reclaim(&mut self) -> bool {
        let state = mem::replace(&mut self.state, State::Disposed);
        match state {
            State::Active {
                pump,
                identity,
                facts,
            } => match pump.into_owner() {
                Ok(connection) => {
                    self.state = State::Idle(Authenticated {
                        connection,
                        identity,
                        facts,
                        authority: None,
                    });
                    true
                }
                Err(pump) => {
                    self.state = State::Active {
                        pump,
                        identity,
                        facts,
                    };
                    false
                }
            },
            other => {
                self.state = other;
                false
            }
        }
    }
    fn observation(&self) -> (bool, Facts) {
        let facts = match &self.state {
            State::Idle(authenticated) => authenticated.facts.clone(),
            State::Active { facts, .. } => facts.clone(),
            _ => Facts::default(),
        };
        if self.exchanges == 0 {
            (false, facts)
        } else {
            let mut facts = facts;
            facts.credential_offered = false;
            (true, facts)
        }
    }
}
impl Drop for NativeResource {
    fn drop(&mut self) {
        if let State::Connecting(job) = &self.state {
            job.cancel();
        }
    }
}
impl NativeResource {
    fn setup_failure(&self, error: &io::Error) -> Failure {
        if let (Some(context), Some(ended)) = (
            &self.setup_context,
            error
                .get_ref()
                .and_then(|cause| cause.downcast_ref::<super::ssh_setup_context::SetupEnded>()),
        ) {
            context.failure(ended.0)
        } else {
            failure_from_io(error)
        }
    }
}
pub(crate) fn classify_setup_result(
    now: Instant,
    deadline: Option<Instant>,
    identity_matches: bool,
    authenticated: bool,
) -> Result<(), Failure> {
    if deadline.is_some_and(|at| now >= at) {
        return Err(timeout_failure(TimeoutReason::Aggregate));
    }
    if identity_matches && authenticated {
        Ok(())
    } else {
        Err(failure(io::ErrorKind::PermissionDenied))
    }
}
pub(crate) fn setup_is_reusable(result: &Result<(), Failure>) -> bool {
    result.is_ok()
}
fn timeout_failure(reason: TimeoutReason) -> Failure {
    Failure {
        detail: None,
        setup_cause: Some(reason.setup_cause()),
        facts: None,
        code: ErrorCode::Timeout,
        effect: Effect::None,
    }
}
/// The failure a setup job's error stands for, with a timeout's origin.
pub(crate) fn failure_from_io(error: &io::Error) -> Failure {
    if let Some(reason) = timeout_reason(error) {
        return timeout_failure(reason);
    }
    failure(error.kind())
}
fn failure(kind: io::ErrorKind) -> Failure {
    let code = match kind {
        io::ErrorKind::TimedOut => ErrorCode::Timeout,
        io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted => ErrorCode::Cancelled,
        io::ErrorKind::PermissionDenied => ErrorCode::Authentication,
        io::ErrorKind::InvalidData => ErrorCode::Protocol,
        io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
        io::ErrorKind::WouldBlock => ErrorCode::Capacity,
        io::ErrorKind::NotFound
        | io::ErrorKind::AddrNotAvailable
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::Unsupported => ErrorCode::Unavailable,
        _ => ErrorCode::Io,
    };
    Failure {
        detail: None,
        setup_cause: match kind {
            io::ErrorKind::ConnectionRefused => Some(SetupFailureCause::ConnectionRefused),
            io::ErrorKind::NotFound => Some(SetupFailureCause::NotFound),
            io::ErrorKind::AddrNotAvailable => Some(SetupFailureCause::AddressNotAvailable),
            _ => None,
        },
        facts: None,
        code,
        effect: Effect::None,
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
