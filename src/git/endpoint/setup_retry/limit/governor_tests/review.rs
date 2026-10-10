//! The tests that close the adaptive step's review
//! (GwzTransportAdaptiveStep-ReviewCodeState.md), over the rig of the parent.
use super::*;

#[test]
fn p2_1_a_new_operation_sees_the_connections_that_predate_it() {
    let mut rig = Rig::new(32, true);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 32, true, 0);
    rig.scope = "a";
    let a_held = hold_connections(&mut rig, 20);
    rig.governor.begin_operation("b", 32, true, 30);
    rig.scope = "b";
    rig.op().admission(&key(), 30);
    assert_eq!(rig.view().connected, 20, "b sees what a's connections hold");
    let mut b_held = Vec::new();
    for t in 31..=32 {
        rig.now = t;
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, t);
        b_held.push(lease);
    }
    rig.now = 33;
    let (third, connection) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, None, false, 40);
    assert_eq!(ruling, Some(Ruling::Overload { n: 22 }), "not 2");
    let b = rig.view();
    rig.scope = "a";
    assert_eq!((b.connected, rig.view().connected), (22, 22));
    drop((a_held, b_held, third));
}

#[test]
fn p2_1_connections_kept_idle_by_an_ended_operation_are_seen_by_the_next() {
    let mut rig = Rig::new(8, true);
    rig.governor.end_operation("op", 0);
    rig.governor.begin_operation("a", 8, true, 0);
    rig.scope = "a";
    let held = hold_connections(&mut rig, 3);
    rig.governor.end_operation("a", 10);
    rig.governor.begin_operation("b", 8, true, 20);
    rig.scope = "b";
    rig.op().admission(&key(), 20);
    let view = rig.view();
    assert_eq!((view.connected, view.possible), (3, 3));
    drop(held);
}

#[test]
fn p2_2_another_operations_connection_does_not_take_an_armed_test() {
    let (mut rig, held) = stable_at_three();
    rig.governor.begin_operation("b", 8, true, 20);
    rig.op().set_demand(&key(), 1, 1, 100);
    let (_, token) = arm(&rig, false, 1_000).unwrap();
    // A member of b starts the next connection, and b's endpoint answers it.
    rig.scope = "b";
    rig.now = 1_000;
    rig.op().admission(&key(), 1_000);
    let (b_lease, b_conn) = rig.open();
    rig.op().answered(&key(), b_conn, 1_010);
    rig.scope = "op";
    rig.tick(5_000);
    assert!(
        !rig.op().admission(&key(), 5_000).gate_open,
        "the test is still armed for op's own carrier"
    );
    // The carrier's own connection is the test: refused, and the gate reopens.
    rig.now = 5_000;
    let (own, own_conn) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), own_conn, Throttle, None, false, 5_010);
    // It was the test (an ordinary refusal would be an Overload), and b's
    // connection joined mid-window, so the test ran unfair and is due again.
    assert_eq!(ruling, Some(Ruling::Unfair));
    assert!(rig.op().admission(&key(), 5_010).gate_open);
    drop((token, own, b_lease, held));
}

#[test]
fn p2_3_another_operations_answers_raise_a_stable_operations_n() {
    let (mut rig, held) = stable_at_three();
    rig.governor.begin_operation("b", 8, true, 20);
    rig.scope = "b";
    rig.op().admission(&key(), 300);
    let mut b_held = Vec::new();
    for t in 300..304 {
        rig.now = t;
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, t);
        b_held.push(lease);
    }
    rig.scope = "op";
    // The server holds seven: op's N follows what it is visibly holding.
    let view = rig.view();
    assert_eq!((view.connected, view.n), (7, 7));
    drop((held, b_held));
}

#[test]
fn p2_4_a_refused_test_is_judged_on_its_own_window_not_a_waves_unreported_setup() {
    let mut rig = Rig::new(8, true);
    let held = hold_connections(&mut rig, 3);
    let wave = refused_setup(&mut rig, 10);
    assert_eq!(
        rig.op().setup_failed(&key(), &wave, Suspect, 11),
        Some(Ruling::Confirmation)
    );
    // Another setup of the wave ends and its endpoint never reports it (an
    // SSH reset before authentication).
    let _unreported = refused_setup(&mut rig, 20);
    rig.op().set_demand(&key(), 1, 1, 21);
    let (target, _token) = arm(&rig, false, 21).unwrap();
    assert_eq!(target, 4);
    // The confirming test is refused fairly and its own member reports it.
    let test = refused_setup(&mut rig, 22);
    assert_eq!(
        rig.op().setup_failed(&key(), &test, Suspect, 23),
        Some(Ruling::Overload { n: 3 })
    );
    drop(held);
}

