//! The adapter against the in-process carrier: messages cross unchanged in both directions, a full lane gives the
//! message back, a closed carrier ends the lane, and the core end's registry answers from the session.
use super::{Calls, CarrierLane};
use crate::transport_host::{
    Attachment,
    carrier::{
        Carrier, CarrierBinding, CarrierId, CarrierLimits, CloseCode, Peer, Side, StreamCarrier,
        in_process,
    },
};
use gwz_connection::{
    Clock, DriverWork, HostNamespace, Progress, ProgressUnavailable, Sleep, Tick,
    remote::{CoreBinding, Lane, LaneEnded, LaneSend},
};
use gwz_transport::protocol::{EndpointRole, MessageKind};
use std::sync::Arc;

struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Tick {
        Tick::new(0)
    }
    fn sleep_until(&self, _deadline: Tick) -> Sleep {
        Box::pin(std::future::pending())
    }
    fn epoch(&self) -> u64 {
        0
    }
}
/// An executor that keeps what it is given and runs none of it: these rows read the lane, not the drivers.
struct Held;
impl Progress for Held {
    fn check(&self) -> Result<(), ProgressUnavailable> {
        Ok(())
    }
    fn spawn(&self, work: DriverWork) -> Result<(), ProgressUnavailable> {
        drop(work);
        Ok(())
    }
}
struct Registry;
impl Calls for Registry {
    fn is_live(&self, request_id: &str) -> bool {
        request_id == "call-1"
    }
    fn endpoint_path_base(&self, request_id: &str) -> Option<String> {
        (request_id == "call-1").then(|| "/home/me".to_owned())
    }
}

type Dyn = Arc<dyn Carrier>;
fn offer(session: &str) -> gwz_transport::protocol::Envelope {
    gwz_transport::binding::offer(session, EndpointRole::Driver)
}
fn bind(request: &str, session: &str) -> Attachment {
    (request.to_owned(), offer(session))
}
fn ends() -> (Arc<CarrierLane>, Arc<CarrierLane>, Dyn, Dyn) {
    let (a, b) = in_process(CarrierLimits::DEFAULT).unwrap();
    let (a, b): (Dyn, Dyn) = (Arc::new(a), Arc::new(b));
    let core = CarrierLane::core(
        a.clone(),
        Arc::new(Fixed),
        Arc::new(Held),
        HostNamespace::new(9),
        Arc::new(Registry),
    );
    let client = CarrierLane::client(b.clone(), Arc::new(Fixed), Arc::new(Held));
    (core, client, a, b)
}

#[test]
fn messages_cross_the_lane_unchanged_in_both_directions() {
    let (core, client, _a, _b) = ends();
    core.send(bind("call-1", "s.1")).unwrap();
    let got = client.recv().unwrap().expect("a message");
    assert_eq!(
        (got.0.as_str(), got.1.kind, got.1.session_id.as_str()),
        ("call-1", MessageKind::Bind, "s.1")
    );
    assert!(client.recv().unwrap().is_none());
    let mut reply = offer("s.1");
    reply.kind = MessageKind::Bound;
    assert!(client.send(("call-1".into(), reply)).is_ok());
    assert_eq!(
        core.recv().unwrap().expect("a reply").1.kind,
        MessageKind::Bound
    );
}

#[test]
fn the_core_end_answers_from_its_session() {
    let (core, client, _a, _b) = ends();
    assert_eq!(core.namespace(), HostNamespace::new(9));
    assert!(core.call_is_live("call-1") && !core.call_is_live("call-2"));
    assert_eq!(
        core.endpoint_path_base("call-1").as_deref(),
        Some("/home/me")
    );
    assert_eq!(core.endpoint_path_base("call-2"), None);
    assert!(
        !client.call_is_live("call-1"),
        "a client end has no registry"
    );
}

#[test]
fn a_full_lane_gives_the_message_back_and_a_closed_carrier_ends_the_lane() {
    let (core, client, a, _b) = ends();
    // A control message is accepted until the lane's control reserve is full; the refused one comes back whole.
    let mut refused = None;
    for n in 0..2000 {
        if let Err(error) = core.send(bind("call-1", &format!("s.{n}"))) {
            refused = Some(error);
            break;
        }
    }
    match refused {
        Some(LaneSend::Full(message)) => assert_eq!(message.0, "call-1"),
        other => panic!("expected a full lane, got {other:?}"),
    }
    a.close(CloseCode::Normal);
    // What the peer sent before it closed still arrives; then the lane ends.
    let mut seen = 0;
    while client.recv().is_ok_and(|message| message.is_some()) {
        seen += 1;
    }
    assert!(seen > 0);
    assert_eq!(client.recv(), Err(LaneEnded));
    assert!(matches!(
        client.send(bind("call-1", "s.late")),
        Err(LaneSend::Closed(_))
    ));
}

