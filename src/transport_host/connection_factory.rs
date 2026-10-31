//! gwz-core's production `EndpointFactory` (plan steps 1.3b, 1.3c and R3; contract section 4.1 of
//! dev-docs/GwzConnectionApiContract.md): what `TransportRuntime::build_native` gives its connection host to make endpoint
//! adapters with.
//!
//! Each `create` makes one adapter from the `EndpointSpec`'s profile (`EndpointInputs`, `endpoint_environment.rs`, the
//! validated immutable inputs one operation's capture made), allocating metadata only: no thread, network, key, agent or
//! helper (contract section 5). So an instance is configured by the operation whose profile made it, and not by the
//! runtime that built the host: its home and `known_hosts`, its agent, its TLS trust, proxy and timeouts are that
//! profile's (Phase 1 Code P2-2). A spec whose profile carries no inputs is refused `Unavailable`. The factory itself
//! holds only what is the host's, not an operation's:
//! - SSH: the key registry the selected keys are admitted to, the host's shared reservation, where a refused host key
//!   is noted, and the SSH password helpers;
//! - HTTPS: the host's helper slots (whose launches hold its spawn gate) and the same shared reservation.
//!
//! The native caller is the operation's too: it is in its `AuthContext` (`https_auth::OperationAuth`), and the HTTPS
//! adapter reads it there, per exchange.
//!
//! The adapter is a [`SchemeRouter`] over two halves, each present when the factory has that scheme's host parts and the
//! profile has that scheme's settings:
//! - SSH: `git::endpoint::ssh_adapter::SshAdapter` over today's connector (`ssh_local::native_connector`), on the
//!   profile's known-hosts file and agent;
//! - HTTPS: `git::endpoint::https_adapter::HttpsAdapter` over today's HTTPS connector, on the profile's TLS and proxy
//!   configuration and the composition's `ProcessSafety` (`FactoryEnv::safety`), which its native route fails closed on.
//!
//! The router sends each connect to the half its key's scheme names, and every later command for that connection to
//! the same half; a connect for a scheme the instance has no half for completes at once with `UnsupportedOperation`.
//! Neither half holds a pool: the host does.
use super::endpoint_environment::EndpointInputs;
use crate::git::endpoint::{
    https_adapter::{HttpsAdapter, HttpsInputs, HttpsSettings},
    https_auth::HelperSlots,
    shared_reservation::Authority,
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_handoff::Handoff,
    ssh_home::KnownHostsFile,
    ssh_key_snapshot::Registry,
    ssh_local::{self, NativeInputs},
    ssh_password_helpers::Helpers,
};
use gwz_connection::{
    AuthContext, CleanupStatus, Completion, ConnectCommand, EndpointAdapter, EndpointFactory,
    EndpointSpec, ExchangeIo, ExchangeSpec, FactoryEnv, FactoryError, FactoryReason, ProofId,
    ReuseCommand,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Effect, ErrorCode, Failure, Scheme},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

/// The SSH half of the factory's host parts: the key registry the instances' selected keys are admitted to, the host's
/// shared reservation, where a refused host key is noted, and the SSH password helpers (the runtime's).
#[derive(Clone)]
pub(crate) struct SshHost {
    pub(crate) registry: Registry,
    pub(crate) authority: Authority,
    pub(crate) trust_log: Handoff,
    pub(crate) helpers: Option<Arc<Helpers>>,
}
/// The HTTPS half of the factory's host parts: the host's helper slots and shared reservation.
#[derive(Clone)]
pub(crate) struct HttpsHost {
    pub(crate) helper_slots: HelperSlots,
    pub(crate) authority: Authority,
}
pub(crate) struct CoreFactory {
    ssh: Option<SshHost>,
    https: Option<HttpsHost>,
}
impl CoreFactory {
    pub(crate) fn new(ssh: Option<SshHost>, https: Option<HttpsHost>) -> Self {
        Self { ssh, https }
    }
    /// The registry the factory's adapters look selected keys up in.
    pub(crate) fn registry(&self) -> Option<Registry> {
        self.ssh.as_ref().map(|ssh| ssh.registry.clone())
    }
}
impl EndpointFactory for CoreFactory {
    fn create(
        &self,
        spec: &EndpointSpec,
        env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let unavailable = || FactoryError::new(FactoryReason::Unavailable, CleanupStatus::Complete);
        // A profile that carries no inputs (or inputs of another composition) is not one this factory can make an
        // adapter from: the instance's configuration is its profile's, never a default.
        let inputs = spec
            .profile
            .inputs()
            .and_then(|inputs| inputs.downcast_ref::<EndpointInputs>())
            .ok_or_else(unavailable)?;
        let ssh = self
            .ssh
            .clone()
            .zip(inputs.local.ssh.as_ref())
            .map(|(host, ssh)| {
                let pool = &inputs.local.pool;
                let native = NativeInputs {
                    cleanup: Duration::from_millis(pool.cleanup_timeout_ms),
                    known_hosts: KnownHostsFile::under(&ssh.home, ssh.home_source).path,
                    agent: ssh.agent.clone(),
                    authority: host.authority,
                    trust_log: host.trust_log,
                    helpers: host.helpers,
                };
                let registry = host.registry;
                SshAdapter::new(
                    move |origin| ssh_local::native_connector(origin, registry, native),
                    AdapterSettings {
                        connect_timeout_ms: pool.connect_timeout_ms,
                        io_timeout_ms: inputs.local.io_timeout_ms,
                        interaction_timeout_ms: pool.interaction_timeout_ms,
                        cleanup_timeout_ms: pool.cleanup_timeout_ms,
                    },
                )
            });
        let https = self.https.clone().map(|host| {
            let pool = &inputs.local.pool;
            HttpsAdapter::new(
                HttpsInputs {
                    tls: inputs.tls.clone(),
                    helper_slots: host.helper_slots,
                    authority: host.authority,
                    settings: HttpsSettings {
                        allocation_timeout_ms: pool.allocation_timeout_ms,
                        connect_timeout_ms: pool.connect_timeout_ms,
                        io_timeout_ms: inputs.local.io_timeout_ms,
                        interaction_timeout_ms: pool.interaction_timeout_ms,
                        cleanup_timeout_ms: pool.cleanup_timeout_ms,
                    },
                },
                env.safety().clone(),
            )
        });
        if ssh.is_none() && https.is_none() {
            // Neither scheme's host parts, and an SSH-less profile with no HTTPS parts: no endpoint to make.
            return Err(unavailable());
        }
        Ok(Box::new(SchemeRouter::new(ssh, https)))
    }
}

