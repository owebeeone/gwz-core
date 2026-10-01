//! S6.1's cancel while running, over HTTPS (TR2.17): the entry returns
//! within `RETIRED` of the cancel, as it does over SSH
//! (`cancellable_tests.rs`).

use super::cancellable_tests::{assert_prompt, cancel_while_running};
use super::https_tests;
use super::*;
use crate::git::GitBackend;
use crate::session_host::EnvironmentSnapshot;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

/// A cancel while running over HTTPS: `git http-backend` serves the
/// advertisement, and the server holds the clone's upload-pack POST
/// unanswered. The cancel fails the clone's read of the POST's response at
/// once, and the entry returns within `RETIRED`, as over SSH.
#[test]
fn a_cancel_while_running_over_https_fails_its_io_and_returns_promptly() {
    let root = tempfile::TempDir::new().unwrap();
    let posted = Arc::new(AtomicBool::new(false));
    let server = HeldPosts::start(root.path(), posted.clone());
    let ca = root.path().join("ca.pem");
    std::fs::write(&ca, &server.ca).unwrap();
    let home = https_tests::endpoint_home(root.path());
    let environment = EnvironmentSnapshot::from_os_pairs([
        ("HOME".into(), home.into_os_string()),
        ("GIT_SSL_CAINFO".into(), ca.into_os_string()),
    ])
    .unwrap();
    let target = root.path().join("clone");
    let cancelled = cancel_while_running(
        &https_tests::meta("https-cancel-while-running"),
        &environment,
        || posted.load(Ordering::Acquire),
        || server.stop(),
        |backend| backend.clone_repo(&server.url, &target),
    );
    assert!(cancelled.value.is_err(), "the cancelled clone fails");
    assert_prompt("HTTPS", &cancelled);
}

/// The HTTPS fixture, on a runtime and thread of its own, since the entry
/// runs its own executor. `git http-backend` answers every GET; a POST is
/// recorded in `posted` and held unanswered until the server stops.
struct HeldPosts {
    url: String,
    ca: Vec<u8>,
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HeldPosts {
    fn start(root: &Path, posted: Arc<AtomicBool>) -> Self {
        let (repository, _) = https_tests::repository(root);
        let repository = Arc::new(repository);
        let (started, receiver) = mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let server = https_tests::fixture::Server::start(Arc::new(move |request| {
                    let repository = repository.clone();
                    let posted = posted.clone();
                    Box::pin(async move {
                        if *request.method() == hyper::Method::POST {
                            posted.store(true, Ordering::Release);
                            std::future::pending().await
                        } else {
                            https_tests::git_http_backend(repository, request).await
                        }
                    })
                }))
                .await;
                started
                    .send((server.url.clone(), server.ca.clone()))
                    .unwrap();
                let _ = stopped.await;
            });
        });
        let (url, ca) = receiver.recv().unwrap();
        Self {
            url,
            ca,
            stop: Mutex::new(Some(stop)),
            thread: Some(thread),
        }
    }

    /// Stops the server, which ends every connection it holds.
    fn stop(&self) {
        let stop = self
            .stop
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
    }
}

impl Drop for HeldPosts {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
