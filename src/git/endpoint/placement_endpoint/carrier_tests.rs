//! The test of a site's limit is given back on every exit of its carrier
//! (adaptive concurrency design §5.2: no wait is unbounded). A wave of four
//! members is refused, which opens a confirmation, and a fifth member queued
//! ahead of the wave's requeued members is the first to be offered the
//! confirming test. Its open breaks at one of the carrier's exits, and the
//! test must go back to the next member, which then starts: a gate left shut
//! would hang the site for the rest of the operation.
use super::{
    retry_tests::{OPERATION, endpoint, open, step},
    *,
};
use gwz_transport::protocol::{IdentityMode, SetupFailureCause};
use std::sync::{Arc, Mutex};

fn refused() -> Failure {
    Failure {
        detail: None,
        setup_cause: Some(SetupFailureCause::ConnectionRefused),
        code: ErrorCode::Unavailable,
        effect: Effect::None,
        facts: None,
    }
}

/// Runs the wave with a fifth member (stream 5) whose open `spoil` breaks,
/// until a fifth setup starts: the confirming test, carried by a member of the
/// wave once the spoilt carrier's exit has given the test back. Returns the
/// terminals handed out meanwhile and the number of setups started.
fn carrier_exit(spoil: impl FnOnce(&mut Envelope)) -> (Vec<Envelope>, usize) {
    let (mut endpoint, starts): (PlacementEndpoint, Arc<Mutex<usize>>) =
        endpoint(refused(), Some(Duration::from_millis(30)), 4, 3);
    let mut terminals = Vec::new();
    for id in 1..=4 {
        endpoint.accept(OPERATION.into(), open(id, 30_000)).unwrap();
    }
    let mut late = open(5, 30_000);
    spoil(&mut late);
    endpoint.accept(OPERATION.into(), late).unwrap();
    let begun = Instant::now();
    while *starts.lock().unwrap() < 5 {
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "the gate stayed shut after the carrier's exit: {} setups started",
            starts.lock().unwrap()
        );
        step(&mut endpoint, 0, &mut terminals);
    }
    let started = *starts.lock().unwrap();
    endpoint.shutdown();
    (terminals, started)
}

fn failed_with(terminals: &[Envelope], stream: i64) -> Option<ErrorCode> {
    terminals
        .iter()
        .find(|t| t.stream_id == stream && t.kind == MessageKind::OpenFailed)
        .and_then(|t| t.open_failed.as_ref().map(|failure| failure.code))
}

#[test]
fn a_carrier_whose_identity_cannot_be_selected_gives_the_test_back() {
    let (terminals, started) = carrier_exit(|envelope| {
        let identity = &mut envelope.open.as_mut().unwrap().identity;
        identity.mode = IdentityMode::ExplicitKey;
        identity.key_path = Some("~someone-else/key".into());
    });
    assert_eq!(started, 5);
    assert_eq!(
        failed_with(&terminals, 5),
        Some(ErrorCode::InvalidRequest),
        "the carrier failed at selected_path"
    );
}

#[test]
fn a_carrier_the_worker_refuses_gives_the_test_back() {
    let (terminals, started) = carrier_exit(|envelope| {
        envelope.open.as_mut().unwrap().destination.path = "repo\0".into();
    });
    assert_eq!(started, 5);
    assert_eq!(
        failed_with(&terminals, 5),
        Some(ErrorCode::InvalidRequest),
        "the carrier failed at start_endpoint_open"
    );
}
