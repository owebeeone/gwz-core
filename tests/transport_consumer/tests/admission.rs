//! Consumer-side proof of the generated envelope handoff and admission order.
use gwz_transport::{
    binding::{self, EndpointConfig},
    codec,
    pool::{Identity as PoolIdentity, Key, Owner, Request as PoolRequest},
    protocol::*,
    stream::{Config as StreamConfig, Side as StreamSide},
};
use gwz_transport_consumer_proof::{cbor, generated::GwzTransportDelivery};

fn handoff(message: Envelope, encoded: bool) -> Envelope {
    let limits = binding::default_limits();
    handoff_with_limits(message, encoded, &limits)
        .unwrap_or_else(|error| panic!("fake endpoint receives admitted input ({error:?})"))
}

fn handoff_with_limits(
    message: Envelope,
    encoded: bool,
    limits: &Limits,
) -> Result<Envelope, codec::Error> {
    if encoded {
        return encoded_receiver_handoff(&message, limits);
    }
    codec::admit_limited(&message, limits)?;
    Ok(raw_generated_handoff(message, false))
}

fn encoded_receiver_handoff(
    message: &Envelope,
    receiver_limits: &Limits,
) -> Result<Envelope, codec::Error> {
    // The test sender may use larger limits than its receiver. The outer
    // generated wrapper is a trusted fixture; receiver admission is applied
    // to the inner bytes before the owner's allocating decoder runs.
    let inner = codec::encode(message)?;
    let message = codec::decode_limited(&inner, receiver_limits)?;
    Ok(raw_generated_handoff(message, true))
}

fn raw_generated_handoff(message: Envelope, encoded: bool) -> Envelope {
    let delivery = GwzTransportDelivery { message };
    if encoded {
        let bytes = cbor::encode(&delivery.to_cbor());
        let tree = cbor::try_decode(&bytes).expect("generated wrapper must decode");
        return GwzTransportDelivery::from_cbor(&tree)
            .expect("generated wrapper must preserve the owner envelope")
            .message;
    }
    delivery.message
}

fn dispatch_open(
    binding: &binding::Binding,
    message: Envelope,
    encoded: bool,
    effects: &mut usize,
) -> Result<(), Failure> {
    // These schema/ownership fixtures model an endpoint with network timing
    // explicitly disabled. The finite-policy matrix below uses the same host
    // boundary with captured finite values, before its sole effects increment.
    dispatch_host_open(
        binding,
        message,
        encoded,
        HostPolicy {
            connect_ms: 0,
            io_ms: 0,
            interaction_ms: 120_000,
        },
        0,
        effects,
    )
    .map(|_| ())
}

fn open_message(session_id: &str, endpoint_id: &str) -> Envelope {
    Envelope {
        version: 1,
        session_id: session_id.into(),
        stream_id: 1,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: endpoint_id.into(),
            operation_id: "operation-1".into(),
            destination: Destination {
                scheme: Scheme::Ssh,
                host: "example.test".into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
            },
            service: GitService::UploadPackExchange,
            identity: Identity::default(),
            policy: AuthPolicy::SshAmbient,
            deadlines: Deadlines {
                allocation_ms: 1,
                connect_ms: 1,
                io_ms: 1,
                interaction_ms: 1,
                cleanup_ms: 1,
            },
            receive_limits: binding::default_limits(),
        }),
        ..Default::default()
    }
}

