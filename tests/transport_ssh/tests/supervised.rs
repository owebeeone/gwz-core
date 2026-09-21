#![allow(dead_code, unused_imports)]
mod common;
use common::{SshConnection, ssh_channel, ssh_connection};
#[path = "../../../src/git/endpoint/agent_auth.rs"]
mod agent_auth;
#[path = "../../../src/git/endpoint/agent_client.rs"]
mod agent_client;
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
#[path = "../../../src/git/endpoint/agent_socket.rs"]
mod agent_socket;
#[path = "../../../src/git/endpoint/ssh_destination.rs"]
mod ssh_destination;
#[path = "../../../src/git/endpoint/ssh_endpoint.rs"]
mod ssh_endpoint;
#[path = "../../../src/git/endpoint/ssh_pool.rs"]
mod ssh_pool;
#[path = "../../../src/git/endpoint/ssh_pump.rs"]
mod ssh_pump;
#[path = "../../../src/git/endpoint/ssh_remote.rs"]
mod ssh_remote;
#[path = "../../../src/git/endpoint/ssh_key_auth.rs"]
mod ssh_key_auth;
#[path = "../../../src/git/endpoint/ssh_key_container.rs"]
mod ssh_key_container;
#[path = "../../../src/git/endpoint/ssh_key_snapshot.rs"]
mod ssh_key_snapshot;
#[path = "../../../src/git/endpoint/ssh_admission.rs"]
mod ssh_admission;
#[path = "../../../src/git/endpoint/ssh_setup.rs"]
mod ssh_setup;
#[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
mod ssh_shutdown;
#[path = "../../../src/git/endpoint/ssh_worker.rs"]
mod ssh_worker;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;
use gwz_transport::{
    pool::{Config, Identity, Key},
    protocol::{AuthMethod, Facts, Opened},
};
use ssh_setup::{Authenticated, Setup, SetupConnector};
use ssh_worker::Endpoint;
use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn done(endpoint: &Endpoint) -> ssh_worker::ShutdownStatus {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = endpoint.shutdown_status();
        if status.cleanup_complete {
            return status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn config() -> Config {
    Config {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        cleanup_timeout_ms: 50,
        ..Config::default()
    }
}
#[test]
fn healthy_empty_worker_shutdown_reports_complete() {
    let endpoint = Endpoint::with_connector(
        config(),
        |origin| {
            SetupConnector::new(
                origin,
                Duration::from_millis(50),
                |_: &Key, _: &Identity| -> io::Result<Setup> { Err(io::ErrorKind::Other.into()) },
            )
        },
        100,
    )
    .unwrap();
    endpoint.shutdown();
    assert_eq!(done(&endpoint).failure, None);
}
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[path = "../support/agent_auth.rs"] mod support;
        #[test]
        fn native_setup_reuses_connection_with_operation_scoped_facts() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let user = fixture.ssh.user.clone(); let path = fixture.path.clone();
            let setup: Setup = Box::new(move |control| {
                let connection = agent_auth::authenticate(connection, &user, &host, control.clone(), || agent_socket::connect(&path, control))?;
                Authenticated::new(connection, Identity::Ambient, Facts { method: AuthMethod::SshAgent, authenticated: Some(true), credential_offered: true, ..Facts::default() })
            });
            let mut setup = Some(setup);
            let endpoint = Endpoint::with_connector(config(), move |origin| SetupConnector::new(origin, Duration::from_millis(50), move |_: &Key, _: &Identity| setup.take().ok_or_else(|| io::ErrorKind::Other.into())), 1000).unwrap();
            let key = Key::ssh(&fixture.ssh.user, "127.0.0.1", fixture.ssh.port);
            for reuse in [false, true] {
                let (mut stream, opened) = endpoint.open_observed(key.clone(), Identity::Ambient, ssh_channel::GitService::UploadPack, fixture.ssh.repository.to_str().unwrap()).unwrap();
                assert_eq!(opened.reused, reuse); assert_eq!(opened.facts.credential_offered, !reuse); assert_eq!(opened.facts.authenticated, Some(true));
                stream.write_all(b"0000").unwrap(); stream.end_write().unwrap(); let mut bytes = Vec::new(); stream.read_to_end(&mut bytes).unwrap();
                let close = stream.close().unwrap(); assert_eq!(close.facts, opened.facts);
            }
            endpoint.shutdown(); assert_eq!(done(&endpoint).failure, None);
            fixture.assert_requests("ssh-ed25519", 1); fixture.assert_tcp_closed();
        }
    }
}

