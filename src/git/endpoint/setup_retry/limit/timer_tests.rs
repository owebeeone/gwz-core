//! §4.7's probe timer: the gaps, the cap and the jitter.
use super::timer::*;

fn timer(permille: u64) -> Timer {
    Timer::new(Spread::fixed(permille))
}

#[test]
fn refused_tests_at_a_steady_limit_come_at_the_gaps_section_4_7_gives_for_jitter_0_8() {
    let mut timer = timer(800);
    timer.arm(0);
    let mut at = Vec::new();
    for _ in 0..8 {
        let deadline = timer.deadline().expect("armed");
        at.push(deadline);
        timer.back_off();
        timer.arm(deadline);
    }
    assert_eq!(
        at,
        [400, 1_200, 2_800, 6_000, 12_400, 25_200, 49_200, 73_200]
    );
}

#[test]
fn the_first_expiry_is_t0_scaled_by_the_jitter_factor() {
    for (permille, want) in [(800, 400), (1_000, 500), (1_200, 600)] {
        let mut timer = timer(permille);
        timer.arm(0);
        assert_eq!(timer.deadline(), Some(want));
    }
}

#[test]
fn a_draw_outside_0_8_to_1_2_is_clamped() {
    let mut low = timer(10);
    low.arm(0);
    assert_eq!(low.deadline(), Some(400));
    let mut high = timer(9_000);
    high.arm(0);
    assert_eq!(high.deadline(), Some(600));
}

#[test]
fn reset_returns_the_gap_to_t0_and_the_cap_is_thirty_seconds() {
    let mut timer = timer(1_000);
    for _ in 0..10 {
        timer.back_off();
    }
    timer.arm(0);
    assert_eq!(timer.deadline(), Some(TMAX_MS));
    timer.reset();
    timer.arm(100);
    assert_eq!(timer.deadline(), Some(100 + T0_MS));
}

#[test]
fn a_timer_is_expired_at_its_deadline_made_due_at_once_and_disarmed_never() {
    let mut timer = timer(1_000);
    assert!(!timer.expired(u64::MAX), "unarmed");
    timer.arm(0);
    assert!(!timer.expired(499));
    assert!(timer.expired(500));
    timer.disarm();
    assert_eq!((timer.deadline(), timer.expired(1_000)), (None, false));
    timer.make_due(700);
    assert!(timer.expired(700));
}