fn envelope_inventory() -> Vec<Envelope> {
    let session_id = "inventory-session";
    let offer = binding::offer(session_id, EndpointRole::Driver);
    let endpoint = EndpointConfig {
        endpoint_id: "endpoint-1".into(),
        role: EndpointRole::Driver,
        schemes: offer.bind.as_ref().unwrap().schemes.clone(),
        policies: offer.bind.as_ref().unwrap().policies.clone(),
        limits: binding::default_limits(),
        trust_owner: "test-host".into(),
    };
    let (bound, _) = endpoint.accept(&offer).unwrap();
    vec![
        offer,
        bound,
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 0,
            kind: MessageKind::BindRejected,
            bind_rejected: Some(Failure {
                code: ErrorCode::UnsupportedVersion,
                effect: Effect::None,
                facts: None,
            }),
            ..Default::default()
        },
        open_message(session_id, "endpoint-1"),
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Opened,
            opened: Some(Opened {
                connection_id: "connection-1".into(),
                reused: false,
                endpoint_id: "endpoint-1".into(),
                trust_owner: "test-host".into(),
                facts: Facts::default(),
                receive_limits: binding::default_limits(),
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::OpenFailed,
            open_failed: Some(Failure {
                code: ErrorCode::Authentication,
                effect: Effect::None,
                facts: None,
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Data,
            data: Some(Data {
                offset: 0,
                payload: vec![0, 255, 128],
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Window,
            window: Some(Window { max_offset: 3 }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Flush,
            flush: Some(Barrier {
                barrier_id: 1,
                offset: 3,
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Flushed,
            flushed: Some(Barrier {
                barrier_id: 1,
                offset: 3,
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::EndWrite,
            end_write: Some(EndWrite { final_offset: 3 }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Close,
            close: Some(Close { final_offset: 3 }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Closed,
            closed: Some(Closed {
                disposition: Disposition::Reusable,
                unread_response_discarded: false,
                facts: Facts::default(),
                failure: None,
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Cancel,
            cancel: Some(Cancel {
                reason: ErrorCode::Cancelled,
            }),
            ..Default::default()
        },
        Envelope {
            version: 1,
            session_id: session_id.into(),
            stream_id: 1,
            kind: MessageKind::Failed,
            failed: Some(Failure {
                code: ErrorCode::Protocol,
                effect: Effect::Possible,
                facts: None,
            }),
            ..Default::default()
        },
    ]
}

#[test]
fn every_envelope_variant_survives_typed_and_encoded_consumer_handoffs() {
    for message in envelope_inventory() {
        let typed = handoff(message.clone(), false);
        let encoded = handoff(message, true);
        assert_eq!(typed, encoded);
    }
}

#[test]
fn bind_and_open_admission_precede_fake_endpoint_effects() {
    for encoded in [false, true] {
        let mut effects = 0;
        let mut binding_installs = 0;
        let offer = handoff(binding::offer("session-1", EndpointRole::Driver), encoded);
        let mut endpoint_limits = binding::default_limits();
        endpoint_limits.data_payload -= 1;
        let endpoint = EndpointConfig {
            endpoint_id: "endpoint-1".into(),
            role: EndpointRole::Driver,
            schemes: vec![Scheme::Ssh],
            policies: vec![AuthPolicy::SshAmbient],
            limits: endpoint_limits,
            trust_owner: "test-host".into(),
        };
        let (reply, binding) = endpoint
            .accept(&offer)
            .expect("valid bind must be accepted");
        binding_installs += 1;
        let reply = handoff(reply, encoded);
        assert_eq!(
            binding::verify(&offer, &reply).unwrap().endpoint_id(),
            "endpoint-1"
        );
        assert_eq!(binding_installs, 1);
        assert_eq!(effects, 0, "binding must not perform endpoint work");

        let mut bad_version = offer.clone();
        bad_version.bind.as_mut().unwrap().versions = vec![99];
        let failure = endpoint.accept(&bad_version).unwrap_err();
        assert_eq!(failure.effect, Effect::None);
        let rejected = handoff(
            Envelope {
                version: 1,
                session_id: "session-1".into(),
                stream_id: 0,
                kind: MessageKind::BindRejected,
                bind_rejected: Some(failure),
                ..Default::default()
            },
            encoded,
        );
        assert_eq!(rejected.kind, MessageKind::BindRejected);
        assert_eq!(effects, 0);

        let mut open = open_message("session-1", "endpoint-1");
        open.open.as_mut().unwrap().receive_limits = binding.limits().clone();
        dispatch_open(&binding, open.clone(), encoded, &mut effects)
            .expect("valid open must be admitted");
        assert_eq!(effects, 1);

        for (connect_ms, io_ms) in [(0, 0), (i32::MAX as i64, i32::MAX as i64)] {
            let mut timed_open = open.clone();
            timed_open.open.as_mut().unwrap().deadlines.connect_ms = connect_ms;
            timed_open.open.as_mut().unwrap().deadlines.io_ms = io_ms;
            dispatch_open(&binding, timed_open, encoded, &mut effects)
                .expect("network deadline policy must be admitted");
        }
        assert_eq!(effects, 3);

        let mut bad_open = open.clone();
        bad_open.session_id = "stale-session".into();
        let failure = dispatch_open(&binding, bad_open, encoded, &mut effects).unwrap_err();
        assert_eq!(failure.effect, Effect::None);
        assert_eq!(failure.code, ErrorCode::Unavailable);

        let mut bad_endpoint = open.clone();
        bad_endpoint.open.as_mut().unwrap().endpoint_id = "other-endpoint".into();
        let failure = dispatch_open(&binding, bad_endpoint, encoded, &mut effects).unwrap_err();
        assert_eq!(failure.code, ErrorCode::Unavailable);

        let mut bad_scheme = open.clone();
        bad_scheme.open.as_mut().unwrap().destination.scheme = Scheme::Https;
        bad_scheme.open.as_mut().unwrap().policy = AuthPolicy::Anonymous;
        bad_scheme.open.as_mut().unwrap().destination.ssh_username = None;
        bad_scheme.open.as_mut().unwrap().identity.mode = IdentityMode::CredentialsDisabled;
        let failure = dispatch_open(&binding, bad_scheme, encoded, &mut effects).unwrap_err();
        assert_eq!(failure.code, ErrorCode::UnsupportedOperation);

        let mut bad_policy = open.clone();
        bad_policy.open.as_mut().unwrap().policy = AuthPolicy::SshExplicit;
        bad_policy.open.as_mut().unwrap().identity.mode = IdentityMode::ExplicitKey;
        bad_policy.open.as_mut().unwrap().identity.key_path = Some("id_ed25519".into());
        let failure = dispatch_open(&binding, bad_policy, encoded, &mut effects).unwrap_err();
        assert_eq!(failure.code, ErrorCode::UnsupportedOperation);

        let mut raised_limits = open;
        raised_limits
            .open
            .as_mut()
            .unwrap()
            .receive_limits
            .data_payload += 1;
        let failure = dispatch_open(&binding, raised_limits, encoded, &mut effects).unwrap_err();
        assert_eq!(failure.effect, Effect::None);
        assert_eq!(failure.code, ErrorCode::UnsupportedOperation);
        assert_eq!(effects, 3);

        let failed = handoff(
            Envelope {
                version: 1,
                session_id: "session-1".into(),
                stream_id: 1,
                kind: MessageKind::OpenFailed,
                open_failed: Some(Failure {
                    code: failure.code,
                    effect: failure.effect,
                    facts: None,
                }),
                ..Default::default()
            },
            encoded,
        );
        assert_eq!(failed.kind, MessageKind::OpenFailed);
    }
}

#[test]
fn negotiated_open_metadata_is_checked_before_effects_in_both_handoffs() {
    for encoded in [false, true] {
        let offer = binding::offer("metadata-session", EndpointRole::Driver);
        let mut endpoint_limits = binding::default_limits();
        endpoint_limits.metadata_bytes = 256;
        let endpoint = EndpointConfig {
            endpoint_id: "endpoint-1".into(),
            role: EndpointRole::Driver,
            schemes: vec![Scheme::Ssh],
            policies: vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit],
            limits: endpoint_limits,
            trust_owner: "test-host".into(),
        };
        let (reply, binding) = endpoint.accept(&offer).unwrap();
        assert_eq!(handoff(reply, encoded).kind, MessageKind::Bound);
        let mut effects = 0;
        let base = open_message("metadata-session", "endpoint-1");

        for field in ["operation", "destination", "identity"] {
            let mut accepted = base.clone();
            accepted.open.as_mut().unwrap().receive_limits = binding.limits().clone();
            apply_metadata(&mut accepted, field, 256);
            dispatch_open(&binding, accepted, encoded, &mut effects)
                .expect("256-byte negotiated metadata must be accepted");

            let mut rejected = base.clone();
            rejected.open.as_mut().unwrap().receive_limits = binding.limits().clone();
            apply_metadata(&mut rejected, field, 257);
            let before_rejection = effects;
            let failure = dispatch_open(&binding, rejected, encoded, &mut effects).unwrap_err();
            assert_eq!(failure.code, ErrorCode::InvalidRequest);
            assert_eq!(failure.effect, Effect::None);
            assert_eq!(effects, before_rejection);
        }
        assert_eq!(effects, 3);
    }
}

fn apply_metadata(message: &mut Envelope, field: &str, length: usize) {
    let open = message.open.as_mut().unwrap();
    match field {
        "operation" => {
            open.operation_id = "o".repeat(length);
        }
        "destination" => {
            open.destination.path = "/".to_owned() + &"p".repeat(length.saturating_sub(1));
        }
        "identity" => {
            open.policy = AuthPolicy::SshExplicit;
            open.identity.mode = IdentityMode::ExplicitKey;
            open.identity.key_path = Some("k".repeat(length));
        }
        _ => panic!("unknown metadata field"),
    }
}

#[derive(Debug)]
struct CapturedHostInputs {
    request: PoolRequest,
    stream: StreamConfig,
    helper_remaining_ms: u64,
}

#[derive(Clone, Copy)]
struct HostPolicy {
    connect_ms: u64,
    io_ms: u64,
    interaction_ms: u64,
}

fn dispatch_host_open(
    binding: &binding::Binding,
    message: Envelope,
    encoded: bool,
    policy: HostPolicy,
    helper_spent_ms: u64,
    effects: &mut usize,
) -> Result<CapturedHostInputs, Failure> {
    let message = handoff_with_limits(message, encoded, binding.limits()).map_err(|_| Failure {
        code: ErrorCode::InvalidRequest,
        effect: Effect::None,
        facts: None,
    })?;
    binding.check_open(&message)?;
    let inputs = resolve_host_inputs(
        &message,
        policy.connect_ms,
        policy.io_ms,
        policy.interaction_ms,
        helper_spent_ms,
    )?;
    *effects += 1;
    Ok(inputs)
}

fn resolve_host_inputs(
    message: &Envelope,
    endpoint_connect_ms: u64,
    endpoint_io_ms: u64,
    endpoint_interaction_ms: u64,
    helper_spent_ms: u64,
) -> Result<CapturedHostInputs, Failure> {
    let open = message.open.as_ref().unwrap();
    let connect_ms = resolve_network_timeout(open.deadlines.connect_ms, endpoint_connect_ms)?;
    let io_ms = resolve_network_timeout(open.deadlines.io_ms, endpoint_io_ms)?;
    let helper_total_ms = u64::try_from(open.deadlines.interaction_ms)
        .unwrap()
        .min(endpoint_interaction_ms);
    let helper_remaining_ms = helper_total_ms
        .checked_sub(helper_spent_ms)
        .ok_or(Failure {
            code: ErrorCode::InvalidRequest,
            effect: Effect::None,
            facts: None,
        })?;
    let identity = match (open.destination.scheme, open.identity.mode) {
        (Scheme::Ssh, IdentityMode::Ambient) => PoolIdentity::Ambient,
        (Scheme::Ssh, IdentityMode::ExplicitKey) => {
            // A fixed proof supplied by this fake identity resolver. A real
            // endpoint must resolve the key; a path is never a reuse proof.
            PoolIdentity::Explicit("fake-resolved-key-proof".into())
        }
        (Scheme::Https, _) => PoolIdentity::Https,

        _ => {
            return Err(Failure {
                code: ErrorCode::UnsupportedOperation,
                effect: Effect::None,
                facts: None,
            });
        }
    };
    let key = match open.destination.scheme {
        Scheme::Ssh => Key::ssh(
            open.destination.ssh_username.as_deref().unwrap(),
            open.destination.host.clone(),
            open.destination.port as u16,
        ),
        Scheme::Https => Key::https(open.destination.host.clone(), open.destination.port as u16),
    };
    let request = PoolRequest::new(
        key,
        identity,
        Owner::new(message.session_id.clone(), open.operation_id.clone()),
    );
    let mut request = request;
    request.allocation_timeout_ms = Some(open.deadlines.allocation_ms as u64);
    request.connect_timeout_ms = Some(connect_ms);
    request.interaction_timeout_ms = Some(helper_total_ms);
    let mut stream = StreamConfig::new(
        message.session_id.clone(),
        message.stream_id,
        StreamSide::Endpoint,
    );
    stream.io_timeout_ms = io_ms;
    stream.interaction_budget_ms = helper_remaining_ms;
    Ok(CapturedHostInputs {
        request,
        stream,
        helper_remaining_ms,
    })
}

fn resolve_network_timeout(requested_ms: i64, endpoint_ms: u64) -> Result<u64, Failure> {
    let requested_ms = u64::try_from(requested_ms).map_err(|_| Failure {
        code: ErrorCode::InvalidRequest,
        effect: Effect::None,
        facts: None,
    })?;
    if requested_ms > i32::MAX as u64 {
        return Err(Failure {
            code: ErrorCode::InvalidRequest,
            effect: Effect::None,
            facts: None,
        });
    }
    if endpoint_ms != 0 && (requested_ms == 0 || requested_ms > endpoint_ms) {
        return Err(Failure {
            code: ErrorCode::UnsupportedOperation,
            effect: Effect::None,
            facts: None,
        });
    }
    Ok(requested_ms)
}

#[test]
fn fake_endpoint_composes_network_policies_before_effects() {
    for encoded in [false, true] {
        let offer = binding::offer("policy-session", EndpointRole::Driver);
        let endpoint = EndpointConfig {
            endpoint_id: "endpoint-1".into(),
            role: EndpointRole::Driver,
            schemes: vec![Scheme::Ssh],
            policies: vec![AuthPolicy::SshAmbient],
            limits: binding::default_limits(),
            trust_owner: "test-host".into(),
        };
        let (reply, binding) = endpoint.accept(&offer).unwrap();
        assert_eq!(handoff(reply, encoded).kind, MessageKind::Bound);
        let mut effects = 0;
        let mut candidate = open_message("policy-session", "endpoint-1");
        candidate.open.as_mut().unwrap().receive_limits = binding.limits().clone();
        candidate.open.as_mut().unwrap().deadlines.interaction_ms = 100;

        let finite_policy = HostPolicy {
            connect_ms: 5_000,
            io_ms: 6_000,
            interaction_ms: 120_000,
        };
        for (connect_ms, io_ms) in [(0, 3_000), (5_001, 3_000), (2_500, 0), (2_500, 6_001)] {
            let mut rejected = candidate.clone();
            rejected.open.as_mut().unwrap().deadlines.connect_ms = connect_ms;
            rejected.open.as_mut().unwrap().deadlines.io_ms = io_ms;
            let failure =
                dispatch_host_open(&binding, rejected, encoded, finite_policy, 0, &mut effects)
                    .unwrap_err();
            assert_eq!(failure.effect, Effect::None);
            assert_eq!(effects, 0);
        }

        for (connect_ms, io_ms) in [(2_500, 3_000), (5_000, 6_000)] {
            let mut accepted = candidate.clone();
            accepted.open.as_mut().unwrap().deadlines.connect_ms = connect_ms;
            accepted.open.as_mut().unwrap().deadlines.io_ms = io_ms;
            let inputs =
                dispatch_host_open(&binding, accepted, encoded, finite_policy, 30, &mut effects)
                    .unwrap();
            assert_eq!(inputs.request.connect_timeout_ms, Some(connect_ms as u64));
            assert_eq!(inputs.stream.io_timeout_ms, io_ms as u64);
            assert_eq!(inputs.helper_remaining_ms, 70);
            assert_eq!(inputs.stream.interaction_budget_ms, 70);
        }
        assert_eq!(effects, 2);

        let mut capped_helper = candidate.clone();
        capped_helper
            .open
            .as_mut()
            .unwrap()
            .deadlines
            .interaction_ms = 120_001;
        capped_helper.open.as_mut().unwrap().deadlines.connect_ms = 2_500;
        capped_helper.open.as_mut().unwrap().deadlines.io_ms = 3_000;
        let inputs = dispatch_host_open(
            &binding,
            capped_helper,
            encoded,
            finite_policy,
            30,
            &mut effects,
        )
        .unwrap();
        assert_eq!(inputs.request.interaction_timeout_ms, Some(120_000));
        assert_eq!(inputs.helper_remaining_ms, 119_970);
        assert_eq!(effects, 3);

        let mut exhausted = candidate.clone();
        exhausted.open.as_mut().unwrap().deadlines.connect_ms = 2_500;
        exhausted.open.as_mut().unwrap().deadlines.io_ms = 3_000;
        let inputs = dispatch_host_open(
            &binding,
            exhausted,
            encoded,
            finite_policy,
            100,
            &mut effects,
        )
        .unwrap();
        assert_eq!(inputs.helper_remaining_ms, 0);
        assert_eq!(inputs.stream.interaction_budget_ms, 0);
        assert_eq!(effects, 4);
        let mut over_budget = candidate.clone();
        over_budget.open.as_mut().unwrap().deadlines.connect_ms = 2_500;
        over_budget.open.as_mut().unwrap().deadlines.io_ms = 3_000;
        assert!(
            dispatch_host_open(
                &binding,
                over_budget,
                encoded,
                finite_policy,
                101,
                &mut effects,
            )
            .is_err()
        );
        assert_eq!(effects, 4);

        let disabled_policy = HostPolicy {
            connect_ms: 0,
            io_ms: 0,
            interaction_ms: 120_000,
        };
        for (connect_ms, io_ms) in [(0, 0), (i32::MAX as i64, i32::MAX as i64)] {
            let mut accepted = candidate.clone();
            accepted.open.as_mut().unwrap().deadlines.connect_ms = connect_ms;
            accepted.open.as_mut().unwrap().deadlines.io_ms = io_ms;
            let inputs = dispatch_host_open(
                &binding,
                accepted,
                encoded,
                disabled_policy,
                30,
                &mut effects,
            )
            .unwrap();
            assert_eq!(inputs.request.connect_timeout_ms, Some(connect_ms as u64));
            assert_eq!(inputs.stream.io_timeout_ms, io_ms as u64);
        }
        assert_eq!(effects, 6);
    }
}
