//! The SSH endpoint consults its pool's limit machine where it consulted the
//! per-host limit (adaptive concurrency design §4.5, §5.2 and §4.9): an open
//! waits behind a hold with its allocation clock stopped, and starts when the
//! hold ends. The machine is told what the server said, as the classification
//! of a failure will tell it (a later step); the retry machine's and the
//! pool's own limits are unchanged.
use super::{
    retry_tests::{OPERATION, endpoint, open, stall, step},
    *,
};
use crate::git::endpoint::setup_retry::{Conn, Signal};

fn site_key() -> Key {
    Key::ssh("git", "host", 22)
}
/// Steps at `now` until `done`, for at most 10 s of real time.
fn until(
    endpoint: &mut PlacementEndpoint,
    now: u64,
    terminals: &mut Vec<Envelope>,
    done: impl Fn(&PlacementEndpoint) -> bool,
) {
    let begun = Instant::now();
    while !done(endpoint) {
        assert!(begun.elapsed() < Duration::from_secs(10), "never happened");
        step(endpoint, now, terminals);
    }
}

#[test]
fn an_open_behind_a_hold_waits_with_its_allocation_stopped_and_starts_when_it_ends() {
    // One setup that never ends is the connection the machine has begun.
    let (mut endpoint, starts) = endpoint(stall(), None, 4, 3);
    let mut terminals = Vec::new();
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |endpoint| {
        endpoint
            .endpoint
            .governor()
            .scoped(OPERATION)
            .view(&site_key(), endpoint.endpoint.pool_now())
            .is_some_and(|view| view.possible == 1)
    });
    assert_eq!(*starts.lock().unwrap(), 1);
    // The server's word: a Retry-After of 1.1 s on the first connection.
    let pool_now = endpoint.endpoint.pool_now();
    endpoint.endpoint.governor().scoped(OPERATION).refused(
        &site_key(),
        Conn(1),
        Signal::Throttle,
        Some(1_100),
        false,
        pool_now,
    );
    // A second open, with an allocation of 500 ms, is held while the endpoint's
    // own clock runs 5 s past it: a wait behind a hold is no allocation.
    endpoint.accept(OPERATION.into(), open(2, 500)).unwrap();
    for now in [0, 400, 5_000] {
        step(&mut endpoint, now, &mut terminals);
    }
    assert_eq!(*starts.lock().unwrap(), 1, "no start during the hold");
    assert!(terminals.is_empty(), "the held open has not timed out");
    // The hold ends in the pool's time; the open then starts, with what was
    // left of its allocation, and the idle connections (none here) are gone.
    until(&mut endpoint, 5_000, &mut terminals, |_| {
        *starts.lock().unwrap() == 2
    });
    assert!(terminals.is_empty());
    endpoint.shutdown();
}

#[test]
fn an_open_is_held_by_the_believed_limit_of_its_site_not_only_by_the_ceiling() {
    // With a ceiling of four and a believed limit of one, the second open of
    // the site waits for the first, as it would for a limit of one.
    let (mut endpoint, starts) = endpoint(stall(), None, 4, 3);
    let mut terminals = Vec::new();
    endpoint
        .endpoint
        .governor()
        .begin_operation(OPERATION, 4, true, 0);
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |endpoint| {
        endpoint
            .endpoint
            .governor()
            .scoped(OPERATION)
            .view(&site_key(), endpoint.endpoint.pool_now())
            .is_some_and(|view| view.possible == 1)
    });
    // The first connection's attempt is refused with nothing else counted:
    // hi = 0 is the retry machine's, so make a second connection first.
    endpoint.accept(OPERATION.into(), open(2, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |_| {
        *starts.lock().unwrap() == 2
    });
    let pool_now = endpoint.endpoint.pool_now();
    endpoint.endpoint.governor().scoped(OPERATION).refused(
        &site_key(),
        Conn(2),
        Signal::Throttle,
        None,
        false,
        pool_now,
    );
    let view = endpoint
        .endpoint
        .governor()
        .scoped(OPERATION)
        .view(&site_key(), pool_now)
        .unwrap();
    // The first connection is still setting up, so nothing is Connected:
    // N = max(1, min(0, 1)) = 1, the floor.
    assert_eq!(view.n, 1);
    endpoint.accept(OPERATION.into(), open(3, 30_000)).unwrap();
    for now in [1, 2, 3] {
        step(&mut endpoint, now, &mut terminals);
    }
    assert_eq!(*starts.lock().unwrap(), 2, "the third open waits for room");
    // It waits in the endpoint, not in the pool: it was never submitted.
    assert_eq!((endpoint.opens.len(), endpoint.queued_opens.len()), (2, 1));
    endpoint.shutdown();
}

