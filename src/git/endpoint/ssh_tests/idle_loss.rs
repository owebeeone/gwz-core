//! Idle sessions the server closed (dev-docs/GwzTransportIdleLossDesign.md):
//! noticed while idle, and replaced when one dies at its channel open. The
//! fixture's `sshd` sits behind a proxy that closes connections from the
//! server's side.
use super::attachment;
use crate::git::endpoint::{
    cut_proxy::CutProxy,
    ssh_channel::GitService,
    ssh_fixture::SshdFixture,
    ssh_key_snapshot::Registry,
    ssh_setup::{Authenticated, Setup, SetupConnector},
    ssh_worker::Endpoint,
    ssh_worker::EndpointOpenFailure,
};
use gwz_transport::{
    pool::{Config, Identity, Key},
    protocol::{AuthMethod, ErrorCode, Facts, Opened},
};
use std::{
    io::{self, Read, Write},
    sync::{Arc, atomic::AtomicBool, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

fn config() -> Config {
    sessions(2)
}
/// Room for `count` sessions.
fn sessions(count: usize) -> Config {
    Config {
        total: count,
        per_host: count,
        per_user_host: count,
        ..Config::default()
    }
}
struct Case {
    fixture: SshdFixture,
    proxy: Arc<CutProxy>,
    endpoint: Endpoint,
}
impl Case {
    /// `arm_new`: each new session dies when it first sends after its setup.
    fn new(arm_new: bool) -> Self {
        Self::with_config(arm_new, config())
    }
    fn with_config(arm_new: bool, config: Config) -> Self {
        let fixture = SshdFixture::new();
        let proxy = Arc::new(CutProxy::start(fixture.port));
        let dial = Arc::new(fixture.dialer(proxy.port));
        let armed = proxy.clone();
        let endpoint = Endpoint::with_registry(
            config,
            Registry::new(),
            move |origin, _| {
                SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                    let (dial, armed) = (dial.clone(), armed.clone());
                    let setup: Setup = Box::new(move |_| {
                        let connection = dial()?;
                        if arm_new {
                            armed.arm_existing();
                        }
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
                })
            },
            5_000,
        )
        .unwrap();
        Self {
            fixture,
            proxy,
            endpoint,
        }
    }
    fn key(&self) -> Key {
        Key::ssh(&self.fixture.user, "127.0.0.1", self.fixture.port)
    }
    /// One upload-pack exchange, to its end: its session is then idle.
    fn exchange(&self) -> io::Result<Opened> {
        let (mut stream, opened) = attachment::open(
            &self.endpoint,
            self.key(),
            None,
            GitService::UploadPack,
            self.fixture.repository.to_str().unwrap(),
            attachment::deadlines(&config(), 5_000),
        )?;
        let mut advertisement = Vec::new();
        stream.write_all(b"0000")?;
        stream.end_write()?;
        stream.read_to_end(&mut advertisement)?;
        stream.close()?;
        Ok(opened)
    }
    fn wait_for(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(self) {
            assert!(Instant::now() < deadline, "{what}");
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        self.endpoint.shutdown();
    }
}

#[test]
fn the_server_closing_an_idle_session_frees_its_slot_with_no_open() {
    let case = Case::new(false);
    assert!(!case.exchange().unwrap().reused);
    case.wait_for("the session is idle", |case| {
        case.endpoint.pool().counts().idle == 1
    });
    case.proxy.cut_all();
    case.wait_for("the idle loss is reported", |case| {
        case.endpoint.pool().counts().total() == 0
    });
    assert!(!case.exchange().unwrap().reused);
    assert_eq!(case.proxy.connections(), 2);
}

#[test]
fn a_reused_session_dying_at_its_channel_open_is_replaced_by_a_fresh_one() {
    let case = Case::new(false);
    case.exchange().unwrap();
    case.wait_for("the session is idle", |case| {
        case.endpoint.pool().counts().idle == 1
    });
    case.proxy.arm_existing();
    let opened = case.exchange().unwrap();
    assert!(!opened.reused);
    assert!(opened.facts.credential_offered);
    assert_eq!(case.proxy.connections(), 2);
}

/// Two idle sessions, both of which die at their channel open: the open that
/// leases one is retried on a fresh connection, which never leases the other
/// idle one (P3-2 of the idle-loss State review). Were the retry not fresh, it
/// would lease the second dead session and use it up.
#[test]
fn the_retry_after_a_dead_lease_is_fresh_and_leaves_another_idle_session_alone() {
    let case = Case::with_config(false, sessions(3));
    // Two exchanges at once, so that the pool opens two sessions.
    let repository = case.fixture.repository.to_str().unwrap();
    let mut both = Vec::new();
    for _ in 0..2 {
        both.push(
            attachment::open(
                &case.endpoint,
                case.key(),
                None,
                GitService::UploadPack,
                repository,
                attachment::deadlines(&sessions(3), 5_000),
            )
            .unwrap(),
        );
    }
    for (mut stream, _) in both {
        let mut advertisement = Vec::new();
        stream.write_all(b"0000").unwrap();
        stream.end_write().unwrap();
        stream.read_to_end(&mut advertisement).unwrap();
        stream.close().unwrap();
    }
    case.wait_for("both sessions are idle", |case| {
        case.endpoint.pool().counts().idle == 2
    });
    case.proxy.arm_existing();
    let opened = case.exchange().unwrap();
    assert!(!opened.reused);
    assert_eq!(case.proxy.connections(), 3);
    case.wait_for("the new session is idle beside the untouched one", |case| {
        case.endpoint.pool().counts().idle == 2
    });
}

#[test]
fn a_fresh_session_dying_at_its_channel_open_fails_the_open() {
    let case = Case::new(true);
    assert!(case.exchange().is_err());
    assert_eq!(case.proxy.connections(), 1);
}

#[test]
fn an_open_cancelled_before_its_channel_opens_is_cancelled() {
    let case = Case::new(false);
    case.exchange().unwrap();
    case.wait_for("the session is idle", |case| {
        case.endpoint.pool().counts().idle == 1
    });
    // The server stops answering: the reused session's channel never opens.
    case.proxy.freeze_existing();
    let cancelled = Arc::new(AtomicBool::new(false));
    let open = case
        .endpoint
        .start_endpoint_open(
            case.key(),
            None,
            GitService::UploadPack,
            case.fixture.repository.to_str().unwrap(),
            attachment::context(attachment::deadlines(&config(), 5_000)),
            cancelled.clone(),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(open.poll().is_pending(), "the reply waits for the channel");
    cancelled.store(true, Ordering::Release);
    let error = attachment::finish(&open)
        .err()
        .expect("the open is cancelled");
    let failure = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
        .expect("a typed open failure");
    assert_eq!(failure.failure.code, ErrorCode::Cancelled);
}
