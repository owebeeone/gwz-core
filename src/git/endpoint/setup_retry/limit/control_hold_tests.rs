//! One key end to end, continued: admission against settling connections,
//! the races of §4.4, holds, `--max-retries 0`, restore steps and the
//! clockless setup.
use super::{
    control::*,
    control_tests::Rig,
    filter::Signal::{Suspect, Throttle},
    fsm::State,
    notes::Note,
    states::{ConnEvent, ConnId},
    windows::AttemptKind,
};

#[test]
fn saturated_does_not_wait_for_settling_connections_and_stable_does() {
    let mut rig = Rig::new(2);
    rig.connect(1);
    rig.conn(1, ConnEvent::Closing);
    rig.conn(1, ConnEvent::Disposed);
    rig.connect(1);
    assert!(
        rig.limit.admits_ordinary(0),
        "held 1 < C 2 though Possible is 2"
    );
    let mut rig = Rig::stable(4, 2);
    rig.conn(1, ConnEvent::Closing);
    rig.conn(1, ConnEvent::Disposed);
    assert!(
        !rig.limit.admits_ordinary(0),
        "Possible 2 = N while connection 1 settles"
    );
    rig.at(250);
    assert!(rig.limit.admits_ordinary(250));
}

#[test]
fn r1_an_inconclusive_refusal_makes_the_next_start_wait_for_what_overlapped_it() {
    // Cases 22a and 50: C = 4, Y settling when X starts.
    let mut rig = Rig::new(4);
    rig.connect(3);
    rig.conn(1, ConnEvent::Closing);
    rig.conn(1, ConnEvent::Disposed);
    rig.connect(1);
    let x = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(x, Throttle), Ruling::Inconclusive, "hi = 4 = C");
    assert_eq!(rig.limit.n(), 4);
    assert!(!rig.limit.admits_ordinary(100), "waits for Y");
    rig.at(250);
    assert!(rig.limit.admits_ordinary(250));
}

#[test]
fn r1_in_saturated_with_room_the_refusal_is_conclusive_and_one_test_repairs_it() {
    // Case 22b: 20 Connected, Y settling, the server's limit 21 still counts Y.
    let mut rig = Rig::new(32);
    rig.connect(21);
    rig.conn(21, ConnEvent::Closing);
    rig.conn(21, ConnEvent::Disposed);
    let x = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(x, Throttle), Ruling::Overload { n: 20 });
    rig.demand(5, 5);
    rig.at(500);
    let probe = rig.go(Start::Probe);
    assert_eq!(rig.target, 21);
    rig.ok(probe);
    assert_eq!(rig.limit.n(), 21);
}

#[test]
fn a_refusal_with_nothing_else_counted_is_the_retry_machines_and_moves_nothing() {
    // Case 6c.
    let mut rig = Rig::new(32);
    let a = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(a, Suspect), Ruling::RetryMachine);
    assert!(!rig.limit.confirmation_open());
    assert_eq!((rig.limit.n(), rig.limit.state()), (32, State::Saturated));
}

#[test]
fn a_retry_after_holds_new_starts_and_first_exchanges_and_a_discovery_hold_lets_posts_go_on() {
    // Cases 7 and 51.
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse_with(a, Throttle, Some(5_000), false);
    assert!(!rig.limit.admits_ordinary(10) && !rig.limit.admits_first_exchange(10));
    assert!(rig.limit.admits_continuing(10));
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse_with(a, Throttle, Some(5_000), true);
    assert!(
        !rig.limit.admits_continuing(10),
        "a POST-set hold holds the next POST too"
    );
    assert_eq!(rig.at(5_000), Some(Action::DiscardIdle));
    assert!(
        !rig.limit.admits_continuing(5_000),
        "lifts after the discard"
    );
    rig.limit.idle_discarded(5_000);
    assert!(rig.limit.admits_continuing(5_000));
}

