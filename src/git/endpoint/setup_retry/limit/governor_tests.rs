//! The governor over a real pool and pool host, on a hand-set clock: what
//! the endpoints rely on (§4.5, §4.9, §10.2 cases 21, 41, 42 and 51).
use super::{
    control::Ruling,
    filter::Signal::{Suspect, Throttle},
    fsm::State,
    governor::{Conn, Governor, Scoped},
    timer::Spread,
};
use crate::git::endpoint::ssh_pool::{Connector, PoolHost, Resource};
use gwz_transport::{
    pool::{Checkout, Config, Identity, Key, Lease, Owner, Pool, Request},
    protocol::{Disposition, Effect, ErrorCode, Failure},
};
use std::{
    future::Future,
    io,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

#[derive(Default)]
struct Script {
    ready: bool,
    /// Each new connection takes the next entry: true refuses it.
    refuse: Vec<bool>,
    opened: usize,
    /// The peer closed the connection while it was idle.
    lost: bool,
}
struct Fake {
    script: Arc<Mutex<Script>>,
    refused: bool,
}
struct Factory(Arc<Mutex<Script>>);
impl Connector for Factory {
    type Resource = Fake;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Fake, Failure> {
        let mut script = self.0.lock().unwrap();
        let refused = script.refuse.get(script.opened).copied().unwrap_or(false);
        script.opened += 1;
        Ok(Fake {
            script: self.0.clone(),
            refused,
        })
    }
}
impl Resource for Fake {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        if !self.script.lock().unwrap().ready {
            Poll::Pending
        } else if self.refused {
            Poll::Ready(Err(Failure {
                detail: None,
                setup_cause: None,
                facts: None,
                code: ErrorCode::Io,
                effect: Effect::None,
            }))
        } else {
            Poll::Ready(Ok(Some(Identity::Https)))
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        true
    }
    fn poll_idle_lost(&mut self, _: &mut Context<'_>) -> Poll<()> {
        if self.script.lock().unwrap().lost {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct Rig {
    pool: Pool,
    host: PoolHost<Factory>,
    governor: Governor,
    script: Arc<Mutex<Script>>,
    now: u64,
    /// The operation the rig's helpers speak for.
    scope: &'static str,
}
fn key() -> Key {
    Key::https("host", 443)
}
impl Rig {
    fn new(ceiling: usize, adaptive: bool) -> Self {
        let script = Arc::new(Mutex::new(Script {
            ready: true,
            ..Script::default()
        }));
        let (pool, mut host) = PoolHost::new(
            Config {
                per_host: ceiling,
                per_user_host: ceiling,
                ..Config::default()
            },
            Factory(script.clone()),
            0,
        )
        .unwrap();
        let governor = Governor::new(pool.control(), ceiling, adaptive, || Spread::fixed(1_000));
        host.set_observer(Arc::new(governor.clone()));
        governor.begin_operation("op", ceiling, adaptive, 0);
        Self {
            pool,
            host,
            governor,
            script,
            now: 0,
            scope: "op",
        }
    }
    fn op(&self) -> Scoped {
        self.governor.scoped(self.scope)
    }
    fn tick(&mut self, now: u64) {
        self.now = now;
        self.host
            .step(&mut Context::from_waker(Waker::noop()), now)
            .unwrap();
        self.governor.tick(now);
        self.host
            .step(&mut Context::from_waker(Waker::noop()), now)
            .unwrap();
    }
    fn request(&self) -> Request {
        let mut request = Request::new(key(), Identity::Https, Owner::new("session", "operation"));
        request.fresh = true;
        request
    }
    /// A new connection, leased. The pool host runs a turn before and after.
    fn open(&mut self) -> (Lease, Conn) {
        let mut checkout = self.pool.checkout(self.request()).unwrap();
        self.tick(self.now);
        let lease = take(&mut checkout);
        let id = Conn::of(lease.connection().unwrap());
        (lease, id)
    }
    fn view(&self) -> super::governor::View {
        self.op().view(&key(), self.now).unwrap()
    }
    fn site(&self) -> gwz_transport::pool::Site {
        key().site()
    }
}
fn take(checkout: &mut Checkout) -> Lease {
    match pin!(checkout).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Ok(lease)) => lease,
        Poll::Ready(Err(error)) => panic!("checkout failed: {error:?}"),
        Poll::Pending => panic!("checkout pending"),
    }
}

#[test]
fn a_new_key_is_saturated_and_tells_the_pool_its_ceiling_and_no_settle_time() {
    let mut rig = Rig::new(8, true);
    let (_lease, _) = rig.open();
    let view = rig.view();
    assert_eq!(
        (view.state, view.n, view.pool_limit),
        (State::Saturated, 8, 8)
    );
    assert_eq!(rig.pool.limit(&rig.site()), Some(8));
    assert_eq!(rig.op().admission(&key(), 0).target, 8);
    assert!(rig.op().admission(&key(), 0).gate_open);
}

#[test]
fn https_is_connected_when_its_first_exchange_is_answered_not_when_tls_is_up() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    assert_eq!(rig.view().connected, 0, "TLS up is not set up (§4.1)");
    assert_eq!(rig.view().possible, 1);
    rig.op().answered(&key(), connection, 5);
    assert_eq!(rig.view().connected, 1);
    drop(lease);
}

#[test]
fn a_throttle_lowers_the_pool_limit_to_n_and_starts_the_settle_time() {
    // Three answered, the fourth throttled: N = min(Connected 3, hi 3) = 3.
    let mut rig = Rig::new(8, true);
    let mut leases = Vec::new();
    for now in 1..=3 {
        rig.now = now;
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, now);
        leases.push(lease);
    }
    let (fourth, connection) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, None, false, 10);
    assert_eq!(ruling, Some(Ruling::Overload { n: 3 }));
    let view = rig.view();
    assert_eq!((view.state, view.n, view.pool_limit), (State::Stable, 3, 3));
    assert_eq!(rig.pool.limit(&rig.site()), Some(3));
    assert_eq!(view.settle_ms, 250);
    drop(fourth);
}

