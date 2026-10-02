use super::*;
#[test]
            fn gh_failure_is_final_and_does_not_replay() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    let requests = Arc::new(AtomicUsize::new(0));
                    let count = requests.clone();
                    let server = Server::start(Arc::new(move |_| {
                        let count = count.clone();
                        Box::pin(async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            response(401, GitService::UploadPackAdvertisement, "challenge")
                        })
                    })).await;
                    let log = tempfile::NamedTempFile::new().unwrap();
                    let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                    let mut endpoint = Endpoint::new(server.config(), Some(auth), gwz_transport::pool::Config::default()).unwrap();
                    let mut first = None;
                    let error = endpoint.client.prepare_auto(input(&server, GitService::UploadPackAdvertisement), &CancellationToken::new(), &mut first).await.err().unwrap();
                    assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                    assert_eq!(error.code, ErrorCode::Authentication);
                    assert_eq!(requests.load(Ordering::SeqCst), 2);
                    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                    assert_eq!(fs::read_to_string(log.path()).unwrap().matches("url=https://").count(), 1);
                    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                });
            }

            #[test]
            fn receive_pack_public_advertisement_and_post_remain_anonymous() {
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
                                let authorization = request
                                    .headers()
                                    .get(AUTHORIZATION)
                                    .map(|value| value.as_bytes().to_vec());
                                observed.lock().unwrap().push(authorization);
                                if request.method() == "POST" {
                                    let _ = request.into_body().collect().await.unwrap();
                                    response(200, GitService::ReceivePackExchange, "result")
                                } else {
                                    response(200, GitService::ReceivePackAdvertisement, "advertisement")
                                }
                            })
                        }))
                        .await;
                        let token = tempfile::NamedTempFile::new().unwrap();
                        fs::write(token.path(), "receive-token").unwrap();
                        let (_helper_dir, auth) = super::super::fake_gh(token.path());
                        let mut endpoint = Endpoint::new(
                            server.config(),
                            Some(auth),
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        let mut first_receipt = None;
                        finish(
                            endpoint
                                .client
                                .prepare_auto(
                                    input(&server, GitService::ReceivePackAdvertisement),
                                    &CancellationToken::new(),
                                    &mut first_receipt,
                                )
                                .await
                                .unwrap(),
                        )
                        .await;
                        finish(
                            endpoint
                                .client
                                .prepare_auto(
                                    input(&server, GitService::ReceivePackExchange),
                                    &CancellationToken::new(),
                                    &mut first_receipt,
                                )
                                .await
                                .unwrap(),
                        )
                        .await;
                        let seen = seen.lock().unwrap();
                        assert_eq!(seen.len(), 2);
                        assert!(seen.iter().all(Option::is_none));
                        assert!(first_receipt.is_none());
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn explicit_anonymous_never_invokes_configured_helper() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let server = Server::start(Arc::new(|_| {
                            Box::pin(async {
                                response(200, GitService::UploadPackAdvertisement, "public")
                            })
                        }))
                        .await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let (_helper_dir, auth) = recording_gh(
                            log.path(),
                            &authority(&server.url),
                            "unused-a",
                            "unused-b",
                            "unused-b",
                        );
                        let mut endpoint = Endpoint::new(
                            server.config(),
                            Some(auth),
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        finish(
                            endpoint
                                .client
                                .prepare_budget_for_transition(
                                    input(&server, GitService::UploadPackAdvertisement),
                                    &CancellationToken::new(),
                                    &mut endpoint.client.budget(),
                                    &mut None,
                                )
                                .await
                                .unwrap(),
                        )
                        .await;
                        assert!(fs::read_to_string(log.path()).unwrap().is_empty());
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn gh_401_does_not_replay_the_same_discovery_request() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let requests = Arc::new(AtomicUsize::new(0));
                        let count = requests.clone();
                        let server = Server::start(Arc::new(move |_| {
                            let count = count.clone();
                            Box::pin(async move {
                                count.fetch_add(1, Ordering::SeqCst);
                                response(401, GitService::UploadPackAdvertisement, "")
                            })
                        }))
                        .await;
                        let token = tempfile::NamedTempFile::new().unwrap();
                        fs::write(token.path(), "one-token").unwrap();
                        let (_helper_dir, auth) = super::super::fake_gh(token.path());
                        let mut endpoint = Endpoint::new(
                            server.config(),
                            Some(auth),
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        let mut request = input(&server, GitService::UploadPackAdvertisement);
                        request.policy = AuthPolicy::Gh;
                        let failure = endpoint
                            .client
                            .prepare_budget_for_transition(
                                request,
                                &CancellationToken::new(),
                                &mut endpoint.client.budget(),
                                &mut None,
                            )
                            .await
                            .err().expect("expected failure");
                        assert_eq!(failure.code, ErrorCode::Authentication);
                        assert_eq!(failure.facts.unwrap().http_status, Some(401));
                        assert_eq!(requests.load(Ordering::SeqCst), 1);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn anonymous_public_discovery_works_without_gh() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let server = Server::start(Arc::new(|_| {
                            Box::pin(async {
                                response(200, GitService::UploadPackAdvertisement, "public")
                            })
                        }))
                        .await;
                        let mut endpoint = Endpoint::new(
                            server.config(),
                            None,
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        let prepared = endpoint
                            .client
                            .prepare_budget_for_transition(
                                input(&server, GitService::UploadPackAdvertisement),
                                &CancellationToken::new(),
                                &mut endpoint.client.budget(),
                                &mut None,
                            )
                            .await
                            .unwrap();
                        assert!(!prepared.opened.facts.credential_offered);
                        finish(prepared).await;
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }
