//! A worker with nothing to do parks for up to a second between turns. A
//! release leaves the pool a Close for the session just discarded, and the
//! worker must take the turn that disposes of it at once: the host's step
//! wakes it when a turn made progress (`PoolHost::step`), and a turn that took
//! the Close is one. The test waits at most `PROMPTLY`, far short of the park.
use super::attachment;
use crate::git::endpoint::{
    ssh_channel::GitService,
    ssh_fixture::SshdFixture,
    ssh_key_snapshot::Registry,
    ssh_setup::{Authenticated, Setup, SetupConnector},
    ssh_worker::Endpoint,
};
use gwz_transport::{
    pool::{Config, Identity, Key},
    protocol::{AuthMethod, Facts},
};
use std::{
    io::{Read, Write},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const PROMPTLY: Duration = Duration::from_millis(500);

fn config() -> Config {
    Config {
        total: 2,
        per_host: 2,
        per_user_host: 2,
        ..Config::default()
    }
}

fn endpoint(fixture: &SshdFixture) -> Endpoint {
    let dial = Arc::new(fixture.dialer(fixture.port));
    Endpoint::with_registry(
        config(),
        Registry::new(),
        move |origin, _| {
            SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                let dial = dial.clone();
                let setup: Setup = Box::new(move |_| {
                    Authenticated::new(
                        dial()?,
                        Identity::Ambient,
                        Facts {
                            method: AuthMethod::SshKey,
                            authenticated: Some(true),
                            credential_offered: true,
                            ..Facts::default()
                        },
                    )
                });
                Ok(setup)
            })
        },
        5_000,
    )
    .unwrap()
}

/// One upload-pack exchange, to its end, that the worker discards when it is
/// done (a setup the key's retry machine did not admit).
fn discarded_exchange(endpoint: &Endpoint, fixture: &SshdFixture) {
    let deadlines = attachment::deadlines(&config(), 5_000);
    let context = attachment::context(deadlines.clone());
    let pending = attachment::start(
        endpoint,
        Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
        None,
        GitService::UploadPack,
        fixture.repository.to_str().unwrap(),
        deadlines,
    )
    .unwrap();
    let (attached, _) = attachment::finish(&pending).unwrap();
    attached.discard_after_use();
    let mut stream = attachment::drive(attached, &context);
    let mut advertisement = Vec::new();
    stream.write_all(b"0000").unwrap();
    stream.end_write().unwrap();
    stream.read_to_end(&mut advertisement).unwrap();
    stream.close().unwrap();
}

#[test]
fn a_discarded_release_on_an_otherwise_idle_worker_is_disposed_of_within_milliseconds() {
    let fixture = SshdFixture::new();
    let endpoint = endpoint(&fixture);
    discarded_exchange(&endpoint, &fixture);
    let began = Instant::now();
    while endpoint.pool().counts().total() != 0 {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the discarded session was never disposed of"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        began.elapsed() < PROMPTLY,
        "the discarded session was disposed of after {:?}; the idle worker had parked",
        began.elapsed()
    );
    endpoint.shutdown();
}
