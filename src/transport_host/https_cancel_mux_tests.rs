use super::*;
use crate::git::endpoint::https_fixture as fixture;
use crate::git::endpoint::{
    https_destination::Destination as HttpsDestination, shared_reservation::Authority,
};
use gwz_transport::{binding, mux, pool};
use std::{
    sync::Arc,
    task::{Context, Waker},
    time::Duration,
};

pub(super) fn small_limits() -> Limits {
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
        policies: vec![AuthPolicy::Anonymous, AuthPolicy::WindowsDefault],
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

pub(super) fn open_for(destination: &HttpsDestination, limits: &Limits) -> Open {
    Open {
        endpoint_id: "endpoint".into(),
        operation_id: "operation".into(),
        destination: Destination {
            scheme: Scheme::Https,
            host: destination.host().into(),
            port: destination.port() as i64,
            path: destination.url.path().into(),
            ssh_username: None,
            https_username: None,
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
                HelperSlots::new(),
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

/// The actual worker succeeds before D. The host then delays either collection
/// or mux publication while holding a real connection reference, so physical
/// disposal cannot acknowledge its reservation prematurely.
async fn native_publication_case(delay_collection: bool, backpressure: bool, expire: bool) {
    use crate::git::endpoint::{https_fixture::Server, https_worker::native};
    let server = Server::start(Arc::new(|request| {
        Box::pin(async move {
            let mut response = crate::git::endpoint::https_fixture::response(
                if request.headers().contains_key("authorization") {
                    200
                } else {
                    401
                },
                GitService::UploadPackAdvertisement,
                bytes::Bytes::from_static(b"ok"),
            );
            if response.status() == 401 {
                response
                    .headers_mut()
                    .insert("www-authenticate", "NTLM".parse().unwrap());
            }
            response
        })
    }))
    .await;
    let destination = HttpsDestination::parse(&server.url).unwrap();
    let mut config = pool::Config::default();
    config.total = 1;
    config.per_host = 1;
    let authority = Authority::new(1, 1);
    let mut endpoint = HttpsEndpoint::new_native(
        HttpsEndpointConfig {
            tls: server.config(),
            auth: None,
        },
        config,
        3000,
        authority.clone(),
        "endpoint".into(),
        HelperSlots::new(),
        Some(native::publication_fixture()),
    )
    .unwrap();
    let (mut owner, mut initiator, limits) = mux_pair();
    let mut actions = Vec::new();
    for request in ["target", "sibling-1", "sibling-2", "sibling-3", "sibling-4"] {
        initiator
            .register(request, Some("operation".into()))
            .unwrap();
        owner.register(request, None).unwrap();
        let mut open = open_for(&destination, &limits);
        if request == "target" {
            open.policy = AuthPolicy::WindowsDefault;
            open.identity.mode = IdentityMode::Ambient;
            open.deadlines.connect_ms = 1000;
        }
        let id = initiator.open(request, open).unwrap();
        owner.receive(&initiator.next_message().unwrap()).unwrap();
        actions.push((request, id, owner.next_action().unwrap().1));
    }
    endpoint
        .accept("target".into(), actions[0].2.clone())
        .unwrap();
    let key = (String::from("target"), actions[0].1);
    let task = endpoint
        .entries
        .get_mut(&key)
        .unwrap()
        .preparing
        .take()
        .unwrap();
    let attempt = task.await.unwrap();
    let prepared = attempt
        .0
        .as_ref()
        .expect("real native preparation before D");
    assert_eq!(prepared.opened.facts.authenticated, Some(true));
    let (route, connection) = prepared.publication_resources_for_test();
    let until = attempt.2.budget.publication_deadline().unwrap();
    assert!(tokio::time::Instant::now() < until);
    endpoint.entries.get_mut(&key).unwrap().preparing =
        Some(endpoint.runtime.spawn(async move { attempt }));
    let mut cx = Context::from_waker(Waker::noop());
    if delay_collection {
        tokio::time::sleep_until(until).await;
    }
    let mut receipt = loop {
        endpoint.step(0, &mut cx).unwrap();
        if let Some(outbound) = endpoint.take_outbound(&mut cx) {
            break outbound.envelope;
        }
        tokio::task::yield_now().await;
    };
    if backpressure {
        assert_eq!(receipt.kind, MessageKind::Opened);
        for (request, id, _) in &actions[1..] {
            owner.send(request, &opened_for(*id, &limits)).unwrap();
        }
        endpoint.before_handoff("target", &mut receipt);
        assert_eq!(owner.send("target", &receipt), Err(mux::Error::WouldBlock));
    }
    if expire && !delay_collection {
        tokio::time::sleep_until(until).await;
    }
    endpoint.before_handoff("target", &mut receipt);
    if expire {
        assert_eq!(
            receipt.kind,
            MessageKind::OpenFailed,
            "late native Open must never publish"
        );
        let failure = receipt.open_failed.as_ref().unwrap();
        assert_eq!(failure.code, ErrorCode::Timeout);
        assert_eq!(failure.effect, Effect::None);
        assert_eq!(failure.facts.as_ref().unwrap().authenticated, Some(true));
        assert!(
            route.revoked_for_test(),
            "authenticated generation must be revoked"
        );
        assert_eq!(authority.counts(destination.host()), (1, 1));
        assert!(
            authority.try_reserve(destination.host()).is_none(),
            "undisposed connection remains charged"
        );
    } else {
        assert_eq!(receipt.kind, MessageKind::Opened);
    }
    if backpressure {
        for _ in 0..4 {
            let message = owner.next_message().unwrap();
            initiator.receive(&message).unwrap();
            initiator.next_action().unwrap();
        }
    }
    owner.send("target", &receipt).unwrap();
    endpoint.handed_off("target", &receipt);
    let terminal = owner.next_message().unwrap();
    assert_eq!(terminal.1.kind, receipt.kind);
    initiator.receive(&terminal).unwrap();
    assert_eq!(initiator.next_action().unwrap().1.kind, receipt.kind);
    assert!(initiator.next_action().is_none());
    assert!(owner.next_message().is_none());
    if expire {
        assert_eq!(
            owner.send("target", &receipt),
            Err(mux::Error::InvalidRequest)
        );
        assert_eq!(initiator.active_streams(), 4);
    }
    endpoint.shutdown();
    endpoint.step(0, &mut cx).unwrap();
    assert_eq!(authority.counts(destination.host()), (1, 1));
    drop(connection);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while endpoint.pending() != 0 {
        endpoint.step(0, &mut cx).unwrap();
        assert!(
            tokio::time::Instant::now() < deadline,
            "physical cleanup did not complete"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(authority.counts(destination.host()), (0, 0));
}
#[test]
fn native_publication_delayed_collection_expires_and_retains_physical_cleanup() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(native_publication_case(true, false, true));
}
#[test]
fn native_publication_backpressured_handoff_expires_and_retains_physical_cleanup() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(native_publication_case(false, true, true));
}
#[test]
fn native_publication_before_deadline_is_successful() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(native_publication_case(false, false, false));
}

#[test]
fn native_publication_deadline_equality_is_expired() {
    let deadline = tokio::time::Instant::now();
    assert!(publication_expired(Some(deadline), deadline));
    assert!(!publication_expired(
        Some(deadline),
        deadline - Duration::from_nanos(1)
    ));
    assert!(!publication_expired(None, deadline));
}

/// State P2-3 through the endpoint Session's own pump and mux Owner. The actual
/// worker prepares before D. The pump's check before the mux lock reads D−ε and
/// the decision under the lock reads `in_lock(D)`, as when the pump waits for
/// the lock across D. Each reading records whether the session's mux mutex was
/// held while it was taken. The test plays the driver with an initiator mux,
/// reads every message the endpoint publishes, and holds a real connection
/// reference so that physical disposal cannot settle early.
async fn session_publication_case(in_lock: fn(tokio::time::Instant) -> tokio::time::Instant) {
    use crate::git::endpoint::{
        https_fixture::{Server, response},
        https_worker::native,
        ssh_handoff::Handoff,
    };
    use crate::transport_host::{
        EndpointSettings,
        session::{self, Session},
    };
    use std::{future::Future, pin::pin, sync::Mutex};
    // The authenticated response waits until the test has taken the attempt,
    // so no pass can collect it first.
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let reached = Arc::new(tokio::sync::Notify::new());
    let (gate, signal) = (release.clone(), reached.clone());
    let server = Server::start(Arc::new(move |request| {
        let (gate, signal) = (gate.clone(), signal.clone());
        Box::pin(async move {
            let authorized = request.headers().contains_key("authorization");
            if authorized {
                signal.notify_one();
                drop(gate.acquire().await);
            }
            let mut reply = response(
                if authorized { 200 } else { 401 },
                GitService::UploadPackAdvertisement,
                bytes::Bytes::from_static(b"ok"),
            );
            if !authorized {
                reply
                    .headers_mut()
                    .insert("www-authenticate", "NTLM".parse().unwrap());
            }
            reply
        })
    }))
    .await;
    let destination = HttpsDestination::parse(&server.url).unwrap();
    let mut pool_config = pool::Config::default();
    pool_config.total = 1;
    pool_config.per_host = 1;
    let (endpoint, port) = Session::endpoint_with_https_native(
        EndpointSettings {
            ssh: None,
            pool: pool_config,
            io_timeout_ms: 3000,
        },
        Some((
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )),
        Handoff::default(),
        Some(native::publication_fixture()),
    )
    .unwrap();
    endpoint.register("target", None).unwrap();
    let mut initiator = mux::Mux::initiator(
        "publication",
        mux::Config {
            limits: session::limits(),
            ..Default::default()
        },
    )
    .unwrap();
    initiator
        .register("target", Some("operation".into()))
        .unwrap();
    initiator.begin("target").unwrap();
    port.deliver(initiator.next_message().unwrap())
        .await
        .unwrap();
    initiator
        .receive(&port.next_message().await.unwrap().unwrap())
        .unwrap();
    let binding = initiator.binding().unwrap();
    let mut open = open_for(&destination, binding.limits());
    open.endpoint_id = binding.endpoint_id().into();
    open.policy = AuthPolicy::WindowsDefault;
    open.identity.mode = IdentityMode::Ambient;
    let stream_id = initiator.open("target", open).unwrap();
    port.deliver(initiator.next_message().unwrap())
        .await
        .unwrap();

    reached.notified().await;
    let key = (String::from("target"), stream_id);
    let task = endpoint.with_https_for_test(|https| {
        https
            .entries
            .get_mut(&key)
            .unwrap()
            .preparing
            .take()
            .unwrap()
    });
    release.add_permits(1);
    let attempt = task.await.unwrap();
    let prepared = attempt
        .0
        .as_ref()
        .expect("real native preparation before D");
    assert_eq!(prepared.opened.facts.authenticated, Some(true));
    let (route, connection) = prepared.publication_resources_for_test();
    let until = attempt.2.budget.publication_deadline().unwrap();
    assert!(tokio::time::Instant::now() < until);
    let script = [until - Duration::from_nanos(1), in_lock(until)];
    // Another call on the session's mux owner finishes within 100 ms unless
    // the reading's thread holds the mux mutex.
    let owner = endpoint.mux_owner_for_test();
    let readings = Arc::new(Mutex::new(Vec::new()));
    let taken = readings.clone();
    endpoint.with_https_for_test(|https| {
        https.clock = Clock(Some(Arc::new(move || {
            let (other, (done, finished)) = (owner.clone(), std::sync::mpsc::channel());
            std::thread::spawn(move || {
                other.phase();
                let _ = done.send(());
            });
            let held = finished.recv_timeout(Duration::from_millis(100)).is_err();
            let mut taken = taken.lock().unwrap();
            taken.push(held);
            script[(taken.len() - 1).min(script.len() - 1)]
        })));
        let preparing = https.runtime.spawn(async move { attempt });
        https.entries.get_mut(&key).unwrap().preparing = Some(preparing);
    });
    let (request, published) = port.next_message().await.unwrap().unwrap();
    assert_eq!(
        (request.as_str(), published.stream_id),
        ("target", stream_id)
    );
    let authority = endpoint.authority_for_test();
    if in_lock(until) < until {
        assert_eq!(published.kind, MessageKind::Opened);
        assert!(!route.revoked_for_test());
    } else {
        assert_eq!(
            published.kind,
            MessageKind::OpenFailed,
            "late native Open must never publish"
        );
        let failure = published.open_failed.as_ref().unwrap();
        assert_eq!(failure.code, ErrorCode::Timeout);
        assert_eq!(failure.effect, Effect::None);
        assert_eq!(failure.facts.as_ref().unwrap().authenticated, Some(true));
        assert_eq!(
            *readings.lock().unwrap(),
            [false, true],
            "D−ε before the mux lock, then the decision under it"
        );
        assert!(
            route.revoked_for_test(),
            "authenticated generation must be revoked"
        );
        assert_eq!(authority.counts(destination.host()), (1, 1));
        assert!(
            authority.try_reserve(destination.host()).is_none(),
            "undisposed connection remains charged"
        );
        // That terminal is the stream's only one: its entry retires and the
        // pump runs on with nothing more to publish.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !endpoint.with_https_for_test(|https| https.entries.is_empty()) {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let passes = endpoint.pumps_for_test();
        while endpoint.pumps_for_test() < passes + 2 {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let mut cx = Context::from_waker(Waker::noop());
        assert!(pin!(port.next_message()).poll(&mut cx).is_pending());
    }
    initiator.receive(&(request, published.clone())).unwrap();
    assert_eq!(initiator.next_action().unwrap().1.kind, published.kind);
    assert!(initiator.next_action().is_none());
    endpoint.close();
    assert_eq!(authority.counts(destination.host()), (1, 1));
    drop(connection);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while authority.counts(destination.host()) != (0, 0) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "physical cleanup did not complete"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}
#[test]
fn native_publication_session_crossing_d_before_the_mux_lock_times_out() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(session_publication_case(|until| {
            until + Duration::from_millis(1)
        }));
}
#[test]
fn native_publication_session_exact_d_under_the_mux_lock_is_expired() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(session_publication_case(|until| until));
}
#[test]
fn native_publication_session_before_d_across_the_mux_lock_is_published() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(session_publication_case(|until| {
            until - Duration::from_nanos(1)
        }));
}
#[test]
fn https_only_capacity_replacement_waits_for_actual_physical_disposal() {
    use crate::git::endpoint::{
        https_fixture::{Server, input, response},
        https_worker::native,
    };
    use crate::transport_host::{EndpointSettings, TransportRuntime};
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|request| {
                Box::pin(async move {
                    let mut reply = response(
                        if request.headers().contains_key("authorization") {
                            200
                        } else {
                            401
                        },
                        GitService::UploadPackAdvertisement,
                        bytes::Bytes::from_static(b"ok"),
                    );
                    if reply.status() == 401 {
                        reply
                            .headers_mut()
                            .insert("www-authenticate", "NTLM".parse().unwrap());
                    }
                    reply
                })
            }))
            .await;
            let runtime = TransportRuntime::build_native(
                EndpointSettings {
                    ssh: None,
                    pool: pool::Config::default(),
                    io_timeout_ms: 3000,
                },
                Some((
                    HttpsEndpointConfig {
                        tls: server.config(),
                        auth: None,
                    },
                    HelperSlots::new(),
                )),
                Some(native::publication_fixture()),
            )
            .unwrap();
            let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
            let metadata = |id: &str, limit| crate::RequestMeta {
                request_id: id.into(),
                schema_version: "gwz.protocol/v0".into(),
                policy: Some(crate::OperationPolicy {
                    max_connections_per_host: Some(limit),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let first = runtime
                .request(metadata("physical-first", 1), "first".into())
                .await
                .unwrap();
            first.finish().await;
            let client = endpoint.https_client_for_test();
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::WindowsDefault;
            let mut budget = client.budget();
            let (result, _) = client
                .prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None)
                .await;
            let prepared = result.unwrap();
            let (_, connection) = prepared.publication_resources_for_test();
            prepared.revoke_native_route();
            client.finish_operation("operation");
            drop(prepared);
            let until = tokio::time::Instant::now() + Duration::from_secs(2);
            while endpoint.https_counts_for_test().unwrap().closing == 0 {
                assert!(tokio::time::Instant::now() < until);
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            let authority = endpoint.authority_for_test();
            assert_eq!(authority.counts("localhost"), (1, 1));
            assert!(authority.try_reserve("localhost").is_none());
            let error = runtime
                .request(metadata("physical-next", 2), "replacement".into())
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.code,
                crate::model::ErrorCode::TransportCapacityConflict
            );
            assert_eq!(endpoint.capacity_for_test().unwrap().per_host, 1);
            assert_eq!(authority.counts("localhost"), (1, 1));
            drop(connection);
            while endpoint.https_counts_for_test().unwrap().total() != 0 {
                assert!(tokio::time::Instant::now() < until);
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            assert_eq!(authority.counts("localhost"), (0, 0));
            let replacement = runtime
                .request(metadata("physical-next", 2), "replacement".into())
                .await
                .unwrap();
            assert_eq!(endpoint.capacity_for_test().unwrap().per_host, 2);
            replacement.finish().await;
            assert_eq!(runtime.shutdown().await.pending_local_work, 0);
        });
}
