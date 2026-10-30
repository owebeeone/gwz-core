//! Step 1.6's HTTPS rows (plan §4; contract §7.2): a connection a helper's token or the logon session authenticated is
//! scoped to that authentication, and is never leased to another operation binding. Two bindings here are two inputs
//! of one endpoint with different sessions; they carry the same operation name, which is the only way two bindings
//! reach one route of this worker (see the first row).
use super::*;
use crate::git::endpoint::https_fixture::{self as fixture, Server, input, response};
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use sha2::Digest;

/// What the server saw of one request: the SHA-256 of its Authorization header (none when it carried no credential, so
/// a failure message shows no header), and the connection it came on.
type Seen = Arc<Mutex<Vec<(Option<[u8; 32]>, usize)>>>;

async fn finish(prepared: Prepared) {
    let (stream, task) = fixture::attach(prepared);
    stream.end_write().await.unwrap();
    let mut bytes = [0; 64];
    while stream.read(&mut bytes).await.unwrap() != 0 {}
    stream.close().await.unwrap();
    task.await.unwrap();
}

fn recording(seen: &Seen, challenge: Option<&'static str>) -> fixture::Handler {
    let records = seen.clone();
    Arc::new(move |request| {
        let records = records.clone();
        Box::pin(async move {
            let authorization: Option<[u8; 32]> = request
                .headers()
                .get(AUTHORIZATION)
                .map(|value| sha2::Sha256::digest(value.as_bytes()).into());
            let connection = request
                .extensions()
                .get::<fixture::ConnectionId>()
                .unwrap()
                .0;
            let post = request.method() == hyper::Method::POST;
            records.lock().unwrap().push((authorization, connection));
            let service = if post {
                GitService::UploadPackExchange
            } else {
                GitService::UploadPackAdvertisement
            };
            let mut result = response(
                if authorization.is_some() || post {
                    200
                } else {
                    401
                },
                service,
                "fixture",
            );
            if let (None, Some(scheme), false) = (&authorization, challenge, post) {
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, scheme.parse().unwrap());
            }
            result
        })
    })
}

/// A Git credential fixture whose `fill` answers with the password in `token`, read at each lookup.
fn token_git(directory: &std::path::Path, token: &std::path::Path) -> https_auth::Config {
    let mut config = crate::git::endpoint::helper_script::git_fixture(
        directory,
        "printf 'username=fixture\\npassword=%s\\n\\n' \"$(cat \"$TOKEN\")\"",
    );
    config.environment.push((
        "TOKEN".into(),
        crate::git::endpoint::helper_script::script_path(token).into(),
    ));
    config
}

/// Binding A's helper token authenticates a connection; the token then expires (the helper now answers another) and
/// binding B, of the same operation name, sends its request. What the server saw of each: the digest of its Authorization header and
/// its connection.
async fn helper_token_bindings() -> ((Option<[u8; 32]>, usize), (Option<[u8; 32]>, usize)) {
    let seen: Seen = Arc::default();
    let server = Server::start(recording(&seen, None)).await;
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "token-of-binding-a").unwrap();
    let mut endpoint = Endpoint::new(
        server.config(),
        Some(token_git(dir.path(), &token)),
        pool::Config::default(),
    )
    .unwrap();
    let mut a = input(&server, GitService::UploadPackAdvertisement);
    a.policy = AuthPolicy::Gh;
    a.session = "binding-a".into();
    let prepared = endpoint
        .client
        .prepare_budget_for_transition(
            a.clone(),
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await
        .unwrap();
    finish(prepared).await;
    std::fs::write(&token, "token-of-binding-b").unwrap();
    let mut b = a.clone();
    b.session = "binding-b".into();
    let prepared = endpoint
        .client
        .prepare_budget_for_transition(
            b,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await
        .unwrap();
    finish(prepared).await;
    let records = seen.lock().unwrap().clone();
    endpoint.client.finish_operation("operation");
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    (
        records.first().cloned().unwrap(),
        records.last().cloned().unwrap(),
    )
}

/// Step 1.6's connection leg: binding B's request must not go on binding A's token-authenticated connection.
///
/// The route leg is a characterization of this worker, not a defect of the connection host. The worker's routes (and the
/// helper answers they cache) are per operation, because a transport request is one operation with one binding and the
/// runtime's one helper (`Client::auth`); the `session` of an input names one open, not a binding. Two bindings under
/// one operation name exist only in this rig. A binding sending its own operation's helper answer is the connection
/// host's rule, whose adapter takes each exchange's helper from that operation's `AuthContext`: the rows are
/// `connection_host::https_auth_tests::a_changed_gh_token_...` and `a_changed_git_config_global_helper_...`
/// (step 1.3c/C2(b)).
#[tokio::test]
async fn p1_6_an_expired_helper_tokens_connection_is_not_reused_by_another_binding() {
    let ((a_auth, a_connection), (b_auth, b_connection)) = helper_token_bindings().await;
    assert!(a_auth.is_some() && b_auth.is_some());
    assert_ne!(
        a_connection, b_connection,
        "binding B leased binding A's token-authenticated connection"
    );
    assert!(
        a_auth == b_auth,
        "this worker's routes stopped being per operation: the connection host's rows now hold the per-binding rule, \
         so retire this characterization with the worker's route"
    );
}

/// Red row of step 1.6 for the logon session (amendment 2 §3.18): binding A authenticated a connection with the logon
/// session (the native route, through a fake SSPI port that completes); binding B's service request must not be
/// leased A's authenticated connection. It is refused instead (B's binding has no authentication of its own on that
/// route), which is the fail-closed answer until routes are per binding.
#[tokio::test]
async fn p1_6_a_logon_session_connection_is_not_reused_by_another_binding() {
    let seen: Seen = Arc::default();
    let server = Server::start(recording(&seen, Some("NTLM"))).await;
    let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
    endpoint
        .client
        .set_native(super::native::publication_fixture());
    let mut a = input(&server, GitService::UploadPackAdvertisement);
    a.policy = AuthPolicy::WindowsDefault;
    a.session = "binding-a".into();
    let (prepared, _) = endpoint
        .client
        .prepare_attempt(
            a.clone(),
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    let prepared = prepared.unwrap();
    assert_eq!(prepared.opened.facts.authenticated, Some(true));
    finish(prepared).await;
    let authenticated_on = seen.lock().unwrap().last().cloned().unwrap().1;
    let before = seen.lock().unwrap().len();
    let mut b = input(&server, GitService::UploadPackExchange);
    b.policy = AuthPolicy::WindowsDefault;
    b.session = "binding-b".into();
    let (prepared, _) = endpoint
        .client
        .prepare_attempt(
            b,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    if let Ok(prepared) = prepared {
        finish(prepared).await;
    }
    let records = seen.lock().unwrap().clone();
    assert!(
        records[before..]
            .iter()
            .all(|(_, connection)| *connection != authenticated_on),
        "binding B's request went on binding A's logon-session connection: {records:?}"
    );
    endpoint.client.finish_operation("operation");
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
}