#[test]
fn outside_saturated_a_discarded_connections_slot_is_held_for_the_settle_time() {
    // Case 21 (R1 prevented): five answered, the sixth throttled, so N = 5.
    // A connection closes; its replacement cannot start inside the settle
    // time, though the pool has room for it the moment the close is done.
    let mut rig = Rig::new(8, true);
    let mut held = Vec::new();
    for now in 1..=5 {
        rig.now = now;
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, now);
        held.push(lease);
    }
    let (sixth, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, None, false, 10);
    assert_eq!(rig.view().n, 5);
    rig.host.release(sixth, Disposition::Discarded).unwrap();
    rig.host
        .release(held.pop().unwrap(), Disposition::Discarded)
        .unwrap();
    rig.tick(20);
    let mut next = rig.pool.checkout(rig.request()).unwrap();
    rig.tick(20);
    let pending = |next: &mut Checkout| {
        matches!(
            pin!(next).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        )
    };
    assert!(pending(&mut next), "no replacement inside Ts");
    rig.tick(20 + 249);
    assert!(pending(&mut next));
    rig.tick(20 + 250);
    let _lease = take(&mut next);
    assert_eq!(rig.view().possible, 5);
}

#[test]
fn a_long_retry_after_holds_the_gate_and_its_end_discards_the_idle_connections_in_one_step() {
    // Case 51 at the governor: ten idle connections, a 429 with Retry-After.
    let mut rig = Rig::new(32, false);
    let mut idle = Vec::new();
    for _ in 0..10 {
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, 0);
        idle.push((lease, connection));
    }
    for (lease, _) in idle {
        rig.host.release(lease, Disposition::Reusable).unwrap();
    }
    rig.tick(1_000);
    assert_eq!(rig.pool.limit(&rig.site()), Some(32));
    let (thrower, connection) = rig.open();
    // A request-level refusal with Retry-After: 10 s sets the hold.
    rig.op()
        .refused(&key(), connection, Throttle, Some(10_000), false, 1_000);
    assert!(!rig.op().admission(&key(), 1_000).gate_open);
    assert!(!rig.op().exchange_may_begin(&key(), 1_000));
    rig.host.release(thrower, Disposition::Discarded).unwrap();
    rig.tick(10_999);
    assert!(!rig.op().admission(&key(), 10_999).gate_open);
    // The hold ends: ten Closing(Discarded) in one turn, no eviction, and
    // only then does the gate open.
    let before = rig.pool.limit(&rig.site());
    rig.tick(11_000);
    assert!(rig.op().admission(&key(), 11_000).gate_open);
    assert_eq!(before, rig.pool.limit(&rig.site()));
    assert_eq!(rig.view().connected, 0, "all ten left Connected at once");
}

