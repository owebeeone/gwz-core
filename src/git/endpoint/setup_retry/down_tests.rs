//! The retry machine's Down state (adaptive concurrency design §5.5, §10.2
//! cases 28, 29 and 32): a transient failure does not close the key. The
//! machine is driven by hand, as `machine_tests` drives it, with fixed
//! jitter.
use super::{
    machine::{Change, Final},
    *,
};
use gwz_transport::protocol::{Effect, ErrorCode, Failure, SetupFailureCause};

const JITTER: u64 = 7;

fn failure(code: ErrorCode, setup_cause: Option<SetupFailureCause>) -> Failure {
    Failure {
        detail: None,
        setup_cause,
        code,
        effect: Effect::None,
        facts: None,
    }
}
fn stall() -> Failure {
    failure(ErrorCode::Timeout, Some(SetupFailureCause::Stall))
}
fn last(attempt: u32) -> Final {
    Final {
        failure: stall(),
        attempt,
        attempts: 4,
    }
}
/// Starts `member`, which the key must allow.
fn start(key: &mut Machine<&'static str>, member: &'static str, now: u64) {
    assert_eq!(key.decide(now), Decision::Start, "{member} at {now}");
    key.start(member);
}
/// A key whose budget (`--max-retries 3`) was spent by one member at `now`,
/// which is Down: its first retest is due 30 s on.
fn down(now: u64) -> Machine<&'static str> {
    let mut key = Machine::new(3);
    let mut at = 0;
    for attempt in 1..=4 {
        start(&mut key, "spent", at);
        at += 100;
        let outcome = key.failed(&"spent", Verdict::Retry, stall(), at, JITTER);
        if attempt < 4 {
            assert_eq!(outcome, Outcome::Retry);
            at = key.wake_at().unwrap();
        } else {
            assert_eq!(outcome, Outcome::Sweep(last(4)));
        }
    }
    assert!(at <= now);
    key
}

#[test]
fn exhaustion_puts_the_key_down_and_arrivals_park_until_the_retest_is_due() {
    let mut key = down(10_000);
    let retest_at = key.wake_at().unwrap();
    // The queued members were finished by the endpoint with the sweep; an
    // arrival parks rather than fails.
    assert_eq!(key.decide(retest_at - 1), Decision::Wait);
    assert_eq!(key.decide(retest_at), Decision::Start);
    // Only one retest at a time: the next member shares its result.
    key.start("carrier");
    assert_eq!(key.decide(retest_at), Decision::Wait);
    assert_eq!(key.decide(retest_at + 1), Decision::Wait);
}

#[test]
fn a_retest_that_succeeds_heals_the_key_and_the_parked_members_proceed() {
    let mut key = down(10_000);
    let retest_at = key.wake_at().unwrap();
    assert_eq!(key.take_change(), Some(Change::Left), "the outage began");
    start(&mut key, "carrier", retest_at);
    assert!(key.succeeded(&"carrier", true));
    assert_eq!(key.take_change(), Some(Change::Healed));
    assert_eq!(key.decide(retest_at + 1), Decision::Start, "Healthy");
    // And it counts from zero: a later failure is attempt 1 again.
    start(&mut key, "later", retest_at + 1);
    assert_eq!(
        key.failed(&"later", Verdict::Retry, stall(), retest_at + 2, JITTER),
        Outcome::Retry
    );
    assert_eq!(key.wake_at(), Some(retest_at + 2 + 1_000 + JITTER));
}

