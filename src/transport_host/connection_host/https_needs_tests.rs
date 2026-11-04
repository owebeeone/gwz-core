//! The operation's credential record driving the connection host's HTTPS adapter (HTTPS model draft r5 sub-steps H1b,
//! rows H1b-1..4, and H1c, rows H1c-1..6; contract 7.2): a key an advertisement of the operation learned is answered,
//! and its POST sent, with the credential the record holds, from its first byte, on whichever connection the pool
//! leased; a credential refused (401) is rejected for every key of its origin holding it, and never sent again. The
//! fixture servers record the SHA-256 of each Authorization header and never the header, so no failure message holds a
//! credential. The record's bounds (sub-step H1d, rows H1d-1..5) are in `overflow`.
mod overflow;
use super::{
    https_tests::{ADVERTISEMENT, Case, body, context_of, operation_of, port_of},
    tests::block_on,
};
use crate::git::endpoint::{
    helper_script,
    https_adapter::Binding,
    https_auth,
    https_fixture::{Handler, Server, response},
};
use bytes::Bytes;
use gwz_connection::{
    AuthContext, Exchange, ExchangeSpec, GitService, HostPhase, Operation, StreamError,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{self, Disposition},
};
use hyper::header::AUTHORIZATION;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// One request the fixture server saw.
#[derive(Clone)]
struct Seen {
    post: bool,
    path: String,
    /// The SHA-256 of its Authorization header, when it had one.
    digest: Option<[u8; 32]>,
    connection: usize,
}
type Requests = Arc<Mutex<Vec<Seen>>>;
/// The SHA-256 digests of the Authorization headers a server answers 401, as a server that no longer accepts that
/// credential does.
type Refused = Arc<Mutex<Vec<[u8; 32]>>>;

/// A server for both Git services that answers 401 (`Basic`) to any request without an Authorization header, or with
/// one `refused` names, except under `/public`, and records each request. An advertisement under `/big` is `big`; any
/// other is the usual one. It reads a request's whole body before it answers, so that a POST's connection stays
/// reusable whatever the timing.
fn authorizing(requests: &Requests, big: Bytes, refused: &Refused) -> Handler {
    let (requests, refused) = (requests.clone(), refused.clone());
    Arc::new(move |request| {
        let post = request.method() == hyper::Method::POST;
        let path = request.uri().path().to_string();
        let digest = request
            .headers()
            .get(AUTHORIZATION)
            .map(|value| Sha256::digest(value.as_bytes()).into());
        let connection = request
            .extensions()
            .get::<crate::git::endpoint::https_fixture::ConnectionId>()
            .map_or(0, |id| id.0);
        let receive = request.uri().to_string().contains("receive-pack");
        let service = match (receive, post) {
            (false, false) => protocol::GitService::UploadPackAdvertisement,
            (false, true) => protocol::GitService::UploadPackExchange,
            (true, false) => protocol::GitService::ReceivePackAdvertisement,
            (true, true) => protocol::GitService::ReceivePackExchange,
        };
        let accepted = digest.is_some_and(|digest| !refused.lock().unwrap().contains(&digest));
        let served = accepted || path.starts_with("/public");
        let answer = match (served, post) {
            (false, _) => Bytes::new(),
            (true, true) => Bytes::from_static(b"0000"),
            (true, false) if path.starts_with("/big") => big.clone(),
            (true, false) => Bytes::from_static(ADVERTISEMENT),
        };
        let status = if served { 200 } else { 401 };
        requests.lock().unwrap().push(Seen {
            post,
            path,
            digest,
            connection,
        });
        let incoming = request.into_body();
        Box::pin(async move {
            let _ = http_body_util::BodyExt::collect(incoming).await;
            response(status, service, answer)
        })
    })
}

/// The SHA-256 of the header a helper answering `token` makes (`Basic` of `fixture:<token>`).
fn basic_digest(token: &str) -> [u8; 32] {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    Sha256::digest(format!(
        "Basic {}",
        STANDARD.encode(format!("fixture:{token}"))
    ))
    .into()
}

