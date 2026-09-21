#![allow(dead_code, unused_imports)]

#[path = "../../../src/git/endpoint/agent_auth.rs"]
mod agent_auth;
#[path = "../../../src/git/endpoint/agent_client.rs"]
mod agent_client;
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
#[path = "../../../src/git/endpoint/agent_socket.rs"]
mod agent_socket;
#[path = "../../../src/git/endpoint/placement_endpoint.rs"]
mod placement_endpoint;
#[path = "../../../src/git/endpoint/ssh_admission.rs"]
mod ssh_admission;
#[path = "../../../src/git/endpoint/ssh_channel.rs"]
mod ssh_channel;
#[path = "../../../src/git/endpoint/ssh_connection.rs"]
mod ssh_connection;
#[path = "../../../src/git/endpoint/ssh_destination.rs"]
mod ssh_destination;
#[path = "../../../src/git/endpoint/ssh_key_auth.rs"]
mod ssh_key_auth;
#[path = "../../../src/git/endpoint/ssh_key_container.rs"]
mod ssh_key_container;
#[path = "../../../src/git/endpoint/ssh_key_snapshot.rs"]
mod ssh_key_snapshot;
#[path = "../../../src/git/endpoint/ssh_network.rs"]
mod ssh_network;
#[path = "../../../src/git/endpoint/ssh_pool.rs"]
mod ssh_pool;
#[path = "../../../src/git/endpoint/ssh_pump.rs"]
mod ssh_pump;
#[path = "../../../src/git/endpoint/ssh_setup.rs"]
mod ssh_setup;
#[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
mod ssh_shutdown;
#[path = "../../../src/git/endpoint/ssh_worker.rs"]
mod ssh_worker;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;

use gwz_transport::{
    pool::Config as PoolConfig,
    protocol::{
        AuthPolicy, CheckIdentity, Deadlines, Destination, Envelope, GitService, Identity,
        IdentityMode, MessageKind, Open,
    },
};
use placement_endpoint::{EndpointError, PlacementEndpoint};
use ssh_pool::Connector;
use ssh_worker::Endpoint;
use std::{
    fs,
    path::PathBuf,
    task::{Context, Waker},
};

fn endpoint() -> Endpoint {
    struct Noop;
    impl Connector for Noop {
        type Resource = ssh_setup::NativeResource;
        fn start(
            &mut self,
            _: &gwz_transport::pool::Key,
            _: &gwz_transport::pool::Identity,
            _: Option<u64>,
        ) -> Result<Self::Resource, gwz_transport::protocol::Failure> {
            Err(gwz_transport::protocol::Failure {
                code: gwz_transport::protocol::ErrorCode::Unavailable,
                effect: gwz_transport::protocol::Effect::None,
                facts: None,
            })
        }
    }
    Endpoint::with_connector(PoolConfig::default(), |_| Noop, 100).unwrap()
}

fn open(identity: Identity) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: 1,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: gwz_transport::protocol::Scheme::Ssh,
                host: "host".into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
            },
            service: GitService::UploadPackExchange,
            identity,
            policy: AuthPolicy::SshExplicit,
            deadlines: Deadlines {
                allocation_ms: 10,
                connect_ms: 10,
                io_ms: 10,
                interaction_ms: 10,
                cleanup_ms: 10,
            },
            receive_limits: gwz_transport::binding::default_limits(),
        }),
        ..Default::default()
    }
}

#[test]
fn placement_open_rejects_relative_path_without_endpoint_base_before_worker_effects() {
    let mut endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    let result = endpoint.accept(
        "request".into(),
        open(Identity {
            mode: IdentityMode::ExplicitKey,
            key_path: Some("id_ed25519".into()),
            path_base: None,
        }),
    );
    assert_eq!(result, Ok(()));
    let failure = endpoint.take_outbound().expect("typed open failure");
    gwz_transport::codec::admit(&failure.envelope).unwrap();
    assert_eq!(failure.envelope.kind, MessageKind::OpenFailed);
    assert_eq!(
        failure.envelope.open_failed.expect("failure").code,
        gwz_transport::protocol::ErrorCode::InvalidRequest
    );
}