#[test]
fn overrun_retains_physical_charge_after_worker_and_endpoint_exit() {
    use std::sync::mpsc;
    let (started, entered) = mpsc::channel();
    let (release, finish) = mpsc::channel();
    let setup: Setup = Box::new(move |_| {
        started.send(()).unwrap();
        finish.recv().unwrap();
        Err(io::ErrorKind::Other.into())
    });
    let mut setup = Some(setup);
    let mut c = config();
    c.connect_timeout_ms = 0;
    let endpoint = Endpoint::with_connector(
        c,
        move |origin| {
            SetupConnector::new(
                origin,
                Duration::from_millis(10),
                move |_: &Key, _: &Identity| {
                    setup.take().ok_or_else(|| io::ErrorKind::Other.into())
                },
            )
        },
        100,
    )
    .unwrap();
    let other = endpoint.clone();
    let caller = std::thread::spawn(move || {
        other.open(
            Key::ssh("git", "host", 22),
            Identity::Ambient,
            ssh_channel::GitService::UploadPack,
            "repo",
        )
    });
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    endpoint.shutdown();
    assert!(caller.join().unwrap().is_err());
    let deadline = Instant::now() + Duration::from_secs(2);
    while endpoint.shutdown_status().failure.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let status = endpoint.shutdown_status();
    assert!(!status.cleanup_complete);
    assert_eq!(status.pending_connections, 1);
    assert!(
        endpoint
            .open(
                Key::ssh("git", "host", 22),
                Identity::Ambient,
                ssh_channel::GitService::UploadPack,
                "repo"
            )
            .is_err()
    );
    let watch = endpoint.shutdown_watch();
    let before = Instant::now();
    drop(endpoint);
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(!watch.status().cleanup_complete);
    release.send(()).unwrap();
    while !watch.status().cleanup_complete {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(watch.status().pending_connections, 0);
    assert_eq!(watch.status().failure, Some(io::ErrorKind::TimedOut));
}
#[test]
fn connector_deadline_keeps_original_origin_and_refuses_expired_effects() {
    use ssh_pool::{Connector, Resource};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll, Waker},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut connector = SetupConnector::new(
        Instant::now() - Duration::from_secs(1),
        Duration::from_millis(10),
        move |_: &Key, _: &Identity| -> io::Result<Setup> {
            let calls = observed.clone();
            Ok(Box::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(io::ErrorKind::Other.into())
            }))
        },
    );
    let mut resource = connector
        .start(&Key::ssh("git", "host", 22), &Identity::Ambient, Some(1))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Poll::Ready(result) =
            resource.poll_connected(&mut Context::from_waker(Waker::noop()))
        {
            assert_eq!(
                result.unwrap_err().code,
                gwz_transport::protocol::ErrorCode::Timeout
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        resource.poll_dispose(&mut Context::from_waker(Waker::noop()), true),
        Poll::Ready(Ok(()))
    ));
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        struct Ambient;
        impl ssh_endpoint::IdentityResolver for Ambient {
            fn resolve(&self, _: &Key) -> io::Result<Identity> { Ok(Identity::Ambient) }
        }
        #[test]
        fn per_remote_git_operations_share_setup_but_keep_separate_receipts() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let user = fixture.ssh.user.clone(); let path = fixture.path.clone();
            let setup: Setup = Box::new(move |control| {
                let connection = agent_auth::authenticate(connection, &user, &host, control.clone(), || agent_socket::connect(&path, control))?;
                Authenticated::new(connection, Identity::Ambient, Facts { method: AuthMethod::SshAgent, authenticated: Some(true), credential_offered: true, ..Facts::default() })
            });
            let mut setup = Some(setup);
            let endpoint = Endpoint::with_connector(config(), move |origin| SetupConnector::new(origin, Duration::from_millis(50), move |_: &Key, _: &Identity| setup.take().ok_or_else(|| io::ErrorKind::Other.into())), 1000).unwrap();
            let url = format!("ssh://{}@127.0.0.1:{}/{}", fixture.ssh.user, fixture.ssh.port, fixture.ssh.repository.display());
            let first = Arc::new(Mutex::new(Vec::<Opened>::new())); let second = Arc::new(Mutex::new(Vec::<Opened>::new()));
            for (i, rows) in [first.clone(), second.clone()].into_iter().enumerate() {
                let route = ssh_endpoint::Route::observed(endpoint.clone(), Arc::new(Ambient), Arc::new(move |opened| rows.lock().unwrap().push(opened.clone())));
                let mut options = git2::FetchOptions::new(); options.remote_callbacks(ssh_remote::callbacks(Arc::new(route)));
                let repo = git2::build::RepoBuilder::new().fetch_options(options).clone(&url, &fixture.ssh.temp.path().join(format!("clone-{i}"))).unwrap();
                assert!(repo.is_empty().unwrap());
            }
            let first = first.lock().unwrap(); let second = second.lock().unwrap();
            assert_eq!(first.len(), 1); assert_eq!(second.len(), 1);
            assert!(!first[0].reused); assert!(second[0].reused); assert_eq!(first[0].connection_id, second[0].connection_id);
            assert!(first[0].facts.credential_offered); assert!(!second[0].facts.credential_offered);
            assert_eq!(second[0].facts.authenticated, Some(true));
            endpoint.shutdown(); assert_eq!(done(&endpoint).failure, None); fixture.assert_requests("ssh-ed25519", 1);
        }
        #[test]
        fn worker_shutdown_interrupts_native_sign_without_network_deadline() {
            let fixture = support::Fixture::new("ssh-ed25519", true);
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let user = fixture.ssh.user.clone(); let path = fixture.path.clone();
            let setup: Setup = Box::new(move |control| {
                let connection = agent_auth::authenticate(connection, &user, &host, control.clone(), || agent_socket::connect(&path, control))?;
                Authenticated::new(connection, Identity::Ambient, Facts { method: AuthMethod::SshAgent, authenticated: Some(true), credential_offered: true, ..Facts::default() })
            });
            let mut setup = Some(setup); let mut c = config(); c.connect_timeout_ms = 0; c.cleanup_timeout_ms = 1000;
            let endpoint = Endpoint::with_connector(c, move |origin| SetupConnector::new(origin, Duration::from_secs(1), move |_: &Key, _: &Identity| setup.take().ok_or_else(|| io::ErrorKind::Other.into())), 1000).unwrap();
            let key = Key::ssh(&fixture.ssh.user, "127.0.0.1", fixture.ssh.port); let repo = fixture.ssh.repository.to_str().unwrap().to_owned();
            let other = endpoint.clone(); let caller = std::thread::spawn(move || other.open(key, Identity::Ambient, ssh_channel::GitService::UploadPack, &repo));
            fixture.signing.recv_timeout(Duration::from_secs(3)).unwrap(); endpoint.shutdown(); assert!(caller.join().unwrap().is_err());
            assert_eq!(done(&endpoint).failure, None); fixture.assert_tcp_closed(); fixture.wait_closed();
        }
    }
}