/// A helper that answers `GH_TOKEN` and counts its runs in `directory`'s `count` file.
fn counting_helper(directory: &Path, token: &str) -> https_auth::Config {
    let mut config = helper_script::git_fixture(
        directory,
        "printf 'lookup\\n' >> \"$COUNTER\"\nprintf 'username=fixture\\npassword=%s\\n\\n' \"$GH_TOKEN\"",
    );
    config.environment.push((
        "COUNTER".into(),
        helper_script::script_path(&directory.join("count")).into(),
    ));
    config.environment.push(("GH_TOKEN".into(), token.into()));
    config
}
/// A helper that answers `OTHER_TOKEN` for a URL under `/other` and `GH_TOKEN` for any other, counting its runs in
/// `directory`'s `count` file: two unrelated credentials at one origin.
fn routing_helper(directory: &Path, token: &str, other: &str) -> https_auth::Config {
    let mut config = helper_script::git_fixture(
        directory,
        "printf 'lookup\\n' >> \"$COUNTER\"\nread -r request\ncase \"$request\" in\n *'/other'*) token=\"$OTHER_TOKEN\" ;;\n *) token=\"$GH_TOKEN\" ;;\nesac\nprintf 'username=fixture\\npassword=%s\\n\\n' \"$token\"",
    );
    config.environment.push((
        "COUNTER".into(),
        helper_script::script_path(&directory.join("count")).into(),
    ));
    config.environment.push(("GH_TOKEN".into(), token.into()));
    config
        .environment
        .push(("OTHER_TOKEN".into(), other.into()));
    config
}
/// How often the helper of `directory` ran.
fn lookups(directory: &Path) -> usize {
    std::fs::read_to_string(directory.join("count")).map_or(0, |text| text.lines().count())
}

/// A host over the authorizing server, whose instance holds no helper (each operation's own is in its context).
fn case(requests: &Requests, big: Bytes) -> Case {
    refusing(requests, big, &Refused::default())
}
/// `case`, over a server that answers 401 to the credentials `refused` names.
fn refusing(requests: &Requests, big: Bytes, refused: &Refused) -> Case {
    Case::with(
        |_| authorizing(requests, big, refused),
        Default::default(),
        |_| (None, None),
        None,
    )
}
/// A second origin (draft r5 3(a): scheme, host and port): another fixture server, on its own port and runtime. The
/// case's adapter trusts it through the fixture CA every fixture server shares.
struct Origin {
    /// The server runs on it.
    _runtime: tokio::runtime::Runtime,
    server: Server,
}
impl Origin {
    fn start(handler: Handler) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let server = runtime.block_on(Server::start(handler));
        Self {
            _runtime: runtime,
            server,
        }
    }
    /// An exchange of `service` at the repository `path` of this origin.
    fn exchange_at(&self, operation: &Operation, service: GitService, path: &str) -> Exchange {
        let key = Key::https("localhost", port_of(&self.server));
        let spec = ExchangeSpec::git(key, Identity::Https, service, path).unwrap();
        block_on(operation.open(spec).unwrap().wait()).unwrap()
    }
}
/// The record of the operation whose context is `context`.
fn counts(context: &AuthContext) -> https_auth::needs::Counts {
    https_auth::OperationAuth::of(context)
        .expect("core's own provider")
        .needs()
        .counts()
}

