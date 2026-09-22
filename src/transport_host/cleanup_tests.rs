use crate::git::endpoint::{
    placement_endpoint::PlacementEndpoint,
    ssh_channel::{GitService, SshChannel},
    ssh_pool::{Connector, Resource},
    ssh_pump::SshPump,
    ssh_worker::{ChannelResource, Endpoint},
};
use gwz_transport::{
    pool::{self, Identity, Key},
    protocol::{
        AuthPolicy, Deadlines, Destination, Envelope, Failure, GitService as WireService,
        Identity as WireIdentity, MessageKind, Open, Scheme,
    },
    stream::{MessageEndpoint, Stream},
};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

struct GateResource {
    released: Arc<AtomicBool>,
    disposing: Arc<AtomicBool>,
}
struct GateConnector {
    released: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
    disposing: Arc<AtomicBool>,
}
impl Connector for GateConnector {
    type Resource = GateResource;
    fn start(
        &mut self,
        _key: &Key,
        _identity: &Identity,
        _deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        self.entered.store(true, Ordering::Release);
        Ok(GateResource {
            disposing: self.disposing.clone(),
            released: self.released.clone(),
        })
    }
}
impl Resource for GateResource {
    fn poll_connected(&mut self, _cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Ready(Ok(Some(Identity::Ambient)))
    }
    fn poll_dispose(&mut self, _cx: &mut Context<'_>, _force: bool) -> Poll<io::Result<()>> {
        self.disposing.store(true, Ordering::Release);
        if self.released.load(Ordering::Acquire) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for GateResource {
    fn start_exchange(
        &mut self,
        _stream: Stream,
        _endpoint: MessageEndpoint,
        _service: GitService,
        _path: &str,
    ) -> io::Result<()> {
        Ok(())
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        true
    }
}

#[test]
fn shutdown_reports_blocked_physical_disposal_then_eventual_zero() {
    let released = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicBool::new(false));
    let disposing = Arc::new(AtomicBool::new(false));
    let mut config = pool::Config::default();
    config.total = 1;
    config.per_host = 1;
    config.cleanup_timeout_ms = 20;
    let endpoint = Endpoint::with_connector(
        config,
        |_| GateConnector {
            entered: entered.clone(),
            disposing: disposing.clone(),
            released: released.clone(),
        },
        3_000,
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let engine = PlacementEndpoint::new(
        endpoint,
        home.path().to_path_buf(),
        "endpoint".into(),
        "trust".into(),
    )
    .unwrap();
    let mut initial = super::Session::empty();
    initial.engine = Some(engine);
    let session = super::Session::start(initial).unwrap();
    session
        .register("request", Some("operation".into()))
        .unwrap();
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: 1,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: Scheme::Ssh,
                host: "example.test".into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
            },
            service: WireService::UploadPackAdvertisement,
            identity: WireIdentity::default(),
            policy: AuthPolicy::SshAmbient,
            deadlines: Deadlines {
                allocation_ms: 100,
                connect_ms: 100,
                io_ms: 3_000,
                interaction_ms: 100,
                cleanup_ms: 20,
            },
            receive_limits: super::limits(),
        }),
        ..Default::default()
    };
    {
        let mut state = session.state.lock().unwrap();
        state
            .engine
            .as_mut()
            .unwrap()
            .accept("request".into(), envelope)
            .unwrap();
    }
    let until = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::Acquire) {
        assert!(Instant::now() < until, "physical setup was never admitted");
        std::thread::sleep(Duration::from_millis(2));
    }
    session.drive();
    session.cancel("request");
    session.close();
    while !disposing.load(Ordering::Acquire) {
        assert!(Instant::now() < until, "physical disposal was never polled");
        std::thread::sleep(Duration::from_millis(2));
    }
    let blocked = session.report();
    assert!(
        blocked.pending_local_work > 0,
        "cleanup snapshot must retain physical work"
    );
    released.store(true, Ordering::Release);
    let report = crate::transport_host::driver_tests::block_on(session.cleanup());
    assert_eq!(report.pending_local_work, 0);
}

#[test]
fn completed_request_retirement_cannot_expire_the_shared_session() {
    use crate::transport_host::{SshEndpointConfig, TransportRuntime};
    use crate::{RequestMeta, TransportOptions, TransportPlacement};
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(
        std::path::PathBuf::from("/nonexistent-endpoint-home"),
        None,
    ))
    .unwrap();
    let request_meta = |id: &str| RequestMeta {
        request_id: id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(TransportPlacement::Local),
            ..Default::default()
        }),
        ..Default::default()
    };
    let request = crate::transport_host::driver_tests::block_on(
        runtime.request(request_meta("retired"), "fetch".into()),
    )
    .unwrap();
    let session = request.context.session.clone();
    let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
    assert_eq!(
        crate::transport_host::driver_tests::block_on(request.finish()).pending_local_work,
        0
    );
    for side in [&session, &endpoint] {
        let mut state = side.state.lock().unwrap();
        let record = state.registrations.get_mut("retired").unwrap();
        assert!(record.result.is_some());
        // Advance only the retained cleanup age, without sleeping or touching
        // another timeout domain. Retirement already completed successfully.
        record.sealed = Some(Instant::now() - super::CLEANUP - Duration::from_millis(1));
    }
    session.drive();
    endpoint.drive();
    assert!(
        !endpoint.is_closed(),
        "completed retirement killed endpoint session"
    );
    assert!(
        !session.is_closed(),
        "completed retirement killed the shared session"
    );
    let next = crate::transport_host::driver_tests::block_on(
        runtime.request(request_meta("next"), "fetch".into()),
    )
    .unwrap();
    assert_eq!(
        crate::transport_host::driver_tests::block_on(next.finish()).pending_local_work,
        0
    );
    assert_eq!(
        crate::transport_host::driver_tests::block_on(runtime.shutdown()).pending_local_work,
        0
    );
}
