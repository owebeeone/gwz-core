//! SSH's setup limit `Ns` (§4.10, §10.2 cases 3, 46, 49, 52 and 53), on a
//! scripted clock: a drop before authentication feeds a second machine and
//! never `N`.
use super::{
    control::*,
    control_tests::{Flight, Rig},
    filter::Signal::SetupDrop,
    fsm::State,
    states::{ConnEvent, ConnId},
    windows::{AttemptId, AttemptKind, Own},
};

/// A key whose setup limit is `ns`, found by a wave of `wave` setups of which
/// the first `ns` log in and the rest are dropped. Returns the wave's flights.
fn limited(wave: usize, ns: usize) -> (Rig, Vec<Flight>) {
    let mut rig = Rig::new(wave);
    let flights: Vec<Flight> = (0..wave).map(|_| rig.go(Start::Ordinary)).collect();
    assert_eq!(rig.refuse(flights[ns], SetupDrop), Ruling::SetupLimit);
    for flight in &flights[ns + 1..] {
        assert_eq!(rig.refuse(*flight, SetupDrop), Ruling::Inconclusive);
    }
    for flight in &flights[..ns] {
        rig.ok(*flight);
    }
    assert_eq!(rig.limit.setup_limit(), ns);
    (rig, flights)
}

#[test]
fn a_wave_of_32_against_a_limit_of_10_assigns_ns_when_its_window_resolves() {
    // Case 3: the drops arrive first and the logins later, so the value
    // cannot be taken before the co-setups have results.
    let mut rig = Rig::new(32);
    let flights: Vec<Flight> = (0..32).map(|_| rig.go(Start::Ordinary)).collect();
    // Thirty-one others were in setup for the whole window: conclusive
    // (31 < Ns = 32), and new setups are held while the window is open.
    assert_eq!(rig.refuse(flights[10], SetupDrop), Ruling::SetupLimit);
    assert!(
        !rig.limit.admits_ordinary(0),
        "held until the wave resolves"
    );
    for flight in &flights[11..] {
        assert_eq!(rig.refuse(*flight, SetupDrop), Ruling::Inconclusive);
    }
    assert_eq!(rig.limit.setup_limit(), 32, "not taken before the window");
    for flight in &flights[..9] {
        rig.ok(*flight);
    }
    assert_eq!(
        rig.limit.setup_limit(),
        32,
        "one co-setup has no result yet"
    );
    rig.ok(flights[9]);
    assert_eq!(
        rig.limit.setup_limit(),
        10,
        "the co-setups the server let through"
    );
    assert_eq!(rig.limit.setup_state(), State::Stable);
    // N is not touched.
    assert_eq!((rig.limit.n(), rig.limit.state()), (32, State::Saturated));
    // The second wave admits ten setups and no more.
    for _ in 0..10 {
        rig.go(Start::Ordinary);
    }
    assert!(rig.start(Start::Ordinary).is_none(), "ten setups in flight");
    assert!(!rig.limit.has_room());
}

#[test]
fn a_drop_with_nothing_else_in_setup_is_the_retry_machines() {
    let mut rig = Rig::new(8);
    let only = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(only, SetupDrop), Ruling::RetryMachine);
    assert_eq!(rig.limit.setup_limit(), 8);
    assert!(rig.limit.admits_ordinary(0), "no window was opened");
}

#[test]
fn at_max_retries_zero_a_drop_never_lowers_the_setup_limit() {
    // Case 46.
    let mut rig = Rig::with(8, false, 1_000);
    let flights: Vec<Flight> = (0..4).map(|_| rig.go(Start::Ordinary)).collect();
    assert_eq!(rig.refuse(flights[0], SetupDrop), Ruling::RetryMachine);
    for flight in &flights[1..] {
        rig.ok(*flight);
    }
    assert_eq!(rig.limit.setup_limit(), 8);
    assert!(rig.limit.admits_ordinary(0));
}

#[test]
fn a_drop_while_holding_as_many_setups_as_the_limit_is_inconclusive() {
    let (mut rig, _) = limited(8, 2);
    let a = rig.go(Start::Ordinary);
    let b = rig.go(Start::Ordinary);
    // A third setup the operation's rules did not admit (another operation's):
    // two others in flight, which is Ns.
    let extra = (AttemptId(900), ConnId(900));
    assert!(rig.limit.begin(
        extra.0,
        AttemptKind::Ordinary,
        8,
        Own::New(extra.1),
        true,
        0
    ));
    assert_eq!(rig.refuse(extra, SetupDrop), Ruling::Inconclusive);
    assert_eq!(rig.limit.setup_limit(), 2);
    rig.ok(a);
    rig.ok(b);
}

