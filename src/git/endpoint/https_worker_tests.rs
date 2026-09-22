use super::*;
#[test]
fn request_rejects_non_https_before_any_helper_or_connection() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut endpoint = Endpoint::new(
            super::super::https_connection::Config::default(),
            None,
            gwz_transport::pool::Config::default(),
        )
        .unwrap();
        let request = Input {
            destination: "http://example.invalid/repo".into(),
            service: GitService::UploadPackAdvertisement,
            policy: AuthPolicy::Anonymous,
            session: "s".into(),
            operation: "op".into(),
        };
        let result = endpoint
            .client
            .prepare(request, &CancellationToken::new())
            .await;
        assert!(matches!(
            result,
            Err(Failure {
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
        assert_eq!(endpoint.shutdown(Duration::from_secs(1)).await, 0);
    });
}

#[path = "https_fixture.rs"]
mod fixture;
use fixture::*;
#[test]
fn tls_discovery_and_seeded_large_exchange_reuse_one_connection() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    if request.method() == "GET" {
                        response(200, GitService::UploadPackAdvertisement, "advertisement")
                    } else {
                        let data = request.into_body().collect().await.unwrap().to_bytes();
                        response(200, GitService::UploadPackExchange, data)
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
            let first = endpoint
                .client
                .prepare(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(!first.opened.reused);
            let id = first.opened.connection_id.clone();
            let (stream, task) = attach(first);
            stream.end_write().await.unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 3];
            loop {
                let n = stream.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..n]);
            }
            assert_eq!(received, b"advertisement");
            assert_eq!(
                stream.close().await.unwrap().disposition,
                Disposition::Reusable
            );
            task.await.unwrap();
            let next = endpoint
                .client
                .prepare(
                    input(&server, GitService::UploadPackExchange),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(next.opened.reused);
            assert_eq!(id, next.opened.connection_id);
            let (stream, task) = attach(next);
            let seed = std::env::var("GWZ_HTTPS_SEED")
                .ok()
                .map(|s| s.parse::<u64>().expect("GWZ_HTTPS_SEED must be a u64"))
                .unwrap_or(0x48545450_u64);
            eprintln!("HTTPS replay seed={seed}");
            let mut state = seed;
            let payload: Vec<u8> = (0..300_000)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect();
            let write = async {
                let mut offset = 0;
                while offset < payload.len() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let end = (offset + 1 + state as usize % 7919).min(payload.len());
                    stream.write_all(&payload[offset..end]).await.unwrap();
                    offset = end;
                }
                stream.end_write().await.unwrap();
            };
            let read = async {
                let mut output = Vec::new();
                let mut read_state = seed ^ 0xd00d;
                let mut buffer = [0; 4096];
                loop {
                    read_state ^= read_state << 13;
                    read_state ^= read_state >> 7;
                    read_state ^= read_state << 17;
                    let size = 1 + read_state as usize % buffer.len();
                    let n = stream.read(&mut buffer[..size]).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    output.extend_from_slice(&buffer[..n]);
                }
                output
            };
            let (_, actual) = tokio::join!(write, read);
            assert_eq!(actual, payload, "replay seed={seed}");
            stream.close().await.unwrap();
            task.await.unwrap();
            assert_eq!(server.connections.load(Ordering::SeqCst), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
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
                    .prepare(input(&server, service), &CancellationToken::new())
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
fn early_post_rejection_wakes_blocked_writer_without_replay() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let posts = Arc::new(AtomicUsize::new(0));
            let counter = posts.clone();
            let server = Server::start(Arc::new(move |request| {
                let counter = counter.clone();
                Box::pin(async move {
                    if request.method() == "GET" {
                        response(200, GitService::ReceivePackAdvertisement, "ok")
                    } else {
                        counter.fetch_add(1, Ordering::SeqCst);
                        // Keep the rejected body alive until the server flushes its response.
                        // Dropping Hyper Incoming here can abort the response itself.
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            drop(request);
                        });
                        response(401, GitService::ReceivePackExchange, "")
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
            let prepared = endpoint
                .client
                .prepare(
                    input(&server, GitService::ReceivePackAdvertisement),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            let (stream, task) = attach(prepared);
            stream.end_write().await.unwrap();
            let mut buffer = [0; 20];
            while stream.read(&mut buffer).await.unwrap() != 0 {}
            stream.close().await.unwrap();
            task.await.unwrap();
            let prepared = endpoint
                .client
                .prepare(
                    input(&server, GitService::ReceivePackExchange),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            let (stream, task) = attach(prepared);
            let result = timeout(
                Duration::from_secs(2),
                stream.write_all(&vec![7; 16 * 1024 * 1024]),
            )
            .await
            .expect("early response must wake blocked writer");
            assert!(
                matches!(
                    result,
                    Err(gwz_transport::stream::Error::PeerFailed {
                        code: ErrorCode::Authentication,
                        effect: Effect::Possible
                    })
                ),
                "{result:?}"
            );
            assert_eq!(
                stream.retained_failure_facts().unwrap().http_status,
                Some(401)
            );
            task.await.unwrap();
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn native_git_clone_fetch_and_push_use_https_rpc_messages() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let origin = temp.path().join("origin");
            let source = temp.path().join("source");
            let source_repo = git2::Repository::init(&source).unwrap();
            let mut seed = 47u64;
            let content: Vec<u8> = (0..300_000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            std::fs::write(source.join("large.bin"), &content).unwrap();
            let mut index = source_repo.index().unwrap();
            index.add_path(std::path::Path::new("large.bin")).unwrap();
            let tree = index.write_tree().unwrap();
            let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
            source_repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    "initial",
                    &source_repo.find_tree(tree).unwrap(),
                    &[],
                )
                .unwrap();
            let mut clone = git2::build::RepoBuilder::new();
            clone.bare(true);
            clone.clone(source.to_str().unwrap(), &origin).unwrap();
            let fetch_posts = Arc::new(AtomicUsize::new(0));
            let server_posts = fetch_posts.clone();
            let origin_for_server = origin.clone();
            let server = Server::start(Arc::new(move |request| {
                let origin = origin_for_server.clone();
                let fetch_posts = server_posts.clone();
                Box::pin(async move {
                    let advertisement = request.method() == "GET";
                    let receive = request.uri().to_string().contains("receive-pack");
                    let service = match (advertisement, receive) {
                        (true, false) => GitService::UploadPackAdvertisement,
                        (false, false) => GitService::UploadPackExchange,
                        (true, true) => GitService::ReceivePackAdvertisement,
                        (false, true) => GitService::ReceivePackExchange,
                    };
                    if !advertisement && !receive {
                        fetch_posts.fetch_add(1, Ordering::SeqCst);
                    }
                    let input = request.into_body().collect().await.unwrap().to_bytes();
                    // This process is the fixture's remote Git server, never a client fallback.
                    let output = tokio::task::spawn_blocking(move || {
                        use std::io::Write;
                        let mut command = std::process::Command::new("git");
                        command
                            .arg(if receive {
                                "receive-pack"
                            } else {
                                "upload-pack"
                            })
                            .arg("--stateless-rpc");
                        if advertisement {
                            command.arg("--advertise-refs");
                        }
                        command
                            .arg(origin)
                            .stdin(std::process::Stdio::piped())
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::null());
                        let mut child = command.spawn().unwrap();
                        let mut stdin = child.stdin.take().unwrap();
                        let writer = std::thread::spawn(move || {
                            stdin.write_all(&input).unwrap();
                        });
                        let output = child.wait_with_output().unwrap();
                        writer.join().unwrap();
                        assert!(output.status.success());
                        output.stdout
                    })
                    .await
                    .unwrap();
                    let mut body = Vec::new();
                    if advertisement {
                        let line = format!("# service={}\n", https_policy::service_name(service));
                        body.extend_from_slice(format!("{:04x}", line.len() + 4).as_bytes());
                        body.extend_from_slice(line.as_bytes());
                        body.extend_from_slice(b"0000");
                    }
                    body.extend(output);
                    response(200, service, body)
                })
            }))
            .await;
            let mut endpoint = Endpoint::new(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let rpc = super::super::https_local::LocalRpc::new(
                endpoint.client.clone(),
                "session".into(),
                "operation".into(),
                Some(AuthPolicy::Anonymous),
            );
            let url = server.url.clone();
            let destination = temp.path().join("clone");
            let rpc_client = rpc.clone();
            let origin_client = origin.clone();
            tokio::task::spawn_blocking(move || {
                let mut fetch = git2::FetchOptions::new();
                fetch.remote_callbacks(super::super::https_remote::callbacks(rpc_client.clone()));
                let repo = git2::build::RepoBuilder::new()
                    .fetch_options(fetch)
                    .clone(&url, &destination)
                    .unwrap();
                assert_eq!(
                    std::fs::read(destination.join("large.bin")).unwrap(),
                    content
                );
                // Diverge enough to require several stateless negotiation rounds.
                let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
                for n in 0..128 {
                    let parent = repo.head().unwrap().peel_to_commit().unwrap();
                    repo.commit(
                        Some("HEAD"),
                        &sig,
                        &sig,
                        &format!("local {n}"),
                        &parent.tree().unwrap(),
                        &[&parent],
                    )
                    .unwrap();
                }
                let origin_repo = git2::Repository::open_bare(origin_client).unwrap();
                let parent = origin_repo.head().unwrap().peel_to_commit().unwrap();
                origin_repo
                    .commit(
                        Some("HEAD"),
                        &sig,
                        &sig,
                        "remote divergence",
                        &parent.tree().unwrap(),
                        &[&parent],
                    )
                    .unwrap();
                fetch_posts.store(0, Ordering::SeqCst);
                let mut remote = repo.find_remote("origin").unwrap();
                let mut options = git2::FetchOptions::new();
                options.remote_callbacks(super::super::https_remote::callbacks(rpc_client.clone()));
                remote
                    .fetch(&[] as &[&str], Some(&mut options), None)
                    .unwrap();
                assert!(
                    fetch_posts.load(Ordering::SeqCst) >= 2,
                    "fixture must exercise multiple fetch POSTs"
                );
                let changed: Vec<u8> = content.iter().map(|b| b.wrapping_add(17)).collect();
                std::fs::write(destination.join("large.bin"), changed).unwrap();
                let mut index = repo.index().unwrap();
                index.add_path(std::path::Path::new("large.bin")).unwrap();
                let tree = index.write_tree().unwrap();
                let parent = repo.head().unwrap().peel_to_commit().unwrap();
                let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
                repo.commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    "large push",
                    &repo.find_tree(tree).unwrap(),
                    &[&parent],
                )
                .unwrap();
                let mut push = git2::PushOptions::new();
                push.remote_callbacks(super::super::https_remote::callbacks(rpc_client));
                remote
                    .push(&["HEAD:refs/heads/copied"], Some(&mut push))
                    .unwrap();
            })
            .await
            .unwrap();
            assert_eq!(rpc.drain(Duration::from_secs(2)).await, 0);
            assert!(
                git2::Repository::open_bare(origin)
                    .unwrap()
                    .find_reference("refs/heads/copied")
                    .is_ok()
            );
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

async fn finish(prepared: Prepared) -> Vec<u8> {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut output = Vec::new();
    let mut buffer = [0; 113];
    loop {
        let n = stream.read(&mut buffer).await.unwrap();
        if n == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..n]);
    }
    stream.close().await.unwrap();
    task.await.unwrap();
    output
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
                    .prepare(request, &CancellationToken::new())
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
                .prepare(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                )
                .await;
            assert!(matches!(
                failed,
                Err(Failure {
                    code: ErrorCode::UnsupportedOperation,
                    ..
                })
            ));
            assert_eq!(count.load(Ordering::SeqCst) - before, 6);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

cfg_if::cfg_if! { if #[cfg(unix)] {
    fn fake_gh(token:&std::path::Path)->(tempfile::TempDir,https_auth::Config) {
        use std::os::unix::fs::PermissionsExt;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("gh");
        std::fs::write(&path,"#!/bin/sh\n[ \"$1 $2 $3\" = 'auth git-credential get' ] || exit 4\n/bin/cat >/dev/null\nprintf 'username=fixture\\npassword='\n/bin/cat \"$TOKEN_FILE\"\nprintf '\\n\\n'\n").unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir,https_auth::Config{executable:path,environment:vec![("TOKEN_FILE".into(),token.as_os_str().into())]})
    }
    #[test]
    fn anonymous_challenge_retries_once_and_reused_tls_gets_fresh_gh_credentials() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let temp=tempfile::tempdir().unwrap();let token=temp.path().join("token");std::fs::write(&token,"first-fixture-token").unwrap();let (_helper,auth)=fake_gh(&token);
            let seen=Arc::new(Mutex::new(Vec::new()));let observed=seen.clone();
            let server=Server::start(Arc::new(move |request|{let observed=observed.clone();Box::pin(async move {
                let auth=request.headers().get(AUTHORIZATION).map(|value|value.as_bytes().to_vec());let status=if auth.is_some(){200}else{401};observed.lock().unwrap().push(auth);response(status,GitService::UploadPackAdvertisement,"ok")
            })})).await;
            let mut endpoint=Endpoint::new(server.config(),Some(auth),gwz_transport::pool::Config::default()).unwrap();
            let mut receipt=None;let first=endpoint.client.prepare_auto(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut receipt).await.unwrap();
            assert_eq!(receipt.unwrap().facts.unwrap().http_status,Some(401));assert!(first.opened.facts.credential_offered);assert_eq!(first.opened.facts.authenticated,None);let id=first.opened.connection_id.clone();finish(first).await;
            std::fs::write(&token,"second-fixture-token").unwrap();let mut request=input(&server,GitService::UploadPackAdvertisement);request.policy=AuthPolicy::Gh;
            let second=endpoint.client.prepare(request,&CancellationToken::new()).await.unwrap();assert!(second.opened.reused);assert_eq!(second.opened.connection_id,id);assert!(second.opened.facts.credential_offered);finish(second).await;
            let seen=seen.lock().unwrap();assert_eq!(seen.len(),3);assert!(seen[0].is_none());assert!(seen[1].is_some() && seen[2].is_some() && seen[1]!=seen[2]);drop(seen);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        });
    }
} }
#[test]
fn truncated_tls_response_fails_and_never_returns_connection_to_pool() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let server=Server::raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 100\r\n\r\npartial".to_vec()).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();let prepared=endpoint.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await.unwrap();
        let (stream,task)=attach(prepared);stream.end_write().await.unwrap();let mut buffer=[0;50];let failed=loop {match stream.read(&mut buffer).await {Ok(0)=>panic!("truncation must not be EOF"),Ok(_)=>{},Err(error)=>break error}};
        assert!(matches!(failed,gwz_transport::stream::Error::PeerFailed{code:ErrorCode::Protocol,..}));task.await.unwrap();assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
    });
}
#[test]
fn informational_responses_are_bounded_and_tls_trust_is_required() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut bytes=b"HTTP/1.1 103 Early Hints\r\n\r\n".repeat(9);bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 0\r\n\r\n");let server=Server::raw(bytes).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();let failed=endpoint.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await;assert!(matches!(failed,Err(Failure{code:ErrorCode::Protocol,..})));assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        let mut untrusted=Endpoint::new(https_connection::Config::default(),None,gwz_transport::pool::Config::default()).unwrap();assert!(matches!(untrusted.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await,Err(Failure{code:ErrorCode::Trust,..})));assert_eq!(untrusted.shutdown(Duration::from_secs(2)).await,0);
    });
}
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
                        .prepare(
                            input(&server, GitService::UploadPackAdvertisement),
                            &CancellationToken::new(),
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
                    .prepare(
                        input(&server, GitService::UploadPackAdvertisement),
                        &CancellationToken::new()
                    )
                    .await,
                Err(Failure {
                    code: ErrorCode::Authentication,
                    ..
                })
            ));
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
#[test]
fn stalled_post_response_times_out_after_endwrite_without_replay() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    if request.method() == "GET" {
                        response(200, GitService::ReceivePackAdvertisement, "ok")
                    } else {
                        let _ = request.into_body().collect().await.unwrap();
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        response(200, GitService::ReceivePackExchange, "")
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
            finish(
                endpoint
                    .client
                    .prepare(
                        input(&server, GitService::ReceivePackAdvertisement),
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap(),
            )
            .await;
            let prepared = endpoint
                .client
                .prepare(
                    input(&server, GitService::ReceivePackExchange),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            let (stream, task) = attach(prepared);
            stream.end_write().await.unwrap();
            let result = timeout(Duration::from_secs(5), stream.read(&mut [0; 1]))
                .await
                .unwrap();
            assert!(matches!(
                result,
                Err(gwz_transport::stream::Error::PeerFailed {
                    code: ErrorCode::Timeout,
                    effect: Effect::Possible
                })
            ));
            task.await.unwrap();
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
#[test]
fn idle_connection_is_reaped_and_shutdown_can_be_polled_again() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|_| {
                Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
            }))
            .await;
            let config = gwz_transport::pool::Config {
                idle_timeout_ms: 20,
                ..Default::default()
            };
            let mut endpoint = Endpoint::new(server.config(), None, config).unwrap();
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
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(endpoint.client.pool.pending(), 0);
            let prepared = endpoint
                .client
                .prepare(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(!prepared.opened.reused);
            finish(prepared).await;
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            assert_eq!(endpoint.shutdown(Duration::from_millis(1)).await, 0);
        });
}

