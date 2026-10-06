//! The placement thread runs a pass when something gives it work, and parks
//! for `PARK` only when nothing does: a change that waited out the park would
//! add up to 5 ms to each step of a command's shutdown, which TR8.1's gap to
//! 1.0.17 counted (about 8 ms of a fetch's 150 ms). Each test parks the thread
//! for a minute, so that a pass within the wait can only have been a wake.
use super::*;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(3);

/// A driver session whose placement thread has settled into a park of a
/// minute, and the passes it had finished by then.
fn parked() -> (Arc<Session>, TransportPort, usize) {
    let (session, port) = Session::driver(3000, 3000, None).expect("driver session");
    session
        .register("request", Some("operation".into()))
        .expect("registration");
    session.park_for_test(Duration::from_secs(60));
    // The park the thread was in when the minute was set ends within PARK;
    // once it has read the minute, it passes no more.
    let mut passes = session.passes_for_test();
    loop {
        thread::sleep(Duration::from_millis(30));
        let now = session.passes_for_test();
        if now == passes {
            return (session, port, passes);
        }
        passes = now;
    }
}

/// Waits for a pass beyond `passes`: whether the thread ran one.
fn passed_beyond(session: &Session, passes: usize) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if session.passes_for_test() > passes {
            return true;
        }
        thread::sleep(Duration::from_millis(1));
    }
    false
}

#[test]
fn sealing_a_request_wakes_the_placement_thread() {
    let (session, _port, passes) = parked();
    session.seal("request");
    assert!(
        passed_beyond(&session, passes),
        "the seal left the placement thread parked"
    );
}

#[test]
fn closing_the_session_wakes_the_placement_thread() {
    let (session, _port, passes) = parked();
    session.close();
    assert!(
        passed_beyond(&session, passes),
        "the close left the placement thread parked"
    );
}
