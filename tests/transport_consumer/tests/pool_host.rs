use gwz_transport::{
    pool::{Action, Config, Identity, Key, Owner, Pool, Request},
    protocol::{Disposition, Effect, ErrorCode, Failure},
};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn request(host: &str) -> Request {
    Request::new(
        Key::ssh("git", host, 22),
        Identity::Ambient,
        Owner::new("consumer-session", "consumer-operation"),
    )
}

#[test]
fn host_clock_keeps_large_nonzero_origin_for_connect_budget() {
    let (pool, mut driver) = Pool::new(Config::default()).unwrap();
    driver.advance(4_000_000);
    let mut checkout = pool.checkout(request("host")).unwrap();
    let mut context = Context::from_waker(Waker::noop());
    assert!(pin!(&mut checkout).poll(&mut context).is_pending());
    let Poll::Ready(Some(Action::Connect {
        connection,
        network_deadline,
        ..
    })) = pin!(driver.next_action()).poll(&mut context)
    else {
        panic!("expected connect action");
    };
    assert_eq!(network_deadline, Some(4_010_000));
    driver.advance(4_009_999);
    assert!(pin!(&mut checkout).poll(&mut context).is_pending());
    assert!(pin!(driver.next_action()).poll(&mut context).is_pending());
    driver.advance(4_010_000);
    assert!(matches!(
        pin!(&mut checkout).poll(&mut context),
        Poll::Ready(Err(gwz_transport::pool::Error::ConnectTimeout))
    ));
    driver
        .connected(
            connection,
            Err(Failure {
                code: ErrorCode::Io,
                effect: Effect::None,
                facts: None,
            }),
        )
        .unwrap();
    assert_eq!(pool.counts().total(), 0);
}

#[test]
fn periodic_tick_services_new_earlier_allocation_deadline_while_driver_waits() {
    let (pool, mut driver) = Pool::new(Config {
        per_user_host: 1,
        per_host: 1,
        total: 1,
        ..Config::default()
    })
    .unwrap();
    driver.advance(10_000);
    let mut first = pool.checkout(request("first")).unwrap();
    let mut context = Context::from_waker(Waker::noop());
    assert!(pin!(&mut first).poll(&mut context).is_pending());
    let Poll::Ready(Some(Action::Connect { connection, .. })) =
        pin!(driver.next_action()).poll(&mut context)
    else {
        panic!("expected first connect action");
    };
    driver
        .connected(connection, Ok(Some(Identity::Ambient)))
        .unwrap();
    let lease = match pin!(&mut first).poll(&mut context) {
        Poll::Ready(Ok(lease)) => lease,
        _ => panic!("unexpected first checkout result"),
    };
    let long_wait = pool.checkout(request("first")).unwrap();
    assert_eq!(driver.next_deadline(), Some(40_000));
    let mut waiting;
    {
        let mut action = pin!(driver.next_action());
        assert!(action.as_mut().poll(&mut context).is_pending());
        waiting = pool
            .checkout(Request {
                allocation_timeout_ms: Some(25),
                ..request("first")
            })
            .unwrap();
        assert!(pin!(&mut waiting).poll(&mut context).is_pending());
        assert!(action.as_mut().poll(&mut context).is_pending());
        // The independent host timer cancels its pending action wait on a tick.
    }
    assert_eq!(driver.next_deadline(), Some(10_025));
    driver.advance(10_024);
    assert!(pin!(&mut waiting).poll(&mut context).is_pending());
    driver.advance(10_025);
    assert!(matches!(
        pin!(&mut waiting).poll(&mut context),
        Poll::Ready(Err(gwz_transport::pool::Error::AllocationTimeout))
    ));
    drop(long_wait);
    lease.release(Disposition::Discarded).unwrap();
    assert!(matches!(
        pin!(driver.next_action()).poll(&mut context),
        Poll::Ready(Some(Action::Close { .. }))
    ));
    driver.closed(connection).unwrap();
}

#[test]
fn final_pool_clone_drop_shuts_down_live_lease_for_host_cleanup() {
    let (pool, mut driver) = Pool::new(Config::default()).unwrap();
    let retained = pool.clone();
    let mut context = Context::from_waker(Waker::noop());
    let mut checkout = pool.checkout(request("host")).unwrap();
    let Poll::Ready(Some(Action::Connect { connection, .. })) =
        pin!(driver.next_action()).poll(&mut context)
    else {
        panic!("expected connect action");
    };
    driver
        .connected(connection, Ok(Some(Identity::Ambient)))
        .unwrap();
    let lease = match pin!(&mut checkout).poll(&mut context) {
        Poll::Ready(Ok(lease)) => lease,
        _ => panic!("unexpected checkout result"),
    };
    drop(pool);
    assert_eq!(lease.connection().unwrap(), connection);
    drop(retained);
    assert_eq!(lease.connection(), Err(gwz_transport::pool::Error::Stale));
    assert!(matches!(
        pin!(driver.next_action()).poll(&mut context),
        Poll::Ready(Some(Action::Close { connection: id, .. })) if id == connection
    ));
    driver.closed(connection).unwrap();
    assert!(matches!(
        pin!(driver.next_action()).poll(&mut context),
        Poll::Ready(None)
    ));
}
