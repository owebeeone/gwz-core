use super::*;
use crate::{RequestMeta, TransportOptions, TransportPlacement};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn config() -> SshEndpointConfig {
    SshEndpointConfig::fixture(std::path::PathBuf::from("/nonexistent-endpoint-home"), None)
}
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

#[allow(dead_code)]
mod guide_fixture {
    use crate as gwz_core;
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transport_backend/guide_example.rs"
    ));
}

fn wait<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        assert!(std::time::Instant::now() < deadline, "host future stranded");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn local_scope_clones_share_retirement_and_complete_without_a_network_exchange() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let meta = meta("local", TransportPlacement::Local);
    let scope = wait(runtime.request(meta.clone(), "op".into())).unwrap();
    let context = scope.context.clone();
    context.validate(&meta, "op").unwrap();
    let mut other = meta.clone();
    other.request_id = "other".into();
    assert!(context.validate(&other, "op").is_err());
    assert!(context.validate(&meta, "other").is_err());
    assert_eq!(wait(scope.finish()).pending_local_work, 0);
    assert!(context.validate(&meta, "op").is_err());
    assert_eq!(wait(runtime.shutdown()).pending_local_work, 0);
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
fn cancelling_one_bound_request_preserves_its_sibling_and_future_requests() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let first =
        wait(runtime.request(meta("first", TransportPlacement::Local), "op1".into())).unwrap();
    let second_meta = meta("second", TransportPlacement::Local);
    let second = wait(runtime.request(second_meta.clone(), "op2".into())).unwrap();
    first.cancel();
    assert_eq!(wait(first.finish()).pending_local_work, 0);
    second.context.validate(&second_meta, "op2").unwrap();
    assert_eq!(wait(second.finish()).pending_local_work, 0);
    let third =
        wait(runtime.request(meta("third", TransportPlacement::Local), "op3".into())).unwrap();
    assert_eq!(wait(third.finish()).pending_local_work, 0);
    wait(runtime.shutdown());
}

#[test]
fn finish_after_supervisor_shutdown_does_not_strand_request_owner() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let scope = wait(runtime.request(meta("r", TransportPlacement::Local), "op".into())).unwrap();
    wait(runtime.shutdown());
    std::thread::sleep(Duration::from_millis(20));
    let report = wait(scope.finish());
    assert_eq!(report.pending_local_work, 0);
    assert!(!report.peer_cleanup_confirmed);
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