#[test]
fn one_operation_can_own_parallel_streams_and_shutdown_preserves_terminals() {
    let mut endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    let mut first = open(Identity::default());
    let mut second = open(Identity::default());
    second.stream_id = 2;
    endpoint.accept("operation".into(), first.clone()).unwrap();
    endpoint.accept("operation".into(), second).unwrap();
    assert!(endpoint.pending_request("operation"));
    endpoint.shutdown();
    assert!(endpoint.take_outbound().is_none());
    assert!(endpoint.pending_request("operation"));
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    for _ in 0..100 {
        endpoint.step(1_000, &mut cx).unwrap();
        if endpoint.pending() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(endpoint.pending(), 0);
    first.stream_id = 2;
    assert_eq!(
        endpoint.accept("operation".into(), first),
        Err(EndpointError::Shutdown)
    );
}

#[test]
fn check_identity_path_rejection_is_typed_and_has_no_facts() {
    let mut endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    endpoint
        .accept(
            "check".into(),
            Envelope {
                version: 2,
                session_id: "session".into(),
                stream_id: 7,
                kind: MessageKind::CheckIdentity,
                check_identity: Some(CheckIdentity {
                    endpoint_id: "endpoint".into(),
                    operation_id: "check-op".into(),
                    identity: Identity {
                        mode: IdentityMode::ExplicitKey,
                        key_path: Some("relative-key".into()),
                        path_base: None,
                    },
                    timeout_ms: 10,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let result = endpoint.take_outbound().expect("typed check failure");
    gwz_transport::codec::admit(&result.envelope).unwrap();
    let failure = result
        .envelope
        .identity_check_failed
        .expect("identity failure");
    assert_eq!(
        failure.code,
        gwz_transport::protocol::ErrorCode::InvalidRequest
    );
    assert_eq!(failure.effect, gwz_transport::protocol::Effect::None);
    assert!(failure.facts.is_none());
}

fn check(path: String, stream_id: i64) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::CheckIdentity,
        check_identity: Some(CheckIdentity {
            endpoint_id: "endpoint".into(),
            operation_id: format!("check-{stream_id}"),
            identity: Identity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some(path),
                path_base: None,
            },
            timeout_ms: 1_000,
        }),
        ..Default::default()
    }
}

#[test]
fn check_identity_reads_regular_file_without_parsing_and_reports_missing_file() {
    let path = std::env::temp_dir().join(format!("gwz-placement-{}", std::process::id()));
    fs::write(&path, b"not an ssh key").unwrap();
    let mut regular_endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    regular_endpoint
        .accept(
            "check".into(),
            check(path.to_string_lossy().into_owned(), 9),
        )
        .unwrap();
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    for _ in 0..100 {
        regular_endpoint.step(0, &mut cx).unwrap();
        if regular_endpoint.pending() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let checked = regular_endpoint.take_outbound().unwrap();
    gwz_transport::codec::admit(&checked.envelope).unwrap();
    assert_eq!(checked.envelope.kind, MessageKind::IdentityChecked);
    fs::remove_file(&path).unwrap();

    let mut endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    endpoint
        .accept(
            "missing".into(),
            check(path.to_string_lossy().into_owned(), 10),
        )
        .unwrap();
    for _ in 0..100 {
        endpoint.step(0, &mut cx).unwrap();
        if endpoint.pending() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let failed = endpoint.take_outbound().unwrap();
    gwz_transport::codec::admit(&failed.envelope).unwrap();
    let failure = failed.envelope.identity_check_failed.unwrap();
    assert_eq!(
        failure.code,
        gwz_transport::protocol::ErrorCode::Unavailable
    );
    assert_eq!(failure.effect, gwz_transport::protocol::Effect::None);
    assert!(failure.facts.is_none());
}

#[test]
fn preopen_cancellation_contains_open_failed_payload_accepted_by_codec() {
    let mut endpoint = PlacementEndpoint::new(
        endpoint(),
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    endpoint
        .accept("cancel".into(), open(Identity::default()))
        .unwrap();
    endpoint.cancel_request("cancel");
    let outbound = endpoint.take_outbound().expect("cancel terminal");
    assert_eq!(outbound.envelope.kind, MessageKind::OpenFailed);
    assert_eq!(
        outbound
            .envelope
            .open_failed
            .as_ref()
            .expect("open failure payload")
            .code,
        gwz_transport::protocol::ErrorCode::Cancelled
    );
    gwz_transport::codec::admit(&outbound.envelope).unwrap();
}

#[test]
fn message_deadlines_tighten_pool_policy_and_cannot_disable_a_positive_policy() {
    use std::sync::{Arc, Mutex};
    struct Record(Arc<Mutex<Vec<Option<u64>>>>);
    impl Connector for Record {
        type Resource = ssh_setup::NativeResource;
        fn start(
            &mut self,
            _: &gwz_transport::pool::Key,
            _: &gwz_transport::pool::Identity,
            deadline: Option<u64>,
        ) -> Result<Self::Resource, gwz_transport::protocol::Failure> {
            self.0.lock().unwrap().push(deadline);
            Err(gwz_transport::protocol::Failure {
                code: gwz_transport::protocol::ErrorCode::Unavailable,
                effect: gwz_transport::protocol::Effect::None,
                facts: None,
            })
        }
    }
    for (configured, requested, admitted) in [
        (3000, 0, false),
        (3000, 100, true),
        (0, 100, true),
        (0, 0, true),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let worker = Endpoint::with_connector(
            PoolConfig {
                connect_timeout_ms: configured,
                ..Default::default()
            },
            |_| Record(calls.clone()),
            configured,
        )
        .unwrap();
        let mut bridge = PlacementEndpoint::new(
            worker,
            PathBuf::from("/endpoint-home"),
            "endpoint".into(),
            "owner".into(),
        )
        .unwrap();
        let mut message = open(Identity::default());
        let body = message.open.as_mut().unwrap();
        body.policy = AuthPolicy::SshAmbient;
        body.deadlines.allocation_ms = 1000;
        body.deadlines.interaction_ms = 1000;
        body.deadlines.connect_ms = requested;
        body.deadlines.io_ms = requested;
        gwz_transport::codec::admit(&message).unwrap();
        bridge.accept("r".into(), message).unwrap();
        let start = std::time::Instant::now();
        let mut cx = Context::from_waker(Waker::noop());
        let outcome = loop {
            bridge
                .step(start.elapsed().as_millis() as u64, &mut cx)
                .unwrap();
            if let Some(outcome) = bridge.take_outbound() {
                break outcome;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(2),
                "open reply stranded"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        gwz_transport::codec::admit(&outcome.envelope).unwrap();
        let code = outcome.envelope.open_failed.unwrap().code;
        let calls = calls.lock().unwrap();
        if admitted {
            assert_eq!(code, gwz_transport::protocol::ErrorCode::Unavailable);
            assert_eq!(calls.len(), 1);
            if requested == 0 {
                assert_eq!(calls[0], None);
            } else {
                assert!(
                    calls[0].is_some_and(|at| at < 500),
                    "request must tighten configured 3s timeout: {calls:?}"
                );
            }
        } else {
            assert_eq!(code, gwz_transport::protocol::ErrorCode::InvalidRequest);
            assert!(
                calls.is_empty(),
                "invalid timeout must fail before connection work"
            );
        }
        bridge.shutdown();
    }
}

#[test]
fn open_timeout_replies_before_blocked_physical_work_finishes() {
    use std::sync::{Arc, Mutex, mpsc};
    struct Blocked {
        entered: mpsc::SyncSender<()>,
        release: Arc<Mutex<mpsc::Receiver<()>>>,
    }
    impl Connector for Blocked {
        type Resource = ssh_setup::NativeResource;
        fn start(
            &mut self,
            _: &gwz_transport::pool::Key,
            _: &gwz_transport::pool::Identity,
            _: Option<u64>,
        ) -> Result<Self::Resource, gwz_transport::protocol::Failure> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Err(gwz_transport::protocol::Failure {
                code: gwz_transport::protocol::ErrorCode::Unavailable,
                effect: gwz_transport::protocol::Effect::None,
                facts: None,
            })
        }
    }
    let (entered, seen) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::sync_channel(1);
    let worker = Endpoint::with_connector(
        PoolConfig::default(),
        |_| Blocked {
            entered,
            release: Arc::new(Mutex::new(blocked)),
        },
        3000,
    )
    .unwrap();
    let mut bridge = PlacementEndpoint::new(
        worker,
        PathBuf::from("/endpoint-home"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    let mut message = open(Identity::default());
    message.open.as_mut().unwrap().policy = AuthPolicy::SshAmbient;
    bridge.accept("r".into(), message).unwrap();
    seen.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    bridge.step(1000, &mut cx).unwrap();
    let result = bridge.take_outbound();
    let pending = bridge.pending_request_count("r");
    // Release even if an assertion fails, so the deliberately blocked test worker
    // cannot obscure the logical timeout failure with its own teardown wait.
    release.send(()).unwrap();
    let result = result.expect("logical timeout reply must not wait for disposal");
    gwz_transport::codec::admit(&result.envelope).unwrap();
    assert_eq!(
        result.envelope.open_failed.unwrap().code,
        gwz_transport::protocol::ErrorCode::Timeout
    );
    assert!(
        pending > 0,
        "unresolved physical work remains accounted after reply"
    );
    bridge.shutdown();
}
