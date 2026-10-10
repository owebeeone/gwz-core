//! The retry plan's S3.1 sentences, read with amendment 2's §3.20 (OD18), on
//! one key's machine. Time is a number the tests advance; jitter is fixed.
use super::{backoff::wait_ms, machine::Final, *};
use gwz_transport::protocol::{Effect, ErrorCode, Failure, SetupFailureCause};
use std::collections::VecDeque;

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
fn authentication() -> Failure {
    failure(ErrorCode::Authentication, None)
}
fn last(failure: Failure, attempt: u32, attempts: u32) -> Final {
    Final {
        failure,
        attempt,
        attempts,
    }
}
/// Starts `member`, which the key must allow.
fn start<M: PartialEq>(key: &mut Machine<M>, member: M, now: u64) {
    assert_eq!(key.decide(now), Decision::Start);
    key.start(member);
}

#[test]
fn the_waits_double_from_one_second_and_stop_at_thirty() {
    let waits: Vec<_> = (1..=7).map(wait_ms).collect();
    assert_eq!(waits, [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
    assert_eq!(wait_ms(u32::MAX), 30_000);
    assert_eq!(wait_bound_ms(0), 0);
    // At the defaults the waits add 7 s, and at most 0.75 s of jitter.
    assert_eq!(wait_bound_ms(3), 7_750);
    assert_eq!(wait_bound_ms(6), 61_000 + 6 * 250);
    assert!(wait_bound_ms(u32::MAX) > wait_bound_ms(u32::MAX - 1));
}

#[test]
fn a_stall_on_attempt_one_does_not_finish_the_member_and_attempt_two_starts_at_its_wake() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    assert_eq!(
        key.failed(&"a", Verdict::Retry, stall(), 100, JITTER),
        Outcome::Retry
    );
    assert_eq!(key.wake_at(), Some(100 + 1_000 + JITTER));
    assert_eq!(key.decide(100 + 1_000 + JITTER - 1), Decision::Wait);
    assert_eq!(key.decide(100 + 1_000 + JITTER), Decision::Start);
}

/// One member alone on a dead key: each attempt stalls after `stall_ms`, and
/// the member waits out each wake. Returns its waits and its final failure.
fn alone(max_retries: u32, stall_ms: u64) -> (Vec<u64>, u32, Final) {
    let mut key = Machine::new(max_retries);
    let mut now = 0;
    let mut waits = Vec::new();
    for attempt in 1.. {
        start(&mut key, "a", now);
        now += stall_ms;
        match key.failed(&"a", Verdict::Retry, stall(), now, JITTER) {
            Outcome::Retry => {
                let wake = key.wake_at().expect("a retried member waits for a wake");
                waits.push(wake - now);
                now = wake;
            }
            Outcome::Sweep(last) => {
                // Down, not closed: the next arrival would carry a retest.
                assert_eq!(key.decide(now), Decision::Wait, "parked until the retest");
                assert_eq!(key.decide(now + 30_000), Decision::Start);
                return (waits, attempt, last);
            }
            Outcome::Finish(_) | Outcome::Return => panic!("a stall in setup is a setup outcome"),
        }
    }
    unreachable!()
}

#[test]
fn attempt_four_finishes_the_member_with_its_stall_as_attempt_four_of_four() {
    let (waits, attempts, finished) = alone(3, 9_000);
    assert_eq!(waits, [1_000 + JITTER, 2_000 + JITTER, 4_000 + JITTER]);
    assert_eq!(attempts, 4);
    assert_eq!(finished, last(stall(), 4, 4));
}

#[test]
fn max_retries_zero_finishes_on_the_first_stall_and_one_allows_two_attempts() {
    let (waits, attempts, finished) = alone(0, 9_000);
    assert!(waits.is_empty());
    assert_eq!((attempts, finished), (1, last(stall(), 1, 1)));
    let (waits, attempts, finished) = alone(1, 9_000);
    assert_eq!(waits, [1_000 + JITTER]);
    assert_eq!((attempts, finished), (2, last(stall(), 2, 2)));
}

#[test]
fn an_authentication_failure_waits_for_nothing_and_opens_no_handshake_beyond_the_wave() {
    let mut key = Machine::new(3);
    // A first wave of eight, at the per-host limit.
    for member in 0..8 {
        start(&mut key, member, 0);
    }
    let closed = last(authentication(), 1, 4);
    assert_eq!(
        key.failed(&0, Verdict::Close, authentication(), 50, JITTER),
        Outcome::Finish(closed.clone())
    );
    assert_eq!(key.wake_at(), None, "no wait");
    // The rest of the wave finish: with their own failure, or with the
    // recorded one when theirs was retriable.
    assert_eq!(
        key.failed(&1, Verdict::Close, authentication(), 60, JITTER),
        Outcome::Finish(closed.clone())
    );
    assert_eq!(
        key.failed(&2, Verdict::Retry, stall(), 60, JITTER),
        Outcome::Finish(closed.clone())
    );
    // The members beyond the wave start nothing: each finishes at once.
    for _ in 8..32 {
        assert_eq!(key.decide(70), Decision::Finish(closed.clone()));
    }
}

#[test]
fn an_interaction_timeout_is_returned_once_and_closes_the_key_and_an_allocation_one_moves_nothing()
{
    let interaction = failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction));
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    let verdict = classify(&interaction, Phase::Setup);
    assert_eq!(
        key.failed(&"a", verdict, interaction.clone(), 10, JITTER),
        Outcome::Finish(last(interaction.clone(), 1, 4))
    );
    assert_eq!(key.wake_at(), None);
    assert_eq!(key.decide(10), Decision::Finish(last(interaction, 1, 4)));

    let allocation = failure(ErrorCode::Timeout, Some(SetupFailureCause::Allocation));
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    let verdict = classify(&allocation, Phase::Other);
    assert_eq!(
        key.failed(&"a", verdict, allocation, 10, JITTER),
        Outcome::Return
    );
    // The key did not move: the next member starts at once, as the first did.
    assert_eq!(key.decide(10), Decision::Start);
    assert_eq!(key.wake_at(), None);
}