#[test]
fn at_max_retries_zero_a_throttle_sets_the_hold_and_lowers_nothing() {
    let mut rig = Rig::new(8, false);
    let (lease, connection) = rig.open();
    rig.op().answered(&key(), connection, 0);
    let (second, other) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), other, Throttle, Some(500), false, 5);
    assert_eq!(ruling, Some(Ruling::HoldOnly));
    let view = rig.view();
    assert_eq!(
        (view.state, view.n, view.pool_limit),
        (State::Saturated, 8, 8)
    );
    assert_eq!(rig.pool.settle(&rig.site()), 0);
    drop((lease, second));
}

#[test]
fn a_leased_exchange_that_is_refused_is_judged_on_its_own_window() {
    // R10: eight answered; the exchange on the third is throttled.
    let mut rig = Rig::new(32, true);
    let mut held = Vec::new();
    for now in 0..8 {
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, now);
        held.push((lease, connection));
    }
    let third = held[2].1;
    assert!(rig.op().exchange_begins(&key(), third, 100));
    let ruling = rig.op().refused(&key(), third, Throttle, None, false, 101);
    assert_eq!(
        ruling,
        Some(Ruling::Overload { n: 7 }),
        "min(Connected 8, hi 7)"
    );
}

#[test]
fn a_leased_exchange_may_not_begin_during_a_hold_and_a_successful_one_changes_nothing() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.op().answered(&key(), connection, 0);
    assert!(rig.op().exchange_begins(&key(), connection, 1));
    rig.op().answered(&key(), connection, 2);
    assert_eq!(rig.view().connected, 1);
    let (second, other) = rig.open();
    rig.op()
        .refused(&key(), other, Throttle, Some(3_000), false, 3);
    assert!(!rig.op().exchange_begins(&key(), connection, 4));
    drop((lease, second));
}

#[test]
fn a_hold_is_a_deadline_the_endpoint_wakes_for() {
    let mut rig = Rig::new(8, false);
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(5_000), false, 100);
    assert_eq!(rig.governor.next_deadline(100), Some(5_100));
    drop(lease);
}

#[test]
fn an_operation_that_ends_clears_its_numbers_and_the_next_starts_saturated() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.op().answered(&key(), connection, 0);
    let (second, other) = rig.open();
    rig.op().refused(&key(), other, Throttle, None, false, 1);
    assert_eq!(rig.view().state, State::Stable);
    drop((lease, second));
    // The operation ends: its numbers leave the pool, and the next starts
    // SATURATED at its own ceiling.
    rig.governor.end_operation("op", 2);
    assert_eq!(rig.pool.limit(&rig.site()), None);
    rig.governor.begin_operation("a", 4, true, 2);
    rig.scope = "a";
    assert!(rig.op().view(&key(), 2).is_none());
    assert_eq!(rig.op().admission(&key(), 2).target, 4);
}

#[test]
fn a_connection_that_leaves_before_its_exchange_is_answered_ends_its_attempt_with_no_verdict() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.host.release(lease, Disposition::Discarded).unwrap();
    rig.tick(5);
    // The attempt's window is closed: a late refusal finds nothing.
    assert_eq!(
        rig.op()
            .refused(&key(), connection, Throttle, None, false, 6),
        None
    );
}

#[test]
fn a_retry_after_on_a_connection_with_no_attempt_in_flight_still_sets_the_hold() {
    // A second request on a carried connection: its first exchange was
    // answered, so no window is open, and the server's word holds all the same.
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.op().answered(&key(), connection, 0);
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, Some(2_000), false, 10);
    assert_eq!(ruling, None);
    assert!(!rig.op().admission(&key(), 10).gate_open);
    assert_eq!(rig.view().n, 8, "nothing was judged");
    drop(lease);
}

#[test]
fn a_connection_no_exchange_answered_is_gone_when_the_server_closes_it_idle() {
    // POST-only use, or a lease returned unanswered: the entry must not stay
    // Setting up with its window open, or the key is never quiet.
    let mut rig = Rig::new(8, true);
    let (lease, _) = rig.open();
    assert_eq!(rig.view().possible, 1);
    rig.host.release(lease, Disposition::Reusable).unwrap();
    rig.script.lock().unwrap().lost = true;
    rig.tick(5);
    let view = rig.view();
    assert_eq!((view.possible, view.connected), (0, 0));
    assert!(rig.op().admission(&key(), 5).gate_open);
}

