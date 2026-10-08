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
