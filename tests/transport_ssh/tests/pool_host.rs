#[path = "../../../src/git/endpoint/ssh_pool.rs"]
mod ssh_pool;

use gwz_transport::{
    pool::{Config, Identity, Key, Owner, Pool, Request},
    protocol::{Disposition, Effect, ErrorCode, Failure},
};
use ssh_pool::{Connector, PoolHost, Resource};
use std::{
    future::Future,
    io,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

#[derive(Default)]
struct State {
    opens: usize,
    disposed: usize,
    ready: bool,
    finish_close: bool,
    reusable: bool,
    fail_connect: bool,
    fail_dispose: bool,
    forced: usize,
}
struct Fake(Arc<Mutex<State>>, bool);
struct Factory(Arc<Mutex<State>>);
impl Connector for Factory {
    type Resource = Fake;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Fake, Failure> {
        self.0.lock().unwrap().opens += 1;
        Ok(Fake(self.0.clone(), false))
    }
}
impl Resource for Fake {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        let state = self.0.lock().unwrap();
        if state.fail_connect {
            Poll::Ready(Err(Failure {
                facts: None,
                code: ErrorCode::Io,
                effect: Effect::None,
            }))
        } else if state.ready {
            Poll::Ready(Ok(Some(Identity::Ambient)))
        } else {
            Poll::Pending
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        let mut state = self.0.lock().unwrap();
        if force {
            state.forced += 1;
        }
        if state.fail_dispose && !force {
            return Poll::Ready(Err(io::Error::other("scripted disposal error")));
        }
        if force || state.finish_close {
            if !self.1 {
                state.disposed += 1;
                self.1 = true;
            }
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn reusable(&self) -> bool {
        self.0.lock().unwrap().reusable
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        if !self.1 {
            self.0.lock().unwrap().disposed += 1;
        }
    }
}
fn request(operation: &str) -> Request {
    Request::new(
        Key::ssh("git", "host", 22),
        Identity::Ambient,
        Owner::new("session", operation),
    )
}
fn setup() -> (Pool, PoolHost<Factory>, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        ready: true,
        reusable: true,
        ..State::default()
    }));
    let (pool, host) = PoolHost::new(
        Config {
            total: 1,
            per_host: 1,
            per_user_host: 1,
            idle_timeout_ms: 60,
            cleanup_timeout_ms: 5,
            connect_timeout_ms: 10,
            ..Config::default()
        },
        Factory(state.clone()),
        10_000,
    )
    .unwrap();
    (pool, host, state)
}
fn tick(host: &mut PoolHost<Factory>, now: u64) {
    host.step(&mut Context::from_waker(Waker::noop()), now)
        .unwrap();
}
fn take(checkout: &mut gwz_transport::pool::Checkout) -> gwz_transport::pool::Lease {
    match pin!(checkout).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Ok(lease)) => lease,
        other => panic!("expected lease, pending={}", other.is_pending()),
    }
}

#[test]
fn healthy_operations_share_one_physical_connection_and_idle_reaping_waits_for_disposal() {
    let (pool, mut host, state) = setup();
    let mut first = pool.checkout(request("one")).unwrap();
    tick(&mut host, 10_000);
    let lease = take(&mut first);
    let id = lease.connection().unwrap();
    host.resource(&lease).unwrap();
    host.release(lease, Disposition::Reusable).unwrap();
    let clone = pool.clone();
    drop(pool);
    let mut next = clone.checkout(request("two")).unwrap();
    tick(&mut host, 10_059);
    let lease = take(&mut next);
    assert_eq!(lease.connection().unwrap(), id);
    assert_eq!(state.lock().unwrap().opens, 1);
    host.release(lease, Disposition::Reusable).unwrap();
    tick(&mut host, 10_118);
    assert_eq!(clone.counts().idle, 1);
    tick(&mut host, 10_119);
    assert_eq!(clone.counts().closing, 1);
    assert_eq!(state.lock().unwrap().disposed, 0);
    state.lock().unwrap().finish_close = true;
    tick(&mut host, 10_120);
    assert_eq!(clone.counts().total(), 0);
    assert_eq!(state.lock().unwrap().disposed, 1);
}

#[test]
fn rejected_reuse_discards_and_holds_capacity_until_forced_disposal() {
    let (pool, mut host, state) = setup();
    let mut first = pool.checkout(request("one")).unwrap();
    tick(&mut host, 10_000);
    let lease = take(&mut first);
    state.lock().unwrap().reusable = false;
    assert!(host.release(lease, Disposition::Reusable).is_err());
    let mut next = pool.checkout(request("two")).unwrap();
    tick(&mut host, 10_001);
    assert!(
        pin!(&mut next)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(pool.counts().closing, 1);
    tick(&mut host, 10_005);
    tick(&mut host, 10_006);
    assert_eq!(state.lock().unwrap().disposed, 1);
    assert_eq!(state.lock().unwrap().forced, 1);
    let lease = take(&mut next);
    assert_eq!(state.lock().unwrap().opens, 2);
    drop(lease);
}

#[test]
fn cancel_connect_never_accepts_late_success_and_keeps_capacity_until_abort() {
    let (pool, mut host, state) = setup();
    state.lock().unwrap().ready = false;
    let checkout = pool.checkout(request("cancelled")).unwrap();
    tick(&mut host, 10_000);
    drop(checkout);
    state.lock().unwrap().ready = true;
    tick(&mut host, 10_001);
    assert_eq!(pool.counts().opening, 1);
    assert_eq!(pool.counts().idle, 0);
    tick(&mut host, 10_005);
    assert_eq!(pool.counts().total(), 0);
    assert_eq!(state.lock().unwrap().disposed, 1);
}

#[test]
fn failed_connect_disposes_before_ack_and_owner_drop_terminates_all_resources() {
    let (pool, mut host, state) = setup();
    state.lock().unwrap().fail_connect = true;
    let mut checkout = pool.checkout(request("failed")).unwrap();
    tick(&mut host, 10_000);
    assert_eq!(pool.counts().opening, 1);
    assert!(
        pin!(&mut checkout)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    state.lock().unwrap().finish_close = true;
    tick(&mut host, 10_001);
    assert!(matches!(
        pin!(&mut checkout).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(_))
    ));
    assert_eq!(pool.counts().total(), 0);
    state.lock().unwrap().fail_connect = false;
    let mut second = pool.checkout(request("live")).unwrap();
    tick(&mut host, 10_002);
    let lease = take(&mut second);
    drop(host);
    assert_eq!(state.lock().unwrap().disposed, 2);
    assert!(lease.connection().is_err());
}

#[test]
fn exact_connect_deadline_beats_ready_and_disposal_error_keeps_capacity() {
    let (pool, mut host, state) = setup();
    state.lock().unwrap().ready = false;
    let mut checkout = pool.checkout(request("deadline")).unwrap();
    tick(&mut host, 10_000);
    assert_eq!(host.next_deadline(), Some(10_010));
    state.lock().unwrap().ready = true;
    state.lock().unwrap().fail_dispose = true;
    tick(&mut host, 10_010);
    assert!(matches!(
        pin!(&mut checkout).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(_))
    ));
    assert_eq!(pool.counts().opening, 1);
    assert_eq!(state.lock().unwrap().disposed, 0);
    assert_eq!(
        host.take_disposal_error().unwrap().to_string(),
        "scripted disposal error"
    );
    assert!(host.take_disposal_error().is_none());
    tick(&mut host, 10_011);
    assert_eq!(state.lock().unwrap().disposed, 1);
    pool.shutdown();
    tick(&mut host, 10_012);
    assert!(host.shutdown_complete());
}
