//! RESTORING (adaptive concurrency design §4.6, §5.5, §10.2 cases 27, 37 and
//! 54): after an outage that lowered `N`, judged doubling steps back up to
//! `N_good`. One key on a scripted clock.
use super::{
    control::{Outcome, Ruling, Start},
    control_tests::{Flight, Rig},
    filter::Signal::{Suspect, Throttle},
    fsm::State,
    windows::AttemptKind,
};

/// A restore step's start, as the host begins it: only while the key has room
/// below the step's target `S`.
fn step_start(rig: &mut Rig) -> Flight {
    let (kind, target) = rig.limit.start_kind();
    assert_eq!(kind, AttemptKind::Restore);
    assert!(
        rig.limit.admits_ordinary(rig.now),
        "room below S = {target}"
    );
    rig.go(Start::Restore { target })
}
fn outage_ends(rig: &mut Rig) {
    rig.limit.outage();
    rig.limit.recovered(rig.now);
}

#[test]
fn an_outage_that_lowered_n_is_restored_by_doubling_steps_up_to_the_ceiling() {
    // Ceiling 8, N = 2 with two connected, and no success before the outage:
    // N_good is the ceiling.
    let mut rig = Rig::stable(8, 2);
    outage_ends(&mut rig);
    assert_eq!(rig.limit.state(), State::Restoring);
    // S = max(2, min(8, 2 x 2)) = 4: room for two more, and no third.
    assert_eq!(rig.limit.pool_limit(), 4);
    let first = [step_start(&mut rig), step_start(&mut rig)];
    assert!(!rig.limit.admits_ordinary(rig.now), "the step is full");
    assert_eq!(rig.ok(first[0]), Ruling::Succeeded);
    assert_eq!(rig.limit.state(), State::Restoring, "a step is not done");
    assert_eq!(rig.ok(first[1]), Ruling::Succeeded);
    // N = Connected = 4, and the next step doubles: S = min(8, 2 x 4).
    assert_eq!((rig.limit.n(), rig.limit.state()), (4, State::Restoring));
    assert_eq!(rig.limit.pool_limit(), 8);
    let second: Vec<_> = (0..4).map(|_| step_start(&mut rig)).collect();
    for flight in second {
        rig.ok(flight);
    }
    // N reached N_good = C (case 37): SATURATED, and the restore is over.
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Saturated));
    assert!(!rig.limit.restoring());
}

#[test]
fn reaching_n_good_below_the_ceiling_is_stable_with_the_timer_at_t0() {
    // N_good = 6 (the N at the last success), then an Overload to 3.
    let mut rig = Rig::stable(32, 6);
    rig.limit.observed_success(0);
    for id in 1..=3 {
        rig.conn(id, super::states::ConnEvent::ServerClosed);
    }
    let a = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(a, Throttle), Ruling::Overload { n: 3 });
    rig.at(1_000);
    outage_ends(&mut rig);
    assert_eq!(rig.limit.state(), State::Restoring);
    assert_eq!(rig.limit.pool_limit(), 6, "S = max(3, min(6, 2 x 3))");
    let starts: Vec<_> = (0..3).map(|_| step_start(&mut rig)).collect();
    for flight in starts {
        rig.ok(flight);
    }
    assert_eq!((rig.limit.n(), rig.limit.state()), (6, State::Stable));
    assert_eq!(rig.limit.next_deadline(1_000), Some(1_500), "T := T0");
}

#[test]
fn a_step_with_a_refusal_and_no_overload_ends_the_restore_and_a_confirmation_settles_it() {
    // Case 27b: N = 4 of 32, the host returns with a limit of 6. The first
    // step targets 8: four starts, two succeed (N = 6), two are refused.
    let mut rig = Rig::stable(32, 4);
    outage_ends(&mut rig);
    assert_eq!(rig.limit.pool_limit(), 8);
    let steps: Vec<_> = (0..4).map(|_| step_start(&mut rig)).collect();
    rig.ok(steps[0]);
    rig.ok(steps[1]);
    // hi = 4 connected + 3 other starts = 7 < 8: conclusive. A reset is a
    // Suspect, so it opens a confirmation; the second refusal is its
    // co-refusal.
    assert_eq!(rig.refuse(steps[2], Suspect), Ruling::Confirmation);
    assert_eq!(rig.limit.state(), State::Restoring, "a step is in flight");
    assert_eq!(rig.refuse(steps[3], Suspect), Ruling::Inconclusive);
    assert_eq!((rig.limit.n(), rig.limit.state()), (6, State::Stable));
    assert!(!rig.limit.restoring());
    assert_eq!(rig.limit.next_deadline(0), Some(500), "T := T0");
    // The confirmation tests at k = 7 and is refused fairly: Overload at 6.
    rig.demand(3, 3);
    let plan = rig.plan().unwrap();
    assert_eq!((plan.kind, plan.target), (AttemptKind::Confirming, 7));
    let test = rig.go(Start::Confirming);
    assert_eq!(rig.refuse(test, Suspect), Ruling::Overload { n: 6 });
    assert_eq!((rig.limit.n(), rig.limit.state()), (6, State::Stable));
}