/// Two stream carriers over OS pipes with no binding open on either end, each seen as a lane: what a served session has
/// before its first `bind`. The carriers come back too, for the rows that send a registry frame or read one.
fn stream_ends() -> (
    Arc<CarrierLane>,
    Arc<CarrierLane>,
    Arc<StreamCarrier>,
    Arc<StreamCarrier>,
) {
    let (core_read, client_write) = std::io::pipe().expect("pipe");
    let (client_read, core_write) = std::io::pipe().expect("pipe");
    let id = CarrierId::random().expect("random");
    let end = |read, write, side| {
        Arc::new(
            StreamCarrier::new(
                Box::new(read),
                Box::new(write),
                Box::new(|| {}),
                CarrierBinding::new(id, side, Peer::InProcess),
                CarrierLimits::DEFAULT,
            )
            .expect("a carrier"),
        )
    };
    let core = end(core_read, core_write, Side::Core);
    let client = end(client_read, client_write, Side::Client);
    (
        CarrierLane::core(
            core.clone(),
            Arc::new(Fixed),
            Arc::new(Held),
            HostNamespace::new(9),
            Arc::new(Registry),
        ),
        CarrierLane::client(client.clone(), Arc::new(Fixed), Arc::new(Held)),
        core,
        client,
    )
}
fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn a_handle_registers_over_a_stream_carrier_whose_bindings_the_adapter_opens_retires_and_reports_faulted()
 {
    use gwz_connection::{
        ExchangeSpec, GitService, OpenFailure, OperationRef, Registration, remote::handle,
    };
    use gwz_session_contract::{Frame, FrameKey, Target};
    use gwz_transport::{
        binding::{EndpointConfig, default_limits},
        mux::{Config, Mux},
        pool::{Identity, Key},
        protocol::{AuthPolicy, Cancel, ErrorCode, Scheme},
    };
    let (core, client, core_carrier, client_carrier) = stream_ends();
    let host = handle(core.clone());
    // The core's call, its admission, and a deferred commit: the `bind` leaves as a `TransportLaneFrame` and the
    // client's end, which has opened nothing, admits it.
    let admission = host.admit(OperationRef::new("call-1").unwrap()).unwrap();
    let permit = {
        let mut acquire = Box::pin(admission.acquire());
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match acquire.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(permit) => permit.unwrap(),
            std::task::Poll::Pending => panic!("a free slot"),
        }
    };
    let op = permit.register();
    assert_eq!(op.registration(), Registration::Pending);
    let bind = eventually("the bind at the client", || client.recv().unwrap());
    assert_eq!(
        (bind.0.as_str(), bind.1.kind),
        ("call-1", MessageKind::Bind)
    );
    // The session's own frames travel on the same ends: a registry frame ahead of the answer is not the lane's, and
    // does not end the carrier.
    client_carrier
        .send_frame(
            Frame {
                tag: 1,
                body: b"a call".to_vec(),
            },
            crate::transport_host::carrier::Lane::Data,
            FrameKey::ordinary(Target::new(0)),
        )
        .unwrap();
    // The client's endpoint half answers `bound`, which the core's end routes because it opened the binding.
    let mut mux = Mux::endpoint(
        &bind.1.session_id,
        EndpointConfig {
            endpoint_id: "endpoint-1".into(),
            trust_owner: "trust-1".into(),
            role: EndpointRole::Driver,
            schemes: vec![Scheme::Ssh],
            policies: vec![AuthPolicy::SshAmbient],
            limits: default_limits(),
        },
        Config::default(),
    )
    .unwrap();
    mux.register("call-1", None).unwrap();
    mux.receive(&bind).unwrap();
    while let Some(reply) = mux.next_message() {
        client.send(reply).unwrap();
    }
    let registered = eventually("the registration", || match op.registration() {
        Registration::Pending => None,
        other => Some(other),
    });
    assert!(
        matches!(registered, Registration::Registered(_)),
        "{registered:?}"
    );
    let frame = eventually("the registry frame", || {
        core_carrier
            .recv_registry_timeout(std::time::Duration::from_millis(50))
            .unwrap()
    });
    assert_eq!(frame.body, b"a call");

    // A peer that overruns the binding's bounds faults that binding at the carrier, which drops what it queued: the
    // lane reports it, and the operation's later opens fail with it. (Five cancels of one stream pass the four slots.)
    let session = bind.1.session_id.clone();
    for _ in 0..5 {
        let mut cancel = gwz_transport::binding::offer(&session, EndpointRole::Driver);
        cancel.kind = MessageKind::Cancel;
        cancel.stream_id = 1;
        cancel.bind = None;
        cancel.cancel = Some(Cancel {
            reason: ErrorCode::Cancelled,
        });
        client.send(("call-1".into(), cancel)).unwrap();
    }
    eventually("the carrier's fault", || {
        core.faulted(&session).then_some(())
    });
    let spec = ExchangeSpec::git(
        Key::ssh("git", "example.invalid", 22),
        Identity::Ambient,
        GitService::UploadPackExchange,
        "/r",
    )
    .unwrap();
    let mut wait = Box::pin(op.open(spec).unwrap().wait());
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let failure = eventually("the open's failure", || match wait.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(Err(failure)) => Some(failure),
        std::task::Poll::Ready(Ok(_)) => panic!("a faulted binding opened"),
        std::task::Poll::Pending => None,
    });
    assert!(
        matches!(failure, OpenFailure::Fault(ErrorCode::Protocol)),
        "{failure:?}"
    );

    drop(wait);
    // Finishing the operation retires the binding at the core's end: a late frame for it is discarded there.
    let mut done = Box::pin(op.finish());
    eventually("the operation's end", || {
        done.as_mut().poll(&mut cx).is_ready().then_some(())
    });
    assert!(
        core_carrier.bind_fault(&session).is_none(),
        "the retired binding's record is gone"
    );
    let discarded = core_carrier.discarded_frames();
    let mut late = gwz_transport::binding::offer(&session, EndpointRole::Driver);
    late.kind = MessageKind::Window;
    late.stream_id = 1;
    late.bind = None;
    late.window = Some(gwz_transport::protocol::Window { max_offset: 1 });
    client.send(("call-1".into(), late)).unwrap();
    eventually("the late frame to be discarded", || {
        (core_carrier.discarded_frames() == discarded + 1).then_some(())
    });
    assert!(core.recv().unwrap().is_none(), "and no one gets it");
}
