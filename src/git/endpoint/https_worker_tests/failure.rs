use super::*;

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
                .prepare_budget_for_transition(
                    input(&server, GitService::ReceivePackAdvertisement),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
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
                .prepare_budget_for_transition(
                    input(&server, GitService::ReceivePackExchange),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
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
fn truncated_tls_response_fails_and_never_returns_connection_to_pool() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let server=Server::raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 100\r\n\r\npartial".to_vec()).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();let prepared=endpoint.client.prepare_budget_for_transition(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut endpoint.client.budget(),&mut None).await.unwrap();
        let (stream,task)=attach(prepared);stream.end_write().await.unwrap();let mut buffer=[0;50];let failed=loop {match stream.read(&mut buffer).await {Ok(0)=>panic!("truncation must not be EOF"),Ok(_)=>{},Err(error)=>break error}};
        assert!(matches!(failed,gwz_transport::stream::Error::PeerFailed{code:ErrorCode::Protocol,..}));task.await.unwrap();assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
    });
}
#[test]
fn informational_responses_are_bounded_and_tls_trust_is_required() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut bytes=b"HTTP/1.1 103 Early Hints\r\n\r\n".repeat(9);bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 0\r\n\r\n");let server=Server::raw(bytes).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();let failed=endpoint.client.prepare_budget_for_transition(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut endpoint.client.budget(),&mut None).await;assert!(matches!(failed,Err(Failure{code:ErrorCode::Protocol,..})));assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        let mut untrusted=Endpoint::new(https_connection::Config::default(),None,gwz_transport::pool::Config::default()).unwrap();assert!(matches!(untrusted.client.prepare_budget_for_transition(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut untrusted.client.budget(),&mut None).await,Err(Failure{code:ErrorCode::Trust,..})));assert_eq!(untrusted.shutdown(Duration::from_secs(2)).await,0);
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
            let mut endpoint = Endpoint::new_with_io_timeout(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
                1_000,
            )
            .unwrap();
            finish(
                endpoint
                    .client
                    .prepare_budget_for_transition(
                        input(&server, GitService::ReceivePackAdvertisement),
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap(),
            )
            .await;
            let prepared = endpoint
                .client
                .prepare_budget_for_transition(
                    input(&server, GitService::ReceivePackExchange),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
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
