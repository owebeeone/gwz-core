//! The HTTPS endpoint adapter of the connection host (plan step 1.3c; contract sections 4.1, 6, 7.2, 8 and 9 of
//! dev-docs/GwzConnectionApiContract.md): a `gwz_connection::EndpointAdapter` at the Connector/Resource level, over
//! today's HTTPS connector (`https_connection::HttpConnector` behind the host's shared reservation: TLS, proxy, the
//! final origin's channel binding, one Hyper HTTP/1 connection per resource) and today's authentication machinery (the
//! configured credential helper through `https_auth`, the logon session through `https_worker::native`).
//!
//! - `connect` starts one physical HTTPS connection for the connection the host's pool named; `cancel_connect` and
//!   `close` dispose of it, and each ends with exactly one completion.
//! - `start_exchange` runs one smart-HTTP request on a leased, idle connection: the `ExchangeSpec`'s Git service names
//!   the method and the service path under the spec's repository path (`GET .../info/refs?service=...` for an
//!   advertisement, `POST .../git-upload-pack` or `.../git-receive-pack` otherwise). The connection is idle again only
//!   after the response body reached its validated end, the client read all of it and asked for the end, the response
//!   did not close the connection and Hyper says the connection can carry another request; the exchange's end
//!   (`ExchangeIo::poll_finish`) says `Reusable` only then (contract 8: an unread body discards the lease).
//! - A challenge (401) to an advertisement is answered on the same connection, once per exchange: by the logon
//!   session when it offers `Negotiate` or `NTLM` and the exchange's operation has a native caller (the one its `AuthContext`
//!   holds, never another's; the adapter holds none), otherwise by the helper of the
//!   operation the exchange serves (the one its `AuthContext` holds, never another's; the adapter holds none) when it
//!   offers `Basic`. A challenge to a POST is refused (contract 7.2): its body is on the wire, and nothing is replayed.
//! - The credential and its need are operation state (HTTPS model draft r5 section 3; contract 7.2): an advertisement
//!   whose challenge a fresh helper lookup answered records the key (origin, username, repository path, service class)
//!   and the credential in the operation's own record (`https_auth::needs`, in its `AuthContext` provider), after the
//!   authenticated 200 and before any byte of its body reaches the client. A later challenge to that key's
//!   advertisement is answered from the record, and its POST carries the record's credential from its first byte, on
//!   whichever connection the pool leased; no helper runs for either. A POST of a key the operation never learned goes
//!   out anonymously, and a challenge to it is refused: a typed `Authentication`, nothing replayed. The record's
//!   header is built under its lock; the adapter, its connections and its scope records hold no credential. Past one
//!   of the record's bounds (contract section 10) an advertisement ends with the budget code (`Capacity`, which the
//!   client's stream reads as `QuotaExceeded`) before any byte of its body, and a key the record could not keep sends
//!   no POST; an `Exhausted` record answers no challenge and starts no lookup.
//! - Connection-bound keys (draft r5 section 3.1): NTLM and Negotiate authenticate the connection, so an advertisement
//!   the logon session answered (`Challenged`), or served unchallenged on a connection the logon session had
//!   authenticated (`OnBound`), records its key connection-bound, with no credential, and the exchange says its
//!   connection is bound (`ExchangeIo::connection_bound`): the binding then prefers that connection for the key's next
//!   exchange. A POST of such a key on any other connection first re-authenticates it with the key's own advertisement
//!   GET, answered only by the operation's logon session and drained before the POST's first byte; a re-authentication
//!   that does not authenticate the connection ends the exchange before any POST byte, except that a key learned
//!   `OnBound` and served unchallenged again goes as an unrecorded key's. Re-authentications are counted in the
//!   [`Counters`] the factory gives every adapter. An exchange's `Effect` is its own request's: `Possible` once that
//!   request is handed to Hyper (contract 9, A7).
//!   The native route **fails closed**
//!   (contract 7.3, step 1.3a): when the composition's `ProcessSafety` is unavailable, or the caller's gate gave its
//!   supervisor no window, it refuses before any credential is read or token sent.
//! - Per-exchange scoping (contract 7.2): each connection's [`ConnectionScope`] records which authentication bound
//!   it; a helper token (a fresh lookup's or the record's) or the logon session marks it per-exchange scoped, and the
//!   record only rises (draft 3(e) W3). The adapter's reuse
//!   authorization (`authorize_reuse`, plan step 1.6) reads this record: a connection nothing authenticated is
//!   leased to another binding at once, and one a helper token or the logon session authenticated never is.
//! - Idle liveness: an idle connection whose peer closes it, resets it or sends unasked bytes (Hyper's driver ends) is
//!   disposed and reported `Completion::IdleClosed`.
//! - Fault scope: a failed request, response or stream ends that exchange only (`Discarded`; the host's pool closes
//!   that one connection); a connection's physical failure retires that connection only; neither touches another
//!   connection or the host.
//!
//! It owns no pool and no reuse policy. Its I/O runs on the adapter's own worker thread and the small runtime that
//! thread drives (`worker.rs`), started by the first `connect`, outside every connection-host lock; the methods here
//! only queue commands and read completions. Nothing here is a global.
use super::{
    https_auth::HelperSlots,
    https_connection,
    shared_reservation::Authority,
    ssh_adapter::{Settle, Shared},
};
use gwz_connection::{
    AuthContext, Completion, ConnectCommand, EndpointAdapter, ExchangeIo, ExchangeSpec,
    ReuseCommand, ReuseVerdict, SharedProcessSafety,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Effect, ErrorCode, Failure},
};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::SendError};