#[test]
fn an_answered_leased_exchange_sets_up_a_connection_no_discovery_answered() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    assert_eq!(rig.view().connected, 0);
    assert!(rig.op().exchange_begins(&key(), connection, 1));
    rig.op().answered(&key(), connection, 2);
    assert_eq!(rig.view().connected, 1);
    drop(lease);
}

#[test]
fn a_second_live_operation_does_not_erase_a_hold() {
    let mut rig = Rig::new(8, false);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, false, 0);
    rig.scope = "a";
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 100);
    // Another request is admitted while the first still runs: its machine is
    // its own, and it hears the server's word.
    rig.governor.begin_operation("b", 8, false, 200);
    rig.scope = "b";
    assert!(!rig.op().admission(&key(), 200).gate_open);
    assert!(!rig.op().admission(&key(), 20_099).gate_open);
    assert!(rig.op().admission(&key(), 20_100).gate_open);
    drop(lease);
}

#[test]
fn a_hold_still_in_force_outlives_its_operation_but_the_rest_of_its_state_does_not() {
    let mut rig = Rig::new(32, false);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 4, false, 0);
    rig.scope = "a";
    let (lease, connection) = rig.open();
    rig.op().answered(&key(), connection, 1);
    let other = Key::https("other", 443);
    rig.op().admission(&other, 1);
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 100);
    assert_eq!(rig.view().pool_limit, 4);
    rig.governor.end_operation("a", 150);
    // The next operation has another ceiling.
    rig.governor.begin_operation("b", 32, false, 200);
    rig.scope = "b";
    assert!(
        !rig.op().admission(&key(), 200).gate_open,
        "the server's word stands"
    );
    assert!(
        rig.op().view(&other, 200).is_none(),
        "a slot with no hold starts over"
    );
    let view = rig.view();
    assert_eq!(
        (view.pool_limit, view.connected, view.possible),
        (32, 0, 0),
        "only the hold survived: the new ceiling, none of the old table"
    );
    drop(lease);
}

#[test]
fn a_slot_kept_for_its_hold_takes_the_next_operations_ceiling_and_resends_the_pools_numbers() {
    // The reviewer's probe (P2-3): hold under ceiling 4, then an operation
    // with 32 on a pool whose numbers its admission has just cleared.
    let mut rig = Rig::new(32, false);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 4, false, 0);
    rig.scope = "a";
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 100);
    assert_eq!(rig.pool.limit(&rig.site()), Some(4));
    rig.governor.end_operation("a", 120);
    rig.host.release(lease, Disposition::Discarded).unwrap();
    rig.tick(150);
    rig.pool.install_capacity(rig.pool.capacity()).unwrap();
    assert_eq!(
        rig.pool.limit(&rig.site()),
        None,
        "install cleared the pool"
    );
    rig.governor.begin_operation("b", 32, false, 200);
    rig.scope = "b";
    assert!(!rig.op().admission(&key(), 20_099).gate_open);
    // The hold ends: the gate opens at the new operation's ceiling.
    let admission = rig.op().admission(&key(), 25_000);
    assert!(admission.gate_open);
    assert_eq!(admission.target, 32);
    assert!(
        rig.pool.limit(&rig.site()).is_none_or(|limit| limit == 32),
        "the pool's limit is {:?}",
        rig.pool.limit(&rig.site())
    );
}

/// Opens `count` connections on the rig and has `answer` of them answered by
/// the rig's operation: the connections the server holds.
fn hold_connections(rig: &mut Rig, count: usize) -> Vec<(Lease, Conn)> {
    (0..count)
        .map(|now| {
            rig.now = now as u64;
            let (lease, connection) = rig.open();
            rig.op().answered(&key(), connection, now as u64);
            (lease, connection)
        })
        .collect()
}

