//! A session that ends an exchange is idle, over and over (`--ignored`).
//!
//! The `idle_loss` rows make two exchanges each, so a fault that ends a link once in several thousand exchanges
//! shows in one run of the family in a few hundred. These rows make the exchanges in their thousands, on one session
//! per thread: while the proxy read its sockets with a read timeout, four or five of eight threads lost their session
//! in a run of 2,000 exchanges each on the Windows host, under load (a read that raced its own timeout answered
//! `ERROR_IO_PENDING`, which ended the link), and the pool counted nothing idle.
//!
//! An exchange that fails is the server's: `sshd.exe` resets a connection now and then (one exchange in some ten
//! thousand, under load), and the next exchange dials afresh. A few failures are allowed; a session lost *after* an
//! exchange that worked is not.
use super::Case;
use crate::git::endpoint::ssh_fixture::DialRetries;
use std::thread;

const THREADS: usize = 8;
const EXCHANGES: usize = 1_500;

/// The failed exchanges one run may have, of the `THREADS * EXCHANGES` it makes.
const FAILED_EXCHANGES_ALLOWED: usize = 12;

#[test]
#[ignore = "thousands of exchanges: run with --ignored"]
fn a_session_is_idle_after_each_of_many_exchanges() {
    let failed: usize = thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|thread_number| {
                scope.spawn(move || {
                    let case = Case::new(false, DialRetries::default());
                    let mut failed = 0;
                    for exchange in 0..EXCHANGES {
                        if case.exchange().is_err() {
                            failed += 1;
                            continue;
                        }
                        let what = format!(
                            "thread {thread_number}, exchange {exchange}: the session is idle"
                        );
                        case.wait_for(&what, |case| case.endpoint.pool().counts().idle == 1);
                    }
                    failed
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .sum()
    });
    assert!(
        failed <= FAILED_EXCHANGES_ALLOWED,
        "{failed} of {} exchanges failed",
        THREADS * EXCHANGES
    );
}
