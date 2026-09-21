use gwz_transport::protocol::{AuthPolicy, Limits, Scheme};
use gwz_transport_consumer_proof::{
    candidate_admission::{AdmissionError, ReceiverAdmission, ReceiverGeneration},
    candidate_generated as new, cbor, retained_old_generated as old,
};

fn limits() -> Limits {
    Limits {
        encoded_frame: 4096,
        data_payload: 1024,
        metadata_bytes: 256,
        nesting: 10,
        collection_entries: 128,
        decode_allocation: 65536,
        queued_bytes: 131072,
        queued_frames: 8,
        receive_window: 65536,
        control_reserve_bytes: 4096,
        control_reserve_frames: 2,
    }
}

fn candidate_capabilities() -> new::TransportCapabilitiesResponse {
    new::TransportCapabilitiesResponse {
        file_identity: true,
        exact_agent_identity: true,
        message_versions: Some(vec![2]),
        placements: Some(vec![
            new::TransportPlacement::Local,
            new::TransportPlacement::Cli,
        ]),
        schemes: Some(vec![Scheme::Ssh]),
        auth_policies: Some(vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit]),
        message_limits: Some(limits()),
    }
}

fn new_options(placement: Option<new::TransportPlacement>) -> new::TransportOptions {
    new::TransportOptions {
        placement,
        endpoint_path_base: Some("/client/workspace".into()),
        ..Default::default()
    }
}

#[test]
fn candidate_writer_is_readable_by_retained_old_rust_decoder() {
    let wire = cbor::encode(&new_options(Some(new::TransportPlacement::Cli)).to_cbor());
    let tree = cbor::try_decode(&wire).expect("candidate wire must decode");
    let retained =
        old::TransportOptions::from_cbor(&tree).expect("old reader accepts unknown tags");
    assert_eq!(retained.default_identity, None);
    assert_eq!(retained.remote_identities, Vec::new());
    assert_eq!(retained.url_scheme, None);
}

#[test]
fn retained_old_rust_writer_is_readable_by_new_candidate_decoder() {
    let retained = old::TransportOptions::default();
    let decoded = new::TransportOptions::from_cbor(&retained.to_cbor())
        .expect("missing new fields are accepted by candidate reader");
    assert_eq!(decoded.placement, None);
    assert_eq!(decoded.endpoint_path_base, None);
}

#[test]
fn candidate_null_and_absent_are_equivalent_but_present_malformed_is_rejected() {
    let absent = new::TransportOptions::from_cbor(&cbor::Cbor::Map(vec![
        (1, cbor::Cbor::Null),
        (2, cbor::Cbor::Array(Vec::new())),
        (3, cbor::Cbor::Null),
    ]))
    .expect("new fields may be absent");
    let null = new::TransportOptions::from_cbor(
        &new::TransportOptions {
            placement: None,
            endpoint_path_base: None,
            ..Default::default()
        }
        .to_cbor(),
    )
    .expect("new fields may be null");
    assert_eq!(absent.placement, null.placement);
    assert_eq!(absent.endpoint_path_base, null.endpoint_path_base);

    let malformed = cbor::Cbor::Map(vec![
        (1, cbor::Cbor::Null),
        (2, cbor::Cbor::Array(Vec::new())),
        (3, cbor::Cbor::Null),
        (4, cbor::Cbor::Text("cli".into())),
    ]);
    assert!(new::TransportOptions::from_cbor(&malformed).is_err());
}

#[test]
fn external_owner_envelope_is_used_without_a_duplicate_definition() {
    let envelope = gwz_transport::protocol::Envelope {
        version: 1,
        session_id: "candidate".into(),
        ..Default::default()
    };
    let meta = new::RequestMeta {
        request_id: "request".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport_message: Some(envelope.clone()),
        ..Default::default()
    };
    assert_eq!(meta.transport_message, Some(envelope));
}

