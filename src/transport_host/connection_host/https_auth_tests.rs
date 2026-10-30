//! The connection host's HTTPS adapter answering challenges, against the fixture HTTPS servers (plan step 1.3c;
//! contract 7.2 and 7.3): a helper token and the logon session each mark the connection they authenticated
//! per-exchange scoped, and the logon-session route fails closed while a gate cannot demonstrate its scope. The
//! logon session here is the native module's fake provider; real SSPI runs on Windows (`https_sspi_tests.rs`).
use super::{
    https_tests::{ADVERTISEMENT, Case, Unavailable, body, challenging, context_of, operation_of},
    tests::{block_on, operation},
};
use crate::git::{
    endpoint::{
        helper_script,
        https_adapter::Binding,
        https_auth,
        https_fixture::{Behaviour, Handler, response},
        https_worker::native::{self, NativeHost},
    },
    spawn_gate::{GateFault, SpawnGate},
};
use gwz_connection::{GitService, HostPhase, StreamError};
use gwz_transport::protocol::{self, Disposition};
use hyper::header::AUTHORIZATION;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A credential helper fixture that gives one Basic credential and counts its lookups.
fn helper(directory: &std::path::Path) -> https_auth::Config {
    let counter = directory.join("count");
    let mut config = crate::git::endpoint::helper_script::git_fixture(
        directory,
        "printf 'lookup\\n' >> \"$COUNTER\"\nprintf 'username=fixture\\npassword=private-fixture\\n\\n'",
    );
    config.environment.push((
        "COUNTER".into(),
        crate::git::endpoint::helper_script::script_path(&counter).into(),
    ));
    config
}

#[test]
fn a_helper_token_authenticated_connection_is_marked_per_exchange_scoped() {
    let case = Case::with(
        |seen| challenging(seen, "Basic realm=\"fixture\""),
        Behaviour::default(),
        |directory| (Some(helper(directory)), None),
        None,
    );
    let a = case.operation("op-a");
    let mut exchange = case.exchange(&a, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    // The challenge and the helper's answer went on one connection.
    let seen = case.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [false, true]);
    assert_eq!(seen[0].1, seen[1].1);
    let scopes = case.scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].1.binding, Binding::HelperToken);
    assert!(scopes[0].1.binding.per_exchange());
    assert!(
        scopes[0].1.exchange.is_some(),
        "the exchange that bound it is named"
    );
    assert!(case.close());
}

#[test]
fn a_helper_token_connection_is_never_leased_to_another_binding() {
    // Contract 7.2 and step 1.6: the connection a helper token authenticated serves only the binding that used it, so
    // B opens a connection of its own (and is challenged on it).
    let case = Case::with(
        |seen| challenging(seen, "Basic realm=\"fixture\""),
        Behaviour::default(),
        |directory| (Some(helper(directory)), None),
        None,
    );
    for name in ["op-a", "op-b"] {
        let operation = case.operation(name);
        let mut exchange = case.exchange(&operation, true);
        assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
        block_on(operation.finish());
    }
    assert_eq!(case.accepted(), 2, "B did not reuse A's connection");
    assert!(case.close());
}

#[test]
fn the_logon_session_route_marks_its_connection_per_exchange_scoped() {
    // A logon session whose provider completes at once (the native module's fake port), against a server that
    // challenges with NTLM: the connection the session authenticated is bound to it.
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        |_| (None, Some(native::publication_fixture())),
        None,
    );
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    let seen = case.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [false, true]);
    assert_eq!(
        seen[0].1, seen[1].1,
        "the token went on the challenged connection"
    );
    let scopes = case.scopes();
    assert_eq!(scopes[0].1.binding, Binding::LogonSession);
    assert!(scopes[0].1.binding.per_exchange());
    assert!(case.close());
}

/// The exchange is refused, the server saw no credential and the logon session took no step.
fn refused_before_any_credential(case: &Case, steps: &AtomicUsize) {
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    assert!(body(&mut exchange).is_err(), "the native route refuses");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(a.finish());
    let seen = case.seen.lock().unwrap().clone();
    assert!(
        seen.iter().all(|(authorized, _)| !authorized),
        "no token was sent: {seen:?}"
    );
    assert_eq!(steps.load(Ordering::SeqCst), 0, "no SSPI step was taken");
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(case.close());
}

