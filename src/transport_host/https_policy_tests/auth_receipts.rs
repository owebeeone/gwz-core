use super::*;
#[test]
fn automatic_discovery_crosses_real_failure_then_gh_open_and_keeps_receipts_private() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let observed_sockets = sockets.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            let observed_sockets = observed_sockets.clone();
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                observed_sockets.lock().unwrap().push((
                    request.headers().contains_key("authorization"),
                    request
                        .extensions()
                        .get::<fixture::ConnectionId>()
                        .unwrap()
                        .0,
                ));
                fixture::response(
                    if request.headers().contains_key("authorization") {
                        200
                    } else {
                        401
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
            .request(meta("automatic"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let facts = Arc::new(Mutex::new(Vec::<Facts>::new()));
        let output = facts.clone();
        let opened = Arc::new(Mutex::new(Vec::<Opened>::new()));
        let rows = opened.clone();
        let first_receipt = Arc::new(Mutex::new(None));
        let retained = first_receipt.clone();
        let result = tokio::task::spawn_blocking(move || {
            for _ in 0..2 {
                let out = output.clone();
                let rows = rows.clone();
                let stream = context
                    .open_https_recording(
                        &url,
                        GitService::UploadPackAdvertisement,
                        None,
                        Arc::new(move |_, o| rows.lock().unwrap().push(o.clone())),
                        Arc::new(move |f| out.lock().unwrap().push(f.clone())),
                        retained.clone(),
                    )
                    .unwrap();
                let mut rpc = RpcIo::new(stream, true);
                let mut bytes = Vec::new();
                rpc.read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"advertisement");
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let first = first_receipt
            .lock()
            .unwrap()
            .clone()
            .expect("first receipt survives later cached RPC");
        assert_eq!(first.failure.facts.as_ref().unwrap().http_status, Some(401));
        let rows = opened.lock().unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].endpoint_id, "https-endpoint");
        assert!(
            rows[0].reused,
            "Gh Open should inherit the challenge socket"
        );
        assert!(rows[1].reused);
        drop(rows);
        let sockets = sockets.lock().unwrap().clone();
        assert_eq!(sockets.len(), 3);
        assert_eq!(sockets[0].0, false);
        assert_eq!(sockets[1].0, true);
        assert_eq!(sockets[0].1, sockets[1].1);
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        let facts = facts.lock().unwrap();
        assert!(!facts.is_empty());
        assert!(facts.iter().all(|f| f.method == AuthMethod::Gh
            && f.credential_offered
            && f.authenticated.is_none()
            && f.http_status != Some(401)));
        drop(facts);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn no_helper_after_anonymous_refusal_preserves_first_receipt_without_suppression() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async {
                fixture::response(
                    404,
                    GitService::UploadPackAdvertisement,
                    "private body sentinel",
                )
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
            .request(meta("no-helper"), "fetch".into())
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
        assert!(receipt.anonymous.is_none());
        assert_eq!(
            receipt.failure.facts.as_ref().unwrap().http_status,
            Some(404)
        );
        assert_eq!(
            receipt.failure.facts.as_ref().unwrap().method,
            AuthMethod::None
        );
        assert_eq!(receipt.failure.code, TransportError::RepositoryRefused);
        assert!(!failure.to_string().contains("private body sentinel"));
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn only_final_https_repository_refusal_enters_private_member_suppression() {
    run(async {
        use crate::git::GitBackend;
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|request| {
            Box::pin(async move {
                let status = if request.uri().path().starts_with("/forbidden/") {
                    403
                } else {
                    401
                };
                fixture::response(
                    status,
                    GitService::UploadPackAdvertisement,
                    "refusal body sentinel",
                )
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
        for (name, status) in [
            ("forbidden", crate::model::ErrorCode::RemoteRejected),
            (
                "authentication",
                crate::model::ErrorCode::ExternalToolMissing,
            ),
        ] {
            let request = runtime.request(meta(name), "clone".into()).await.unwrap();
            let backend = request.backend().clone();
            let target = root.path().join(name);
            let url = server.url.replace("/repo", &format!("/{name}"));
            let error = tokio::task::spawn_blocking(move || {
                backend.clone_repo(&url, &target).err().unwrap()
            })
            .await
            .unwrap();
            assert_eq!(error.code, status, "{error:?}");
            assert!(!error.message.contains("sentinel"));
            assert_eq!(request.finish().await.pending_local_work, 0);
        }
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
