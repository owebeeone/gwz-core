use super::*;
#[test]
fn anonymous_challenge_and_gh_retry_keep_the_same_physical_socket() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let observed = Arc::new(Mutex::new(Vec::new()));
            let records = observed.clone();
            let server = Server::start(Arc::new(move |request| {
                let records = records.clone();
                Box::pin(async move {
                    let authenticated = request.headers().contains_key(AUTHORIZATION);
                    let status = if authenticated { 200 } else { 401 };
                    let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                    records
                        .lock()
                        .unwrap()
                        .push((authenticated, status, connection));
                    response(status, GitService::UploadPackAdvertisement, "challenge")
                })
            }))
            .await;
            let log = tempfile::NamedTempFile::new().unwrap();
            let (_helper, auth) = recording_gh(
                log.path(),
                &authority(&server.url),
                "token",
                "unused-host",
                "unused-token",
            );
            let mut endpoint = Endpoint::new(
                server.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            for operation in ["clone", "fetch-1", "fetch-2"] {
                let mut request = input(&server, GitService::UploadPackAdvertisement);
                request.operation = operation.into();
                let mut first = None;
                let prepared = endpoint
                    .client
                    .prepare_auto(request, &CancellationToken::new(), &mut first)
                    .await
                    .unwrap();
                assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                finish(prepared).await;
                endpoint.client.finish_operation(operation);
            }
            let seen = observed.lock().unwrap().clone();
            assert_eq!(seen.len(), 6);
            for pair in seen.chunks_exact(2) {
                assert_eq!(pair[0].0, false);
                assert_eq!(pair[0].1, 401);
                assert_eq!(pair[1].0, true);
                assert_eq!(pair[1].1, 200);
                assert_eq!(pair[0].2, pair[1].2);
            }
            assert_eq!(server.connections.load(Ordering::SeqCst), 3);
            assert_eq!(
                fs::read_to_string(log.path())
                    .unwrap()
                    .matches("url=https://")
                    .count(),
                3
            );
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn challenge_handoff_selects_its_socket_with_another_idle_connection() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let observed = Arc::new(Mutex::new(Vec::new()));
            let records = observed.clone();
            let server = Server::start(Arc::new(move |request| {
                let records = records.clone();
                Box::pin(async move {
                    let authenticated = request.headers().contains_key(AUTHORIZATION);
                    let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                    records.lock().unwrap().push((authenticated, connection));
                    response(
                        if authenticated { 200 } else { 401 },
                        GitService::UploadPackAdvertisement,
                        "ok",
                    )
                })
            }))
            .await;
            let log = tempfile::NamedTempFile::new().unwrap();
            let (_helper, auth) = recording_gh(
                log.path(),
                &authority(&server.url),
                "token",
                "unused",
                "unused",
            );
            let mut endpoint = Endpoint::new(
                server.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let mut a = input(&server, GitService::UploadPackAdvertisement);
            a.policy = AuthPolicy::Gh;
            a.operation = "seed-a".into();
            let mut b = a.clone();
            b.operation = "seed-b".into();
            let seed_cancel = CancellationToken::new();
            let (mut budget_a, mut budget_b) = (endpoint.client.budget(), endpoint.client.budget());
            let (mut challenge_a, mut challenge_b) = (None, None);
            let (a, b) = tokio::join!(
                endpoint.client.prepare_budget_for_transition(
                    a,
                    &seed_cancel,
                    &mut budget_a,
                    &mut challenge_a
                ),
                endpoint.client.prepare_budget_for_transition(
                    b,
                    &seed_cancel,
                    &mut budget_b,
                    &mut challenge_b
                ),
            );
            finish(a.unwrap()).await;
            finish(b.unwrap()).await;
            assert_eq!(server.connections.load(Ordering::SeqCst), 2);
            let mut first = None;
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            request.operation = "challenged".into();
            let prepared = endpoint
                .client
                .prepare_auto(request, &CancellationToken::new(), &mut first)
                .await
                .unwrap();
            assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
            finish(prepared).await;
            let seen = observed.lock().unwrap().clone();
            assert_eq!(seen.len(), 4);
            assert_eq!(seen[2].0, false);
            assert_eq!(seen[3].0, true);
            assert_eq!(seen[2].1, seen[3].1);
            assert_eq!(server.connections.load(Ordering::SeqCst), 3);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn unsafe_challenge_connections_are_discarded_before_gh_retry() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for close in [true, false] {
                let observed = Arc::new(Mutex::new(Vec::new()));
                let records = observed.clone();
                let server = Server::start(Arc::new(move |request| {
                    let records = records.clone();
                    Box::pin(async move {
                        let authenticated = request.headers().contains_key(AUTHORIZATION);
                        let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                        records.lock().unwrap().push((authenticated, connection));
                        if authenticated {
                            response(200, GitService::UploadPackAdvertisement, "ok")
                        } else if close {
                            hyper::Response::builder()
                                .status(401)
                                .header("WWW-Authenticate", "Basic")
                                .header("Connection", "close")
                                .body(http_body_util::Full::new(bytes::Bytes::from_static(
                                    b"challenge",
                                )))
                                .unwrap()
                        } else {
                            response(
                                401,
                                GitService::UploadPackAdvertisement,
                                vec![b'x'; 64 * 1024 + 1],
                            )
                        }
                    })
                }))
                .await;
                let log = tempfile::NamedTempFile::new().unwrap();
                let (_helper, auth) = recording_gh(
                    log.path(),
                    &authority(&server.url),
                    "token",
                    "unused",
                    "unused",
                );
                let mut endpoint = Endpoint::new(
                    server.config(),
                    Some(auth),
                    gwz_transport::pool::Config::default(),
                )
                .unwrap();
                let mut first = None;
                let prepared = endpoint
                    .client
                    .prepare_auto(
                        input(&server, GitService::UploadPackAdvertisement),
                        &CancellationToken::new(),
                        &mut first,
                    )
                    .await
                    .unwrap();
                assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                finish(prepared).await;
                let seen = observed.lock().unwrap().clone();
                assert_eq!(seen.len(), 2);
                assert_eq!(seen[0].0, false);
                assert_eq!(seen[1].0, true);
                assert_ne!(seen[0].1, seen[1].1);
                assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
        });
}