#[test]
fn a_failed_retest_is_shared_and_the_next_is_30_seconds_on_and_two_in_a_row_give_up() {
    let mut key = down(10_000);
    let first = key.wake_at().unwrap();
    start(&mut key, "one", first);
    // The retest's failure goes to its carrier and to every member parked:
    // the endpoint sweeps its queue with it, and it carries no count.
    assert_eq!(
        key.failed(&"one", Verdict::Retry, stall(), first + 50, JITTER),
        Outcome::Sweep(last(0))
    );
    let second = first + 50 + 30_000;
    assert_eq!(key.wake_at(), Some(second));
    // Members park again for the second retest.
    assert_eq!(key.decide(second - 1), Decision::Wait);
    start(&mut key, "two", second);
    assert_eq!(
        key.failed(&"two", Verdict::Retry, stall(), second + 50, JITTER),
        Outcome::Sweep(last(0))
    );
    // From then on, before the next retest is due, a member finishes at once
    // with the recorded failure and no attempt of its own.
    let third = second + 50 + 30_000;
    assert_eq!(key.decide(second + 51), Decision::Finish(last(0)));
    assert_eq!(key.decide(third - 1), Decision::Finish(last(0)));
    // A member that selects the key at or after the retest is due carries it.
    start(&mut key, "three", third);
    assert_eq!(key.decide(third), Decision::Wait);
    // A retest that succeeds serves arrivals again (case 29b).
    assert!(key.succeeded(&"three", true));
    assert_eq!(key.decide(third + 1), Decision::Start);
}

#[test]
fn a_permanent_failure_of_a_retest_closes_the_key_for_good() {
    let mut key = down(10_000);
    let at = key.wake_at().unwrap();
    start(&mut key, "carrier", at);
    let refused = failure(ErrorCode::Authentication, None);
    let closed = Final {
        failure: refused.clone(),
        attempt: 0,
        attempts: 4,
    };
    assert_eq!(
        key.failed(&"carrier", Verdict::Close, refused, at + 1, JITTER),
        Outcome::Finish(closed.clone())
    );
    assert_eq!(key.decide(at + 1_000_000), Decision::Finish(closed));
}

#[test]
fn a_retest_that_ends_without_a_verdict_or_leases_an_idle_connection_leaves_the_next_to_carry() {
    let mut key = down(10_000);
    let at = key.wake_at().unwrap();
    // Cancelled, or an allocation timeout (a Return): no verdict, no count.
    start(&mut key, "a", at);
    assert_eq!(
        key.failed(&"a", Verdict::Return, stall(), at + 1, JITTER),
        Outcome::Return
    );
    // It leased an idle connection: that is no setup, the key stays Down.
    start(&mut key, "b", at + 1);
    assert!(key.succeeded(&"b", false));
    assert_eq!(key.take_change(), Some(Change::Left), "not healed");
    start(&mut key, "c", at + 1);
    key.abandoned(&"c");
    assert_eq!(
        key.decide(at + 2),
        Decision::Start,
        "still due, none in flight"
    );
}

#[test]
fn an_attempt_from_before_the_key_went_down_counts_nothing_and_heals_nothing() {
    let mut key = Machine::new(0);
    for member in ["old", "older", "spent"] {
        start(&mut key, member, 0);
    }
    // R = 0: the first failure exhausts the budget.
    let spent = Final {
        failure: stall(),
        attempt: 1,
        attempts: 1,
    };
    assert_eq!(
        key.failed(&"spent", Verdict::Retry, stall(), 5, JITTER),
        Outcome::Sweep(spent)
    );
    // An older attempt that fails parks with the others; it is not a retest
    // and moves nothing.
    assert_eq!(
        key.failed(&"old", Verdict::Retry, stall(), 6, JITTER),
        Outcome::Retry
    );
    assert_eq!(key.wake_at(), Some(5 + 30_000));
    // One that succeeds serves its own member, is not admitted for reuse, and
    // does not heal the key: only a retest does.
    assert!(!key.succeeded(&"older", true));
    assert_eq!(key.decide(7), Decision::Wait);
}

#[test]
fn leaving_cold_or_healthy_is_reported_once_and_a_cold_success_is_no_recovery() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    assert!(key.succeeded(&"a", true));
    assert_eq!(key.take_change(), None, "Cold to Healthy is no recovery");
    start(&mut key, "b", 1);
    key.failed(&"b", Verdict::Retry, stall(), 2, JITTER);
    assert_eq!(key.take_change(), Some(Change::Left));
    assert_eq!(key.take_change(), None, "reported once");
    // Waiting to Degraded to Healthy is a recovery.
    let wake = key.wake_at().unwrap();
    start(&mut key, "c", wake);
    assert!(key.succeeded(&"c", true));
    assert_eq!(key.take_change(), Some(Change::Healed));
}

