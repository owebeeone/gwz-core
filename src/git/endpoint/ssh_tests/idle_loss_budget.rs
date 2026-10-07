//! Idle loss and the host's job budget. A reused session found dead at its
//! channel open is replaced by one fresh connection, whose setup takes its
//! place in the same `Supervisor` the first setup used, once: the dead
//! session's place was returned when its setup finished, and the retry holds
//! none while it waits.
use super::attachment;
use crate::git::endpoint::{
    agent_job::Supervisor,
    cut_proxy::CutProxy,
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
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn config() -> Config {
    Config {
        total: 2,
        per_host: 2,
        per_user_host: 2,
        ..Config::default()
    }
}

#[test]
fn a_fresh_retry_after_idle_loss_takes_one_place_from_the_hosts_supervisor() {
    let fixture = SshdFixture::new();
    let proxy = Arc::new(CutProxy::start(fixture.port));
    let dial = Arc::new(fixture.dialer(proxy.port));
    let supervisor = Supervisor::new();
    // The places the supervisor had taken as each setup ran inside its job.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let endpoint = {
        let (supervisor, seen) = (supervisor.clone(), seen.clone());
        Endpoint::with_registry(
            config(),
            Registry::with_supervisor(supervisor.clone()),
            move |origin, registry| {
                SetupConnector::reported(
                    origin,
                    Duration::from_millis(500),
                    registry.supervisor(),
                    move |_, _, _| {
                        let (dial, supervisor, seen) =
                            (dial.clone(), supervisor.clone(), seen.clone());
                        let setup: Setup = Box::new(move |_| {
                            seen.lock().unwrap().push(supervisor.taken());
                            let connection = dial()?;
                            Authenticated::new(
                                connection,
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
                    },
                )
            },
            5_000,
        )
        .unwrap()
    };
    let exchange = || {
        let (mut stream, opened) = attachment::open(
            &endpoint,
            Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
            None,
            GitService::UploadPack,
            fixture.repository.to_str().unwrap(),
            attachment::deadlines(&config(), 5_000),
        )
        .unwrap();
        let mut advertisement = Vec::new();
        stream.write_all(b"0000").unwrap();
        stream.end_write().unwrap();
        stream.read_to_end(&mut advertisement).unwrap();
        stream.close().unwrap();
        opened
    };
    assert!(!exchange().reused);
    let deadline = Instant::now() + Duration::from_secs(5);
    while endpoint.pool().counts().idle != 1 {
        assert!(Instant::now() < deadline, "the session is idle");
        thread::sleep(Duration::from_millis(5));
    }
    proxy.arm_existing();
    assert!(!exchange().reused, "the dead session was replaced");
    assert_eq!(proxy.connections(), 2);
    assert_eq!(
        *seen.lock().unwrap(),
        [1, 1],
        "each setup held one place, the retry's not added to the dead session's"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while supervisor.taken() != 0 {
        assert!(Instant::now() < deadline, "every place was returned");
        thread::sleep(Duration::from_millis(5));
    }
    endpoint.shutdown();
}
