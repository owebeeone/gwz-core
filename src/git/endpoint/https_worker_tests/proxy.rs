use super::*;

#[test]
fn connect_proxy_and_no_proxy_keep_proxy_credentials_out_of_origin_requests() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    assert!(!request.headers().contains_key("Proxy-Authorization"));
                    assert!(!request.headers().contains_key(AUTHORIZATION));
                    response(200, GitService::UploadPackAdvertisement, "ok")
                })
            }))
            .await;
            let proxy = Tunnel::start(200).await;
            for bypass in [false, true] {
                let mut config = server.config();
                config.proxy = Some(proxy.config.clone());
                if bypass {
                    config.no_proxy = vec!["localhost".into()];
                }
                let mut endpoint =
                    Endpoint::new(config, None, gwz_transport::pool::Config::default()).unwrap();
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
                assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
            let seen = proxy.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert!(seen[0].contains("Proxy-Authorization: Basic fixture-proxy-only"));
            assert!(!seen[0].contains("\r\nAuthorization:"));
            drop(seen);
            let denied = Tunnel::start(407).await;
            let mut config = server.config();
            config.proxy = Some(denied.config.clone());
            let mut endpoint =
                Endpoint::new(config, None, gwz_transport::pool::Config::default()).unwrap();
            assert!(matches!(
                endpoint
                    .client
                    .prepare_budget_for_transition(
                        input(&server, GitService::UploadPackAdvertisement),
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await,
                Err(Failure {
                    setup_cause: None,
                    code: ErrorCode::Authentication,
                    ..
                })
            ));
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
#[test]
fn https_connect_proxy_preserves_origin_tls_verification() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    assert!(!request.headers().contains_key("Proxy-Authorization"));
                    response(200, GitService::UploadPackAdvertisement, "ok")
                })
            }))
            .await;
            let proxy = Tunnel::with_tls(200, true).await;
            let mut config = server.config();
            config.proxy = Some(proxy.config.clone());
            let mut endpoint =
                Endpoint::new(config, None, gwz_transport::pool::Config::default()).unwrap();
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
            assert_eq!(proxy.seen.lock().unwrap().len(), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
