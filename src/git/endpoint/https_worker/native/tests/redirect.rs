//! A redirect after a native exchange has started (TR1.8 section 7 and section 14 item 13, owed to step 4.9).
//!
//! One test covers both refusal points, because they are one rule: no credential follows a redirect that arrives
//! once the connection has a native exchange.
//! - `native/exchange.rs`: a 3xx during the advertisement's SSPI exchange (`status != 401 || history.complete`);
//! - `serve.rs`: a 3xx answering a natively authenticated POST (`check_response`, `UnsupportedOperation`).
//!
//! In both, the operation ends, the redirect is not followed, nothing is asked of the new location, and the
//! credential is never replayed to the server that redirected. The SSPI side is the fake port, so the rows run on
//! every platform.
use super::*;
use crate::git::endpoint::https_fixture::{Server, attach, input, response};
use hyper::header::LOCATION;

/// What the new location saw: every request, and those that carried an `Authorization` header.
struct Target {
    server: Server,
    requests: Arc<AtomicUsize>,
    credentialled: Arc<AtomicUsize>,
}

async fn target() -> Target {
    let requests = Arc::new(AtomicUsize::new(0));
    let credentialled = Arc::new(AtomicUsize::new(0));
    let (seen, authorized) = (requests.clone(), credentialled.clone());
    let server = Server::start(Arc::new(move |request| {
        seen.fetch_add(1, Ordering::Relaxed);
        if request.headers().contains_key(AUTHORIZATION) {
            authorized.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {
            response(
                200,
                GitService::UploadPackAdvertisement,
                Bytes::from_static(b"the new location"),
            )
        })
    }))
    .await;
    Target {
        server,
        requests,
        credentialled,
    }
}

/// The server that redirects. It challenges a request with no `Authorization` header (`NTLM`), and answers one that has
/// it with `after_token`: a 200 advertisement (the exchange completes) or a 302. A POST is answered with a 302.
/// `credentialled` counts the requests that carried a credential and `posts` the POSTs.
struct Origin {
    server: Server,
    credentialled: Arc<AtomicUsize>,
    posts: Arc<AtomicUsize>,
}

async fn origin(location: String, redirect_the_token: bool) -> Origin {
    let credentialled = Arc::new(AtomicUsize::new(0));
    let posts = Arc::new(AtomicUsize::new(0));
    let (authorized, posted) = (credentialled.clone(), posts.clone());
    let server = Server::start(Arc::new(move |request| {
        let post = request.method() == "POST";
        let has_credential = request.headers().contains_key(AUTHORIZATION);
        if has_credential {
            authorized.fetch_add(1, Ordering::Relaxed);
        }
        if post {
            posted.fetch_add(1, Ordering::Relaxed);
        }
        let location = location.clone();
        Box::pin(async move {
            let service = if post {
                GitService::UploadPackExchange
            } else {
                GitService::UploadPackAdvertisement
            };
            if post || (has_credential && redirect_the_token) {
                let mut moved = response(302, service, Bytes::new());
                moved
                    .headers_mut()
                    .insert(LOCATION, location.parse().unwrap());
                return moved;
            }
            let mut result = response(
                if has_credential { 200 } else { 401 },
                service,
                Bytes::new(),
            );
            if !has_credential {
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
            }
            result
        })
    }))
    .await;
    Origin {
        server,
        credentialled,
        posts,
    }
}

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

/// The new location was never asked for anything, so no credential reached it.
fn assert_untouched(target: &Target, what: &str) {
    assert_eq!(
        target.requests.load(Ordering::Acquire),
        0,
        "{what}: the new location saw a request"
    );
    assert_eq!(
        target.credentialled.load(Ordering::Acquire),
        0,
        "{what}: a credential reached the new location"
    );
    assert_eq!(
        target.server.connections.load(Ordering::Acquire),
        0,
        "{what}: a connection was made to the new location"
    );
}

#[test]
fn a_redirect_after_a_native_exchange_has_started_ends_the_operation_and_sends_no_credential_on() {
    run(async {
        // Point 1: the server answers the client's token with a redirect (exchange.rs).
        {
            let target = target().await;
            let origin = origin(format!("{}/info/refs", target.server.url), true).await;
            let mut endpoint =
                Endpoint::new(origin.server.config(), None, pool::Config::default()).unwrap();
            let (caller, port) = fake(true, 2);
            endpoint.client.set_native(caller);
            let mut request = input(&origin.server, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::WindowsDefault;
            let (result, _) = endpoint
                .client
                .prepare_attempt(
                    request,
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            let failure = result
                .as_ref()
                .err()
                .expect("the redirect ends the exchange");
            assert_eq!(failure.code, ErrorCode::Authentication);
            assert_eq!(failure.facts.as_ref().unwrap().http_status, Some(302));
            assert_eq!(port.starts.load(Ordering::Acquire), 1);
            assert_eq!(
                origin.credentialled.load(Ordering::Acquire),
                1,
                "the token is sent once, to the server that asked for it"
            );
            assert_untouched(&target, "a redirect during the exchange");
            drop(result);
            endpoint.client.finish_operation("operation");
            let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
        // Point 2: the exchange completes, then the POST on the natively authenticated route is redirected (serve.rs).
        {
            let target = target().await;
            let origin = origin(format!("{}/git-upload-pack", target.server.url), false).await;
            let mut endpoint =
                Endpoint::new(origin.server.config(), None, pool::Config::default()).unwrap();
            let (caller, _) = fake(true, 2);
            endpoint.client.set_native(caller);
            let mut request = input(&origin.server, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::WindowsDefault;
            let (advertised, _) = endpoint
                .client
                .prepare_attempt(
                    request.clone(),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            let advertised = advertised.expect("the exchange completes");
            assert!(advertised.native_route.is_some());
            let (stream, task) = attach(advertised);
            stream.end_write().await.unwrap();
            let mut bytes = [0; 32];
            while stream.read(&mut bytes).await.unwrap() != 0 {}
            stream.close().await.unwrap();
            task.await.unwrap();
            let before = origin.credentialled.load(Ordering::Acquire);
            request.service = GitService::UploadPackExchange;
            let (posting, _) = endpoint
                .client
                .prepare_attempt(
                    request,
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            // The POST goes out when the exchange is served; the redirect that answers it ends the exchange.
            let (stream, task) = attach(posting.expect("the native route admits the POST"));
            stream.end_write().await.unwrap();
            let result = stream.read(&mut bytes).await;
            assert!(
                matches!(
                    result,
                    Err(gwz_transport::stream::Error::PeerFailed {
                        code: ErrorCode::UnsupportedOperation,
                        ..
                    })
                ),
                "the redirect ends the POST: {result:?}"
            );
            let _ = stream.close().await;
            task.await.unwrap();
            assert_eq!(
                origin.posts.load(Ordering::Acquire),
                1,
                "the POST is sent once and not replayed"
            );
            // NTLM and Negotiate authenticate the connection, not the request: the POST carries no header, and the
            // only connection that is authenticated is the one to the server that redirected.
            assert_eq!(
                origin.credentialled.load(Ordering::Acquire),
                before,
                "the POST carried a credential header"
            );
            assert_eq!(origin.server.connections.load(Ordering::Acquire), 1);
            assert_untouched(&target, "a redirect of a natively authenticated POST");
            endpoint.client.finish_operation("operation");
            let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
    });
}
