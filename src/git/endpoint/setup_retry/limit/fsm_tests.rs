//! §4.6's machine, transition by transition, and its invariants.
use super::fsm::{State::*, *};

fn stable(ceiling: usize, n: usize) -> Fsm {
    let mut fsm = Fsm::new(ceiling);
    fsm.overload(n, n);
    fsm
}

#[test]
fn it_starts_saturated_at_the_ceiling() {
    let fsm = Fsm::new(32);
    assert_eq!((fsm.n(), fsm.state()), (32, Saturated));
}

#[test]
fn an_overload_sets_n_to_the_smaller_of_connected_and_hi_with_a_floor_of_one() {
    for (connected, hi, want) in [(3, 31, 3), (8, 7, 7), (8, 8, 8), (0, 5, 1), (5, 0, 1)] {
        let mut fsm = Fsm::new(32);
        fsm.overload(connected, hi);
        assert_eq!((fsm.n(), fsm.state()), (want, Stable), "{connected} {hi}");
    }
}

#[test]
fn an_overload_from_every_state_ends_in_stable() {
    for from in [Discovering, Stable, Probing, Saturated] {
        let mut fsm = stable(32, 8);
        match from {
            Saturated => fsm = Fsm::new(32),
            Discovering => {
                fsm.test_started();
                fsm.test_succeeded(9, true);
            }
            Probing => fsm.test_started(),
            Stable => {}
        }
        assert_eq!(fsm.state(), from);
        fsm.overload(5, 6);
        assert_eq!((fsm.n(), fsm.state()), (5, Stable), "{from:?}");
    }
}

#[test]
fn a_success_raises_n_to_connected_and_never_lowers_it() {
    let mut fsm = stable(32, 8);
    fsm.success(5);
    assert_eq!(fsm.n(), 8, "fewer connections is no evidence");
    fsm.success(11);
    assert_eq!((fsm.n(), fsm.state()), (11, Stable));
}

#[test]
fn n_reaching_the_ceiling_by_any_route_is_saturated() {
    let mut fsm = stable(32, 8);
    fsm.success(40);
    assert_eq!((fsm.n(), fsm.state()), (32, Saturated));
    let mut fsm = Fsm::new(4);
    fsm.overload(4, 9);
    assert_eq!(
        (fsm.n(), fsm.state()),
        (4, Saturated),
        "overload at the ceiling"
    );
}

#[test]
fn a_fair_successful_probe_moves_to_discovering_or_saturated() {
    let mut fsm = stable(32, 8);
    fsm.test_started();
    assert_eq!(fsm.state(), Probing);
    fsm.test_succeeded(9, true);
    assert_eq!((fsm.n(), fsm.state()), (9, Discovering));
    fsm.test_started();
    assert_eq!(
        fsm.state(),
        Discovering,
        "a discovering test stays discovering"
    );
    fsm.test_succeeded(32, true);
    assert_eq!((fsm.n(), fsm.state()), (32, Saturated));
}

#[test]
fn an_unfair_successful_probe_raises_nothing_and_returns_to_stable() {
    // R2: one left while the test ran, so Connected is N again.
    let mut fsm = stable(32, 8);
    fsm.test_started();
    fsm.test_succeeded(8, false);
    assert_eq!((fsm.n(), fsm.state()), (8, Stable));
}

#[test]
fn a_refused_probe_returns_to_stable_and_says_how_the_timer_moves() {
    let mut fsm = stable(32, 8);
    fsm.test_started();
    assert_eq!(fsm.probe_refused(), Backoff::Double);
    assert_eq!((fsm.n(), fsm.state()), (8, Stable));
    fsm.test_started();
    fsm.test_succeeded(9, true);
    fsm.test_started();
    assert_eq!(fsm.probe_refused(), Backoff::Reset);
    assert_eq!((fsm.n(), fsm.state()), (9, Stable));
}

#[test]
fn a_test_that_said_nothing_leaves_probing_and_discovering_as_they_were_or_stable() {
    let mut fsm = stable(32, 8);
    fsm.test_started();
    fsm.test_inconclusive();
    assert_eq!(fsm.state(), Stable);
    fsm.test_started();
    fsm.test_succeeded(9, true);
    fsm.test_started();
    fsm.test_inconclusive();
    assert_eq!(fsm.state(), Discovering);
}

#[test]
fn saturated_is_exactly_n_equal_to_the_ceiling_whatever_the_sequence() {
    // Every sequence of up to five operations over a ceiling of 3.
    type Op = fn(&mut Fsm);
    let ops: [Op; 9] = [
        |f| f.success(3),
        |f| f.success(1),
        |f| f.overload(2, 2),
        |f| f.overload(3, 9),
        |f| f.test_started(),
        |f| f.test_succeeded(1, true),
        |f| f.test_succeeded(2, false),
        |f| {
            f.probe_refused();
        },
        |f| f.test_inconclusive(),
    ];
    for code in 0..ops.len().pow(5) {
        let mut fsm = Fsm::new(3);
        let mut code = code;
        for _ in 0..5 {
            let op = ops[code % ops.len()];
            code /= ops.len();
            op(&mut fsm);
            assert!((1..=3).contains(&fsm.n()));
            assert_eq!(fsm.n() == 3, fsm.state() == Saturated, "{:?}", fsm.state());
        }
    }
}