#[test]
fn a_probe_that_ends_without_a_verdict_frees_the_key_for_the_next_members_probe() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    key.failed(&"a", Verdict::Retry, stall(), 0, JITTER);
    let wake = key.wake_at().unwrap();
    // Past its wake the machine only answers the members that ask: one probe
    // at a time.
    start(&mut key, "b", wake);
    assert_eq!(key.decide(wake), Decision::Wait, "one probe at a time");
    // b is cancelled, or waits out its allocation: no verdict, no count.
    key.abandoned(&"b");
    start(&mut key, "c", wake);
    key.failed(&"c", Verdict::Retry, stall(), wake + 1, JITTER);
    assert_eq!(
        key.wake_at(),
        Some(wake + 1 + 2_000 + JITTER),
        "c's failure is attempt 2"
    );
}

/// `members` on one key against a dead server, as an endpoint would run them:
/// every setup fails with `failure` `setup_ms` after it starts, at most
/// `limit` setups run at once (the operation's per-host limit), and a member
/// that may not start waits in the endpoint's queue. Returns the setups'
/// start times, the most that ran at once after the key first failed, and
/// each member's final failure.
fn dead_key(
    members: usize,
    limit: usize,
    max_retries: u32,
    failure: Failure,
    setup_ms: u64,
) -> (Vec<u64>, usize, Vec<Final>) {
    let mut key = Machine::new(max_retries);
    let verdict = classify(&failure, Phase::Setup);
    let mut queue: VecDeque<usize> = (0..members).collect();
    let mut running: Vec<(u64, usize)> = Vec::new();
    let mut finals: Vec<Option<Final>> = vec![None; members];
    let mut starts = Vec::new();
    let mut after_failure = 0;
    let mut failed_once = false;
    let mut now = 0;
    loop {
        let mut held = VecDeque::new();
        while let Some(member) = queue.pop_front() {
            match key.decide(now) {
                Decision::Finish(last) => finals[member] = Some(last),
                Decision::Start if running.len() < limit => {
                    key.start(member);
                    starts.push(now);
                    running.push((now + setup_ms, member));
                }
                Decision::Start | Decision::Wait => held.push_back(member),
            }
        }
        queue = held;
        if failed_once {
            after_failure = after_failure.max(running.len());
        }
        let ends = running.iter().map(|(end, _)| *end).min();
        let wake = key.wake_at().filter(|at| *at > now && !queue.is_empty());
        now = match (ends, wake) {
            (Some(end), Some(wake)) => end.min(wake),
            (Some(at), None) | (None, Some(at)) => at,
            (None, None) => break,
        };
        let (due, rest): (Vec<_>, Vec<_>) = running.into_iter().partition(|(end, _)| *end <= now);
        running = rest;
        for (_, member) in due {
            failed_once = true;
            match key.failed(&member, verdict, failure.clone(), now, JITTER) {
                Outcome::Retry => queue.push_back(member),
                // The members queued when the key went Down share its failure.
                Outcome::Sweep(last) => {
                    finals[member] = Some(last.clone());
                    for queued in queue.drain(..) {
                        finals[queued] = Some(last.clone());
                    }
                }
                Outcome::Finish(last) => finals[member] = Some(last),
                Outcome::Return => panic!("a dead key's setup failure is a setup outcome"),
            }
        }
    }
    let finals = finals
        .into_iter()
        .map(|last| last.expect("every member finishes"))
        .collect();
    (starts, after_failure, finals)
}