#[test]
fn each_operation_has_its_own_machine_over_the_connections_the_server_holds() {
    // Two overlapping operations on one site (§4.1: one machine per
    // operation). "a" is refused with three held and lowers its own N; "b",
    // which heard nothing, stays SATURATED, though it sees the same table.
    let mut rig = Rig::new(8, true);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, true, 0);
    rig.governor.begin_operation("b", 8, true, 0);
    rig.scope = "a";
    let held = hold_connections(&mut rig, 3);
    let (fourth, connection) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, None, false, 10);
    assert_eq!(ruling, Some(Ruling::Overload { n: 3 }));
    let a = rig.op().view(&key(), 10).unwrap();
    rig.scope = "b";
    let b = rig.op().view(&key(), 10).unwrap();
    assert_eq!((a.state, a.n, a.connected), (State::Stable, 3, 3));
    assert_eq!((b.state, b.n, b.connected), (State::Saturated, 8, 3));
    // The pool follows the most permissive machine, and the endpoint of each
    // operation reads its own target.
    assert_eq!(rig.pool.limit(&rig.site()), Some(8));
    assert_eq!(rig.op().admission(&key(), 10).target, 8);
    rig.scope = "a";
    assert_eq!(rig.op().admission(&key(), 10).target, 3);
    drop((held, fourth));
}

#[test]
fn the_pools_numbers_follow_the_live_operations_and_leave_with_the_last() {
    let mut rig = Rig::new(8, true);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, true, 0);
    rig.governor.begin_operation("b", 4, true, 0);
    rig.scope = "a";
    let held = hold_connections(&mut rig, 2);
    let (third, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, None, false, 10);
    assert_eq!(rig.view().n, 2);
    // "b" is at its ceiling of 4 and "a" at 2: the pool allows the larger.
    assert_eq!(rig.pool.limit(&rig.site()), Some(4));
    assert_eq!(rig.pool.settle(&rig.site()), 250, "a is outside SATURATED");
    rig.governor.end_operation("b", 11);
    assert_eq!(rig.pool.limit(&rig.site()), Some(2));
    rig.governor.end_operation("a", 12);
    assert_eq!(rig.pool.limit(&rig.site()), None);
    assert_eq!(rig.pool.settle(&rig.site()), 0);
    drop((held, third));
}

#[test]
fn a_retry_after_one_operation_hears_holds_every_live_operation_and_the_next() {
    let mut rig = Rig::new(8, false);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, false, 0);
    rig.governor.begin_operation("b", 8, false, 0);
    rig.scope = "a";
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(5_000), false, 100);
    rig.scope = "b";
    assert!(!rig.op().admission(&key(), 200).gate_open, "b hears it");
    rig.governor.end_operation("a", 300);
    rig.governor.end_operation("b", 300);
    rig.governor.begin_operation("c", 8, false, 400);
    rig.scope = "c";
    assert!(!rig.op().admission(&key(), 400).gate_open, "so does c");
    assert!(rig.op().admission(&key(), 5_100).gate_open);
    drop(lease);
}

#[test]
fn an_operation_judges_its_refusals_by_its_own_flag_and_ceiling() {
    // "a" is at `--max-retries 0` (no refusal lowers N); "b" adapts. The same
    // refusal reported by each is a hold for one and an Overload for the other.
    let mut rig = Rig::new(8, true);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, false, 0);
    rig.governor.begin_operation("b", 4, true, 0);
    rig.scope = "a";
    let held = hold_connections(&mut rig, 2);
    let (third, connection) = rig.open();
    assert_eq!(
        rig.op()
            .refused(&key(), connection, Throttle, None, false, 10),
        Some(Ruling::HoldOnly)
    );
    rig.scope = "b";
    let (fourth, other) = rig.open();
    assert_eq!(
        rig.op().refused(&key(), other, Throttle, None, false, 11),
        Some(Ruling::Overload { n: 2 })
    );
    let view = rig.view();
    assert_eq!((view.n, view.ceiling), (2, 4));
    drop((held, third, fourth));
}

/// Starts a connection the script refuses at setup and ticks until the host
/// has seen it end: the pool's request fails, which the rig ignores.
fn refused_setup(rig: &mut Rig, at: u64) {
    rig.script.lock().unwrap().refuse = vec![false; 64];
    let opened = rig.script.lock().unwrap().opened;
    rig.script.lock().unwrap().refuse[opened] = true;
    let _checkout = rig.pool.checkout(rig.request()).unwrap();
    rig.tick(at);
    rig.tick(at);
}

