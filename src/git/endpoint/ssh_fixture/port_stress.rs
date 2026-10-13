//! Many fixtures starting at once, as the tests of a parallel run do: how many meet a server that is not theirs.
//!
//! Run by name, with `--ignored` (and `--nocapture` to see each retry the fixtures made):
//! `cargo test --lib ssh_fixture::port_stress -- --ignored --nocapture`. The race is rare in an ordinary run: the
//! fixture picks a port by binding and releasing one, and another fixture's pick must be the same port in the
//! moments before the server binds it. A host that hands a released port out again soon meets it now and then (the
//! Linux CI job, 2026-10-13); macOS did not in 2,880 starts of the first row. So the other rows pick from a pool of
//! 512 ports, which makes the race common: the old fixture, which took any server that accepted on its port for its
//! own, fails the second row (`temporary host key did not match`; the run's README has the counts), and this one
//! does not fail it, or the third, which starts debugging servers.
use super::SshdFixture;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::thread;

/// Fixtures started together, and how many each thread starts one after another.
const THREADS: usize = 48;
const EACH: usize = 20;
/// The size of the small pool of ports.
const POOL: u16 = 512;

/// Starts `THREADS` threads that each start `EACH` fixtures with `start` and open one session on each with `open`;
/// returns the failures.
fn run(
    start: impl Fn() -> SshdFixture + Sync,
    open: impl Fn(&mut SshdFixture) -> Result<(), String> + Sync,
) -> Vec<String> {
    let failures = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                for _ in 0..EACH {
                    let outcome = catch_unwind(AssertUnwindSafe(|| {
                        let mut fixture = start();
                        open(&mut fixture)
                    }));
                    let failure = match outcome {
                        Ok(Ok(())) => continue,
                        Ok(Err(error)) => error,
                        Err(_) => "the fixture failed to start".to_owned(),
                    };
                    failures.lock().unwrap().push(failure);
                }
            });
        }
    });
    failures.into_inner().unwrap()
}

fn try_session(fixture: &mut SshdFixture) -> Result<(), String> {
    fixture.try_session().map(drop).map_err(|e| e.to_string())
}

fn none_failed(failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} of {} fixtures did not reach their own server: {failures:?}",
        failures.len(),
        THREADS * EACH
    );
}

#[test]
#[ignore = "a stress run for the port race, by name"]
fn many_fixtures_starting_together_each_reach_their_own_server() {
    none_failed(run(SshdFixture::new, try_session));
}

/// A port that nothing listens on, chosen at random from `POOL` ports from `first`: bound and released, as the
/// fixture's own pick is.
fn from_the_pool(first: u16) -> u16 {
    loop {
        let draw = RandomState::new().build_hasher().finish();
        let port = first + (draw % u64::from(POOL)) as u16;
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

#[test]
#[ignore = "a stress run for the port race, by name"]
fn many_fixtures_starting_together_from_a_small_pool_of_ports_each_reach_their_own_server() {
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let first = probe.local_addr().unwrap().port() & !(POOL - 1);
    drop(probe);
    none_failed(run(
        || SshdFixture::new_on_ports(|| from_the_pool(first)),
        try_session,
    ));
}

/// The debugging server serves one connection, so it is the session that tells it from another test's server.
#[test]
#[ignore = "a stress run for the port race, by name"]
fn many_debug_fixtures_starting_together_from_a_small_pool_of_ports_each_reach_their_own_server() {
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let first = probe.local_addr().unwrap().port() & !(POOL - 1);
    drop(probe);
    none_failed(run(
        || SshdFixture::new_debug_on_ports(|| from_the_pool(first)),
        |fixture| {
            drop(fixture.debug_session());
            Ok(())
        },
    ));
}
