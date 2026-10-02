use super::*;
#[test]
            fn gh_redirect_looks_up_each_origin_and_never_reuses_auth_header() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let seen = Arc::new(Mutex::new(Vec::<(String, Option<Vec<u8>>)>::new()));
                        let seen_target = seen.clone();
                        let target = Server::start(Arc::new(move |request| {
                            let seen_target = seen_target.clone();
                            Box::pin(async move {
                                let authorization = request
                                    .headers()
                                    .get(AUTHORIZATION)
                                    .map(|value| value.as_bytes().to_vec());
                                seen_target
                                    .lock()
                                    .unwrap()
                                    .push((request.uri().to_string(), authorization.clone()));
                                if authorization.is_some() { response(200, GitService::UploadPackAdvertisement, "target") } else { response(401, GitService::UploadPackAdvertisement, "challenge") }
                            })
                        }))
                        .await;
                        let target_url = target.url.clone();
                        let source_seen = seen.clone();
                        let source = Server::start(Arc::new(move |request| {
                            let source_seen = source_seen.clone();
                            let target_url = target_url.clone();
                            Box::pin(async move {
                                let authorization = request
                                    .headers()
                                    .get(AUTHORIZATION)
                                    .map(|value| value.as_bytes().to_vec());
                                source_seen
                                    .lock()
                                    .unwrap()
                                    .push((request.uri().to_string(), authorization.clone()));
                                let location = format!(
                                    "{target_url}/info/refs?service=git-upload-pack"
                                );
                                hyper::Response::builder()
                                    .status(302)
                                    .header("Location", location)
                                    .body(http_body_util::Full::new(bytes::Bytes::new()))
                                    .unwrap()
                            })
                        }))
                        .await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let source_host = authority(&source.url);
                        let target_host = authority(&target.url);
                        let (_helper_dir, auth) = recording_gh(
                            log.path(),
                            &source_host,
                            "source-token",
                            &target_host,
                            "target-token",
                        );
                        let mut endpoint = Endpoint::new(
                            source.config(),
                            Some(auth),
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        let mut request = input(
                            &source,
                            GitService::UploadPackAdvertisement,
                        );
                        request.policy = AuthPolicy::Gh;
                        finish(
                            endpoint
                                .client
                                .prepare_budget_for_transition(
                                    request,
                                    &CancellationToken::new(),
                                    &mut endpoint.client.budget(),
                                    &mut None,
                                )
                                .await
                                .unwrap(),
                        )
                        .await;
                        let records = seen.lock().unwrap().clone();
                        assert_eq!(records.len(), 3);
                        assert_eq!(records[0].1, Some(auth_header("source-token")));
                        assert!(records[1].1.is_none());
                        assert_eq!(records[2].1, Some(auth_header("target-token")));
                        assert_ne!(records[0].1, records[2].1);
                        let helper_input = fs::read_to_string(log.path()).unwrap();
                        assert!(helper_input.contains(&format!("url=https://{source_host}/")));
                        assert!(helper_input.contains(&format!("url=https://{target_host}/")));
                        assert!(helper_input.matches("/repo").count() >= 2);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn concurrent_same_operation_conflicting_redirects_fail_protocol() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let target_a = Server::start(Arc::new(|_| {
                            Box::pin(async {
                                response(200, GitService::UploadPackAdvertisement, "a")
                            })
                        }))
                        .await;
                        let target_b = Server::start(Arc::new(|_| {
                            Box::pin(async {
                                response(200, GitService::UploadPackAdvertisement, "b")
                            })
                        }))
                        .await;
                        let choices = Arc::new(AtomicUsize::new(0));
                        let counter = choices.clone();
                        let location_a = format!(
                            "{}/info/refs?service=git-upload-pack",
                            target_a.url
                        );
                        let location_b = format!(
                            "{}/info/refs?service=git-upload-pack",
                            target_b.url
                        );
                        let source = Server::start(Arc::new(move |_| {
                            let counter = counter.clone();
                            let location = if counter.fetch_add(1, Ordering::SeqCst) % 2 == 0 {
                                location_a.clone()
                            } else {
                                location_b.clone()
                            };
                            Box::pin(async move {
                                hyper::Response::builder()
                                    .status(302)
                                    .header("Location", location)
                                    .body(http_body_util::Full::new(bytes::Bytes::new()))
                                    .unwrap()
                            })
                        }))
                        .await;
                        let mut endpoint = Endpoint::new(
                            source.config(),
                            None,
                            gwz_transport::pool::Config::default(),
                        )
                        .unwrap();
                        let cancellation=CancellationToken::new();
                        let (mut first_budget, mut second_budget) =
                            (endpoint.client.budget(), endpoint.client.budget());
                        let (mut first_challenge, mut second_challenge) = (None, None);
                        let first = endpoint.client.prepare_budget_for_transition(
                            input(&source, GitService::UploadPackAdvertisement),
                            &cancellation,
                            &mut first_budget,
                            &mut first_challenge,
                        );
                        let second = endpoint.client.prepare_budget_for_transition(
                            input(&source, GitService::UploadPackAdvertisement),
                            &cancellation,
                            &mut second_budget,
                            &mut second_challenge,
                        );
                        let (first, second) = tokio::join!(first, second);
                        let mut successes = 0;
                        let mut protocol_failures = 0;
                        for result in [first, second] {
                            match result {
                                Ok(prepared) => {
                                    successes += 1;
                                    finish(prepared).await;
                                }
                                Err(failure) if failure.code == ErrorCode::Protocol => {
                                    protocol_failures += 1;
                                }
                                Err(failure) => panic!("unexpected conflict result: {failure:?}"),
                            }
                        }
                        assert_eq!(successes, 1);
                        assert_eq!(protocol_failures, 1);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }
