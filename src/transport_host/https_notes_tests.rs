//! The notes of the adaptive concurrency design's §9 leave the transport host
//! through the reader a command gives it (`TransportRuntime::set_notes`, which
//! `with_local_transport_native_noted` calls): a real listing of refs over
//! HTTPS against a host that asks for a wait says so once.
use super::https_tests::{endpoint_home, fixture, git_http_backend, meta, repository};
use super::*;
use crate::git::GitBackend;
use hyper::header::HeaderValue;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn a_host_that_asks_for_a_wait_is_noted_once_to_the_reader_the_command_gave() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (repository, commit) = repository(root.path());
        let repository = Arc::new(repository);
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if first {
                    let mut reply = fixture::response(
                        429,
                        gwz_transport::protocol::GitService::UploadPackAdvertisement,
                        "",
                    );
                    reply
                        .headers_mut()
                        .insert("Retry-After", HeaderValue::from_static("2"));
                    return reply;
                }
                git_http_backend(repository, request).await
            })
        }))
        .await;
        let port = server.url.split(':').next_back().unwrap();
        let port: u16 = port.split('/').next().unwrap().parse().unwrap();
        let transport = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )
        .unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        transport.set_notes(&NoteSink::new(move |line| {
            kept.lock().unwrap().push(line.to_owned())
        }));
        let request = transport
            .request(meta("https-notes"), "ls-remote".into())
            .await
            .unwrap();
        let backend = request.backend().clone();
        let local = git2::Repository::init(root.path().join("local")).unwrap();
        let local = local.path().to_path_buf();
        let url = server.url.clone();
        let refs = tokio::task::spawn_blocking(move || {
            backend.ls_remote_url(&local, &url, "origin", None)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(refs.iter().any(|r| r.target == commit.to_string()));
        let begun = std::time::Instant::now();
        while lines.lock().unwrap().is_empty() && begun.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            *lines.lock().unwrap(),
            [format!(
                "note: localhost:{port} asked gwz to wait 2 s; waiting"
            )]
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
    });
}
