use super::*;
use std::{fs, process::Command, sync::Arc, time::Duration};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

#[path = "https_fixture.rs"]
mod fixture;
use fixture::{Server, input, response};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

cfg_if::cfg_if! { if #[cfg(unix)] {
#[test]
fn delayed_helper_is_not_capped_by_one_millisecond_allocation() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("gh");
        fs::write(
            &helper,
            "#!/bin/sh\n/bin/cat >/dev/null\n/bin/sleep 0.02\nprintf 'username=fixture\\npassword=token\\n\\n'\n",
        )
        .unwrap();
        let status = Command::new("chmod")
            .args(["+x", helper.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success());
        let auth = https_auth::Config {
            executable: helper,
            environment: Vec::new(),
        };
        let mut pool = gwz_transport::pool::Config::default();
        pool.allocation_timeout_ms = 1;
        pool.interaction_timeout_ms = 1_000;
        let mut endpoint = Endpoint::new(server.config(), Some(auth), pool).unwrap();
        let mut request = input(&server, GitService::UploadPackAdvertisement);
        request.policy = AuthPolicy::Gh;
        let prepared = endpoint
            .client
            .prepare(request, &CancellationToken::new())
            .await
            .expect("helper work must use interaction budget");
        drop(prepared);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

} }

#[test]
fn active_io_custom_timeout_expires_delayed_headers() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async {
                sleep(Duration::from_millis(40)).await;
                response(200, GitService::UploadPackAdvertisement, "ok")
            })
        }))
        .await;
        let mut endpoint = Endpoint::new_with_io_timeout(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
            5,
        )
        .unwrap();
        let result = endpoint
            .client
            .prepare(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            result,
            Err(Failure {
                code: ErrorCode::Timeout,
                ..
            })
        ));
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn active_io_zero_allows_delayed_headers() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async {
                sleep(Duration::from_millis(40)).await;
                response(200, GitService::UploadPackAdvertisement, "ok")
            })
        }))
        .await;
        let mut endpoint = Endpoint::new_with_io_timeout(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
            0,
        )
        .unwrap();
        let result = endpoint
            .client
            .prepare(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
            )
            .await;
        let prepared = result.expect("zero active I/O timeout must be disabled");
        assert_eq!(prepared.io_timeout_ms(), 0);
        drop(prepared);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn physical_capacity_wait_uses_allocation_deadline() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let mut config = gwz_transport::pool::Config::default();
        config.total = 1;
        config.per_host = 1;
        config.per_user_host = 1;
        config.allocation_timeout_ms = 15;
        let mut endpoint = Endpoint::new(server.config(), None, config).unwrap();
        let held = endpoint
            .client
            .prepare(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let result = endpoint
            .client
            .prepare(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            result,
            Err(Failure {
                code: ErrorCode::Timeout,
                ..
            })
        ));
        drop(held);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn redirects_do_not_refill_network_budget() {
    runtime().block_on(async {
        let attempt = Arc::new(AtomicUsize::new(0));
        let count = attempt.clone();
        let server = Server::start(Arc::new(move |_| {
            let first = count.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                sleep(Duration::from_millis(40)).await;
                if first {
                    let mut reply = response(302, GitService::UploadPackAdvertisement, "");
                    reply
                        .headers_mut()
                        .insert("Location", "/final/info/refs".parse().unwrap());
                    reply
                } else {
                    response(200, GitService::UploadPackAdvertisement, "ok")
                }
            })
        }))
        .await;
        let mut endpoint = Endpoint::new_with_io_timeout(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
            65,
        )
        .unwrap();
        let failed = endpoint
            .client
            .prepare(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            failed,
            Err(Failure {
                code: ErrorCode::Timeout,
                ..
            })
        ));
        assert_eq!(attempt.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn open_can_shorten_but_cannot_disable_endpoint_network_timeout() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async {
                sleep(Duration::from_millis(40)).await;
                response(200, GitService::UploadPackAdvertisement, "ok")
            })
        }))
        .await;
        let mut endpoint = Endpoint::new_with_io_timeout(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
            200,
        )
        .unwrap();
        let deadlines = Deadlines {
            io_ms: 5,
            ..Default::default()
        };
        let failed = endpoint
            .client
            .prepare_open(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
                &deadlines,
            )
            .await;
        assert!(matches!(
            failed,
            Err(Failure {
                code: ErrorCode::Timeout,
                ..
            })
        ));
        let prepared = endpoint
            .client
            .prepare_open(
                input(&server, GitService::UploadPackAdvertisement),
                &CancellationToken::new(),
                &Deadlines::default(),
            )
            .await
            .unwrap();
        assert!(prepared.io_timeout_ms() > 0 && prepared.io_timeout_ms() <= 200);
        drop(prepared);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
