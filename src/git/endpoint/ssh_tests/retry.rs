//! The setup retry machine on the endpoint production builds, against the
//! fixture's sshd: a key whose setup succeeded and whose server then died
//! spends its retries once for the whole operation (the retry plan's S3.1,
//! as `--jobs 1` runs the operation's members one after another), and one
//! identity's refused key closes only that identity's machine.
use crate::git::endpoint::{
    placement_endpoint::PlacementEndpoint,
    setup_retry::Jitter,
    shared_reservation::Authority,
    ssh_fixture::{SshdFixture, run_fixture_key},
    ssh_local,
};
use gwz_transport::{
    pool::Config,
    protocol::{
        AuthPolicy, Cancel, Deadlines, Destination, Envelope, ErrorCode, GitService, Identity,
        IdentityMode, MessageKind, Open, Scheme,
    },
};
use std::{
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Waker},
    thread,
    time::{Duration, Instant},
};

const OPERATION: &str = "operation";

/// Stream `stream_id`'s Open, with the key the fixture's server
/// authorizes.
fn open(fixture: &SshdFixture, stream_id: i64) -> Envelope {
    open_as(
        fixture,
        stream_id,
        &fixture.temp.path().join("client_ed25519"),
    )
}
/// Stream `stream_id`'s Open, with the explicit key `key`.
fn open_as(fixture: &SshdFixture, stream_id: i64, key: &Path) -> Envelope {
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: Scheme::Ssh,
                host: "127.0.0.1".into(),
                port: fixture.port as i64,
                path: fixture.repository.to_str().unwrap().into(),
                ssh_username: Some(fixture.user.clone()),
                https_username: None,
            },
            service: GitService::UploadPackExchange,
            identity: Identity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some(key.to_str().unwrap().into()),
                path_base: None,
            },
            policy: AuthPolicy::SshExplicit,
            deadlines: Deadlines {
                allocation_ms: 10_000,
                connect_ms: 10_000,
                io_ms: 3_000,
                interaction_ms: 10_000,
                cleanup_ms: 1_000,
            },
            receive_limits: gwz_transport::binding::default_limits(),
        }),
        ..Default::default()
    };
    gwz_transport::codec::admit(&envelope).unwrap();
    envelope
}
fn cancel(stream_id: i64) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Cancel,
        cancel: Some(Cancel {
            reason: ErrorCode::Cancelled,
        }),
        ..Default::default()
    }
}
/// Steps until `stream_id`'s open ends or opens. Whenever no attempt is
/// in flight the endpoint's clock moves on 5 s, past any of the
/// default budget's waits, so no test waits out a real backoff.
fn reply(endpoint: &mut PlacementEndpoint, now: &mut u64, stream_id: i64) -> Envelope {
    let mut cx = Context::from_waker(Waker::noop());
    let begun = Instant::now();
    loop {
        endpoint.step(*now, &mut cx).unwrap();
        while let Some(item) = endpoint.take_outbound() {
            if item.envelope.stream_id == stream_id
                && matches!(
                    item.envelope.kind,
                    MessageKind::Opened | MessageKind::OpenFailed
                )
            {
                return item.envelope;
            }
        }
        assert!(
            begun.elapsed() < Duration::from_secs(30),
            "no reply for {stream_id}"
        );
        if endpoint.attempts_in_flight_for_test() == 0 {
            *now += 5_000;
        }
        thread::sleep(Duration::from_millis(2));
    }
}
/// A listener on `port` that closes every connection it accepts, as a
/// dead server's address might; it counts them.
fn closing(port: u16) -> Arc<AtomicUsize> {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            counter.fetch_add(1, Ordering::AcqRel);
            drop(stream);
        }
    });
    accepted
}

#[test]
fn a_key_that_set_up_and_then_died_spends_its_retries_once_for_the_operation() {
    let mut fixture = SshdFixture::new();
    let endpoint = ssh_local::connect_with_authority(
        Config::default(),
        fixture.known_hosts.clone(),
        None,
        3_000,
        Authority::new(256, 32),
        Default::default(),
    )
    .unwrap();
    let mut placement = PlacementEndpoint::new(
        endpoint,
        std::env::temp_dir(),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    placement.set_jitter(Jitter::fixed(0));
    let mut now = 0;
    // The first member sets up a session: the key is Healthy.
    placement
        .accept(OPERATION.into(), open(&fixture, 1))
        .unwrap();
    assert_eq!(reply(&mut placement, &mut now, 1).kind, MessageKind::Opened);
    placement.accept(OPERATION.into(), cancel(1)).unwrap();
    // The server dies, and what answers its address closes at once.
    fixture.child.kill().unwrap();
    fixture.child.wait().unwrap();
    let accepted = closing(fixture.port);
    // The next member's four attempts each fail, and it finishes.
    placement
        .accept(OPERATION.into(), open(&fixture, 2))
        .unwrap();
    let failed = reply(&mut placement, &mut now, 2);
    assert_eq!(failed.kind, MessageKind::OpenFailed);
    assert_eq!(failed.open_failed.as_ref().unwrap().code, ErrorCode::Io);
    assert_eq!(
        accepted.load(Ordering::Acquire),
        4,
        "four attempts after Healthy"
    );
    // Every later member of the operation finishes with that failure,
    // without a connection: the key is Closed, not Cold.
    for stream_id in 3..=5 {
        placement
            .accept(OPERATION.into(), open(&fixture, stream_id))
            .unwrap();
        let late = reply(&mut placement, &mut now, stream_id);
        assert_eq!(late.open_failed.as_ref().unwrap().code, ErrorCode::Io);
    }
    assert_eq!(accepted.load(Ordering::Acquire), 4);
    placement.shutdown();
}

#[test]
fn one_identitys_refused_key_closes_only_its_own_machine_on_the_host() {
    let fixture = SshdFixture::new();
    // A key the server does not authorize, beside the one it does: a
    // workspace can name either for a remote (gwzSshIdentity).
    let stranger = fixture.temp.path().join("stranger_ed25519");
    run_fixture_key(&stranger);
    let endpoint = ssh_local::connect_with_authority(
        Config::default(),
        fixture.known_hosts.clone(),
        None,
        3_000,
        Authority::new(256, 32),
        Default::default(),
    )
    .unwrap();
    let mut placement = PlacementEndpoint::new(
        endpoint,
        std::env::temp_dir(),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    placement.set_jitter(Jitter::fixed(0));
    let mut now = 0;
    // The stranger's member is refused, which closes its identity's
    // machine for the operation: its next member finishes the same way.
    for stream_id in [1, 2] {
        placement
            .accept(OPERATION.into(), open_as(&fixture, stream_id, &stranger))
            .unwrap();
        let refused = reply(&mut placement, &mut now, stream_id);
        assert_eq!(refused.kind, MessageKind::OpenFailed);
        assert_eq!(
            refused.open_failed.as_ref().unwrap().code,
            ErrorCode::Authentication
        );
    }
    // A member of the same host with the key the server takes still
    // sets up and opens, as 1.0.17 authenticated each member on its
    // own.
    placement
        .accept(OPERATION.into(), open(&fixture, 3))
        .unwrap();
    assert_eq!(reply(&mut placement, &mut now, 3).kind, MessageKind::Opened);
    placement.accept(OPERATION.into(), cancel(3)).unwrap();
    placement.shutdown();
}