#[test]
fn the_logon_session_route_refuses_while_the_process_gate_is_unavailable() {
    // The composition's gate cannot demonstrate its scope: the route fails closed (contract 7.3).
    let (caller, steps) = native::counting_fixture("localhost");
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        move |_| (None, Some(caller)),
        Some(Arc::new(Unavailable)),
    );
    refused_before_any_credential(&case, &steps);
    // The caller's own gate gave its supervisor no window (`NativeHost::on_gate`): no port, so no conversation.
    let host = NativeHost::on_gate(SpawnGate::faulted(GateFault::NotCreated), |_| {
        panic!("no supervisor is made on an unavailable gate")
    });
    let none = AtomicUsize::new(0);
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        move |_| (None, Some(host.caller())),
        None,
    );
    refused_before_any_credential(&case, &none);
}

// Step 1.3c/C2(b) (contract 7.2, section 15 rows for 1.5 and 1.6): an exchange authenticates with the AuthContext of the
// operation it serves, and a connection one context authenticated is leased to no other. The rows run the real adapter
// against the fixture server, whose recorded requests carry the digest of each Authorization header and never the
// header, so a failure message holds no credential.

/// One request the fixture server saw: whether it was a POST, the SHA-256 of its Authorization header (none when it
/// had none) and the connection it came on.
#[derive(Clone, Copy)]
struct Seen {
    post: bool,
    digest: Option<[u8; 32]>,
    connection: usize,
}
type Requests = Arc<Mutex<Vec<Seen>>>;

/// The SHA-256 of the header a helper answering `token` makes (`Basic` of `fixture:<token>`).
fn basic_digest(token: &str) -> [u8; 32] {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    Sha256::digest(format!(
        "Basic {}",
        STANDARD.encode(format!("fixture:{token}"))
    ))
    .into()
}

/// A server for both Git services that answers 401 (`Basic`) to any request without an Authorization header, and records
/// each request.
fn authorizing(requests: &Requests) -> Handler {
    let requests = requests.clone();
    Arc::new(move |request| {
        let post = request.method() == hyper::Method::POST;
        let digest = request
            .headers()
            .get(AUTHORIZATION)
            .map(|value| Sha256::digest(value.as_bytes()).into());
        let connection = request
            .extensions()
            .get::<crate::git::endpoint::https_fixture::ConnectionId>()
            .map_or(0, |id| id.0);
        requests.lock().unwrap().push(Seen {
            post,
            digest,
            connection,
        });
        // The advertisement names its service in the query, the exchange in the path.
        let receive = request.uri().to_string().contains("receive-pack");
        let service = match (receive, post) {
            (false, false) => protocol::GitService::UploadPackAdvertisement,
            (false, true) => protocol::GitService::UploadPackExchange,
            (true, false) => protocol::GitService::ReceivePackAdvertisement,
            (true, true) => protocol::GitService::ReceivePackExchange,
        };
        let (status, bytes) = match (digest.is_some(), post) {
            (false, _) => (401, &b""[..]),
            (true, false) => (200, ADVERTISEMENT),
            (true, true) => (200, &b"0000"[..]),
        };
        Box::pin(async move { response(status, service, bytes) })
    })
}

/// A credential helper that answers `GH_TOKEN`: the fixture for an operation whose account token is `token`.
fn token_helper(directory: &std::path::Path, token: &str) -> https_auth::Config {
    let mut config = helper_script::git_fixture(
        directory,
        "printf 'username=fixture\\npassword=%s\\n\\n' \"$GH_TOKEN\"",
    );
    config.environment.push(("GH_TOKEN".into(), token.into()));
    config
}
/// `helper` with another account token: an operation whose `GH_TOKEN` changed.
fn retoken(helper: &https_auth::Config, token: &str) -> https_auth::Config {
    let mut changed = helper.clone();
    changed.environment.retain(|(name, _)| name != "GH_TOKEN");
    changed.environment.push(("GH_TOKEN".into(), token.into()));
    changed
}
/// The real Git, configured by a global file (`GIT_CONFIG_GLOBAL`) that names a script helper answering `token`: the
/// fixture for an operation whose Git configuration is that file. Another file naming another helper is another
/// operation's credential.
fn config_helper(directory: &std::path::Path, name: &str, token: &str) -> https_auth::Config {
    let helper = directory.join(format!("helper-{name}"));
    helper_script::write_helper_script(
        &helper,
        &format!("printf 'username=fixture\\npassword={token}\\n'\n"),
    );
    let global = directory.join(format!("gitconfig-{name}"));
    std::fs::write(
        &global,
        format!(
            "[credential]\n helper = {}\n",
            helper_script::script_path(&helper)
        ),
    )
    .unwrap();
    let mut environment: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
        ("HOME".into(), directory.as_os_str().into()),
        ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    // A Windows process is started with no `SystemRoot` unless it is given one.
    if let Some(system) = std::env::var_os("SystemRoot") {
        environment.push(("SystemRoot".into(), system));
    }
    https_auth::Config {
        executable: helper_script::real_git(),
        environment,
    }
}