#[test]
fn failed_setup_releases_capacity_for_another_attempt() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let endpoint = Endpoint::with_connector(
        config(),
        move |origin| {
            SetupConnector::new(
                origin,
                Duration::from_millis(50),
                move |_: &Key, _: &Identity| -> io::Result<Setup> {
                    let observed = observed.clone();
                    Ok(Box::new(move |_| {
                        observed.fetch_add(1, Ordering::SeqCst);
                        Err(io::ErrorKind::PermissionDenied.into())
                    }))
                },
            )
        },
        1000,
    )
    .unwrap();
    for _ in 0..2 {
        assert!(
            endpoint
                .open(
                    Key::ssh("git", "host", 22),
                    Identity::Ambient,
                    ssh_channel::GitService::UploadPack,
                    "repo"
                )
                .is_err()
        );
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    endpoint.shutdown();
    assert_eq!(done(&endpoint).failure, None);
}

#[test]
fn cleanup_overrun_stops_admission_without_explicit_shutdown() {
    use std::sync::mpsc;
    let (started, entered) = mpsc::channel();
    let (release, finish) = mpsc::channel();
    let setup: Setup = Box::new(move |_| {
        started.send(()).unwrap();
        finish.recv().unwrap();
        Err(io::ErrorKind::Other.into())
    });
    let mut setup = Some(setup);
    let mut c = config();
    c.connect_timeout_ms = 100;
    let endpoint = Endpoint::with_connector(
        c,
        move |origin| {
            SetupConnector::new(
                origin,
                Duration::from_millis(10),
                move |_: &Key, _: &Identity| {
                    setup.take().ok_or_else(|| io::ErrorKind::Other.into())
                },
            )
        },
        1000,
    )
    .unwrap();
    let other = endpoint.clone();
    let caller = std::thread::spawn(move || {
        other.open(
            Key::ssh("git", "host", 22),
            Identity::Ambient,
            ssh_channel::GitService::UploadPack,
            "repo",
        )
    });
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(caller.join().unwrap().is_err());
    let deadline = Instant::now() + Duration::from_secs(3);
    while endpoint.shutdown_status().failure.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(endpoint.shutdown_status().pending_connections, 1);
    assert!(!endpoint.shutdown_status().cleanup_complete);
    assert!(
        endpoint
            .open(
                Key::ssh("git", "host", 22),
                Identity::Ambient,
                ssh_channel::GitService::UploadPack,
                "repo"
            )
            .is_err()
    );
    release.send(()).unwrap();
    assert_eq!(done(&endpoint).failure, Some(io::ErrorKind::TimedOut));
}

