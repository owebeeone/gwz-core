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
    let connector = HttpConnector::new(
        server.config(),
        deadline_epoch,
        slots.clone(),
        crate::git::endpoint::agent_job::Supervisor::new(),
    );
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

/// The connector's resource reports how long its socket connect took, once
/// the connect has completed and not before (the settle time follows it).
#[test]
fn a_connection_reports_its_tcp_connect_time_once_the_socket_connects() {
    runtime().block_on(async {
        let server = server().await;
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().try_acquire_owned().unwrap();
        let mut connector = HttpConnector::new(
            server.config(),
            std::time::Instant::now(),
            slots,
            crate::git::endpoint::agent_job::Supervisor::new(),
        );
        let mut resource = connector
            .start(&key_of(&server), &pool::Identity::Https, None)
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        assert_eq!(resource.tcp_connect_ms(), None, "queued: nothing connected");
        drop(held);
        let until = Instant::now() + Duration::from_secs(10);
        while resource.poll_connected(&mut cx).is_pending() {
            assert!(Instant::now() < until, "never connected");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(resource.tcp_connect_ms().is_some_and(|ms| ms < 5_000));
    });
}

/// F4: a connection waiting for a setup slot has sent nothing to the server, so
/// its wait is local. It reports that it waits (`waiting_locally`), the pool
/// pauses its connect clock, and the open's allocation alone bounds it, ending
/// as a local failure (`job_budget_wait_tests`, `Budget::SetupSlot`).
#[test]
fn a_connection_waiting_for_a_setup_slot_reports_a_local_wait() {
    runtime().block_on(async {
        let server = server().await;
        let (mut connector, _slots, held) =
            connector_with_held_slot(&server, std::time::Instant::now());
        let mut resource = connector
            .start(&key_of(&server), &pool::Identity::Https, Some(40))
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        assert!(resource.waiting_locally());
        // Past its connect deadline the resource itself still waits: the pool,
        // not the resource, ends a wait, at the allocation.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        drop(held);
        let until = Instant::now() + Duration::from_secs(10);
        while resource.waiting_locally() {
            let _ = resource.poll_connected(&mut cx);
            assert!(Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(!resource.waiting_locally());
    });
}

/// Case 13's last row: one setup slot, 32 connections, and every connection
/// connects, each taking the slot in its turn.
#[test]
fn thirty_two_connections_through_one_setup_slot_all_connect() {
    runtime().block_on(async {
        let server = server().await;
        let slots = Arc::new(Semaphore::new(1));
        let mut connector = HttpConnector::new(
            server.config(),
            std::time::Instant::now(),
            slots.clone(),
            crate::git::endpoint::agent_job::Supervisor::new(),
        );
        let mut resources: Vec<_> = (0..32)
            .map(|_| {
                connector
                    .start(&key_of(&server), &pool::Identity::Https, None)
                    .expect("a busy setup slot queues the connection; it is not refused")
            })
            .collect();
        let mut cx = Context::from_waker(Waker::noop());
        let mut connected = [false; 32];
        let until = Instant::now() + Duration::from_secs(20);
        while connected.iter().any(|done| !done) {
            assert!(Instant::now() < until, "not every connection connected");
            for (index, resource) in resources.iter_mut().enumerate() {
                if !connected[index]
                    && let Poll::Ready(result) = resource.poll_connected(&mut cx)
                {
                    result.expect("a queued connection connects");
                    connected[index] = true;
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 32);
        for resource in &mut resources {
            while resource.poll_dispose(&mut cx, false).is_pending() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }
        assert_eq!(slots.available_permits(), 1);
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

mod throttle;
