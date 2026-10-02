use super::*;
#[test]
fn cancellation_wakes_pending_https_open_and_retires_its_owner() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let started = Arc::new(AtomicUsize::new(0));
        let seen = started.clone();
        let server = fixture::Server::start(Arc::new(move |_| {
            let seen = seen.clone();
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
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
            .request(meta("cancel-headers"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let pending = tokio::task::spawn_blocking(move || {
            context.open_https_recording(
                &url,
                GitService::UploadPackAdvertisement,
                None,
                Arc::new(|_, _| {}),
                Arc::new(|_| {}),
                Arc::new(Mutex::new(None)),
            )
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while started.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        request.cancel();
        let error = tokio::time::timeout(Duration::from_millis(250), pending)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("canceled pending discovery");
        let receipt = error
            .get_ref()
            .unwrap()
            .downcast_ref::<HttpsOpenFailure>()
            .unwrap();
        assert_eq!(receipt.failure.code, TransportError::Cancelled);
        assert_eq!(
            receipt.failure.effect,
            gwz_transport::protocol::Effect::None
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        // Canceling one request must not destroy the reusable host registration.
        let next = runtime
            .request(meta("after-cancel"), "fetch".into())
            .await
            .unwrap();
        assert_eq!(next.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn cancelling_receive_pack_challenge_helper_has_no_publication_effect() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let post_count = Arc::new(AtomicUsize::new(0));
        let seen = post_count.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            Box::pin(async move {
                if request.method() == "POST" { seen.fetch_add(1, Ordering::SeqCst); }
                fixture::response(401, GitService::ReceivePackAdvertisement, "challenge")
            })
        })).await;
        let executable = root.path().join("git-gated");
        let blocked = root.path().join("blocked");
        crate::git::endpoint::helper_script::write_git_fixture(
            &executable, "touch \"$BLOCKED\"; exec /bin/sleep 10",
        );
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig { tls: server.config(), auth: Some(https_auth::Config {
                executable, environment: vec![("BLOCKED".into(), blocked.clone().into_os_string())],
            }) }, HelperSlots::new(),
        ).unwrap();
        let request = runtime.request(meta("cancel-receive-pack"), "push".into()).await.unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let pending = tokio::task::spawn_blocking(move || context.open_https_recording(
            &url, GitService::ReceivePackAdvertisement, None, Arc::new(|_, _| {}),
            Arc::new(|_| {}), Arc::new(Mutex::new(None)),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !blocked.exists() { tokio::time::sleep(Duration::from_millis(2)).await; }
        }).await.unwrap();
        request.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(2), pending).await.unwrap().unwrap().is_err());
        assert_eq!(post_count.load(Ordering::SeqCst), 0);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