#[test]
fn p2_4_a_report_takes_its_own_members_setup_not_the_newest() {
    let mut rig = Rig::new(8, true);
    let held = hold_connections(&mut rig, 3);
    let first = refused_setup(&mut rig, 10);
    assert_eq!(
        rig.op().setup_failed(&key(), &first, Suspect, 11),
        Some(Ruling::Confirmation)
    );
    // A second setup of the wave ends unreported; then the confirming test
    // runs and is refused too.
    let wave = refused_setup(&mut rig, 20);
    rig.op().set_demand(&key(), 1, 1, 21);
    let (_, _token) = arm(&rig, false, 21).unwrap();
    let test = refused_setup(&mut rig, 22);
    // The wave's member reports late: its own window is an ordinary one, and
    // the confirmation is open, so it is inconclusive; the test's window is
    // not the one it is judged on.
    assert_eq!(
        rig.op().setup_failed(&key(), &wave, Suspect, 23),
        Some(Ruling::Inconclusive)
    );
    assert_eq!(
        rig.op().setup_failed(&key(), &test, Suspect, 24),
        Some(Ruling::Overload { n: 3 })
    );
    drop(held);
}

#[test]
fn p2_4_a_report_that_comes_before_the_host_sees_the_connect_end_is_judged_at_once() {
    // A timed-out connect fails the request before the host disposes of the
    // connection, so the endpoint's report precedes the Retired event.
    let mut rig = Rig::new(8, true);
    let held = hold_connections(&mut rig, 3);
    rig.script.lock().unwrap().ready = false;
    rig.script.lock().unwrap().refuse = vec![false; 64];
    let request = rig.request();
    let member = format!("m{}", rig.members);
    let _checkout = rig.pool.checkout(request).unwrap();
    rig.tick(10);
    assert_eq!(rig.view().possible, 4, "the connect is in flight");
    assert_eq!(
        rig.op().setup_failed(&key(), &member, Suspect, 11),
        Some(Ruling::Confirmation),
        "judged on its own window, before the Retired event"
    );
    drop(held);
}

#[test]
fn p2_5_a_dropped_carrier_token_reopens_the_gate_for_good() {
    let (rig, held) = stable_at_three();
    rig.op().set_demand(&key(), 1, 1, 100);
    let (_, token) = arm(&rig, false, 1_000).unwrap();
    drop(token);
    rig.governor.tick(600_000);
    assert!(rig.op().admission(&key(), 600_000).gate_open);
    assert!(arm(&rig, false, 600_000).is_some());
    drop(held);
}

#[test]
fn p2_7_a_report_for_an_ended_operation_does_not_revive_it() {
    let (rig, held) = stable_at_three();
    assert_eq!(rig.pool.limit(&rig.site()), Some(3));
    rig.governor.begin_operation("gone", 8, true, 20);
    rig.governor.end_operation("gone", 30);
    let gone = rig.governor.scoped("gone");
    gone.set_demand(&key(), 0, 0, 40);
    assert!(gone.start_test(&key(), false, 40).is_none());
    assert_eq!(gone.admission(&key(), 40).target, 8, "unscoped: no limit");
    assert!(gone.view(&key(), 40).is_none());
    rig.governor.tick(50);
    assert!(!rig.governor.book().scopes.contains_key("gone"));
    assert_eq!(rig.pool.limit(&rig.site()), Some(3));
    drop(held);
}

#[test]
fn p2_8_a_lone_throttle_without_retry_after_holds_the_site_for_t0() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    let ruling = rig
        .op()
        .refused(&key(), connection, Throttle, None, false, 100);
    assert_eq!(ruling, Some(Ruling::RetryMachine), "hi = 0: learns nothing");
    assert!(!rig.op().admission(&key(), 100).gate_open);
    assert!(!rig.op().admission(&key(), 599).gate_open);
    assert!(rig.op().admission(&key(), 600).gate_open, "T0 = 500 ms");
    drop(lease);
}