#[test]
fn idle_peer_close_is_a_typed_failure_without_a_hidden_get_retry() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let server=Server::raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 2\r\n\r\nok".to_vec()).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();finish(endpoint.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await.unwrap()).await;
        tokio::time::sleep(Duration::from_millis(80)).await;let failed=endpoint.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await;
        assert!(matches!(failed,Err(Failure{code:ErrorCode::Io,..})));assert_eq!(server.connections.load(Ordering::SeqCst),1);assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
    });
}
#[test]
fn seeded_cancellation_releases_paused_streams_and_physical_capacity() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let server=Server::start(Arc::new(|request|Box::pin(async move {if request.method()=="GET" {response(200,GitService::UploadPackAdvertisement,"ok")}else{let data=request.into_body().collect().await;response(200,GitService::UploadPackExchange,data.map(|v|v.to_bytes()).unwrap_or_default())}}))).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();finish(endpoint.client.prepare(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new()).await.unwrap()).await;
        let seed=0xca11ce1u64;let mut state=seed;
        for iteration in 0..16 {
            state^=state<<13;state^=state>>7;state^=state<<17;
            let prepared=endpoint.client.prepare(input(&server,GitService::UploadPackExchange),&CancellationToken::new()).await.unwrap();let (stream,task)=attach(prepared);let payload=vec![state as u8;300_000];
            let writing=stream.write_all(&payload);tokio::pin!(writing);
            tokio::select! {_=&mut writing=>{},_=tokio::time::sleep(Duration::from_millis(1+state%7))=>{}}
            // No response reader grants additional credit; cancellation must still pass.
            stream.cancel();let result=stream.read(&mut [0;1]).await;assert_eq!(result,Err(gwz_transport::stream::Error::Cancelled),"seed={seed} iteration={iteration}");
            timeout(Duration::from_secs(2),task).await.expect("cancelled relay stalled").unwrap();
        }
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
    });
}
#[path = "https_auth_integration_tests.rs"]
mod auth_integration;

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
                    .prepare(
                        input(&server, GitService::UploadPackAdvertisement),
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(proxy.seen.lock().unwrap().len(), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
