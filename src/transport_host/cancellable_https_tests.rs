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

/// A host that asks for a wait is noted to the reader the caller of the noted
/// cancellable entry gave (adaptive concurrency design §9; Code P3-9): the
/// notes are what gwz-py turns into `logging` records on `gwz`. The detail
/// lines have a reader of their own, at a level of their own, when one is set.
#[test]
fn the_noted_cancellable_entry_gives_its_reader_the_notes_of_its_transport() {
    let root = tempfile::TempDir::new().unwrap();
    let (repository, commit) = https_tests::repository(root.path());
    let repository = Arc::new(repository);
    let (started, receiver) = mpsc::channel();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let served = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let first = Arc::new(AtomicBool::new(true));
            let server = https_tests::fixture::Server::start(Arc::new(move |request| {
                let repository = repository.clone();
                let first = first.swap(false, Ordering::SeqCst);
                Box::pin(async move {
                    if first {
                        let mut reply = https_tests::fixture::response(
                            429,
                            gwz_transport::protocol::GitService::UploadPackAdvertisement,
                            "",
                        );
                        reply
                            .headers_mut()
                            .insert("Retry-After", hyper::header::HeaderValue::from_static("2"));
                        return reply;
                    }
                    https_tests::git_http_backend(repository, request).await
                })
            }))
            .await;
            started
                .send((server.url.clone(), server.ca.clone()))
                .unwrap();
            let _ = stopped.await;
        });
    });
    let (url, ca_pem) = receiver.recv().unwrap();
    let ca = root.path().join("ca.pem");
    std::fs::write(&ca, ca_pem).unwrap();
    let environment = EnvironmentSnapshot::from_os_pairs([
        (
            "HOME".into(),
            https_tests::endpoint_home(root.path()).into_os_string(),
        ),
        ("GIT_SSL_CAINFO".into(), ca.into_os_string()),
    ])
    .unwrap();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let kept = lines.clone();
    let notes = crate::transport_host::NoteSink::new(move |line| {
        kept.lock().unwrap().push(line.to_owned())
    });
    let (_controls, gate) = CallControls::new(&Arc::new(()));
    let token = gate.token().clone();
    let local = git2::Repository::init(root.path().join("local")).unwrap();
    let local = local.path().to_path_buf();
    let (result, cleanup) = super::cancellable::with_cancellable_local_transport_native_noted(
        https_tests::meta("noted-cancellable"),
        "ls-remote".into(),
        &environment,
        &token,
        None,
        notes,
        |backend| backend.ls_remote_url(&local, &url, "origin", None),
    );
    let refs = result
        .expect("the entry ran")
        .expect("the listing succeeded");
    assert!(refs.iter().any(|r| r.target == commit.to_string()));
    assert_eq!(cleanup.pending_local_work, 0);
    let lines = lines.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("note: ") && lines[0].ends_with(" asked gwz to wait 2 s; waiting"),
        "{lines:?}"
    );
    let _ = stop.send(());
    served.join().unwrap();
}
