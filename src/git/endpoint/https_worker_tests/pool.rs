use super::*;
use crate::git::endpoint::cut_proxy::CutProxy;

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
                    .prepare_budget_for_transition(
                        input(&server, GitService::UploadPackAdvertisement),
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap(),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(endpoint.client.pool.pending(), 0);
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
            assert!(!prepared.opened.reused);
            finish(prepared).await;
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            assert_eq!(endpoint.shutdown(Duration::from_millis(1)).await, 0);
        });
}

#[test]
fn seeded_cancellation_releases_paused_streams_and_physical_capacity() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let server=Server::start(Arc::new(|request|Box::pin(async move {if request.method()=="GET" {response(200,GitService::UploadPackAdvertisement,"ok")}else{let data=request.into_body().collect().await;response(200,GitService::UploadPackExchange,data.map(|v|v.to_bytes()).unwrap_or_default())}}))).await;
        let mut endpoint=Endpoint::new(server.config(),None,gwz_transport::pool::Config::default()).unwrap();finish(endpoint.client.prepare_budget_for_transition(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut endpoint.client.budget(),&mut None).await.unwrap()).await;
        let seed=0xca11ce1u64;let mut state=seed;
        for iteration in 0..16 {
            state^=state<<13;state^=state>>7;state^=state<<17;
            let prepared=endpoint.client.prepare_budget_for_transition(input(&server,GitService::UploadPackExchange),&CancellationToken::new(),&mut endpoint.client.budget(),&mut None).await.unwrap();let (stream,task)=attach(prepared);let payload=vec![state as u8;300_000];
            let writing=stream.write_all(&payload);tokio::pin!(writing);
            tokio::select! {_=&mut writing=>{},_=tokio::time::sleep(Duration::from_millis(1+state%7))=>{}}
            // No response reader grants additional credit; cancellation must still pass.
            stream.cancel();let result=stream.read(&mut [0;1]).await;assert_eq!(result,Err(gwz_transport::stream::Error::Cancelled),"seed={seed} iteration={iteration}");
            timeout(Duration::from_secs(2),task).await.expect("cancelled relay stalled").unwrap();
        }
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
    });
}

// Idle connections the server closed (dev-docs/GwzTransportIdleLossDesign.md):
// noticed while idle, replaced when found dead before a byte was written, and
// never retried once the request was written.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
async fn advertisement(endpoint: &mut Endpoint, input: Input) -> Result<Prepared, Failure> {
    endpoint
        .client
        .prepare_budget_for_transition(
            input,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await
}

#[test]
fn the_server_closing_an_idle_connection_frees_its_slot_with_no_open() {
    runtime().block_on(async {
        // The fixture answers one request per connection, then closes it.
        let server = Server::raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 2\r\n\r\nok".to_vec()).await;
        let mut endpoint =
            Endpoint::new(server.config(), None, gwz_transport::pool::Config::default())
                .unwrap();
        let input = input(&server, GitService::UploadPackAdvertisement);
        finish(advertisement(&mut endpoint, input.clone()).await.unwrap()).await;
        let deadline = Instant::now() + Duration::from_secs(5);
        while endpoint.client.pool.pool.counts().total() != 0 {
            assert!(Instant::now() < deadline, "the idle loss is reported");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(endpoint.client.pool.pending(), 0);
        let prepared = advertisement(&mut endpoint, input).await.unwrap();
        assert!(!prepared.opened.reused);
        finish(prepared).await;
        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_get_written_before_the_server_closes_is_not_retried() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let port: u16 = server
            .url
            .rsplit(':')
            .next()
            .unwrap()
            .trim_end_matches("/repo")
            .parse()
            .unwrap();
        let proxy = CutProxy::start(port);
        let mut input = input(&server, GitService::UploadPackAdvertisement);
        input.destination = format!("https://localhost:{}/repo", proxy.port);
        let mut endpoint = Endpoint::new(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
        )
        .unwrap();
        finish(advertisement(&mut endpoint, input.clone()).await.unwrap()).await;
        // The server closes the idle connection as the next request arrives.
        proxy.arm_existing();
        let failed = advertisement(&mut endpoint, input).await;
        assert!(matches!(
            failed,
            Err(Failure {
                code: ErrorCode::Io,
                ..
            })
        ));
        assert_eq!(proxy.connections(), 1);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

fn get() -> Request<RequestBody> {
    let (sender, body) = body_channel();
    drop(sender);
    Request::get("/repo").body(body).unwrap()
}

#[test]
fn a_request_on_a_connection_already_closed_is_not_started() {
    runtime().block_on(async {
        let (client, server) = tokio::io::duplex(4096);
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(client))
                .await
                .unwrap();
        drop(server);
        let _ = connection.await;
        assert!(matches!(
            send_request(&mut sender, get()).await,
            Err(SendFailure::NotStarted)
        ));
    });
}

#[test]
fn a_request_the_peer_received_before_closing_was_sent() {
    runtime().block_on(async {
        use tokio::io::AsyncReadExt;
        let (client, mut server) = tokio::io::duplex(4096);
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(client))
                .await
                .unwrap();
        let driver = tokio::spawn(connection);
        let peer = tokio::spawn(async move {
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(server.read_u8().await.unwrap());
            }
        });
        assert!(matches!(
            send_request(&mut sender, get()).await,
            Err(SendFailure::Sent(_))
        ));
        peer.await.unwrap();
        let _ = driver.await;
    });
}
