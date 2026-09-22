//! Native HTTP/git compatibility alongside the HTTPS host endpoint.

use super::https_tests;
use super::*;
use crate::git::GitBackend;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    task::JoinSet,
    time::sleep,
};

type Handler = Arc<
    dyn Fn(Request<Incoming>) -> Pin<Box<dyn Future<Output = Response<Full<Bytes>>> + Send>>
        + Send
        + Sync,
>;

struct PlainHttpServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl PlainHttpServer {
    async fn start(repository: Arc<PathBuf>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler: Handler = Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move { https_tests::git_http_backend(repository, request).await })
        });
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break; };
                        let handler = handler.clone();
                        children.spawn(async move {
                            let service = service_fn(move |request| {
                                let handler = handler.clone();
                                async move { Ok::<_, std::convert::Infallible>((handler)(request).await) }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(TokioIo::new(socket), service)
                                .await;
                        });
                    }
                    _ = children.join_next(), if !children.is_empty() => {}
                }
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/repo"),
            task,
        }
    }
}

impl Drop for PlainHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct GitDaemon {
    url: String,
    child: Option<Child>,
}

impl GitDaemon {
    async fn start(repository: &Path) -> Self {
        let parent = repository.parent().unwrap().to_path_buf();
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let mut daemon = Self {
            url: format!("git://127.0.0.1:{port}/repo"),
            child: Some(
                Command::new("git")
                    .arg("daemon")
                    .arg("--reuseaddr")
                    .arg("--export-all")
                    .arg(format!("--base-path={}", parent.display()))
                    .arg(format!("--port={port}"))
                    .arg("--listen=127.0.0.1")
                    .kill_on_drop(true)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            ),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let ready = loop {
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                break true;
            }
            if daemon
                .child
                .as_mut()
                .is_some_and(|child| child.try_wait().unwrap().is_some())
            {
                break false;
            }
            sleep(Duration::from_millis(5)).await;
        };
        if !ready {
            daemon.stop().await;
            panic!("git daemon failed to become ready");
        }
        daemon
    }

    async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

impl Drop for GitDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

#[test]
fn local_http_and_git_remain_native_compatibility_transports() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (repository, expected_commit) = https_tests::repository(root.path());
        let repository = Arc::new(repository);
        let http = PlainHttpServer::start(repository.clone()).await;
        let mut git = GitDaemon::start(repository.as_ref()).await;
        let transport = TransportRuntime::new(SshEndpointConfig::fixture(
            https_tests::endpoint_home(&root.path().join("endpoint")),
            None,
        ))
        .unwrap();

        for (request_id, url) in [
            ("native-http", http.url.clone()),
            ("native-git", git.url.clone()),
        ] {
            let request = transport
                .request(https_tests::meta(request_id), "clone".into())
                .await
                .unwrap();
            let backend = request.backend().clone();
            let target = root.path().join(request_id);
            let target_for_clone = target.clone();
            let url_for_clone = url.clone();
            let result = tokio::task::spawn_blocking(move || {
                backend.clone_repo(&url_for_clone, &target_for_clone)
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.head.commit, Some(expected_commit.to_string()));
            let refs_backend = request.backend().clone();
            let refs_target = target.clone();
            let refs =
                tokio::task::spawn_blocking(move || refs_backend.ls_remote(&refs_target, "origin"))
                    .await
                    .unwrap()
                    .unwrap();
            assert!(
                refs.iter()
                    .any(|item| item.target == expected_commit.to_string())
            );
            assert_eq!(request.finish().await.pending_local_work, 0);
        }
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
        git.stop().await;
    });
}