#[test]
fn thirty_two_cold_members_on_a_dead_key_open_a_wave_at_the_limit_then_one_at_a_time() {
    for limit in [32, 8] {
        let (starts, after_failure, finals) = dead_key(32, limit, 3, stall(), 9_000);
        assert_eq!(starts.len(), limit + 3, "limit {limit}");
        assert!(
            starts[..limit].iter().all(|at| *at == 0),
            "the first wave starts at once: {starts:?}"
        );
        assert!(after_failure <= 1, "one handshake at a time after the wave");
        assert!(finals.iter().all(|f| *f == last(stall(), 4, 4)));
    }
}

/// Members one at a time, as `--jobs 1` runs them: the next member asks only
/// once the one before it has finished, and a parked member waits for the
/// key's next retest. With `healthy_first` the first member's setup succeeds
/// and the server is dead from the second on.
fn jobs_one(members: usize, failure: Failure, healthy_first: bool) -> (usize, Vec<Final>) {
    let mut key = Machine::new(3);
    let verdict = classify(&failure, Phase::Setup);
    let mut handshakes = 0;
    let mut finals = Vec::new();
    let mut now = 0;
    for member in 0..members {
        loop {
            match key.decide(now) {
                Decision::Finish(last) => {
                    finals.push(last);
                    break;
                }
                Decision::Wait => now = key.wake_at().expect("a waiting key has a wake"),
                Decision::Start => {
                    key.start(member);
                    handshakes += 1;
                    now += 9_000;
                    if healthy_first && member == 0 {
                        assert!(key.succeeded(&member, true));
                        break;
                    }
                    match key.failed(&member, verdict, failure.clone(), now, JITTER) {
                        Outcome::Retry => {}
                        Outcome::Finish(last) | Outcome::Sweep(last) => {
                            finals.push(last);
                            break;
                        }
                        Outcome::Return => panic!("a setup failure is a setup outcome"),
                    }
                }
            }
        }
    }
    (handshakes, finals)
}

#[test]
fn with_jobs_1_a_dead_key_costs_four_handshakes_and_two_parked_retests_then_members_finish_at_once()
{
    // §5.5, case 32: the first member spends the budget (attempt 4 of 4), the
    // next two each carry a retest 30 s on, and after two failed retests in a
    // row every other member finishes with no handshake and no count.
    let (handshakes, finals) = jobs_one(32, stall(), false);
    assert_eq!(handshakes, 4 + 2);
    assert_eq!(finals.len(), 32);
    assert_eq!(finals[0], last(stall(), 4, 4));
    assert!(finals[1..].iter().all(|f| *f == last(stall(), 0, 4)));
    // Healthy earlier in the operation, then exhausted: Down, not Cold.
    let (handshakes, finals) = jobs_one(32, stall(), true);
    assert_eq!(handshakes, 1 + 4 + 2);
    assert_eq!(finals.len(), 31);
    assert_eq!(finals[0], last(stall(), 4, 4));
    assert!(finals[1..].iter().all(|f| *f == last(stall(), 0, 4)));
}