/// Runs one advertisement as `name` over `helper`'s context and releases the connection to idle.
fn advertise(
    case: &Case,
    requests: &Requests,
    name: &str,
    helper: &https_auth::Config,
) -> (usize, Vec<Seen>) {
    let operation = operation_of(&case.handle, name, context_of(Some(helper.clone())));
    let before = requests.lock().unwrap().len();
    let mut exchange = case.exchange(&operation, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(operation.finish());
    case.wait("the connection is idle", |counts| counts.leased == 0);
    let seen = requests.lock().unwrap()[before..].to_vec();
    (case.accepted(), seen)
}

/// Two operations on one host, whose helpers differ as `a` and `b` do: each is answered with its own credential,
/// and the connection A's credential authenticated serves B nothing.
fn changed_helper_gives_each_operation_its_own_credential(
    a: &https_auth::Config,
    b: &https_auth::Config,
    tokens: [&str; 2],
) {
    let requests = Requests::default();
    // The instance's helper (while it has one) is the first operation's, as the runtime's capture makes it.
    let first = a.clone();
    let case = Case::with(
        |_| authorizing(&requests),
        Behaviour::default(),
        move |_| (Some(first), None),
        None,
    );
    let (accepted, first) = advertise(&case, &requests, "op-a", a);
    let (accepted_after, second) = advertise(&case, &requests, "op-b", b);
    assert_eq!(accepted, 1);
    assert_eq!(
        accepted_after, 2,
        "B was given A's authenticated connection"
    );
    let credential = |seen: &[Seen]| seen.iter().filter_map(|s| s.digest).collect::<Vec<_>>();
    assert_eq!(credential(&first).len(), 1);
    assert!(
        credential(&first)[0] == basic_digest(tokens[0]),
        "A's request did not carry A's credential"
    );
    assert_eq!(credential(&second).len(), 1);
    assert!(
        credential(&second)[0] == basic_digest(tokens[1]),
        "B's request did not carry B's own credential"
    );
    let connection = |seen: &[Seen]| seen.last().unwrap().connection;
    assert_ne!(connection(&first), connection(&second));
    assert!(case.close());
}

#[test]
fn a_changed_gh_token_gives_the_second_operation_its_own_credential_on_a_connection_of_its_own() {
    let directory = tempfile::tempdir().unwrap();
    let a = token_helper(directory.path(), "fixture-token-a");
    let b = retoken(&a, "fixture-token-b");
    changed_helper_gives_each_operation_its_own_credential(
        &a,
        &b,
        ["fixture-token-a", "fixture-token-b"],
    );
}

#[test]
fn a_changed_git_config_global_helper_gives_the_second_operation_its_own_credential_on_a_connection_of_its_own()
 {
    let directory = tempfile::tempdir().unwrap();
    let a = config_helper(directory.path(), "config-a", "fixture-token-a");
    let b = config_helper(directory.path(), "config-b", "fixture-token-b");
    changed_helper_gives_each_operation_its_own_credential(
        &a,
        &b,
        ["fixture-token-a", "fixture-token-b"],
    );
}

#[test]
fn an_operation_whose_context_holds_no_helper_is_never_answered_with_another_operations() {
    // The instance holds no helper: B's context is of another provider, so its challenge is refused and the credential
    // A's helper gave is not sent for it, whichever operation reached the host first.
    let requests = Requests::default();
    let case = Case::with(
        |_| authorizing(&requests),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let directory = tempfile::tempdir().unwrap();
    let helper = token_helper(directory.path(), "fixture-token-a");
    advertise(&case, &requests, "op-a", &helper);
    let b = operation_of(&case.handle, "op-b", context_of(None));
    let mut exchange = case.exchange(&b, true);
    assert!(matches!(
        body(&mut exchange),
        Err(StreamError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(b.finish());
    let seen = requests.lock().unwrap().clone();
    let authorized = seen.iter().filter(|s| s.digest.is_some()).count();
    assert_eq!(authorized, 1, "only A's own request carried a credential");
    assert!(case.close());
}

#[test]
fn a_refusing_helper_ends_the_exchange_denied_and_keeps_no_connection() {
    let requests = Requests::default();
    let case = Case::with(
        |_| authorizing(&requests),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let directory = tempfile::tempdir().unwrap();
    let refusing = helper_script::git_fixture(directory.path(), "cat >/dev/null\nexit 1\n");
    let a = operation_of(&case.handle, "op-a", context_of(Some(refusing)));
    let mut exchange = case.exchange(&a, true);
    match body(&mut exchange) {
        Err(StreamError::Io(error)) => {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied)
        }
        other => panic!("expected the typed denial, got {}", other.is_ok()),
    }
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(a.finish());
    case.wait("no connection is kept", |counts| counts.total() == 0);
    let seen = requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "only the unauthenticated request went out");
    assert!(
        seen.iter().all(|s| s.digest.is_none()),
        "a credential was sent"
    );
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(case.close());
}

#[test]
fn an_authenticated_idle_connection_is_reused_only_by_an_equal_context_of_the_same_binding() {
    let requests = Requests::default();
    let case = Case::with(
        |_| authorizing(&requests),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let directory = tempfile::tempdir().unwrap();
    let token_a = token_helper(directory.path(), "fixture-token-a");
    let token_b = retoken(&token_a, "fixture-token-b");
    let authorized = |seen: &[Seen]| {
        seen.iter()
            .filter_map(|s| s.digest.map(|digest| (digest, s.connection)))
            .collect::<Vec<_>>()
    };
    // A authenticates a connection, and A's next exchange (the same binding, so an equal context) reuses it: the
    // second request is challenged again and answered by a fresh lookup of A's own helper.
    let a = operation_of(&case.handle, "op-a", context_of(Some(token_a.clone())));
    let mut served = Vec::new();
    for _ in 0..2 {
        let mut exchange = case.exchange(&a, true);
        assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
        case.wait("the connection is idle", |counts| counts.leased == 0);
        served = authorized(&requests.lock().unwrap());
    }
    assert_eq!(
        case.accepted(),
        1,
        "A's second exchange reused its connection"
    );
    assert_eq!(served.len(), 2);
    assert!(
        served
            .iter()
            .all(|(d, c)| *d == basic_digest("fixture-token-a") && *c == served[0].1)
    );
    // Another binding with an equal context is given a connection of its own: the authenticated one serves only the
    // binding that last used it (contract 7.2).
    let (accepted, equal) = advertise(&case, &requests, "op-equal", &token_a);
    assert_eq!(accepted, 2);
    assert_ne!(authorized(&equal)[0].1, served[0].1);
    // And a context that differs is given neither.
    let (accepted, other) = advertise(&case, &requests, "op-other", &token_b);
    assert_eq!(accepted, 3);
    assert!(authorized(&other)[0].0 == basic_digest("fixture-token-b"));
    assert!(
        authorized(&other)[0].1 != served[0].1
            && authorized(&other)[0].1 != authorized(&equal)[0].1
    );
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn a_push_is_authenticated_on_the_connection_its_advertisement_was() {
    // Contract 7.2: the adapter replays nothing and refuses a challenge to a POST, whose body is already on the wire.
    // A POST on a connection a helper token authenticated carries the operation's own credential from its first byte.
    let requests = Requests::default();
    let case = Case::with(
        |_| authorizing(&requests),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let directory = tempfile::tempdir().unwrap();
    let helper = token_helper(directory.path(), "fixture-token-a");
    let operation = operation_of(&case.handle, "op-push", context_of(Some(helper)));
    let mut advertisement = case.exchange_of(&operation, GitService::ReceivePackAdvertisement);
    assert_eq!(body(&mut advertisement).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(advertisement.finish()).disposition,
        Disposition::Reusable
    );
    case.wait("the connection is idle", |counts| counts.leased == 0);
    let mut push = case.exchange_of(&operation, GitService::ReceivePackExchange);
    let mut sent = 0;
    while sent < 4 {
        match push.stream().write(&b"0000"[sent..]) {
            Ok(n) => sent += n,
            Err(StreamError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::yield_now();
            }
            Err(_) => panic!("the request body was refused"),
        }
    }
    assert_eq!(body(&mut push).unwrap(), b"0000");
    assert_eq!(block_on(push.finish()).disposition, Disposition::Reusable);
    block_on(operation.finish());
    let seen = requests.lock().unwrap().clone();
    let post: Vec<_> = seen.iter().filter(|s| s.post).collect();
    assert_eq!(post.len(), 1, "the POST was sent once, never replayed");
    assert!(
        post[0].digest == Some(basic_digest("fixture-token-a")),
        "the POST did not carry the operation's own credential"
    );
    assert_eq!(post[0].connection, seen[0].connection);
    assert_eq!(case.accepted(), 1);
    assert!(case.close());
}
