use super::super::https_tests::{endpoint_home, fixture, meta};
use super::super::*;
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

fn open_for_test(
    context: RequestContext,
    url: String,
    allocation_ms: i64,
    attempt_started: Arc<Notify>,
    owner_opened: Arc<Notify>,
    observed_allocations: Arc<Mutex<Vec<i64>>>,
) -> tokio::task::JoinHandle<Result<crate::git::endpoint::stream_io::BlockingStream, std::io::Error>>
{
    tokio::task::spawn_blocking(move || {
        attempt_started.notify_one();
        let observed = observed_allocations.clone();
        context.open_https_recording_for_test(
            &url,
            gwz_transport::protocol::GitService::UploadPackAdvertisement,
            Some(gwz_transport::protocol::AuthPolicy::Anonymous),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
            Arc::new(std::sync::Mutex::new(None)),
            allocation_ms,
            Arc::new(move |value| {
                observed.lock().unwrap().push(value);
                owner_opened.notify_one();
            }),
        )
    })
}

fn local_config(root: &std::path::Path) -> SshEndpointConfig {
    let mut config = SshEndpointConfig::fixture(endpoint_home(root), None);
    config.pool.total = 1;
    config.pool.per_host = 1;
    config.pool.per_user_host = 1;
    config
}

#[test]
fn route_wait_reduces_first_open_allocation_budget_and_second_stage_times_out() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let first_entered = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());
        let server = fixture::Server::start(Arc::new({
            let calls = calls.clone();
            let first_entered = first_entered.clone();
            let release_first = release_first.clone();
            move |_request| {
                let number = calls.fetch_add(1, Ordering::SeqCst) + 1;
                let first_entered = first_entered.clone();
                let release_first = release_first.clone();
                Box::pin(async move {
                    if number == 1 {
                        first_entered.notify_one();
                        release_first.notified().await;
                    }
                    fixture::response(
                        200,
                        gwz_transport::protocol::GitService::UploadPackAdvertisement,
                        "advertisement",
                    )
                })
            }
        }))
        .await;
        let runtime = TransportRuntime::with_https(
            local_config(root.path()),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
        )
        .unwrap();
        let request = runtime
            .request(meta("gate-budget"), "fetch".into())
            .await
            .unwrap();
        let first_attempt = Arc::new(Notify::new());
        let second_attempt = Arc::new(Notify::new());
        let first_owner = Arc::new(Notify::new());
        let second_owner = Arc::new(Notify::new());
        let observed = Arc::new(Mutex::new(Vec::new()));
        let first = open_for_test(
            request.context.clone(),
            server.url.clone(),
            1000,
            first_attempt.clone(),
            first_owner,
            observed.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), first_entered.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), first_attempt.notified())
            .await
            .unwrap();
        let second = open_for_test(
            request.context.clone(),
            server.url.clone(),
            1000,
            second_attempt.clone(),
            second_owner.clone(),
            observed.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), second_attempt.notified())
            .await
            .unwrap();
        let started = std::time::Instant::now();
        tokio::time::sleep(Duration::from_millis(600)).await;
        release_first.notify_one();
        let first_stream = first.await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(2), second_owner.notified())
            .await
            .unwrap();
        let result = second.await.unwrap();
        let elapsed = started.elapsed();
        drop(first_stream);
        let error = result.err().expect("occupied physical pool must time out");
        let failure = error
            .get_ref()
            .unwrap()
            .downcast_ref::<HttpsOpenFailure>()
            .unwrap();
        assert_eq!(
            failure.failure.code,
            gwz_transport::protocol::ErrorCode::Timeout
        );
        assert!(
            elapsed < Duration::from_millis(1300),
            "allocation was replenished: {elapsed:?}"
        );
        let allocations = observed.lock().unwrap().clone();
        assert_eq!(allocations.len(), 2);
        assert!(
            allocations[1] > 0 && allocations[1] < 500,
            "gate remainder was refilled: {allocations:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn exhausted_route_budget_sends_no_second_open() {
    run(async {
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let first_entered = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());
        let server = fixture::Server::start(Arc::new({
            let calls = calls.clone();
            let first_entered = first_entered.clone();
            let release_first = release_first.clone();
            move |_request| {
                let first_entered = first_entered.clone();
                let release_first = release_first.clone();
                let number = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Box::pin(async move {
                    if number == 1 {
                        first_entered.notify_one();
                        release_first.notified().await;
                    }
                    fixture::response(
                        200,
                        gwz_transport::protocol::GitService::UploadPackAdvertisement,
                        "advertisement",
                    )
                })
            }
        }))
        .await;
        let runtime = TransportRuntime::with_https(
            local_config(root.path()),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
        )
        .unwrap();
        let request = runtime
            .request(meta("gate-exhausted"), "fetch".into())
            .await
            .unwrap();
        let first_attempt = Arc::new(Notify::new());
        let second_attempt = Arc::new(Notify::new());
        let first_owner = Arc::new(Notify::new());
        let second_owner = Arc::new(Notify::new());
        let observed = Arc::new(Mutex::new(Vec::new()));
        let first = open_for_test(
            request.context.clone(),
            server.url.clone(),
            50,
            first_attempt,
            first_owner,
            observed.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), first_entered.notified())
            .await
            .unwrap();
        let second = open_for_test(
            request.context.clone(),
            server.url.clone(),
            50,
            second_attempt.clone(),
            second_owner,
            observed.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), second_attempt.notified())
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(300), second)
            .await
            .unwrap()
            .unwrap();
        let error = result.err().expect("expired gate must fail");
        let failure = error
            .get_ref()
            .unwrap()
            .downcast_ref::<HttpsOpenFailure>()
            .unwrap();
        assert_eq!(
            failure.failure.code,
            gwz_transport::protocol::ErrorCode::Timeout
        );
        release_first.notify_one();
        assert!(first.await.unwrap().is_ok());
        assert_eq!(observed.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
