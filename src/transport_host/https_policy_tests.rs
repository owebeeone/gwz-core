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
        assert!(first.stream_id.is_some());
        assert_eq!(first.policy, gwz_transport::protocol::AuthPolicy::Anonymous);
        assert_eq!(first.failure.facts.as_ref().unwrap().http_status, Some(401));
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
        let first = receipt.anonymous.as_ref().expect("complete first receipt");
        assert_eq!(first.failure.facts.as_ref().unwrap().http_status, Some(404));
        assert!(first.stream_id.is_some());
        assert_eq!(first.policy, gwz_transport::protocol::AuthPolicy::Anonymous);
        assert!(receipt.stream_id.is_some());
        assert_ne!(receipt.stream_id, first.stream_id);
        assert_eq!(receipt.policy, gwz_transport::protocol::AuthPolicy::Gh);
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
                .open_https(
                    &url,
                    GitService::UploadPackAdvertisement,
                    None,
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
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
            [false, true, false, true],
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
                .open_https(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(AuthPolicy::Anonymous),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
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
                .open_https(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(AuthPolicy::Gh),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                )
                .err()
                .unwrap();
            let receipt = error
                .get_ref()
                .unwrap()
                .downcast_ref::<HttpsOpenFailure>()
                .unwrap();
            assert_eq!(receipt.failure.code, TransportError::InvalidRequest);
            assert!(receipt.stream_id.is_none());
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
                    .open_https(
                        &url,
                        GitService::UploadPackAdvertisement,
                        Some(AuthPolicy::Gh),
                        Arc::new(|_, _| {}),
                        Arc::new(|_| {}),
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

#[test]
fn cancelling_receive_pack_preparation_has_no_publication_effect() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let post_count = Arc::new(AtomicUsize::new(0));
        let seen = post_count.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            Box::pin(async move {
                if request.method() == "POST" {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
                fixture::response(200, GitService::ReceivePackAdvertisement, "advertisement")
            })
        }))
        .await;
        let executable = root.path().join("gh-gated");
        let counter = root.path().join("first");
        let blocked = root.path().join("blocked");
        std::fs::write(&executable, b"#!/bin/sh\nwhile IFS= read -r line; do [ -z \"$line\" ] && break; done\nif [ -e \"$COUNTER\" ]; then touch \"$BLOCKED\"; exec sleep 10; fi\ntouch \"$COUNTER\"\nprintf 'username=fixture\\npassword=fixture-token\\n\\n'\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: Some(https_auth::Config {
                    executable,
                    environment: vec![
                        ("COUNTER".into(), counter.into_os_string()),
                        ("BLOCKED".into(), blocked.clone().into_os_string()),
                    ],
                }),
            },
        )
        .unwrap();
        let request = runtime
            .request(meta("cancel-receive-pack"), "push".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        tokio::task::spawn_blocking(move || {
            let stream = context
                .open_https(
                    &url,
                    GitService::ReceivePackAdvertisement,
                    None,
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                )
                .unwrap();
            RpcIo::new(stream, true)
                .read_to_end(&mut Vec::new())
                .unwrap();
        })
        .await
        .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let pending = tokio::task::spawn_blocking(move || {
            context.open_https(
                &url,
                GitService::ReceivePackExchange,
                None,
                Arc::new(|_, _| {}),
                Arc::new(|_| {}),
            )
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !blocked.exists() {
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
            .expect("canceled before POST");
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
        assert_eq!(post_count.load(Ordering::SeqCst), 0);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