#[test]
fn a_throttled_step_start_is_an_overload_at_once_and_ends_the_restore() {
    let mut rig = Rig::stable(32, 4);
    outage_ends(&mut rig);
    let steps: Vec<_> = (0..4).map(|_| step_start(&mut rig)).collect();
    rig.ok(steps[0]);
    rig.ok(steps[1]);
    assert_eq!(rig.refuse(steps[2], Throttle), Ruling::Overload { n: 6 });
    assert_eq!((rig.limit.n(), rig.limit.state()), (6, State::Stable));
    assert!(!rig.limit.restoring(), "the Overload ended it");
    // A step start is judged against the step's target, not the new N: the
    // wave's later refusal is an Overload too, to the same value.
    assert_eq!(rig.refuse(steps[3], Throttle), Ruling::Overload { n: 6 });
    assert_eq!(rig.limit.n(), 6);
}

#[test]
fn the_probe_timer_is_suspended_during_a_restore_and_starts_again_at_t0_after_it() {
    // Case 54: a probe 100 ms from due when the restore begins.
    let mut rig = Rig::stable(8, 3);
    rig.at(400);
    outage_ends(&mut rig);
    rig.demand(3, 3);
    rig.at(600);
    assert_eq!(rig.plan(), None, "no probe is due while restoring");
    assert_eq!(rig.limit.next_deadline(600), None, "and none is awaited");
    let steps: Vec<_> = (0..3).map(|_| step_start(&mut rig)).collect();
    rig.ok(steps[0]);
    assert_eq!(rig.refuse(steps[1], Throttle), Ruling::Overload { n: 4 });
    rig.refuse(steps[2], Throttle);
    assert_eq!(
        rig.limit.next_deadline(600),
        Some(1_100),
        "T := T0 from the exit"
    );
}

#[test]
fn with_no_member_to_carry_a_step_the_restore_is_left_to_the_probe_timer() {
    let mut rig = Rig::stable(8, 3);
    outage_ends(&mut rig);
    // Three members want a connection and every one is on its final attempt.
    rig.demand(3, 0);
    assert_eq!(
        (rig.limit.state(), rig.limit.restoring()),
        (State::Stable, false)
    );
    assert_eq!(rig.limit.next_deadline(0), Some(500));
}

#[test]
fn nothing_is_restored_without_a_lowered_n_an_outage_or_a_budget() {
    // N is still N_good: nothing to restore.
    let mut rig = Rig::new(8);
    rig.limit.outage();
    rig.limit.recovered(0);
    assert_eq!(rig.limit.state(), State::Saturated);
    // No outage began.
    let mut rig = Rig::stable(8, 2);
    rig.limit.recovered(0);
    assert_eq!(rig.limit.state(), State::Stable);
    // `--max-retries 0` could never check the steps.
    let mut rig = Rig::with(8, false, 1_000);
    rig.connect(2);
    rig.limit.outage();
    rig.limit.recovered(0);
    assert!(!rig.limit.restoring());
}

#[test]
fn n_good_is_frozen_at_the_outages_start_whatever_succeeds_after() {
    // Case 27d: N_good is the N in force at the last success before the
    // outage, not a later one.
    let mut rig = Rig::stable(32, 4);
    rig.limit.outage();
    rig.connect(2);
    rig.limit.observed_success(0);
    assert_eq!(rig.limit.n(), 6);
    rig.limit.outage();
    rig.limit.recovered(0);
    // N_good = 32 (no success before the outage): S = min(32, 2 x 6).
    assert_eq!(rig.limit.state(), State::Restoring);
    assert_eq!(rig.limit.pool_limit(), 12);
}

#[test]
fn a_restore_step_start_is_judged_against_its_own_target_not_n() {
    // hi must stay below S, the step's target, for a refusal to be evidence.
    let mut rig = Rig::stable(32, 4);
    outage_ends(&mut rig);
    let a = step_start(&mut rig);
    let outcome = Outcome::Refused {
        signal: Throttle,
        retry_after_ms: None,
        post: false,
    };
    rig.conn(6, super::states::ConnEvent::SetupEnded);
    // hi = 4 connected < S = 8: conclusive, so an Overload (not Inconclusive,
    // which a comparison with N = 4 would give).
    assert_eq!(rig.finish(a.0, outcome), Ruling::Overload { n: 4 });
}

#[test]
fn n_good_is_the_n_at_the_last_setup_that_succeeded_through_its_own_result() {
    // N = 4 of 8 (STABLE). A connection leaves, and an ordinary start takes
    // its place and succeeds: N_good is 4, not the ceiling it started at.
    let mut rig = Rig::stable(8, 4);
    rig.conn(1, super::states::ConnEvent::ServerClosed);
    let replacement = rig.go(Start::Ordinary);
    assert_eq!(rig.ok(replacement), Ruling::Succeeded);
    // Two more leave, and a throttle takes N to 2.
    rig.conn(2, super::states::ConnEvent::ServerClosed);
    rig.conn(3, super::states::ConnEvent::ServerClosed);
    let refused = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(refused, Throttle), Ruling::Overload { n: 2 });
    outage_ends(&mut rig);
    assert_eq!(rig.limit.state(), State::Restoring);
    // One step of two starts reaches N_good = 4, which is short of the
    // ceiling: STABLE, not another step toward 8.
    let steps: Vec<_> = (0..2).map(|_| step_start(&mut rig)).collect();
    for flight in steps {
        rig.ok(flight);
    }
    assert_eq!((rig.limit.n(), rig.limit.state()), (4, State::Stable));
}
