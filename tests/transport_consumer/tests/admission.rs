//! Consumer-side proof of the generated envelope handoff and admission order.
use gwz_transport::{
    binding::{self, EndpointConfig},
    codec,
    protocol::*,
};
use gwz_transport_consumer_proof::{cbor, generated::GwzTransportDelivery};

fn handoff(message: Envelope, encoded: bool) -> Envelope {
    let limits = binding::default_limits();
    codec::admit_limited(&message, &limits).unwrap_or_else(|error| {
        panic!("fake endpoint receives admitted input ({error:?}): {message:?}")
    });
    generated_handoff(message, encoded, &limits)
}

fn generated_handoff(message: Envelope, encoded: bool, limits: &Limits) -> Envelope {
    let decoded = raw_generated_handoff(message, encoded);
    if !encoded {
        return decoded;
    }
    let inner = codec::encode_limited(&decoded, limits)
        .expect("owner codec must retain bounded encoded input");
    codec::decode_limited(&inner, limits).expect("owner codec must decode bounded encoded input")
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
    let message = handoff(message, encoded);
    let result = binding.check_open(&message);
    if result.is_ok() {
        *effects += 1;
    }
    result
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
                }),
                ..Default::default()
            },
            encoded,
        );
        assert_eq!(failed.kind, MessageKind::OpenFailed);
    }
}