/// Opens an advertisement of `/repo`-style `path` and reads it whole, leaving the exchange open.
fn advertised(case: &Case, operation: &Operation, path: &str) -> Exchange {
    let mut exchange = case.exchange_at(operation, GitService::ReceivePackAdvertisement, path);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
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
    posted(case.exchange_at(operation, GitService::ReceivePackExchange, path))
}
/// Writes a POST's body on `exchange` and reads its response or the stream's failure.
fn posted(mut exchange: Exchange) -> (Exchange, Result<Vec<u8>, StreamError>) {
    let _ = block_on(exchange.stream().write_all(b"0000"));
    let read = body(&mut exchange);
    (exchange, read)
}
/// Whether `read` failed as `Authentication` does.
fn refused<T>(read: &Result<T, StreamError>) -> bool {
    matches!(read, Err(StreamError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
}
/// Ends `exchange`, which failed: its connection cannot serve another.
#[track_caller]
fn discarded(exchange: Exchange) {
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
}
/// The requests the server saw from the `before`th on.
fn since(requests: &Requests, before: usize) -> Vec<Seen> {
    requests.lock().unwrap()[before..].to_vec()
}

#[test]
fn h1b_1_a_learned_post_carries_the_recorded_credential_on_an_anonymous_connection() {
    // PR5: one binding holds anonymous c1 (a public advertisement) and c2, which the private advertisement's challenge
    // authenticated. The pool gives the private POST c1, the lowest idle connection: it carries the credential the
    // record holds from its first byte. The helper ran once: three POSTs and a later challenge spawn it zero times.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let operation = operation_of(&case.handle, "op-pr5", context_of(Some(helper), None));
    let public = advertised(&case, &operation, "/public");
    let private = advertised(&case, &operation, "/private");
    finish(private);
    settle(&case, public);
    assert_eq!(case.accepted(), 2);
    assert_eq!(lookups(directory.path()), 1, "the challenge was answered");
    let anonymous = requests.lock().unwrap()[0].connection;
    for round in 0..3 {
        let (exchange, read) = push(&case, &operation, "/private");
        assert!(read.is_ok(), "POST round {round} was refused");
        settle(&case, exchange);
    }
    // W1: the POST that carried the record's credential marked its connection a helper token's. Checked before the later
    // challenge, which is answered on c1 too and would mark it whatever the POST reported: until then c1 carried only the
    // anonymous GET and the POSTs.
    let scopes = case.scopes();
    assert_eq!(scopes.len(), 2);
    assert!(
        scopes
            .iter()
            .all(|(_, scope)| scope.binding == Binding::HelperToken),
        "a POST with the record's credential left its connection unmarked"
    );
    // A later challenge to the key's advertisement is answered from the record.
    settle(&case, advertised(&case, &operation, "/private"));
    block_on(operation.finish());
    let seen = requests.lock().unwrap().clone();
    let posts: Vec<_> = seen.iter().filter(|s| s.post).collect();
    assert_eq!(posts.len(), 3, "each POST was sent once");
    assert!(
        posts
            .iter()
            .all(|s| s.digest == Some(basic_digest("fixture-token-a"))),
        "a POST went without the recorded credential"
    );
    assert_eq!(
        posts[0].connection, anonymous,
        "the first POST went on the anonymous connection"
    );
    let last = seen.last().unwrap();
    assert!(!last.post && last.digest == Some(basic_digest("fixture-token-a")));
    assert_eq!(lookups(directory.path()), 1, "the helper ran once in all");
    assert_eq!(case.accepted(), 2);
    assert!(case.close());
}

#[test]
fn h1b_2_a_post_is_authenticated_after_another_key_was_learned_on_its_connection() {
    // PR6: on one connection, GET `/a` and GET `/b` are each challenged and answered; POST `/a` is authenticated, though
    // `/b` was the last key learned there.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let operation = operation_of(&case.handle, "op-pr6", context_of(Some(helper), None));
    settle(&case, advertised(&case, &operation, "/a"));
    settle(&case, advertised(&case, &operation, "/b"));
    let (exchange, read) = push(&case, &operation, "/a");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "POST /a was refused"
    );
    settle(&case, exchange);
    block_on(operation.finish());
    let seen = requests.lock().unwrap().clone();
    let posts: Vec<_> = seen.iter().filter(|s| s.post).collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/a/git-receive-pack");
    assert!(posts[0].digest == Some(basic_digest("fixture-token-a")));
    assert_eq!(case.accepted(), 1, "one connection carried all three");
    assert!(seen.iter().all(|s| s.connection == seen[0].connection));
    assert_eq!(lookups(directory.path()), 2, "one lookup per key learned");
    assert!(case.close());
}

#[test]
fn h1b_3_another_operations_post_never_gets_the_record() {
    // R4: A learns `/private`. B, with an equal-scope context and then with a different-scope one, POSTs `/private` on
    // the same host and instance: no Authorization on any of B's requests, B's helper is not started, and the POST is
    // refused as `Authentication`. The record is A's operation's, in no adapter-shared state.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper.clone()), None);
    let a = operation_of(&case.handle, "op-a", context.clone());
    settle(&case, advertised(&case, &a, "/private"));
    let mut other = helper.clone();
    other.environment.retain(|(name, _)| name != "GH_TOKEN");
    other
        .environment
        .push(("GH_TOKEN".into(), "fixture-token-b".into()));
    let equal = context_of(Some(helper), None);
    let different = context_of(Some(other), None);
    assert!(equal.scope() == context.scope() && different.scope() != context.scope());
    for (name, b_context) in [("op-b-equal", equal), ("op-b-other", different)] {
        let before = requests.lock().unwrap().len();
        let b = operation_of(&case.handle, name, b_context.clone());
        let (exchange, read) = push(&case, &b, "/private");
        assert!(
            matches!(&read, Err(StreamError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied),
            "{name}: the POST was not refused as Authentication"
        );
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Discarded
        );
        block_on(b.finish());
        let seen = requests.lock().unwrap()[before..].to_vec();
        assert_eq!(seen.iter().filter(|s| s.post).count(), 1, "{name}");
        assert!(
            seen.iter().all(|s| s.digest.is_none()),
            "{name}: a request carried a credential"
        );
        assert!(
            !seen
                .iter()
                .any(|s| s.digest == Some(basic_digest("fixture-token-a"))),
            "{name}: A's credential was sent"
        );
        assert_eq!(counts(&b_context).keys, 0, "{name} learned nothing");
    }
    assert_eq!(lookups(directory.path()), 1, "B's helper never ran");
    assert_eq!(counts(&context).keys, 1, "A's own record holds the key");
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn h1b_4_the_record_is_written_before_the_first_advertisement_byte() {
    // Draft 3(h): an advertisement's key is recorded before any byte of its body reaches the client. Each round reads one
    // byte of a fresh key's advertisement (1 MiB, which the exchange cannot finish reading while its client holds it),
    // then opens the key's POST, which the pool places on another connection: every POST is authenticated.
    let requests = Requests::default();
    let big = Bytes::from(vec![b'0'; 1 << 20]);
    let case = case(&requests, big.clone());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-race", context.clone());
    for round in 0..100 {
        let path = format!("/big-{round}");
        let mut advertisement =
            case.exchange_at(&operation, GitService::ReceivePackAdvertisement, &path);
        let mut first = [0u8; 1];
        assert_eq!(
            block_on(advertisement.stream().read(&mut first)).unwrap(),
            1
        );
        let (exchange, read) = push(&case, &operation, &path);
        if let Err(error) = &read {
            panic!("round {round}: the POST was refused: {error:?}");
        }
        finish(exchange);
        let rest = body(&mut advertisement).unwrap();
        assert_eq!(rest.len() + 1, big.len());
        settle(&case, advertisement);
        let prefix = format!("{path}/");
        let seen: Vec<_> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path.starts_with(&prefix))
            .cloned()
            .collect();
        let post = seen.iter().find(|s| s.post).expect("the POST was sent");
        assert!(
            post.digest == Some(basic_digest("fixture-token-a")),
            "round {round}: the POST went without the credential"
        );
        assert!(
            seen.iter()
                .filter(|s| !s.post)
                .all(|s| s.connection != post.connection),
            "round {round}: the POST shared the advertisement's connection"
        );
    }
    assert_eq!(counts(&context).keys, 100);
    // Two advertisements of one key at once each answer their own challenge; the first insert wins.
    let a = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/twice");
    let b = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/twice");
    for mut exchange in [a, b] {
        assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
        finish(exchange);
    }
    let after = counts(&context);
    assert_eq!(
        after.keys, 101,
        "two advertisements of one key left one entry"
    );
    assert_eq!(
        after.credentials, 1,
        "equal bytes at one origin share one owner"
    );
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1c_1_only_the_learned_repository_and_class_carry_the_credential() {
    // Draft r5 3(a): `/public`, served anonymously, records nothing, and its POST stays anonymous. `/repo` is learned
    // for push. `/repo.git` is another key (PR7) and an upload-pack POST to `/repo` another class: each goes out
    // anonymously, once, and its challenge is refused, never answered.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-keys", context.clone());
    settle(&case, advertised(&case, &operation, "/public"));
    assert_eq!(
        counts(&context).keys,
        0,
        "an anonymously served advertisement records nothing"
    );
    let (exchange, read) = push(&case, &operation, "/public");
    assert_eq!(read.ok().as_deref(), Some(&b"0000"[..]));
    settle(&case, exchange);
    settle(&case, advertised(&case, &operation, "/repo"));
    assert_eq!(counts(&context).keys, 1);
    let before = requests.lock().unwrap().len();
    for (service, path) in [
        (GitService::ReceivePackExchange, "/repo.git"),
        (GitService::UploadPackExchange, "/repo"),
    ] {
        let (exchange, read) = posted(case.exchange_at(&operation, service, path));
        assert!(refused(&read), "{path} {service:?} was not refused");
        discarded(exchange);
    }
    block_on(operation.finish());
    let seen = requests.lock().unwrap().clone();
    let after = &seen[before..];
    assert_eq!(after.len(), 2, "each POST was sent once and not answered");
    assert!(
        after.iter().all(|s| s.post && s.digest.is_none()),
        "a POST of another key or class carried the credential"
    );
    assert!(
        seen.iter()
            .filter(|s| s.path.starts_with("/public"))
            .all(|s| s.digest.is_none()),
        "a request of `/public` carried a credential"
    );
    assert_eq!(lookups(directory.path()), 1);
    assert!(case.close());
}

#[test]
fn h1c_2_a_fresh_credential_refused_to_its_advertisement_rejects_the_key() {
    // G6: the helper's credential answers `/repo`'s challenge and is refused. The key is `Rejected` and only the
    // credential's fingerprint is kept. The key's next advertisement is challenged and not answered: nothing credentialed
    // and no helper (G11). The key's POST sends no request and runs no helper (P7).
    let (requests, refused_set) = (Requests::default(), Refused::default());
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-a"));
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-g6", context.clone());
    let mut exchange = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/repo");
    assert!(refused(&body(&mut exchange)), "the refused answer");
    discarded(exchange);
    let after = counts(&context);
    assert_eq!(
        (after.keys, after.credentials, after.fingerprints),
        (1, 0, 1),
        "the key is rejected and only a fingerprint is kept"
    );
    let seen = requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "the GET and its answer");
    assert_eq!(seen[1].digest, Some(basic_digest("fixture-token-a")));
    let before = seen.len();
    let mut exchange = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/repo");
    assert!(refused(&body(&mut exchange)), "the retried advertisement");
    discarded(exchange);
    let retry = since(&requests, before);
    assert_eq!(retry.len(), 1, "the retried challenge was answered");
    assert!(!retry[0].post && retry[0].digest.is_none());
    let before = requests.lock().unwrap().len();
    let (exchange, read) = push(&case, &operation, "/repo");
    assert!(refused(&read), "the rejected key's POST");
    discarded(exchange);
    assert!(
        since(&requests, before).is_empty(),
        "the rejected key's POST reached the server"
    );
    assert_eq!(lookups(directory.path()), 1, "the helper ran once in all");
    block_on(operation.finish());
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(case.close());
}

#[test]
fn h1c_3_a_refused_post_rejects_its_credential_for_every_key_of_the_origin_holding_it() {
    // P2: `/a` and `/b` are learned with T, `/other` with U. The server stops accepting T. POST `/a` carries T and is
    // refused: `Authentication`, `Discarded`, one POST, its connection closed, the host `Open`, and T's buffers read zero
    // at once. The next POST of `/a`, and of `/b`, which held T, ends before a byte; `/other`'s POST, with U, succeeds.
    let (requests, refused_set) = (Requests::default(), Refused::default());
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let directory = tempfile::tempdir().unwrap();
    let helper = routing_helper(directory.path(), "fixture-token-a", "fixture-token-u");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-p2", context.clone());
    for path in ["/a", "/b", "/other"] {
        settle(&case, advertised(&case, &operation, path));
    }
    let learned = counts(&context);
    assert_eq!((learned.keys, learned.credentials), (3, 2));
    let wipes = https_auth::OperationAuth::of(&context)
        .unwrap()
        .needs()
        .wipes();
    assert!(wipes.lock().unwrap().is_empty());
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-a"));
    let open = case.handle.snapshot().connections.total();
    let before = requests.lock().unwrap().len();
    let (exchange, read) = push(&case, &operation, "/a");
    assert!(refused(&read), "the POST with the refused credential");
    let wiped = wipes.lock().unwrap().clone();
    assert!(
        wiped.len() == 2 && wiped.iter().all(|(_, _, zero)| *zero),
        "T's username and password were not wiped when the POST failed: {wiped:?}"
    );
    discarded(exchange);
    case.wait("the refused POST's connection is closed", |counts| {
        counts.leased == 0 && counts.closing == 0 && counts.total() + 1 == open
    });
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    let sent = since(&requests, before);
    assert_eq!(sent.len(), 1, "the POST was sent once, never replayed");
    assert!(sent[0].post && sent[0].digest == Some(basic_digest("fixture-token-a")));
    let rejected = counts(&context);
    assert_eq!((rejected.credentials, rejected.fingerprints), (1, 1));
    for path in ["/a", "/b"] {
        let before = requests.lock().unwrap().len();
        let (exchange, read) = push(&case, &operation, path);
        assert!(refused(&read), "{path}'s POST after the rejection");
        discarded(exchange);
        assert!(
            since(&requests, before).is_empty(),
            "{path}'s POST reached the server"
        );
    }
    let (exchange, read) = push(&case, &operation, "/other");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the unrelated credential"
    );
    settle(&case, exchange);
    let last = requests.lock().unwrap().last().cloned().unwrap();
    assert!(last.post && last.digest == Some(basic_digest("fixture-token-u")));
    assert_eq!(lookups(directory.path()), 3, "one lookup per key learned");
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1c_4_a_rejection_at_one_origin_leaves_another_origins_equal_credential_usable() {
    // Safety P3-1(b): one helper answers T at origin A (the case's server) and at origin B (a second fixture server); both
    // learn `/repo`. A stops accepting T, and A's POST rejects it there. B's entry, the same bytes, is its own: B's POST
    // carries T and succeeds, and a fresh lookup at B for a new key is not refused by A's fingerprint.
    let (requests, refused_set) = (Requests::default(), Refused::default());
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let b_requests = Requests::default();
    let b = Origin::start(authorizing(&b_requests, Bytes::new(), &Refused::default()));
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-origins", context.clone());
    settle(&case, advertised(&case, &operation, "/repo"));
    let mut exchange = b.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/repo");
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    finish(exchange);
    let learned = counts(&context);
    assert_eq!(
        (learned.keys, learned.credentials),
        (2, 2),
        "equal bytes are never shared across origins"
    );
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-a"));
    let (exchange, read) = push(&case, &operation, "/repo");
    assert!(refused(&read), "A's POST with the refused credential");
    discarded(exchange);
    let rejected = counts(&context);
    assert_eq!((rejected.credentials, rejected.fingerprints), (1, 1));
    let (exchange, read) =
        posted(b.exchange_at(&operation, GitService::ReceivePackExchange, "/repo"));
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "B's POST after A's rejection"
    );
    finish(exchange);
    let mut exchange = b.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/fresh");
    assert_eq!(
        body(&mut exchange).unwrap(),
        ADVERTISEMENT,
        "B's fresh lookup of the same bytes"
    );
    finish(exchange);
    block_on(operation.finish());
    let seen = b_requests.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|s| s.post && s.digest == Some(basic_digest("fixture-token-a")))
    );
    assert_eq!(
        seen.last().unwrap().digest,
        Some(basic_digest("fixture-token-a"))
    );
    assert_eq!(lookups(directory.path()), 3);
    assert!(case.close());
}

