//! The pool's supervisor sleeps until it is woken, so each way the pool learns
//! that an idle connection is lost must wake it (dev-docs/
//! GwzTransportIdleLossDesign.md; `PoolWake`). With the pool's idle timeout at
//! 60 s, a lost signal that wakes nothing is found only at that deadline, and
//! each test gives the supervisor one second.
use super::*;
use crate::git::endpoint::{cut_proxy::CutProxy, https_fixture::Server};
use http_body_util::BodyExt;
use std::{
    future::Future,
    pin::pin,
    task::{Poll, Waker},
};

const PROMPTLY: Duration = Duration::from_secs(1);

fn get() -> hyper::Request<https_connection::RequestBody> {
    let (sender, rx) = tokio::sync::mpsc::channel(1);
    drop(sender);
    hyper::Request::get("/repo")
        .header(hyper::header::HOST, "localhost")
        .body(https_connection::RequestBody { rx })
        .unwrap()
}

/// A pool with one idle connection, made through a proxy that can cut it.
struct Idle {
    running: RunningPool,
    proxy: CutProxy,
    key: Key,
    owner: Owner,
    _server: Server,
}
impl Idle {
    async fn new() -> Self {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async {
                crate::git::endpoint::https_fixture::response(
                    200,
                    gwz_transport::protocol::GitService::UploadPackAdvertisement,
                    "ok",
                )
            })
        }))
        .await;
        let port = server
            .url
            .rsplit(':')
            .next()
            .unwrap()
            .trim_end_matches("/repo");
        let proxy = CutProxy::start(port.parse().unwrap());
        let key = Key::https("localhost", proxy.port);
        let owner = Owner::new("session", "operation");
        let running = RunningPool::with_authority(
            pool::Config::default(),
            server.config(),
            Authority::new(8, 8),
        )
        .unwrap();
        let lease = running
            .client
            .checkout(
                key.clone(),
                owner.clone(),
                5_000,
                5_000,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        {
            let connection = lease.connection.clone().unwrap();
            let mut guard = connection.lock().await;
            let response = guard.sender.send_request(get()).await.unwrap();
            response.into_body().collect().await.unwrap();
        }
        lease.finish(Disposition::Reusable).unwrap();
        Self {
            running,
            proxy,
            key,
            owner,
            _server: server,
        }
    }
    /// Waits for the pool to hold no connection, and returns how long it took.
    async fn emptied(&self, within: Duration) -> Option<Duration> {
        let began = Instant::now();
        while self.running.client.pending() != 0 {
            if began.elapsed() >= within {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        Some(began.elapsed())
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Hyper's connection task ends when the server closes the idle socket, which
/// is the only thing that happens: nothing checks out, releases or shuts down.
#[test]
fn the_server_closing_an_idle_connection_wakes_the_supervisor() {
    runtime().block_on(async {
        let idle = Idle::new().await;
        assert_eq!(idle.running.client.pending(), 1);
        idle.proxy.cut_all();
        let took = idle.emptied(PROMPTLY).await;
        assert!(
            took.is_some(),
            "the idle connection the server closed was not disposed of within {PROMPTLY:?}"
        );
    });
}

/// A reused lease found dead is released by `adopt`, and its disposal is the
/// supervisor's: nothing else happens to the pool afterwards.
#[test]
fn a_dead_lease_released_by_adopt_is_disposed_of_at_once() {
    runtime().block_on(async {
        let idle = Idle::new().await;
        let pool = idle.running.client.clone();
        let mut checkout = pool
            .pool
            .checkout(Request::new(
                idle.key.clone(),
                Identity::Https,
                idle.owner.clone(),
            ))
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        let Poll::Ready(Ok(lease)) = pin!(&mut checkout).poll(&mut cx) else {
            panic!("the idle connection is leased at once");
        };
        idle.proxy.cut_all();
        let until = Instant::now() + Duration::from_secs(5);
        while pool.host.lock().unwrap().lost(&lease).is_none() {
            assert!(Instant::now() < until, "the host finds the connection dead");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(matches!(
            pool.adopt(lease, Instant::now()),
            Ok(Adopted::Dead)
        ));
        let took = idle.emptied(PROMPTLY).await;
        assert!(
            took.is_some(),
            "the connection adopt released was not disposed of within {PROMPTLY:?}"
        );
    });
}

/// A reused connection that dies at the exchange is replaced from the same
/// supervisor, which sleeps through the replacement's setup and wakes for it.
#[test]
fn a_fresh_retry_after_a_dead_lease_is_served_promptly() {
    runtime().block_on(async {
        let idle = Idle::new().await;
        let pool = idle.running.client.clone();
        let mut checkout = pool
            .pool
            .checkout(Request::new(
                idle.key.clone(),
                Identity::Https,
                idle.owner.clone(),
            ))
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        let Poll::Ready(Ok(lease)) = pin!(&mut checkout).poll(&mut cx) else {
            panic!("the idle connection is leased at once");
        };
        idle.proxy.cut_all();
        let until = Instant::now() + Duration::from_secs(5);
        while pool.host.lock().unwrap().lost(&lease).is_none() {
            assert!(Instant::now() < until, "the host finds the connection dead");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(matches!(
            pool.adopt(lease, Instant::now()),
            Ok(Adopted::Dead)
        ));
        let began = Instant::now();
        let fresh = tokio::time::timeout(
            Duration::from_secs(5),
            pool.checkout_scoped(
                idle.key.clone(),
                idle.owner.clone(),
                20_000,
                20_000,
                &CancellationToken::new(),
                None,
                true,
                true,
                None,
            ),
        )
        .await
        .expect("the fresh retry is served")
        .unwrap();
        assert!(!fresh.reused);
        assert!(
            began.elapsed() < PROMPTLY,
            "the fresh retry took {:?}",
            began.elapsed()
        );
        fresh.finish(Disposition::Discarded).unwrap();
    });
}
