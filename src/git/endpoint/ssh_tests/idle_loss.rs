//! Idle sessions the server closed (dev-docs/GwzTransportIdleLossDesign.md):
//! noticed while idle, and replaced when one dies at its channel open. The
//! fixture's `sshd` sits behind a proxy that closes connections from the
//! server's side.
use super::attachment;
use crate::git::endpoint::{
    cut_proxy::CutProxy,
    ssh_channel::GitService,
    ssh_fixture::{DialRetries, SshdFixture},
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
    retries: DialRetries,
}
impl Case {
    /// `arm_new`: each new session dies when it first sends after its setup. `retries`: how the dialer's retried
    /// connections are counted, and whether each dial's first key exchange is made to fail.
    fn new(arm_new: bool, retries: DialRetries) -> Self {
        Self::with_config(arm_new, config(), retries)
    }
    fn with_config(arm_new: bool, config: Config, retries: DialRetries) -> Self {
        let fixture = SshdFixture::new();
        let proxy = Arc::new(CutProxy::start(fixture.port));
        let dial = Arc::new(fixture.dialer_counting(proxy.port, retries.clone()));
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
            retries,
        }
    }
    /// The sessions the proxy's connections carried: its connections less the ones a dial abandoned and replaced
    /// (`DialRetries`), as a key exchange that libssh2 ended on its own side, which Windows CNG does about once in a
    /// hundred.
    fn sessions_dialed(&self) -> usize {
        self.retries.sessions_in(self.proxy.connections())
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
            assert!(
                Instant::now() < deadline,
                "{what}: the pool counts {:?}, the proxy accepted {} connections, {} of them sessions",
                self.endpoint.pool().counts(),
                self.proxy.connections(),
                self.sessions_dialed()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        self.endpoint.shutdown();
    }
}

fn closing_an_idle_session(retries: DialRetries) {
    let case = Case::new(false, retries);
    assert!(!case.exchange().unwrap().reused);
    case.wait_for("the session is idle", |case| {
        case.endpoint.pool().counts().idle == 1
    });
    case.proxy.cut_all();
    case.wait_for("the idle loss is reported", |case| {
        case.endpoint.pool().counts().total() == 0
    });
    assert!(!case.exchange().unwrap().reused);
    assert_eq!(case.sessions_dialed(), 2);
}

#[test]
fn the_server_closing_an_idle_session_frees_its_slot_with_no_open() {
    closing_an_idle_session(DialRetries::default());
}

#[test]
fn the_server_closing_an_idle_session_frees_its_slot_with_no_open_when_each_first_key_exchange_fails()
 {
    closing_an_idle_session(DialRetries::failing_key_exchanges());
}

fn replacing_a_reused_session(retries: DialRetries) {
    let case = Case::new(false, retries);
    case.exchange().unwrap();
    case.wait_for("the session is idle", |case| {
        case.endpoint.pool().counts().idle == 1
    });
    case.proxy.arm_existing();
    let opened = case.exchange().unwrap();
    assert!(!opened.reused);
    assert!(opened.facts.credential_offered);
    assert_eq!(case.sessions_dialed(), 2);
}

#[test]
fn a_reused_session_dying_at_its_channel_open_is_replaced_by_a_fresh_one() {
    replacing_a_reused_session(DialRetries::default());
}

#[test]
fn a_reused_session_dying_at_its_channel_open_is_replaced_by_a_fresh_one_when_each_first_key_exchange_fails()
 {
    replacing_a_reused_session(DialRetries::failing_key_exchanges());
}

/// Two idle sessions, both of which die at their channel open: the open that
/// leases one is retried on a fresh connection, which never leases the other
/// idle one (P3-2 of the idle-loss State review). Were the retry not fresh, it
/// would lease the second dead session and use it up.
fn retry_after_a_dead_lease(retries: DialRetries) {
    let case = Case::with_config(false, sessions(3), retries);
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
    assert_eq!(case.sessions_dialed(), 3);
    case.wait_for("the new session is idle beside the untouched one", |case| {
        case.endpoint.pool().counts().idle == 2
    });
}

#[test]
fn the_retry_after_a_dead_lease_is_fresh_and_leaves_another_idle_session_alone() {
    retry_after_a_dead_lease(DialRetries::default());
}

#[test]
fn the_retry_after_a_dead_lease_is_fresh_and_leaves_another_idle_session_alone_when_each_first_key_exchange_fails()
 {
    retry_after_a_dead_lease(DialRetries::failing_key_exchanges());
}

fn fresh_session_dying(retries: DialRetries) {
    let case = Case::new(true, retries);
    assert!(case.exchange().is_err());
    assert_eq!(case.sessions_dialed(), 1);
}

#[test]
fn a_fresh_session_dying_at_its_channel_open_fails_the_open() {
    fresh_session_dying(DialRetries::default());
}

#[test]
fn a_fresh_session_dying_at_its_channel_open_fails_the_open_when_each_first_key_exchange_fails() {
    fresh_session_dying(DialRetries::failing_key_exchanges());
}

#[test]
fn an_open_cancelled_before_its_channel_opens_is_cancelled() {
    let case = Case::new(false, DialRetries::default());
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

/// State review P2-5: a connection that the peer resets in the middle of an exchange is disposed of, and its slot is
/// free for the next open. The review's trigger is a `shutdown` that fails on the reset socket, which a forced
/// disposal once returned, to be retried at once for ever with the slot held. Microsoft documents such errors, but
/// the Windows host used to qualify the transport does not give one here, so this row passes with and without the
/// fix there; the injected failures in `regression` and in `ssh_connection` are what tell them apart.
fn connection_reset(retries: DialRetries) {
    let case = Case::with_config(false, sessions(1), retries);
    let (mut stream, opened) = attachment::open(
        &case.endpoint,
        case.key(),
        None,
        GitService::UploadPack,
        case.fixture.repository.to_str().unwrap(),
        attachment::deadlines(&sessions(1), 5_000),
    )
    .unwrap();
    assert!(!opened.reused);
    assert_eq!(case.endpoint.pool().counts().total(), 1);
    case.proxy.reset_all();
    // The exchange is over: what the stream reports is a failure or an early end, and either way it closes.
    let mut advertisement = Vec::new();
    let _ = stream.read_to_end(&mut advertisement);
    let _ = stream.close();
    case.wait_for("the reset connection's slot is freed", |case| {
        case.endpoint.pool().counts().total() == 0
    });
    // The one slot serves the next open, on a new connection.
    assert!(!case.exchange().unwrap().reused);
    assert_eq!(case.sessions_dialed(), 2);
}

#[test]
fn a_connection_reset_in_the_middle_of_an_exchange_frees_its_slot_for_the_next_open() {
    connection_reset(DialRetries::default());
}

#[test]
fn a_connection_reset_in_the_middle_of_an_exchange_frees_its_slot_for_the_next_open_when_each_first_key_exchange_fails()
 {
    connection_reset(DialRetries::failing_key_exchanges());
}

mod stress;