#[test]
fn h1c_5_equal_bytes_for_another_key_after_a_rejection_are_never_sent() {
    // Safety P3-1(d), G7: the fresh credential T is refused to `/k1`'s answer (G6). `/k2`'s lookup returns T again: it is
    // not sent, the advertisement ends `Authentication` with only its anonymous GET on the wire, and `/k2`'s POST sends
    // nothing. Likewise for a credential the record held: R, learned for `/r1` and refused to its POST (P2), is not sent
    // for `/r2`.
    let (requests, refused_set) = (Requests::default(), Refused::default());
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-t"));
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let fresh_directory = tempfile::tempdir().unwrap();
    let fresh_helper = counting_helper(fresh_directory.path(), "fixture-token-t");
    let fresh_context = context_of(Some(fresh_helper), None);
    let fresh = operation_of(&case.handle, "op-fresh", fresh_context.clone());
    let mut exchange = case.exchange_at(&fresh, GitService::ReceivePackAdvertisement, "/k1");
    assert!(refused(&body(&mut exchange)));
    discarded(exchange);
    let before = requests.lock().unwrap().len();
    let mut exchange = case.exchange_at(&fresh, GitService::ReceivePackAdvertisement, "/k2");
    assert!(refused(&body(&mut exchange)), "/k2's advertisement");
    discarded(exchange);
    let sent = since(&requests, before);
    assert!(
        sent.len() == 1 && sent[0].digest.is_none(),
        "the rejected bytes were sent for /k2"
    );
    assert_eq!(
        lookups(fresh_directory.path()),
        2,
        "/k2's lookup returned the bytes"
    );
    assert_eq!(counts(&fresh_context).keys, 2);
    let before = requests.lock().unwrap().len();
    let (exchange, read) = push(&case, &fresh, "/k2");
    assert!(refused(&read));
    discarded(exchange);
    assert!(
        since(&requests, before).is_empty(),
        "/k2's POST reached the server"
    );
    block_on(fresh.finish());
    let record_directory = tempfile::tempdir().unwrap();
    let record_helper = counting_helper(record_directory.path(), "fixture-token-r");
    let recorded = operation_of(
        &case.handle,
        "op-record",
        context_of(Some(record_helper), None),
    );
    settle(&case, advertised(&case, &recorded, "/r1"));
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-r"));
    let (exchange, read) = push(&case, &recorded, "/r1");
    assert!(refused(&read));
    discarded(exchange);
    let before = requests.lock().unwrap().len();
    let mut exchange = case.exchange_at(&recorded, GitService::ReceivePackAdvertisement, "/r2");
    assert!(refused(&body(&mut exchange)), "/r2's advertisement");
    discarded(exchange);
    let sent = since(&requests, before);
    assert!(
        sent.len() == 1 && sent[0].digest.is_none(),
        "the rejected record credential was sent for /r2"
    );
    assert_eq!(lookups(record_directory.path()), 2);
    block_on(recorded.finish());
    assert!(case.close());
}

#[test]
fn h1c_6_a_rejected_keys_advertisement_is_not_answered() {
    // G11: `/a` is learned with T, and T refused to its POST (P2) rejects the key. Its next advertisement is challenged
    // and answered neither from the record nor by the helper: one anonymous GET, `Authentication`.
    let (requests, refused_set) = (Requests::default(), Refused::default());
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let operation = operation_of(&case.handle, "op-g11", context_of(Some(helper), None));
    settle(&case, advertised(&case, &operation, "/a"));
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-a"));
    let (exchange, read) = push(&case, &operation, "/a");
    assert!(refused(&read));
    discarded(exchange);
    let before = requests.lock().unwrap().len();
    let mut exchange = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/a");
    assert!(
        refused(&body(&mut exchange)),
        "the rejected key's advertisement"
    );
    discarded(exchange);
    let sent = since(&requests, before);
    assert_eq!(sent.len(), 1, "the challenge was answered");
    assert!(!sent[0].post && sent[0].digest.is_none());
    assert_eq!(lookups(directory.path()), 1, "the helper ran again");
    block_on(operation.finish());
    assert!(case.close());
}