mod auth;
mod exchange;
mod pipe;
mod worker;

/// The budgets of the adapter's connections and exchanges, from the endpoint's pool policy.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpsSettings {
    /// A helper lookup's admission bound (0: none).
    pub(crate) allocation_timeout_ms: u64,
    /// The setup's bound, and the native route's fixed deadline D (0: none; the native route then refuses).
    pub(crate) connect_timeout_ms: u64,
    /// An exchange's network stall bound (0: none).
    pub(crate) io_timeout_ms: u64,
    /// A helper's interaction bound (0: none).
    pub(crate) interaction_timeout_ms: u64,
    /// The bound on the settlement of an exchange's end, after which its connection is discarded.
    pub(crate) cleanup_timeout_ms: u64,
}

/// What an HTTPS adapter is made from: the endpoint's TLS and proxy configuration, the host's helper slots (whose
/// launches hold the host's spawn gate), the host's shared reservation, which its SSH adapters share, and the counters
/// its factory gives every adapter it builds. It holds no credential helper, environment, token or native caller:
/// those are the operation's own, in the `AuthContext` that `start_exchange` carries (contract section 7).
#[derive(Clone)]
pub(crate) struct HttpsInputs {
    pub(crate) tls: https_connection::Config,
    pub(crate) helper_slots: HelperSlots,
    pub(crate) authority: Authority,
    pub(crate) settings: HttpsSettings,
    pub(crate) counters: Counters,
}

/// The HTTPS adapters' counters (HTTPS model draft r5 3.1, Counter): one shared saturating count of the
/// re-authentications of connection-bound keys, which the composition's factory owns and gives every adapter it
/// builds, so that it does not grow with instance turnover. A clone counts into the same total; no adapter keeps a
/// count of its own. How it reaches `TransportObservation` is the 3.1/3.4 composition's (draft U5).
#[derive(Clone, Debug, Default)]
pub(crate) struct Counters {
    reauthentications: Arc<AtomicU64>,
}
impl Counters {
    /// One more re-authentication; the count saturates.
    pub(super) fn reauthenticated(&self) {
        let _ =
            self.reauthentications
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    Some(count.saturating_add(1))
                });
    }
    /// The re-authentications every adapter sharing this counter has made.
    #[allow(
        dead_code,
        reason = "read by the plan 3.4 sweep through the 3.1 composition (HTTPS model draft r5 U5); the rows read it now"
    )]
    pub(crate) fn reauthentications(&self) -> u64 {
        self.reauthentications.load(Ordering::Relaxed)
    }
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            /// Whether `self` and `other` count into one total.
            pub(crate) fn shares(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.reauthentications, &other.reauthentications)
            }
        }
    }
}

