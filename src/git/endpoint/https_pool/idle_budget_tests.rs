//! A reused lease found dead and replaced by a fresh connection, against the
//! host's job budget (dev-docs/GwzTransportIdleLossDesign.md §6.1 (a), adaptive
//! concurrency design §7.5): the retry's setup takes its place from the same
//! `Supervisor`, and every place is returned.
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
fn a_fresh_retry_after_a_dead_reused_lease_takes_its_place_from_the_hosts_supervisor() {
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
            let authority = Authority::new(8, 8);
            let supervisor = authority.supervisor().clone();
            let running =
                RunningPool::with_authority(pool::Config::default(), server.config(), authority)
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
            let mut checkout = pool
                .pool
                .checkout(Request::new(key.clone(), Identity::Https, owner.clone()))
                .unwrap();
            let mut cx = Context::from_waker(Waker::noop());
            let Poll::Ready(Ok(lease)) = pin!(&mut checkout).poll(&mut cx) else {
                panic!("the idle connection is leased at once");
            };
            proxy.cut_all();
            let deadline = Instant::now() + Duration::from_secs(5);
            while pool.host.lock().unwrap().lost(&lease).is_none() {
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
            assert_eq!(proxy.connections(), 2);
            // Two setups ran in all; neither the dead connection's place nor
            // the retry's was counted twice, and none is kept.
            assert!(supervisor.taken() <= 1, "the retry holds at most its own");
            fresh.finish(Disposition::Discarded).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while supervisor.taken() != 0 {
                assert!(Instant::now() < deadline, "every place was returned");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
}
