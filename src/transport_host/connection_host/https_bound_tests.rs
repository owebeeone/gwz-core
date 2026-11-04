//! Connection-bound keys through the connection host's HTTPS adapter (HTTPS model draft r5 section 3.1; sub-step H3a,
//! rows H3a-1..5): the operation learns a key `ConnectionBound` when the logon session answered its advertisement's
//! challenge (G8, `Challenged`) or the advertisement was served unchallenged on a connection the logon session had
//! authenticated (G2, `OnBound`); the exchange then says its connection is bound (`ExchangeIo::connection_bound`), and
//! the binding prefers that connection for the key's next exchange. The re-authentication of sub-steps H3b-H3d is in
//! the child modules.
//!
//! The fixture server is NTLM-style: it authenticates a connection when a request on it carries a logon-session token
//! and remembers that, and it challenges each request on any other connection (`challenging` in `https_tests.rs` is per
//! request instead). The logon session is the native module's fake provider, so the rows run on every platform. The
//! server records each request's Authorization scheme and never its value.
use super::{
    https_tests::{ADVERTISEMENT, Case, Parts, Wrap, body, context_of, operation_of},
    tests::block_on,
};
use crate::git::endpoint::{
    helper_script,
    https_adapter::Binding,
    https_auth::{
        OperationAuth,
        needs::{Evidence, NeedKey, Recorded},
    },
    https_destination::Destination,
    https_fixture::{Behaviour, ConnectionId as Served, Handler, response},
    https_worker::native,
};
use bytes::Bytes;
use gwz_connection::{
    AuthContext, Completion, ConnectCommand, EndpointAdapter, Exchange, ExchangeIo, ExchangeSpec,
    GitService, HttpResponseHead, Operation, ProofId, ReuseCommand, StartError, StreamError,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{self, Disposition, Effect},
};
use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, LOCATION, WWW_AUTHENTICATE};
use std::{
    collections::BTreeSet,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

mod branches;
mod conditions;
mod reauth;

/// One request the server saw.
#[derive(Clone, Debug)]
struct Seen {
    get: bool,
    /// Its path.
    path: String,
    /// The scheme of its Authorization header (`NTLM`, `Basic`), if it had one; never the value.
    scheme: Option<String>,
    /// The server's number of the connection it came on.
    connection: usize,
    /// Its request line and every header but Authorization, in order (row H3d-2).
    head: Vec<String>,
}
impl Seen {
    /// Whether it carried a logon-session token.
    fn token(&self) -> bool {
        self.scheme.as_deref() == Some("NTLM")
    }
}

/// How the server answers now; a row changes it between exchanges.
#[derive(Default)]
struct Mode {
    /// Every path is served to anyone, on any connection: a server that no longer challenges (RA2, RA2a).
    open: AtomicBool,
    /// `/pub` is challenged like any other path (RA1 on a key learned `OnBound`).
    protect_pub: AtomicBool,
    /// A challenge offers `Basic` only (RA3); otherwise `NTLM`.
    basic_only: AtomicBool,
    /// A logon-session token is refused: a 401 with no continuation (G9, RA4).
    refuse_tokens: AtomicBool,
    /// A POST is challenged even on an authenticated connection (P6).
    refuse_posts: AtomicBool,
    /// A GET on a connection nothing authenticated is answered with a redirect (RA7).
    redirect: AtomicBool,
    /// The 200 a token earns ends its connection before its body does (RA5).
    truncate: AtomicBool,
}

/// The NTLM-style server: what it saw, how it answers, and the connections a token authenticated.
#[derive(Clone, Default)]
struct Ntlm {
    seen: Arc<Mutex<Vec<Seen>>>,
    mode: Arc<Mode>,
    authenticated: Arc<Mutex<BTreeSet<usize>>>,
    /// The advertisement served under `/big` (row H3d-3).
    big: Arc<Mutex<Bytes>>,
}
/// The Git service a request names.
fn service_of(get: bool, target: &str) -> protocol::GitService {
    match (target.contains("receive-pack"), get) {
        (false, true) => protocol::GitService::UploadPackAdvertisement,
        (false, false) => protocol::GitService::UploadPackExchange,
        (true, true) => protocol::GitService::ReceivePackAdvertisement,
        (true, false) => protocol::GitService::ReceivePackExchange,
    }
}
/// How the server answers one request.
enum Answer {
    Serve,
    Challenge,
    Redirect,
    Truncated,
}
impl Ntlm {
    fn handler(&self) -> Handler {
        let server = self.clone();
        Arc::new(move |request| {
            let connection = request.extensions().get::<Served>().map_or(0, |id| id.0);
            let get = request.method() == hyper::Method::GET;
            let path = request.uri().path().to_string();
            let target = request.uri().to_string();
            let scheme = request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(' ').next())
                .map(str::to_owned);
            let mut head = vec![format!("{} {target}", request.method())];
            for (name, value) in request.headers() {
                if name != AUTHORIZATION {
                    head.push(format!(
                        "{name}: {}",
                        String::from_utf8_lossy(value.as_bytes())
                    ));
                }
            }
            let seen = Seen {
                get,
                path: path.clone(),
                scheme,
                connection,
                head,
            };
            let answer = server.answer(&seen);
            server.seen.lock().unwrap().push(seen);
            let service = service_of(get, &target);
            let big = server.big.lock().unwrap().clone();
            let basic = server.mode.basic_only.load(Ordering::SeqCst) || path.starts_with("/basic");
            let incoming = request.into_body();
            Box::pin(async move {
                let _ = http_body_util::BodyExt::collect(incoming).await;
                let served = match (get, path.starts_with("/big")) {
                    (false, _) => Bytes::from_static(b"0000"),
                    (true, true) => big,
                    (true, false) => Bytes::from_static(ADVERTISEMENT),
                };
                match answer {
                    Answer::Serve => response(200, service, served),
                    Answer::Truncated => {
                        // Promises more than it sends: the server ends the connection short of the body's end.
                        let mut cut = response(200, service, served.clone());
                        cut.headers_mut()
                            .insert(CONTENT_LENGTH, (served.len() + 4096).into());
                        cut
                    }
                    Answer::Redirect => {
                        let mut moved = response(302, service, Bytes::new());
                        moved.headers_mut().insert(
                            LOCATION,
                            "/moved/info/refs?service=git-receive-pack".parse().unwrap(),
                        );
                        moved
                    }
                    Answer::Challenge => {
                        let mut challenge = response(401, service, Bytes::new());
                        let offer = if basic {
                            "Basic realm=\"fixture\""
                        } else {
                            "NTLM"
                        };
                        challenge
                            .headers_mut()
                            .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                        challenge
                    }
                }
            })
        })
    }
    fn answer(&self, seen: &Seen) -> Answer {
        let mode = &self.mode;
        let mut authenticated = self.authenticated.lock().unwrap();
        if seen.token() {
            if mode.refuse_tokens.load(Ordering::SeqCst) {
                return Answer::Challenge;
            }
            authenticated.insert(seen.connection);
            if seen.get && mode.truncate.load(Ordering::SeqCst) {
                return Answer::Truncated;
            }
            return Answer::Serve;
        }
        if seen.scheme.as_deref() == Some("Basic") {
            return Answer::Serve;
        }
        let public = mode.open.load(Ordering::SeqCst)
            || (seen.path.starts_with("/pub") && !mode.protect_pub.load(Ordering::SeqCst))
            || seen.path.starts_with("/hold");
        if authenticated.contains(&seen.connection) || public {
            if !seen.get && mode.refuse_posts.load(Ordering::SeqCst) {
                return Answer::Challenge;
            }
            return Answer::Serve;
        }
        if seen.get && mode.redirect.load(Ordering::SeqCst) {
            return Answer::Redirect;
        }
        Answer::Challenge
    }
    /// Every request so far.
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    /// The requests from the `before`th on.
    fn since(&self, before: usize) -> Vec<Seen> {
        self.seen.lock().unwrap()[before..].to_vec()
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

/// The `connection_bound()` each exchange reported when it ended, with its connection, service and repository path.
#[derive(Clone, Default)]
struct Bounds(Arc<Mutex<Vec<Noted>>>);
/// One exchange's connection, service, repository path and `connection_bound()` at its end.
type Noted = (ConnectionId, GitService, String, bool);
impl Bounds {
    fn wrap(&self) -> Wrap {
        let bounds = self.clone();
        Arc::new(move |inner| {
            Box::new(Observing {
                inner,
                bounds: bounds.clone(),
            })
        })
    }
    /// What the exchanges of `service` at `path` reported, in order.
    fn of(&self, service: GitService, path: &str) -> Vec<bool> {
        let all = self.0.lock().unwrap();
        all.iter()
            .filter(|(_, s, p, _)| *s == service && p == path)
            .map(|(_, _, _, bound)| *bound)
            .collect()
    }
}
/// The production adapter shape, with each exchange's `connection_bound()` noted when it ends.
struct Observing {
    inner: Box<dyn EndpointAdapter>,
    bounds: Bounds,
}
impl Observing {
    fn observed(
        &self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        io: Box<dyn ExchangeIo>,
    ) -> Box<dyn ExchangeIo> {
        Box::new(ObservedIo {
            inner: io,
            at: Some((connection, spec.service(), spec.path().to_owned())),
            bounds: self.bounds.clone(),
        })
    }
}
impl EndpointAdapter for Observing {
    fn connect(&mut self, command: ConnectCommand) {
        self.inner.connect(command);
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        self.inner.cancel_connect(connection);
    }
    fn close(&mut self, connection: ConnectionId) {
        self.inner.close(connection);
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        let io = self.inner.start_exchange(connection, spec, auth)?;
        Ok(self.observed(connection, spec, io))
    }
    fn start_exchange_checked(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> Result<Box<dyn ExchangeIo>, StartError> {
        let io = self.inner.start_exchange_checked(connection, spec, auth)?;
        Ok(self.observed(connection, spec, io))
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        self.inner.authorize_reuse(command)
    }
    fn cancel_reuse(&mut self, proof: ProofId) {
        self.inner.cancel_reuse(proof);
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        self.inner.poll_completion(cx)
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_dispose(cx)
    }
}
struct ObservedIo {
    inner: Box<dyn ExchangeIo>,
    at: Option<(ConnectionId, GitService, String)>,
    bounds: Bounds,
}
impl ObservedIo {
    fn note(&mut self) {
        if let Some((connection, service, path)) = self.at.take() {
            let bound = self.inner.connection_bound();
            self.bounds
                .0
                .lock()
                .unwrap()
                .push((connection, service, path, bound));
        }
    }
}
impl ExchangeIo for ObservedIo {
    fn poll_read(&mut self, cx: &mut Context<'_>, out: &mut [u8]) -> Poll<io::Result<usize>> {
        self.inner.poll_read(cx, out)
    }
    fn poll_write(&mut self, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        self.inner.poll_write(cx, data)
    }
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_flush(cx)
    }
    fn poll_shutdown_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_shutdown_write(cx)
    }
    fn poll_response_head(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<HttpResponseHead>> {
        self.inner.poll_response_head(cx)
    }
    fn effect(&self) -> Effect {
        self.inner.effect()
    }
    fn peer_confirmed(&self) -> Option<u64> {
        self.inner.peer_confirmed()
    }
    fn connection_bound(&self) -> bool {
        self.inner.connection_bound()
    }
    fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Disposition> {
        self.note();
        self.inner.poll_finish(cx)
    }
    fn abort(&mut self) {
        self.note();
        self.inner.abort();
    }
}

/// A host over a fresh NTLM-style server, whose adapter notes each exchange's `connection_bound()` in `bounds`.
fn bound_case(server: &Ntlm, bounds: &Bounds) -> Case {
    bound_case_with(server, bounds, Behaviour::default())
}
fn bound_case_with(server: &Ntlm, bounds: &Bounds, behaviour: Behaviour) -> Case {
    let handler = server.handler();
    Case::with_parts(
        |_| handler,
        behaviour,
        |_| (None, None),
        None,
        Parts {
            wrap: Some(bounds.wrap()),
            ..Parts::default()
        },
    )
}
/// The context of an operation whose logon session is the native module's fake provider, which completes at once.
fn logon_context() -> AuthContext {
    context_of(None, Some(native::publication_fixture()))
}
/// The context of an operation with both a credential helper (answering `fixture-token`) and that logon session.
fn helper_and_logon_context(directory: &std::path::Path) -> AuthContext {
    let helper = helper_script::git_fixture(
        directory,
        "printf 'username=fixture\\npassword=fixture-token\\n\\n'",
    );
    context_of(Some(helper), Some(native::publication_fixture()))
}
/// What the record of the operation whose context is `context` says of the push key of `path`.
fn need(case: &Case, context: &AuthContext, path: &str) -> Recorded {
    let key = case.key();
    let destination = Destination::parse(&format!("https://localhost:{}{path}", key.port)).unwrap();
    let need = NeedKey::new(
        &key,
        &destination,
        protocol::GitService::ReceivePackAdvertisement,
    );
    OperationAuth::of(context)
        .expect("core's own provider")
        .needs()
        .answer(&need)
        .unwrap()
}
/// Whether `recorded` is a connection-bound need learned with `evidence`.
fn bound_by(recorded: &Recorded, evidence: Evidence) -> bool {
    matches!(recorded, Recorded::ConnectionBound { evidence: learned, .. } if *learned == evidence)
}
/// Opens the push advertisement of `path` and reads it whole, leaving the exchange open.
fn advertised(case: &Case, operation: &Operation, path: &str) -> Exchange {
    let mut exchange = case.exchange_at(operation, GitService::ReceivePackAdvertisement, path);
    assert_eq!(
        body(&mut exchange).unwrap(),
        ADVERTISEMENT,
        "{path}'s advertisement"
    );
    exchange
}
/// Ends `exchange`, which read its whole response: its connection can serve another.
#[track_caller]
fn finish(exchange: Exchange) {
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
}
/// Ends `exchange`, as `finish`, and waits until no connection is leased.
#[track_caller]
fn settle(case: &Case, exchange: Exchange) {
    finish(exchange);
    case.wait("no connection is leased", |counts| counts.leased == 0);
}
/// A push's POST to `path`: its body, and the response or the stream's failure.
fn push(
    case: &Case,
    operation: &Operation,
    path: &str,
) -> (Exchange, Result<Vec<u8>, StreamError>) {
    let mut exchange = case.exchange_at(operation, GitService::ReceivePackExchange, path);
    let _ = block_on(exchange.stream().write_all(b"0000"));
    let read = body(&mut exchange);
    (exchange, read)
}
/// Whether `read` failed as `Authentication` does.
fn refused<T>(read: &Result<T, StreamError>) -> bool {
    matches!(read, Err(StreamError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
}
/// The server's connection number of the last request to `path`.
fn connection_of(server: &Ntlm, path: &str) -> usize {
    server
        .requests()
        .iter()
        .rev()
        .find(|seen| seen.path.starts_with(path))
        .map(|seen| seen.connection)
        .expect("a request to the path")
}

#[test]
fn h3a_1_a_logon_session_answer_records_the_key_connection_bound_and_challenged() {
    // 3.1 Learning, G8: the advertisement of `/private` is challenged with NTLM and the operation's logon session
    // answers it. The record holds the key `ConnectionBound { Challenged }` with the advertisement's length, the exchange
    // says its connection is bound, and the connection's scope is `LogonSession`.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-learn", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let recorded = need(&case, &context, "/private");
    assert!(
        matches!(recorded, Recorded::ConnectionBound { observed, evidence: Evidence::Challenged } if observed == ADVERTISEMENT.len() as u64),
        "the key was learned as {recorded:?}"
    );
    assert_eq!(
        bounds.of(GitService::ReceivePackAdvertisement, "/private"),
        [true],
        "the exchange did not say its connection is bound"
    );
    let scopes = case.scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].1.binding, Binding::LogonSession);
    let seen = server.requests();
    assert_eq!(seen.len(), 2, "the challenge and the token");
    assert!(!seen[0].token() && seen[1].token() && seen[0].connection == seen[1].connection);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3a_2_a_key_served_unchallenged_on_a_bound_connection_is_learned_on_bound() {
    // Safety P2-2, G2: `/a`'s challenge authenticates c1; `/b` is then served on c1 with no challenge, so it is learned
    // `ConnectionBound { OnBound }` (which proves nothing about its need) and its exchange says its connection is bound.
    // X4: once the record is `Exhausted` (its lock poisoned, X1), a key served unchallenged on c1 is still served and
    // nothing is recorded for it.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-on-bound", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    let before = server.count();
    settle(&case, advertised(&case, &operation, "/b"));
    let b = server.since(before);
    assert!(
        b.len() == 1 && b[0].scheme.is_none() && b[0].connection == connection_of(&server, "/a"),
        "/b was not served unchallenged on /a's connection: {b:?}"
    );
    assert!(bound_by(&need(&case, &context, "/a"), Evidence::Challenged));
    let recorded = need(&case, &context, "/b");
    assert!(
        bound_by(&recorded, Evidence::OnBound),
        "/b was learned as {recorded:?}"
    );
    assert_eq!(
        bounds.of(GitService::ReceivePackAdvertisement, "/b"),
        [true]
    );
    // X1 (the record's lock poisoned; the next call to consult it finds it so), then X4 for G2: `/c` is served on c1,
    // unchallenged, and records nothing.
    let record = OperationAuth::of(&context).unwrap();
    record.poison();
    assert!(
        record.needs().exhausted(),
        "a poisoned lock exhausts the record"
    );
    let counts = OperationAuth::of(&context).unwrap().needs().counts();
    assert!(counts.exhausted);
    let before = server.count();
    settle(&case, advertised(&case, &operation, "/c"));
    let c = server.since(before);
    assert!(
        c.len() == 1 && c[0].scheme.is_none() && c[0].connection == connection_of(&server, "/a"),
        "/c was not served unchallenged on c1: {c:?}"
    );
    assert_eq!(
        record.needs().counts().keys,
        counts.keys,
        "an Exhausted record wrote an entry"
    );
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3a_3_the_post_goes_on_the_connection_its_advertisement_authenticated() {
    // Consistency P3-4: the binding holds anonymous c1 (a public advertisement) and c2, which the logon session
    // authenticated for `/private`'s advertisement; c2 is the higher id. The POST of `/private` goes on c2, by the
    // binding's hint, with no second GET; two connections in all. Then 100 rounds of GET and POST with no wait for idle
    // between: every POST lands on c2 (the pool waits for its finishing lease).
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-placed", context.clone());
    let public = advertised(&case, &operation, "/pub");
    let private = advertised(&case, &operation, "/private");
    finish(private);
    settle(&case, public);
    let c2 = connection_of(&server, "/private");
    assert_ne!(c2, connection_of(&server, "/pub"), "two connections");
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/private");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the POST was refused"
    );
    settle(&case, exchange);
    let sent = server.since(before);
    assert!(
        sent.len() == 1 && !sent[0].get && sent[0].connection == c2 && sent[0].scheme.is_none(),
        "the POST was not sent alone on c2: {sent:?}"
    );
    assert_eq!(case.accepted(), 2);
    for round in 0..100 {
        let mut advertisement =
            case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/private");
        assert_eq!(
            body(&mut advertisement).unwrap(),
            ADVERTISEMENT,
            "round {round}"
        );
        finish(advertisement);
        let (exchange, read) = push(&case, &operation, "/private");
        assert!(read.is_ok(), "round {round}: the POST was refused");
        finish(exchange);
    }
    case.wait("no connection is leased", |counts| counts.leased == 0);
    let rounds = server.since(before + 1);
    assert_eq!(rounds.len(), 200, "a request was added or lost");
    assert!(
        rounds
            .iter()
            .all(|seen| seen.connection == c2 && seen.scheme.is_none()),
        "a round left c2 or was challenged"
    );
    assert_eq!(case.accepted(), 2);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3a_4_another_binding_never_gets_the_bound_connection_and_re_authenticates_its_own() {
    // R4: A's logon session authenticates c1 for `/private`. B, another operation with a logon session of its own,
    // learns `/private` on a connection of its own; with that connection held by B's own exchange, B's POST is placed
    // while c1 is idle: it never gets c1 (the adapter refuses A's bound connection to B), and it re-authenticates a new
    // connection with B's own logon session before its first byte.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let a = operation_of(&case.handle, "op-a", logon_context());
    settle(&case, advertised(&case, &a, "/private"));
    let c1 = connection_of(&server, "/private");
    let b_context = logon_context();
    let b = operation_of(&case.handle, "op-b", b_context.clone());
    let b_first = server.count();
    settle(&case, advertised(&case, &b, "/private"));
    let held = advertised(&case, &b, "/private");
    let before = server.count();
    let (exchange, read) = push(&case, &b, "/private");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "B's POST was refused"
    );
    finish(exchange);
    settle(&case, held);
    let all = server.since(b_first);
    assert!(
        all.iter().all(|seen| seen.connection != c1),
        "B's request went on A's connection: {all:?}"
    );
    let sent = server.since(before);
    let post = sent.iter().position(|seen| !seen.get).expect("B's POST");
    assert!(
        sent[..post]
            .iter()
            .any(|seen| seen.get && seen.token() && seen.connection == sent[post].connection),
        "B's POST was not preceded by a re-authentication on its connection: {sent:?}"
    );
    assert!(bound_by(
        &need(&case, &b_context, "/private"),
        Evidence::Challenged
    ));
    block_on(b.finish());
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn h3a_5_a_basic_key_gets_no_hint_and_sends_no_re_authentication() {
    // 3(d), 3.1: `/basic`'s challenge offers Basic and the helper answers it, on c2 while c1 serves a public
    // advertisement. The exchange does not say its connection is bound, so the binding notes no hint: the POST goes on
    // the lowest idle connection, c1, carrying the record's credential from its first byte, with no GET before it.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let directory = tempfile::tempdir().unwrap();
    let context = helper_and_logon_context(directory.path());
    let operation = operation_of(&case.handle, "op-basic", context.clone());
    let public = advertised(&case, &operation, "/pub");
    let basic = advertised(&case, &operation, "/basic");
    finish(basic);
    settle(&case, public);
    let c1 = connection_of(&server, "/pub");
    assert_ne!(c1, connection_of(&server, "/basic"));
    assert!(matches!(
        need(&case, &context, "/basic"),
        Recorded::Basic(_)
    ));
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/basic");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the POST was refused"
    );
    settle(&case, exchange);
    let sent = server.since(before);
    assert!(
        sent.len() == 1
            && !sent[0].get
            && sent[0].scheme.as_deref() == Some("Basic")
            && sent[0].connection == c1,
        "the POST did not go alone, with the credential, on the lowest idle connection: {sent:?}"
    );
    assert_eq!(
        bounds.of(GitService::ReceivePackAdvertisement, "/basic"),
        [false]
    );
    assert_eq!(
        bounds.of(GitService::ReceivePackExchange, "/basic"),
        [false]
    );
    assert!(
        server.requests().iter().all(|seen| !seen.token()),
        "a logon-session token was sent"
    );
    block_on(operation.finish());
    assert!(case.close());
}
