use super::*;
use crate::git::endpoint::{
    https_destination::Destination as HttpsDestination, shared_reservation::Authority,
};
use crate::transport_host::https_tests::fixture;
use gwz_transport::{binding, mux, pool, protocol::*};
use std::{
    sync::Arc,
    task::{Context, Waker},
    time::Duration,
};

fn small_limits() -> Limits {
    let mut limits = binding::default_limits();
    limits.queued_frames = 4;
    limits.control_reserve_frames = 2;
    limits
}

fn mux_pair() -> (mux::Mux, mux::Mux, Limits) {
    let limits = small_limits();
    let config = mux::Config {
        limits: limits.clone(),
        max_streams: 16,
        ..Default::default()
    };
    let endpoint_config = binding::EndpointConfig {
        endpoint_id: "endpoint".into(),
        role: EndpointRole::Driver,
        schemes: vec![Scheme::Https],
        policies: vec![AuthPolicy::Anonymous],
        limits: limits.clone(),
        trust_owner: "endpoint".into(),
    };
    let mut endpoint = mux::Mux::endpoint("session", endpoint_config, config.clone()).unwrap();
    let mut initiator = mux::Mux::initiator("session", config).unwrap();
    initiator
        .register("bootstrap", Some("operation".into()))
        .unwrap();
    endpoint.register("bootstrap", None).unwrap();
    initiator.begin("bootstrap").unwrap();
    let offer = initiator.next_message().unwrap();
    endpoint.receive(&offer).unwrap();
    let bound = endpoint.next_message().unwrap();
    initiator.receive(&bound).unwrap();
    assert_eq!(initiator.phase(), mux::Phase::Ready);
    assert_eq!(endpoint.phase(), mux::Phase::Ready);
    (endpoint, initiator, limits)
}

fn open_for(destination: &HttpsDestination, limits: &Limits) -> Open {
    Open {
        endpoint_id: "endpoint".into(),
        operation_id: "operation".into(),
        destination: Destination {
            scheme: Scheme::Https,
            host: destination.host().into(),
            port: destination.port() as i64,
            path: destination.url.path().into(),
            ssh_username: None,
        },
        service: GitService::UploadPackAdvertisement,
        identity: Identity {
            mode: IdentityMode::CredentialsDisabled,
            ..Default::default()
        },
        policy: AuthPolicy::Anonymous,
        deadlines: Deadlines {
            allocation_ms: 30_000,
            connect_ms: 10_000,
            io_ms: 3_000,
            interaction_ms: 120_000,
            cleanup_ms: 5_000,
        },
        receive_limits: limits.clone(),
    }
}

fn opened_for(stream_id: i64, limits: &Limits) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Opened,
        opened: Some(Opened {
            connection_id: format!("connection-{stream_id}"),
            reused: false,
            endpoint_id: "endpoint".into(),
            trust_owner: "endpoint".into(),
            facts: Facts::default(),
            receive_limits: limits.clone(),
        }),
        ..Default::default()
    }
}

#[test]
fn cancelled_opened_waiting_for_mux_capacity_is_replaced_before_handoff() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = fixture::Server::start(Arc::new(|_| {
                Box::pin(async {
                    fixture::response(200, GitService::UploadPackAdvertisement, "ok")
                })
            }))
            .await;
            let destination = HttpsDestination::parse(&server.url).unwrap();
            let pool_config = pool::Config::default();
            let authority = Authority::new(pool_config.total, pool_config.per_host);
            let mut endpoint = HttpsEndpoint::new(
                HttpsEndpointConfig {
                    tls: server.config(),
                    auth: None,
                },
                pool_config,
                3_000,
                authority,
                "endpoint".into(),
            )
            .unwrap();
            let (mut mux_endpoint, mut initiator, limits) = mux_pair();
            let target = "target";
            let siblings = ["sibling-1", "sibling-2", "sibling-3", "sibling-4"];
            let mut actions = Vec::new();
            for request in [target, siblings[0], siblings[1], siblings[2], siblings[3]] {
                initiator
                    .register(request, Some("operation".into()))
                    .unwrap();
                mux_endpoint.register(request, None).unwrap();
                let stream_id = initiator
                    .open(request, open_for(&destination, &limits))
                    .unwrap();
                let open = initiator.next_message().unwrap();
                assert_eq!(open.1.stream_id, stream_id);
                mux_endpoint.receive(&open).unwrap();
                actions.push((request, stream_id, mux_endpoint.next_action().unwrap().1));
            }
            let target_action = actions
                .iter()
                .find(|(request, _, _)| *request == target)
                .unwrap()
                .2
                .clone();
            endpoint.accept(target.into(), target_action).unwrap();

            let mut cx = Context::from_waker(Waker::noop());
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            let pending_opened = loop {
                endpoint.step(0, &mut cx).unwrap();
                if let Some(outbound) = endpoint.take_outbound(&mut cx) {
                    break outbound;
                }
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(2)).await;
            };
            assert_eq!(pending_opened.envelope.kind, MessageKind::Opened);
            let target_stream = pending_opened.envelope.stream_id;

            for (_, stream_id, _) in actions.iter().filter(|(request, _, _)| *request != target) {
                let sibling = opened_for(*stream_id, &limits);
                mux_endpoint
                    .send(
                        actions.iter().find(|(_, id, _)| id == stream_id).unwrap().0,
                        &sibling,
                    )
                    .unwrap();
            }
            assert!(mux_endpoint.queued_bytes().0 > 0);
            assert_eq!(
                mux_endpoint.send(target, &pending_opened.envelope),
                Err(mux::Error::WouldBlock)
            );

            initiator.cancel(target).unwrap();
            let cancel = initiator
                .next_message()
                .expect("reserved cancellation bypasses bulk queue");
            assert_eq!(cancel.1.kind, MessageKind::Cancel);
            assert_eq!(cancel.1.stream_id, target_stream);
            mux_endpoint.receive(&cancel).unwrap();
            let (request, message) = mux_endpoint.next_action().unwrap();
            endpoint.accept(request, message).unwrap();
            let mut pending = pending_opened.envelope;
            endpoint.before_handoff(target, &mut pending);
            assert_eq!(pending.kind, MessageKind::OpenFailed);
            assert_eq!(
                pending.open_failed.as_ref().map(|failure| failure.code),
                Some(ErrorCode::Cancelled)
            );

            for _ in 0..siblings.len() {
                let (request, message) = mux_endpoint.next_message().unwrap();
                assert!(siblings.contains(&request.as_str()));
                assert_eq!(message.kind, MessageKind::Opened);
                initiator.receive(&(request, message)).unwrap();
                assert_eq!(initiator.next_action().unwrap().1.kind, MessageKind::Opened);
            }
            assert!(mux_endpoint.next_message().is_none());
            mux_endpoint.send(target, &pending).unwrap();
            let (request, terminal) = mux_endpoint.next_message().unwrap();
            assert_eq!(request, target);
            assert_eq!(terminal.kind, MessageKind::OpenFailed);
            assert_eq!(
                terminal.open_failed.as_ref().map(|failure| failure.code),
                Some(ErrorCode::Cancelled)
            );
            initiator.receive(&(request, terminal)).unwrap();
            assert_eq!(
                initiator.next_action().unwrap().1.kind,
                MessageKind::OpenFailed
            );
            assert!(initiator.next_action().is_none());
            assert_eq!(initiator.active_streams(), siblings.len());
            assert!(mux_endpoint.next_message().is_none());

            endpoint.shutdown();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while endpoint.pending() != 0 {
                endpoint.step(0, &mut cx).unwrap();
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
}
