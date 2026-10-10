//! A reused lease the host finds dead before any byte is released for a fresh
//! connection (dev-docs/GwzTransportIdleLossDesign.md §6.1 (a)). The race is
//! forced: the pool leases the idle connection, and only then does the server
//! close it, before the lease is adopted.
use super::*;
use crate::git::endpoint::{cut_proxy::CutProxy, https_fixture::Server};
use http_body_util::BodyExt;
use std::{
    future::Future,
    pin::pin,
    task::{Poll, Waker},
};

fn get() -> hyper::Request<https_connection::RequestBody> {
    let (sender, rx) = tokio::sync::mpsc::channel(1);
    drop(sender);
    hyper::Request::get("/repo")
        .header(hyper::header::HOST, "localhost")
        .body(https_connection::RequestBody { rx })
        .unwrap()
}

#[test]
fn a_reused_lease_found_dead_is_released_for_a_fresh_one() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
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
            let cancel = CancellationToken::new();
            let running = RunningPool::with_authority(
                pool::Config::default(),
                server.config(),
                Authority::new(8, 8),
            )
            .unwrap();
            let pool = running.client.clone();
            let lease = pool
                .checkout(key.clone(), owner.clone(), 5_000, 5_000, &cancel)
                .await
                .unwrap();
            {
                let connection = lease.connection.clone().unwrap();
                let mut guard = connection.lock().await;
                let response = guard.sender.send_request(get()).await.unwrap();
                response.into_body().collect().await.unwrap();
            }
            lease.finish(Disposition::Reusable).unwrap();
            // The pool leases the idle connection before anything is wrong.
            let mut checkout = pool
                .pool
                .checkout(Request::new(key.clone(), Identity::Https, owner.clone()))
                .unwrap();
            let mut cx = Context::from_waker(Waker::noop());
            let Poll::Ready(Ok(lease)) = pin!(&mut checkout).poll(&mut cx) else {
                panic!("the idle connection is leased at once");
            };
            // Then the server closes it, and the host finds it dead.
            proxy.cut_all();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let lost = pool.host.lock().unwrap().lost(&lease);
                if lost.is_some() {
                    assert_eq!(lost, Some(true));
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "the host finds the connection dead"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(matches!(
                pool.adopt(lease, Instant::now()),
                Ok(Adopted::Dead)
            ));
            let fresh = pool
                .checkout(key, owner, 5_000, 5_000, &cancel)
                .await
                .unwrap();
            assert!(!fresh.reused);
            fresh.finish(Disposition::Discarded).unwrap();
            assert_eq!(proxy.connections(), 2);
        });
}

/// Two idle connections, the older of which the server has closed while the
/// host is still disposing of it (something holds the connection, so its
/// disposal is pending and the pool counts it Idle): a checkout leases the
/// dead one first.
struct OneDead {
    _server: Server,
    proxy: CutProxy,
    pool: HttpsPool,
    _running: RunningPool,
    key: Key,
    owner: Owner,
    cancel: CancellationToken,
    /// Keeps the dead connection's disposal pending.
    _held: Arc<tokio::sync::Mutex<https_connection::Connection>>,
}
async fn one_dead_of_two() -> OneDead {
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
    let cancel = CancellationToken::new();
    let running = RunningPool::with_authority(
        pool::Config::default(),
        server.config(),
        Authority::new(8, 8),
    )
    .unwrap();
    let pool = running.client.clone();
    // Two exchanges at once, so that the pool opens two connections.
    let older = pool
        .checkout(key.clone(), owner.clone(), 5_000, 5_000, &cancel)
        .await
        .unwrap();
    let newer = pool
        .checkout(key.clone(), owner.clone(), 5_000, 5_000, &cancel)
        .await
        .unwrap();
    let held = older.connection.clone().unwrap();
    for lease in [older, newer] {
        let connection = lease.connection.clone().unwrap();
        {
            let mut guard = connection.lock().await;
            let response = guard.sender.send_request(get()).await.unwrap();
            response.into_body().collect().await.unwrap();
        }
        drop(connection);
        lease.finish(Disposition::Reusable).unwrap();
    }
    assert_eq!(pool.pool.counts().idle, 2);
    proxy.cut_oldest();
    // Let the older connection's driver end.
    tokio::time::sleep(Duration::from_millis(100)).await;
    OneDead {
        _server: server,
        proxy,
        pool,
        _running: running,
        key,
        owner,
        cancel,
        _held: held,
    }
}

/// The retry after a dead lease is fresh, so it leaves the other idle
/// connection alone (P3-2 of the idle-loss State review), and is the open's
/// retry (P3-3).
#[test]
fn the_retry_after_a_dead_lease_is_fresh_and_leaves_another_idle_connection_alone() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let dead = one_dead_of_two().await;
            let fresh = dead
                .pool
                .checkout_scoped(
                    dead.key.clone(),
                    dead.owner.clone(),
                    5_000,
                    5_000,
                    &dead.cancel,
                    None,
                    false,
                    true,
                    None,
                )
                .await
                .unwrap();
            assert!(!fresh.reused, "the retry took the other idle connection");
            assert!(fresh.retried, "the lease is the open's retry");
            assert_eq!(dead.proxy.connections(), 3);
            assert_eq!(
                dead.pool.pool.counts().idle,
                1,
                "the other connection is idle"
            );
            fresh.finish(Disposition::Discarded).unwrap();
        });
}

/// An open that has used its retry is not retried again (P3-3): the dead
/// lease fails it.
#[test]
fn a_dead_lease_is_not_retried_by_an_open_that_has_retried() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let dead = one_dead_of_two().await;
            let failed = dead
                .pool
                .checkout_scoped(
                    dead.key.clone(),
                    dead.owner.clone(),
                    5_000,
                    5_000,
                    &dead.cancel,
                    None,
                    false,
                    false,
                    None,
                )
                .await;
            let (failure, _) = failed.err().expect("no second retry");
            assert_eq!(failure.code, ErrorCode::Io);
            assert_eq!(dead.proxy.connections(), 2);
        });
}
