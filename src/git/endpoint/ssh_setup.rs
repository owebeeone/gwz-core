//! Bounded setup ownership for one authenticated SSH connection.
use super::{
    agent_job::{self, Job},
    ssh_channel::{GitService, SshChannel},
    ssh_connection::SshConnection,
    ssh_key_auth::Verified,
    ssh_key_snapshot::Entry,
    ssh_pool::{Connector, Progress, Resource},
    ssh_pump::SshPump,
    ssh_worker::ChannelResource,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{AuthMethod, Effect, ErrorCode, Facts, Failure},
    stream::{MessageEndpoint, Stream},
};
use std::{
    io, mem,
    sync::Arc,
    task::{Context, Poll},
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
type Factory = Box<dyn FnMut(&Key, &Identity, Progress) -> io::Result<Setup> + Send>;
pub(crate) struct SetupConnector {
    origin: Instant,
    cleanup: Duration,
    factory: Factory,
}
impl SetupConnector {
    pub(crate) fn new(
        origin: Instant,
        cleanup: Duration,
        mut factory: impl FnMut(&Key, &Identity) -> io::Result<Setup> + Send + 'static,
    ) -> Self {
        Self::reported(origin, cleanup, move |key, identity, _| {
            factory(key, identity)
        })
    }
    pub(crate) fn reported(
        origin: Instant,
        cleanup: Duration,
        factory: impl FnMut(&Key, &Identity, Progress) -> io::Result<Setup> + Send + 'static,
    ) -> Self {
        Self {
            origin,
            cleanup,
            factory: Box::new(factory),
        }
    }
}
impl Connector for SetupConnector {
    type Resource = NativeResource;

    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        self.start_reported(key, identity, deadline, Progress::default())
    }
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        progress: Progress,
    ) -> Result<Self::Resource, Failure> {
        let deadline = deadline
            .map(|milliseconds| {
                self.origin
                    .checked_add(Duration::from_millis(milliseconds))
                    .ok_or_else(|| failure(io::ErrorKind::InvalidInput))
            })
            .transpose()?;
        let setup = (self.factory)(key, identity, progress.clone())
            .map_err(|error| failure(error.kind()))?;
        let requested = identity.clone();
        let job = Job::start(deadline, self.cleanup, move |control| setup(control))
            .map_err(|error| failure(error.kind()))?;
        Ok(NativeResource {
            state: State::Connecting(job),
            requested,
            exchanges: 0,
            deadline,
            authority: None,
            progress,
        })
    }
}
enum State {
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
    authority: Option<Arc<Entry>>,
    progress: Progress,
}
impl Resource for NativeResource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        let state = mem::replace(&mut self.state, State::Disposed);
        match state {
            State::Connecting(mut job) => match job.poll_result(cx) {
                Poll::Pending => {
                    self.state = State::Connecting(job);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(failure(error.kind()))),
                Poll::Ready(Ok(mut authenticated)) => {
                    let live = self.deadline.is_none_or(|at| Instant::now() < at);
                    let valid = live
                        && authenticated.identity == self.requested
                        && authenticated.connection.session().authenticated()
                        && authenticated.facts.authenticated == Some(true);
                    if valid {
                        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) =
                            authenticated.facts.clone();
                        self.authority = authenticated.authority.take();
                        if let Some(entry) = &self.authority {
                            entry.promote();
                        }
                        let identity = authenticated.identity.clone();
                        self.state = State::Idle(authenticated);
                        Poll::Ready(Ok(Some(identity)))
                    } else {
                        drop(authenticated);
                        Poll::Ready(Err(failure(io::ErrorKind::PermissionDenied)))
                    }
                }
            },
            other => {
                self.state = other;
                Poll::Ready(Err(failure(io::ErrorKind::BrokenPipe)))
            }
        }
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
}
impl ChannelResource for NativeResource {
    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: GitService,
        path: &str,
    ) -> io::Result<()> {
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
        code,
        effect: Effect::None,
    }
}
