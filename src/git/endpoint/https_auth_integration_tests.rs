//! Candidate HTTPS authentication and route-isolation integration coverage.
//!
//! This file is included by the worker test module.  The Unix boundary keeps
//! the fake executable helper out of unsupported host builds.

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::*;
            use super::super::fixture::{attach, input, response, Server};
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
