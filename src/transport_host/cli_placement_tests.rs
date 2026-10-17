//! The Cli placement's lifecycle through the transport host: the client driver
//! a runtime installs, the endpoint a client builds, the capabilities a runtime
//! reports for them, and the requests the placement refuses. The Cli placement
//! is the 1.2.0 session host's transport entry (S7.1 (1.1.0) step C3a), so these
//! run in the session candidate only.
use super::cli_driver_tests::CliHarness;
use super::driver_tests::block_on;
use super::tests::{config, wait};
use super::*;
use crate::{RequestMeta, TransportCapabilitiesRequest, TransportOptions, TransportPlacement};
use std::{
    pin::pin,
    task::{Context, Poll, Waker},
};

fn meta(id: &str, placement: TransportPlacement) -> RequestMeta {
    RequestMeta {
        request_id: id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(placement),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn explicit_cli_requires_installed_endpoint_and_local_rejects_remote_path_base() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    let result =
        pin!(runtime.request(meta("r", TransportPlacement::Cli), "op".into())).poll(&mut cx);
    assert!(matches!(result, Poll::Ready(Err(_))));
    let mut local = meta("local", TransportPlacement::Local);
    local.transport.as_mut().unwrap().endpoint_path_base = Some("/remote".into());
    assert!(matches!(
        pin!(runtime.request(local, "op".into())).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
}

#[test]
fn dropped_bootstrap_owner_closes_installed_generation() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let port = runtime.install_cli().unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    {
        let mut request = pin!(runtime.request(meta("r", TransportPlacement::Cli), "op".into()));
        assert!(request.as_mut().poll(&mut cx).is_pending());
    }
    assert!(matches!(
        pin!(port.next_message()).poll(&mut cx),
        Poll::Ready(Ok(None))
    ));
    assert!(runtime.install_cli().is_err());
}

#[test]
fn unregistered_bind_cannot_create_endpoint_authority() {
    let (endpoint, port) = CliEndpoint::new(config()).unwrap();
    let mut bind =
        gwz_transport::binding::offer("session", gwz_transport::protocol::EndpointRole::Driver);
    bind.bind.as_mut().unwrap().versions = vec![2];
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        pin!(port.deliver(("unknown".into(), bind))).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert!(endpoint.register_request("later").is_err());
}

/// The session projection's identity cells on every platform (decision D30, 2026-10-17; D38 builds exact agent-key
/// selection in a separate lane): a selected key file is advertised (run 2026-10-15-parityrows, rows
/// home-unset-absolute-identity and home-unset-tilde-identity), and no exact agent identity is, because no code selects
/// one agent key yet. It agrees with the static `transport_capabilities::handle`.
#[test]
fn the_projection_advertises_a_key_file_identity_and_no_exact_agent_identity_on_every_platform() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let caps = runtime
        .capabilities(TransportCapabilitiesRequest {
            schema_version: "gwz.protocol/v0".into(),
        })
        .unwrap();
    assert!(caps.file_identity);
    assert!(!caps.exact_agent_identity);
    let statically =
        crate::protocol::transport_capabilities::handle(TransportCapabilitiesRequest {
            schema_version: "gwz.protocol/v0".into(),
        })
        .unwrap();
    assert_eq!(caps.file_identity, statically.file_identity);
    assert_eq!(caps.exact_agent_identity, statically.exact_agent_identity);
    wait(runtime.shutdown());
}

#[test]
fn last_port_clone_drop_invalidates_all_scopes_and_remove_allows_fresh_binding() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let port = runtime.install_cli().unwrap();
    let clone = port.clone();
    drop(port);
    assert!(
        runtime
            .capabilities(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into()
            })
            .unwrap()
            .placements
            .unwrap()
            .contains(&TransportPlacement::Cli)
    );
    drop(clone);
    assert!(
        !runtime
            .capabilities(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into()
            })
            .unwrap()
            .placements
            .unwrap()
            .contains(&TransportPlacement::Cli)
    );
    let cleanup = wait(runtime.remove_cli());
    assert!(!cleanup.peer_cleanup_confirmed);
    assert!(runtime.install_cli().is_ok());
    wait(runtime.shutdown());
}

#[test]
fn registration_dropped_before_bind_cannot_be_resurrected_by_late_message() {
    let (endpoint, port) = CliEndpoint::new(config()).unwrap();
    let owner = endpoint.register_request("cancelled").unwrap();
    drop(owner);
    let mut bind = gwz_transport::binding::offer(
        "late-session",
        gwz_transport::protocol::EndpointRole::Driver,
    );
    bind.bind.as_mut().unwrap().versions = vec![2];
    bind.bind.as_mut().unwrap().receive_limits = session::limits();
    let delivery = wait(port.deliver(("cancelled".into(), bind)));
    assert!(
        delivery.is_err(),
        "late Bind must not revive a retired request"
    );
    assert!(endpoint.register_request("cancelled").is_err());
    assert_eq!(wait(endpoint.shutdown()).pending_local_work, 0);
}

/// TR2.18: a URL's password goes only to an endpoint in the driver's own
/// process. The client placement's driver has none, so it refuses the open
/// before anything is queued, and its error holds no part of the password.
#[test]
fn a_driver_with_no_endpoint_in_process_refuses_a_url_password() {
    let (driver, _port) = session::Session::driver(3000, 3000, None).unwrap();
    let error = driver
        .open(
            "request",
            "operation",
            "ssh://git:sentinel-pw@example.invalid/repository.git",
            crate::git::endpoint::ssh_channel::GitService::UploadPack,
            Default::default(),
            true,
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
        )
        .err()
        .expect("the open is refused");
    let failure = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<session::SshOpenFailure>())
        .expect("an open failure");
    assert_eq!(
        failure.0.code,
        gwz_transport::protocol::ErrorCode::InvalidRequest
    );
    assert!(!format!("{error:?} {error}").contains("sentinel"));
}

#[test]
fn the_cli_placement_refuses_a_retry_budget_it_cannot_carry_to_its_endpoint() {
    // The Cli placement's endpoint is beyond the port, and the session
    // protocol carries no budget to it: it would retry three times while
    // this driver counted the request's budget (the State review's P3-1).
    let harness = CliHarness::new();
    let mut meta = harness.meta("cli-budget");
    meta.policy = Some(crate::OperationPolicy {
        max_retries: Some(0),
        ..Default::default()
    });
    let _budgeted = harness.endpoint.register_request(&meta.request_id).unwrap();
    let Err(error) = block_on(harness.runtime.request(meta, "fetch".into())) else {
        panic!("a Cli-placed request cannot carry --max-retries");
    };
    assert_eq!(error.code, crate::model::ErrorCode::UnsupportedOperation);
    assert!(error.message.contains("Cli placement"), "{}", error.message);
    // Without one it opens, and the driver's record keeps the default.
    let meta = harness.meta("cli-default");
    let _endpoint = harness.endpoint.register_request(&meta.request_id).unwrap();
    let request = block_on(harness.runtime.request(meta, "fetch".into())).unwrap();
    assert_eq!(
        request.context.session.max_retries("cli-default"),
        crate::git::endpoint::setup_retry::DEFAULT_MAX_RETRIES
    );
    block_on(request.finish());
    block_on(harness.runtime.shutdown());
}
