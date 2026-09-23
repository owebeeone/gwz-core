//! Candidate HTTPS authentication and route-isolation integration coverage.
//!
//! This file is included by the worker test module.  The Unix boundary keeps
//! the fake executable helper out of unsupported host builds.

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::*;
            use super::super::fixture::{attach, input, response, ConnectionId, Server};
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            use hyper::header::AUTHORIZATION;
            use http_body_util::BodyExt;
            use std::{
                fs,
                path::Path,
                sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}},
                time::Duration,
            };
            use tempfile::TempDir;
            use tokio_util::sync::CancellationToken;

            fn auth_header(token: &str) -> Vec<u8> {
                format!("Basic {}", STANDARD.encode(format!("fixture:{token}"))).into_bytes()
            }

            fn recording_gh(
                log: &Path,
                host_a: &str,
                token_a: &str,
                host_b: &str,
                token_b: &str,
            ) -> (TempDir, https_auth::Config) {
                use std::os::unix::fs::PermissionsExt;
                let directory = tempfile::tempdir().unwrap();
                let executable = directory.path().join("gh");
                fs::write(
                    &executable,
                    "#!/bin/sh\n\
if [ \"$1 $2 $3\" != 'auth git-credential get' ]; then exit 4; fi\n\
input=$(/bin/cat)\n\
printf '%s\\n' \"$input\" >> \"$GH_LOG\"\n\
case \"$input\" in\n\
  *\"host=$GH_HOST_A\"*) token=\"$GH_TOKEN_A\" ;;\n\
  *\"host=$GH_HOST_B\"*) token=\"$GH_TOKEN_B\" ;;\n\
  *) exit 5 ;;\n\
esac\n\
printf 'username=fixture\\npassword=%s\\n\\n' \"$token\"\n",
                )
                .unwrap();
                fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
                (
                    directory,
                    https_auth::Config {
                        executable,
                        environment: vec![
                            ("GH_LOG".into(), log.as_os_str().into()),
                            ("GH_HOST_A".into(), host_a.into()),
                            ("GH_TOKEN_A".into(), token_a.into()),
                            ("GH_HOST_B".into(), host_b.into()),
                            ("GH_TOKEN_B".into(), token_b.into()),
                        ],
                    },
                )
            }

            fn authority(url: &str) -> String {
                let parsed = url::Url::parse(url).unwrap();
                format!(
                    "localhost:{}",
                    parsed.port_or_known_default().unwrap()
                )
            }

            async fn finish(prepared: Prepared) {
                let (stream, task) = attach(prepared);
                stream.end_write().await.unwrap();
                let mut buffer = [0; 113];
                while stream.read(&mut buffer).await.unwrap() != 0 {}
                stream.close().await.unwrap();
                task.await.unwrap();
            }

            #[test]
            fn anonymous_challenge_and_gh_retry_keep_the_same_physical_socket() {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let observed = Arc::new(Mutex::new(Vec::new()));
                        let records = observed.clone();
                        let server = Server::start(Arc::new(move |request| {
                            let records = records.clone();
                            Box::pin(async move {
                                let authenticated = request.headers().contains_key(AUTHORIZATION);
                                let status = if authenticated { 200 } else { 401 };
                                let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                                records.lock().unwrap().push((authenticated, status, connection));
                                response(status, GitService::UploadPackAdvertisement, "challenge")
                            })
                        })).await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let (_helper, auth) = recording_gh(
                            log.path(),
                            &authority(&server.url),
                            "token",
                            "unused-host",
                            "unused-token",
                        );
                        let mut endpoint = Endpoint::new(
                            server.config(),
                            Some(auth),
                            gwz_transport::pool::Config::default(),
                        ).unwrap();
                        for operation in ["clone", "fetch-1", "fetch-2"] {
                            let mut request = input(&server, GitService::UploadPackAdvertisement);
                            request.operation = operation.into();
                            let mut first = None;
                            let prepared = endpoint.client.prepare_auto(
                                request,
                                &CancellationToken::new(),
                                &mut first,
                            ).await.unwrap();
                            assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                            finish(prepared).await;
                            endpoint.client.finish_operation(operation);
                        }
                        let seen = observed.lock().unwrap().clone();
                        assert_eq!(seen.len(), 6);
                        for pair in seen.chunks_exact(2) {
                            assert_eq!(pair[0].0, false);
                            assert_eq!(pair[0].1, 401);
                            assert_eq!(pair[1].0, true);
                            assert_eq!(pair[1].1, 200);
                            assert_eq!(pair[0].2, pair[1].2);
                        }
                        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                        assert_eq!(fs::read_to_string(log.path()).unwrap().matches("host=").count(), 3);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn challenge_handoff_selects_its_socket_with_another_idle_connection() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    let observed = Arc::new(Mutex::new(Vec::new()));
                    let records = observed.clone();
                    let server = Server::start(Arc::new(move |request| {
                        let records = records.clone();
                        Box::pin(async move {
                            let authenticated = request.headers().contains_key(AUTHORIZATION);
                            let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                            records.lock().unwrap().push((authenticated, connection));
                            response(if authenticated { 200 } else { 401 }, GitService::UploadPackAdvertisement, "ok")
                        })
                    })).await;
                    let log = tempfile::NamedTempFile::new().unwrap();
                    let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                    let mut endpoint = Endpoint::new(server.config(), Some(auth), gwz_transport::pool::Config::default()).unwrap();
                    let mut a = input(&server, GitService::UploadPackAdvertisement);
                    a.policy = AuthPolicy::Gh;
                    a.operation = "seed-a".into();
                    let mut b = a.clone();
                    b.operation = "seed-b".into();
                    let seed_cancel = CancellationToken::new();
                    let (a, b) = tokio::join!(
                        endpoint.client.prepare(a, &seed_cancel),
                        endpoint.client.prepare(b, &seed_cancel),
                    );
                    finish(a.unwrap()).await;
                    finish(b.unwrap()).await;
                    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                    let mut first = None;
                    let mut request = input(&server, GitService::UploadPackAdvertisement);
                    request.operation = "challenged".into();
                    let prepared = endpoint.client.prepare_auto(request, &CancellationToken::new(), &mut first).await.unwrap();
                    assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                    finish(prepared).await;
                    let seen = observed.lock().unwrap().clone();
                    assert_eq!(seen.len(), 4);
                    assert_eq!(seen[2].0, false);
                    assert_eq!(seen[3].0, true);
                    assert_eq!(seen[2].1, seen[3].1);
                    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                });
            }

            #[test]
            fn unsafe_challenge_connections_are_discarded_before_gh_retry() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    for close in [true, false] {
                        let observed = Arc::new(Mutex::new(Vec::new()));
                        let records = observed.clone();
                        let server = Server::start(Arc::new(move |request| {
                            let records = records.clone();
                            Box::pin(async move {
                                let authenticated = request.headers().contains_key(AUTHORIZATION);
                                let connection = request.extensions().get::<ConnectionId>().unwrap().0;
                                records.lock().unwrap().push((authenticated, connection));
                                if authenticated {
                                    response(200, GitService::UploadPackAdvertisement, "ok")
                                } else if close {
                                    hyper::Response::builder().status(401).header("Connection", "close")
                                        .body(http_body_util::Full::new(bytes::Bytes::from_static(b"challenge"))).unwrap()
                                } else {
                                    response(401, GitService::UploadPackAdvertisement, vec![b'x'; 64 * 1024 + 1])
                                }
                            })
                        })).await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                        let mut endpoint = Endpoint::new(server.config(), Some(auth), gwz_transport::pool::Config::default()).unwrap();
                        let mut first = None;
                        let prepared = endpoint.client.prepare_auto(input(&server, GitService::UploadPackAdvertisement), &CancellationToken::new(), &mut first).await.unwrap();
                        assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                        finish(prepared).await;
                        let seen = observed.lock().unwrap().clone();
                        assert_eq!(seen.len(), 2);
                        assert_eq!(seen[0].0, false);
                        assert_eq!(seen[1].0, true);
                        assert_ne!(seen[0].1, seen[1].1);
                        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    }
                });
            }

            #[test]
            fn truncated_challenge_and_expired_cleanup_do_not_start_gh() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    for short_cleanup in [false, true] {
                        let server = Server::raw(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 100\r\n\r\npartial".to_vec()).await;
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                        let mut config = gwz_transport::pool::Config::default();
                        if short_cleanup {
                            config.cleanup_timeout_ms = 1;
                        }
                        let mut endpoint = Endpoint::new(server.config(), Some(auth), config).unwrap();
                        let mut first = None;
                        let error = endpoint.client.prepare_auto(
                            input(&server, GitService::UploadPackAdvertisement),
                            &CancellationToken::new(),
                            &mut first,
                        ).await.err().unwrap();
                        assert_eq!(error.code, if short_cleanup { ErrorCode::Timeout } else { ErrorCode::Protocol });
                        assert!(first.is_none());
                        assert!(fs::read_to_string(log.path()).unwrap().is_empty());
                        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
                        let _ = endpoint.shutdown(Duration::from_secs(2)).await;
                    }
                });
            }

            #[test]
            fn cancelled_challenge_handoff_discards_its_socket_without_gh_lookup() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    let seen = Arc::new(Mutex::new(Vec::new()));
                    let observed = seen.clone();
                    let server = Server::start(Arc::new(move |request| {
                        let observed = observed.clone();
                        Box::pin(async move {
                            let authenticated = request.headers().contains_key(AUTHORIZATION);
                            observed.lock().unwrap().push((authenticated, request.extensions().get::<ConnectionId>().unwrap().0));
                            response(if authenticated { 200 } else { 401 }, GitService::UploadPackAdvertisement, "challenge")
                        })
                    })).await;
                    let log = tempfile::NamedTempFile::new().unwrap();
                    let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                    let mut endpoint = Endpoint::new(server.config(), Some(auth), gwz_transport::pool::Config::default()).unwrap();
                    let mut budget = endpoint.client.budget();
                    let mut challenge = None;
                    let cancelled = CancellationToken::new();
                    let mut request = input(&server, GitService::UploadPackAdvertisement);
                    let first = endpoint.client.prepare_budget_for_transition(request.clone(), &cancelled, &mut budget, &mut challenge).await.err().unwrap();
                    assert_eq!(first.code, ErrorCode::Authentication);
                    assert!(challenge.is_some());
                    cancelled.cancel();
                    request.policy = AuthPolicy::Gh;
                    let failure = endpoint.client.prepare_budget_for_transition(request, &cancelled, &mut budget, &mut challenge).await.err().unwrap();
                    assert_eq!(failure.code, ErrorCode::Cancelled);
                    drop(challenge);
                    assert!(fs::read_to_string(log.path()).unwrap().is_empty());
                    let mut fresh = input(&server, GitService::UploadPackAdvertisement);
                    fresh.policy = AuthPolicy::Gh;
                    fresh.operation = "fresh".into();
                    finish(endpoint.client.prepare(fresh, &CancellationToken::new()).await.unwrap()).await;
                    let requests = seen.lock().unwrap().clone();
                    assert_eq!(requests.len(), 2);
                    assert_eq!(requests[0].0, false);
                    assert_eq!(requests[1].0, true);
                    assert_ne!(requests[0].1, requests[1].1);
                    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                });
            }

            #[test]
            fn delayed_challenge_handoff_discards_the_reserved_socket() {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    let seen = Arc::new(Mutex::new(Vec::new()));
                    let observed = seen.clone();
                    let server = Server::start(Arc::new(move |request| {
                        let observed = observed.clone();
                        Box::pin(async move {
                            let authenticated = request.headers().contains_key(AUTHORIZATION);
                            observed.lock().unwrap().push(request.extensions().get::<ConnectionId>().unwrap().0);
                            response(if authenticated { 200 } else { 401 }, GitService::UploadPackAdvertisement, "challenge")
                        })
                    })).await;
                    let log = tempfile::NamedTempFile::new().unwrap();
                    let (_helper, auth) = recording_gh(log.path(), &authority(&server.url), "token", "unused", "unused");
                    let mut config = gwz_transport::pool::Config::default();
                    config.cleanup_timeout_ms = 200;
                    let mut endpoint = Endpoint::new(server.config(), Some(auth), config).unwrap();
                    let mut budget = endpoint.client.budget();
                    let mut challenge = None;
                    let cancellation = CancellationToken::new();
                    let mut request = input(&server, GitService::UploadPackAdvertisement);
                    let first = endpoint.client.prepare_budget_for_transition(request.clone(), &cancellation, &mut budget, &mut challenge).await.err().unwrap();
                    assert_eq!(first.code, ErrorCode::Authentication);
                    assert!(challenge.is_some());
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    request.policy = AuthPolicy::Gh;
                    finish(endpoint.client.prepare_budget_for_transition(request, &cancellation, &mut budget, &mut challenge).await.unwrap()).await;
                    let requests = seen.lock().unwrap().clone();
                    assert_eq!(requests.len(), 2);
                    assert_ne!(requests[0], requests[1]);
                    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
                    assert_eq!(fs::read_to_string(log.path()).unwrap().matches("host=").count(), 1);
                    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                });
            }

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
                    assert_eq!(fs::read_to_string(log.path()).unwrap().matches("host=").count(), 1);
                    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                });
            }

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
                                    .push((request.uri().to_string(), authorization));
                                response(200, GitService::UploadPackAdvertisement, "target")
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
                                    .push((request.uri().to_string(), authorization));
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
                                .prepare(request, &CancellationToken::new())
                                .await
                                .unwrap(),
                        )
                        .await;
                        let records = seen.lock().unwrap().clone();
                        assert_eq!(records.len(), 2);
                        assert_eq!(records[0].1, Some(auth_header("source-token")));
                        assert_eq!(records[1].1, Some(auth_header("target-token")));
                        assert_ne!(records[0].1, records[1].1);
                        let helper_input = fs::read_to_string(log.path()).unwrap();
                        assert!(helper_input.contains(&format!("host={source_host}")));
                        assert!(helper_input.contains(&format!("host={target_host}")));
                        assert!(helper_input.matches("path=/repo").count() >= 2);
                        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
                    });
            }

            #[test]
            fn receive_pack_auto_selects_gh_before_advertisement_and_post() {
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
                        assert!(seen.iter().all(Option::is_some));
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
                                .prepare(
                                    input(&server, GitService::UploadPackAdvertisement),
                                    &CancellationToken::new(),
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
                            .prepare(request, &CancellationToken::new())
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
                            .prepare(
                                input(&server, GitService::UploadPackAdvertisement),
                                &CancellationToken::new(),
                            )
                            .await
                            .unwrap();
                        assert!(!prepared.opened.facts.credential_offered);
                        finish(prepared).await;
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
                        let first = endpoint.client.prepare(
                            input(&source, GitService::UploadPackAdvertisement),
                            &cancellation,
                        );
                        let second = endpoint.client.prepare(
                            input(&source, GitService::UploadPackAdvertisement),
                            &cancellation,
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
        }
    }
}
