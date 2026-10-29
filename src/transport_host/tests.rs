use super::*;
use crate::RequestMeta;
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

pub(super) fn config() -> SshEndpointConfig {
    SshEndpointConfig::fixture(std::env::temp_dir().join("nonexistent-endpoint-home"), None)
}
/// The endpoint session's bootstrap refusals (the in-process Local endpoint is the same `Session::endpoint`): a Bind
/// for a request nobody registered creates no authority, and registering after that is refused too. The Cli twin is
/// `cli_placement_tests`.
#[test]
fn unregistered_bind_cannot_create_endpoint_authority() {
    let (endpoint, port) = session::Session::endpoint(config()).unwrap();
    let mut bind =
        gwz_transport::binding::offer("session", gwz_transport::protocol::EndpointRole::Driver);
    bind.bind.as_mut().unwrap().versions = vec![2];
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        pin!(port.deliver(("unknown".into(), bind))).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert!(ClientRequest::new(endpoint.clone(), "later").is_err());
}
/// A registration retired before the driver's Bind cannot be revived by that Bind arriving late, which the Local
/// route races too: `open_request` admits the client before the driver binds. The Cli twin is `cli_placement_tests`.
#[test]
fn registration_dropped_before_bind_cannot_be_resurrected_by_late_message() {
    let (endpoint, port) = session::Session::endpoint(config()).unwrap();
    let owner = ClientRequest::new(endpoint.clone(), "cancelled").unwrap();
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
    assert!(ClientRequest::new(endpoint.clone(), "cancelled").is_err());
    endpoint.close();
    assert_eq!(wait(endpoint.cleanup()).pending_local_work, 0);
}
#[test]
fn operation_policy_carries_max_retries_and_an_older_writer_leaves_it_absent() {
    use crate::cbor::Cbor;
    let policy = crate::OperationPolicy {
        max_retries: Some(0),
        ..Default::default()
    };
    let decoded = crate::OperationPolicy::from_cbor(&policy.to_cbor()).unwrap();
    assert_eq!(decoded.max_retries, Some(0));
    // A writer without the field, production's encoder among them, sends
    // keys 1 to 8 only; its policy decodes with the field absent.
    let Cbor::Map(mut entries) = policy.to_cbor() else {
        panic!("a policy encodes as a map");
    };
    entries.retain(|(key, _)| *key != 9);
    let older = crate::OperationPolicy::from_cbor(&Cbor::Map(entries)).unwrap();
    assert_eq!(older.max_retries, None);
}
#[test]
fn request_records_its_resolved_limits_and_nothing_is_installed() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let ceilings = endpoint.capacity_for_test().unwrap();
    let mut custom = meta("capacity-custom");
    custom.policy = Some(crate::OperationPolicy {
        concurrency: Some(400),
        max_connections_per_host: Some(50),
        ..Default::default()
    });
    let request = wait(runtime.request(custom, "operation-one".into())).unwrap();
    assert_eq!(
        endpoint.limits_for_test("capacity-custom"),
        Some(pool::Capacity {
            per_user_host: 50,
            per_host: 50,
            total: 400,
            max_requests: 1024,
        })
    );
    // A request that overlaps it has limits of its own; the first keeps its own, and the pool's ceilings are untouched.
    let next = wait(runtime.request(meta("capacity-default"), "operation-two".into())).unwrap();
    assert_eq!(
        endpoint.limits_for_test("capacity-default"),
        Some(pool::Capacity {
            per_user_host: 32,
            per_host: 32,
            total: 256,
            max_requests: 1024,
        })
    );
    assert_eq!(
        endpoint
            .limits_for_test("capacity-custom")
            .unwrap()
            .per_host,
        50
    );
    assert_eq!(endpoint.capacity_for_test(), Some(ceilings));
    wait(next.finish());
    wait(request.finish());
    wait(runtime.shutdown());
}
#[test]
fn live_request_between_leases_admits_a_different_limit() {
    // Plan 1.4: differing limits are admitted while another request is live; nothing is refused (no
    // `TransportCapacityConflict`), and neither request's limits change when the other ends.
    let runtime = TransportRuntime::new(config()).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let first = wait(runtime.request(meta("capacity-first"), "first".into())).unwrap();
    let mut different = meta("capacity-different");
    different.policy = Some(crate::OperationPolicy {
        max_connections_per_host: Some(16),
        ..Default::default()
    });
    let second = wait(runtime.request(different, "different".into()))
        .expect("a live request no longer blocks a different limit");
    assert_eq!(
        endpoint.limits_for_test("capacity-first").unwrap().per_host,
        32
    );
    assert_eq!(
        endpoint
            .limits_for_test("capacity-different")
            .unwrap()
            .per_host,
        16
    );
    wait(first.finish());
    assert_eq!(
        endpoint
            .limits_for_test("capacity-different")
            .unwrap()
            .per_host,
        16
    );
    wait(second.finish());
    wait(runtime.shutdown());
}
fn meta(id: &str) -> RequestMeta {
    RequestMeta {
        request_id: id.into(),
        schema_version: "gwz.protocol/v0".into(),
        ..Default::default()
    }
}

pub(super) fn wait<F: Future>(future: F) -> F::Output {
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
    let meta = meta("local");
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
fn cancelling_one_bound_request_preserves_its_sibling_and_future_requests() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let first = wait(runtime.request(meta("first"), "op1".into())).unwrap();
    let second_meta = meta("second");
    let second = wait(runtime.request(second_meta.clone(), "op2".into())).unwrap();
    first.cancel();
    assert_eq!(wait(first.finish()).pending_local_work, 0);
    second.context.validate(&second_meta, "op2").unwrap();
    assert_eq!(wait(second.finish()).pending_local_work, 0);
    let third = wait(runtime.request(meta("third"), "op3".into())).unwrap();
    assert_eq!(wait(third.finish()).pending_local_work, 0);
    wait(runtime.shutdown());
}

#[test]
fn cloned_cancellation_handle_only_cancels_its_live_request() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<super::TransportCancellation>();
    let runtime = TransportRuntime::new(config()).unwrap();
    let first_meta = meta("handle-first");
    let first = wait(runtime.request(first_meta.clone(), "op1".into())).unwrap();
    let second_meta = meta("handle-second");
    let second = wait(runtime.request(second_meta.clone(), "op2".into())).unwrap();
    let handle = first.cancellation_handle();
    let clone = handle.clone();
    std::thread::spawn(move || clone.cancel()).join().unwrap();
    assert!(first.context.validate(&first_meta, "op1").is_err());
    second.context.validate(&second_meta, "op2").unwrap();
    wait(first.finish());
    handle.cancel();
    second.context.validate(&second_meta, "op2").unwrap();
    wait(second.finish());
    wait(runtime.shutdown());
}

#[test]
fn finish_after_supervisor_shutdown_does_not_strand_request_owner() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let scope = wait(runtime.request(meta("r"), "op".into())).unwrap();
    wait(runtime.shutdown());
    std::thread::sleep(Duration::from_millis(20));
    let report = wait(scope.finish());
    assert_eq!(report.pending_local_work, 0);
    assert!(!report.peer_cleanup_confirmed);
}