#[test]
fn p3_1_a_due_probe_that_is_not_ready_stops_the_pool_evicting_for_others() {
    // §4.5 rule 2: once a due test has reached its base, nothing is closed or
    // evicted on the key until it is quiet.
    let (mut rig, mut held) = stable_at_three();
    rig.op().set_demand(&key(), 1, 1, 400);
    // A connection closes just before the timer expires: the key is not quiet.
    let (lease, _) = held.pop().unwrap();
    rig.host.release(lease, Disposition::Discarded).unwrap();
    rig.tick(505);
    rig.tick(511);
    assert!(rig.pool.no_evict(&rig.site()), "closes are suppressed");
    // Quiet again, the test is ready and evictions are allowed.
    rig.tick(900);
    rig.op().admission(&key(), 900);
    assert!(!rig.pool.no_evict(&rig.site()));
    drop(held);
}

/// Three answered connections whose socket connect took `tcp_ms` (never
/// reported when `None`) and whose first exchange was answered 700 ms after
/// the connect began, then a refusal: the settle time the machine holds
/// slots for.
fn settle_after(tcp_ms: Option<u64>) -> u64 {
    let mut rig = Rig::new(8, true);
    rig.script.lock().unwrap().tcp_ms = tcp_ms;
    let mut held = Vec::new();
    for _ in 0..3 {
        rig.now = 0;
        let (lease, connection) = rig.open();
        rig.op().answered(&key(), connection, 700);
        held.push(lease);
    }
    let (fourth, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, None, false, 710);
    let settle = rig.view().settle_ms;
    drop((held, fourth));
    settle
}

#[test]
fn p3_a_the_settle_time_follows_the_tcp_connect_not_the_whole_setup() {
    // A login or a first exchange of 700 ms is not the round trip: 2 x 40 +
    // 100 is under the 250 ms floor.
    assert_eq!(settle_after(Some(40)), 250);
    // A slow socket connect does raise it: 2 x 300 + 100.
    assert_eq!(settle_after(Some(300)), 700);
    // An answer is no connect time: with no socket report the floor stands.
    assert_eq!(settle_after(None), 250);
}

#[test]
fn an_outage_is_restored_by_steps_whose_starts_the_pool_admits_up_to_the_steps_target() {
    use crate::git::endpoint::setup_retry::Change;
    // N = 3 of 8 after a throttle, N_good = 8 (its last success was at 8).
    let (mut rig, _held) = stable_at_three();
    rig.op().health(&key(), Change::Left, 20);
    rig.op().health(&key(), Change::Healed, 21);
    // The refused connection's slot is held for the settle time first.
    rig.tick(400);
    let view = rig.view();
    assert_eq!(
        (view.state, view.n, view.pool_limit),
        (State::Restoring, 3, 6)
    );
    assert_eq!(rig.pool.limit(&rig.site()), Some(6), "S = min(8, 2 x 3)");
    // The step's three starts are restore starts; their answers raise N to 6.
    let mut step = Vec::new();
    for _ in 0..3 {
        step.push(rig.open());
    }
    for (_, connection) in &step {
        rig.op().answered(&key(), *connection, 30);
    }
    let view = rig.view();
    assert_eq!(
        (view.state, view.n, view.pool_limit),
        (State::Restoring, 6, 8)
    );
    assert_eq!(
        rig.pool.limit(&rig.site()),
        Some(8),
        "the next step doubles"
    );
    let mut second = Vec::new();
    for _ in 0..2 {
        second.push(rig.open());
    }
    for (_, connection) in &second {
        rig.op().answered(&key(), *connection, 40);
    }
    let view = rig.view();
    assert_eq!((view.state, view.n), (State::Saturated, 8));
    drop((step, second));
}

#[test]
fn a_restore_steps_new_connection_is_never_started_by_a_member_on_its_final_attempt() {
    use crate::git::endpoint::setup_retry::Change;
    let (rig, _held) = stable_at_three();
    let before = rig.op().admission(&key(), 20);
    assert!(!before.restoring);
    assert!(
        !before.holds_final_attempt(true, false, false),
        "no restore"
    );
    rig.op().health(&key(), Change::Left, 20);
    rig.op().health(&key(), Change::Healed, 21);
    let admission = rig.op().admission(&key(), 22);
    assert!(admission.restoring);
    // Only the final-attempt member that needs a new connection waits.
    assert!(admission.holds_final_attempt(true, false, false));
    assert!(
        !admission.holds_final_attempt(false, false, false),
        "a spare member"
    );
    assert!(
        !admission.holds_final_attempt(true, true, false),
        "it can lease"
    );
    assert!(
        !admission.holds_final_attempt(true, false, true),
        "a test's carrier"
    );
}