#[test]
fn a_hung_setup_with_no_clock_is_not_waited_for_and_counts_as_not_succeeded() {
    // Case 52: `--ssh-timeout 0`.
    let mut rig = Rig::new(8);
    // The hung setup is in progress for the whole window of the drop below.
    let hung = (AttemptId(900), ConnId(900));
    assert!(
        rig.limit
            .begin(hung.0, AttemptKind::Ordinary, 8, Own::New(hung.1), false, 0)
    );
    let flights: Vec<Flight> = (0..7).map(|_| rig.go(Start::Ordinary)).collect();
    assert_eq!(rig.refuse(flights[6], SetupDrop), Ruling::SetupLimit);
    for flight in &flights[..3] {
        rig.ok(*flight);
    }
    assert!(
        !rig.limit.admits_ordinary(0),
        "three clocked setups still pending"
    );
    for flight in &flights[3..6] {
        assert_eq!(rig.refuse(*flight, SetupDrop), Ruling::Inconclusive);
    }
    // The window resolved with the hung setup still hanging.
    assert_eq!(rig.limit.setup_limit(), 3);
    assert!(rig.limit.admits_ordinary(0), "the others proceed");
}

#[test]
fn a_success_beside_more_setups_than_ns_raises_it() {
    // The server let a setup through while holding two others for its whole
    // window: the limit is at least three.
    let (mut rig, _) = limited(8, 2);
    let flights: Vec<Flight> = (0..3)
        .map(|n| (AttemptId(900 + n), ConnId(900 + n)))
        .collect();
    for flight in &flights {
        assert!(rig.limit.begin(
            flight.0,
            AttemptKind::Ordinary,
            8,
            Own::New(flight.1),
            true,
            0
        ));
    }
    // The last of the three started with the other two in progress.
    rig.ok(flights[2]);
    assert_eq!(rig.limit.setup_limit(), 3);
}

/// `limited(16, 4)` with its four setups in flight again, at `now`.
fn waiting_for_a_test(now: u64) -> (Rig, Vec<Flight>) {
    let (mut rig, _) = limited(16, 4);
    let setups: Vec<Flight> = (0..4).map(|_| rig.go(Start::Ordinary)).collect();
    rig.at(now);
    rig.demand(1, 1);
    (rig, setups)
}

#[test]
fn the_setup_test_waits_for_its_timer_and_for_the_setups_to_fill_ns() {
    let (mut rig, setups) = waiting_for_a_test(499);
    assert!(rig.limit.arm_test(false, 499).is_none(), "T0 is 500 ms");
    rig.at(500);
    // A member on its final attempt never carries a probe.
    assert!(rig.limit.arm_test(true, 500).is_none());
    // Not at Ns setups in flight: ordinary starts fill them first.
    rig.ok(setups[0]);
    assert!(rig.limit.arm_test(false, 500).is_none());
    // Back at four in flight.
    rig.go(Start::Ordinary);
    let (pool_limit, _) = rig.limit.arm_test(false, 500).expect("ready");
    assert_eq!(
        pool_limit,
        rig.limit.n(),
        "the pool's limit is N's: a setup test adds no connection beyond it"
    );
}

#[test]
fn a_fair_setup_test_that_succeeds_raises_ns_by_one_and_climbs() {
    let (mut rig, setups) = waiting_for_a_test(500);
    let test = rig.go(Start::SetupProbe);
    assert_eq!(
        rig.limit.pool_limit(),
        16,
        "the test adds a setup, not a slot"
    );
    assert!(rig.start(Start::Ordinary).is_none(), "one test at a time");
    assert!(
        rig.limit.arm_test(false, 500).is_none(),
        "the slot is taken"
    );
    assert_eq!(rig.ok(test), Ruling::Succeeded);
    assert_eq!(rig.limit.setup_limit(), 5);
    assert_eq!(
        rig.limit.setup_state(),
        State::Discovering,
        "the next is due at once"
    );
    for flight in setups {
        rig.ok(flight);
    }
}