#[test]
fn an_authentication_failure_under_jobs_1_is_one_handshake_and_every_member_shares_it() {
    let (handshakes, finals) = jobs_one(32, authentication(), false);
    assert_eq!(handshakes, 1);
    assert_eq!(finals.len(), 32);
    assert!(finals.iter().all(|f| *f == last(authentication(), 1, 4)));
}

#[test]
fn a_success_from_a_generation_the_key_left_completes_its_member_but_not_the_key() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    start(&mut key, "b", 0);
    assert_eq!(
        key.failed(&"a", Verdict::Retry, stall(), 10, JITTER),
        Outcome::Retry
    );
    assert!(
        !key.succeeded(&"b", true),
        "b's session serves b, and is not admitted for reuse"
    );
    assert_eq!(key.decide(11), Decision::Wait, "the key is still waiting");
    // The count did not reset: the next probe is attempt 2.
    let wake = key.wake_at().unwrap();
    start(&mut key, "a", wake);
    key.failed(&"a", Verdict::Retry, stall(), wake + 5, JITTER);
    assert_eq!(key.wake_at(), Some(wake + 5 + 2_000 + JITTER));
}

#[test]
fn a_wave_success_before_any_failure_heals_the_key_and_the_rest_of_the_wave_is_its_own() {
    let mut key = Machine::new(3);
    for member in ["a", "b", "c"] {
        start(&mut key, member, 0);
    }
    assert!(key.succeeded(&"a", true));
    assert_eq!(key.decide(1), Decision::Start, "Healthy: normal allocation");
    // b is Healthy's own now: its retriable failure is counted, as attempt 1.
    assert_eq!(
        key.failed(&"b", Verdict::Retry, stall(), 2, JITTER),
        Outcome::Retry
    );
    assert_eq!(key.wake_at(), Some(2 + 1_000 + JITTER));
    // c belongs to the generation the key has just left: it counts nothing.
    assert_eq!(
        key.failed(&"c", Verdict::Retry, stall(), 3, JITTER),
        Outcome::Retry
    );
    assert_eq!(key.wake_at(), Some(2 + 1_000 + JITTER));
}

#[test]
fn a_non_retriable_failure_of_a_setup_the_key_left_closes_the_key() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    start(&mut key, "b", 0);
    key.failed(&"a", Verdict::Retry, stall(), 10, JITTER);
    let closed = last(authentication(), 1, 4);
    assert_eq!(
        key.failed(&"b", Verdict::Close, authentication(), 20, JITTER),
        Outcome::Finish(closed.clone())
    );
    assert_eq!(
        key.decide(5_000),
        Decision::Finish(closed),
        "a's wake opens nothing: the key is closed"
    );
}

#[test]
fn leasing_an_idle_connection_is_not_a_setup() {
    let mut key = Machine::new(3);
    start(&mut key, "a", 0);
    assert!(
        key.succeeded(&"a", false),
        "an idle lease was admitted before"
    );
    assert_eq!(key.decide(1), Decision::Start);
    key.start("b");
    // b sets up and fails: the key was still Cold, so this is attempt 1.
    key.failed(&"b", Verdict::Retry, stall(), 2, JITTER);
    let wake = key.wake_at().unwrap();
    assert_eq!(wake, 2 + 1_000 + JITTER);
    // The probe leases an idle connection: no verdict, and the next member
    // probes. The count is unchanged, so its failure is attempt 2.
    start(&mut key, "c", wake);
    assert!(key.succeeded(&"c", false));
    start(&mut key, "d", wake);
    key.failed(&"d", Verdict::Retry, stall(), wake + 3, JITTER);
    assert_eq!(key.wake_at(), Some(wake + 3 + 2_000 + JITTER));
}
