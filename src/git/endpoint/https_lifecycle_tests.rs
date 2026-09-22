use super::super::super::{https_local::LocalRpc, https_remote::OpenRpc};
use super::*;
use std::io::{Read, Write};

async fn rpc_exchange(
    rpc: Arc<LocalRpc>,
    url: String,
    service: GitService,
) -> std::io::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        let mut stream = rpc.open(&url, service)?;
        if service == GitService::UploadPackExchange {
            stream.write_all(b"request")?;
        }
        stream.end_write()?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes)?;
        stream.close()?;
        Ok(bytes)
    })
    .await
    .unwrap()
}

#[test]
fn dropping_one_remote_preserves_the_other_remotes_pinned_route() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let other = Arc::new(AtomicBool::new(false));
            let choice = other.clone();
            let server = Server::start(Arc::new(move |request| {
                let other = choice.load(Ordering::SeqCst);
                Box::pin(async move {
                    if request.uri().path() == "/repo/info/refs" {
                        let mut reply = response(302, GitService::UploadPackAdvertisement, "");
                        reply.headers_mut().insert(
                            "Location",
                            if other {
                                "/b/info/refs"
                            } else {
                                "/a/info/refs"
                            }
                            .parse()
                            .unwrap(),
                        );
                        reply
                    } else if request.method() == "POST" {
                        let path = request.uri().path().to_owned();
                        request.into_body().collect().await.unwrap();
                        response(200, GitService::UploadPackExchange, path)
                    } else {
                        response(200, GitService::UploadPackAdvertisement, "advertisement")
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
            let a = LocalRpc::new(
                endpoint.client.clone(),
                "s".into(),
                "op".into(),
                Some(AuthPolicy::Anonymous),
            );
            let b = LocalRpc::new(
                endpoint.client.clone(),
                "s".into(),
                "op".into(),
                Some(AuthPolicy::Anonymous),
            );
            rpc_exchange(
                a.clone(),
                server.url.clone(),
                GitService::UploadPackAdvertisement,
            )
            .await
            .unwrap();
            rpc_exchange(
                b.clone(),
                server.url.clone(),
                GitService::UploadPackAdvertisement,
            )
            .await
            .unwrap();
            assert_eq!(a.drain(Duration::from_secs(1)).await, 0);
            drop(a);
            other.store(true, Ordering::SeqCst);
            let c = LocalRpc::new(
                endpoint.client.clone(),
                "s".into(),
                "op".into(),
                Some(AuthPolicy::Anonymous),
            );
            let conflict = rpc_exchange(
                c.clone(),
                server.url.clone(),
                GitService::UploadPackAdvertisement,
            )
            .await;
            assert!(
                conflict.is_err(),
                "dropping A allowed C to replace B's route"
            );
            assert_eq!(
                rpc_exchange(
                    b.clone(),
                    server.url.clone(),
                    GitService::UploadPackExchange
                )
                .await
                .unwrap(),
                b"/a/git-upload-pack"
            );
            assert_eq!(b.drain(Duration::from_secs(1)).await, 0);
            assert_eq!(c.drain(Duration::from_secs(1)).await, 0);
            endpoint.client.finish_operation("op");
            assert!(
                rpc_exchange(
                    b.clone(),
                    server.url.clone(),
                    GitService::UploadPackAdvertisement
                )
                .await
                .is_err(),
                "sealed operation admitted new work"
            );
            assert_eq!(b.drain(Duration::from_secs(1)).await, 0);
            drop((b, c));
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

cfg_if::cfg_if! { if #[cfg(unix)] {
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn helper() -> (tempfile::TempDir, https_auth::Config, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let executable = dir.path().join("gh");
        std::fs::write(&executable, "#!/bin/sh\n/bin/cat >/dev/null\nprintf 'started\\n' >> \"$MARKER\"\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = https_auth::Config { executable, environment: vec![("MARKER".into(), marker.clone().into())] };
        (dir, config, marker)
    }
    async fn started(path: &std::path::Path) {
        timeout(Duration::from_secs(2), async { while !path.exists() { tokio::time::sleep(Duration::from_millis(2)).await; } }).await.unwrap();
    }
    fn request() -> Input { Input { destination: "https://localhost/repo".into(), service: GitService::UploadPackAdvertisement, policy: AuthPolicy::Gh, session: "s".into(), operation: "op".into() } }
    #[test]
    fn endpoint_shutdown_owns_active_helpers_and_rejects_later_preparations() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (_dir, config, marker) = helper();
            let mut endpoint = Endpoint::new(https_connection::Config::default(), Some(config), gwz_transport::pool::Config::default()).unwrap();
            let client = endpoint.client.clone();
            let task = tokio::spawn(async move { client.prepare(request(), &CancellationToken::new()).await });
            started(&marker).await;
            let pending = endpoint.shutdown(Duration::from_secs(1)).await;
            assert!(task.is_finished() || pending != 0, "shutdown reported clean with active helper");
            let result = timeout(Duration::from_secs(2), task).await.expect("shutdown left helper alive").unwrap();
            assert!(matches!(result, Err(Failure { code: ErrorCode::Cancelled, .. })));
            let before = std::fs::read(&marker).unwrap();
            let result = endpoint.client.prepare(request(), &CancellationToken::new()).await;
            assert!(matches!(result, Err(Failure { code: ErrorCode::Cancelled, .. })));
            assert_eq!(std::fs::read(&marker).unwrap(), before, "post-shutdown helper ran");
            assert_eq!(endpoint.shutdown(Duration::from_secs(1)).await, 0);
        });
    }
    #[test]
    fn shutdown_does_not_cancel_or_count_another_endpoints_helper() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (_dir, config, marker) = helper();
            let mut a = Endpoint::new(https_connection::Config::default(), None, gwz_transport::pool::Config::default()).unwrap();
            let mut b = Endpoint::new(https_connection::Config::default(), Some(config), gwz_transport::pool::Config::default()).unwrap();
            let client = b.client.clone();
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            let task = tokio::spawn(async move { client.prepare(request(), &token).await });
            started(&marker).await;
            assert_eq!(a.shutdown(Duration::from_millis(20)).await, 0);
            assert!(!task.is_finished(), "A shutdown touched B's helper");
            cancel.cancel();
            assert!(timeout(Duration::from_secs(2), task).await.unwrap().unwrap().is_err());
            assert_eq!(b.shutdown(Duration::from_secs(1)).await, 0);
        });
    }
}
} }
