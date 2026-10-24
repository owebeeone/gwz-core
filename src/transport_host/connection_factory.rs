//! gwz-core's production `EndpointFactory` (plan steps 1.3b and 1.3c; contract section 4.1 of
//! dev-docs/GwzConnectionApiContract.md): what `TransportRuntime::build_native` gives its connection host to make endpoint
//! adapters with.
//!
//! The factory is made from the runtime's endpoint settings once, and each `create` makes one adapter from them,
//! allocating metadata only: no thread, network, key, agent or helper (contract section 5). The adapter is a
//! [`SchemeRouter`] over two halves, each present when the runtime has that scheme's settings:
//! - SSH: `git::endpoint::ssh_adapter::SshAdapter` over today's connector (`ssh_local::native_connector`), on the
//!   runtime's known-hosts file, agent, SSH helpers and the host's shared reservation;
//! - HTTPS: `git::endpoint::https_adapter::HttpsAdapter` over today's HTTPS connector, on the `HttpsEndpointConfig` and
//!   `HelperSlots` the endpoint session's HTTPS endpoint is built from, the native caller of a native-route runtime, the
//!   same shared reservation and the composition's `ProcessSafety` (`FactoryEnv::safety`), which its native route
//!   fails closed on.
//!
//! The router sends each connect to the half its key's scheme names, and every later command for that connection to
//! the same half; a connect for a scheme the runtime has no settings for completes at once with
//! `UnsupportedOperation`. Neither half holds a pool: the host does.
use crate::git::endpoint::{
    https_adapter::{HttpsAdapter, HttpsInputs},
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_key_snapshot::Registry,
    ssh_local::{self, NativeInputs},
};
use gwz_connection::{
    CleanupStatus, Completion, ConnectCommand, EndpointAdapter, EndpointFactory, EndpointSpec,
    ExchangeIo, ExchangeSpec, FactoryEnv, FactoryError, FactoryReason,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Effect, ErrorCode, Failure, Scheme},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    task::{Context, Poll, Waker},
};

/// The SSH half of the factory: the connector's inputs, the key registry its selected keys are admitted to, and the
/// budgets of its connections.
#[derive(Clone)]
pub(crate) struct SshInputs {
    pub(crate) native: NativeInputs,
    pub(crate) registry: Registry,
    pub(crate) settings: AdapterSettings,
}
pub(crate) struct CoreFactory {
    ssh: Option<SshInputs>,
    https: Option<HttpsInputs>,
}
impl CoreFactory {
    pub(crate) fn new(ssh: Option<SshInputs>, https: Option<HttpsInputs>) -> Self {
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
        _spec: &EndpointSpec,
        env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        if self.ssh.is_none() && self.https.is_none() {
            // A runtime with neither scheme's settings has no endpoint to make.
            return Err(FactoryError::new(
                FactoryReason::Unavailable,
                CleanupStatus::Complete,
            ));
        }
        let ssh = self.ssh.clone().map(
            |SshInputs {
                 native,
                 registry,
                 settings,
             }| {
                SshAdapter::new(
                    move |origin| ssh_local::native_connector(origin, registry, native),
                    settings,
                )
            },
        );
        let https = self
            .https
            .clone()
            .map(|inputs| HttpsAdapter::new(inputs, env.safety().clone()));
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
    ) -> io::Result<Box<dyn ExchangeIo>> {
        match self.routed(connection) {
            Some(half) => half.start_exchange(connection, spec),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "no adapter holds the connection",
            )),
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
