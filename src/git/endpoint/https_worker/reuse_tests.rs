//! Step 1.6's HTTPS rows (plan §4; contract §7.2): a connection a helper's token or the logon session authenticated is
//! scoped to that authentication, and is never leased to another operation binding. Two bindings here are two inputs
//! of one endpoint with different sessions; they carry the same operation name, which is the only way two bindings
//! reach one route today (the finding this step reports: routes are keyed by operation name, not by binding).
use super::*;
use crate::git::endpoint::https_fixture::{self as fixture, Server, input, response};
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};

/// What the server saw of one request: whether it carried a credential, and on which connection.
type Seen = Arc<Mutex<Vec<(Option<String>, usize)>>>;

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
            let authorization = request
                .headers()
                .get(AUTHORIZATION)
                .map(|value| value.to_str().unwrap().to_owned());
            let connection = request
                .extensions()
                .get::<fixture::ConnectionId>()
                .unwrap()
                .0;
            let post = request.method() == hyper::Method::POST;
            records
                .lock()
                .unwrap()
                .push((authorization.clone(), connection));
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

/// Red row of step 1.6: binding A's helper token authenticated a connection; the token then expired (the
/// helper now answers another), and binding B's request must not go on A's authenticated connection.
#[tokio::test]
async fn p1_6_an_expired_helper_tokens_connection_is_not_reused_by_another_binding() {
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
    let (a_auth, a_connection) = records.first().cloned().unwrap();
    let (b_auth, b_connection) = records.last().cloned().unwrap();
    assert!(a_auth.is_some() && b_auth.is_some());
    assert_ne!(
        a_connection, b_connection,
        "binding B leased binding A's token-authenticated connection"
    );
    // Recorded for the step's finding (routes keyed by operation name share a route's answer): whether B's
    // request carried A's token or its own.
    eprintln!(
        "p1-6 finding probe: binding B's request carried {} token",
        if a_auth == b_auth {
            "binding A's"
        } else {
            "its own"
        }
    );
    endpoint.client.finish_operation("operation");
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
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
