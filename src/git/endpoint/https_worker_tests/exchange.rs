use super::*;

#[test]
fn request_rejects_non_https_before_any_helper_or_connection() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut endpoint = Endpoint::new(
            https_connection::Config::default(),
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
            .prepare_budget_for_transition(
                request,
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        assert!(matches!(
            result,
            Err(Failure {
                setup_cause: None,
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
        assert_eq!(endpoint.shutdown(Duration::from_secs(1)).await, 0);
    });
}

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
                .prepare_budget_for_transition(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
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
                .prepare_budget_for_transition(
                    input(&server, GitService::UploadPackExchange),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
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
            let rpc = https_local::LocalRpc::new(
                endpoint.client.clone(),
                "session".into(),
                "operation".into(),
                Some(AuthPolicy::Anonymous),
            );
            let url = server.url.clone();
            let destination = temp.path().join("clone");
            let rpc_client = rpc.clone();
            let origin_client = origin.clone();
            // Each remote gets its own stateless transport over the RPC, as
            // `transport_binding::configure` installs one.
            let callbacks = |rpc: Arc<https_local::LocalRpc>| {
                let mut callbacks = git2::RemoteCallbacks::new();
                https_remote::install(&mut callbacks, rpc);
                callbacks
            };
            tokio::task::spawn_blocking(move || {
                let mut fetch = git2::FetchOptions::new();
                fetch.remote_callbacks(callbacks(rpc_client.clone()));
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
                options.remote_callbacks(callbacks(rpc_client.clone()));
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
                push.remote_callbacks(callbacks(rpc_client));
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

/// Git stops reading an advertisement at its flush packet, so the initiator can
/// close while the response is still streaming. From the initiator's Close the
/// stream refuses I/O-state reports (its Close owns a separate cleanup deadline),
/// which the SSH pump honours; the worker must finish that close cleanly rather
/// than fail the stream with Io.
#[test]
fn a_close_while_the_response_streams_completes_cleanly() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // Far beyond the stream's send buffer and window, so most of the
            // response is still unsent when the Close arrives.
            let body = bytes::Bytes::from(vec![b'x'; 4 << 20]);
            let server = Server::start(Arc::new(move |_| {
                let body = body.clone();
                Box::pin(async move { response(200, GitService::UploadPackAdvertisement, body) })
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
                .prepare_budget_for_transition(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await
                .unwrap();
            let (stream, task) = attach(prepared);
            let mut first = [0; 1];
            assert_eq!(stream.read(&mut first).await.unwrap(), 1);
            stream.end_write().await.unwrap();
            stream.close().await.unwrap();
            task.await.unwrap();
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}
