//! The WH3 matrix's adversity rows at the endpoint (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` section 6, step 4.9),
//! on the route the product has: the worker, the pool, the routes and the native exchange, with the SSPI side the
//! fake port so that every platform runs them. The real SSPI rows are `real_sspi.rs` (EPA matched and mismatched, B16).
//!
//! Where each WH3 case is covered, beside the rows below:
//! - original deadline D across reuse, 401 and native rounds: `the_original_deadline_holds_across_a_reused_connection_a_401_and_every_native_round`,
//!   with `exchange.rs` (`production_pool_wait_cannot_extend_fixed_deadline`, `production_round_cap_and_http_allowance_do_not_reset_native_deadline`)
//!   and `rejection.rs` (the eight rounds are the operation's);
//! - cancel during HTTP and native IPC: `a_cancel_while_the_server_stalls_a_native_round_returns_promptly_and_leaves_nothing_behind`
//!   (HTTP), `retention.rs` (native IPC);
//! - cleanup retained before replacement admission: `retention.rs`, and `transport_host::https_cancel_mux_tests`;
//! - pool reuse within an opaque scope, no cross-operation credential reuse: `a_new_operation_authenticates_again_and_never_borrows_the_last_ones_connection`,
//!   `exchange.rs` (`production_native_exchange_keeps_facts_and_refuses_replacement_before_post`);
//! - receive-pack POST failure never retried: `a_receive_pack_post_failure_after_native_authentication_is_not_retried_and_starts_no_second_exchange`;
//! - untrusted or mismatched TLS sends no token: `an_untrusted_certificate_sends_no_token_and_asks_the_server_nothing`;
//! - a discovery redirect between names (B15): `a_discovery_redirect_between_names_re_authenticates_for_the_new_name_and_forwards_nothing`;
//! - the challenged POST (B18): `precedence.rs`; the redirect-after-exchange refusals: `redirect.rs`.
use super::*;
use crate::git::endpoint::https_fixture::{Server, attach, input, response};

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

/// Reads an attached exchange to its end and closes it.
async fn drain(prepared: Prepared) {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut bytes = [0; 64];
    while stream.read(&mut bytes).await.unwrap() != 0 {}
    stream.close().await.unwrap();
    task.await.unwrap();
}

async fn finish(mut endpoint: Endpoint, operations: &[&str]) {
    for operation in operations {
        endpoint.client.finish_operation(operation);
    }
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
}

fn windows(
    server: &Server,
    service: GitService,
    operation: &str,
) -> crate::git::endpoint::https_worker::Input {
    let mut request = input(server, service);
    request.policy = AuthPolicy::WindowsDefault;
    request.operation = operation.into();
    request
}