/// One endpoint instance's adapter: the SSH half and the HTTPS half, each connection on the half its key's scheme
/// names. See the module doc.
pub(crate) struct SchemeRouter {
    ssh: Option<SshAdapter>,
    https: Option<HttpsAdapter>,
    /// The half each live connection is on.
    routes: BTreeMap<ConnectionId, Scheme>,
    /// Completions the router makes itself (a connect no half serves, a close of a connection no half holds).
    own: VecDeque<Completion>,
    waker: Option<Waker>,
    /// Which half is polled first, alternately, so that neither starves the other.
    https_first: bool,
}
impl SchemeRouter {
    pub(crate) fn new(ssh: Option<SshAdapter>, https: Option<HttpsAdapter>) -> Self {
        Self {
            ssh,
            https,
            routes: BTreeMap::new(),
            own: VecDeque::new(),
            waker: None,
            https_first: false,
        }
    }
    fn half(&mut self, scheme: Scheme) -> Option<&mut dyn EndpointAdapter> {
        match scheme {
            Scheme::Ssh => self.ssh.as_mut().map(|ssh| ssh as &mut dyn EndpointAdapter),
            Scheme::Https => self
                .https
                .as_mut()
                .map(|https| https as &mut dyn EndpointAdapter),
        }
    }
    fn routed(&mut self, connection: ConnectionId) -> Option<&mut dyn EndpointAdapter> {
        let scheme = *self.routes.get(&connection)?;
        self.half(scheme)
    }
    fn complete(&mut self, completion: Completion) {
        self.own.push_back(completion);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
    /// Forgets a connection's route once its half has ended it.
    fn ended(&mut self, completion: &Completion) {
        match completion {
            Completion::Connected {
                connection,
                result: Err(_),
            }
            | Completion::Closed { connection }
            | Completion::IdleClosed { connection } => {
                self.routes.remove(connection);
            }
            _ => {}
        }
    }
}
impl EndpointAdapter for SchemeRouter {
    fn connect(&mut self, command: ConnectCommand) {
        let (connection, scheme) = (command.connection, command.key.scheme);
        match self.half(scheme) {
            Some(half) => {
                half.connect(command);
                self.routes.insert(connection, scheme);
            }
            None => self.complete(Completion::Connected {
                connection,
                result: Err(Failure {
                    detail: None,
                    setup_cause: None,
                    facts: None,
                    code: ErrorCode::UnsupportedOperation,
                    effect: Effect::None,
                }),
            }),
        }
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        if let Some(half) = self.routed(connection) {
            half.cancel_connect(connection);
        }
    }
    fn close(&mut self, connection: ConnectionId) {
        match self.routed(connection) {
            Some(half) => half.close(connection),
            // Its half disposed of it already (lost while idle): the close is confirmed at once.
            None => self.complete(Completion::Closed { connection }),
        }
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        match self.routed(connection) {
            Some(half) => half.start_exchange(connection, spec, auth),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "no adapter holds the connection",
            )),
        }
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        // A connection no half holds has nothing to prove: the host takes `false` as `Unprovable`.
        self.routed(command.connection)
            .is_some_and(|half| half.authorize_reuse(command))
    }
    fn cancel_reuse(&mut self, proof: ProofId) {
        // The router does not know which half runs the proof: both are told, and the one that does not stops nothing.
        for scheme in [Scheme::Ssh, Scheme::Https] {
            if let Some(half) = self.half(scheme) {
                half.cancel_reuse(proof);
            }
        }
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        if let Some(completion) = self.own.pop_front() {
            return Poll::Ready(completion);
        }
        self.waker = Some(cx.waker().clone());
        self.https_first = !self.https_first;
        let order = if self.https_first {
            [Scheme::Https, Scheme::Ssh]
        } else {
            [Scheme::Ssh, Scheme::Https]
        };
        for scheme in order {
            if let Some(half) = self.half(scheme)
                && let Poll::Ready(completion) = half.poll_completion(cx)
            {
                self.ended(&completion);
                return Poll::Ready(completion);
            }
        }
        Poll::Pending
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let ssh = self
            .ssh
            .as_mut()
            .map_or(Poll::Ready(()), |ssh| ssh.poll_dispose(cx));
        let https = self
            .https
            .as_mut()
            .map_or(Poll::Ready(()), |https| https.poll_dispose(cx));
        if ssh.is_ready() && https.is_ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