#[test]
fn a_long_hold_ends_with_a_discard_and_lifts_only_after_it() {
    let mut rig = Rig::new(32);
    rig.connect(8);
    rig.now = 100;
    let a = rig.go(Start::Ordinary);
    rig.refuse_with(a, Throttle, Some(20_000), false);
    assert!(
        rig.limit
            .drain_notes()
            .contains(&Note::Hold { wait_ms: 20_000 })
    );
    assert_eq!(rig.at(20_099), None);
    assert!(!rig.limit.admits_first_exchange(20_099));
    assert_eq!(rig.at(20_100), Some(Action::DiscardIdle));
    assert_eq!(rig.at(20_101), None, "asked once");
    assert!(
        !rig.limit.admits_first_exchange(20_101),
        "not before the discard has run"
    );
    assert_eq!(rig.limit.next_deadline(20_101), Some(20_101));
    rig.limit.idle_discarded(20_101);
    assert!(rig.limit.admits_first_exchange(20_101));
}

#[test]
fn a_short_hold_lifts_at_its_end_with_no_discard_and_no_note() {
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse_with(a, Throttle, Some(800), false);
    assert!(!rig.limit.admits_first_exchange(799));
    assert_eq!(rig.at(800), None);
    assert!(rig.limit.admits_first_exchange(800));
    assert!(
        !rig.limit
            .drain_notes()
            .iter()
            .any(|n| matches!(n, Note::Hold { .. }))
    );
}

#[test]
fn a_test_is_not_due_before_the_hold_ends() {
    // Case 7: the first test after the hold is at N + 1.
    let mut rig = Rig::stable(32, 8);
    rig.demand(5, 5);
    rig.now = 100;
    let step = rig.go(Start::Restore { target: 9 });
    assert_eq!(rig.target, 9);
    rig.refuse_with(step, Throttle, Some(1_000), false);
    rig.at(1_099);
    assert_eq!(rig.plan(), None, "held");
    rig.at(1_100);
    assert_eq!(
        rig.plan().map(|p| (p.kind, p.target)),
        Some((AttemptKind::Probe, 9))
    );
}

#[test]
fn at_max_retries_zero_a_refusal_sets_the_hold_and_lowers_nothing() {
    // Case 11 and §5.3.
    for signal in [Throttle, Suspect] {
        let mut rig = Rig::with(32, false, 1_000);
        rig.connect(8);
        let a = rig.go(Start::Ordinary);
        // A Suspect is not evidence without a budget to test it with: the
        // retry machine's, though the server's `Retry-After` still holds.
        let want = if signal == Throttle {
            Ruling::HoldOnly
        } else {
            Ruling::RetryMachine
        };
        assert_eq!(rig.refuse_with(a, signal, Some(1_000), false), want);
        assert!(!rig.limit.confirmation_open());
        assert_eq!((rig.limit.n(), rig.limit.state()), (32, State::Saturated));
        assert!(!rig.limit.admits_first_exchange(999));
        assert!(rig.limit.admits_first_exchange(1_000));
    }
}

#[test]
fn n_reaching_the_ceiling_by_a_restore_step_is_saturated_and_tests_nothing() {
    // Case 37.
    let mut rig = Rig::stable(32, 8);
    rig.demand(5, 5);
    rig.at(500);
    rig.connect(23);
    let step = rig.go(Start::Restore { target: 32 });
    assert_eq!(rig.ok(step), Ruling::Succeeded);
    assert_eq!((rig.limit.n(), rig.limit.state()), (32, State::Saturated));
    assert_eq!(rig.plan(), None);
}

#[test]
fn a_hung_setup_with_no_clock_does_not_keep_the_key_from_being_quiet() {
    // Case 47.
    let mut rig = Rig::new(32);
    rig.connect(8);
    rig.limit
        .conn(ConnId(900), ConnEvent::Started { clocked: false }, 0);
    let a = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(a, Suspect), Ruling::Confirmation);
    rig.demand(4, 4);
    let plan = rig.plan().unwrap();
    assert_eq!(
        (plan.kind, plan.target, plan.ready),
        (AttemptKind::Confirming, 9, true)
    );
}

#[test]
fn a_settle_time_follows_the_measured_connect() {
    let mut rig = Rig::new(4);
    rig.connect(1);
    rig.limit.connect_time(400);
    rig.conn(1, ConnEvent::Closing);
    rig.conn(1, ConnEvent::Disposed);
    assert_eq!(rig.limit.next_deadline(0), Some(900));
}