#[test]
fn a_new_operation_authenticates_again_and_never_borrows_the_last_ones_connection() {
    run(async {
        // A connection that NTLM or Negotiate authenticated belongs to the operation that authenticated it: another
        // operation to the same host gets its own connection and its own exchange, and carries nothing of the first.
        let headers = Arc::new(Mutex::new(Vec::<bool>::new()));
        let seen = headers.clone();
        let server = Server::start(Arc::new(move |request| {
            let has_credential = request.headers().contains_key(AUTHORIZATION);
            seen.lock().unwrap().push(has_credential);
            Box::pin(async move {
                let mut result = response(
                    if has_credential { 200 } else { 401 },
                    GitService::UploadPackAdvertisement,
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
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(true, 2);
        endpoint.client.set_native(caller);
        for operation in ["first", "second"] {
            let (prepared, _) = endpoint
                .client
                .prepare_attempt(
                    windows(&server, GitService::UploadPackAdvertisement, operation),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            let prepared = prepared.expect(operation);
            assert_eq!(
                prepared.opened.facts.authenticated,
                Some(true),
                "{operation}"
            );
            drain(prepared).await;
            endpoint.client.finish_operation(operation);
        }
        assert_eq!(
            port.starts.load(Ordering::Acquire),
            2,
            "each operation had its own exchange"
        );
        assert_eq!(
            *headers.lock().unwrap(),
            [false, true, false, true],
            "the second operation began unauthenticated and was challenged"
        );
        assert_eq!(server.connections.load(Ordering::Acquire), 2);
        finish(endpoint, &[]).await;
    });
}

#[test]
fn an_untrusted_certificate_sends_no_token_and_asks_the_server_nothing() {
    run(async {
        let asked = Arc::new(AtomicUsize::new(0));
        let seen = asked.clone();
        let server = Server::start(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {
                let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
                result
            })
        }))
        .await;
        // The endpoint trusts the platform's roots, which do not include the fixture's CA.
        let mut endpoint = Endpoint::new(
            crate::git::endpoint::https_connection::Config::default(),
            None,
            pool::Config::default(),
        )
        .unwrap();
        let (caller, port) = fake(true, 2);
        endpoint.client.set_native(caller);
        let (result, _) = endpoint
            .client
            .prepare_attempt(
                windows(&server, GitService::UploadPackAdvertisement, "operation"),
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        assert_eq!(result.err().expect("untrusted").code, ErrorCode::Trust);
        assert_eq!(
            asked.load(Ordering::Acquire),
            0,
            "the server was asked for something"
        );
        assert_eq!(
            port.starts.load(Ordering::Acquire),
            0,
            "an SSPI attempt began"
        );
        finish(endpoint, &["operation"]).await;
    });
}

#[test]
fn the_original_deadline_holds_across_a_reused_connection_a_401_and_every_native_round() {
    run(async {
        // The deadline D is anchored once, when the open is admitted. A connection reused from the pool, the 401 and
        // each round of an exchange that never ends all run inside it: the open ends at D, not after a deadline per step.
        let server = Server::start(Arc::new(|request| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                if request.uri().path().starts_with("/ok") {
                    return response(200, GitService::UploadPackAdvertisement, Bytes::new());
                }
                let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                result.headers_mut().insert(
                    WWW_AUTHENTICATE,
                    if request.headers().contains_key(AUTHORIZATION) {
                        "NTLM YQ=="
                    } else {
                        "NTLM"
                    }
                    .parse()
                    .unwrap(),
                );
                result
            })
        }))
        .await;
        let config = pool::Config {
            connect_timeout_ms: 300,
            ..Default::default()
        };
        let mut endpoint = Endpoint::new(server.config(), None, config).unwrap();
        let (caller, port) = fake(false, 2);
        endpoint.client.set_native(caller);
        // An idle connection to the host, which the next open reuses.
        let mut warm = windows(&server, GitService::UploadPackAdvertisement, "warm");
        warm.destination = server.url.replace("/repo", "/ok");
        let (warmed, _) = endpoint
            .client
            .prepare_attempt(
                warm,
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        drain(warmed.expect("an anonymous open")).await;
        let mut budget = endpoint.client.budget();
        let started = Instant::now();
        let (result, _) = endpoint
            .client
            .prepare_attempt(
                windows(&server, GitService::UploadPackAdvertisement, "operation"),
                &CancellationToken::new(),
                &mut budget,
                &mut None,
            )
            .await;
        let elapsed = started.elapsed();
        assert_eq!(result.err().expect("D passes").code, ErrorCode::Timeout);
        assert_eq!(
            server.connections.load(Ordering::Acquire),
            1,
            "the open reused the idle connection"
        );
        assert!(
            elapsed >= Duration::from_millis(250),
            "ended early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "ended late: {elapsed:?}"
        );
        assert_eq!(
            port.deadlines.lock().unwrap().as_slice(),
            &[budget.logical_deadline.unwrap()],
            "the exchange ran under the open's one deadline"
        );
        assert!(
            port.steps.load(Ordering::Acquire) < 8,
            "the rounds ran out the deadline, not their cap"
        );
        finish(endpoint, &["warm", "operation"]).await;
    });
}

#[test]
fn a_cancel_while_the_server_stalls_a_native_round_returns_promptly_and_leaves_nothing_behind() {
    run(async {
        let server = Server::start(Arc::new(|request| {
            Box::pin(async move {
                if request.headers().contains_key(AUTHORIZATION) {
                    // The answer to the client's token never comes.
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
                result
            })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(false, 2);
        endpoint.client.set_native(caller);
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let client = endpoint.client.clone();
        let request = windows(&server, GitService::UploadPackAdvertisement, "operation");
        let task = tokio::spawn(async move {
            let mut budget = client.budget();
            client
                .prepare_attempt(request, &cancel, &mut budget, &mut None)
                .await
                .0
        });
        let until = Instant::now() + Duration::from_secs(10);
        while port.steps.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < until, "the exchange never began");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Give the token time to reach the server and the stall to begin.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let cancelled = Instant::now();
        signal.cancel();
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("a cancel returns promptly")
            .unwrap();
        assert_eq!(result.err().expect("cancelled").code, ErrorCode::Cancelled);
        assert!(cancelled.elapsed() < Duration::from_secs(3));
        finish(endpoint, &["operation"]).await;
    });
}

#[test]
fn a_receive_pack_post_failure_after_native_authentication_is_not_retried_and_starts_no_second_exchange()
 {
    run(async {
        let posts = Arc::new(AtomicUsize::new(0));
        let counted = posts.clone();
        let server = Server::start(Arc::new(move |request| {
            let post = request.method() == "POST";
            if post {
                counted.fetch_add(1, Ordering::Relaxed);
            }
            let has_credential = request.headers().contains_key(AUTHORIZATION);
            Box::pin(async move {
                if post {
                    // The server fails the push; it is never asked again.
                    return response(500, GitService::ReceivePackExchange, Bytes::new());
                }
                let mut result = response(
                    if has_credential { 200 } else { 401 },
                    GitService::ReceivePackAdvertisement,
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
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(true, 2);
        endpoint.client.set_native(caller);
        let (advertised, _) = endpoint
            .client
            .prepare_attempt(
                windows(&server, GitService::ReceivePackAdvertisement, "operation"),
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        drain(advertised.expect("authenticated")).await;
        let (posting, _) = endpoint
            .client
            .prepare_attempt(
                windows(&server, GitService::ReceivePackExchange, "operation"),
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        let (stream, task) = attach(posting.expect("the native route admits the POST"));
        stream.write_all(b"0000").await.ok();
        let _ = stream.end_write().await;
        let mut bytes = [0; 8];
        let result = stream.read(&mut bytes).await;
        assert!(
            matches!(
                result,
                Err(gwz_transport::stream::Error::PeerFailed {
                    code: ErrorCode::Io,
                    ..
                })
            ),
            "the push fails: {result:?}"
        );
        let _ = stream.close().await;
        task.await.unwrap();
        assert_eq!(posts.load(Ordering::Acquire), 1, "the POST was replayed");
        assert_eq!(
            port.starts.load(Ordering::Acquire),
            1,
            "a second exchange began"
        );
        finish(endpoint, &["operation"]).await;
    });
}

#[test]
fn a_discovery_redirect_between_names_re_authenticates_for_the_new_name_and_forwards_nothing() {
    run(async {
        // B15: `localhost` and `127.0.0.1` reach one server, but a name is an origin. The redirect is followed (it came
        // before any credential existed), the new name is challenged afresh on its own connection, and the SSPI target
        // names the new host; nothing the first name saw reaches the second.
        let by_host = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
        let seen = by_host.clone();
        let location = Arc::new(Mutex::new(String::new()));
        let moved_to = location.clone();
        let server = Server::start(Arc::new(move |request| {
            let host = request
                .headers()
                .get(hyper::header::HOST)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let has_credential = request.headers().contains_key(AUTHORIZATION);
            seen.lock().unwrap().push((host.clone(), has_credential));
            let target = moved_to.lock().unwrap().clone();
            Box::pin(async move {
                if host.starts_with("localhost") {
                    let mut moved =
                        response(302, GitService::UploadPackAdvertisement, Bytes::new());
                    moved
                        .headers_mut()
                        .insert(hyper::header::LOCATION, target.parse().unwrap());
                    return moved;
                }
                let mut result = response(
                    if has_credential { 200 } else { 401 },
                    GitService::UploadPackAdvertisement,
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
        let port_number = server
            .url
            .rsplit(':')
            .next()
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        *location.lock().unwrap() =
            format!("https://127.0.0.1:{port_number}/repo/info/refs?service=git-upload-pack");
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake_for("127.0.0.1", true, 2);
        endpoint.client.set_native(caller);
        let (result, _) = endpoint
            .client
            .prepare_attempt(
                windows(&server, GitService::UploadPackAdvertisement, "operation"),
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        let prepared = result.expect("the new name authenticates");
        assert_eq!(prepared.opened.facts.authenticated, Some(true));
        drain(prepared).await;
        assert_eq!(port.starts.load(Ordering::Acquire), 1);
        let hosts = by_host.lock().unwrap().clone();
        assert!(
            hosts
                .iter()
                .filter(|(host, _)| host.starts_with("localhost"))
                .all(|(_, credential)| !credential),
            "the first name was sent a credential: {hosts:?}"
        );
        assert_eq!(
            hosts
                .iter()
                .filter(|(host, credential)| host.starts_with("127.0.0.1") && *credential)
                .count(),
            1,
            "{hosts:?}"
        );
        assert_eq!(
            server.connections.load(Ordering::Acquire),
            2,
            "one connection per name"
        );
        // The followed hop is on record for `--verbose`.
        let followed = endpoint.client.redirects().drain_followed();
        assert_eq!(followed.len(), 1);
        assert!(
            followed[0].from.contains("localhost") && followed[0].to.contains("127.0.0.1"),
            "{followed:?}"
        );
        finish(endpoint, &["operation"]).await;
    });
}