#[test]
fn a_setup_the_server_ended_is_judged_when_its_endpoint_says_why() {
    // The host saw a refused connect end; the endpoint, which has the failure,
    // asks what the machine made of it (§4.8). Three connections were held,
    // so nothing was counted besides: a conclusive Suspect opens a
    // confirmation (§4.5 rule 3).
    let mut rig = Rig::new(8, true);
    let held = hold_connections(&mut rig, 3);
    refused_setup(&mut rig, 10);
    assert_eq!(rig.view().possible, 3, "the refused setup is Gone");
    let ruling = rig.op().setup_failed(&key(), Suspect, 11);
    assert_eq!(ruling, Some(Ruling::Confirmation));
    assert!(rig.view().confirmation);
    assert!(!rig.op().admission(&key(), 11).gate_open);
    // It is judged once: a second report finds nothing to judge.
    assert_eq!(rig.op().setup_failed(&key(), Suspect, 12), None);
    drop(held);
}

#[test]
fn a_setup_alone_is_the_retry_machines_and_an_unreported_one_expires() {
    let mut rig = Rig::new(8, true);
    refused_setup(&mut rig, 10);
    assert_eq!(
        rig.op().setup_failed(&key(), Suspect, 11),
        Some(Ruling::RetryMachine),
        "hi = 0: nothing else was counted"
    );
    // A test that ends and is never reported must not hold the slot for good.
    let held = hold_connections(&mut rig, 2);
    refused_setup(&mut rig, 100);
    assert_eq!(rig.op().setup_failed(&key(), Suspect, 5_000), None);
    drop(held);
}

/// A rig in STABLE at `N = 3` with three connections held and the probe timer
/// due at 510 ms: a Throttle took the fourth.
fn stable_at_three() -> (Rig, Vec<(Lease, Conn)>) {
    let mut rig = Rig::new(8, true);
    let held = hold_connections(&mut rig, 3);
    let (fourth, connection) = rig.open();
    assert_eq!(
        rig.op()
            .refused(&key(), connection, Throttle, None, false, 10),
        Some(Ruling::Overload { n: 3 })
    );
    drop(fourth);
    rig.tick(11);
    (rig, held)
}

#[test]
fn a_due_probe_is_armed_for_one_carrier_and_the_pool_limit_rises_with_it() {
    let (mut rig, held) = stable_at_three();
    rig.op().set_demand(&key(), 1, 1, 100);
    assert_eq!(
        rig.op().start_test(&key(), false, 100),
        None,
        "T0 not expired"
    );
    assert_eq!(rig.op().start_test(&key(), false, 1_000), Some(4));
    assert_eq!(
        rig.pool.limit(&rig.site()),
        Some(4),
        "raised under the lock"
    );
    assert!(
        !rig.op().admission(&key(), 1_000).gate_open,
        "one test only"
    );
    assert_eq!(rig.op().start_test(&key(), false, 1_000), None);
    // The next connection the pool starts is the test: refused fairly, it
    // backs the timer off and leaves N alone.
    rig.now = 1_000;
    let (probe, connection) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, None, false, 1_010);
    assert_eq!(ruling, Some(Ruling::RefusedTest));
    assert_eq!(rig.pool.limit(&rig.site()), Some(3), "lowered again");
    drop((probe, held));
}

#[test]
fn a_probe_that_succeeds_raises_n_and_a_final_attempt_never_carries_one() {
    let (mut rig, held) = stable_at_three();
    rig.op().set_demand(&key(), 1, 1, 100);
    assert_eq!(rig.op().start_test(&key(), true, 1_000), None);
    assert_eq!(rig.op().start_test(&key(), false, 1_000), Some(4));
    rig.now = 1_000;
    let (probe, connection) = rig.open();
    rig.op().answered(&key(), connection, 1_010);
    let view = rig.view();
    assert_eq!((view.state, view.n), (State::Discovering, 4));
    drop((probe, held));
}

#[test]
fn a_probe_needs_a_carrier_and_a_carrier_that_never_connects_gives_the_test_back() {
    let (rig, held) = stable_at_three();
    // No member wants a connection: no connection is opened only to test.
    rig.op().set_demand(&key(), 0, 0, 100);
    assert_eq!(rig.op().start_test(&key(), false, 1_000), None);
    rig.op().set_demand(&key(), 1, 1, 1_000);
    assert_eq!(rig.op().start_test(&key(), false, 1_000), Some(4));
    // The carrier leased an idle connection instead: no connect, no test.
    rig.op().test_unused(&key(), 1_100);
    assert_eq!(rig.pool.limit(&rig.site()), Some(3));
    assert!(rig.op().admission(&key(), 1_100).gate_open);
    assert_eq!(
        rig.op().start_test(&key(), false, 1_100),
        Some(4),
        "due again at once"
    );
    drop(held);
}
