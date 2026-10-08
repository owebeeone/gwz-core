//! §4.4's judgement and §4.5's evidence filter, as tables.
use super::{filter::*, windows::AttemptKind};
use AttemptKind::{Confirming, Ordinary, Probe, Restore};
use Signal::{Suspect, Throttle};

fn refusal(kind: AttemptKind, signal: Signal, target: usize, lo: usize, hi: usize) -> Refusal {
    Refusal {
        kind,
        signal,
        target,
        lo,
        hi,
        adaptive: true,
    }
}

#[test]
fn an_ordinary_refusal_is_conclusive_iff_hi_is_below_its_target() {
    for target in 1..=33 {
        for hi in 0..=40 {
            let want = if hi < target {
                Judgement::Conclusive
            } else {
                Judgement::Inconclusive
            };
            assert_eq!(judge(Ordinary, target, 0, hi), want, "N {target} hi {hi}");
            assert_eq!(judge(Restore, target, 0, hi), want, "S {target} hi {hi}");
        }
    }
}

#[test]
fn a_wave_has_one_conclusive_refusal_and_the_rest_inconclusive() {
    // Case 15, R3: C = 32 and hi = 31; after the first Overload N = 3.
    assert_eq!(judge(Ordinary, 32, 0, 31), Judgement::Conclusive);
    assert_eq!(judge(Ordinary, 3, 0, 31), Judgement::Inconclusive);
}

#[test]
fn a_restore_step_is_judged_against_its_step_target() {
    // Case 27b: the step targets 8; hi = 7 refuses conclusively.
    assert_eq!(judge(Restore, 8, 0, 7), Judgement::Conclusive);
    assert_eq!(judge(Restore, 8, 0, 8), Judgement::Inconclusive);
}

#[test]
fn a_probe_test_is_fair_iff_all_n_stayed_connected_through_its_window() {
    // Target N + 1 = 9.
    assert_eq!(judge(Probe, 9, 8, 8), Judgement::Fair);
    assert_eq!(
        judge(Probe, 9, 7, 9),
        Judgement::Unfair,
        "case 23, R2: one left"
    );
    assert!(fair(9, 8));
    assert!(!fair(9, 7));
}

#[test]
fn a_confirming_test_is_fair_iff_lo_is_k_minus_one_and_k_one_is_not_a_limit() {
    assert_eq!(judge(Confirming, 9, 8, 8), Judgement::Fair);
    assert_eq!(judge(Confirming, 9, 5, 9), Judgement::Unfair);
    assert_eq!(judge(Confirming, 1, 0, 0), Judgement::NotALimit);
}

#[test]
fn a_conclusive_throttle_is_an_overload_and_a_conclusive_suspect_opens_a_confirmation() {
    assert_eq!(
        evidence(&refusal(Ordinary, Throttle, 32, 0, 31)),
        Evidence::Overload
    );
    assert_eq!(
        evidence(&refusal(Ordinary, Suspect, 32, 0, 31)),
        Evidence::Confirm
    );
    assert_eq!(
        evidence(&refusal(Restore, Throttle, 8, 0, 7)),
        Evidence::Overload
    );
    assert_eq!(
        evidence(&refusal(Restore, Suspect, 8, 0, 7)),
        Evidence::Confirm
    );
}

#[test]
fn an_inconclusive_refusal_is_no_input_whatever_its_signal() {
    for signal in [Throttle, Suspect] {
        assert_eq!(
            evidence(&refusal(Ordinary, signal, 3, 0, 31)),
            Evidence::Inconclusive
        );
        assert_eq!(
            evidence(&refusal(Restore, signal, 8, 0, 8)),
            Evidence::Inconclusive
        );
    }
}

#[test]
fn a_refusal_with_nothing_else_counted_is_the_retry_machines() {
    // Case 6c, §4.8: hi = 0.
    for signal in [Throttle, Suspect] {
        for kind in [Ordinary, Restore] {
            assert_eq!(
                evidence(&refusal(kind, signal, 8, 0, 0)),
                Evidence::RetryMachine
            );
        }
    }
}

#[test]
fn at_max_retries_zero_a_conclusive_refusal_only_sets_the_hold() {
    // Case 11, §5.3: no decrease and no confirmation; hi = 0 is still the
    // retry machine's.
    for signal in [Throttle, Suspect] {
        let mut r = refusal(Ordinary, signal, 32, 0, 31);
        r.adaptive = false;
        assert_eq!(evidence(&r), Evidence::HoldOnly);
        r.hi = 0;
        assert_eq!(evidence(&r), Evidence::RetryMachine);
        r.hi = 40;
        assert_eq!(evidence(&r), Evidence::Inconclusive);
    }
}

#[test]
fn a_refused_test_is_judged_by_fairness_not_by_hi_or_signal() {
    for signal in [Throttle, Suspect] {
        // A probe: fair backs the timer off; unfair is re-armed.
        assert_eq!(
            evidence(&refusal(Probe, signal, 9, 8, 8)),
            Evidence::RefusedTest
        );
        assert_eq!(
            evidence(&refusal(Probe, signal, 9, 7, 9)),
            Evidence::UnfairTest
        );
        // A confirming test: fair is an Overload; unfair is re-armed; k = 1
        // goes to the retry machine.
        assert_eq!(
            evidence(&refusal(Confirming, signal, 9, 8, 8)),
            Evidence::Overload
        );
        assert_eq!(
            evidence(&refusal(Confirming, signal, 9, 7, 8)),
            Evidence::UnfairTest
        );
        assert_eq!(
            evidence(&refusal(Confirming, signal, 1, 0, 0)),
            Evidence::RetryMachine
        );
    }
}