#[test]
fn capability_gate_requires_complete_candidate_and_stale_generation_is_rejected() {
    let generation = ReceiverGeneration {
        id: 7,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let admission = ReceiverAdmission::establish(
        generation.clone(),
        &candidate_capabilities(),
        AuthPolicy::SshAmbient,
    )
    .expect("candidate capabilities should admit cli placement");
    let permit = admission
        .admit_explicit_cli(&generation)
        .expect("current receiver should be admitted");

    let stale = ReceiverGeneration {
        id: 6,
        ..generation.clone()
    };
    assert!(matches!(
        admission.admit_explicit_cli(&stale),
        Err(AdmissionError::StaleGeneration)
    ));
    let mut sends = 0;
    assert_eq!(
        permit.dispatch(|pinned| {
            assert_eq!(pinned.id, generation.id);
            sends += 1;
        }),
        Ok(()),
        "a current permit sends while its receiver remains installed"
    );
    assert_eq!(sends, 1);
    assert_eq!(
        ReceiverAdmission::placement_or_local(None),
        new::TransportPlacement::Local
    );
}

#[test]
fn replacement_barrier_invalidates_old_permits_before_any_send_or_effect() {
    let generation = ReceiverGeneration {
        id: 7,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let admission = ReceiverAdmission::establish(
        generation.clone(),
        &candidate_capabilities(),
        AuthPolicy::SshAmbient,
    )
    .unwrap();
    let old_permit = admission.admit_explicit_cli(&generation).unwrap();
    let replacement = ReceiverGeneration {
        id: 8,
        receiver_id: "core-2".into(),
        backend_family: "ssh".into(),
    };
    admission
        .replace_receiver(
            replacement.clone(),
            &candidate_capabilities(),
            AuthPolicy::SshAmbient,
        )
        .unwrap();
    let mut sends = 0;
    assert!(matches!(
        old_permit.dispatch(|_| sends += 1),
        Err(AdmissionError::StaleGeneration)
    ));
    assert_eq!(sends, 0, "replacement must prevent stale dispatch effects");
    let new_permit = admission.admit_explicit_cli(&replacement).unwrap();
    assert_eq!(
        new_permit.dispatch(|pinned| {
            assert_eq!(pinned.id, replacement.id);
            sends += 1;
        }),
        Ok(())
    );
    assert_eq!(sends, 1);
}

#[test]
fn failed_old_core_replacement_closes_owner_before_old_permit_can_send() {
    let generation = ReceiverGeneration {
        id: 7,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let admission = ReceiverAdmission::establish(
        generation.clone(),
        &candidate_capabilities(),
        AuthPolicy::SshAmbient,
    )
    .unwrap();
    let old_permit = admission.admit_explicit_cli(&generation).unwrap();
    let old_core = ReceiverGeneration {
        id: 8,
        receiver_id: "core-old".into(),
        backend_family: "ssh".into(),
    };
    let mut unsupported = candidate_capabilities();
    unsupported.message_versions = None;
    assert!(matches!(
        admission.replace_receiver(old_core, &unsupported, AuthPolicy::SshAmbient),
        Err(AdmissionError::UnsupportedCapabilities)
    ));
    let mut sends = 0;
    assert!(matches!(
        old_permit.dispatch(|_| sends += 1),
        Err(AdmissionError::ReceiverClosed)
    ));
    assert_eq!(sends, 0);
}

#[test]
fn cloned_admission_observer_cannot_evade_invalidation_and_limits_are_usable() {
    let generation = ReceiverGeneration {
        id: 1,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let admission = ReceiverAdmission::establish(
        generation.clone(),
        &candidate_capabilities(),
        AuthPolicy::SshAmbient,
    )
    .unwrap();
    assert_eq!(admission.limits().unwrap(), limits());
    let observer = admission.clone();
    let permit = admission.admit_explicit_cli(&generation).unwrap();
    observer.invalidate();
    let mut sends = 0;
    assert_eq!(
        permit.dispatch(|_| sends += 1),
        Err(AdmissionError::ReceiverClosed)
    );
    assert_eq!(sends, 0);
}

#[test]
fn capability_gate_refuses_old_or_incomplete_response_before_dispatch() {
    let generation = ReceiverGeneration {
        id: 1,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let mut old = candidate_capabilities();
    old.message_versions = None;
    assert!(matches!(
        ReceiverAdmission::establish(generation.clone(), &old, AuthPolicy::SshAmbient),
        Err(AdmissionError::UnsupportedCapabilities)
    ));
    let mut wrong_route = candidate_capabilities();
    wrong_route.schemes = Some(vec![Scheme::Https]);
    assert!(matches!(
        ReceiverAdmission::establish(generation, &wrong_route, AuthPolicy::SshAmbient),
        Err(AdmissionError::UnsupportedCapabilities)
    ));
}

#[test]
fn capability_gate_rejects_unsafe_limits_non_ssh_policy_and_empty_receiver() {
    let generation = ReceiverGeneration {
        id: 1,
        receiver_id: "core-1".into(),
        backend_family: "ssh".into(),
    };
    let mut zero_limits = candidate_capabilities();
    zero_limits.message_limits.as_mut().unwrap().queued_frames = 0;
    assert!(matches!(
        ReceiverAdmission::establish(generation.clone(), &zero_limits, AuthPolicy::SshAmbient),
        Err(AdmissionError::UnsupportedCapabilities)
    ));
    assert!(matches!(
        ReceiverAdmission::establish(
            generation.clone(),
            &candidate_capabilities(),
            AuthPolicy::Gh
        ),
        Err(AdmissionError::UnsupportedCapabilities)
    ));
    let empty = ReceiverGeneration {
        receiver_id: String::new(),
        ..generation
    };
    assert!(matches!(
        ReceiverAdmission::establish(empty, &candidate_capabilities(), AuthPolicy::SshAmbient),
        Err(AdmissionError::InvalidReceiver)
    ));
}