#[test]
fn a_setup_test_that_succeeds_unfair_raises_nothing_and_is_due_again() {
    let (mut rig, setups) = waiting_for_a_test(500);
    let test = rig.go(Start::SetupProbe);
    // One of the four finishes during the test's window: lo_s = 3.
    rig.ok(setups[0]);
    assert_eq!(rig.ok(test), Ruling::Unfair);
    assert_eq!(rig.limit.setup_limit(), 4);
    assert_eq!(rig.limit.setup_state(), State::Stable);
    // Due again as soon as four setups are in flight.
    rig.go(Start::Ordinary);
    assert!(rig.limit.arm_test(false, 500).is_some());
}

#[test]
fn a_fair_refused_setup_test_backs_the_timer_off() {
    let (mut rig, _) = waiting_for_a_test(500);
    let test = rig.go(Start::SetupProbe);
    assert_eq!(rig.refuse(test, SetupDrop), Ruling::RefusedTest);
    assert_eq!(rig.limit.setup_limit(), 4);
    assert_eq!(rig.limit.n(), 16);
    rig.at(1_499);
    assert!(
        rig.limit.arm_test(false, 1_499).is_none(),
        "T doubled to 1 s"
    );
    rig.at(1_500);
    assert!(rig.limit.arm_test(false, 1_500).is_some());
}

#[test]
fn a_setup_test_runs_while_a_probe_whose_base_is_unmet_stays_due() {
    // Case 53: N = 20 (STABLE) with only ten Connected: the probe's base is
    // unmet, so it stays due and unstarted, and the setup test takes the slot.
    let mut rig = Rig::stable(32, 20);
    for id in 6..=20 {
        rig.conn(id, ConnEvent::ServerClosed);
    }
    let wave: Vec<Flight> = (0..6).map(|_| rig.go(Start::Ordinary)).collect();
    assert_eq!(rig.refuse(wave[5], SetupDrop), Ruling::SetupLimit);
    for flight in &wave[..5] {
        rig.ok(*flight);
    }
    assert_eq!((rig.limit.setup_limit(), rig.limit.n()), (5, 20));
    for _ in 0..5 {
        rig.go(Start::Ordinary);
    }
    rig.at(500);
    rig.demand(1, 1);
    let probe = rig.plan().expect("the probe is due");
    assert_eq!(probe.kind, AttemptKind::Probe);
    assert!(!probe.ready, "its base is unmet");
    let (_, arm) = rig.limit.arm_test(false, 500).expect("the setup test runs");
    rig.limit.disarm(arm);
    let test = rig.go(Start::SetupProbe);
    assert!(
        rig.limit.arm_test(false, 500).is_none(),
        "never two tests in flight"
    );
    assert_eq!(rig.ok(test), Ruling::Succeeded);
    assert_eq!(rig.limit.setup_limit(), 6);
}

#[test]
fn a_setup_test_reaching_the_ceiling_is_saturated_and_tests_no_more() {
    let (mut rig, _) = limited(5, 4);
    for id in 1..=4 {
        rig.conn(id, ConnEvent::ServerClosed);
    }
    for _ in 0..4 {
        rig.go(Start::Ordinary);
    }
    rig.at(500);
    rig.demand(1, 1);
    let test = rig.go(Start::SetupProbe);
    assert_eq!(rig.ok(test), Ruling::Succeeded);
    assert_eq!(
        (rig.limit.setup_limit(), rig.limit.setup_state()),
        (5, State::Saturated)
    );
    assert!(rig.limit.arm_test(false, 500).is_none());
}

#[test]
fn a_setup_test_ending_with_no_verdict_is_due_again() {
    let (mut rig, _) = waiting_for_a_test(500);
    let test = rig.go(Start::SetupProbe);
    rig.limit.conn(test.1, ConnEvent::Closing, 500);
    assert_eq!(rig.finish(test.0, Outcome::Ended), Ruling::Ended);
    assert_eq!(rig.limit.setup_state(), State::Stable);
    assert!(
        rig.limit.arm_test(false, 500).is_some(),
        "the four are still in flight"
    );
}

#[test]
fn a_probe_hit_by_a_setup_drop_is_no_input_to_n_and_is_due_again() {
    // The drop is the setup limit's evidence: for `N` the test ended with no
    // verdict, so PROBING returns to STABLE and the probe is re-armed.
    let mut rig = Rig::stable(32, 8);
    rig.at(500);
    rig.demand(1, 1);
    let probe = rig.go(Start::Probe);
    assert_eq!(rig.limit.state(), State::Probing);
    assert_eq!(rig.refuse(probe, SetupDrop), Ruling::RetryMachine);
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
    assert_eq!(rig.plan().map(|plan| plan.kind), Some(AttemptKind::Probe));
}