#[test]
fn in_saturated_the_pool_site_limit_is_the_pools_own_cap_not_the_open_ceiling() {
    // `--max-per-host 64`: opens in flight stop at the endpoint's own ceiling
    // of 32, but the connections they leave behind (streaming on their
    // leases) are bounded by the pool's cap, 64. A site limit of 32 would
    // make the 33rd connection wait, which 1.0.17 does not.
    let (mut endpoint, _) = endpoint(stall(), None, 64, 3);
    let mut terminals = Vec::new();
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |endpoint| {
        endpoint
            .endpoint
            .governor()
            .scoped(OPERATION)
            .view(&site_key(), endpoint.endpoint.pool_now())
            .is_some_and(|view| view.possible == 1)
    });
    let site = site_key().site();
    assert!(
        endpoint.pool().limit(&site).is_none_or(|limit| limit == 64),
        "the pool's site limit is {:?}",
        endpoint.pool().limit(&site)
    );
    let view = endpoint
        .endpoint
        .governor()
        .scoped(OPERATION)
        .view(&site_key(), endpoint.endpoint.pool_now())
        .unwrap();
    assert_eq!((view.ceiling, view.pool_limit), (64, 64));
    endpoint.shutdown();
}

#[test]
fn a_second_request_does_not_erase_the_hold_a_live_request_set() {
    let (mut endpoint, starts) = endpoint(stall(), None, 4, 3);
    let mut terminals = Vec::new();
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |endpoint| {
        endpoint
            .endpoint
            .governor()
            .scoped(OPERATION)
            .view(&site_key(), endpoint.endpoint.pool_now())
            .is_some_and(|view| view.possible == 1)
    });
    let pool_now = endpoint.endpoint.pool_now();
    endpoint.endpoint.governor().scoped(OPERATION).refused(
        &site_key(),
        Conn(1),
        Signal::Throttle,
        Some(1_100),
        false,
        pool_now,
    );
    // Request "other" is admitted while OPERATION still runs.
    endpoint.set_max_retries("other", 3);
    endpoint.accept("other".into(), open(2, 30_000)).unwrap();
    for now in [0, 1, 2] {
        step(&mut endpoint, now, &mut terminals);
    }
    assert_eq!(*starts.lock().unwrap(), 1, "no start inside the hold");
    until(&mut endpoint, 3, &mut terminals, |_| {
        *starts.lock().unwrap() == 2
    });
    endpoint.shutdown();
}

fn refused() -> Failure {
    Failure {
        detail: None,
        setup_cause: Some(gwz_transport::protocol::SetupFailureCause::ConnectionRefused),
        code: ErrorCode::Unavailable,
        effect: Effect::None,
        facts: None,
    }
}
/// Steps at `now` until the endpoint has no attempt in flight.
fn quiesce(endpoint: &mut PlacementEndpoint, now: u64, terminals: &mut Vec<Envelope>) {
    until(endpoint, now, terminals, |endpoint| {
        endpoint.opens.is_empty()
    });
}

#[test]
fn a_refused_wave_is_requeued_not_counted_and_its_confirming_test_is_the_first_counted_failure() {
    // Four members start at once against a host that refuses every connect:
    // each refusal has the others in its window (hi = 3, below the ceiling 4),
    // so it is limit evidence. It is requeued for the machine to judge, and
    // the key is not made to wait (§5.1): a fifth start follows at the same
    // instant, the confirmation's test (§4.5 rule 3). The wave resolved with
    // nothing Connected, so that test runs at k = 1: its refusal is not a
    // limit (§4.8) and goes to the retry machine, whose first counted failure
    // it is.
    let (mut endpoint, starts) = endpoint(refused(), Some(Duration::from_millis(30)), 4, 3);
    let mut terminals = Vec::new();
    for id in 1..=4 {
        endpoint.accept(OPERATION.into(), open(id, 30_000)).unwrap();
    }
    until(&mut endpoint, 0, &mut terminals, |_| {
        *starts.lock().unwrap() == 5
    });
    quiesce(&mut endpoint, 0, &mut terminals);
    assert!(terminals.is_empty());
    let wake = endpoint
        .retries
        .machine(OPERATION, &super::retry_tests::key())
        .wake_at();
    assert!(
        wake.is_some(),
        "the confirming handshake was the key's first counted failure"
    );
    assert!(
        !endpoint
            .endpoint
            .governor()
            .scoped(OPERATION)
            .view(&site_key(), endpoint.endpoint.pool_now())
            .unwrap()
            .confirmation,
        "and the confirmation is closed"
    );
    endpoint.shutdown();
}

#[test]
fn at_max_retries_zero_a_refused_member_finishes_at_once_and_nothing_adapts() {
    let (mut endpoint, _) = endpoint(refused(), Some(Duration::from_millis(30)), 4, 0);
    let mut terminals = Vec::new();
    for id in 1..=4 {
        endpoint.accept(OPERATION.into(), open(id, 30_000)).unwrap();
    }
    let begun = Instant::now();
    while terminals.len() < 4 {
        assert!(begun.elapsed() < Duration::from_secs(10), "never finished");
        step(&mut endpoint, 0, &mut terminals);
    }
    for terminal in &terminals {
        let failure = terminal.open_failed.as_ref().unwrap();
        assert_eq!(failure.code, ErrorCode::Unavailable);
    }
    let view = endpoint
        .endpoint
        .governor()
        .scoped(OPERATION)
        .view(&site_key(), endpoint.endpoint.pool_now())
        .unwrap();
    assert!(!view.confirmation);
    endpoint.shutdown();
}
