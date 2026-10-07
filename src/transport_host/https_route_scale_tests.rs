//! A request over more HTTPS repositories than the old 64-route caps allowed
//! (adaptive concurrency design §7.1 and §7.2, §10.2 case 12).
//!
//! One fixture host serves `COUNT` distinct repository URLs, every one of them
//! the same bare repository under another name. The request's backend lists
//! each URL's refs from `jobs` threads at once, through the transport host, as
//! the workspace commands do. None may fail, and once the request has
//! finished the endpoint's route table is empty.
use super::https_tests::{endpoint_home, fixture, git_http_backend, meta, repository};
use super::*;
use crate::git::GitBackend;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const COUNT: usize = 200;

/// Lists the refs of `COUNT` distinct repository URLs from `jobs` threads of
/// one request whose per-host limit is `per_host`. Returns the failures.
fn list_all(jobs: u32, per_host: u32) -> Vec<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (repository, commit) = repository(root.path());
        for index in 0..COUNT {
            std::os::unix::fs::symlink(&repository, root.path().join(format!("repo-{index}")))
                .unwrap();
        }
        let repository = Arc::new(repository);
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move { git_http_backend(repository, request).await })
        }))
        .await;
        let transport = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )
        .unwrap();
        let mut request_meta = meta("https-route-scale");
        request_meta.policy = Some(crate::OperationPolicy {
            concurrency: Some(jobs.into()),
            max_connections_per_host: Some(per_host.into()),
            ..Default::default()
        });
        let request = transport
            .request(request_meta, "ls-remote".into())
            .await
            .unwrap();
        let client = transport
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .local_endpoint
            .https_client_for_test();
        let backend = request.backend().clone();
        let local = git2::Repository::init(root.path().join("local")).unwrap();
        let local = local.path().to_path_buf();
        let next = Arc::new(AtomicUsize::new(0));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let base = server.url.trim_end_matches("/repo").to_owned();
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                let (backend, local, next, failures, base, client, peak) = (
                    backend.clone(),
                    local.clone(),
                    next.clone(),
                    failures.clone(),
                    base.clone(),
                    client.clone(),
                    peak.clone(),
                );
                tokio::task::spawn_blocking(move || {
                    loop {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        if index >= COUNT {
                            return;
                        }
                        let url = format!("{base}/repo-{index}");
                        match backend.ls_remote_url(&local, &url, "origin", None) {
                            Ok(refs) => {
                                assert!(
                                    refs.iter().any(|r| r.target == commit.to_string()),
                                    "{url} listed {refs:?}"
                                );
                            }
                            Err(error) => failures
                                .lock()
                                .unwrap()
                                .push(format!("{url}: {:?} {}", error.code, error.message)),
                        }
                        peak.fetch_max(client.route_count_for_test(), Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.await.unwrap();
        }
        assert!(
            peak.load(Ordering::SeqCst) > 64 || !failures.lock().unwrap().is_empty(),
            "the request never held more than 64 routes: {}",
            peak.load(Ordering::SeqCst)
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(
            client.route_count_for_test(),
            0,
            "a finished request leaves no route behind"
        );
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
        failures.lock().unwrap().clone()
    })
}

#[test]
fn two_hundred_distinct_https_repositories_with_one_hundred_jobs_all_succeed() {
    let failed = list_all(100, 32);
    assert!(
        failed.is_empty(),
        "{} of {COUNT} failed, first: {:?}",
        failed.len(),
        failed.first()
    );
}

#[test]
fn two_hundred_distinct_https_repositories_succeed_at_a_limit_of_eight() {
    let failed = list_all(100, 8);
    assert!(
        failed.is_empty(),
        "{} of {COUNT} failed, first: {:?}",
        failed.len(),
        failed.first()
    );
}
