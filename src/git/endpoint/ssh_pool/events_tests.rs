//! The pool host reports what happens to each connection, in the order it
//! happened, to the one observer of its pool (adaptive concurrency design
//! §4.9, F10): setup started, connected, ended by the server, retired,
//! closing, disposed, closed by the server while idle. It is the one place
//! that sees the physical events of both endpoints in order.
use super::*;
use gwz_transport::pool::{Config, ConnectionId, Owner, Request};
use std::{pin::pin, sync::Mutex};

type Log = Arc<Mutex<Vec<(u64, Seen, u64)>>>;

struct Recorder(Log);
impl Observer for Recorder {
    fn seen(&self, _: &Key, connection: ConnectionId, seen: Seen, now: u64) {
        self.0
            .lock()
            .unwrap()
            .push((connection.sequence(), seen, now));
    }
}

#[derive(Default)]
struct Script {
    /// Fail the next connector start outright.
    refuse_start: bool,
    /// The next resource fails its connect.
    fail_connect: bool,
    /// The resources are ready once this is set.
    ready: bool,
    lost: bool,
}
struct Fake(Arc<Mutex<Script>>);
struct Factory(Arc<Mutex<Script>>);
impl Connector for Factory {
    type Resource = Fake;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Fake, Failure> {
        if self.0.lock().unwrap().refuse_start {
            return Err(failure(ErrorCode::Capacity));
        }
        Ok(Fake(self.0.clone()))
    }
}
fn failure(code: ErrorCode) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        facts: None,
        code,
        effect: Effect::None,
    }
}
impl Resource for Fake {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        let script = self.0.lock().unwrap();
        if script.fail_connect {
            Poll::Ready(Err(failure(ErrorCode::Io)))
        } else if script.ready {
            Poll::Ready(Ok(Some(Identity::Ambient)))
        } else {
            Poll::Pending
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        true
    }
    fn poll_idle_lost(&mut self, _: &mut Context<'_>) -> Poll<()> {
        if self.0.lock().unwrap().lost {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

fn rig(script: Script) -> (Pool, PoolHost<Factory>, Arc<Mutex<Script>>, Log) {
    let script = Arc::new(Mutex::new(script));
    let log = Log::default();
    let (pool, mut host) = PoolHost::new(Config::default(), Factory(script.clone()), 0).unwrap();
    host.set_observer(Arc::new(Recorder(log.clone())));
    (pool, host, script, log)
}
fn request() -> Request {
    Request::new(
        Key::ssh("git", "host", 22),
        Identity::Ambient,
        Owner::new("session", "operation"),
    )
}
fn tick(host: &mut PoolHost<Factory>, now: u64) {
    host.step(&mut Context::from_waker(Waker::noop()), now)
        .unwrap();
}
fn lease(checkout: &mut gwz_transport::pool::Checkout) -> Lease {
    match pin!(checkout).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Ok(lease)) => lease,
        other => panic!("expected a lease, pending={}", other.is_pending()),
    }
}
fn seen(log: &Log) -> Vec<(Seen, u64)> {
    log.lock()
        .unwrap()
        .iter()
        .map(|(_, seen, now)| (*seen, *now))
        .collect()
}

#[test]
fn a_connection_that_connects_is_started_then_connected() {
    let (pool, mut host, _, log) = rig(Script {
        ready: true,
        ..Script::default()
    });
    let mut checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    let _lease = lease(&mut checkout);
    assert_eq!(
        seen(&log),
        [(Seen::Started { clocked: true }, 10), (Seen::Connected, 10)]
    );
}

#[test]
fn a_connect_the_server_refused_is_started_then_ended_when_it_is_disposed() {
    let (pool, mut host, _, log) = rig(Script {
        fail_connect: true,
        ready: true,
        ..Script::default()
    });
    let mut checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    tick(&mut host, 11);
    assert!(matches!(
        pin!(&mut checkout).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(_))
    ));
    assert_eq!(
        seen(&log),
        [
            (Seen::Started { clocked: true }, 10),
            (Seen::SetupEnded, 11)
        ]
    );
}

#[test]
fn a_start_the_connector_refused_reports_nothing() {
    // A local refusal: no socket was begun, so no connection was started.
    let (pool, mut host, _, log) = rig(Script {
        refuse_start: true,
        ..Script::default()
    });
    let _checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    assert_eq!(seen(&log), []);
}

#[test]
fn a_released_discard_is_closing_when_the_host_acts_and_disposed_when_it_is_gone() {
    let (pool, mut host, _, log) = rig(Script {
        ready: true,
        ..Script::default()
    });
    let mut checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    let held = lease(&mut checkout);
    host.release(held, Disposition::Discarded).unwrap();
    // The pool decided at the release; the host acts, and reports, on its
    // next turn: the server is not affected before then.
    assert_eq!(seen(&log).len(), 2);
    tick(&mut host, 20);
    let events = seen(&log);
    assert_eq!(
        &events[2..],
        [(Seen::Closing, 20), (Seen::Disposed, 20)],
        "closing is reported before the disposal it leads to"
    );
}

#[test]
fn an_idle_connection_the_server_closed_is_reported_as_the_servers() {
    let (pool, mut host, script, log) = rig(Script {
        ready: true,
        ..Script::default()
    });
    let mut checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    let held = lease(&mut checkout);
    host.release(held, Disposition::Reusable).unwrap();
    script.lock().unwrap().lost = true;
    tick(&mut host, 30);
    let events = seen(&log);
    assert_eq!(&events[2..], [(Seen::ServerClosed, 30)]);
}

#[test]
fn a_connect_the_client_cancelled_is_retired() {
    let (pool, mut host, _, log) = rig(Script::default());
    let checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    drop(checkout);
    tick(&mut host, 20);
    tick(&mut host, 21);
    assert_eq!(
        seen(&log),
        [(Seen::Started { clocked: true }, 10), (Seen::Retired, 20)]
    );
}

#[test]
fn every_event_of_a_connection_carries_its_pool_id() {
    let (pool, mut host, _, log) = rig(Script {
        ready: true,
        ..Script::default()
    });
    let mut checkout = pool.checkout(request()).unwrap();
    tick(&mut host, 10);
    let held = lease(&mut checkout);
    let id = held.connection().unwrap().sequence();
    host.release(held, Disposition::Discarded).unwrap();
    tick(&mut host, 20);
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|(connection, _, _)| *connection == id)
    );
}
