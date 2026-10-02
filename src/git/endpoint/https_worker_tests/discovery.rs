use super::*;

#[test]
fn discovery_redirect_pins_post_route_and_final_connection() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    if request.uri().path() == "/repo/info/refs" {
                        hyper::Response::builder()
                            .status(302)
                            .header("Location", "/moved/info/refs?service=git-upload-pack")
                            .body(http_body_util::Full::new(Bytes::new()))
                            .unwrap()
                    } else if request.uri().path() == "/moved/info/refs" {
                        response(200, GitService::UploadPackAdvertisement, "ok")
                    } else {
                        assert_eq!(request.uri().path(), "/moved/git-upload-pack");
                        let _ = request.into_body().collect().await.unwrap();
                        response(200, GitService::UploadPackExchange, "result")
                    }
                })
            }))
            .await;
            let mut endpoint = Endpoint::new(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            for service in [
                GitService::UploadPackAdvertisement,
                GitService::UploadPackExchange,
            ] {
                let prepared = endpoint
                    .client
                    .prepare_budget_for_transition(
                        input(&server, service),
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap();
                let (stream, task) = attach(prepared);
                stream.end_write().await.unwrap();
                let mut buffer = [0; 20];
                while stream.read(&mut buffer).await.unwrap() != 0 {}
                stream.close().await.unwrap();
                task.await.unwrap();
            }
            assert_eq!(server.connections.load(Ordering::SeqCst), 2);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
#[test]
fn discovery_failures_preserve_status_without_git_bytes_and_hops_are_bounded() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let count = Arc::new(AtomicUsize::new(0));
            let requests = count.clone();
            let server = Server::start(Arc::new(move |req| {
                let requests = requests.clone();
                Box::pin(async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    let status = req
                        .uri()
                        .path()
                        .split('/')
                        .nth(1)
                        .unwrap()
                        .parse::<u16>()
                        .unwrap_or(302);
                    let mut result = response(status, GitService::UploadPackAdvertisement, "");
                    if status == 302 {
                        result
                            .headers_mut()
                            .insert("Location", "/redirect/info/refs".parse().unwrap());
                    }
                    result
                })
            }))
            .await;
            let mut endpoint = Endpoint::new(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            for (status, code) in [
                (401, ErrorCode::Authentication),
                (403, ErrorCode::RepositoryRefused),
                (404, ErrorCode::RepositoryRefused),
                (500, ErrorCode::Io),
                (204, ErrorCode::Protocol),
            ] {
                let mut request = input(&server, GitService::UploadPackAdvertisement);
                request.destination = server.url.replace("/repo", &format!("/{status}"));
                let failure = match endpoint
                    .client
                    .prepare_budget_for_transition(
                        request,
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                {
                    Err(f) => f,
                    Ok(_) => panic!("unexpected Opened"),
                };
                assert_eq!(failure.code, code);
                assert_eq!(failure.effect, Effect::None);
                assert_eq!(failure.facts.unwrap().http_status, Some(status));
            }
            let before = count.load(Ordering::SeqCst);
            let failed = endpoint
                .client
                .prepare_budget_for_transition(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            assert!(matches!(
                failed,
                Err(Failure {
                    setup_cause: None,
                    code: ErrorCode::UnsupportedOperation,
                    ..
                })
            ));
            assert_eq!(count.load(Ordering::SeqCst) - before, 6);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn exhausted_retry_domains_fail_before_gh_lookup_without_refilling() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut endpoint = Endpoint::new(
                https_connection::Config::default(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            for domain in 0..5 {
                // The configured budget, so each pass exhausts its one domain alone:
                // an Open's zero interaction deadline would exhaust the helper in all.
                let mut budget = endpoint.client.budget();
                match domain {
                    0 => budget.allocation = Duration::ZERO,
                    1 => budget.helper = Duration::ZERO,
                    2 => budget.connect = Some(Duration::ZERO),
                    3 => budget.network = Some(Duration::ZERO),
                    _ => budget.cleanup = Duration::ZERO,
                }
                let result = endpoint
                    .client
                    .prepare_budget_for_transition(
                        Input {
                            destination: "https://example.invalid/repo".into(),
                            service: GitService::UploadPackAdvertisement,
                            policy: AuthPolicy::Gh,
                            session: "s".into(),
                            operation: "op".into(),
                        },
                        &CancellationToken::new(),
                        &mut budget,
                        &mut None,
                    )
                    .await;
                assert!(
                    matches!(
                        result,
                        Err(Failure {
                            code: ErrorCode::Timeout,
                            effect: Effect::None,
                            ..
                        })
                    ),
                    "domain {domain} must fail before missing-gh authentication lookup"
                );
            }
            assert_eq!(endpoint.shutdown(Duration::from_secs(1)).await, 0);
        });
}
