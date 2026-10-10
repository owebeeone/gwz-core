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
            .view(&site_key(), endpoint.endpoint.pool_now())
            .is_some_and(|view| view.possible == 1)
    });
    assert_eq!(*starts.lock().unwrap(), 1);
    // The server's word: a Retry-After of 1.1 s on the first connection.
    let pool_now = endpoint.endpoint.pool_now();
    endpoint.endpoint.governor().refused(
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
    endpoint.endpoint.governor().begin_operation(4, true);
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    until(&mut endpoint, 0, &mut terminals, |endpoint| {
        endpoint
            .endpoint
            .governor()
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
    endpoint.endpoint.governor().refused(
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
