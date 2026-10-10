//! What a key tells the pool (§4.9): the admission target and the settle
//! time, and a window that opens when the pool has made the connection.
use super::{
    control::*,
    control_tests::Rig,
    filter::Signal::Throttle,
    fsm::State,
    states::{ConnEvent, ConnId},
    windows::{AttemptId, AttemptKind, Own},
};

#[test]
fn a_saturated_key_tells_the_pool_its_ceiling_and_no_settle_time() {
    let rig = Rig::new(32);
    assert_eq!((rig.limit.pool_limit(), rig.limit.settle_ms()), (32, 0));
}

#[test]
fn a_stable_key_tells_the_pool_n_and_the_settle_time() {
    let mut rig = Rig::stable(32, 8);
    assert_eq!(rig.limit.state(), State::Stable);
    assert_eq!(rig.limit.pool_limit(), 8);
    // Ts with no connect measured is 250 ms; a measured connect raises it.
    assert_eq!(rig.limit.settle_ms(), 250);
    rig.limit.connect_time(400);
    assert_eq!(rig.limit.settle_ms(), 900);
}

#[test]
fn a_test_in_flight_raises_the_pool_limit_to_its_target_and_the_result_lowers_it() {
    // Case 41: STABLE at 8, a probe due; the pool's limit is 9 while it runs.
    let mut rig = Rig::stable(32, 8);
    rig.demand(1, 1);
    rig.at(500);
    assert_eq!(rig.limit.pool_limit(), 8);
    let test = rig.go(Start::Probe);
    assert_eq!(rig.limit.pool_limit(), 9);
    let ruling = rig.refuse(test, Throttle);
    assert_eq!(ruling, Ruling::RefusedTest);
    assert_eq!(rig.limit.pool_limit(), 8);
}

#[test]
fn begin_opens_a_window_for_a_connection_the_pool_has_made_without_asking_admission_again() {
    // Four idle connections of other identities fill the ceiling, so an
    // ordinary start is not admitted; the pool evicted one and made a new
    // connection under its own limit, and the window opens at its connect.
    let mut rig = Rig::new(4);
    rig.connect(4);
    assert!(!rig.limit.admits_ordinary(0));
    assert!(rig.limit.begin(
        AttemptId(1),
        AttemptKind::Ordinary,
        4,
        Own::New(ConnId(9)),
        true,
        0
    ));
    // The same attempt cannot begin twice.
    assert!(!rig.limit.begin(
        AttemptId(1),
        AttemptKind::Ordinary,
        4,
        Own::New(ConnId(10)),
        true,
        0
    ));
    rig.limit.conn(ConnId(9), ConnEvent::SetupEnded, 0);
    let ruling = rig
        .limit
        .result(
            AttemptId(1),
            Outcome::Refused {
                signal: Throttle,
                retry_after_ms: None,
                post: false,
            },
            0,
        )
        .unwrap();
    // hi = 4 others, not below the target 4: inconclusive, no decrease.
    assert_eq!(ruling, Ruling::Inconclusive);
    assert_eq!(rig.limit.n(), 4);
}

#[test]
fn a_connection_discarded_before_its_setup_was_answered_closes_and_settles() {
    // HTTPS: a throttled discovery's connection was never Connected (§4.1).
    let mut rig = Rig::new(8);
    rig.limit
        .conn(ConnId(1), ConnEvent::Started { clocked: true }, 0);
    rig.limit.conn(ConnId(1), ConnEvent::Closing, 10);
    assert_eq!(rig.limit.table().possible(), 1);
    assert!(!rig.limit.table().is_quiet());
    rig.limit.conn(ConnId(1), ConnEvent::Disposed, 20);
    rig.limit.tick(20 + 250);
    assert_eq!(rig.limit.table().possible(), 0);
}

#[test]
fn the_gate_is_closed_by_a_hold_until_the_idle_connections_are_discarded_not_by_the_count() {
    // §4.5: no start while a hold is in force, and a long hold lifts only
    // after the key's idle connections have been discarded. The count of
    // connections is the pool's limit, not the gate's.
    let mut rig = Rig::new(4);
    rig.connect(4);
    assert!(!rig.limit.admits_ordinary(0), "four held, ceiling four");
    assert!(rig.limit.gate_open(0), "but the gate is open");
    let attempt = AttemptId(7);
    assert!(rig.limit.begin(
        attempt,
        AttemptKind::Ordinary,
        4,
        Own::New(ConnId(9)),
        true,
        0
    ));
    rig.limit.conn(ConnId(9), ConnEvent::SetupEnded, 0);
    rig.limit.result(
        attempt,
        Outcome::Refused {
            signal: Throttle,
            retry_after_ms: Some(5_000),
            post: false,
        },
        0,
    );
    assert!(!rig.limit.gate_open(0));
    assert_eq!(rig.limit.tick(4_999), None);
    assert!(!rig.limit.gate_open(4_999));
    assert_eq!(rig.limit.tick(5_000), Some(Action::DiscardIdle));
    assert!(!rig.limit.gate_open(5_000), "lifts only after the discard");
    rig.limit.idle_discarded(5_000);
    assert!(rig.limit.gate_open(5_000));
}
