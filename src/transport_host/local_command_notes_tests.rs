//! What a command's runtime does with the reader it is given (§9): the
//! endpoints it built give the reader what their limit machines say, with no
//! request in flight and no global.
use super::*;
use crate::git::endpoint::setup_retry::{Conn, Signal};
use crate::transport_host::{SshEndpointConfig, https_tests::endpoint_home};
use gwz_transport::pool;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// A runtime over fixture endpoints, built as a command builds its own.
fn runtime(notes: Option<&NoteSink>) -> TransportRuntime {
    let home = tempfile::tempdir().unwrap();
    build_runtime(
        SshEndpointConfig::fixture(endpoint_home(home.path()), None).into(),
        HttpsEndpointConfig {
            tls: Default::default(),
            auth: None,
        },
        None,
        notes,
    )
    .unwrap()
}

#[test]
fn a_runtime_built_with_a_reader_gives_it_the_endpoints_notes_as_lines() {
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let kept = lines.clone();
    let sink = NoteSink::new(move |line| kept.lock().unwrap().push(line.to_owned()));
    let runtime = runtime(Some(&sink));
    let client = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .https_client_for_test();
    let governor = client.governor();
    governor.begin_operation("op", 8, true, client.pool_now());
    governor.scoped("op").refused(
        &pool::Key::https("example.com", 443),
        Conn(1),
        Signal::Throttle,
        Some(20_000),
        false,
        client.pool_now(),
    );
    let begun = Instant::now();
    while lines.lock().unwrap().is_empty() && begun.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        *lines.lock().unwrap(),
        ["note: example.com:443 asked gwz to wait 20 s; waiting"]
    );
}

#[test]
fn a_runtime_built_without_a_reader_keeps_its_notes_to_itself() {
    let runtime = runtime(None);
    let client = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .https_client_for_test();
    let governor = client.governor();
    governor.begin_operation("op", 8, true, client.pool_now());
    governor.scoped("op").refused(
        &pool::Key::https("example.com", 443),
        Conn(1),
        Signal::Throttle,
        Some(20_000),
        false,
        client.pool_now(),
    );
    std::thread::sleep(Duration::from_millis(300));
    // Nothing took them: they are still kept for a reader that asks.
    assert_eq!(governor.drain_all_notes().len(), 1);
}
