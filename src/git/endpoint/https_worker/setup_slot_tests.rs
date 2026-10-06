//! The connector's eight setup slots bound the blocking jobs that resolve a
//! host and build its TLS configuration. A connection past them waits for a
//! slot inside its connect budget; the per-host and total ceilings, which the
//! operation's `--max-per-host` sets, are what refuse or queue connections.
use super::*;
use crate::git::endpoint::{
    https_connection::HttpConnector,
    https_fixture::{Server, response},
    ssh_pool::{Connector, Resource},
};
use std::task::{Context, Poll, Waker};
use tokio::sync::Semaphore;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn server() -> Server {
    Server::start(Arc::new(|_| {
        Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
    }))
    .await
}

fn key_of(server: &Server) -> Key {
    let destination = Destination::parse(&server.url).unwrap();
    Key::https(destination.host(), destination.port())
}

/// A connector whose only setup slot the test holds, and that held slot.
fn connector_with_held_slot(
    server: &Server,
    deadline_epoch: std::time::Instant,
) -> (
    HttpConnector,
    Arc<Semaphore>,
    tokio::sync::OwnedSemaphorePermit,
) {
    let slots = Arc::new(Semaphore::new(1));
    let held = slots.clone().try_acquire_owned().unwrap();
    let connector = HttpConnector {
        config: server.config(),
        epoch: deadline_epoch,
        setup_slots: slots.clone(),
    };
    (connector, slots, held)
}

#[test]
fn a_connection_past_the_setup_slots_waits_for_one_instead_of_failing() {
    runtime().block_on(async {
        let server = server().await;
        let (mut connector, slots, held) =
            connector_with_held_slot(&server, std::time::Instant::now());
        let mut resource = connector
            .start(&key_of(&server), &pool::Identity::Https, None)
            .expect("a busy setup slot queues the connection; it is not refused");
        let mut cx = Context::from_waker(Waker::noop());
        assert!(
            matches!(resource.poll_connected(&mut cx), Poll::Pending),
            "the connection must wait while no setup slot is free"
        );
        assert_eq!(slots.available_permits(), 0);
        drop(held);
        let until = Instant::now() + Duration::from_secs(10);
        let connected = loop {
            if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
                break result;
            }
            assert!(Instant::now() < until, "the freed slot was never taken");
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        assert!(connected.is_ok(), "{connected:?}");
        let disposed = loop {
            if let Poll::Ready(result) = resource.poll_dispose(&mut cx, false) {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        assert!(disposed.is_ok());
        assert_eq!(slots.available_permits(), 1);
    });
}

#[test]
fn a_connection_waiting_for_a_setup_slot_times_out_at_its_connect_deadline() {
    runtime().block_on(async {
        let server = server().await;
        let (mut connector, _slots, _held) =
            connector_with_held_slot(&server, std::time::Instant::now());
        let mut resource = connector
            .start(&key_of(&server), &pool::Identity::Https, Some(40))
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        tokio::time::sleep(Duration::from_millis(80)).await;
        let Poll::Ready(Err(failed)) = resource.poll_connected(&mut cx) else {
            panic!("a connection still waiting at its deadline must fail");
        };
        assert_eq!(failed.code, ErrorCode::Timeout);
        assert_eq!(failed.setup_cause, Some(SetupFailureCause::Aggregate));
    });
}

#[test]
fn disposing_a_connection_that_waits_for_a_setup_slot_leaves_the_slot_free() {
    runtime().block_on(async {
        let server = server().await;
        let (mut connector, slots, held) =
            connector_with_held_slot(&server, std::time::Instant::now());
        let mut resource = connector
            .start(&key_of(&server), &pool::Identity::Https, None)
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        assert!(matches!(
            resource.poll_dispose(&mut cx, false),
            Poll::Ready(Ok(()))
        ));
        drop(held);
        assert_eq!(
            slots.available_permits(),
            1,
            "a disposed waiter must not keep the slot it queued for"
        );
    });
}

/// The measured failure: 32 connections wanted at once, the default per-host
/// ceiling, and 24 of them refused with `Capacity` by the eight setup slots.
#[test]
fn thirty_two_concurrent_connections_all_succeed_at_the_default_per_host_ceiling() {
    runtime().block_on(async {
        let server = server().await;
        let config = pool::Config::default();
        assert_eq!(config.per_host, 32);
        let mut endpoint = Endpoint::new(server.config(), None, config).unwrap();
        let pool = endpoint.client.pool.clone();
        let key = key_of(&server);
        let cancel = CancellationToken::new();
        let tasks: Vec<_> = (0..32)
            .map(|n| {
                let (pool, key, cancel) = (pool.clone(), key.clone(), cancel.clone());
                tokio::spawn(async move {
                    pool.checkout(
                        key,
                        Owner::new("session", format!("operation-{n}")),
                        20_000,
                        20_000,
                        &cancel,
                    )
                    .await
                })
            })
            .collect();
        let mut leases = Vec::new();
        for task in tasks {
            match task.await.unwrap() {
                Ok(lease) => leases.push(lease),
                Err((failed, _)) => panic!("a connection was refused: {:?}", failed.code),
            }
        }
        assert_eq!(leases.len(), 32);
        assert_eq!(server.connections.load(Ordering::SeqCst), 32);
        for lease in leases {
            lease.finish(Disposition::Discarded).unwrap();
        }
        assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
    });
}
