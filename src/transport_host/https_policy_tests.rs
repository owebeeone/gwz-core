//! Policy and failure receipts exercised through the actual host mux.
use super::https_tests::{endpoint_home, fixture, meta};
use super::*;
use crate::git::endpoint::{https_auth, https_remote::RpcIo};
use gwz_transport::protocol::{AuthMethod, ErrorCode as TransportError, Facts, GitService, Opened};
use std::{
    io::Read,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

fn auth(root: &std::path::Path) -> https_auth::Config {
    let executable = root.join("fake-gh");
    std::fs::write(&executable,b"#!/bin/sh\nwhile IFS= read -r line; do [ -z \"$line\" ] && break; done\nprintf 'username=fixture\\npassword=sentinel-h2-token\\n\\n'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    https_auth::Config {
        executable,
        environment: Vec::new(),
    }
}
fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}
use std::future::Future;

#[test]
fn automatic_discovery_crosses_real_failure_then_gh_open_and_keeps_receipts_private() {
    run(async {
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
        let result = tokio::task::spawn_blocking(move || {
            for _ in 0..2 {
                let out = output.clone();
                let rows = rows.clone();
                let stream = context
                    .open_https(
                        &url,
                        GitService::UploadPackAdvertisement,
                        None,
                        Arc::new(move |_, o| rows.lock().unwrap().push(o.clone())),
                        Arc::new(move |f| out.lock().unwrap().push(f.clone())),
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
        let rows = opened.lock().unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].endpoint_id, "https-endpoint");
        assert!(rows[1].reused);
        drop(rows);
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
                .open_https(
                    &url,
                    GitService::UploadPackAdvertisement,
                    None,
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
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
                .open_https(
                    &url,
                    GitService::UploadPackAdvertisement,
                    None,
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
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
        assert_eq!(receipt.anonymous_status, Some(404));
        assert_ne!(receipt.failure.code, TransportError::RepositoryRefused);
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
        )
        .unwrap();
        for (name, status) in [
            ("forbidden", crate::model::ErrorCode::RemoteRejected),
            ("authentication", crate::model::ErrorCode::GitCommandFailed),
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
        )
        .unwrap();
        let request = runtime
            .request(meta("cancel-headers"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let pending = tokio::task::spawn_blocking(move || {
            context.open_https(
                &url,
                GitService::UploadPackAdvertisement,
                None,
                Arc::new(|_, _| {}),
                Arc::new(|_| {}),
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
        assert!(
            tokio::time::timeout(Duration::from_millis(250), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
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
                        .open_https(
                            &url,
                            GitService::UploadPackAdvertisement,
                            Some(AuthPolicy::Anonymous),
                            Arc::new(|_, _| {}),
                            Arc::new(|_| {}),
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