/// Which authentication bound a connection's exchanges (contract 7.2), in the order a connection's record rises
/// (draft 3(e) W3): `Anonymous` < `HelperToken` < `LogonSession`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Binding {
    /// No credential was offered on it.
    Anonymous,
    /// A configured helper's credential authenticated an exchange on it (request-scoped `Basic`).
    HelperToken,
    /// The logon session authenticated it (`Negotiate` or `NTLM`, bound to the connection and its TLS binding).
    LogonSession,
}
impl Binding {
    /// Whether the connection is per-exchange scoped: authenticated by a helper token or the logon session.
    pub(crate) fn per_exchange(self) -> bool {
        !matches!(self, Self::Anonymous)
    }
}
/// The scope record of one connection: the highest authentication that bound it and, for an authenticated one, the
/// adapter-local number of the last exchange that bound it so (an opaque tag, unique within the adapter). Once
/// authenticated, a connection stays per-exchange scoped until it is disposed: the record only rises (draft 3(e) W3),
/// so a later anonymous exchange does not clear it and a helper token's does not lower the logon session's. Which
/// repositories need a credential is the operation's record, never the connection's. Nothing in it is a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionScope {
    pub(crate) binding: Binding,
    pub(crate) exchange: Option<u64>,
}
/// The adapter's scope records, one per live connection. A clone reads the same records.
#[derive(Clone, Default)]
pub(crate) struct Scopes(Arc<Mutex<BTreeMap<ConnectionId, ConnectionScope>>>);
impl Scopes {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<ConnectionId, ConnectionScope>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The record of `connection`, while it lives.
    pub(crate) fn of(&self, connection: ConnectionId) -> Option<ConnectionScope> {
        self.lock().get(&connection).copied()
    }
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            /// Every live connection's record.
            pub(crate) fn all(&self) -> Vec<(ConnectionId, ConnectionScope)> {
                self.lock().iter().map(|(id, scope)| (*id, *scope)).collect()
            }
        }
    }
    fn connected(&self, connection: ConnectionId) {
        self.lock().insert(
            connection,
            ConnectionScope {
                binding: Binding::Anonymous,
                exchange: None,
            },
        );
    }
    /// The exchange numbered `exchange` ended having bound `connection` as `binding`. The record only rises (draft 3(e)
    /// W3): `LogonSession` is sticky, `HelperToken` is replaced only by `LogonSession`, and `Anonymous` changes nothing.
    fn bound(&self, connection: ConnectionId, binding: Binding, exchange: u64) {
        if !binding.per_exchange() {
            return;
        }
        if let Some(scope) = self.lock().get_mut(&connection)
            && binding >= scope.binding
        {
            *scope = ConnectionScope {
                binding,
                exchange: Some(exchange),
            };
        }
    }
    fn forget(&self, connection: ConnectionId) {
        self.lock().remove(&connection);
    }
}

pub(super) fn failure(code: ErrorCode) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        facts: None,
        code,
        effect: Effect::None,
    }
}

/// The commands the adapter's methods queue for its worker.
pub(super) enum Command {
    Connect(ConnectCommand),
    CancelConnect(ConnectionId),
    Close(ConnectionId),
    Start {
        connection: ConnectionId,
        work: Box<exchange::Work>,
    },
    Dispose,
}
impl Command {
    /// The command reaches no worker: a connect fails at once and an exchange ends discarded; nothing physical exists.
    fn refuse(self, shared: &Shared) {
        match self {
            Self::Connect(command) => shared.push(Completion::Connected {
                connection: command.connection,
                result: Err(failure(ErrorCode::Unavailable)),
            }),
            Self::Close(connection) => shared.push(Completion::Closed { connection }),
            Self::Start { work, .. } => work.fail(ErrorCode::Unavailable),
            Self::CancelConnect(_) | Self::Dispose => {}
        }
    }
}

type Start =
    Box<dyn FnOnce(UnboundedReceiver<Command>, Arc<Shared>) -> io::Result<JoinHandle<()>> + Send>;