#[test]
fn worker_panic_retains_existing_setup_until_joined_disposal() {
    use std::sync::mpsc;
    let (started, entered) = mpsc::channel();
    let (release, finish) = mpsc::channel();
    let setup: Setup = Box::new(move |_| {
        started.send(()).unwrap();
        finish.recv().unwrap();
        Err(io::ErrorKind::Other.into())
    });
    let mut setup = Some(setup);
    let mut c = config();
    c.total = 2;
    c.connect_timeout_ms = 0;
    let endpoint = Endpoint::with_connector(
        c,
        move |origin| {
            SetupConnector::new(
                origin,
                Duration::from_millis(10),
                move |_: &Key, _: &Identity| -> io::Result<Setup> {
                    Ok(setup.take().expect("injected bounded factory panic"))
                },
            )
        },
        1000,
    )
    .unwrap();
    let other = endpoint.clone();
    let caller = std::thread::spawn(move || {
        other.open(
            Key::ssh("git", "first", 22),
            Identity::Ambient,
            ssh_channel::GitService::UploadPack,
            "repo",
        )
    });
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        endpoint
            .open(
                Key::ssh("git", "second", 22),
                Identity::Ambient,
                ssh_channel::GitService::UploadPack,
                "repo"
            )
            .is_err()
    );
    assert!(caller.join().unwrap().is_err());
    let deadline = Instant::now() + Duration::from_secs(3);
    while endpoint.shutdown_status().failure.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(endpoint.shutdown_status().pending_connections, 1);
    release.send(()).unwrap();
    assert_eq!(done(&endpoint).failure, Some(io::ErrorKind::Other));
}
