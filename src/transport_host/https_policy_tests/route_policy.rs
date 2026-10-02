use super::*;
#[test]
fn concurrent_same_route_auth_transitions_do_not_interleave() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let seen = order.clone();
        let first = Arc::new(tokio::sync::Notify::new());
        let entered = first.clone();
        let release = Arc::new(tokio::sync::Notify::new());
        let gate = release.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            let entered = entered.clone();
            let gate = gate.clone();
            Box::pin(async move {
                let authenticated = request.headers().contains_key("authorization");
                let count = {
                    let mut order = seen.lock().unwrap();
                    order.push(authenticated);
                    order.len()
                };
                if count == 1 {
                    entered.notify_one();
                    gate.notified().await;
                }
                fixture::response(401, GitService::UploadPackAdvertisement, "refused")
            })
        }))
        .await;
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: Some(auth(root.path())),
            },
            HelperSlots::new(),
        )
        .unwrap();
        let request = runtime
            .request(meta("concurrent-auth"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let open = move || {
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
        };
        let a = tokio::task::spawn_blocking(open.clone());
        tokio::time::timeout(Duration::from_secs(2), first.notified())
            .await
            .unwrap();
        let b = tokio::task::spawn_blocking(open);
        tokio::time::sleep(Duration::from_millis(80)).await;
        release.notify_one();
        let (a, b) = tokio::join!(a, b);
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(
            *order.lock().unwrap(),
            [false, true, false],
            "a continuation must complete before another same-route opening is admitted"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn explicit_anonymous_cannot_switch_policy_and_lend_its_budget_to_gh() {
    run(async {
        use gwz_transport::protocol::AuthPolicy;
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                fixture::response(
                    if request.headers().contains_key("authorization") {
                        200
                    } else {
                        404
                    },
                    GitService::UploadPackAdvertisement,
                    "advertisement",
                )
            })
        }))
        .await;
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: Some(auth(root.path())),
            },
            HelperSlots::new(),
        )
        .unwrap();
        let request = runtime
            .request(meta("explicit-anonymous"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        tokio::task::spawn_blocking(move || {
            let error = context
                .open_https_recording(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(AuthPolicy::Anonymous),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                    Arc::new(Mutex::new(None)),
                )
                .err()
                .unwrap();
            assert_eq!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<HttpsOpenFailure>()
                    .unwrap()
                    .failure
                    .code,
                TransportError::RepositoryRefused
            );
            let error = context
                .open_https_recording(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(AuthPolicy::Gh),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                    Arc::new(Mutex::new(None)),
                )
                .err()
                .unwrap();
            let receipt = error
                .get_ref()
                .unwrap()
                .downcast_ref::<HttpsOpenFailure>()
                .unwrap();
            assert_eq!(receipt.failure.code, TransportError::InvalidRequest);
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(request.finish().await.pending_local_work, 0);
        let request = runtime
            .request(meta("independent-gh"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        tokio::task::spawn_blocking(move || {
            for _ in 0..2 {
                let stream = context
                    .open_https_recording(
                        &url,
                        GitService::UploadPackAdvertisement,
                        Some(AuthPolicy::Gh),
                        Arc::new(|_, _| {}),
                        Arc::new(|_| {}),
                        Arc::new(Mutex::new(None)),
                    )
                    .unwrap();
                let mut rpc = RpcIo::new(stream, true);
                let mut body = Vec::new();
                rpc.read_to_end(&mut body).unwrap();
                assert_eq!(body, b"advertisement");
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