#[test]
fn going_down_is_told_once_per_episode_and_a_failed_retest_is_the_same_episode() {
    // The limit machine's "not answering" note (§9) is made once when the
    // budget is spent, not when the key first leaves Healthy and not again
    // when a retest fails.
    let mut key = Machine::new(3);
    start(&mut key, "first", 0);
    assert_eq!(
        key.failed(&"first", Verdict::Retry, stall(), 100, JITTER),
        Outcome::Retry
    );
    assert!(!key.take_down(), "retrying is not Down");
    let mut key = down(10_000);
    assert!(key.take_down());
    assert!(!key.take_down(), "told once");
    let retest_at = key.wake_at().unwrap();
    start(&mut key, "carrier", retest_at);
    assert!(matches!(
        key.failed(&"carrier", Verdict::Retry, stall(), retest_at + 10, JITTER),
        Outcome::Sweep(_)
    ));
    assert!(!key.take_down(), "a failed retest is the same episode");
    // Healed, then spent again: a new episode.
    let retest_at = key.wake_at().unwrap();
    start(&mut key, "healer", retest_at);
    assert!(key.succeeded(&"healer", true));
    let mut at = retest_at + 100;
    for attempt in 1..=4 {
        start(&mut key, "again", at);
        at += 100;
        let outcome = key.failed(&"again", Verdict::Retry, stall(), at, JITTER);
        if attempt < 4 {
            assert_eq!(outcome, Outcome::Retry);
            at = key.wake_at().unwrap();
        }
    }
    assert!(key.take_down(), "a new episode");
}

#[test]
fn a_key_spent_by_its_first_failure_is_down_and_has_left_healthy_in_one_call() {
    // `--max-retries 0`: one call leaves Cold and goes Down; neither report
    // hides the other.
    let mut key = Machine::new(0);
    start(&mut key, "only", 0);
    assert!(matches!(
        key.failed(&"only", Verdict::Retry, stall(), 100, JITTER),
        Outcome::Sweep(_)
    ));
    assert_eq!(key.take_change(), Some(Change::Left));
    assert!(key.take_down());
}

#[test]
fn a_retest_requeued_as_evidence_counts_as_a_failed_retest_and_paces_the_next() {
    // State P2-7: the retest's connect is refused while another connection
    // counts in its window, so the site's machine takes it as evidence and the
    // member is requeued. The key must not run the next handshake at once.
    let mut key = down(10_000);
    let first = key.wake_at().unwrap();
    start(&mut key, "carrier", first);
    key.retest_refused(&"carrier", first + 5);
    // A failed retest: the next is 30 s on, and arrivals park until then.
    let second = first + 5 + 30_000;
    assert_eq!(key.wake_at(), Some(second));
    assert_eq!(key.decide(first + 6), Decision::Wait);
    assert_eq!(key.decide(second - 1), Decision::Wait);
    assert_eq!(key.decide(second), Decision::Start);
    // Two such retests in a row give up, as two failed ones do.
    start(&mut key, "again", second);
    key.retest_refused(&"again", second);
    let third = second + 30_000;
    assert_eq!(key.decide(third - 1), Decision::Finish(last(0)));
    assert_eq!(key.decide(third), Decision::Start);
}

#[test]
fn a_probe_ended_as_evidence_outside_down_is_released_as_abandoned_is() {
    // Only a Down key's retest is paced; a Degraded probe that is requeued
    // frees the key for the next, as before.
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    key.failed(&"a", Verdict::Retry, stall(), 1, JITTER);
    let wake = key.wake_at().unwrap();
    start(&mut key, "probe", wake);
    assert_eq!(key.decide(wake), Decision::Wait);
    key.retest_refused(&"probe", wake + 1);
    assert_eq!(key.decide(wake + 1), Decision::Start);
}