/// The HTTPS adapter of one endpoint instance. See the module doc.
pub(crate) struct HttpsAdapter {
    /// Starts the worker on the first command.
    start: Option<(Start, UnboundedReceiver<Command>)>,
    commands: UnboundedSender<Command>,
    running: bool,
    shared: Arc<Shared>,
    scopes: Scopes,
    counters: Counters,
    dispose_sent: bool,
}
impl HttpsAdapter {
    /// An adapter over `inputs`, whose native route answers only while `safety` is qualified. Nothing starts here: no
    /// thread, no network, no helper or authentication (contract section 5).
    pub(crate) fn new(inputs: HttpsInputs, safety: SharedProcessSafety) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        let scopes = Scopes::default();
        let records = scopes.clone();
        let counters = inputs.counters.clone();
        let start: Start = Box::new(move |receiver, shared| {
            thread::Builder::new()
                .name("gwz-https-adapter".into())
                .spawn(move || worker::run(inputs, safety, receiver, shared, records))
        });
        Self {
            start: Some((start, receiver)),
            commands,
            running: false,
            shared: Arc::default(),
            scopes,
            counters,
            dispose_sent: false,
        }
    }
    /// The counters this adapter counts into, which its factory's other adapters share (draft r5 3.1, Counter).
    #[allow(
        dead_code,
        reason = "read by the plan 3.4 sweep through the 3.1 composition (HTTPS model draft r5 U5); the rows read it now"
    )]
    pub(crate) fn counters(&self) -> Counters {
        self.counters.clone()
    }
    /// The adapter's scope records, which its own `authorize_reuse` reads.
    #[allow(dead_code, reason = "the rows read the records through the adapter")]
    pub(crate) fn scopes(&self) -> Scopes {
        self.scopes.clone()
    }
    fn send(&mut self, command: Command) {
        if let Some((start, receiver)) = self.start.take() {
            match start(receiver, self.shared.clone()) {
                Ok(_) => self.running = true,
                // No worker: every connect fails at once and nothing physical exists.
                Err(_) => self.shared.disposed(),
            }
        }
        if !self.running {
            return command.refuse(&self.shared);
        }
        if let Err(SendError(command)) = self.commands.send(command) {
            // The worker has ended (it was disposed of): nothing is left to do the work.
            command.refuse(&self.shared);
        }
    }
}
impl EndpointAdapter for HttpsAdapter {
    fn connect(&mut self, command: ConnectCommand) {
        self.send(Command::Connect(command));
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        self.send(Command::CancelConnect(connection));
    }
    fn close(&mut self, connection: ConnectionId) {
        self.send(Command::Close(connection));
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        if !self.running {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "HTTPS adapter has no worker",
            ));
        }
        let settle = Arc::new(Settle::default());
        // The exchange authenticates with the helper this operation's own context holds (plan step 1.3c/C2(b)), never
        // another operation's: the context lives in the exchange's work until its task ends, and nothing else keeps it.
        let (client, work) = pipe::exchange(spec, auth, settle);
        self.send(Command::Start {
            connection,
            work: Box::new(work),
        });
        Ok(Box::new(client))
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        // Contract 7.2: a connection a helper token or the logon session authenticated is per-exchange scoped and
        // serves only the binding that last used it (`Unprovable`: no proof is started). One nothing authenticated
        // carries no one's credential, and the endpoint's TLS and proxy configuration, which every binding of this
        // instance shares, bound it: it serves any binding at once.
        let anonymous = self
            .scopes
            .of(command.connection)
            .is_some_and(|scope| !scope.binding.per_exchange());
        if anonymous {
            self.shared.push(Completion::Reuse {
                connection: command.connection,
                proof: command.proof,
                verdict: ReuseVerdict::Authorized,
            });
        }
        anonymous
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        self.shared.poll_completion(cx)
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if !self.running {
            // Never started: nothing physical was made.
            return Poll::Ready(());
        }
        if !self.dispose_sent {
            self.dispose_sent = true;
            self.send(Command::Dispose);
        }
        self.shared.poll_disposed(cx)
    }
}
// Dropping the adapter closes its command channel: the worker sees it, disposes of every connection as the last
// resort (contract 4.1) and ends.

/// Whether the response asks for its connection to be closed after it.
pub(super) fn closes(headers: &hyper::HeaderMap) -> bool {
    headers
        .get_all(hyper::header::CONNECTION)
        .iter()
        .any(|value| {
            !value.to_str().is_ok_and(|text| {
                !text
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("close"))
            })
        })
}

/// The exchange's failure as the client's stream reads it. `Capacity` ends an exchange only when the operation's
/// credential record is full (HTTPS model draft r5 3(b), contract A9; the pool's own limit refuses an opening and never
/// ends an exchange), so its message names the record and no pool limit.
pub(super) fn stream_error(code: ErrorCode) -> io::Error {
    let kind = match code {
        ErrorCode::Capacity => {
            return io::Error::new(
                io::ErrorKind::QuotaExceeded,
                "operation credential record budget exceeded",
            );
        }
        ErrorCode::Authentication => io::ErrorKind::PermissionDenied,
        ErrorCode::RepositoryRefused => io::ErrorKind::NotFound,
        ErrorCode::Timeout => io::ErrorKind::TimedOut,
        ErrorCode::Cancelled => io::ErrorKind::Interrupted,
        ErrorCode::Protocol | ErrorCode::InvalidRequest => io::ErrorKind::InvalidData,
        ErrorCode::UnsupportedOperation => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("HTTPS exchange failed: {code:?}"))
}
