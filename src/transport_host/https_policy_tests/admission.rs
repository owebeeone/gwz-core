use super::*;
#[test]
fn explicit_anonymous_refusal_does_not_hold_capacity_for_the_request_lifetime() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async {
                fixture::response(401, GitService::UploadPackAdvertisement, "challenge")
            })
        }))
        .await;
        let mut config = SshEndpointConfig::fixture(endpoint_home(root.path()), None);
        config.pool.cleanup_timeout_ms = 200;
        let runtime = TransportRuntime::with_https(
            config,
            HttpsEndpointConfig {
                tls: server.config(),
                auth: Some(auth(root.path())),
            },
            HelperSlots::new(),
        )
        .unwrap();
        let first = runtime
            .request(meta("held-anonymous"), "fetch".into())
            .await
            .unwrap();
        let context = first.context.clone();
        let url = server.url.clone();
        let failure = tokio::task::spawn_blocking(move || {
            context
                .open_https_recording(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(gwz_transport::protocol::AuthPolicy::Anonymous),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                    Arc::new(Mutex::new(None)),
                )
                .err()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(
            failure
                .get_ref()
                .unwrap()
                .downcast_ref::<HttpsOpenFailure>()
                .unwrap()
                .failure
                .code,
            TransportError::Authentication
        );
        let endpoint = runtime
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .local_endpoint
            .clone();
        let until = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let counts = endpoint.https_counts_for_test().unwrap();
            if counts.total() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "challenge still holds capacity: {counts:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        assert_eq!(first.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn ssh_only_bound_peer_rejects_https_without_opening_a_socket() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async {
                fixture::response(200, GitService::UploadPackAdvertisement, "unused")
            })
        }))
        .await;
        let runtime =
            TransportRuntime::new(SshEndpointConfig::fixture(endpoint_home(root.path()), None))
                .unwrap();
        let request = runtime
            .request(meta("ssh-only"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let failure = tokio::task::spawn_blocking(move || {
            context
                .open_https_recording(
                    &url,
                    GitService::UploadPackAdvertisement,
                    None,
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                    Arc::new(Mutex::new(None)),
                )
                .err()
                .unwrap()
        })
        .await
        .unwrap();
        let receipt = failure
            .get_ref()
            .unwrap()
            .downcast_ref::<HttpsOpenFailure>()
            .unwrap();
        assert_eq!(receipt.failure.code, TransportError::UnsupportedOperation);
        assert_eq!(server.connections.load(Ordering::SeqCst), 0);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn more_than_one_admission_window_of_abandoned_rpcs_retires_before_request_finish() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async {
                fixture::response(200, GitService::UploadPackAdvertisement, vec![7u8; 131072])
            })
        }))
        .await;
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )
        .unwrap();
        let request = runtime
            .request(meta("many-rpcs"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        tokio::time::timeout(
            Duration::from_secs(20),
            tokio::task::spawn_blocking(move || {
                for _ in 0..70 {
                    let mut stream = context
                        .open_https_recording(
                            &url,
                            GitService::UploadPackAdvertisement,
                            Some(AuthPolicy::Anonymous),
                            Arc::new(|_, _| {}),
                            Arc::new(|_| {}),
                            Arc::new(Mutex::new(None)),
                        )
                        .unwrap();
                    assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 1);
                    stream.cancel();
                }
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(request.finish().await.pending_local_work, 0);
        let next = runtime
            .request(meta("after-many-rpcs"), "fetch".into())
            .await
            .unwrap();
        assert_eq!(next.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