#[test]
fn truncated_challenge_and_expired_cleanup_do_not_start_gh() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    for short_cleanup in [false, true] {
                        let server = Server::raw(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic\r\nContent-Length: 100\r\n\r\npartial".to_vec()).await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                        let mut config = gwz_transport::pool::Config::default();
                        if short_cleanup {
                            config.cleanup_timeout_ms = 1;
                        }
                        let mut endpoint = Endpoint::new(server.config(), Some(auth), config).unwrap();
                        let mut first = None;
                        let error = endpoint.client.prepare_auto(
                            input(&server, GitService::UploadPackAdvertisement),
                            &CancellationToken::new(),
                            &mut first,
                        ).await.err().unwrap();
                        assert_eq!(error.code, if short_cleanup { ErrorCode::Timeout } else { ErrorCode::Protocol });
                        assert!(first.is_none());
                        assert!(fs::read_to_string(log.path()).unwrap().is_empty());
                        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                        let _ = endpoint.shutdown(Duration::from_secs(2)).await;
                    }
                });
}

#[test]
fn cancelled_challenge_handoff_discards_its_socket_without_gh_lookup() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let observed = seen.clone();
            let server = Server::start(Arc::new(move |request| {
                let observed = observed.clone();
                Box::pin(async move {
                    let authenticated = request.headers().contains_key(AUTHORIZATION);
                    observed.lock().unwrap().push((
                        authenticated,
                        request.extensions().get::<ConnectionId>().unwrap().0,
                    ));
                    response(
                        if authenticated { 200 } else { 401 },
                        GitService::UploadPackAdvertisement,
                        "challenge",
                    )
                })
            }))
            .await;
            let log = tempfile::NamedTempFile::new().unwrap();
            let (_helper, auth) = recording_gh(
                log.path(),
                &authority(&server.url),
                "token",
                "unused",
                "unused",
            );
            let mut endpoint = Endpoint::new(
                server.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let mut budget = endpoint.client.budget();
            let mut challenge = None;
            let cancelled = CancellationToken::new();
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            let first = endpoint
                .client
                .prepare_budget_for_transition(
                    request.clone(),
                    &cancelled,
                    &mut budget,
                    &mut challenge,
                )
                .await
                .err()
                .unwrap();
            assert_eq!(first.code, ErrorCode::Authentication);
            assert!(challenge.is_some());
            cancelled.cancel();
            request.policy = AuthPolicy::Gh;
            let failure = endpoint
                .client
                .prepare_budget_for_transition(request, &cancelled, &mut budget, &mut challenge)
                .await
                .err()
                .unwrap();
            assert_eq!(failure.code, ErrorCode::Cancelled);
            drop(challenge);
            assert!(fs::read_to_string(log.path()).unwrap().is_empty());
            let mut fresh = input(&server, GitService::UploadPackAdvertisement);
            fresh.policy = AuthPolicy::Gh;
            fresh.operation = "fresh".into();
            finish(
                endpoint
                    .client
                    .prepare_budget_for_transition(
                        fresh,
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap(),
            )
            .await;
            let requests = seen.lock().unwrap().clone();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].0, false);
            assert_eq!(requests[1].0, true);
            assert_ne!(requests[0].1, requests[1].1);
            assert_eq!(server.connections.load(Ordering::SeqCst), 2);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn delayed_challenge_handoff_discards_the_reserved_socket() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let observed = seen.clone();
            let server = Server::start(Arc::new(move |request| {
                let observed = observed.clone();
                Box::pin(async move {
                    let authenticated = request.headers().contains_key(AUTHORIZATION);
                    observed
                        .lock()
                        .unwrap()
                        .push(request.extensions().get::<ConnectionId>().unwrap().0);
                    response(
                        if authenticated { 200 } else { 401 },
                        GitService::UploadPackAdvertisement,
                        "challenge",
                    )
                })
            }))
            .await;
            let log = tempfile::NamedTempFile::new().unwrap();
            let (_helper, auth) = recording_gh(
                log.path(),
                &authority(&server.url),
                "token",
                "unused",
                "unused",
            );
            let mut config = gwz_transport::pool::Config::default();
            config.cleanup_timeout_ms = 200;
            let mut endpoint = Endpoint::new(server.config(), Some(auth), config).unwrap();
            let mut budget = endpoint.client.budget();
            let mut challenge = None;
            let cancellation = CancellationToken::new();
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            let first = endpoint
                .client
                .prepare_budget_for_transition(
                    request.clone(),
                    &cancellation,
                    &mut budget,
                    &mut challenge,
                )
                .await
                .err()
                .unwrap();
            assert_eq!(first.code, ErrorCode::Authentication);
            assert!(challenge.is_some());
            tokio::time::sleep(Duration::from_millis(250)).await;
            request.policy = AuthPolicy::Gh;
            finish(
                endpoint
                    .client
                    .prepare_budget_for_transition(
                        request,
                        &cancellation,
                        &mut budget,
                        &mut challenge,
                    )
                    .await
                    .unwrap(),
            )
            .await;
            let requests = seen.lock().unwrap().clone();
            assert_eq!(requests.len(), 2);
            assert_ne!(requests[0], requests[1]);
            assert_eq!(server.connections.load(Ordering::SeqCst), 2);
            assert_eq!(
                fs::read_to_string(log.path())
                    .unwrap()
                    .matches("url=https://")
                    .count(),
                1
            );
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
