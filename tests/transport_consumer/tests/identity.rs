use gwz_transport::{
    binding::{self, EndpointConfig},
    codec,
    protocol::{Data, EndpointRole, Envelope, MessageKind},
};
use gwz_transport_consumer_proof::{cbor, generated::GwzTransportDelivery};

#[test]
fn core_consumer_uses_owner_type_without_conversion_or_duplicate_schema() {
    let message = gwz_transport::binding::offer(
        "consumer-proof",
        gwz_transport::protocol::EndpointRole::Driver,
    );
    let delivered = GwzTransportDelivery {
        message: message.clone(),
    };
    let bytes = cbor::encode(&delivered.to_cbor());
    let decoded = GwzTransportDelivery::from_cbor(&cbor::try_decode(&bytes).unwrap()).unwrap();
    let owned: gwz_transport::protocol::Envelope = decoded.message;
    assert_eq!(owned, message);
}

#[test]
fn bind_bound_and_rejected_lifecycle_stays_typed() {
    let offer = binding::offer("consumer-session", EndpointRole::Driver);
    let config = EndpointConfig {
        endpoint_id: "test-endpoint".into(),
        role: EndpointRole::Driver,
        schemes: offer.bind.as_ref().unwrap().schemes.clone(),
        policies: offer.bind.as_ref().unwrap().policies.clone(),
        limits: binding::default_limits(),
        trust_owner: "test-host".into(),
    };
    let (bound, binding) = config.accept(&offer).expect("compatible bind must succeed");
    assert_eq!(
        binding::verify(&offer, &bound).unwrap().endpoint_id(),
        "test-endpoint"
    );
    assert_eq!(binding.session_id(), "consumer-session");

    let mut unsupported = offer;
    unsupported.bind.as_mut().unwrap().versions = vec![99];
    let failure = config
        .accept(&unsupported)
        .expect_err("unsupported versions must be rejected before effects");
    assert_eq!(
        failure.code,
        gwz_transport::protocol::ErrorCode::UnsupportedVersion
    );
    assert_eq!(failure.effect, gwz_transport::protocol::Effect::None);
}

#[test]
fn binary_payload_and_unknown_fields_round_trip_with_owner_admission() {
    let message = Envelope {
        version: 1,
        session_id: "consumer-session".into(),
        stream_id: 7,
        kind: MessageKind::Data,
        data: Some(Data {
            offset: 0,
            payload: vec![0, 255, 13, 10, 128],
        }),
        ..Default::default()
    };
    codec::admit(&message).expect("bounded binary message must be admitted");
    let mut envelope = message.to_cbor();
    if let cbor::Cbor::Map(entries) = &mut envelope {
        entries.push((99, cbor::Cbor::Bytes(vec![1, 2, 3])));
    } else {
        panic!("generated envelope must be a map");
    }
    let admitted = codec::decode(&cbor::encode(&envelope))
        .expect("owner admission must retain bounded unknown fields");
    assert_eq!(admitted.data.unwrap().payload, vec![0, 255, 13, 10, 128]);
    let wrapped = cbor::Cbor::Map(vec![(1, envelope), (99, cbor::Cbor::Text("future".into()))]);
    let bytes = cbor::encode(&wrapped);
    let decoded = GwzTransportDelivery::from_cbor(
        &cbor::try_decode(&bytes).expect("canonical wrapper must decode"),
    )
    .expect("unknown fields must be ignored by the generated consumer");
    assert_eq!(
        decoded.message.data.unwrap().payload,
        vec![0, 255, 13, 10, 128]
    );
}

#[test]
fn oversized_binary_payload_is_refused_before_message_handoff() {
    let message = Envelope {
        version: 1,
        session_id: "consumer-session".into(),
        stream_id: 7,
        kind: MessageKind::Data,
        data: Some(Data {
            offset: 0,
            payload: vec![0; codec::MAX_DATA + 1],
        }),
        ..Default::default()
    };
    assert_eq!(codec::admit(&message), Err(codec::Error::Bounds));
}
